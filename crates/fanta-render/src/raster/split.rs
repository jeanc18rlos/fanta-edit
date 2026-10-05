use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use fanta_doc::{
    AssetId, BlendMode, CanvasNode, ComponentId, ComponentLibrary, Fill, Gradient,
    InstanceExpansionContext, InstanceNode, ModeId, NodeData, NodeFlags, NodeId, PathSegment,
    Scene, Stroke, TextStyle, VariableCollectionId, VariableRegistry,
};

use crate::asset::{AssetResolver, DecodedImage};

use super::renderer::PreparedInstance;
use super::{BlurKind, RenderInputs, ShadowKind, effects::group_clips_children};

const MAX_PREPARED_NODES: usize = 100_000;
const MAX_PREPARED_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitPhase {
    Below,
    Middle,
    Above,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SplitError {
    #[error("split rendering requires a root group and one descendant moving subtree")]
    InvalidRoots,
    #[error("split rendering does not support node {0}")]
    UnsupportedNode(NodeId),
    #[error("split rendering does not support composited or unbounded painted ancestor {0}")]
    UnsupportedAncestor(NodeId),
    #[error("the split boundary intersects the mask run beginning at {0}")]
    Mask(NodeId),
    #[error(
        "split rendering does not support backdrop-sampling effects or blended ancestors ({0})"
    )]
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
    #[error("the component or variable inputs changed after the split was prepared")]
    StaleInputs,
    #[error("instance {instance} references missing component {component}")]
    MissingInstance {
        instance: NodeId,
        component: ComponentId,
    },
    #[error("recursive instance dependency on component {0}")]
    RecursiveInstance(ComponentId),
    #[error("instance {0} exceeds the prepared node/depth budget")]
    ExpansionLimit(NodeId),
    #[error("instance {instance} at definition path {path:?}: {source}")]
    InstanceContents {
        instance: NodeId,
        path: Vec<NodeId>,
        source: Box<SplitError>,
    },
    #[error(
        "prepared blend, mask, instance or ancestor paint requires ordered phase drawing, not raster phase compositing"
    )]
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
/// artwork between phases. Clipped ancestors, instances, blends and masks require
/// ordered command replay; remaining specs also permit raster phase compositing.
/// Input registries are immutably borrowed for the plan lifetime, so public
/// map edits require dropping/repreparing it. Rendering with another registry
/// identity rejects conservatively even if its values happen to be equal.
pub struct SplitSpec<'inputs> {
    scene_instance: u64,
    scene_revision: u64,
    page_root: NodeId,
    requires_ordered_paint: bool,
    atoms: HashMap<NodeId, PaintAtom>,
    assets: FrozenAssets,
    components: Cow<'inputs, ComponentLibrary>,
    variables: Cow<'inputs, VariableRegistry>,
    active_modes: Cow<'inputs, BTreeMap<VariableCollectionId, ModeId>>,
    mode_generation: u64,
    dark_ui: bool,
    instances: HashMap<NodeId, Arc<PreparedInstance>>,
    used_components: HashSet<ComponentId>,
    prepared_nodes: usize,
    max_prepared_nodes: usize,
    max_prepared_depth: usize,
}

impl<'inputs> SplitSpec<'inputs> {
    pub fn prepare(
        scene: &Scene,
        page_root: NodeId,
        moving: NodeId,
        inputs: &RenderInputs<'inputs>,
        resolver: Option<&dyn AssetResolver>,
    ) -> Result<Self, SplitError> {
        Self::prepare_with_budget(
            scene,
            page_root,
            moving,
            inputs,
            resolver,
            MAX_PREPARED_NODES,
            MAX_PREPARED_DEPTH,
        )
    }

    fn prepare_with_budget(
        scene: &Scene,
        page_root: NodeId,
        moving: NodeId,
        inputs: &RenderInputs<'inputs>,
        resolver: Option<&dyn AssetResolver>,
        max_prepared_nodes: usize,
        max_prepared_depth: usize,
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
            components: Cow::Borrowed(inputs.components),
            variables: Cow::Borrowed(inputs.variables),
            active_modes: Cow::Borrowed(inputs.active_modes),
            mode_generation: inputs.mode_generation,
            dark_ui: inputs.dark_ui,
            instances: HashMap::new(),
            used_components: HashSet::new(),
            prepared_nodes: 0,
            max_prepared_nodes,
            max_prepared_depth,
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
            if let NodeData::Instance(instance) = &node.data {
                spec.requires_ordered_paint = true;
                spec.prepare_instance(scene, id, instance, id, 0, &mut Vec::new(), resolver)?;
                continue;
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
        spec.validate_mask_partitions(scene)?;
        Ok(spec)
    }

    fn into_owned(self) -> SplitSpec<'static> {
        SplitSpec {
            scene_instance: self.scene_instance,
            scene_revision: self.scene_revision,
            page_root: self.page_root,
            requires_ordered_paint: self.requires_ordered_paint,
            atoms: self.atoms,
            assets: self.assets,
            components: Cow::Owned(self.components.into_owned()),
            variables: Cow::Owned(self.variables.into_owned()),
            active_modes: Cow::Owned(self.active_modes.into_owned()),
            mode_generation: self.mode_generation,
            dark_ui: self.dark_ui,
            instances: self.instances,
            used_components: self.used_components,
            prepared_nodes: self.prepared_nodes,
            max_prepared_nodes: self.max_prepared_nodes,
            max_prepared_depth: self.max_prepared_depth,
        }
    }

    fn owned_inputs(&self) -> RenderInputs<'_> {
        RenderInputs {
            components: self.components.as_ref(),
            variables: self.variables.as_ref(),
            active_modes: self.active_modes.as_ref(),
            mode_generation: self.mode_generation,
            dark_ui: self.dark_ui,
            motion: None,
            playback: None,
            video_fill_frames: None,
        }
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
        validate_inputs(inputs)?;
        if !std::ptr::eq(self.components.as_ref(), inputs.components)
            || !std::ptr::eq(self.variables.as_ref(), inputs.variables)
            || !std::ptr::eq(self.active_modes.as_ref(), inputs.active_modes)
            || self.mode_generation != inputs.mode_generation
            || self.dark_ui != inputs.dark_ui
        {
            return Err(SplitError::StaleInputs);
        }
        Ok(())
    }

    pub fn prepared_instance_count(&self) -> usize {
        self.instances.len()
    }

    pub fn prepared_node_count(&self) -> usize {
        self.prepared_nodes
    }

    pub(crate) fn prepared_instance(&self, id: NodeId) -> Option<&Arc<PreparedInstance>> {
        self.instances.get(&id)
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

    #[allow(clippy::too_many_arguments)]
    fn prepare_instance(
        &mut self,
        scene: &Scene,
        instance_id: NodeId,
        instance: &InstanceNode,
        mode_anchor: NodeId,
        depth: usize,
        components_on_path: &mut Vec<ComponentId>,
        resolver: Option<&dyn AssetResolver>,
    ) -> Result<(), SplitError> {
        let context = InstanceExpansionContext::new(
            self.variables.as_ref(),
            self.active_modes.as_ref(),
            mode_anchor,
        );
        let selected = fanta_doc::resolved_component_with_context(
            scene,
            self.components.as_ref(),
            instance,
            &context,
        )
        .ok_or(SplitError::MissingInstance {
            instance: instance_id,
            component: instance.component,
        })?;
        self.used_components.insert(selected.resolved_component);
        if components_on_path.contains(&selected.resolved_component) {
            return Err(SplitError::RecursiveInstance(selected.resolved_component));
        }
        if !scene.contains(selected.resolved_root) {
            return Err(SplitError::MissingInstance {
                instance: instance_id,
                component: instance.component,
            });
        }
        // Reserve before cloning or layout: a wide master otherwise allocates before its budget is checked.
        let mut pending = vec![(selected.resolved_root, depth)];
        while let Some((node, node_depth)) = pending.pop() {
            if self.prepared_nodes >= self.max_prepared_nodes
                || node_depth >= self.max_prepared_depth
            {
                return Err(SplitError::ExpansionLimit(instance_id));
            }
            self.prepared_nodes += 1;
            pending.extend(
                scene
                    .children_of(Some(node))
                    .iter()
                    .copied()
                    .map(|child| (child, node_depth + 1)),
            );
        }
        let nodes = fanta_doc::expand_instance_with_context(
            scene,
            self.components.as_ref(),
            instance,
            &context,
        );
        // Reject invalid resolved geometry before it can reach text shaping or layout.
        for entry in &nodes {
            self.validate_node(&entry.node, resolver)
                .map_err(|source| SplitError::InstanceContents {
                    instance: mode_anchor,
                    path: entry.def_path.to_vec(),
                    source: Box::new(source),
                })?;
        }
        let expanded = Arc::new(super::instance::prepare_instance_nodes(instance, nodes));
        if expanded.nodes.is_empty() || expanded.root.is_none() {
            return Err(SplitError::MissingInstance {
                instance: instance_id,
                component: instance.component,
            });
        }
        components_on_path.push(selected.resolved_component);
        for entry in &expanded.nodes {
            self.validate_node(&entry.node, resolver)
                .map_err(|source| SplitError::InstanceContents {
                    instance: mode_anchor,
                    path: entry.def_path.to_vec(),
                    source: Box::new(source),
                })?;
            if let NodeData::Instance(nested) = &entry.node.data {
                self.prepare_instance(
                    scene,
                    entry.node.id,
                    nested,
                    mode_anchor,
                    depth + entry.def_path.len() + 1,
                    components_on_path,
                    resolver,
                )
                .map_err(|source| SplitError::InstanceContents {
                    instance: mode_anchor,
                    path: entry.def_path.to_vec(),
                    source: Box::new(source),
                })?;
            }
        }
        components_on_path.pop();
        self.instances.insert(instance_id, expanded);
        Ok(())
    }

    fn validate_mask_partitions(&self, scene: &Scene) -> Result<(), SplitError> {
        // Only ancestor sequences cross a phase boundary. Every other live or
        // prepared-instance subtree is retained in one atomic phase.
        for (&parent, atom) in &self.atoms {
            if !matches!(atom, PaintAtom::Ancestor) {
                continue;
            }
            let node = scene.get(parent).ok_or(SplitError::InvalidRoots)?;
            let children = scene.children_of(Some(parent));
            let reversed;
            let children = if matches!(&node.data, NodeData::Group(group)
                if group.auto_layout.as_ref().is_some_and(|layout| layout.reverse_z))
            {
                reversed = children.iter().rev().copied().collect::<Vec<_>>();
                reversed.as_slice()
            } else {
                children
            };
            let is_mask = |id: NodeId| {
                scene
                    .get(id)
                    .is_some_and(|node| node.is_mask && !node.flags.contains(NodeFlags::HIDDEN))
            };
            let mut children = children.iter().copied().peekable();
            while let Some(first) = children.next() {
                if !is_mask(first) {
                    continue;
                }
                let mut run = vec![first];
                while children.peek().is_some_and(|id| is_mask(*id)) {
                    if let Some(mask) = children.next() {
                        run.push(mask);
                    }
                }
                let mut has_content = false;
                while children.peek().is_some_and(|id| !is_mask(*id)) {
                    if let Some(content) = children.next() {
                        has_content = true;
                        run.push(content);
                    }
                }
                if !has_content {
                    continue;
                }
                let Some(PaintAtom::Subtree(phase)) = self.atoms.get(&first) else {
                    return Err(SplitError::Mask(first));
                };
                if run.iter().any(|id| {
                    !matches!(self.atoms.get(id),
                    Some(PaintAtom::Subtree(other)) if other == phase)
                }) {
                    return Err(SplitError::Mask(first));
                }
            }
        }
        Ok(())
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
        self.requires_ordered_paint |= node.is_mask || node.blend_mode != BlendMode::Normal;
        if node
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
            NodeData::Instance(instance) => {
                require_geometry(node.id, instance.local_size.into_iter().all(finite_scalar))?;
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
        self.requires_ordered_paint |= blend != BlendMode::Normal;
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

const RETAINED_PIXEL_AND_PICTURE_BUDGET: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, thiserror::Error)]
pub enum RetainedError {
    #[error(transparent)]
    Split(#[from] SplitError),
    #[error("retained translation has already been disabled after a failed frame")]
    Disabled,
    #[error("retained translation requires only the selected root's finite translation to change")]
    ChangedScene,
    #[error("moving this master changes prepared component {0}")]
    DependentComponent(ComponentId),
    #[error("retained translation requires unchanged component definitions, modes and variables")]
    ChangedInputs,
    #[error(
        "retained translation requires unchanged viewport, dimensions, scale, background and fonts"
    )]
    ChangedFrame,
    #[error("retained image {0} changed or became unavailable")]
    ChangedAsset(AssetId),
    #[error("retained pixel and picture storage exceeds its {limit} byte budget")]
    MemoryLimit { limit: usize },
    #[error("could not allocate or record a retained frame")]
    Allocation,
    #[error("retained painting reported incomplete artwork or an effect failure")]
    IncompleteArtwork,
}

#[derive(Clone, Debug, Default)]
pub struct RetainedBuildMetrics {
    pub prepare_micros: u64,
    pub below_micros: u64,
    pub above_record_micros: u64,
    pub surface_bytes: usize,
    pub frozen_pixel_bytes: usize,
    pub picture_bytes: usize,
    pub prepared_nodes: usize,
    pub prepared_instances: usize,
    pub below: super::RenderMetrics,
    pub above: super::RenderMetrics,
}

#[derive(Clone, Debug, Default)]
pub struct RetainedFrameMetrics {
    pub validation_micros: u64,
    pub below_copy_micros: u64,
    pub middle_micros: u64,
    pub above_replay_micros: u64,
    pub frame_micros: u64,
    pub middle: super::RenderMetrics,
}

pub struct RetainedFrame {
    pub image: skia_safe::Image,
    pub metrics: RetainedFrameMetrics,
}

/// An opt-in CPU experiment for one fixed-view translation gesture. It owns
/// its frame canvas so cached device-space paint cannot be replayed under a
/// different caller matrix or clip. It is not connected to the live worker.
/// Any failed frame disables reuse until the caller prepares a new session.
///
/// `font_generation` must change whenever the caller's font environment does.
/// The storage budget covers two pixel surfaces, retained decoded images and
/// recorded Picture bytes; the existing prepared node/depth limits apply too.
/// Picture accounting is approximate and does not bound total Skia scratch
/// memory. Callers must release old frame images to avoid snapshot copies.
pub struct RetainedTranslationSession {
    spec: SplitSpec<'static>,
    moving: NodeId,
    original_transform: fanta_doc::Transform2D,
    preview_components: HashSet<ComponentId>,
    viewport: fanta_doc::Viewport,
    size: (u32, u32),
    display_scale: f64,
    background: fanta_doc::Color,
    pixel_snap_pan: bool,
    font_generation: u64,
    below: skia_safe::Image,
    above: skia_safe::Picture,
    output: skia_safe::Surface,
    build_metrics: RetainedBuildMetrics,
    disabled: bool,
}

impl RetainedTranslationSession {
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        renderer: &mut super::RasterRenderer,
        scene: &Scene,
        page_root: NodeId,
        moving: NodeId,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
    ) -> Result<Self, RetainedError> {
        Self::prepare_with_budget(
            renderer,
            scene,
            page_root,
            moving,
            viewport,
            inputs,
            resolver,
            font_generation,
            RETAINED_PIXEL_AND_PICTURE_BUDGET,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_with_budget(
        renderer: &mut super::RasterRenderer,
        scene: &Scene,
        page_root: NodeId,
        moving: NodeId,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
        byte_limit: usize,
    ) -> Result<Self, RetainedError> {
        let started = std::time::Instant::now();
        let size = (renderer.width(), renderer.height());
        let surface_bytes = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .and_then(|pixels| pixels.checked_mul(8))
            .filter(|bytes| *bytes <= byte_limit)
            .ok_or(RetainedError::MemoryLimit { limit: byte_limit })?;
        if size.0 == 0 || size.1 == 0 || size.0 > i32::MAX as u32 || size.1 > i32::MAX as u32 {
            return Err(SplitError::InvalidViewport.into());
        }
        let spec = SplitSpec::prepare(scene, page_root, moving, inputs, resolver)?.into_owned();
        let original_transform = scene.get(moving).ok_or(SplitError::InvalidRoots)?.transform;
        let mut chain: HashSet<_> = scene.ancestors_of(moving).map(|node| node.id).collect();
        chain.insert(moving);
        let preview_components: HashSet<_> = inputs
            .components
            .defs
            .iter()
            .filter_map(|(id, definition)| chain.contains(&definition.root).then_some(*id))
            .collect();
        if let Some(dependency) = preview_components
            .intersection(&spec.used_components)
            .next()
        {
            return Err(RetainedError::DependentComponent(*dependency));
        }
        let frozen_pixel_bytes = spec
            .assets
            .0
            .values()
            .try_fold(0usize, |total, image| {
                total.checked_add(image.pixels_rgba.len())
            })
            .ok_or(RetainedError::MemoryLimit { limit: byte_limit })?;
        let retained_bytes = surface_bytes
            .checked_add(frozen_pixel_bytes)
            .filter(|bytes| *bytes <= byte_limit)
            .ok_or(RetainedError::MemoryLimit { limit: byte_limit })?;
        let prepared_nodes = spec.prepared_node_count();
        let prepared_instances = spec.prepared_instance_count();
        let prepare_micros = elapsed_micros(started);
        let owned_inputs = spec.owned_inputs();
        let mut below_surface =
            skia_safe::surfaces::raster_n32_premul((size.0 as i32, size.1 as i32))
                .ok_or(RetainedError::Allocation)?;
        let below_started = std::time::Instant::now();
        let below_metrics = renderer.paint_split_to_canvas(
            below_surface.canvas(),
            size.0,
            size.1,
            scene,
            viewport,
            &owned_inputs,
            &spec,
            SplitPhase::Below,
        )?;
        require_complete(&below_metrics)?;
        let below = below_surface.image_snapshot();
        let below_micros = elapsed_micros(below_started);
        let mut recorder = skia_safe::PictureRecorder::new();
        let recording =
            recorder.begin_recording(skia_safe::Rect::from_wh(size.0 as f32, size.1 as f32), None);
        let above_started = std::time::Instant::now();
        let above_metrics = renderer.paint_split_to_canvas(
            recording,
            size.0,
            size.1,
            scene,
            viewport,
            &owned_inputs,
            &spec,
            SplitPhase::Above,
        )?;
        require_complete(&above_metrics)?;
        let above = recorder
            .finish_recording_as_picture(None)
            .ok_or(RetainedError::Allocation)?;
        let above_record_micros = elapsed_micros(above_started);
        let picture_bytes = above.approximate_bytes_used();
        retained_bytes
            .checked_add(picture_bytes)
            .filter(|bytes| *bytes <= byte_limit)
            .ok_or(RetainedError::MemoryLimit { limit: byte_limit })?;
        let output = skia_safe::surfaces::raster_n32_premul((size.0 as i32, size.1 as i32))
            .ok_or(RetainedError::Allocation)?;
        Ok(Self {
            spec,
            moving,
            original_transform,
            preview_components,
            viewport: *viewport,
            size,
            display_scale: renderer.display_scale,
            background: renderer.background,
            pixel_snap_pan: renderer.pixel_snap_pan(),
            font_generation,
            below,
            above,
            output,
            build_metrics: RetainedBuildMetrics {
                prepare_micros,
                below_micros,
                above_record_micros,
                surface_bytes,
                frozen_pixel_bytes,
                picture_bytes,
                prepared_nodes,
                prepared_instances,
                below: below_metrics,
                above: above_metrics,
            },
            disabled: false,
        })
    }

    pub fn build_metrics(&self) -> &RetainedBuildMetrics {
        &self.build_metrics
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        renderer: &mut super::RasterRenderer,
        scene: &Scene,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
    ) -> Result<RetainedFrame, RetainedError> {
        if self.disabled {
            return Err(RetainedError::Disabled);
        }
        let result =
            self.render_checked(renderer, scene, viewport, inputs, resolver, font_generation);
        if result.is_err() {
            self.disabled = true;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn render_checked(
        &mut self,
        renderer: &mut super::RasterRenderer,
        scene: &Scene,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
    ) -> Result<RetainedFrame, RetainedError> {
        let started = std::time::Instant::now();
        validate_inputs(inputs)?;
        if self.size != (renderer.width(), renderer.height())
            || self.viewport.center != viewport.center
            || self.viewport.zoom != viewport.zoom
            || self.display_scale != renderer.display_scale
            || self.background != renderer.background
            || self.pixel_snap_pan != renderer.pixel_snap_pan()
            || self.font_generation != font_generation
        {
            return Err(RetainedError::ChangedFrame);
        }
        if self.spec.scene_instance != scene.instance_id() {
            return Err(RetainedError::ChangedScene);
        }
        let delta = scene
            .changes_since(self.spec.scene_revision)
            .ok_or(RetainedError::ChangedScene)?;
        if !delta.nodes.is_empty() || delta.transforms.iter().any(|id| *id != self.moving) {
            return Err(RetainedError::ChangedScene);
        }
        let transform = scene
            .get(self.moving)
            .ok_or(RetainedError::ChangedScene)?
            .transform;
        if !transform.is_finite()
            || !transform.to_components().into_iter().all(finite_scalar)
            || transform.0.matrix2 != self.original_transform.0.matrix2
        {
            return Err(RetainedError::ChangedScene);
        }
        if !retained_components_equal(
            self.spec.components.as_ref(),
            inputs.components,
            &self.preview_components,
        ) || self.spec.variables.as_ref() != inputs.variables
            || self.spec.active_modes.as_ref() != inputs.active_modes
            || self.spec.mode_generation != inputs.mode_generation
            || self.spec.dark_ui != inputs.dark_ui
        {
            return Err(RetainedError::ChangedInputs);
        }
        for (asset, frozen) in &self.spec.assets.0 {
            let current = resolver
                .and_then(|resolver| resolver.resolve(*asset))
                .ok_or(RetainedError::ChangedAsset(*asset))?;
            if current.width != frozen.width
                || current.height != frozen.height
                || !Arc::ptr_eq(&current.pixels_rgba, &frozen.pixels_rgba)
            {
                return Err(RetainedError::ChangedAsset(*asset));
            }
        }
        let validation_micros = elapsed_micros(started);
        // This is the only revision advance: every dependency and every intervening scene edit was checked above.
        self.spec.scene_revision = scene.revision();
        let owned_inputs = self.spec.owned_inputs();
        let canvas = self.output.canvas();
        let copy_started = std::time::Instant::now();
        let mut copy_paint = skia_safe::Paint::default();
        copy_paint.set_blend_mode(skia_safe::BlendMode::Src);
        canvas.draw_image(&self.below, (0, 0), Some(&copy_paint));
        let below_copy_micros = elapsed_micros(copy_started);
        let middle_started = std::time::Instant::now();
        let middle = renderer.paint_split_to_canvas(
            canvas,
            self.size.0,
            self.size.1,
            scene,
            viewport,
            &owned_inputs,
            &self.spec,
            SplitPhase::Middle,
        )?;
        require_complete(&middle)?;
        let middle_micros = elapsed_micros(middle_started);
        let above_started = std::time::Instant::now();
        canvas.draw_picture(&self.above, None, None);
        let above_replay_micros = elapsed_micros(above_started);
        let image = self.output.image_snapshot();
        Ok(RetainedFrame {
            image,
            metrics: RetainedFrameMetrics {
                validation_micros,
                below_copy_micros,
                middle_micros,
                above_replay_micros,
                frame_micros: elapsed_micros(started),
                middle,
            },
        })
    }
}

fn require_complete(metrics: &super::RenderMetrics) -> Result<(), RetainedError> {
    if metrics.incomplete_artwork || metrics.effect_failed || metrics.non_artwork_content {
        Err(RetainedError::IncompleteArtwork)
    } else {
        Ok(())
    }
}

fn elapsed_micros(started: std::time::Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn retained_components_equal(
    baseline: &ComponentLibrary,
    current: &ComponentLibrary,
    preview_components: &HashSet<ComponentId>,
) -> bool {
    if baseline.sets != current.sets || baseline.defs.len() != current.defs.len() {
        return false;
    }
    baseline.defs.iter().all(|(id, before)| {
        let Some(after) = current.defs.get(id) else {
            return false;
        };
        let fanta_doc::ComponentDef {
            id: before_id,
            root,
            name,
            variant_of,
            props,
            rev,
            preview_rev,
        } = before;
        before_id == &after.id
            && root == &after.root
            && name == &after.name
            && variant_of == &after.variant_of
            && props == &after.props
            && rev == &after.rev
            && (preview_rev == &after.preview_rev || preview_components.contains(id))
    })
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
    // Ancestor layers would be restored independently in several phases,
    // changing their blend against the backdrop. Atomic descendants keep the
    // original layer and per-paint commands intact on the ordered canvas.
    if node.blend_mode != BlendMode::Normal
        || group
            .background
            .iter()
            .chain(&group.background_fills)
            .chain(group.strokes.iter().map(|stroke| &stroke.paint))
            .any(|fill| match fill {
                Fill::Solid { blend, .. }
                | Fill::Gradient { blend, .. }
                | Fill::Image { blend, .. }
                | Fill::Video { blend, .. }
                | Fill::Pattern { blend, .. }
                | Fill::Shader { blend, .. } => *blend != BlendMode::Normal,
            })
    {
        return Err(SplitError::Backdrop(node.id));
    }
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
    pub(crate) spec: &'a SplitSpec<'a>,
    pub(crate) phase: SplitPhase,
    pub(crate) clear_target: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{BitmapNode, ComponentDef, GroupNode, ImageFitMode};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn split_whole_instance_wide_budget_reserves_before_decoding_and_accepts_the_boundary() {
        struct Resolver(AtomicUsize);
        impl AssetResolver for Resolver {
            fn resolve(&self, _: AssetId) -> Option<DecodedImage> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Some(DecodedImage::new(Arc::new(vec![255; 4]), 1, 1))
            }
        }
        let mut scene = Scene::new();
        let page = scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .expect("page");
        let master = scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([20.0, 20.0]),
                ..Default::default()
            })))
            .expect("master");
        let asset = AssetId::new();
        for _ in 0..4 {
            let mut child = CanvasNode::new(NodeData::Bitmap(BitmapNode {
                asset,
                natural_size: [1, 1],
                local_size: [20.0, 20.0],
                crop: None,
                fit: ImageFitMode::Fill,
                tint: None,
            }));
            child.parent = Some(master);
            scene.insert(child).expect("child");
        }
        let component = ComponentId::new();
        let mut components = ComponentLibrary::new();
        components.defs.insert(
            component,
            ComponentDef::new(component, master, "Wide master"),
        );
        let mut placed = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [20.0, 20.0],
        }));
        placed.parent = Some(page);
        let moving = scene.insert(placed).expect("placed");
        let inputs = RenderInputs {
            components: &components,
            ..RenderInputs::empty()
        };
        let resolver = Resolver(AtomicUsize::new(0));
        let revision = scene.revision();
        assert!(
            matches!(SplitSpec::prepare_with_budget(&scene, page, moving, &inputs, Some(&resolver), 4, 64), Err(SplitError::ExpansionLimit(id)) if id == moving)
        );
        assert_eq!(
            resolver.0.load(Ordering::Relaxed),
            0,
            "reject the whole wide master before resolving even its first image"
        );
        let accepted =
            SplitSpec::prepare_with_budget(&scene, page, moving, &inputs, Some(&resolver), 5, 64)
                .expect("exact five-node budget");
        assert_eq!(accepted.prepared_node_count(), 5);
        assert_eq!(accepted.prepared_instance_count(), 1);
        assert_eq!(
            resolver.0.load(Ordering::Relaxed),
            1,
            "shared asset is frozen once across four retained children"
        );
        assert_eq!(scene.revision(), revision);
    }
}

#[cfg(test)]
mod retained_storage_tests {
    use super::*;

    #[test]
    fn retained_storage_budget_rejects_before_pixels_and_accepts_the_exact_accounted_boundary() {
        let mut scene = Scene::new();
        let page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        let page_id = page.id;
        scene.insert(page).expect("page");
        let mut moving = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
            0.0,
            0.0,
            8.0,
            8.0,
            fanta_doc::Color::WHITE,
        )));
        moving.parent = Some(page_id);
        let moving_id = moving.id;
        scene.insert(moving).expect("moving");
        let mut renderer = super::super::RasterRenderer::new(32, 32).expect("renderer");
        let viewport = fanta_doc::Viewport::default();
        let inputs = RenderInputs::empty();
        assert!(matches!(
            RetainedTranslationSession::prepare_with_budget(
                &mut renderer,
                &scene,
                page_id,
                moving_id,
                &viewport,
                &inputs,
                None,
                0,
                32 * 32 * 8 - 1,
            ),
            Err(RetainedError::MemoryLimit { .. })
        ));
        let initial = RetainedTranslationSession::prepare(
            &mut renderer,
            &scene,
            page_id,
            moving_id,
            &viewport,
            &inputs,
            None,
            0,
        )
        .expect("initial");
        let metrics = initial.build_metrics();
        let exact = metrics.surface_bytes + metrics.frozen_pixel_bytes + metrics.picture_bytes;
        drop(initial);
        assert!(matches!(
            RetainedTranslationSession::prepare_with_budget(
                &mut renderer,
                &scene,
                page_id,
                moving_id,
                &viewport,
                &inputs,
                None,
                0,
                exact - 1,
            ),
            Err(RetainedError::MemoryLimit { .. })
        ));
        RetainedTranslationSession::prepare_with_budget(
            &mut renderer,
            &scene,
            page_id,
            moving_id,
            &viewport,
            &inputs,
            None,
            0,
            exact,
        )
        .expect("exact accounted boundary");
    }
}
