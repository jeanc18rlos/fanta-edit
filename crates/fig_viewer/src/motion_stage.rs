//! The Claude Motion Stage package: one animation clip as a Stage project
//! (`index.html`, `animations.jsx`, `Scene.jsx`) plus a self-contained player
//! (`preview.html`). Each animated layer is a bitmap Fanta renders at rest;
//! `scene.js` moves it with the clip's position, rotation, scale, opacity and
//! visibility keyframes, using a port of Fanta's own keyframe evaluator, so
//! both players and the editor agree frame for frame.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use fanta_doc::{
    AnimationClip, AnimationTrack, BlendMode, BlurKind, BoundProp, Bounds, Doc, Easing,
    Interpolation, Keyframe, LoopMode, MotionProperty, MotionTransform, NodeData, NodeFlags,
    NodeId, ResolvedVarValue, Scene, Transform2D, UnitInterval, Viewport, resolve_effective_mode,
};
use fanta_render::{AssetResolver, RasterRenderer, visual_world_bounds};
use glam::DVec2;
use serde_json::{Value, json};

use crate::export::{
    ExportArtifact, ExportEntry, ExportPayload, ExportSizing, raster_dimensions, render_inputs,
};
use crate::motion_video::{MotionStage, is_descendant, loop_name, node_label, sample_times};

const ANIMATIONS_JSX: &str = include_str!("../assets/motion_stage/animations.jsx");
const SCENE_JSX: &str = include_str!("../assets/motion_stage/Scene.jsx");
const PREVIEW_HTML: &str = include_str!("../assets/motion_stage/preview.html");
const SCENE_RUNTIME_JS: &str = include_str!("../assets/motion_stage/scene-runtime.js");

/// Pinned like Claude Design's Stage projects, with subresource integrity.
const REACT_SCRIPTS: &str = r#"<script src="https://unpkg.com/react@18.3.1/umd/react.development.js" integrity="sha384-hD6/rw4ppMLGNu3tX5cjIb+uRZ7UkRJ6BPkLpg4hAu/6onKUg4lLsHAs9EBPT82L" crossorigin="anonymous"></script>
<script src="https://unpkg.com/react-dom@18.3.1/umd/react-dom.development.js" integrity="sha384-u6aeetuaXnQ38mYT8rp6sbXaQe3NL9t+IBXmnYxwkUI2Hw4bsp2Wvmx4yRQF1uAm" crossorigin="anonymous"></script>
<script src="https://unpkg.com/@babel/standalone@7.29.0/babel.min.js" integrity="sha384-m08KidiNqLdpJqLq95G/LEi8Qvjl/xUYll3QILypMoQ65QorJ9Lvtp2RXYGBFj1y" crossorigin="anonymous"></script>"#;

/// Overlap checks sample the motion this many times at most.
const COVERAGE_SAMPLES: usize = 120;
/// Builds the quarter-circle Bézier of a rounded corner.
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// The keyframed channel a sprite can animate, by its name in `scene.js`.
fn sprite_channel(property: MotionProperty) -> Option<&'static str> {
    match property {
        MotionProperty::PositionX => Some("x"),
        MotionProperty::PositionY => Some("y"),
        MotionProperty::Rotation => Some("rotation"),
        MotionProperty::ScaleX => Some("scaleX"),
        MotionProperty::ScaleY => Some("scaleY"),
        MotionProperty::Bound {
            prop: BoundProp::Opacity,
        } => Some("opacity"),
        MotionProperty::Bound {
            prop: BoundProp::Visible,
        } => Some("visible"),
        MotionProperty::Bound { .. } => None,
    }
}

fn is_transform_channel(property: MotionProperty) -> bool {
    matches!(
        property,
        MotionProperty::PositionX
            | MotionProperty::PositionY
            | MotionProperty::Rotation
            | MotionProperty::ScaleX
            | MotionProperty::ScaleY
    )
}

fn baked_property_label(property: MotionProperty) -> &'static str {
    match property {
        MotionProperty::Bound { prop } => match prop {
            BoundProp::FillColor { .. } => "fill color",
            BoundProp::StrokeColor { .. } => "stroke color",
            BoundProp::StrokeWidth { .. } => "stroke width",
            BoundProp::CornerRadius => "corner radius",
            BoundProp::TextContent => "text",
            BoundProp::TextStyle => "text style",
            BoundProp::ClipWidth | BoundProp::ClipHeight => "size",
            BoundProp::ImageFill { .. } => "image",
            BoundProp::Opacity => "opacity",
            BoundProp::Visible => "visibility",
        },
        MotionProperty::PositionX | MotionProperty::PositionY => "position",
        MotionProperty::Rotation => "rotation",
        MotionProperty::ScaleX | MotionProperty::ScaleY => "scale",
    }
}

/// The CSS `mix-blend-mode` for a blend mode: `Some(None)` is normal blending,
/// `None` has no CSS equivalent.
fn css_blend(mode: BlendMode) -> Option<Option<&'static str>> {
    Some(match mode {
        BlendMode::Normal => None,
        BlendMode::Multiply => Some("multiply"),
        BlendMode::Screen => Some("screen"),
        BlendMode::Overlay => Some("overlay"),
        BlendMode::Darken => Some("darken"),
        BlendMode::Lighten => Some("lighten"),
        BlendMode::ColorDodge => Some("color-dodge"),
        BlendMode::ColorBurn => Some("color-burn"),
        BlendMode::HardLight => Some("hard-light"),
        BlendMode::SoftLight => Some("soft-light"),
        BlendMode::Difference => Some("difference"),
        BlendMode::Exclusion => Some("exclusion"),
        BlendMode::Hue => Some("hue"),
        BlendMode::Saturation => Some("saturation"),
        BlendMode::Color => Some("color"),
        BlendMode::Luminosity => Some("luminosity"),
        BlendMode::LinearDodge => Some("plus-lighter"),
        BlendMode::LinearBurn
        | BlendMode::DarkerColor
        | BlendMode::LighterColor
        | BlendMode::VividLight
        | BlendMode::LinearLight
        | BlendMode::PinLight
        | BlendMode::HardMix
        | BlendMode::Subtract
        | BlendMode::Divide => return None,
    })
}

/// Depth-first paint order of the stage subtree, back to front. Boolean
/// operands are geometry, not painted layers, so the walk stops at booleans.
fn paint_order(scene: &Scene, stage: NodeId) -> Vec<NodeId> {
    let mut order = Vec::new();
    let mut stack = vec![stage];
    while let Some(id) = stack.pop() {
        order.push(id);
        if matches!(
            scene.get(id).map(|node| &node.data),
            Some(NodeData::Boolean(_))
        ) {
            continue;
        }
        stack.extend(scene.children_of(Some(id)).iter().rev().copied());
    }
    order
}

/// Ancestors of `node` below the stage, nearest first.
fn ancestors_below(scene: &Scene, node: NodeId, stage: NodeId) -> Vec<NodeId> {
    scene
        .ancestors_of(node)
        .map(|ancestor| ancestor.id)
        .take_while(|ancestor| *ancestor != stage)
        .collect()
}

fn is_hidden(scene: &Scene, node: NodeId) -> bool {
    scene
        .get(node)
        .is_some_and(|node| node.flags.contains(NodeFlags::HIDDEN))
}

fn paints_itself(scene: &Scene, node: NodeId) -> bool {
    match scene.get(node) {
        Some(canvas_node) => match &canvas_node.data {
            NodeData::Group(group) => {
                group.background.is_some()
                    || !group.background_fills.is_empty()
                    || !group.strokes.is_empty()
                    || !canvas_node.effects.is_empty()
            }
            _ => true,
        },
        None => false,
    }
}

/// A mask masks the siblings painted above it, so the nearest earlier mask
/// sibling is the one in effect.
fn masked_by(scene: &Scene, node: NodeId) -> Option<NodeId> {
    let parent = scene.get(node)?.parent;
    let siblings = scene.children_of(parent);
    let index = siblings.iter().position(|sibling| *sibling == node)?;
    siblings
        .get(..index)?
        .iter()
        .rev()
        .copied()
        .find(|sibling| scene.get(*sibling).is_some_and(|sibling| sibling.is_mask))
}

/// One animated layer of the package; `parent` indexes the nearest animated
/// ancestor, whose element contains this one's.
struct Sprite {
    node: NodeId,
    parent: Option<usize>,
}

pub(crate) fn stage_package(
    stage: &MotionStage,
    resolver: Option<Arc<dyn AssetResolver>>,
    zoom: f64,
) -> Result<ExportArtifact> {
    let clip = stage.clip()?;
    let doc = &stage.doc;
    let scene = &doc.scene;
    let mut notes = stage.notes.clone();

    let order = paint_order(scene, stage.stage);
    let position = order
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<HashMap<_, _>>();

    let mut tracks_by_node = BTreeMap::<NodeId, Vec<&AnimationTrack>>::new();
    for track in clip.tracks.values() {
        let node = track.target.node;
        if node == stage.stage {
            if !is_transform_channel(track.target.property) {
                notes.push(format!(
                    "The stage's {} animation is shown at rest; the MP4, GIF and PNG frame exports animate it.",
                    baked_property_label(track.target.property)
                ));
            }
            continue;
        }
        if position.contains_key(&node) {
            tracks_by_node.entry(node).or_default().push(track);
        } else if scene.get(node).is_some() && is_descendant(doc, node, stage.stage) {
            notes.push(format!(
                "{} is part of a boolean shape, so its animation is shown at rest; the MP4, GIF and PNG frame exports animate it.",
                node_label(doc, node)
            ));
        } else {
            notes.push(format!(
                "{} is outside the stage frame, so its animation is not exported.",
                node_label(doc, node)
            ));
        }
    }

    for (node, tracks) in &tracks_by_node {
        let mut baked = tracks
            .iter()
            .filter(|track| sprite_channel(track.target.property).is_none())
            .map(|track| baked_property_label(track.target.property))
            .collect::<Vec<_>>();
        baked.sort_unstable();
        baked.dedup();
        if !baked.is_empty() {
            notes.push(format!(
                "{} animates its {}, shown at rest here; the MP4, GIF and PNG frame exports animate it.",
                node_label(doc, *node),
                baked.join(", ")
            ));
        }
    }

    let mut candidates = tracks_by_node
        .iter()
        .filter(|(_, tracks)| {
            tracks
                .iter()
                .any(|track| sprite_channel(track.target.property).is_some())
        })
        .map(|(node, _)| *node)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|node| position.get(node).copied().unwrap_or(usize::MAX));

    let mut sprites = Vec::<Sprite>::new();
    let mut sprite_index = HashMap::<NodeId, usize>::new();
    let mut skipped = HashSet::<NodeId>::new();
    for node in candidates {
        let ancestors = ancestors_below(scene, node, stage.stage);
        let parent = ancestors
            .iter()
            .find_map(|ancestor| sprite_index.get(ancestor).copied());
        let has_visibility_track = tracks_by_node.get(&node).is_some_and(|tracks| {
            tracks.iter().any(|track| {
                track.target.property
                    == MotionProperty::Bound {
                        prop: BoundProp::Visible,
                    }
            })
        });
        let static_ancestor_hidden = ancestors
            .iter()
            .take_while(|ancestor| !sprite_index.contains_key(*ancestor))
            .any(|ancestor| is_hidden(scene, *ancestor));
        let never_visible = ancestors.iter().any(|ancestor| skipped.contains(ancestor))
            || static_ancestor_hidden
            || (is_hidden(scene, node) && !has_visibility_track);
        if never_visible {
            skipped.insert(node);
            continue;
        }
        sprite_index.insert(node, sprites.len());
        sprites.push(Sprite { node, parent });
    }

    let mut layers = Vec::with_capacity(sprites.len());
    let mut entries = Vec::new();
    for (index, sprite) in sprites.iter().enumerate() {
        let (layer, image) = layer_entry(
            &LayerContext {
                stage,
                clip,
                sprites: &sprites,
                sprite_index: &sprite_index,
                tracks_by_node: &tracks_by_node,
                resolver: resolver.clone(),
                zoom,
            },
            index,
            sprite,
            &mut notes,
        )?;
        layers.push(layer);
        entries.extend(image);
    }

    let mut background_doc = doc.clone();
    for sprite in &sprites {
        if let Some(node) = background_doc.scene.get_mut(sprite.node) {
            node.flags.insert(NodeFlags::HIDDEN);
        }
    }
    let (background_png, background_box) = render_region(
        &background_doc,
        stage.stage,
        stage.bounds(),
        resolver,
        zoom,
        "the stage background",
    )?;
    entries.push(ExportEntry {
        path: "assets/background.png".to_string(),
        bytes: background_png,
    });

    notes.extend(coverage_notes(stage, &order, &position, &sprite_index)?);

    let duration = seconds(stage.duration_ms);
    let stage_duration = if stage.loop_mode == LoopMode::PingPong {
        duration * 2.0
    } else {
        duration
    };
    let backdrop = stage.explicit_backdrop.map(|color| color.to_hex());
    let scene_data = json!({
        "version": 1,
        "generator": "Fanta",
        "name": stage.clip_name,
        "width": stage.size[0],
        "height": stage.size[1],
        "duration": duration,
        "fps": stage.fps,
        "loop": loop_name(stage.loop_mode),
        "backdrop": backdrop,
        "background": { "src": "assets/background.png", "box": background_box },
        "layers": layers,
    });

    let spec = fanta_doc::MotionSpec::from_clip(doc, clip)?;
    let scene_js = format!(
        "// scene.js: the animation \"{}\" exported by Fanta. Times are seconds, positions are\n// pixels in each layer's parent frame, rotation is degrees and scale is a multiplier.\n// Each keyframe's easing shapes the segment that starts at it.\nconst FANTA_SCENE = {};\n\n{SCENE_RUNTIME_JS}",
        js_comment_text(&stage.clip_name),
        serde_json::to_string_pretty(&scene_data)?
    );
    let index_html = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{title}</title>\n<style>html, body {{ margin: 0; height: 100%; background: #0a0a0a; }} #root {{ position: fixed; inset: 0; }}</style>\n{REACT_SCRIPTS}\n<script src=\"scene.js\"></script>\n<script type=\"text/babel\" src=\"animations.jsx\"></script>\n<script type=\"text/babel\" src=\"Scene.jsx\"></script>\n</head>\n<body>\n<div id=\"root\"></div>\n<script type=\"text/babel\">\nReactDOM.createRoot(document.getElementById('root')).render(\n  <Stage width={{{width}}} height={{{height}}} duration={{{duration}}} fps={{{fps}}} background=\"{background}\" loop={{{looping}}}>\n    <Scene />\n  </Stage>\n);\n</script>\n</body>\n</html>\n",
        title = html_escape(&stage.clip_name),
        width = number(stage.size[0]),
        height = number(stage.size[1]),
        duration = number(stage_duration),
        fps = stage.fps,
        background = backdrop.as_deref().unwrap_or("transparent"),
        looping = stage.loop_mode != LoopMode::Once,
    );

    let readme = readme(stage, zoom, &sprites, &notes);
    entries.extend([
        ExportEntry {
            path: "README.md".to_string(),
            bytes: readme.into_bytes(),
        },
        ExportEntry {
            path: "index.html".to_string(),
            bytes: index_html.into_bytes(),
        },
        ExportEntry {
            path: "preview.html".to_string(),
            bytes: PREVIEW_HTML.as_bytes().to_vec(),
        },
        ExportEntry {
            path: "animations.jsx".to_string(),
            bytes: ANIMATIONS_JSX.as_bytes().to_vec(),
        },
        ExportEntry {
            path: "Scene.jsx".to_string(),
            bytes: SCENE_JSX.as_bytes().to_vec(),
        },
        ExportEntry {
            path: "scene.js".to_string(),
            bytes: scene_js.into_bytes(),
        },
        ExportEntry {
            path: "motion.json".to_string(),
            bytes: serde_json::to_vec_pretty(&spec)?,
        },
    ]);

    let summary = match notes.len() {
        0 => Vec::new(),
        1 => vec!["README.md lists 1 fidelity note.".to_string()],
        count => vec![format!("README.md lists {count} fidelity notes.")],
    };
    Ok(ExportArtifact {
        payload: ExportPayload::Directory(entries),
        extension: None,
        notes: summary,
    })
}

struct LayerContext<'a> {
    stage: &'a MotionStage,
    clip: &'a AnimationClip,
    sprites: &'a [Sprite],
    sprite_index: &'a HashMap<NodeId, usize>,
    tracks_by_node: &'a BTreeMap<NodeId, Vec<&'a AnimationTrack>>,
    resolver: Option<Arc<dyn AssetResolver>>,
    zoom: f64,
}

fn layer_entry(
    context: &LayerContext<'_>,
    index: usize,
    sprite: &Sprite,
    notes: &mut Vec<String>,
) -> Result<(Value, Option<ExportEntry>)> {
    let stage = context.stage;
    let doc = &stage.doc;
    let scene = &doc.scene;
    let node = scene
        .get(sprite.node)
        .context("an animated layer disappeared")?;
    let label = node_label(doc, sprite.node);
    let parent_node = node
        .parent
        .context("an animated layer is not inside the stage")?;
    let frame = scene
        .world_transform(parent_node)
        .context("an animated layer's frame has no placement")?;

    let ancestors = ancestors_below(scene, sprite.node, stage.stage);
    let dom_parent = sprite
        .parent
        .and_then(|parent| context.sprites.get(parent))
        .map(|parent| parent.node);
    let static_ancestors = ancestors
        .iter()
        .take_while(|ancestor| Some(**ancestor) != dom_parent)
        .copied()
        .collect::<Vec<_>>();
    // Opacity of the frames between this layer and its animated parent; the
    // stage's own opacity reaches top-level layers the same way.
    let mut inherited_opacity = static_ancestors
        .iter()
        .filter_map(|ancestor| scene.get(*ancestor))
        .map(|ancestor| f64::from(ancestor.opacity.get()))
        .product::<f64>();
    if dom_parent.is_none()
        && let Some(stage_node) = scene.get(stage.stage)
    {
        inherited_opacity *= f64::from(stage_node.opacity.get());
    }
    if static_ancestors.iter().any(|ancestor| {
        scene.get(*ancestor).is_some_and(|ancestor| {
            ancestor.blend_mode != BlendMode::Normal
                || !ancestor.effects.is_empty()
                || !ancestor.blurs.is_empty()
        })
    }) {
        notes.push(format!(
            "{label} sits in a group with its own blend mode or effects, which the package applies to the frame at rest only."
        ));
    }

    let mut clips = Vec::new();
    for ancestor in ancestors.iter().copied() {
        if let Some(path) = clip_path(scene, ancestor) {
            clips.push(path);
        }
        if Some(ancestor) == dom_parent {
            break;
        }
    }
    clips.reverse();

    if node.is_mask {
        notes.push(format!(
            "{label} is a mask, which the package draws as an ordinary layer."
        ));
    }
    if let Some(mask) = std::iter::once(sprite.node)
        .chain(static_ancestors.iter().copied())
        .find_map(|candidate| masked_by(scene, candidate))
    {
        notes.push(format!(
            "{label} is masked by {}, which the package does not apply.",
            node_label(doc, mask)
        ));
    }
    if node
        .blurs
        .iter()
        .any(|blur| blur.kind == BlurKind::Background)
    {
        notes.push(format!(
            "{label} has a background blur, which needs the live backdrop and is left out."
        ));
    }
    let blend = match css_blend(node.blend_mode) {
        Some(blend) => blend,
        None => {
            let mode = crate::properties_snapshot::BLEND_MODES
                .iter()
                .find_map(|(mode, name)| (*mode == node.blend_mode).then_some(*name))
                .unwrap_or("a blend mode");
            notes.push(format!(
                "{label} uses {mode}, which CSS cannot blend; it is drawn normally."
            ));
            None
        }
    };

    let tracks = context
        .tracks_by_node
        .get(&sprite.node)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let animates_transform = tracks
        .iter()
        .any(|track| is_transform_channel(track.target.property));
    let rest = MotionTransform::decompose(node.transform);
    if animates_transform && rest.is_none() {
        notes.push(format!(
            "{label} is skewed, so its position, rotation and scale tracks do not apply; Fanta ignores them too."
        ));
    }
    let pivot = context
        .clip
        .settings
        .transform_origins
        .get(&sprite.node)
        .map(|origin| {
            node.data.local_bounds().map_or([0.0, 0.0], |bounds| {
                [
                    bounds.min_x + origin[0] * bounds.width(),
                    bounds.min_y + origin[1] * bounds.height(),
                ]
            })
        });
    let mut channels = serde_json::Map::new();
    for track in tracks {
        if let Some(channel) = sprite_channel(track.target.property) {
            channels.insert(channel.to_string(), track_json(track, channel));
        }
    }

    let hidden = context
        .sprite_index
        .keys()
        .copied()
        .filter(|other| *other != sprite.node && is_descendant(doc, *other, sprite.node))
        .collect::<Vec<_>>();
    let sprite_doc = sprite_document(stage, sprite.node, &hidden)?;
    // A group whose content is all animated paints nothing itself; its element
    // still carries its motion to the layers nested in it.
    let image = match visual_world_bounds(&sprite_doc.scene, sprite.node, 0.0) {
        Some(region) => {
            let (png, image_box) = render_region(
                &sprite_doc,
                sprite.node,
                region,
                context.resolver.clone(),
                context.zoom,
                &label,
            )?;
            let path = format!("assets/layer-{:02}-{}.png", index + 1, slug(&node.name));
            Some((image_box, ExportEntry { path, bytes: png }))
        }
        None => None,
    };

    let layer = json!({
        "id": sprite.node.to_string(),
        "name": node.name,
        "parent": sprite.parent,
        "image": image
            .as_ref()
            .map(|(image_box, entry)| json!({ "src": entry.path, "box": image_box })),
        "frame": frame.to_components(),
        "restMatrix": node.transform.to_components(),
        "rest": rest.map(|rest| json!({
            "x": rest.position[0],
            "y": rest.position[1],
            "rotation": rest.rotation_radians.to_degrees(),
            "scaleX": rest.scale[0],
            "scaleY": rest.scale[1],
        })),
        "pivot": pivot,
        "opacity": f64::from(node.opacity.get()),
        "inheritedOpacity": inherited_opacity,
        "visible": !node.flags.contains(NodeFlags::HIDDEN),
        "blend": blend,
        "clips": clips,
        "tracks": channels,
    });
    Ok((layer, image.map(|(_, entry)| entry)))
}

/// The animated layer alone at its rest placement in stage space: its nested
/// animated layers are hidden, and opacity and blending are left to the player.
fn sprite_document(stage: &MotionStage, layer: NodeId, hidden: &[NodeId]) -> Result<Doc> {
    let source = &stage.doc;
    let world = source
        .scene
        .world_transform(layer)
        .context("an animated layer has no placement")?;
    let mut doc = source.clone();
    doc.active_modes = source
        .variables
        .collections
        .values()
        .map(|collection| {
            (
                collection.id,
                resolve_effective_mode(&source.scene, layer, collection, &source.active_modes),
            )
        })
        .collect();
    for id in hidden {
        if let Some(node) = doc.scene.get_mut(*id) {
            node.flags.insert(NodeFlags::HIDDEN);
        }
    }
    let index = doc.scene.next_child_index(None);
    doc.scene
        .set_parent(layer, None, index)
        .context("detaching an animated layer")?;
    let node = doc
        .scene
        .get_mut(layer)
        .context("a detached animated layer disappeared")?;
    node.transform = world;
    node.opacity = UnitInterval::ONE;
    node.blend_mode = BlendMode::Normal;
    node.flags.remove(NodeFlags::HIDDEN);
    Ok(doc)
}

/// Render `root` over `region` (stage coordinates) at `zoom`; the returned box
/// is the stage rectangle the image's whole pixels cover.
fn render_region(
    doc: &Doc,
    root: NodeId,
    region: Bounds,
    resolver: Option<Arc<dyn AssetResolver>>,
    zoom: f64,
    name: &str,
) -> Result<(Vec<u8>, [f64; 4])> {
    let (width, height) = raster_dimensions(region, ExportSizing::Scale(zoom), name)?;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow!("creating a {width}x{height} surface for {name}: {error}"))?;
    if let Some(resolver) = resolver {
        renderer.set_asset_resolver(resolver);
    }
    let span = [f64::from(width) / zoom, f64::from(height) / zoom];
    let viewport = Viewport {
        center: [region.min_x + span[0] / 2.0, region.min_y + span[1] / 2.0],
        zoom,
    };
    renderer.render_page_with(&doc.scene, &viewport, Some(root), &render_inputs(doc));
    let png = renderer
        .encode_png()
        .map_err(|error| anyhow!("encoding {name}: {error}"))?;
    Ok((png, [region.min_x, region.min_y, span[0], span[1]]))
}

/// Keyframes in evaluation order: Fanta keeps the last keyframe at a repeated
/// time and ignores values of the wrong type.
fn canonical_keyframes(track: &AnimationTrack) -> Vec<&Keyframe> {
    let mut keyframes = track
        .keyframes
        .values()
        .filter(|keyframe| track.target.property.accepts_value(&keyframe.value))
        .collect::<Vec<_>>();
    keyframes.sort_by_key(|keyframe| (keyframe.time_ms, keyframe.id));
    let mut canonical = Vec::<&Keyframe>::with_capacity(keyframes.len());
    for keyframe in keyframes {
        match canonical.last_mut() {
            Some(previous) if previous.time_ms == keyframe.time_ms => *previous = keyframe,
            _ => canonical.push(keyframe),
        }
    }
    canonical
}

fn track_json(track: &AnimationTrack, channel: &str) -> Value {
    let keyframes = canonical_keyframes(track);
    let values = keyframes
        .iter()
        .map(|keyframe| match &keyframe.value {
            ResolvedVarValue::Float { value } if channel == "rotation" => {
                json!(value.to_degrees())
            }
            ResolvedVarValue::Float { value } => json!(value),
            ResolvedVarValue::Boolean { value } => json!(value),
            _ => Value::Null,
        })
        .collect::<Vec<_>>();
    // Booleans never interpolate, so every visibility segment holds.
    let easing = keyframes
        .iter()
        .map(|keyframe| {
            if channel == "visible" || keyframe.interpolation == Interpolation::Hold {
                json!("hold")
            } else {
                easing_json(keyframe.easing)
            }
        })
        .collect::<Vec<_>>();
    json!({
        "times": keyframes.iter().map(|keyframe| seconds(keyframe.time_ms)).collect::<Vec<_>>(),
        "values": values,
        "easing": easing,
    })
}

fn easing_json(easing: Easing) -> Value {
    match easing {
        Easing::Linear => json!("linear"),
        Easing::EaseIn => json!("easeIn"),
        Easing::EaseOut => json!("easeOut"),
        Easing::EaseInOut => json!("easeInOut"),
        Easing::CubicBezier { x1, y1, x2, y2 } => json!({ "cubicBezier": [x1, y1, x2, y2] }),
        Easing::Spring {
            mass,
            stiffness,
            damping,
        } => json!({ "spring": [mass, stiffness, damping] }),
    }
}

/// The clip path a frame imposes on its children, as SVG path data in stage
/// coordinates at rest, or `None` when it does not clip.
fn clip_path(scene: &Scene, frame: NodeId) -> Option<String> {
    let node = scene.get(frame)?;
    let NodeData::Group(group) = &node.data else {
        return None;
    };
    let [width, height] = group.clip_size?;
    let clips_children = node
        .meta
        .get("clip_content")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !clips_children {
        return None;
    }
    let transform = scene.world_transform(frame)?;
    let radii = group
        .corner_radii
        .unwrap_or([group.corner_radius.unwrap_or(0.0); 4]);
    Some(rounded_rect_path(width, height, radii, &transform))
}

/// `radii` are `[top-left, top-right, bottom-right, bottom-left]`, scaled down
/// together when adjacent corners would overlap, as CSS does.
fn rounded_rect_path(width: f64, height: f64, radii: [f64; 4], transform: &Transform2D) -> String {
    let radii = radii.map(|radius| {
        if radius.is_finite() {
            radius.max(0.0)
        } else {
            0.0
        }
    });
    let [top_left, top_right, bottom_right, bottom_left] = radii;
    let factor = [
        width / (top_left + top_right),
        width / (bottom_left + bottom_right),
        height / (top_left + bottom_left),
        height / (top_right + bottom_right),
    ]
    .into_iter()
    .filter(|factor| factor.is_finite())
    .fold(1.0_f64, f64::min);
    let [top_left, top_right, bottom_right, bottom_left] = radii.map(|radius| radius * factor);
    let point = |x: f64, y: f64| {
        let point = transform.transform_point(DVec2::new(x, y));
        format!("{} {}", number(point.x), number(point.y))
    };
    let curve = |path: &mut String, from: (f64, f64), to: (f64, f64), end: (f64, f64)| {
        path.push_str(&format!(
            " C {} {} {}",
            point(from.0, from.1),
            point(to.0, to.1),
            point(end.0, end.1)
        ));
    };
    let mut path = format!("M {}", point(top_left, 0.0));
    path.push_str(&format!(" L {}", point(width - top_right, 0.0)));
    if top_right > 0.0 {
        curve(
            &mut path,
            (width - top_right + top_right * KAPPA, 0.0),
            (width, top_right - top_right * KAPPA),
            (width, top_right),
        );
    }
    path.push_str(&format!(" L {}", point(width, height - bottom_right)));
    if bottom_right > 0.0 {
        curve(
            &mut path,
            (width, height - bottom_right + bottom_right * KAPPA),
            (width - bottom_right + bottom_right * KAPPA, height),
            (width - bottom_right, height),
        );
    }
    path.push_str(&format!(" L {}", point(bottom_left, height)));
    if bottom_left > 0.0 {
        curve(
            &mut path,
            (bottom_left - bottom_left * KAPPA, height),
            (0.0, height - bottom_left + bottom_left * KAPPA),
            (0.0, height - bottom_left),
        );
    }
    path.push_str(&format!(" L {}", point(0.0, top_left)));
    if top_left > 0.0 {
        curve(
            &mut path,
            (0.0, top_left - top_left * KAPPA),
            (top_left - top_left * KAPPA, 0.0),
            (top_left, 0.0),
        );
    }
    path.push_str(" Z");
    path
}

/// Static layers that Fanta paints above an animated layer, overlap its
/// motion, and end up beneath it because the package draws every animated
/// layer over the frame image that contains them.
fn coverage_notes(
    stage: &MotionStage,
    order: &[NodeId],
    position: &HashMap<NodeId, usize>,
    sprite_index: &HashMap<NodeId, usize>,
) -> Result<Vec<String>> {
    let doc = &stage.doc;
    let scene = &doc.scene;
    if sprite_index.is_empty() {
        return Ok(Vec::new());
    }
    let times = sample_times(stage.duration_ms, stage.fps, LoopMode::Once)?;
    let stride = times.len().div_ceil(COVERAGE_SAMPLES).max(1);
    let mut swept = HashMap::<NodeId, Bounds>::new();
    for time in times.iter().step_by(stride) {
        let evaluation = stage.evaluate(*time)?;
        let animated = evaluation
            .overrides
            .keys()
            .map(|target| target.node)
            .collect::<HashSet<_>>();
        let mut evaluated = scene.clone();
        for node in animated {
            if let Some(original) = scene.get(node) {
                let moved = evaluation.apply_to_node(original);
                if let Some(slot) = evaluated.get_mut(node) {
                    *slot = moved;
                }
            }
        }
        for node in sprite_index.keys() {
            if let Some(bounds) = visual_world_bounds(&evaluated, *node, 0.0) {
                swept
                    .entry(*node)
                    .and_modify(|existing| *existing = existing.union(&bounds))
                    .or_insert(bounds);
            }
        }
    }

    let container = |node: NodeId| {
        ancestors_below(scene, node, stage.stage)
            .into_iter()
            .find(|ancestor| sprite_index.contains_key(ancestor))
    };
    let mut sprites = sprite_index.keys().copied().collect::<Vec<_>>();
    sprites.sort_by_key(|node| position.get(node).copied().unwrap_or(usize::MAX));
    let mut notes = Vec::new();
    for sprite in sprites {
        let (Some(sprite_position), Some(sweep)) = (position.get(&sprite), swept.get(&sprite))
        else {
            continue;
        };
        let covering = order
            .iter()
            .copied()
            .filter(|node| {
                position
                    .get(node)
                    .is_some_and(|node_position| node_position > sprite_position)
                    && !sprite_index.contains_key(node)
                    && paints_itself(scene, *node)
                    && !is_hidden(scene, *node)
                    && !ancestors_below(scene, *node, stage.stage)
                        .iter()
                        .any(|ancestor| is_hidden(scene, *ancestor))
                    && container(*node).is_none_or(|container| {
                        position
                            .get(&container)
                            .is_some_and(|container_position| container_position < sprite_position)
                    })
                    && visual_world_bounds(scene, *node, 0.0)
                        .is_some_and(|bounds| bounds.intersects(sweep))
            })
            .map(|node| node_label(doc, node))
            .collect::<Vec<_>>();
        if let [first, rest @ ..] = covering.as_slice() {
            let more = if rest.is_empty() {
                String::new()
            } else {
                format!(" and {} more", rest.len())
            };
            notes.push(format!(
                "{first}{more} paints above {} in Fanta but below it here; the MP4, GIF and PNG frame exports keep Fanta's order.",
                node_label(doc, sprite)
            ));
        }
    }
    Ok(notes)
}

fn readme(stage: &MotionStage, zoom: f64, sprites: &[Sprite], notes: &[String]) -> String {
    let layers = if sprites.is_empty() {
        "- None: no layer moves, fades or toggles visibility in this clip.".to_string()
    } else {
        sprites
            .iter()
            .map(|sprite| format!("- {}", node_label(&stage.doc, sprite.node)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let notes = if notes.is_empty() {
        "- None: every animated property in this clip is represented.".to_string()
    } else {
        notes
            .iter()
            .map(|note| format!("- {note}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"# {name}

A Stage project of the Fanta animation **{name}**: {duration} s at {fps} fps, {loop_mode}, {width} × {height}.

| File | Contents |
| --- | --- |
| `index.html` | The Stage project. It loads React, ReactDOM and Babel from unpkg, then `scene.js`, `animations.jsx` and `Scene.jsx`. Serve this folder over HTTP to open it, for example with `python3 -m http.server`. |
| `preview.html` | A self-contained player with no network or build step. Open it straight from disk. |
| `animations.jsx` | Fanta's implementation of the Stage starter API: `Stage`, `Sprite`, `useTime`, `useTimeline`, `useSprite`, `interpolate`, `animate`, `clamp` and `Easing`, published on `window`. |
| `Scene.jsx` | The animation as a Stage scene. It reads only the playhead from `useTime()`. |
| `scene.js` | The motion data (layers, keyframes, stage geometry) and Fanta's keyframe evaluator, shared by both players. Edit keyframes here. |
| `motion.json` | The clip in Fanta's portable motion format. |
| `assets/` | `background.png` (the frame at rest without its animated layers) and a PNG of each animated layer that paints something itself, rendered by Fanta at {zoom}x. |

## Animated layers

{layers}

## Exporting a video

- Exporters that drive Stage projects open `index.html?__render=1`. The page then draws the bare stage at the window origin and sets `window.__videoMeta = {{ width, height, duration, fps }}`, `window.__seek(seconds)` and `window.__ready`. `preview.html?__render=1` offers the same contract offline.
- [claude-video-export](https://github.com/dawoodtrumboo/claude-video-export) turns this folder into an MP4: `node cli.mjs "<this folder>"`.
- Fanta's Design menu also exports this clip as MP4, GIF or PNG frames with its own renderer.

## Assumptions

- `animations.jsx` follows the API of the Claude Design `animations.jsx` Stage starter as published by claude-video-export. `Scene.jsx` uses only React and `useTime()`, so replacing `animations.jsx` with Claude's own starter keeps the scene working.
- Each animated layer is a bitmap of the layer at rest. Its position, rotation, scale, opacity and visibility follow the keyframes. Other animated properties are shown at rest; the MP4, GIF and PNG frame exports render them exactly.
- Playheads are rounded to whole milliseconds, as Fanta evaluates them.

## Notes from this export

{notes}
"#,
        name = stage.clip_name,
        duration = number(seconds(stage.duration_ms)),
        fps = stage.fps,
        loop_mode = match stage.loop_mode {
            LoopMode::Once => "playing once",
            LoopMode::Repeat => "looping",
            LoopMode::PingPong => "playing forward then backward",
        },
        width = number(stage.size[0]),
        height = number(stage.size[1]),
        zoom = number(zoom),
    )
}

fn seconds(milliseconds: u32) -> f64 {
    f64::from(milliseconds) / 1000.0
}

/// Shortest readable decimal, at most four places.
fn number(value: f64) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    let text = format!("{rounded:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" || text.is_empty() {
        "0".to_string()
    } else {
        text.to_string()
    }
}

fn slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "layer".to_string()
    } else {
        slug.chars().take(40).collect()
    }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn js_comment_text(text: &str) -> String {
    text.replace(['\r', '\n', '\u{2028}', '\u{2029}', '"'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::ExportFormat;
    use crate::motion_video::export_motion_media;
    use crate::motion_video::tests::{
        add_clip, animated_stage, directory_entries, entry, insert, layer, square, stage_frame,
    };
    use fanta_doc::{Blur, Color, MotionSpec};
    use std::path::PathBuf;

    fn package(doc: &Doc, root: Option<NodeId>) -> ExportArtifact {
        export_motion_media(
            doc,
            None,
            root,
            ExportFormat::MotionStage,
            ExportSizing::Scale(1.0),
        )
        .expect("Stage package export")
    }

    fn text(entries: &[ExportEntry], path: &str) -> String {
        String::from_utf8(entry(entries, path).to_vec()).expect("text file")
    }

    #[test]
    fn stage_package_is_a_claude_motion_stage_project() {
        let fixture = animated_stage();
        let artifact = package(&fixture.doc, Some(fixture.frame));
        let entries = directory_entries(&artifact);
        let mut paths = entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        paths.sort_unstable();
        // The holder paints nothing itself, so it has no image.
        assert_eq!(
            paths,
            [
                "README.md",
                "Scene.jsx",
                "animations.jsx",
                "assets/background.png",
                "assets/layer-01-card.png",
                "assets/layer-03-dot.png",
                "index.html",
                "motion.json",
                "preview.html",
                "scene.js",
            ]
        );

        // claude-video-export picks the HTML file with `<Stage` and an
        // `animations.jsx` reference, so the offline player must have neither.
        let index = text(entries, "index.html");
        assert!(index.contains(
            r#"<Stage width={200} height={100} duration={1} fps={10} background="transparent" loop={false}>"#
        ));
        assert!(index.contains(r#"<script type="text/babel" src="animations.jsx"></script>"#));
        assert!(index.contains("integrity=\"sha384-"));
        let preview = text(entries, "preview.html");
        assert!(!preview.contains("<Stage") && !preview.contains("animations.jsx"));
        for page in [text(entries, "animations.jsx"), preview] {
            for hook in [
                "__videoMeta",
                "__seek",
                "__ready",
                "__render",
                "data-export-hide",
            ] {
                assert!(page.contains(hook), "a player is missing {hook}");
            }
        }
        for api in [
            "Stage",
            "Sprite",
            "useTime",
            "useTimeline",
            "useSprite",
            "interpolate",
            "animate",
            "clamp",
            "Easing",
        ] {
            assert!(text(entries, "animations.jsx").contains(&format!("  {api},")));
        }

        let spec: MotionSpec = serde_json::from_slice(entry(entries, "motion.json"))
            .expect("motion.json is a motion spec");
        assert_eq!(spec.layers.len(), 3);
        let scene = text(entries, "scene.js");
        assert!(scene.starts_with("// scene.js: the animation \"Hero\""));
        let data = scene
            .split_once("const FANTA_SCENE = ")
            .and_then(|(_, rest)| rest.split_once(";\n\n"))
            .map(|(data, _)| data)
            .expect("scene data");
        let data: Value = serde_json::from_str(data).expect("scene data is JSON");
        let layers = data["layers"].as_array().expect("layers");
        assert_eq!(
            layers
                .iter()
                .map(|layer| layer["name"].clone())
                .collect::<Vec<_>>(),
            [json!("Card"), json!("Holder"), json!("Dot")]
        );
        assert!((layers[0]["inheritedOpacity"].as_f64().expect("opacity") - 0.8).abs() < 1e-6);
        assert_eq!(layers[1]["image"], Value::Null);
        assert_eq!(
            layers[2]["parent"],
            json!(1),
            "the dot moves inside the holder"
        );
        let degrees = layers[0]["tracks"]["rotation"]["values"]
            .as_array()
            .expect("rotation keyframes")
            .iter()
            .map(|value| value.as_f64().expect("degrees"))
            .collect::<Vec<_>>();
        assert!(degrees.len() == 2 && degrees[0].abs() < 1e-9 && (degrees[1] - 45.0).abs() < 1e-9);
        assert_eq!(
            layers[2]["tracks"]["visible"]["easing"],
            json!(["hold", "hold"])
        );
        assert!(artifact.notes.is_empty(), "{:?}", artifact.notes);
        assert!(text(entries, "README.md").contains("- None: every animated property"));
    }

    fn find_node() -> Option<PathBuf> {
        let name = if cfg!(windows) { "node.exe" } else { "node" };
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    }

    fn multiply(left: [f64; 6], right: [f64; 6]) -> [f64; 6] {
        Transform2D::from_components(right)
            .then(&Transform2D::from_components(left))
            .to_components()
    }

    /// Runs `scene.js` in Node and compares every layer's composed element
    /// transform, opacity and visibility with Fanta's own evaluation.
    #[test]
    fn stage_package_plays_the_clip_exactly_as_fanta_does() {
        let Some(node) = find_node() else {
            eprintln!("skipping the Stage player parity check: node is not on PATH");
            return;
        };
        let fixture = animated_stage();
        let artifact = package(&fixture.doc, Some(fixture.frame));
        let directory = tempfile::tempdir().expect("temporary folder");
        std::fs::write(
            directory.path().join("scene.js"),
            entry(directory_entries(&artifact), "scene.js"),
        )
        .expect("write scene.js");
        std::fs::write(
            directory.path().join("probe.js"),
            "const fs = require('fs');\nconst vm = require('vm');\nvm.runInThisContext(fs.readFileSync(process.argv[2], 'utf8'));\nconst times = JSON.parse(process.argv[3]);\nprocess.stdout.write(JSON.stringify({\n  ids: FantaScene.scene.layers.map((layer) => layer.id),\n  parents: FantaScene.scene.layers.map((layer) => layer.parent),\n  states: times.map((time) => FantaScene.stateAt(time)),\n}));\n",
        )
        .expect("write probe.js");
        let times = (0..=40).map(|step| step * 25).collect::<Vec<u32>>();
        let seconds = times
            .iter()
            .map(|time| f64::from(*time) / 1000.0)
            .collect::<Vec<_>>();
        let output = futures::executor::block_on(
            util::command::new_command(&node)
                .arg(directory.path().join("probe.js"))
                .arg(directory.path().join("scene.js"))
                .arg(serde_json::to_string(&seconds).expect("times"))
                .output(),
        )
        .expect("run node");
        assert!(
            output.status.success(),
            "node failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let probe: Value = serde_json::from_slice(&output.stdout).expect("probe output");
        let ids = probe["ids"]
            .as_array()
            .expect("ids")
            .iter()
            .map(|id| {
                id.as_str()
                    .and_then(|id| id.parse::<NodeId>().ok())
                    .expect("node id")
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, [fixture.card, fixture.holder, fixture.dot]);
        let parents = probe["parents"]
            .as_array()
            .expect("parents")
            .iter()
            .map(|parent| parent.as_u64().map(|parent| parent as usize))
            .collect::<Vec<_>>();

        let stage = MotionStage::prepare(&fixture.doc, Some(fixture.frame)).expect("stage");
        let scene = &stage.doc.scene;
        for (sample, time) in times.iter().enumerate() {
            let evaluation = stage.evaluate(*time).expect("evaluate");
            let mut evaluated = scene.clone();
            for target in evaluation.overrides.keys() {
                if let Some(original) = scene.get(target.node) {
                    let moved = evaluation.apply_to_node(original);
                    if let Some(slot) = evaluated.get_mut(target.node) {
                        *slot = moved;
                    }
                }
            }
            let states = probe["states"][sample].as_array().expect("states");
            for (index, id) in ids.iter().enumerate() {
                let mut composed = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                let mut opacity = 1.0;
                let mut visible = true;
                let mut current = Some(index);
                while let Some(layer) = current {
                    let state = &states[layer];
                    if let Some(matrix) = state["matrix"].as_array() {
                        let matrix: [f64; 6] = std::array::from_fn(|component| {
                            matrix[component].as_f64().expect("matrix component")
                        });
                        composed = multiply(matrix, composed);
                    }
                    opacity *= state["opacity"].as_f64().expect("opacity");
                    visible &= state["visible"].as_bool().expect("visible");
                    current = parents[layer];
                }

                let rest = scene.world_transform(*id).expect("rest placement");
                let moved = evaluated.world_transform(*id).expect("placement");
                let expected = rest.inverse().then(&moved).to_components();
                for (actual, expected) in composed.iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 1e-6,
                        "layer {index} at {time} ms: {composed:?} != {expected:?}"
                    );
                }
                let chain = std::iter::once(*id)
                    .chain(evaluated.ancestors_of(*id).map(|ancestor| ancestor.id))
                    .filter_map(|node| evaluated.get(node))
                    .collect::<Vec<_>>();
                let expected_opacity = chain
                    .iter()
                    .map(|node| f64::from(node.opacity.get()))
                    .product::<f64>();
                assert!(
                    (opacity - expected_opacity).abs() < 1e-6,
                    "layer {index} at {time} ms: opacity {opacity} != {expected_opacity}"
                );
                let expected_visible = chain
                    .iter()
                    .all(|node| !node.flags.contains(NodeFlags::HIDDEN));
                assert_eq!(visible, expected_visible, "layer {index} at {time} ms");
            }
        }
    }

    #[test]
    fn stage_package_lists_what_it_cannot_represent() {
        let mut doc = Doc::new();
        let frame = stage_frame(&mut doc);
        let tint = insert(
            &mut doc,
            layer("Tint", frame, square(20.0, Color::WHITE), 0.0, 0.0),
        );
        let mover = insert(
            &mut doc,
            layer("Mover", frame, square(20.0, Color::BLACK), 30.0, 10.0),
        );
        insert(
            &mut doc,
            layer("Lid", frame, square(60.0, Color::WHITE), 60.0, 0.0),
        );
        let mut mask = layer("Mask", frame, square(30.0, Color::BLACK), 0.0, 60.0);
        mask.is_mask = true;
        insert(&mut doc, mask);
        let masked = insert(
            &mut doc,
            layer("Masked", frame, square(30.0, Color::BLACK), 0.0, 60.0),
        );
        let mut burn = layer("Burn", frame, square(10.0, Color::BLACK), 150.0, 60.0);
        burn.blend_mode = BlendMode::LinearBurn;
        let burn = insert(&mut doc, burn);
        let mut glass = layer("Glass", frame, square(10.0, Color::WHITE), 170.0, 60.0);
        glass.blurs.push(Blur::background(8.0));
        let glass = insert(&mut doc, glass);
        add_clip(
            &mut doc,
            json!({
                "version": 1, "name": "Notes", "duration": 1.0, "fps": 10,
                "layers": [
                    {"node": frame.to_string(), "tracks": {"translation_x": [{"t": 0.0, "v": 40.0}, {"t": 1.0, "v": 80.0}]}},
                    {"node": tint.to_string(), "tracks": {"fill_color": [{"t": 0.0, "v": "#FF0000"}, {"t": 1.0, "v": "#0000FF"}]}},
                    {"node": mover.to_string(), "tracks": {"translation_x": [{"t": 0.0, "v": 30.0}, {"t": 1.0, "v": 90.0}]}},
                    {"node": masked.to_string(), "tracks": {"opacity": [{"t": 0.0, "v": 1.0}, {"t": 1.0, "v": 0.0}]}},
                    {"node": burn.to_string(), "tracks": {"rotation": [{"t": 0.0, "v": 0.0}, {"t": 1.0, "v": 90.0}]}},
                    {"node": glass.to_string(), "tracks": {"opacity": [{"t": 0.0, "v": 1.0}, {"t": 1.0, "v": 0.5}]}}
                ]
            }),
        );

        let artifact = package(&doc, None);
        let readme = text(directory_entries(&artifact), "README.md");
        for expected in [
            "“Stage” is the stage, so its own position, rotation and scale tracks are not animated in exports.",
            "“Tint” animates its fill color, shown at rest here",
            "“Lid” paints above “Mover” in Fanta but below it here",
            "“Masked” is masked by “Mask”, which the package does not apply.",
            "“Burn” uses Linear Burn, which CSS cannot blend",
            "“Glass” has a background blur",
        ] {
            assert!(
                readme.contains(expected),
                "README.md is missing {expected:?}:\n{readme}"
            );
        }
        let count = readme
            .split_once("## Notes from this export")
            .map(|(_, notes)| notes.lines().filter(|line| line.starts_with("- ")).count())
            .expect("notes section");
        assert_eq!(
            artifact.notes.first(),
            Some(&format!("README.md lists {count} fidelity notes."))
        );
    }
}
