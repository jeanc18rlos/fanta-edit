use std::collections::{HashMap, HashSet};

use fanta_doc::{
    AssetId, BlendMode, CanvasNode, Fill, Gradient, NodeData, NodeFlags, NodeId, PathSegment,
    Scene, Stroke, TextStyle,
};

use crate::asset::{AssetResolver, DecodedImage};

use super::{BlurKind, RenderInputs, ShadowKind, effects::group_clips_children};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitPhase {
    Below,
    Middle,
    Above,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SplitError {
    #[error("split rendering requires a root group and one descendant moving subtree")]
    InvalidRoots,
    #[error("split rendering does not support node {0}")]
    UnsupportedNode(NodeId),
    #[error("split rendering does not support composited or unbounded painted ancestor {0}")]
    UnsupportedAncestor(NodeId),
    #[error("split rendering does not support masks ({0})")]
    Mask(NodeId),
    #[error("split rendering does not support backdrop-dependent paint ({0})")]
    Backdrop(NodeId),
    #[error("split rendering does not support pattern or shader paint ({0})")]
    DependentPaint(NodeId),
    #[error("split rendering does not support variable-bound node {0}")]
    Binding(NodeId),
    #[error("split rendering requires finite geometry ({0})")]
    InvalidGeometry(NodeId),
    #[error("split rendering requires decoded image {0}")]
    UnresolvedAsset(AssetId),
    #[error("split rendering does not support motion or media playback")]
    DynamicInputs,
    #[error("the scene changed after the split was prepared")]
    StaleScene,
    #[error("clipped ancestor paint requires ordered phase drawing, not raster phase compositing")]
    RequiresOrderedPaint,
    #[error("split rendering requires a finite positive viewport and target size")]
    InvalidViewport,
}

#[derive(Clone, Copy)]
enum PaintAtom {
    Ancestor,
    Subtree(SplitPhase),
}

#[derive(Default)]
struct FrozenAssets(HashMap<AssetId, DecodedImage>);

impl AssetResolver for FrozenAssets {
    fn resolve(&self, id: AssetId) -> Option<DecodedImage> {
        self.0.get(&id).cloned()
    }
}

/// An opt-in paint partition for one immutable scene revision. Prepare again
/// after any edit. This is not a retained-frame cache or an interactive move.
/// Images are frozen here so asynchronous decode/eviction cannot change the
/// artwork between phases. Clipped ancestors require ordered command replay;
/// other specs also permit equal-sized raster phases composited source-over.
pub struct SplitSpec {
    scene_instance: u64,
    scene_revision: u64,
    page_root: NodeId,
    requires_ordered_paint: bool,
    atoms: HashMap<NodeId, PaintAtom>,
    assets: FrozenAssets,
}

impl SplitSpec {
    pub fn prepare(
        scene: &Scene,
        page_root: NodeId,
        moving: NodeId,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
    ) -> Result<Self, SplitError> {
        validate_inputs(inputs)?;
        let page = scene.get(page_root).ok_or(SplitError::InvalidRoots)?;
        if page.parent.is_some() || !matches!(page.data, NodeData::Group(_)) || moving == page_root
        {
            return Err(SplitError::InvalidRoots);
        }
        let ancestors: HashSet<_> = scene.ancestors_of(moving).map(|node| node.id).collect();
        if !scene.contains(moving) || !ancestors.contains(&page_root) {
            return Err(SplitError::InvalidRoots);
        }
        let mut spec = Self {
            scene_instance: scene.instance_id(),
            scene_revision: scene.revision(),
            page_root,
            requires_ordered_paint: false,
            atoms: HashMap::new(),
            assets: FrozenAssets::default(),
        };
        let mut phase = SplitPhase::Below;
        let mut pending = vec![(page_root, false)];
        while let Some((id, exiting)) = pending.pop() {
            if exiting {
                phase = SplitPhase::Above;
                continue;
            }
            let node = scene.get(id).ok_or(SplitError::InvalidRoots)?;
            if id == moving {
                phase = SplitPhase::Middle;
                pending.push((id, true));
            }
            spec.validate_node(node, resolver)?;
            if ancestors.contains(&id) {
                validate_ancestor(node, id == page_root)?;
                spec.requires_ordered_paint |=
                    matches!(&node.data, NodeData::Group(group) if group.clip_size.is_some());
                spec.atoms.insert(id, PaintAtom::Ancestor);
            } else {
                spec.atoms.insert(id, PaintAtom::Subtree(phase));
            }
            let children = scene.children_of(Some(id));
            let reverse_z = matches!(&node.data, NodeData::Group(group)
                if group.auto_layout.as_ref().is_some_and(|layout| layout.reverse_z));
            if reverse_z {
                pending.extend(children.iter().copied().map(|child| (child, false)));
            } else {
                pending.extend(children.iter().rev().copied().map(|child| (child, false)));
            }
        }
        Ok(spec)
    }

    pub fn page_root(&self) -> NodeId {
        self.page_root
    }

    /// Whether phase commands must be painted/replayed in order instead of
    /// compositing separately rasterized phase images.
    pub fn requires_ordered_paint(&self) -> bool {
        self.requires_ordered_paint
    }

    pub(crate) fn validate(&self, scene: &Scene, inputs: &RenderInputs) -> Result<(), SplitError> {
        if self.scene_instance != scene.instance_id() || self.scene_revision != scene.revision() {
            return Err(SplitError::StaleScene);
        }
        validate_inputs(inputs)
    }

    pub(crate) fn resolver(&self) -> &dyn AssetResolver {
        &self.assets
    }

    pub(crate) fn includes(&self, id: NodeId, phase: SplitPhase) -> bool {
        match self.atoms.get(&id) {
            Some(PaintAtom::Ancestor) => true,
            Some(PaintAtom::Subtree(assigned)) => *assigned == phase,
            None => false,
        }
    }

    pub(crate) fn paints_background(&self, id: NodeId, phase: SplitPhase) -> bool {
        match self.atoms.get(&id) {
            Some(PaintAtom::Ancestor) => phase == SplitPhase::Below,
            Some(PaintAtom::Subtree(assigned)) => *assigned == phase,
            None => false,
        }
    }

    pub(crate) fn paints_foreground(&self, id: NodeId, phase: SplitPhase) -> bool {
        match self.atoms.get(&id) {
            Some(PaintAtom::Ancestor) => phase == SplitPhase::Above,
            Some(PaintAtom::Subtree(assigned)) => *assigned == phase,
            None => false,
        }
    }

    fn validate_node(
        &mut self,
        node: &CanvasNode,
        resolver: Option<&dyn AssetResolver>,
    ) -> Result<(), SplitError> {
        if !node
            .transform
            .to_components()
            .into_iter()
            .all(finite_scalar)
            || node.effects.iter().any(|effect| {
                ![
                    effect.blur,
                    effect.spread,
                    effect.offset[0],
                    effect.offset[1],
                ]
                .into_iter()
                .all(finite_scalar)
            })
            || node.blurs.iter().any(|blur| !finite_scalar(blur.radius))
            || node
                .data
                .local_bounds()
                .is_some_and(|bounds| !bounds.is_finite())
        {
            return Err(SplitError::InvalidGeometry(node.id));
        }
        if !node.bindings.is_empty() {
            return Err(SplitError::Binding(node.id));
        }
        if node.is_mask {
            return Err(SplitError::Mask(node.id));
        }
        if node.blend_mode != BlendMode::Normal
            || node
                .blurs
                .iter()
                .any(|blur| blur.kind == BlurKind::Background)
        {
            return Err(SplitError::Backdrop(node.id));
        }
        match &node.data {
            NodeData::Group(group) => {
                require_geometry(
                    node.id,
                    group.local_size.into_iter().flatten().all(finite_scalar)
                        && group.clip_size.into_iter().flatten().all(finite_scalar)
                        && finite_corners(
                            group.corner_radius,
                            group.corner_radii,
                            group.corner_smoothing,
                        )
                        && group.strokes.iter().all(finite_stroke),
                )?;
                for fill in group
                    .background
                    .iter()
                    .chain(&group.background_fills)
                    .chain(group.strokes.iter().map(|stroke| &stroke.paint))
                {
                    self.validate_fill(node.id, fill, resolver)?;
                }
            }
            NodeData::Vector(vector) => {
                require_geometry(
                    node.id,
                    vector.path.segments.iter().all(finite_segment)
                        && vector.local_size.into_iter().flatten().all(finite_scalar)
                        && finite_corners(
                            vector.corner_radius,
                            vector.corner_radii,
                            vector.corner_smoothing,
                        )
                        && vector.strokes.iter().all(finite_stroke),
                )?;
                for fill in vector
                    .fills
                    .iter()
                    .chain(vector.strokes.iter().map(|stroke| &stroke.paint))
                {
                    self.validate_fill(node.id, fill, resolver)?;
                }
            }
            NodeData::Bitmap(bitmap) => {
                require_geometry(
                    node.id,
                    bitmap.local_size.into_iter().all(finite_scalar)
                        && bitmap.crop.into_iter().flatten().all(f32::is_finite),
                )?;
                self.freeze_image(bitmap.asset, resolver)?;
            }
            NodeData::Text(text) => {
                require_geometry(
                    node.id,
                    text.local_size.into_iter().all(finite_scalar)
                        && finite_scalar(text.paragraph_spacing)
                        && finite_scalar(text.paragraph_indent)
                        && finite_text_style(&text.style)
                        && text
                            .style_runs
                            .iter()
                            .all(|run| finite_text_style(&run.style)),
                )?;
            }
            _ => return Err(SplitError::UnsupportedNode(node.id)),
        }
        Ok(())
    }

    fn validate_fill(
        &mut self,
        node: NodeId,
        fill: &Fill,
        resolver: Option<&dyn AssetResolver>,
    ) -> Result<(), SplitError> {
        let blend = match fill {
            Fill::Solid { blend, .. } => *blend,
            Fill::Gradient { gradient, blend } => {
                require_geometry(node, finite_gradient(gradient))?;
                *blend
            }
            Fill::Image {
                asset,
                blend,
                opacity,
                crop,
                scale,
                rotation,
                adjust,
                ..
            } => {
                require_geometry(
                    node,
                    opacity.is_finite()
                        && crop
                            .as_deref()
                            .into_iter()
                            .flatten()
                            .all(|value| value.is_finite())
                        && scale.is_none_or(f32::is_finite)
                        && rotation.is_none_or(f32::is_finite)
                        && [
                            adjust.exposure,
                            adjust.contrast,
                            adjust.saturation,
                            adjust.temperature,
                            adjust.tint,
                            adjust.highlights,
                            adjust.shadows,
                        ]
                        .into_iter()
                        .all(f32::is_finite),
                )?;
                self.freeze_image(*asset, resolver)?;
                *blend
            }
            Fill::Pattern { .. } | Fill::Shader { .. } => {
                return Err(SplitError::DependentPaint(node));
            }
            Fill::Video { .. } => return Err(SplitError::UnsupportedNode(node)),
        };
        if blend != BlendMode::Normal {
            return Err(SplitError::Backdrop(node));
        }
        Ok(())
    }

    fn freeze_image(
        &mut self,
        asset: AssetId,
        resolver: Option<&dyn AssetResolver>,
    ) -> Result<(), SplitError> {
        if self.assets.0.contains_key(&asset) {
            return Ok(());
        }
        let image = resolver
            .and_then(|resolver| resolver.resolve(asset))
            .ok_or(SplitError::UnresolvedAsset(asset))?;
        let expected_len = (image.width as usize)
            .checked_mul(image.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if image.width == 0 || image.height == 0 || expected_len != Some(image.pixels_rgba.len()) {
            return Err(SplitError::UnresolvedAsset(asset));
        }
        self.assets.0.insert(asset, image);
        Ok(())
    }
}

fn require_geometry(node: NodeId, valid: bool) -> Result<(), SplitError> {
    if valid {
        Ok(())
    } else {
        Err(SplitError::InvalidGeometry(node))
    }
}

fn finite_scalar(value: f64) -> bool {
    value.is_finite() && (value as f32).is_finite()
}

fn finite_segment(segment: &PathSegment) -> bool {
    match *segment {
        PathSegment::Move { to } | PathSegment::Line { to } => to.into_iter().all(finite_scalar),
        PathSegment::Quad { ctrl, to } => ctrl.into_iter().chain(to).all(finite_scalar),
        PathSegment::Cubic { ctrl1, ctrl2, to } => {
            ctrl1.into_iter().chain(ctrl2).chain(to).all(finite_scalar)
        }
        PathSegment::Close => true,
    }
}

fn finite_corners(radius: Option<f64>, radii: Option<[f64; 4]>, smoothing: f32) -> bool {
    radius.is_none_or(finite_scalar)
        && radii.into_iter().flatten().all(finite_scalar)
        && smoothing.is_finite()
}

fn finite_stroke(stroke: &Stroke) -> bool {
    finite_scalar(stroke.width)
        && finite_scalar(stroke.miter_limit)
        && stroke.dash.iter().copied().all(finite_scalar)
        && stroke.per_side.into_iter().flatten().all(finite_scalar)
}

fn finite_text_style(style: &TextStyle) -> bool {
    [style.size_px, style.letter_spacing, style.line_height]
        .into_iter()
        .all(finite_scalar)
        && style.line_height_auto_percent.is_none_or(finite_scalar)
        && style
            .font_variations
            .iter()
            .all(|variation| variation.value.is_finite())
}

fn finite_gradient(gradient: &Gradient) -> bool {
    let (geometry, stops) = match gradient {
        Gradient::Linear { start, end, stops } => (
            start.iter().chain(end).all(|value| value.is_finite()),
            stops,
        ),
        Gradient::Radial {
            center,
            radius,
            handles,
            stops,
        }
        | Gradient::Diamond {
            center,
            radius,
            handles,
            stops,
        } => (
            center.iter().all(|value| value.is_finite())
                && radius.is_finite()
                && handles
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|value| value.is_finite()),
            stops,
        ),
        Gradient::Angular {
            center,
            start_angle,
            stops,
        } => (
            center.iter().all(|value| value.is_finite()) && start_angle.is_finite(),
            stops,
        ),
    };
    geometry && stops.iter().all(|stop| stop.position.is_finite())
}

fn validate_inputs(inputs: &RenderInputs) -> Result<(), SplitError> {
    if inputs.motion.is_some() || inputs.playback.is_some() || inputs.video_fill_frames.is_some() {
        return Err(SplitError::DynamicInputs);
    }
    Ok(())
}

fn validate_ancestor(node: &CanvasNode, page: bool) -> Result<(), SplitError> {
    let NodeData::Group(group) = &node.data else {
        return Err(SplitError::UnsupportedAncestor(node.id));
    };
    let page_clear_only = page
        && group.local_size.is_none()
        && group.clip_size.is_none()
        && (group.background.is_none()
            || matches!(
                group.background,
                Some(Fill::Solid {
                    blend: BlendMode::Normal,
                    ..
                })
            ))
        && group.background_fills.is_empty()
        && group.strokes.is_empty()
        && node.effects.is_empty();
    let has_paint = group.background.is_some()
        || !group.background_fills.is_empty()
        || !group.strokes.is_empty()
        || !node.effects.is_empty();
    let fixed_clip = group_clips_children(node, group)
        && group
            .clip_size
            .is_some_and(|size| size.into_iter().all(|value| value > 0.0));
    // A painted ancestor without a fixed child clip can derive its silhouette
    // from the moving child's union bounds. Its cached paint would then move.
    if (has_paint && !page_clear_only && !fixed_clip)
        || node.opacity.get() != 1.0
        || node
            .effects
            .iter()
            .any(|effect| effect.kind != ShadowKind::Inner)
        || !node.blurs.is_empty()
        || node
            .flags
            .intersects(NodeFlags::ISOLATED_BLEND | NodeFlags::HIDDEN)
    {
        return Err(SplitError::UnsupportedAncestor(node.id));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct SplitPass<'a> {
    pub(crate) spec: &'a SplitSpec,
    pub(crate) phase: SplitPhase,
    pub(crate) clear_target: bool,
}
