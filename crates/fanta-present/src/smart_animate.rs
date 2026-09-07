//! Smart-animate transitions for [`PresentSession`](crate::PresentSession).
//!
//! Unlike the raster-composite transitions in [`crate::transition`], smart
//! animate interpolates *matched layers* between the outgoing and incoming
//! frames: a layer present in both (matched by its name-path within the frame)
//! slides/scales/fades from its old position+opacity to its new one, so a card
//! that moves and resizes appears to animate rather than cross-dissolve.
//!
//! The document is immutable, so we build a **scratch scene** — a clone of the
//! destination frame's subtree, re-rooted at the frame's world transform — and
//! mutate *that* each tick. Matched layers tween their local transform + opacity
//! from the source frame's values to the destination's; layers that appear only
//! in the destination fade in at their final position. The scratch scene is then
//! rendered like any other page.
//!
//! Matching is by the path of layer *names* from the frame down to the node.
//! Duplicate sibling names collapse to the first match (a documented v1
//! limitation); unique names — the common authored case — match exactly.

use std::collections::HashMap;

use fanta_doc::{Color, Doc, Fill, NodeData, NodeId, Scene, Transform2D, UnitInterval};

/// A prepared smart-animate transition: the scratch scene plus the per-node
/// tweens applied as eased progress advances.
pub(crate) struct SmartAnimate {
    /// Scratch clone of the destination frame subtree (re-rooted at `root`).
    pub scene: Scene,
    /// The destination frame id — the render root of `scene`.
    pub root: NodeId,
    tweens: Vec<LayerTween>,
    root_background: Option<(Fill, Fill)>,
}

/// One layer's interpolation between its source and destination state.
struct LayerTween {
    node: NodeId,
    from_transform: [f64; 6],
    to_transform: [f64; 6],
    from_opacity: f32,
    to_opacity: f32,
    // for groups with clip (resize in motion)
    from_clip: Option<[f64; 2]>,
    to_clip: Option<[f64; 2]>,
    // explicit non-clipping group box (resize in motion without becoming a frame)
    from_local_size: Option<[f64; 2]>,
    to_local_size: Option<[f64; 2]>,
    // for rounded groups (common in motion) — uniform + per-corner + smoothing
    from_corner: Option<f64>,
    to_corner: Option<f64>,
    from_corners: Option<[f64; 4]>,
    to_corners: Option<[f64; 4]>,
    from_smoothing: Option<f32>,
    to_smoothing: Option<f32>,
}

/// Build a smart-animate plan tweening `from_frame` → `to_frame`. `None` if the
/// destination frame is missing from the scene.
pub(crate) fn prepare(doc: &Doc, from_frame: NodeId, to_frame: NodeId) -> Option<SmartAnimate> {
    prepare_scenes(&doc.scene, from_frame, &doc.scene, to_frame)
}

pub(crate) fn prepare_scenes(
    from_scene: &Scene,
    from_frame: NodeId,
    to_scene: &Scene,
    to_frame: NodeId,
) -> Option<SmartAnimate> {
    if !to_scene.contains(to_frame) {
        return None;
    }

    let root_background = match (
        from_scene.get(from_frame).map(|node| &node.data),
        to_scene.get(to_frame).map(|node| &node.data),
    ) {
        (Some(NodeData::Group(from)), Some(NodeData::Group(to))) => from
            .background
            .as_ref()
            .zip(to.background.as_ref())
            .filter(|(from, to)| interpolated_solid_fill(from, to, 0.0).is_some())
            .map(|(from, to)| (from.clone(), to.clone())),
        _ => None,
    };

    // Source layers keyed by name-path (relative to the source frame). First
    // occurrence wins for duplicate keys.
    // Store extra geometry for richer Smart Animate motion fidelity (clip, corners).
    let mut from_by_key: HashMap<
        Vec<String>,
        (
            Transform2D,
            f32,
            Option<[f64; 2]>,
            Option<[f64; 2]>,
            Option<f64>,
            Option<[f64; 4]>,
            Option<f32>,
        ),
    > = HashMap::new();
    for id in from_scene.descendants_of(from_frame) {
        if let (Some(key), Some(node)) = (key_path(from_scene, from_frame, id), from_scene.get(id))
        {
            let (clip, local_size, corner, corners, smoothing) = match &node.data {
                NodeData::Group(g) => (
                    g.clip_size,
                    g.local_size,
                    g.corner_radius,
                    g.corner_radii,
                    Some(g.corner_smoothing),
                ),
                NodeData::Vector(v) => (
                    None,
                    None,
                    v.corner_radius,
                    v.corner_radii,
                    Some(v.corner_smoothing),
                ),
                _ => (None, None, None, None, None),
            };
            from_by_key.entry(key).or_insert((
                node.transform,
                node.opacity.get(),
                clip,
                local_size,
                corner,
                corners,
                smoothing,
            ));
        }
    }

    // Clone the full scene into scratch storage, then mutate only the
    // destination subtree. Keeping the rest of the scene is intentional:
    // component masters may live outside the animated frame, and instance
    // expansion resolves them through the same `Scene` it renders.
    let mut scene = to_scene.clone();
    let mut tweens = Vec::new();
    let mut matched_layers = 0usize;
    for id in to_scene.descendants_of(to_frame) {
        let key = key_path(to_scene, to_frame, id)?;

        if id == to_frame {
            // The frame is the shared origin: static, positioned at its real
            // world transform, never tweened.
            let node = scene.get_mut(id)?;
            node.parent = None;
            node.transform = to_scene
                .world_transform(to_frame)
                .unwrap_or(Transform2D::IDENTITY);
            continue;
        }

        let node = to_scene.get(id)?;
        let to_transform = node.transform.to_components();
        let to_opacity = node.opacity.get();
        let (to_clip, to_local_size, to_corner, to_corners, to_smoothing) = match &node.data {
            NodeData::Group(g) => (
                g.clip_size,
                g.local_size,
                g.corner_radius,
                g.corner_radii,
                Some(g.corner_smoothing),
            ),
            NodeData::Vector(v) => (
                None,
                None,
                v.corner_radius,
                v.corner_radii,
                Some(v.corner_smoothing),
            ),
            _ => (None, None, None, None, None),
        };
        let tween = match from_by_key.get(&key) {
            // Matched: tween from the source layer's transform + opacity + clip + corners (richer motion fidelity).
            Some((
                from_transform,
                from_opacity,
                from_clip,
                from_local_size,
                from_corner,
                from_corners,
                from_smoothing,
            )) => {
                matched_layers += 1;
                LayerTween {
                    node: id,
                    from_transform: from_transform.to_components(),
                    to_transform,
                    from_opacity: *from_opacity,
                    to_opacity,
                    from_clip: *from_clip,
                    to_clip,
                    from_local_size: *from_local_size,
                    to_local_size,
                    from_corner: *from_corner,
                    to_corner,
                    from_corners: *from_corners,
                    to_corners,
                    from_smoothing: *from_smoothing,
                    to_smoothing,
                }
            }
            // Destination-only: fade in at the final position.
            None => LayerTween {
                node: id,
                from_transform: to_transform,
                to_transform,
                from_opacity: 0.0,
                to_opacity,
                from_clip: None,
                to_clip,
                from_local_size: None,
                to_local_size,
                from_corner: None,
                to_corner,
                from_corners: None,
                to_corners,
                from_smoothing: None,
                to_smoothing: None,
            },
        };
        tweens.push(tween);
    }

    if matched_layers == 0 {
        return None;
    }

    Some(SmartAnimate {
        scene,
        root: to_frame,
        tweens,
        root_background,
    })
}

impl SmartAnimate {
    /// Set every tweened layer to its interpolated state at eased progress
    /// `e` (0 = source, 1 = destination).
    pub(crate) fn apply(&mut self, e: f64) {
        for tween in &self.tweens {
            if let Some(node) = self.scene.get_mut(tween.node) {
                node.transform = Transform2D::from_components(lerp6(
                    tween.from_transform,
                    tween.to_transform,
                    e,
                ));
                let opacity = lerp(
                    f64::from(tween.from_opacity),
                    f64::from(tween.to_opacity),
                    e,
                );
                node.opacity = UnitInterval::new(opacity as f32);
                // tween clip_size for groups (resize during smart animate motion)
                if let NodeData::Group(g) = &mut node.data {
                    if let (Some(from_c), Some(to_c)) = (tween.from_clip, tween.to_clip) {
                        let w = lerp(from_c[0], to_c[0], e);
                        let h = lerp(from_c[1], to_c[1], e);
                        g.clip_size = Some([w, h]);
                    }
                    if let (Some(from_size), Some(to_size)) =
                        (tween.from_local_size, tween.to_local_size)
                    {
                        g.local_size = Some([
                            lerp(from_size[0], to_size[0], e),
                            lerp(from_size[1], to_size[1], e),
                        ]);
                    }
                    // tween corner for rounded motion (uniform)
                    if let (Some(from_r), Some(to_r)) = (tween.from_corner, tween.to_corner) {
                        g.corner_radius = Some(lerp(from_r, to_r, e));
                    }
                    // per-corner radii (mixed rounded rects during motion)
                    if let (Some(from_cs), Some(to_cs)) = (tween.from_corners, tween.to_corners) {
                        g.corner_radii = Some([
                            lerp(from_cs[0], to_cs[0], e),
                            lerp(from_cs[1], to_cs[1], e),
                            lerp(from_cs[2], to_cs[2], e),
                            lerp(from_cs[3], to_cs[3], e),
                        ]);
                    }
                    // corner smoothing (Figma "corner radius" advanced)
                    if let (Some(from_sm), Some(to_sm)) = (tween.from_smoothing, tween.to_smoothing)
                    {
                        g.corner_smoothing = lerp(from_sm as f64, to_sm as f64, e) as f32;
                    }
                }
                // Also support tweening rounded corners on vectors during Smart Animate (motion fidelity)
                if let NodeData::Vector(v) = &mut node.data {
                    if let (Some(from_r), Some(to_r)) = (tween.from_corner, tween.to_corner) {
                        v.corner_radius = Some(lerp(from_r, to_r, e));
                    }
                    if let (Some(from_cs), Some(to_cs)) = (tween.from_corners, tween.to_corners) {
                        v.corner_radii = Some([
                            lerp(from_cs[0], to_cs[0], e),
                            lerp(from_cs[1], to_cs[1], e),
                            lerp(from_cs[2], to_cs[2], e),
                            lerp(from_cs[3], to_cs[3], e),
                        ]);
                    }
                    if let (Some(from_sm), Some(to_sm)) = (tween.from_smoothing, tween.to_smoothing)
                    {
                        v.corner_smoothing = lerp(from_sm as f64, to_sm as f64, e) as f32;
                    }
                }
            }
        }
        if let Some((from, to)) = &self.root_background
            && let Some(NodeData::Group(group)) =
                self.scene.get_mut(self.root).map(|node| &mut node.data)
            && let Some(background) = interpolated_solid_fill(from, to, e)
        {
            group.background = Some(background);
        }
    }
}

fn interpolated_solid_fill(from: &Fill, to: &Fill, progress: f64) -> Option<Fill> {
    let (
        Fill::Solid {
            color: from,
            blend: from_blend,
        },
        Fill::Solid {
            color: to,
            blend: to_blend,
        },
    ) = (from, to)
    else {
        return None;
    };
    if from_blend != to_blend {
        return None;
    }
    let channel = |from: u8, to: u8| {
        (f64::from(from) + (f64::from(to) - f64::from(from)) * progress)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Some(Fill::Solid {
        color: Color::rgba(
            channel(from.r, to.r),
            channel(from.g, to.g),
            channel(from.b, to.b),
            channel(from.a, to.a),
        ),
        blend: *from_blend,
    })
}

/// The path of layer names from `frame` (exclusive) down to `id`, or `None` if
/// `id` is not within `frame`'s subtree. `id == frame` yields the empty path.
fn key_path(scene: &Scene, frame: NodeId, id: NodeId) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut cursor = id;
    while cursor != frame {
        let node = scene.get(cursor)?;
        names.push(node.name.clone());
        cursor = node.parent?;
    }
    names.reverse();
    Some(names)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp6(a: [f64; 6], b: [f64; 6], t: f64) -> [f64; 6] {
    std::array::from_fn(|i| lerp(a[i], b[i], t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, Operation};

    #[test]
    fn smart_animate_tweens_non_clipping_group_boxes() {
        let mut doc = Doc::new();
        let from_frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([400.0, 300.0]),
            ..Default::default()
        }));
        let from_frame_id = from_frame.id;
        doc.apply(Operation::create_node(from_frame)).unwrap();
        let to_frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([400.0, 300.0]),
            ..Default::default()
        }));
        let to_frame_id = to_frame.id;
        doc.apply(Operation::create_node(to_frame)).unwrap();

        let mut from_group = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([100.0, 40.0]),
            ..Default::default()
        }));
        from_group.name = "Card".into();
        from_group.parent = Some(from_frame_id);
        doc.apply(Operation::create_node(from_group)).unwrap();
        let mut to_group = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([220.0, 120.0]),
            ..Default::default()
        }));
        to_group.name = "Card".into();
        to_group.parent = Some(to_frame_id);
        let to_group_id = to_group.id;
        doc.apply(Operation::create_node(to_group)).unwrap();

        let mut animation = prepare(&doc, from_frame_id, to_frame_id).expect("smart animate");
        animation.apply(0.5);
        let NodeData::Group(group) = &animation.scene.get(to_group_id).unwrap().data else {
            panic!("expected group");
        };
        assert_eq!(group.local_size, Some([160.0, 80.0]));
        assert_eq!(group.clip_size, None);
    }
}
