//! Figma keyframe motion: ANIMATION_PRESET_INSTANCE / KEYFRAME_TRACK /
//! KEYFRAME → `doc.motion` clips. Synthetic fixtures mirror the raw shape
//! observed in a real file (see the discovered-schema notes in
//! `mapping/motion.rs`).

use super::*;
use fanta_doc::motion::{Interpolation, MotionProperty, MotionTarget};
use fanta_doc::value::ResolvedVarValue;

// ---- fixture builders in the OBSERVED raw shape -----------------------------

/// A `VariableDataMapEntry` binding `field` to `KEYFRAME(base, track)` — the
/// exact nesting observed on the fixture's consumer TEXT node.
fn keyframe_binding_entry(
    field: &str,
    base: f64,
    preset: (u32, u32),
    track: (u32, u32),
    timeline: (u32, u32),
) -> KiwiValue {
    let track_parameter = o(
        "KeyframeTrackParameter",
        vec![
            ("type", KiwiValue::Enum("ANIMATION_PRESET".into())),
            (
                "value",
                o(
                    "KeyframeTrackAnyParameter",
                    vec![(
                        "animationPreset",
                        o(
                            "AnimationPresetKeyframeTrackParameter",
                            vec![
                                (
                                    "animationPresetId",
                                    o(
                                        "AnimationPresetId",
                                        vec![("guid", guid(preset.0, preset.1))],
                                    ),
                                ),
                                (
                                    "keyframeTrackId",
                                    o("KeyframeTrackId", vec![("guid", guid(track.0, track.1))]),
                                ),
                                ("timelineDefId", guid(timeline.0, timeline.1)),
                            ],
                        ),
                    )],
                ),
            ),
        ],
    );
    o(
        "VariableDataMapEntry",
        vec![
            ("variableField", KiwiValue::Enum(field.into())),
            (
                "variableData",
                o(
                    "VariableData",
                    vec![
                        ("dataType", KiwiValue::Enum("EXPRESSION".into())),
                        ("resolvedDataType", KiwiValue::Enum("FLOAT".into())),
                        (
                            "value",
                            o(
                                "VariableAnyValue",
                                vec![(
                                    "expressionValue",
                                    o(
                                        "Expression",
                                        vec![
                                            (
                                                "expressionFunction",
                                                KiwiValue::Enum("KEYFRAME".into()),
                                            ),
                                            (
                                                "expressionArguments",
                                                KiwiValue::array(vec![
                                                    o(
                                                        "VariableData",
                                                        vec![
                                                            (
                                                                "dataType",
                                                                KiwiValue::Enum("FLOAT".into()),
                                                            ),
                                                            (
                                                                "value",
                                                                o(
                                                                    "VariableAnyValue",
                                                                    vec![(
                                                                        "floatValue",
                                                                        KiwiValue::Float(
                                                                            base as f32,
                                                                        ),
                                                                    )],
                                                                ),
                                                            ),
                                                        ],
                                                    ),
                                                    o(
                                                        "VariableData",
                                                        vec![
                                                            (
                                                                "dataType",
                                                                KiwiValue::Enum(
                                                                    "KEYFRAME_TRACK_PARAMETER_DATA"
                                                                        .into(),
                                                                ),
                                                            ),
                                                            (
                                                                "value",
                                                                o(
                                                                    "VariableAnyValue",
                                                                    vec![(
                                                                        "keyframeTrackParameterValue",
                                                                        o(
                                                                            "KeyframeTrackParameterValue",
                                                                            vec![(
                                                                                "parameters",
                                                                                KiwiValue::array(
                                                                                    vec![
                                                                                    track_parameter,
                                                                                ],
                                                                                ),
                                                                            )],
                                                                        ),
                                                                    )],
                                                                ),
                                                            ),
                                                        ],
                                                    ),
                                                ]),
                                            ),
                                        ],
                                    ),
                                )],
                            ),
                        ),
                    ],
                ),
            ),
        ],
    )
}

fn consumption_map(entries: Vec<KiwiValue>) -> KiwiValue {
    o(
        "VariableDataMap",
        vec![("entries", KiwiValue::array(entries))],
    )
}

/// `timelineDefinitions` in the observed shape (carried by the animated
/// node's top-level container).
fn timeline_definitions(timeline: (u32, u32), duration_us: u64) -> KiwiValue {
    o(
        "TimelineDefinitionsMap",
        vec![(
            "entries",
            KiwiValue::array(vec![o(
                "TimelineDefinitionsMapEntry",
                vec![
                    ("id", guid(timeline.0, timeline.1)),
                    (
                        "data",
                        o(
                            "TimelineData",
                            vec![
                                ("durationUs", KiwiValue::Uint64(duration_us)),
                                ("defaultTimeline", KiwiValue::Bool(true)),
                            ],
                        ),
                    ),
                ],
            )]),
        )],
    )
}

fn preset_change(g: (u32, u32), parent: (u32, u32), name: &str) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guid", guid(g.0, g.1)),
            ("parentIndex", parent_index(parent.0, parent.1)),
            ("type", KiwiValue::Enum("ANIMATION_PRESET_INSTANCE".into())),
            ("name", KiwiValue::String(name.to_owned())),
            ("size", vector(100.0, 100.0)),
        ],
    )
}

fn track_change(g: (u32, u32), parent: (u32, u32), operation: &str) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guid", guid(g.0, g.1)),
            ("parentIndex", parent_index(parent.0, parent.1)),
            ("type", KiwiValue::Enum("KEYFRAME_TRACK".into())),
            ("name", KiwiValue::String("Keyframe Track".to_owned())),
            ("keyframeOperation", KiwiValue::Enum(operation.into())),
        ],
    )
}

/// A `KEYFRAME` change. `time_us: None` mirrors the observed t=0 keyframe,
/// which OMITS `timelinePosition` entirely.
fn keyframe_change(
    g: (u32, u32),
    parent: (u32, u32),
    time_us: Option<i64>,
    value: f64,
    easing_type: &str,
) -> KiwiValue {
    let mut fields = vec![
        ("guid", guid(g.0, g.1)),
        ("parentIndex", parent_index(parent.0, parent.1)),
        ("type", KiwiValue::Enum("KEYFRAME".into())),
        ("name", KiwiValue::String("Keyframe".to_owned())),
        (
            "easingData",
            o(
                "EasingData",
                vec![("easingType", KiwiValue::Enum(easing_type.into()))],
            ),
        ),
        (
            "keyframeValue",
            o(
                "KeyframeValueData",
                vec![
                    (
                        "value",
                        o(
                            "KeyframeAnyValue",
                            vec![("floatValue", KiwiValue::Float(value as f32))],
                        ),
                    ),
                    ("valueType", KiwiValue::Enum("FLOAT".into())),
                ],
            ),
        ),
    ];
    if let Some(us) = time_us {
        fields.push(("timelinePosition", KiwiValue::Int64(us)));
    }
    o("NodeChange", fields)
}

fn node_id_by_name(doc: &Doc, name: &str) -> NodeId {
    doc.scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find(|id| doc.scene.get(*id).is_some_and(|n| n.name == name))
        .unwrap_or_else(|| panic!("no node named {name}"))
}

fn float_keyframes(track: &fanta_doc::motion::AnimationTrack) -> Vec<(u32, f64)> {
    let mut kfs: Vec<(u32, f64)> = track
        .keyframes
        .values()
        .map(|kf| {
            let ResolvedVarValue::Float { value } = kf.value else {
                panic!("motion keyframes are floats");
            };
            (kf.time_ms, value)
        })
        .collect();
    kfs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    kfs
}

/// The observed fixture, reduced: a page with a TEXT consumer, an
/// internal-only canvas holding the "Scale" preset instance with two SCALE
/// tracks (X and Y), each keyframed 1.2 → 1.0 over 0..500 000 µs, OUT_CUBIC.
fn scale_preset_doc() -> FigDocument {
    let scale_x = keyframe_binding_entry("MOTION_SCALE_X", 1.0, (2, 14), (2, 15), (2, 6));
    let scale_y = keyframe_binding_entry("MOTION_SCALE_Y", 1.0, (2, 14), (2, 18), (2, 6));
    doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page 1".to_owned())),
            ],
        ),
        // The animated node's container carries the timeline definitions
        // (observed: durationUs 0 = auto → derive from the last keyframe).
        o(
            "NodeChange",
            vec![
                ("guid", guid(1, 3)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Container".to_owned())),
                ("size", vector(200.0, 100.0)),
                ("timelineDefinitions", timeline_definitions((2, 6), 0)),
            ],
        ),
        // The consumer: both consumption maps carry IDENTICAL entries, as
        // observed — the import must dedupe, not double-bind.
        o(
            "NodeChange",
            vec![
                ("guid", guid(1, 2)),
                ("parentIndex", parent_index(1, 3)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("EDIT ME".to_owned())),
                ("size", vector(48.0, 15.0)),
                ("textData", text_data("EDIT ME")),
                ("fontSize", KiwiValue::Float(12.0)),
                (
                    "variableConsumptionMap",
                    consumption_map(vec![scale_x.clone(), scale_y.clone()]),
                ),
                (
                    "parameterConsumptionMap",
                    consumption_map(vec![scale_x, scale_y]),
                ),
            ],
        ),
        // The internal-only canvas hosting the motion data.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Internal Only Canvas".to_owned())),
                ("internalOnly", KiwiValue::Bool(true)),
            ],
        ),
        preset_change((2, 14), (0, 2), "motion.preset_name.scale"),
        track_change((2, 15), (2, 14), "SCALE"),
        keyframe_change((2, 16), (2, 15), None, 1.2, "OUT_CUBIC"),
        keyframe_change((2, 17), (2, 15), Some(500_000), 1.0, "OUT_CUBIC"),
        track_change((2, 18), (2, 14), "SCALE"),
        keyframe_change((2, 19), (2, 18), None, 1.2, "OUT_CUBIC"),
        keyframe_change((2, 20), (2, 18), Some(500_000), 1.0, "OUT_CUBIC"),
    ])
}

// ---- tests ------------------------------------------------------------------

#[test]
fn scale_preset_imports_clip_tracks_and_keyframes() {
    let fig = scale_preset_doc();
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    assert_eq!(report.motion_clips_imported, 1);
    assert_eq!(report.motion_tracks_imported, 2);
    assert_eq!(report.motion_keyframes_imported, 4);
    assert_eq!(report.motion_keyframes_dropped, 0);
    assert_eq!(report.motion_presets_dropped, 0);
    for ty in ["ANIMATION_PRESET_INSTANCE", "KEYFRAME_TRACK", "KEYFRAME"] {
        assert!(
            !report.skipped_by_type.contains_key(ty),
            "{ty} must no longer be counted as skipped"
        );
    }

    assert_eq!(doc.motion.clips.len(), 1);
    let clip = doc.motion.clips.values().next().unwrap();
    assert_eq!(clip.name, "motion.preset_name.scale");
    assert_eq!(clip.duration_ms, 500, "500 000 µs → 500 ms");
    assert_eq!(clip.tracks.len(), 2, "duplicate map entries dedupe");

    let text = node_id_by_name(&doc, "EDIT ME");
    for property in [MotionProperty::ScaleX, MotionProperty::ScaleY] {
        let track = clip
            .track_for_target(MotionTarget::new(text, property))
            .unwrap_or_else(|| panic!("track targets {property:?} of the TEXT node"));
        // base 1.0 composed via SCALE: 1.0 × 1.2 and 1.0 × 1.0, on the
        // identity committed transform. (The raw value rides an f32 on the
        // wire, so compare through the same widening.)
        assert_eq!(
            float_keyframes(track),
            vec![(0, f64::from(1.2f32)), (500, 1.0)]
        );
        for kf in track.keyframes.values() {
            assert_eq!(kf.easing, Easing::EaseOut, "OUT_CUBIC → EaseOut");
            assert_eq!(kf.interpolation, Interpolation::Linear);
        }
    }

    // Playback sanity: the clip pops from 120% to 100% over its length.
    let target = MotionTarget::new(text, MotionProperty::ScaleX);
    let start = clip.evaluate(0);
    assert_eq!(
        start.get(target),
        Some(&ResolvedVarValue::Float {
            value: f64::from(1.2f32)
        })
    );
    let end = clip.evaluate(500);
    assert_eq!(
        end.get(target),
        Some(&ResolvedVarValue::Float { value: 1.0 })
    );
    let Some(ResolvedVarValue::Float { value: mid }) = clip.evaluate(250).get(target).cloned()
    else {
        panic!("mid-playhead sample is a float");
    };
    assert!(
        mid > 1.0 && mid < 1.2,
        "eased between the keyed values, got {mid}"
    );
}

#[test]
fn translation_offset_and_opacity_bake_against_the_committed_node() {
    // A rect committed at x=100: a MOTION_TRANSLATION_X OFFSET track (base 0,
    // values 0 → 40) bakes to absolute PositionX 100 → 140; an OPACITY SET
    // track passes through absolute. The timeline authors a real duration
    // (800 000 µs) that outlives the last keyframe.
    let translate = keyframe_binding_entry("MOTION_TRANSLATION_X", 0.0, (2, 14), (2, 15), (2, 6));
    let opacity = keyframe_binding_entry("OPACITY", 1.0, (2, 14), (2, 18), (2, 6));
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(1, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
                ("name", KiwiValue::String("Box".to_owned())),
                ("size", vector(10.0, 10.0)),
                ("transform", matrix([1.0, 0.0, 100.0, 0.0, 1.0, 0.0])),
                ("timelineDefinitions", timeline_definitions((2, 6), 800_000)),
                (
                    "variableConsumptionMap",
                    consumption_map(vec![translate, opacity]),
                ),
            ],
        ),
        preset_change((2, 14), (0, 1), "motion.preset_name.slide"),
        track_change((2, 15), (2, 14), "OFFSET"),
        keyframe_change((2, 16), (2, 15), None, 0.0, "LINEAR"),
        keyframe_change((2, 17), (2, 15), Some(500_000), 40.0, "LINEAR"),
        track_change((2, 18), (2, 14), "SET"),
        keyframe_change((2, 19), (2, 18), None, 1.0, "LINEAR"),
        keyframe_change((2, 20), (2, 18), Some(500_000), 0.25, "LINEAR"),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    assert_eq!(report.motion_clips_imported, 1);
    assert_eq!(report.motion_tracks_imported, 2);
    assert_eq!(report.motion_keyframes_imported, 4);
    let clip = doc.motion.clips.values().next().unwrap();
    assert_eq!(
        clip.duration_ms, 800,
        "the authored timeline duration outlives the last keyframe"
    );

    let node = node_id_by_name(&doc, "Box");
    let x_track = clip
        .track_for_target(MotionTarget::new(node, MotionProperty::PositionX))
        .expect("translation track lands on PositionX");
    assert_eq!(float_keyframes(x_track), vec![(0, 100.0), (500, 140.0)]);

    let opacity_track = clip
        .track_for_target(MotionTarget::new(
            node,
            MotionProperty::bound(fanta_doc::binding::BoundProp::Opacity),
        ))
        .expect("opacity track lands on the bound property");
    assert_eq!(float_keyframes(opacity_track), vec![(0, 1.0), (500, 0.25)]);
}

#[test]
fn unmappable_channels_and_unbound_tracks_are_counted_dropped() {
    // Track 2:15 binds MOTION_SHEAR (a channel the model can't address),
    // track 2:18 has NO consumer, track 2:21 maps fine. A second preset owns
    // nothing and is counted dropped.
    let shear = keyframe_binding_entry("MOTION_SHEAR", 0.0, (2, 14), (2, 15), (2, 6));
    let scale = keyframe_binding_entry("MOTION_SCALE_X", 1.0, (2, 14), (2, 21), (2, 6));
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(1, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
                ("name", KiwiValue::String("Box".to_owned())),
                ("size", vector(10.0, 10.0)),
                (
                    "variableConsumptionMap",
                    consumption_map(vec![shear, scale]),
                ),
            ],
        ),
        preset_change((2, 14), (0, 1), "motion.preset_name.mixed"),
        track_change((2, 15), (2, 14), "SET"),
        keyframe_change((2, 16), (2, 15), None, 0.0, "LINEAR"),
        keyframe_change((2, 17), (2, 15), Some(100_000), 0.5, "LINEAR"),
        track_change((2, 18), (2, 14), "SET"),
        keyframe_change((2, 19), (2, 18), None, 0.0, "LINEAR"),
        keyframe_change((2, 20), (2, 18), Some(100_000), 1.0, "LINEAR"),
        track_change((2, 21), (2, 14), "SCALE"),
        keyframe_change((2, 22), (2, 21), None, 1.0, "LINEAR"),
        keyframe_change((2, 23), (2, 21), Some(100_000), 2.0, "LINEAR"),
        preset_change((2, 30), (0, 1), "motion.preset_name.empty"),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    assert_eq!(report.motion_clips_imported, 1);
    assert_eq!(report.motion_tracks_imported, 1);
    assert_eq!(report.motion_keyframes_imported, 2);
    assert_eq!(
        report.motion_keyframes_dropped, 4,
        "2 shear-bound + 2 unbound keyframes are loud, counted losses"
    );
    assert_eq!(
        report.motion_presets_dropped, 1,
        "the empty preset yields no clip"
    );
    assert_eq!(doc.motion.clips.len(), 1);
    for ty in ["ANIMATION_PRESET_INSTANCE", "KEYFRAME_TRACK", "KEYFRAME"] {
        assert!(!report.skipped_by_type.contains_key(ty));
    }
}

#[test]
fn hold_custom_bezier_and_custom_spring_easings_map_onto_the_model() {
    let scale = keyframe_binding_entry("MOTION_SCALE_X", 1.0, (2, 14), (2, 15), (2, 6));
    // HOLD → Interpolation::Hold; CUSTOM_CUBIC reads the schema-verified
    // BezierHandles; CUSTOM_SPRING reads the schema-verified SpringParams.
    let hold = keyframe_change((2, 16), (2, 15), None, 1.0, "HOLD");
    let mut bezier = keyframe_change((2, 17), (2, 15), Some(100_000), 1.5, "CUSTOM_CUBIC");
    bezier.set_field(
        "easingData",
        o(
            "EasingData",
            vec![
                ("easingType", KiwiValue::Enum("CUSTOM_CUBIC".into())),
                (
                    "easingValue",
                    o(
                        "TransitionEasingAnyValue",
                        vec![(
                            "bezierEasing",
                            o(
                                "BezierHandles",
                                vec![
                                    ("p1x", KiwiValue::Float(0.1)),
                                    ("p1y", KiwiValue::Float(0.2)),
                                    ("p2x", KiwiValue::Float(0.3)),
                                    ("p2y", KiwiValue::Float(0.4)),
                                ],
                            ),
                        )],
                    ),
                ),
            ],
        ),
    );
    let mut spring = keyframe_change((2, 18), (2, 15), Some(200_000), 2.0, "CUSTOM_SPRING");
    spring.set_field(
        "easingData",
        o(
            "EasingData",
            vec![
                ("easingType", KiwiValue::Enum("CUSTOM_SPRING".into())),
                (
                    "easingValue",
                    o(
                        "TransitionEasingAnyValue",
                        vec![(
                            "springEasing",
                            o(
                                "SpringParams",
                                vec![
                                    ("stiffness", KiwiValue::Float(150.0)),
                                    ("damping", KiwiValue::Float(12.0)),
                                    ("mass", KiwiValue::Float(2.0)),
                                ],
                            ),
                        )],
                    ),
                ),
            ],
        ),
    );
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(1, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
                ("name", KiwiValue::String("Box".to_owned())),
                ("size", vector(10.0, 10.0)),
                ("variableConsumptionMap", consumption_map(vec![scale])),
            ],
        ),
        preset_change((2, 14), (0, 1), "motion.preset_name.custom"),
        track_change((2, 15), (2, 14), "SCALE"),
        hold,
        bezier,
        spring,
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.motion_keyframes_imported, 3);

    let clip = doc.motion.clips.values().next().unwrap();
    let node = node_id_by_name(&doc, "Box");
    let track = clip
        .track_for_target(MotionTarget::new(node, MotionProperty::ScaleX))
        .expect("scale track lands");
    let mut kfs: Vec<_> = track.keyframes.values().collect();
    kfs.sort_by_key(|kf| kf.time_ms);
    assert_eq!(kfs[0].interpolation, Interpolation::Hold);
    assert_eq!(kfs[0].easing, Easing::Linear);
    assert_eq!(
        kfs[1].easing,
        Easing::CubicBezier {
            x1: 0.1,
            y1: 0.2,
            x2: 0.3,
            y2: 0.4
        }
    );
    assert_eq!(
        kfs[2].easing,
        Easing::Spring {
            mass: 2.0,
            stiffness: 150.0,
            damping: 12.0
        }
    );
}

/// Opt-in: keyframe motion against a real Figma export. Skipped unless
/// `FANTA_FIG_FIXTURE` points at a `.fig`. The zero-skip assertion holds for
/// ANY file; the ≥1-clip assertion is gated on the file actually containing
/// keyframe tracks (the Spectrum fixture has none; the motion fixture does).
/// Run with:
///   FANTA_FIG_FIXTURE=/path/to/file.fig \
///     cargo test -p fanta-fig-interop keyframe_motion -- --ignored --nocapture
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn imports_real_fig_fixture_keyframe_motion() {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping");
        return;
    };
    let bytes = std::fs::read(&path).expect("read fixture");
    let fig = crate::fig::read_fig(&bytes).expect("real .fig must parse");
    let raw_tracks = fig
        .root
        .get("nodeChanges")
        .and_then(KiwiValue::as_array)
        .map(|changes| {
            changes
                .iter()
                .filter(|change| {
                    change.get("type").and_then(KiwiValue::as_str) == Some("KEYFRAME_TRACK")
                })
                .count()
        })
        .unwrap_or(0);
    let (doc, report, _assets) = fig_to_doc(&fig).expect("map to doc");

    eprintln!(
        "  KEYFRAME MOTION: {raw_tracks} raw KEYFRAME_TRACK changes; report: {} clips, \
             {} tracks, {} keyframes imported, {} keyframes dropped, {} presets dropped; \
             doc.motion has {} clips",
        report.motion_clips_imported,
        report.motion_tracks_imported,
        report.motion_keyframes_imported,
        report.motion_keyframes_dropped,
        report.motion_presets_dropped,
        doc.motion.clips.len(),
    );
    for clip in doc.motion.clips.values() {
        eprintln!(
            "    clip {:?}: {} ms, {} tracks",
            clip.name,
            clip.duration_ms,
            clip.tracks.len()
        );
    }

    for ty in ["ANIMATION_PRESET_INSTANCE", "KEYFRAME_TRACK", "KEYFRAME"] {
        assert!(
            !report.skipped_by_type.contains_key(ty),
            "{ty} must never land in the skip table"
        );
    }
    if raw_tracks > 0 {
        assert!(
            !doc.motion.clips.is_empty(),
            "a file with {raw_tracks} keyframe tracks must import ≥1 clip"
        );
        assert!(report.motion_keyframes_imported > 0);
    }
}
