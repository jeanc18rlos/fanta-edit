use anyhow::{Context as _, Result, ensure};
use fanta_doc::{
    Color, Doc, MotionSpec, MotionTransform, NodeId, SpecEasing, SpecEasingCurve, SpecKeyframe,
};
use gpui::{AppContext as _, Context};
use serde_json::{Map, Value, json};

impl crate::view::FigView {
    pub(crate) fn export_selected_motion(&mut self, figma: bool, cx: &mut Context<Self>) {
        let prepared = (|| {
            let item = self.item.read(cx);
            let directory = item
                .project_root()
                .context("save the design as a Fanta project before exporting motion")?
                .join("exports");
            let document = item.document().context("the design is still loading")?;
            let clip = self
                .active_motion_clip_id()
                .or_else(|| {
                    (document.doc.motion.clips.len() == 1)
                        .then(|| document.doc.motion.clips.keys().next().copied())
                        .flatten()
                })
                .context("select an animation in the Motion timeline before exporting")?;
            let animation = document
                .doc
                .motion
                .clip(clip)
                .context("the selected animation no longer exists")?;
            let name = crate::export::sanitize_file_name(&animation.name);
            let filename = format!(
                "{name}.{}",
                if figma {
                    "figma-motion.json"
                } else {
                    "motion.json"
                }
            );
            let mut doc = document.doc.clone();
            doc.motion.clips.retain(|id, _| *id == clip);
            Ok::<_, anyhow::Error>((doc, directory, filename))
        })();
        let (doc, directory, filename) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                crate::view::show_canvas_notice_deferred(
                    format!("Could not export motion: {error:#}"),
                    cx,
                );
                return;
            }
        };
        cx.spawn(async move |_, cx| {
            let exported = cx
                .background_spawn(async move {
                    let bytes = export_motion(&doc, None, figma)?;
                    std::fs::create_dir_all(&directory)
                        .context("creating the exports directory")?;
                    crate::export::write_export_atomically(&directory, &filename, &bytes)
                })
                .await;
            cx.update(|cx| {
                crate::view::show_canvas_notice_deferred(
                    match exported {
                        Ok(path) => format!("Exported motion to {}", path.display()),
                        Err(error) => format!("Could not export motion: {error:#}"),
                    },
                    cx,
                )
            });
        })
        .detach();
    }
}

pub(crate) fn export_motion(doc: &Doc, root: Option<NodeId>, figma: bool) -> Result<Vec<u8>> {
    let contains = |node| {
        root.is_none_or(|root| {
            node == root
                || doc
                    .scene
                    .ancestors_of(node)
                    .any(|ancestor| ancestor.id == root)
        })
    };
    let clips = doc
        .motion
        .clips
        .values()
        .filter(|clip| {
            clip.settings.scope.is_some_and(contains)
                || clip
                    .tracks
                    .values()
                    .any(|track| contains(track.target.node))
        })
        .collect::<Vec<_>>();
    ensure!(
        !clips.is_empty(),
        "this export target has no animation clips"
    );
    ensure!(
        clips.len() == 1,
        "this target contains multiple animation clips; select the frame scoped to the clip you want to export"
    );
    let clip = clips.first().context("the animation clip is unavailable")?;
    ensure!(
        clip.settings.scope.is_none_or(contains)
            && clip
                .tracks
                .values()
                .all(|track| contains(track.target.node)),
        "the animation extends beyond this selection; export the complete scoped frame or use Design > Export Motion JSON"
    );
    let spec = MotionSpec::from_clip(doc, clip)?;
    let value = if figma {
        figma_motion(doc, &spec)?
    } else {
        serde_json::to_value(spec)?
    };
    Ok(serde_json::to_vec_pretty(&value)?)
}

fn figma_value(value: &Value) -> Result<Value> {
    if let Some(number) = value.as_f64() {
        ensure!(number.is_finite(), "a motion value must be finite");
        return Ok(json!({"type":"FLOAT", "value":number}));
    }
    let color = value
        .as_str()
        .and_then(Color::from_hex)
        .context("the motion value is neither a number nor a color")?;
    Ok(
        json!({"type":"COLOR", "value":{"r":f64::from(color.r)/255.0, "g":f64::from(color.g)/255.0, "b":f64::from(color.b)/255.0, "a":f64::from(color.a)/255.0}}),
    )
}

fn spring_value(from: &Value, to: &Value, progress: f64) -> Result<Value> {
    if let (Some(from), Some(to)) = (from.as_f64(), to.as_f64()) {
        return figma_value(&json!(from + (to - from) * progress));
    }
    let from = from
        .as_str()
        .and_then(Color::from_hex)
        .context("invalid spring color")?;
    let to = to
        .as_str()
        .and_then(Color::from_hex)
        .context("invalid spring color")?;
    let channel = |from: u8, to: u8| {
        (f64::from(from) + (f64::from(to) - f64::from(from)) * progress)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    figma_value(&json!(
        Color::rgba(
            channel(from.r, to.r),
            channel(from.g, to.g),
            channel(from.b, to.b),
            channel(from.a, to.a)
        )
        .to_hex()
    ))
}

fn figma_keyframes(keyframes: &[SpecKeyframe], fps: u16) -> Result<Vec<Value>> {
    let mut canonical = Vec::<&SpecKeyframe>::new();
    for keyframe in keyframes {
        if let Some(previous) = canonical.last_mut()
            && previous.t == keyframe.t
        {
            *previous = keyframe;
        } else {
            canonical.push(keyframe);
        }
    }
    let mut output = Vec::new();
    for (index, keyframe) in canonical.iter().enumerate() {
        // Fanta eases the outgoing segment; Figma eases the incoming one.
        let easing = index
            .checked_sub(1)
            .and_then(|index| canonical.get(index))
            .filter(|previous| physical_spring(previous).is_none())
            .map_or_else(
                || json!({"type":"LINEAR"}),
                |previous| {
                    fanta_doc::motion_spec::figma_easing(
                        previous
                            .easing
                            .as_ref()
                            .unwrap_or(&SpecEasing::Named(fanta_doc::SpecEasingName::Linear)),
                        previous.hold,
                    )
                },
            );
        output.push(json!({"timelinePosition":keyframe.t, "value":figma_value(&keyframe.v)?, "easing":easing}));
        if let (Some((mass, stiffness, damping)), Some(next)) =
            (physical_spring(keyframe), canonical.get(index + 1))
        {
            let duration = next.t - keyframe.t;
            ensure!(
                duration.is_finite() && duration >= 0.0,
                "motion keyframes must be ordered"
            );
            let steps = (duration * f64::from(fps)).ceil().max(1.0);
            ensure!(
                steps <= 10_000.0,
                "a spring segment exceeds 10,000 export samples; shorten the clip or reduce its frame rate"
            );
            for sample in 1..steps as u32 {
                let progress = f64::from(sample) / steps;
                output.push(json!({"timelinePosition":keyframe.t+duration*progress,
                    "value":spring_value(&keyframe.v,&next.v,fanta_doc::spring_progress(mass,stiffness,damping,progress))?,
                    "easing":{"type":"LINEAR"}}));
            }
        }
    }
    Ok(output)
}

fn physical_spring(keyframe: &SpecKeyframe) -> Option<(f32, f32, f32)> {
    match keyframe.easing.as_ref() {
        Some(SpecEasing::Curve(SpecEasingCurve::Spring {
            mass,
            stiffness,
            damping,
        })) if !keyframe.hold => Some((*mass, *stiffness, *damping)),
        _ => None,
    }
}

fn relative_transform_keyframes(
    keyframes: &[SpecKeyframe],
    property: &str,
    resting: MotionTransform,
) -> Result<Vec<SpecKeyframe>> {
    keyframes
        .iter()
        .map(|keyframe| {
            let value = keyframe
                .v
                .as_f64()
                .context("a transform value must be numeric")?;
            let value = match property {
                "translation_x" => value - resting.position[0],
                "translation_y" => value - resting.position[1],
                "rotation" => resting.rotation_radians.to_degrees() - value,
                "scale_x" => value / resting.scale[0],
                "scale_y" => value / resting.scale[1],
                _ => value,
            };
            ensure!(
                value.is_finite(),
                "a relative transform value must be finite"
            );
            let mut keyframe = keyframe.clone();
            keyframe.v = json!(value);
            Ok(keyframe)
        })
        .collect()
}

fn figma_transform_origin(doc: &Doc, node: NodeId, origin: Option<[f64; 2]>) -> Option<[f64; 2]> {
    let authored = doc.scene.get(node)?;
    let bounds = doc.scene.local_bounds(node)?;
    let pivot = origin
        .zip(authored.data.local_bounds())
        .map(|(origin, bounds)| {
            [
                bounds.min_x + origin[0] * bounds.width(),
                bounds.min_y + origin[1] * bounds.height(),
            ]
        })
        .unwrap_or([0.0, 0.0]);
    let normalize = |coordinate: f64, minimum: f64, size: f64| {
        if size == 0.0 && coordinate == minimum {
            Some(0.0)
        } else {
            let normalized = (coordinate - minimum) / size;
            normalized.is_finite().then_some(normalized)
        }
    };
    Some([
        normalize(pivot[0], bounds.min_x, bounds.width())?,
        normalize(pivot[1], bounds.min_y, bounds.height())?,
    ])
}

fn figma_motion(doc: &Doc, spec: &MotionSpec) -> Result<Value> {
    let mut diagnostics = Vec::new();
    let mut nodes = Vec::new();
    for layer in &spec.layers {
        let mut tracks = Map::new();
        let node = layer.node.parse::<NodeId>().ok();
        let resting = node
            .and_then(|node| doc.scene.get(node))
            .and_then(|node| MotionTransform::decompose(node.transform));
        let origin = node.and_then(|node| figma_transform_origin(doc, node, layer.origin));
        for (property, keyframes) in &layer.tracks {
            if keyframes.is_empty() {
                continue;
            }
            let field = fanta_doc::motion_spec::figma_track_field(property);
            if field.is_none() && property != "fill_color" {
                diagnostics.push(format!(
                    "{}: {property} is not supported by this Figma exporter",
                    layer.node
                ));
                continue;
            }
            let converted;
            let keyframes = if matches!(
                property.as_str(),
                "translation_x" | "translation_y" | "rotation" | "scale_x" | "scale_y"
            ) {
                if matches!(property.as_str(), "rotation" | "scale_x" | "scale_y")
                    && origin.is_none()
                {
                    diagnostics.push(format!(
                        "{}: {property} has no representable transform origin",
                        layer.node
                    ));
                    continue;
                }
                let Some(resting) = resting else {
                    diagnostics.push(format!(
                        "{}: {property} has no supported resting transform (missing node, shear or zero scale)",
                        layer.node
                    ));
                    continue;
                };
                converted = relative_transform_keyframes(keyframes, property, resting)?;
                converted.as_slice()
            } else {
                keyframes.as_slice()
            };
            let keyframes = figma_keyframes(keyframes, spec.fps.unwrap_or(30).max(1))?;
            let binding = json!({"keyframes":keyframes});
            if let Some(field) = field {
                tracks.insert(field.into(), binding);
            } else {
                tracks.insert("fills".into(), json!({"0":binding}));
            }
        }
        nodes.push(json!({"node":layer.node, "name":layer.name, "transformOrigin":origin, "manualKeyframeTracks":tracks}));
    }
    Ok(
        json!({"version":1, "name":spec.name, "timeline":{"duration":spec.duration}, "fps":spec.fps,
        "loop":spec.loop_mode, "scope":spec.scope, "size":spec.size, "background":spec.background,
        "nodes":nodes, "diagnostics":diagnostics}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, NodeData, Transform2D, VectorNode};

    fn document_with_vector(transform: Transform2D) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            100.0,
            100.0,
            Color::WHITE,
        )));
        node.transform = transform;
        let identifier = node.id;
        doc.scene.insert(node).expect("vector");
        (doc, identifier)
    }

    #[test]
    fn figma_motion_exports_seconds_degrees_fills_and_unsupported_diagnostics() {
        let (doc, node) = document_with_vector(Transform2D::IDENTITY);
        let spec: MotionSpec =
            serde_json::from_value(json!({"version":1,"name":"Hero","duration":1.2,"fps":30,
            "layers":[{"node":node.to_string(),"tracks":{
                "rotation":[{"t":0.0,"v":0.0},{"t":1.2,"v":90.0}],
                "fill_color":[{"t":0.0,"v":"#FF000080"}],
                "stroke_width":[{"t":0.0,"v":2.0}]
            }}]}))
            .expect("spec");
        let output = figma_motion(&doc, &spec).expect("export");
        let tracks = &output["nodes"][0]["manualKeyframeTracks"];
        assert_eq!(tracks["ROTATION"]["keyframes"][1]["timelinePosition"], 1.2);
        assert_eq!(tracks["ROTATION"]["keyframes"][1]["value"]["value"], -90.0);
        assert_eq!(
            tracks["fills"]["0"]["keyframes"][0]["value"]["type"],
            "COLOR"
        );
        assert!(tracks["ROTATION"].get("baseValue").is_none());
        assert_eq!(
            output["diagnostics"].as_array().expect("diagnostics").len(),
            1
        );
    }
    #[test]
    fn figma_motion_duplicate_times_use_the_evaluators_last_keyframe() {
        let keyframes = vec![
            SpecKeyframe {
                t: 0.0,
                v: json!(10.0),
                easing: None,
                hold: false,
            },
            SpecKeyframe {
                t: 0.0,
                v: json!(20.0),
                easing: None,
                hold: false,
            },
        ];
        let exported = figma_keyframes(&keyframes, 30).expect("keyframes");
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0]["value"]["value"], 20.0);
    }

    #[test]
    fn figma_motion_samples_physical_springs_without_invalid_easing_parameters() {
        let (doc, node) = document_with_vector(Transform2D::IDENTITY);
        let spec: MotionSpec =
            serde_json::from_value(json!({"version":1,"name":"Spring","duration":1.0,"fps":30,
            "layers":[{"node":node.to_string(),"tracks":{"translation_x":[
                {"t":0.0,"v":0.0,"easing":{"spring":{"mass":1.0,"stiffness":100.0,"damping":10.0}}},
                {"t":1.0,"v":120.0} ]}}]}))
            .expect("spec");
        let output = figma_motion(&doc, &spec).expect("export");
        let keyframes = output["nodes"][0]["manualKeyframeTracks"]["TRANSLATION_X"]["keyframes"]
            .as_array()
            .expect("keyframes");
        assert_eq!(keyframes.len(), 31);
        assert!(
            keyframes
                .iter()
                .all(|frame| frame["easing"]["type"] == "LINEAR")
        );
        assert_eq!(keyframes.last().expect("end")["value"]["value"], 120.0);
    }

    #[test]
    fn figma_transform_tracks_are_relative_to_the_authored_transform() {
        let transform = Transform2D::scale_xy(2.0, 3.0)
            .then(&Transform2D::rotation(30f64.to_radians()))
            .then(&Transform2D::translation(100.0, 200.0));
        let (doc, node) = document_with_vector(transform);
        let spec: MotionSpec = serde_json::from_value(json!({
            "version":1,"name":"Relative","duration":1.0,
            "layers":[{"node":node.to_string(),"origin":[0.5,0.5],"tracks":{
                "translation_x":[{"t":0,"v":100},{"t":1,"v":120}],
                "translation_y":[{"t":0,"v":200},{"t":1,"v":260}],
                "rotation":[{"t":0,"v":30},{"t":1,"v":120}],
                "scale_x":[{"t":0,"v":2},{"t":1,"v":4}],
                "scale_y":[{"t":0,"v":3},{"t":1,"v":1.5}]
            }}]
        }))
        .expect("spec");
        let output = figma_motion(&doc, &spec).expect("export");
        let tracks = &output["nodes"][0]["manualKeyframeTracks"];
        for (field, start, end) in [
            ("TRANSLATION_X", 0.0, 20.0),
            ("TRANSLATION_Y", 0.0, 60.0),
            ("ROTATION", 0.0, -90.0),
            ("SCALE_X", 1.0, 2.0),
            ("SCALE_Y", 1.0, 0.5),
        ] {
            let samples = &tracks[field]["keyframes"];
            assert!((samples[0]["value"]["value"].as_f64().expect("start") - start).abs() < 1e-9);
            assert!((samples[1]["value"]["value"].as_f64().expect("end") - end).abs() < 1e-9);
        }
        assert_eq!(output["nodes"][0]["transformOrigin"], json!([0.5, 0.5]));
    }

    #[test]
    fn figma_origin_accounts_for_vector_bounds_that_start_away_from_zero() {
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            10.0,
            20.0,
            20.0,
            40.0,
            Color::WHITE,
        )));
        let identifier = node.id;
        doc.scene.insert(node).expect("vector");
        assert_eq!(
            figma_transform_origin(&doc, identifier, None),
            Some([-0.5, -0.5])
        );
        assert_eq!(
            figma_transform_origin(&doc, identifier, Some([0.5, 0.25])),
            Some([0.5, 0.25])
        );
    }

    #[test]
    fn figma_easing_and_hold_describe_the_incoming_segment() {
        let keyframes: Vec<SpecKeyframe> = serde_json::from_value(json!([
            {"t":0,"v":0,"easing":"ease_out"},
            {"t":1,"v":1,"hold":true},
            {"t":2,"v":2,"easing":"ease_in"},
            {"t":3,"v":3}
        ]))
        .expect("keyframes");
        let output = figma_keyframes(&keyframes, 30).expect("export");
        assert_eq!(output[0]["easing"]["type"], "LINEAR");
        assert_eq!(output[1]["easing"]["type"], "EASE_OUT");
        assert_eq!(output[2]["easing"]["type"], "HOLD");
        assert_eq!(output[3]["easing"]["type"], "EASE_IN");
    }

    #[test]
    fn figma_spring_endpoint_keeps_linear_incoming_easing() {
        let keyframes: Vec<SpecKeyframe> = serde_json::from_value(json!([
            {"t":0,"v":0,"easing":{"spring":{"mass":1,"stiffness":100,"damping":10}}},
            {"t":1,"v":10,"easing":"ease_out"},
            {"t":2,"v":20}
        ]))
        .expect("keyframes");
        let output = figma_keyframes(&keyframes, 10).expect("export");
        assert_eq!(output.len(), 12);
        for keyframe in &output[..11] {
            assert_eq!(keyframe["easing"]["type"], "LINEAR");
        }
        assert_eq!(output[11]["easing"]["type"], "EASE_OUT");
    }

    #[test]
    fn figma_reports_unsupported_resting_transforms_without_dropping_opacity() {
        let (doc, node) = document_with_vector(Transform2D::from_components([
            1.0, 0.0, 0.5, 1.0, 20.0, 30.0,
        ]));
        let spec: MotionSpec = serde_json::from_value(json!({
            "version":1,"name":"Shear","duration":1,
            "layers":[{"node":node.to_string(),"tracks":{
                "translation_x":[{"t":0,"v":20}],
                "opacity":[{"t":0,"v":0.5}]
            }}]
        }))
        .expect("spec");
        let output = figma_motion(&doc, &spec).expect("export");
        assert!(
            output["nodes"][0]["manualKeyframeTracks"]
                .get("TRANSLATION_X")
                .is_none()
        );
        assert_eq!(
            output["nodes"][0]["manualKeyframeTracks"]["OPACITY"]["keyframes"][0]["value"]["value"],
            0.5
        );
        assert!(
            output["diagnostics"][0]
                .as_str()
                .expect("diagnostic")
                .contains("shear")
        );
    }
}
