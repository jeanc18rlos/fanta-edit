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
use super::{BlurKind, RenderInputs, ShadowKind};

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
                spec.requires_ordered_paint |= matches!(&node.data, NodeData::Group(group) if group.clip_size.or(group.local_size).is_some());
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
        if image.width == 0 || image.height == 0 || expected_len != Some(image.pixels().len()) {
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
    #[error(
        "retained rendering requires the same compatible GPU context, format and surface properties"
    )]
    ChangedBackend,
    #[error("the retained GPU backend cannot prove compatible surface or recorder ownership")]
    UnsupportedBackend,
    #[error("the retained GPU context was abandoned")]
    BackendLost,
    #[error("the retained GPU target could not submit its work")]
    BackendSubmission,
    #[error("retained image {0} changed or became unavailable")]
    ChangedAsset(AssetId),
    #[error("retained pixel and picture storage exceeds its {limit} byte budget")]
    MemoryLimit { limit: usize },
    #[error("could not allocate or record a retained frame")]
    Allocation,
    #[error("retained painting reported incomplete artwork or an effect failure")]
    IncompleteArtwork,
}

enum RetainedBackend {
    Cpu,
    #[cfg(feature = "metal")]
    Gpu(Box<RetainedGpuSurface>),
}

#[cfg(feature = "metal")]
#[derive(Clone)]
struct RetainedGpuSurface {
    context: RetainedGpuContext,
    info: skia_safe::ImageInfo,
    properties: skia_safe::SurfaceProps,
    sample_count: Option<usize>,
    format: RetainedGpuFormat,
    // Both Skia GPU engines must stay on their creating render thread.
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[cfg(feature = "metal")]
#[derive(Clone, PartialEq, Eq)]
enum RetainedGpuFormat {
    Metal(skia_safe::gpu::mtl::PixelFormat),
    GraphiteMetal(std::rc::Rc<skia_safe::gpu::graphite::TextureInfo>),
}

#[cfg(feature = "metal")]
#[derive(Clone)]
enum RetainedGpuContext {
    Ganesh(skia_safe::gpu::DirectContext),
    Graphite(RetainedGraphiteContext),
}

#[cfg(feature = "metal")]
struct RetainedGraphiteState {
    recorder: skia_safe::gpu::graphite::Recorder,
    context: skia_safe::gpu::graphite::Context,
    failed: bool,
}

#[cfg(feature = "metal")]
impl RetainedGraphiteState {
    fn ensure_available(&self) -> Result<(), RetainedError> {
        if self.failed {
            Err(RetainedError::BackendSubmission)
        } else if self.context.is_device_lost() {
            Err(RetainedError::BackendLost)
        } else {
            Ok(())
        }
    }
}

/// Owns one Graphite context and recorder on their creating thread. Targets and
/// retained sessions keep this owner alive until their surfaces have dropped.
#[cfg(feature = "metal")]
#[derive(Clone)]
pub struct RetainedGraphiteContext {
    state: std::rc::Rc<std::cell::RefCell<RetainedGraphiteState>>,
    identity: u64,
}

#[cfg(feature = "metal")]
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphiteSubmissionMetrics {
    pub snap_wall: std::time::Duration,
    pub insert_wall: std::time::Duration,
    pub submit_wait_wall: std::time::Duration,
}

#[cfg(feature = "metal")]
impl RetainedGraphiteContext {
    pub fn new(mut context: skia_safe::gpu::graphite::Context) -> Result<Self, RetainedError> {
        if context.is_device_lost() {
            return Err(RetainedError::BackendLost);
        }
        let recorder = context
            .make_recorder(None)
            .ok_or(RetainedError::Allocation)?;
        if recorder.backend() != skia_safe::gpu::BackendApi::Metal {
            return Err(RetainedError::UnsupportedBackend);
        }
        static NEXT_IDENTITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Ok(Self {
            state: std::rc::Rc::new(std::cell::RefCell::new(RetainedGraphiteState {
                recorder,
                context,
                failed: false,
            })),
            identity: NEXT_IDENTITY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        })
    }

    pub fn wrap_top_left(
        &mut self,
        texture: &skia_safe::gpu::graphite::BackendTexture,
        color_type: skia_safe::ColorType,
        color_space: Option<skia_safe::ColorSpace>,
    ) -> Result<RetainedGpuTarget, RetainedError> {
        if !texture.is_valid()
            || texture.backend() != skia_safe::gpu::BackendApi::Metal
            || !matches!(
                color_type,
                skia_safe::ColorType::BGRA8888 | skia_safe::ColorType::RGBA8888
            )
        {
            return Err(RetainedError::ChangedBackend);
        }
        let format = texture.info();
        if !format.is_valid() {
            return Err(RetainedError::ChangedBackend);
        }
        let mut surface = {
            let mut state = self
                .state
                .try_borrow_mut()
                .map_err(|_| RetainedError::UnsupportedBackend)?;
            state.ensure_available()?;
            skia_safe::gpu::graphite::surfaces::wrap_backend_texture(
                &mut state.recorder,
                texture,
                color_type,
                color_space,
                None,
            )
            .ok_or(RetainedError::ChangedBackend)?
        };
        let backend = RetainedGpuSurface::capture_graphite(
            &mut surface,
            self.clone(),
            std::rc::Rc::new(format),
        )?;
        Ok(RetainedGpuTarget { surface, backend })
    }

    pub fn wrap_image(
        &mut self,
        texture: &skia_safe::gpu::graphite::BackendTexture,
        color_type: skia_safe::ColorType,
        alpha_type: skia_safe::AlphaType,
        color_space: Option<skia_safe::ColorSpace>,
    ) -> Result<skia_safe::Image, RetainedError> {
        if !texture.is_valid() || texture.backend() != skia_safe::gpu::BackendApi::Metal {
            return Err(RetainedError::ChangedBackend);
        }
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| RetainedError::UnsupportedBackend)?;
        state.ensure_available()?;
        skia_safe::gpu::graphite::images::wrap_texture(
            &mut state.recorder,
            texture,
            color_type,
            alpha_type,
            color_space,
        )
        .ok_or(RetainedError::ChangedBackend)
    }

    pub fn submit_and_wait(&mut self) -> Result<(), RetainedError> {
        self.submit_and_wait_profiled().map(|_| ())
    }

    pub fn submit_and_wait_profiled(&mut self) -> Result<GraphiteSubmissionMetrics, RetainedError> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| RetainedError::UnsupportedBackend)?;
        state.ensure_available()?;
        let result = (|| {
            let started = std::time::Instant::now();
            let mut recording = state.recorder.snap().ok_or(RetainedError::Allocation)?;
            let snap_wall = started.elapsed();
            let started = std::time::Instant::now();
            let status = state.context.insert_recording(
                &skia_safe::gpu::graphite::InsertRecordingInfo::new(&mut recording),
            );
            if status != skia_safe::gpu::graphite::InsertStatus::Success {
                return Err(RetainedError::BackendSubmission);
            }
            let insert_wall = started.elapsed();
            let started = std::time::Instant::now();
            if !state.context.submit_and_wait() {
                return Err(RetainedError::BackendSubmission);
            }
            let submit_wait_wall = started.elapsed();
            state.ensure_available()?;
            Ok(GraphiteSubmissionMetrics {
                snap_wall,
                insert_wall,
                submit_wait_wall,
            })
        })();
        if result.is_err() {
            // Failed recording insertion may leave pending work referencing
            // this frame's borrowed textures. Never replay it in a later frame.
            state.failed = true;
        }
        result
    }

    /// Lends the context and recorder for native calls skia-safe does not
    /// wrap, such as Graphite's cache budgets and cleanup.
    pub fn with_native<R>(
        &mut self,
        use_native: impl FnOnce(
            &mut skia_safe::gpu::graphite::Context,
            &mut skia_safe::gpu::graphite::Recorder,
        ) -> R,
    ) -> Result<R, RetainedError> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| RetainedError::UnsupportedBackend)?;
        state.ensure_available()?;
        let RetainedGraphiteState {
            recorder, context, ..
        } = &mut *state;
        Ok(use_native(context, recorder))
    }

    fn ensure_available(&self) -> Result<(), RetainedError> {
        let state = self
            .state
            .try_borrow()
            .map_err(|_| RetainedError::UnsupportedBackend)?;
        state.ensure_available()
    }

    fn owns_surface(&self, surface: &mut skia_safe::Surface) -> Result<(), RetainedError> {
        use skia_safe::wrapper::PointerWrapper as _;
        let state = self
            .state
            .try_borrow()
            .map_err(|_| RetainedError::UnsupportedBackend)?;
        state.ensure_available()?;
        let recorder = surface
            .canvas()
            .recorder()
            .ok_or(RetainedError::UnsupportedBackend)?;
        if !std::ptr::eq(state.recorder.inner(), recorder.inner()) {
            return Err(RetainedError::ChangedBackend);
        }
        Ok(())
    }
}

#[cfg(feature = "metal")]
impl RetainedGpuFormat {
    fn capture(format: skia_safe::gpu::BackendFormat) -> Result<Self, RetainedError> {
        format
            .as_mtl_format()
            .map(Self::Metal)
            .ok_or(RetainedError::ChangedBackend)
    }
}

#[cfg(feature = "metal")]
impl RetainedGpuSurface {
    fn capture(surface: &mut skia_safe::Surface) -> Result<Self, RetainedError> {
        let mut context = surface
            .direct_context()
            .ok_or(RetainedError::ChangedBackend)?;
        if context.abandoned() {
            return Err(RetainedError::BackendLost);
        }
        let info = surface.image_info();
        // The storage estimate assumes four bytes per pixel; it excludes driver
        // allocation, caches and snapshots retained by the caller.
        if !matches!(
            info.color_type(),
            skia_safe::ColorType::BGRA8888 | skia_safe::ColorType::RGBA8888
        ) || info.alpha_type() != skia_safe::AlphaType::Premul
            || info.color_space().is_some()
        {
            return Err(RetainedError::ChangedBackend);
        }
        let target = skia_safe::gpu::surfaces::get_backend_render_target(
            surface,
            skia_safe::surface::BackendHandleAccess::FlushRead,
        )
        .ok_or(RetainedError::ChangedBackend)?;
        if target.sample_count() > 1 {
            return Err(RetainedError::ChangedBackend);
        }
        Ok(Self {
            context: RetainedGpuContext::Ganesh(context),
            info,
            properties: *surface.props(),
            sample_count: Some(target.sample_count()),
            format: RetainedGpuFormat::capture(target.backend_format())?,
            _thread_bound: std::marker::PhantomData,
        })
    }

    fn capture_graphite(
        surface: &mut skia_safe::Surface,
        context: RetainedGraphiteContext,
        format: std::rc::Rc<skia_safe::gpu::graphite::TextureInfo>,
    ) -> Result<Self, RetainedError> {
        context.owns_surface(surface)?;
        let info = surface.image_info();
        if !matches!(
            info.color_type(),
            skia_safe::ColorType::BGRA8888 | skia_safe::ColorType::RGBA8888
        ) || info.alpha_type() != skia_safe::AlphaType::Premul
        {
            return Err(RetainedError::ChangedBackend);
        }
        Ok(Self {
            context: RetainedGpuContext::Graphite(context),
            info,
            properties: *surface.props(),
            sample_count: None,
            format: RetainedGpuFormat::GraphiteMetal(format),
            _thread_bound: std::marker::PhantomData,
        })
    }

    fn validate_surface(&mut self, surface: &mut skia_safe::Surface) -> Result<(), RetainedError> {
        match &self.context {
            RetainedGpuContext::Ganesh(_) => self.validate(&mut Self::capture(surface)?),
            RetainedGpuContext::Graphite(context) => {
                context.owns_surface(surface)?;
                if self.info != surface.image_info() || self.properties != *surface.props() {
                    return Err(RetainedError::ChangedBackend);
                }
                Ok(())
            }
        }
    }

    fn ensure_available(&mut self) -> Result<(), RetainedError> {
        match &mut self.context {
            RetainedGpuContext::Ganesh(context) => {
                if context.abandoned() {
                    Err(RetainedError::BackendLost)
                } else {
                    Ok(())
                }
            }
            RetainedGpuContext::Graphite(context) => context.ensure_available(),
        }
    }

    fn validate(&mut self, current: &mut Self) -> Result<(), RetainedError> {
        self.ensure_available()?;
        current.ensure_available()?;
        let same_context = match (&self.context, &current.context) {
            (RetainedGpuContext::Ganesh(left), RetainedGpuContext::Ganesh(right)) => {
                left.id() == right.id()
            }
            (RetainedGpuContext::Graphite(left), RetainedGpuContext::Graphite(right)) => {
                left.identity == right.identity && std::rc::Rc::ptr_eq(&left.state, &right.state)
            }
            _ => false,
        };
        if !same_context
            || self.info != current.info
            || self.properties != current.properties
            || self.sample_count != current.sample_count
            || self.format != current.format
        {
            return Err(RetainedError::ChangedBackend);
        }
        Ok(())
    }
}

/// A GPU target wrapped with a known TopLeft origin. Its surface cannot be
/// replaced through this API, so context/format/origin validation does not need
/// a flushing backend-handle query on every frame. The host must retain the
/// external render-target resources for this wrapper's lifetime. Currently only
/// Metal is supported, with the `metal` feature; other backends reject explicitly.
#[cfg(feature = "metal")]
pub struct RetainedGpuTarget {
    surface: skia_safe::Surface,
    backend: RetainedGpuSurface,
}

#[cfg(feature = "metal")]
impl RetainedGpuTarget {
    pub fn wrap_top_left(
        context: &mut skia_safe::gpu::DirectContext,
        target: &skia_safe::gpu::BackendRenderTarget,
        color_type: skia_safe::ColorType,
        properties: Option<&skia_safe::SurfaceProps>,
    ) -> Result<Self, RetainedError> {
        if context.abandoned() {
            return Err(RetainedError::BackendLost);
        }
        if target.sample_count() > 1
            || !matches!(
                color_type,
                skia_safe::ColorType::BGRA8888 | skia_safe::ColorType::RGBA8888
            )
        {
            return Err(RetainedError::ChangedBackend);
        }
        let mut surface = skia_safe::gpu::surfaces::wrap_backend_render_target(
            context,
            target,
            skia_safe::gpu::SurfaceOrigin::TopLeft,
            color_type,
            None,
            properties,
        )
        .ok_or(RetainedError::ChangedBackend)?;
        let backend = RetainedGpuSurface::capture(&mut surface)?;
        Ok(Self { surface, backend })
    }

    pub fn image_info(&self) -> skia_safe::ImageInfo {
        self.backend.info.clone()
    }

    pub fn canvas(&mut self) -> Result<&skia_safe::Canvas, RetainedError> {
        self.backend.ensure_available()?;
        Ok(self.surface.canvas())
    }

    pub fn image_snapshot(&mut self) -> Result<skia_safe::Image, RetainedError> {
        self.backend.ensure_available()?;
        retained_surface_snapshot(&mut self.surface)
    }

    /// Submit the target's work before exposing its external texture to a
    /// consumer. Callers choose the same synchronization policy as normal paint.
    pub fn flush_and_submit(
        &mut self,
        sync_cpu: skia_safe::gpu::SyncCpu,
    ) -> Result<(), RetainedError> {
        self.backend.ensure_available()?;
        match &mut self.backend.context {
            RetainedGpuContext::Ganesh(context) => {
                context.flush_surface(&mut self.surface);
                if !context.submit(sync_cpu) {
                    return Err(RetainedError::BackendSubmission);
                }
            }
            RetainedGpuContext::Graphite(context) => context.submit_and_wait()?,
        }
        self.backend.ensure_available()
    }
}

impl RetainedBackend {
    fn validate_surface(&mut self, surface: &mut skia_safe::Surface) -> Result<(), RetainedError> {
        match self {
            Self::Cpu => {
                #[cfg(feature = "metal")]
                if surface.canvas().recorder().is_some() {
                    return Err(RetainedError::ChangedBackend);
                }
                if surface.recording_context().is_none() {
                    Ok(())
                } else {
                    Err(RetainedError::ChangedBackend)
                }
            }
            #[cfg(feature = "metal")]
            Self::Gpu(backend) => backend.validate_surface(surface),
        }
    }

    fn validate_cpu(&mut self) -> Result<(), RetainedError> {
        match self {
            Self::Cpu => Ok(()),
            #[cfg(feature = "metal")]
            Self::Gpu(_) => Err(RetainedError::ChangedBackend),
        }
    }

    #[cfg(feature = "metal")]
    fn validate_gpu(&mut self, target: &mut RetainedGpuTarget) -> Result<(), RetainedError> {
        match self {
            Self::Gpu(backend) => backend.validate(&mut target.backend),
            Self::Cpu => Err(RetainedError::ChangedBackend),
        }
    }

    fn ensure_available(&mut self) -> Result<(), RetainedError> {
        match self {
            Self::Cpu => Ok(()),
            #[cfg(feature = "metal")]
            Self::Gpu(backend) => backend.ensure_available(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RetainedBuildMetrics {
    /// Entire CPU preparation, including backend validation and surface allocation.
    /// GPU composition, submission and completion remain the caller's responsibility.
    pub build_micros: u64,
    pub backend_validation_micros: u64,
    pub surface_allocation_micros: u64,
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
    pub backend_validation_micros: u64,
    pub validation_micros: u64,
    pub below_copy_micros: u64,
    pub middle_micros: u64,
    pub above_replay_micros: u64,
    /// Entire CPU command preparation, including backend and semantic validation.
    /// The caller must measure GPU composition and synchronization separately.
    pub frame_micros: u64,
    pub middle: super::RenderMetrics,
}

pub struct RetainedFrame {
    pub image: skia_safe::Image,
    pub metrics: RetainedFrameMetrics,
}

/// An opt-in renderer for one fixed-view translation gesture. It owns
/// its frame canvas so cached device-space paint cannot be replayed under a
/// different caller matrix or clip. It is not connected to the live worker.
/// Any failed frame disables reuse until the caller prepares a new session.
///
/// `font_generation` must change whenever the caller's font environment does.
/// The storage budget covers two pixel surfaces, retained decoded images and
/// recorded Picture bytes; the existing prepared node/depth limits apply too.
/// Picture accounting is approximate and does not bound GPU allocation or total
/// Skia scratch memory. Callers must release old frame images to avoid snapshot
/// copies. GPU images may only be composed on their validated target context.
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
    backend: RetainedBackend,
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

    /// Allocate private surfaces compatible with the actual GPU target. The
    /// target's pixels and canvas state are untouched. Call `render_for_target`
    /// for every frame; the CPU-only `render` entry rejects this backend.
    #[cfg(feature = "metal")]
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_for_target(
        renderer: &mut super::RasterRenderer,
        target: &mut RetainedGpuTarget,
        scene: &Scene,
        page_root: NodeId,
        moving: NodeId,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
    ) -> Result<Self, RetainedError> {
        let started = std::time::Instant::now();
        target.backend.ensure_available()?;
        if target.backend.info.width() != renderer.width() as i32
            || target.backend.info.height() != renderer.height() as i32
        {
            return Err(RetainedError::ChangedBackend);
        }
        let backend = RetainedBackend::Gpu(Box::new(target.backend.clone()));
        let backend_validation_micros = elapsed_micros(started);
        let size = (renderer.width() as i32, renderer.height() as i32);
        Self::prepare_internal(
            renderer,
            scene,
            page_root,
            moving,
            viewport,
            inputs,
            resolver,
            font_generation,
            RETAINED_PIXEL_AND_PICTURE_BUDGET,
            backend,
            || target.surface.new_surface_with_dimensions(size),
            started,
            backend_validation_micros,
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
        let size = (renderer.width() as i32, renderer.height() as i32);
        Self::prepare_internal(
            renderer,
            scene,
            page_root,
            moving,
            viewport,
            inputs,
            resolver,
            font_generation,
            byte_limit,
            RetainedBackend::Cpu,
            || skia_safe::surfaces::raster_n32_premul(size),
            std::time::Instant::now(),
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_internal(
        renderer: &mut super::RasterRenderer,
        scene: &Scene,
        page_root: NodeId,
        moving: NodeId,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
        byte_limit: usize,
        mut backend: RetainedBackend,
        mut allocate: impl FnMut() -> Option<skia_safe::Surface>,
        started: std::time::Instant,
        mut backend_validation_micros: u64,
    ) -> Result<Self, RetainedError> {
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
                total.checked_add(image.pixels().len())
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
        let allocation_started = std::time::Instant::now();
        let mut below_surface = allocate().ok_or(RetainedError::Allocation)?;
        let mut surface_allocation_micros = elapsed_micros(allocation_started);
        let validation_started = std::time::Instant::now();
        backend.validate_surface(&mut below_surface)?;
        backend_validation_micros =
            backend_validation_micros.saturating_add(elapsed_micros(validation_started));
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
        let below = retained_surface_snapshot(&mut below_surface)?;
        let below_micros = elapsed_micros(below_started);
        let mut recorder = skia_safe::PictureRecorder::new();
        let recording = recorder.begin_recording(
            skia_safe::Rect::from_wh(size.0 as f32, size.1 as f32),
            false,
        );
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
        let allocation_started = std::time::Instant::now();
        let mut output = allocate().ok_or(RetainedError::Allocation)?;
        surface_allocation_micros =
            surface_allocation_micros.saturating_add(elapsed_micros(allocation_started));
        let validation_started = std::time::Instant::now();
        backend.validate_surface(&mut output)?;
        backend_validation_micros =
            backend_validation_micros.saturating_add(elapsed_micros(validation_started));
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
            backend,
            build_metrics: RetainedBuildMetrics {
                build_micros: elapsed_micros(started),
                backend_validation_micros,
                surface_allocation_micros,
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
        self.render_with_backend(
            renderer,
            RetainedBackend::validate_cpu,
            scene,
            viewport,
            inputs,
            resolver,
            font_generation,
        )
    }

    /// Validate actual target affinity before semantic revision advancement or
    /// output mutation. The returned GPU image is only drawable on that context;
    /// callers own composition and synchronization. Target pixels/state are untouched.
    #[cfg(feature = "metal")]
    #[allow(clippy::too_many_arguments)]
    pub fn render_for_target(
        &mut self,
        renderer: &mut super::RasterRenderer,
        target: &mut RetainedGpuTarget,
        scene: &Scene,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
    ) -> Result<RetainedFrame, RetainedError> {
        self.render_with_backend(
            renderer,
            |backend| backend.validate_gpu(target),
            scene,
            viewport,
            inputs,
            resolver,
            font_generation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_with_backend(
        &mut self,
        renderer: &mut super::RasterRenderer,
        validate_backend: impl FnOnce(&mut RetainedBackend) -> Result<(), RetainedError>,
        scene: &Scene,
        viewport: &fanta_doc::Viewport,
        inputs: &RenderInputs,
        resolver: Option<&dyn AssetResolver>,
        font_generation: u64,
    ) -> Result<RetainedFrame, RetainedError> {
        let started = std::time::Instant::now();
        if self.disabled {
            return Err(RetainedError::Disabled);
        }
        let result = validate_backend(&mut self.backend).and_then(|()| {
            let backend_validation_micros = elapsed_micros(started);
            let mut frame =
                self.render_checked(renderer, scene, viewport, inputs, resolver, font_generation)?;
            self.backend.ensure_available()?;
            frame.metrics.backend_validation_micros = backend_validation_micros;
            frame.metrics.frame_micros = elapsed_micros(started);
            Ok(frame)
        });
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
                || !current.shares_pixels(frozen)
            {
                return Err(RetainedError::ChangedAsset(*asset));
            }
        }
        // Asset resolution is caller code and can abandon the context after the initial check.
        self.backend.ensure_available()?;
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
        let image = retained_surface_snapshot(&mut self.output)?;
        Ok(RetainedFrame {
            image,
            metrics: RetainedFrameMetrics {
                backend_validation_micros: 0,
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

fn retained_surface_snapshot(
    surface: &mut skia_safe::Surface,
) -> Result<skia_safe::Image, RetainedError> {
    super::layer_cache::immutable_surface_snapshot(surface).ok_or(RetainedError::Allocation)
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
    let fixed_paint_box = group
        .clip_size
        .or(group.local_size)
        .is_some_and(|size| size.into_iter().all(|value| value > 0.0));
    // frame_box_bounds uses this explicit box even when children may overflow.
    // Without it the moving child's union bounds can change cached ancestor paint.
    if (has_paint && !page_clear_only && !fixed_paint_box)
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
                Some(DecodedImage::new(&[255; 4], 1, 1))
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

    fn storage_fixture() -> (Scene, NodeId, NodeId) {
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
        (scene, page_id, moving_id)
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[allow(unsafe_code)]
    fn graphite_test_context(
        device: &metal::Device,
        queue: &metal::CommandQueue,
    ) -> RetainedGraphiteContext {
        use metal::foreign_types::ForeignType as _;
        // SAFETY: the test retains the device and queue through every target,
        // recorder, context and completed submission; the binding also retains them.
        let backend = unsafe {
            skia_safe::gpu::graphite::mtl::BackendContext::new(
                device.as_ptr().cast(),
                queue.as_ptr().cast(),
            )
        };
        let context = skia_safe::gpu::graphite::mtl::context_factory::make_metal(&backend, None)
            .expect("actual Graphite Metal context");
        RetainedGraphiteContext::new(context).expect("owned recorder")
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[allow(unsafe_code)]
    fn graphite_test_target(
        context: &mut RetainedGraphiteContext,
        device: &metal::Device,
    ) -> (metal::Texture, RetainedGpuTarget) {
        graphite_test_target_sized(context, device, (32, 32))
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[allow(unsafe_code)]
    fn graphite_test_target_sized(
        context: &mut RetainedGraphiteContext,
        device: &metal::Device,
        dimensions: (i32, i32),
    ) -> (metal::Texture, RetainedGpuTarget) {
        use metal::foreign_types::ForeignType as _;
        assert!(dimensions.0 > 0 && dimensions.1 > 0);
        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_texture_type(metal::MTLTextureType::D2);
        descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        descriptor.set_width(dimensions.0 as u64);
        descriptor.set_height(dimensions.1 as u64);
        descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        let texture = device.new_texture(&descriptor);
        // SAFETY: the returned texture lease stays alive until after its target
        // is dropped and all GPU work completes on the owning test thread.
        let backend = unsafe {
            skia_safe::gpu::graphite::mtl::backend_textures::make_metal(
                dimensions,
                texture.as_ptr().cast(),
            )
        };
        let target = context
            .wrap_top_left(&backend, skia_safe::ColorType::BGRA8888, None)
            .expect("wrapped Graphite target");
        // Binding the lease first makes the target drop before the texture,
        // including assertion-failure unwinding in the native tests.
        (texture, target)
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    fn graphite_target_pixels(target: &mut RetainedGpuTarget) -> Vec<u8> {
        let RetainedGpuContext::Graphite(owner) = &target.backend.context else {
            panic!("Graphite target");
        };
        let info = skia_safe::ImageInfo::new(
            target.surface.image_info().dimensions(),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let mut pixels = vec![0; info.compute_min_byte_size()];
        let mut state = owner.state.try_borrow_mut().expect("idle owner");
        assert!(state.context.read_pixels(
            &mut target.surface,
            &info,
            &mut pixels,
            info.min_row_bytes(),
            (0, 0),
        ));
        pixels
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_retained_frames_match_fresh_pixels_and_refuse_foreign_recorders() {
        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let state = std::rc::Rc::downgrade(&owner.state);
            let (_texture, mut target) = graphite_test_target(&mut owner, &device);
            let (_full_texture, mut full_target) = graphite_test_target(&mut owner, &device);
            drop(owner);
            assert!(
                state.upgrade().is_some(),
                "targets retain their recorder and context"
            );
            let (mut scene, page, moving) = storage_fixture();
            let viewport = fanta_doc::Viewport::default();
            let inputs = RenderInputs::empty();
            let mut renderer = super::super::RasterRenderer::new(32, 32).expect("renderer");
            let mut full = super::super::RasterRenderer::new(32, 32).expect("full renderer");
            let mut session = RetainedTranslationSession::prepare_for_target(
                &mut renderer,
                &mut target,
                &scene,
                page,
                moving,
                &viewport,
                &inputs,
                None,
                0,
            )
            .expect("compatible retained preparation");
            let mut original = None;
            for offset in [0.0, 4.5, -2.0] {
                scene
                    .set_transform(moving, fanta_doc::Transform2D::translation(offset, 0.0))
                    .expect("move selected layer");
                let frame = session
                    .render_for_target(
                        &mut renderer,
                        &mut target,
                        &scene,
                        &viewport,
                        &inputs,
                        None,
                        0,
                    )
                    .expect("retained frame");
                let mut paint = skia_safe::Paint::default();
                paint.set_blend_mode(skia_safe::BlendMode::Src);
                target.canvas().expect("target canvas").draw_image(
                    &frame.image,
                    (0, 0),
                    Some(&paint),
                );
                full.render_to_canvas(
                    full_target.canvas().expect("full canvas"),
                    32,
                    32,
                    &scene,
                    &viewport,
                    Some(page),
                    &inputs,
                );
                target
                    .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                    .expect("both surfaces completed");
                let retained_pixels = graphite_target_pixels(&mut target);
                assert_eq!(retained_pixels, graphite_target_pixels(&mut full_target));
                if original.is_none() {
                    original = Some((frame.image, retained_pixels));
                }
            }
            let (original_image, original_pixels) = original.expect("initial retained frame");
            let mut paint = skia_safe::Paint::default();
            paint.set_blend_mode(skia_safe::BlendMode::Src);
            target
                .canvas()
                .expect("canvas")
                .draw_image(original_image, (0, 0), Some(&paint));
            target
                .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                .expect("frozen image completed");
            assert_eq!(
                graphite_target_pixels(&mut target),
                original_pixels,
                "the first copy stays frozen while the retained output is reused"
            );
            let mut foreign_owner = graphite_test_context(&device, &queue);
            let (_foreign_texture, mut foreign) = graphite_test_target(&mut foreign_owner, &device);
            foreign
                .canvas()
                .expect("foreign canvas")
                .clear(skia_safe::Color::RED);
            foreign
                .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                .expect("foreign sentinel completed");
            let sentinel = graphite_target_pixels(&mut foreign);
            assert!(matches!(
                session.render_for_target(
                    &mut renderer,
                    &mut foreign,
                    &scene,
                    &viewport,
                    &inputs,
                    None,
                    0
                ),
                Err(RetainedError::ChangedBackend)
            ));
            assert_eq!(graphite_target_pixels(&mut foreign), sentinel);
            drop(session);
            drop(target);
            drop(full_target);
            assert!(
                state.upgrade().is_none(),
                "surface owners release after the final target/session"
            );
        });
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_large_rectangle_translation_preserves_dark_canvas_clips_and_foreground() {
        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let (_texture, mut target) = graphite_test_target(&mut owner, &device);
            let (_full_texture, mut full_target) = graphite_test_target(&mut owner, &device);
            let mut scene = Scene::new();
            let mut page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
                background: Some(Fill::solid(fanta_doc::Color::rgb(16, 18, 22))),
                ..Default::default()
            }));
            page.name = "Dark canvas".into();
            let page = scene.insert(page).expect("page");
            let mut stroke = Stroke::solid(fanta_doc::Color::rgb(30, 210, 70), 2.0);
            stroke.align = fanta_doc::StrokeAlign::Inside;
            let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
                clip_size: Some([28.0, 28.0]),
                background: Some(Fill::solid(fanta_doc::Color::rgb(25, 35, 45))),
                strokes: [stroke].into_iter().collect(),
                ..Default::default()
            }));
            frame.parent = Some(page);
            frame.transform = fanta_doc::Transform2D::translation(-14.0, -14.0);
            frame.effects.push(fanta_doc::Shadow {
                kind: ShadowKind::Inner,
                color: fanta_doc::Color::rgba(0, 0, 0, 80),
                blur: 1.0,
                spread: 0.0,
                offset: [1.0, 1.0],
                show_behind_node: false,
            });
            let frame = scene.insert(frame).expect("clipping frame");
            let mut rectangle =
                CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
                    0.0,
                    0.0,
                    5000.0,
                    8.0,
                    fanta_doc::Color::rgb(128, 128, 128),
                )));
            rectangle.parent = Some(frame);
            rectangle.transform = fanta_doc::Transform2D::translation(-2500.0, -2.0);
            let moving = scene.insert(rectangle).expect("large selected rectangle");
            let mut masked = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
            masked.parent = Some(page);
            masked.index = scene.next_child_index(Some(page));
            let masked = scene.insert(masked).expect("closed mask group");
            let mut mask = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
                -6.0,
                -6.0,
                12.0,
                12.0,
                fanta_doc::Color::WHITE,
            )));
            mask.parent = Some(masked);
            mask.is_mask = true;
            scene.insert(mask).expect("mask");
            let mut foreground =
                CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
                    -10.0,
                    -10.0,
                    20.0,
                    20.0,
                    fanta_doc::Color::rgba(20, 70, 190, 160),
                )));
            foreground.parent = Some(masked);
            foreground.index = scene.next_child_index(Some(masked));
            foreground.blend_mode = BlendMode::Screen;
            scene.insert(foreground).expect("foreground");
            let viewport = fanta_doc::Viewport::default();
            let inputs = RenderInputs::empty();
            let mut renderer = super::super::RasterRenderer::new(32, 32).expect("renderer");
            let mut full = super::super::RasterRenderer::new(32, 32).expect("full renderer");
            renderer.set_pixel_snap_pan(true);
            full.set_pixel_snap_pan(true);
            let mut session = RetainedTranslationSession::prepare_for_target(
                &mut renderer,
                &mut target,
                &scene,
                page,
                moving,
                &viewport,
                &inputs,
                None,
                0,
            )
            .expect("eligible clipped rectangle");
            for offset in [-2.0, 3.25, 12.25, -10.0, -2.0] {
                scene
                    .set_transform(moving, fanta_doc::Transform2D::translation(-2500.0, offset))
                    .expect("move rectangle");
                let frame = session
                    .render_for_target(
                        &mut renderer,
                        &mut target,
                        &scene,
                        &viewport,
                        &inputs,
                        None,
                        0,
                    )
                    .expect("eligible translation");
                let mut paint = skia_safe::Paint::default();
                paint.set_blend_mode(skia_safe::BlendMode::Src);
                target
                    .canvas()
                    .expect("target")
                    .draw_image(&frame.image, (0, 0), Some(&paint));
                let metrics = full.render_to_canvas(
                    full_target.canvas().expect("full target"),
                    32,
                    32,
                    &scene,
                    &viewport,
                    Some(page),
                    &inputs,
                );
                assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
                target
                    .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                    .expect("complete both frames");
                let actual = graphite_target_pixels(&mut target);
                let expected = graphite_target_pixels(&mut full_target);
                let maximum = actual
                    .iter()
                    .zip(&expected)
                    .map(|(actual, expected)| actual.abs_diff(*expected))
                    .max()
                    .expect("pixels");
                assert!(
                    maximum <= 2,
                    "translation {offset}: channel difference {maximum}"
                );
                assert_eq!(
                    actual.get(..4),
                    Some([16, 18, 22, 255].as_slice()),
                    "dark page remains opaque outside the clipping frame"
                );
            }
            let previous_revision = session.spec.scene_revision;
            let mut resized = viewport;
            resized.center[0] = 1.0;
            assert!(matches!(
                session.render_for_target(
                    &mut renderer,
                    &mut target,
                    &scene,
                    &resized,
                    &inputs,
                    None,
                    0
                ),
                Err(RetainedError::ChangedFrame)
            ));
            assert_eq!(session.spec.scene_revision, previous_revision);
        });
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_camera_cache_refines_filter_edges_uncovered_pans_and_exact_zoom() {
        fn refine(
            renderer: &mut super::super::RasterRenderer,
            target: &mut RetainedGpuTarget,
            scene: &Scene,
            page: NodeId,
            viewport: &fanta_doc::Viewport,
            inputs: &RenderInputs,
            interacting: bool,
        ) -> super::super::CameraCacheResult {
            for _ in 0..32 {
                let result = renderer.render_camera_to_canvas(
                    target.canvas().expect("camera canvas"),
                    32,
                    32,
                    scene,
                    viewport,
                    Some(page),
                    inputs,
                    1,
                    interacting,
                );
                assert!(!result.metrics.incomplete_artwork && !result.metrics.effect_failed);
                assert!(result.complete_detail && !result.cancelled);
                assert!(!result.cache.overview_drawn);
                assert!(
                    result.cache.tiles_built as usize
                        <= super::super::camera_cache::MANDATORY_TILES
                            + super::super::camera_cache::PREFETCH_TILES
                );
                target
                    .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                    .expect("camera frame completed");
                assert!(
                    graphite_target_pixels(target)
                        .chunks_exact(4)
                        .all(|pixel| pixel.last() == Some(&255)),
                    "exact pictures and retained tiles keep the dark page opaque"
                );
                if !result.needs_refresh {
                    assert!(result.cache.tiles_drawn > 0);
                    return result;
                }
                assert!(result.needs_refresh);
                let presented = graphite_target_pixels(target);
                let maintenance = renderer.maintain_camera_cache(
                    target.canvas().expect("maintenance canvas"),
                    32,
                    32,
                    scene,
                    viewport,
                    Some(page),
                    inputs,
                    1,
                    &|| false,
                );
                assert!(!maintenance.cancelled);
                assert!(maintenance.cache.scene_walks <= 1);
                assert!(maintenance.cache.tiles_built <= 1);
                target
                    .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                    .expect("maintenance completed");
                assert_eq!(graphite_target_pixels(target), presented);
            }
            panic!("bounded native camera tiles must settle");
        }

        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let (_texture, mut target) = graphite_test_target(&mut owner, &device);
            let (_full_texture, mut full_target) = graphite_test_target(&mut owner, &device);
            let mut scene = Scene::new();
            let page = scene
                .insert(CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
                    background: Some(Fill::solid(fanta_doc::Color::rgb(16, 18, 22))),
                    ..Default::default()
                })))
                .expect("dark page");
            for center in [512.0, 2048.0] {
                let mut rectangle =
                    CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
                        -9.0,
                        -10.0,
                        18.0,
                        20.0,
                        fanta_doc::Color::rgba(180, 60, 105, 210),
                    )));
                rectangle.parent = Some(page);
                rectangle.index = scene.next_child_index(Some(page));
                rectangle.transform = fanta_doc::Transform2D::translation(center, 0.0);
                rectangle.blurs.push(fanta_doc::Blur::layer(3.0));
                rectangle.effects.push(fanta_doc::Shadow {
                    kind: ShadowKind::Drop,
                    color: fanta_doc::Color::rgba(0, 0, 0, 160),
                    blur: 5.0,
                    spread: 1.0,
                    offset: [3.0, 2.0],
                    show_behind_node: false,
                });
                scene
                    .insert(rectangle)
                    .expect("filtered tile-boundary shape");
                let mut foreground =
                    CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
                        -5.5,
                        -6.5,
                        11.0,
                        13.0,
                        fanta_doc::Color::rgba(35, 130, 220, 120),
                    )));
                foreground.parent = Some(page);
                foreground.index = scene.next_child_index(Some(page));
                foreground.transform = fanta_doc::Transform2D::translation(center, 0.0);
                foreground.blend_mode = BlendMode::Screen;
                scene.insert(foreground).expect("blended foreground");
            }
            let inputs = RenderInputs::empty();
            let mut renderer = super::super::RasterRenderer::new(32, 32).expect("camera renderer");
            let mut full = super::super::RasterRenderer::new(32, 32).expect("full renderer");
            renderer.set_pixel_snap_pan(true);
            full.set_pixel_snap_pan(true);
            for (center, zoom) in [(512.0, 1.0), (2048.0, 1.0), (512.0, 0.73)] {
                let viewport = fanta_doc::Viewport {
                    center: [center, 0.0],
                    zoom,
                };
                if zoom != 1.0 {
                    refine(
                        &mut renderer,
                        &mut target,
                        &scene,
                        page,
                        &viewport,
                        &inputs,
                        true,
                    );
                }
                refine(
                    &mut renderer,
                    &mut target,
                    &scene,
                    page,
                    &viewport,
                    &inputs,
                    false,
                );
                let warm = refine(
                    &mut renderer,
                    &mut target,
                    &scene,
                    page,
                    &viewport,
                    &inputs,
                    false,
                );
                assert_eq!(warm.metrics.nodes_visited, 0);
                assert_eq!(warm.cache.scene_walks, 0);
                assert_eq!(warm.cache.tiles_built, 0);
                let metrics = full.render_to_canvas(
                    full_target.canvas().expect("full canvas"),
                    32,
                    32,
                    &scene,
                    &viewport,
                    Some(page),
                    &inputs,
                );
                assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
                full_target
                    .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                    .expect("full frame completed");
                let actual = graphite_target_pixels(&mut target);
                let expected = graphite_target_pixels(&mut full_target);
                let maximum = actual
                    .iter()
                    .zip(&expected)
                    .map(|(actual, expected)| actual.abs_diff(*expected))
                    .max()
                    .expect("native pixels");
                assert!(
                    maximum <= 2,
                    "camera center {center}, zoom {zoom}: channel difference {maximum}"
                );
            }
        });
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_camera_tiles_preserve_the_complete_rounded_clip_fixture() {
        use super::super::camera_cache::tests::{HEIGHT, WIDTH, fixture};

        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let dimensions = (WIDTH as i32, HEIGHT as i32);
            let (_texture, mut target) =
                graphite_test_target_sized(&mut owner, &device, dimensions);
            let (_full_texture, mut full_target) =
                graphite_test_target_sized(&mut owner, &device, dimensions);
            let (doc, page, _) = fixture();
            let inputs = RenderInputs::for_doc(&doc);
            let mut renderer = super::super::RasterRenderer::new(WIDTH, HEIGHT).expect("camera");
            let mut full = super::super::RasterRenderer::new(WIDTH, HEIGHT).expect("full");
            renderer.set_pixel_snap_pan(true);
            full.set_pixel_snap_pan(true);
            for center in [
                [512.0, 240.0],
                [613.25, 285.75],
                [300.5, 280.0],
                [512.0, 240.0],
            ] {
                let viewport = fanta_doc::Viewport { center, zoom: 1.0 };
                let metrics = full.render_to_canvas(
                    full_target.canvas().expect("full canvas"),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &viewport,
                    Some(page),
                    &inputs,
                );
                assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
                full_target
                    .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                    .expect("full completed");
                let expected = graphite_target_pixels(&mut full_target);
                let mut completed = None;
                for _ in 0..64 {
                    let result = renderer.render_camera_to_canvas(
                        target.canvas().expect("camera canvas"),
                        WIDTH,
                        HEIGHT,
                        &doc.scene,
                        &viewport,
                        Some(page),
                        &inputs,
                        1,
                        false,
                    );
                    assert!(!result.metrics.incomplete_artwork && !result.metrics.effect_failed);
                    assert!(result.complete_detail && !result.cancelled);
                    assert!(!result.cache.overview_drawn);
                    assert!(
                        result.cache.tiles_built as usize
                            <= super::super::camera_cache::MANDATORY_TILES
                                + super::super::camera_cache::PREFETCH_TILES
                    );
                    target
                        .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                        .expect("camera completed");
                    let actual = graphite_target_pixels(&mut target);
                    assert!(
                        actual
                            .chunks_exact(4)
                            .all(|pixel| pixel.last() == Some(&255))
                    );
                    let maximum = actual
                        .iter()
                        .zip(&expected)
                        .map(|(actual, expected)| actual.abs_diff(*expected))
                        .max()
                        .expect("native sharp pixels");
                    assert!(
                        maximum <= 2,
                        "first and every subsequent frame must preserve sharp rounded clips: {maximum}"
                    );
                    if !result.needs_refresh {
                        completed = Some(result);
                        break;
                    }
                    assert!(result.needs_refresh);
                    let maintenance = renderer.maintain_camera_cache(
                        target.canvas().expect("maintenance canvas"),
                        WIDTH,
                        HEIGHT,
                        &doc.scene,
                        &viewport,
                        Some(page),
                        &inputs,
                        1,
                        &|| false,
                    );
                    assert!(!maintenance.cancelled);
                    assert!(maintenance.cache.scene_walks <= 1);
                    assert!(maintenance.cache.tiles_built <= 1);
                    target
                        .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                        .expect("maintenance completed");
                    assert_eq!(graphite_target_pixels(&mut target), actual);
                }
                let completed = completed.expect("finite visible tiles settle");
                assert!(!completed.needs_refresh);
                assert!(!completed.cache.overview_drawn && completed.cache.tiles_drawn > 0);
                let actual = graphite_target_pixels(&mut target);
                let maximum = actual
                    .iter()
                    .zip(&expected)
                    .map(|(actual, expected)| actual.abs_diff(*expected))
                    .max()
                    .expect("native pixels");
                let differences: Vec<_> = actual
                    .chunks_exact(4)
                    .zip(expected.chunks_exact(4))
                    .enumerate()
                    .filter(|(_, (actual, expected))| {
                        actual
                            .iter()
                            .zip(*expected)
                            .any(|(actual, expected)| actual.abs_diff(*expected) > 2)
                    })
                    .take(8)
                    .map(|(position, (actual, expected))| {
                        (
                            position % WIDTH as usize,
                            position / WIDTH as usize,
                            actual,
                            expected,
                        )
                    })
                    .collect();
                assert!(
                    maximum <= 2,
                    "Graphite rounded-clip tile difference {maximum} at {viewport:?}: {differences:?}"
                );
                let warm = renderer.render_camera_to_canvas(
                    target.canvas().expect("warm canvas"),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &viewport,
                    Some(page),
                    &inputs,
                    1,
                    false,
                );
                assert_eq!(warm.metrics.nodes_visited, 0);
                assert_eq!(warm.cache.scene_walks, 0);
                assert_eq!(warm.cache.tiles_built, 0);
                target
                    .flush_and_submit(skia_safe::gpu::SyncCpu::Yes)
                    .expect("warm completed");
            }
        });
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_owned_context_refuses_poisoned_work_and_cpu_surface_adoption() {
        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let (_texture, mut target) = graphite_test_target(&mut owner, &device);
            assert!(matches!(
                RetainedBackend::Cpu.validate_surface(&mut target.surface),
                Err(RetainedError::ChangedBackend)
            ));
            let mut raster = skia_safe::surfaces::raster_n32_premul((32, 32)).expect("raster");
            assert!(matches!(
                owner.owns_surface(&mut raster),
                Err(RetainedError::UnsupportedBackend)
            ));
            let RetainedGpuContext::Graphite(context) = &target.backend.context else {
                panic!("Graphite target");
            };
            context.state.try_borrow_mut().expect("idle owner").failed = true;
            assert!(matches!(
                target.canvas(),
                Err(RetainedError::BackendSubmission)
            ));
            assert!(matches!(
                target.image_snapshot(),
                Err(RetainedError::BackendSubmission)
            ));
            assert!(matches!(
                owner.submit_and_wait(),
                Err(RetainedError::BackendSubmission)
            ));
        });
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_default_provider_draws_raster_images_shaders_and_mipmaps_per_recorder() {
        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let (_texture, mut target) = graphite_test_target(&mut owner, &device);
            let mut pixels = Vec::with_capacity(64 * 64 * 4);
            for y in 0..64 {
                for x in 0..64 {
                    pixels.extend_from_slice(if (x / 4 + y / 4) % 2 == 0 {
                        &[240, 30, 70, 255]
                    } else {
                        &[20, 170, 230, 255]
                    });
                }
            }
            let info = skia_safe::ImageInfo::new(
                (64, 64),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            let image = skia_safe::images::raster_from_data(
                &info,
                skia_safe::Data::new_copy(&pixels),
                64 * 4,
            )
            .expect("raster input image");
            let image = image.with_default_mipmaps().expect("raster mip pyramid");
            assert!(!image.is_texture_backed());
            assert!(image.has_mipmaps());
            let draw = |canvas: &skia_safe::Canvas, side: f32, mipmapped: skia_safe::MipmapMode| {
                let sampling =
                    skia_safe::SamplingOptions::new(skia_safe::FilterMode::Linear, mipmapped);
                canvas.clear(skia_safe::Color::TRANSPARENT);
                canvas.draw_image_rect_with_sampling_options(
                    &image,
                    None,
                    skia_safe::Rect::from_xywh(0.0, 0.0, side, side),
                    sampling,
                    &skia_safe::Paint::default(),
                );
                let matrix = skia_safe::Matrix::new_all(
                    side / 64.0,
                    0.0,
                    16.0,
                    0.0,
                    side / 64.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                );
                let shader = image
                    .to_shader(
                        (skia_safe::TileMode::Clamp, skia_safe::TileMode::Clamp),
                        sampling,
                        &matrix,
                    )
                    .expect("raster-backed image shader");
                let mut paint = skia_safe::Paint::default();
                paint.set_shader(shader);
                canvas.draw_rect(skia_safe::Rect::from_xywh(16.0, 0.0, side, side), &paint);
            };
            let mut last_pixels = Vec::new();
            for (side, mipmapped) in [
                (16.0, skia_safe::MipmapMode::None),
                (16.0, skia_safe::MipmapMode::Linear),
                (8.0, skia_safe::MipmapMode::Nearest),
                (16.0, skia_safe::MipmapMode::Linear),
            ] {
                let mut reference =
                    skia_safe::surfaces::raster_n32_premul((32, 32)).expect("reference");
                draw(reference.canvas(), side, mipmapped);
                draw(target.canvas().expect("Graphite canvas"), side, mipmapped);
                owner
                    .submit_and_wait()
                    .expect("completed raster uploads and shader paint");
                let actual = graphite_target_pixels(&mut target);
                let read_info = skia_safe::ImageInfo::new(
                    (32, 32),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Premul,
                    None,
                );
                let mut expected = vec![0; actual.len()];
                assert!(reference.read_pixels(&read_info, &mut expected, 32 * 4, (0, 0)));
                assert!(
                    actual
                        .iter()
                        .zip(&expected)
                        .all(|(actual, expected)| actual.abs_diff(*expected) <= 2),
                    "bitmap and shader minification must match raster at side {side}, mip {mipmapped:?}"
                );
                for x in [2, 18] {
                    assert_eq!(
                        actual[(2 * 32 + x) * 4 + 3],
                        255,
                        "both the bitmap and raster shader draw real pixels"
                    );
                }
                last_pixels = actual;
            }
            let mut second_owner = graphite_test_context(&device, &queue);
            let (_second_texture, mut second_target) =
                graphite_test_target(&mut second_owner, &device);
            draw(
                second_target.canvas().expect("second recorder canvas"),
                16.0,
                skia_safe::MipmapMode::Linear,
            );
            second_owner
                .submit_and_wait()
                .expect("independent recorder upload");
            assert_eq!(
                graphite_target_pixels(&mut second_target),
                last_pixels,
                "a second recorder must upload the same raster identity in its own GPU domain"
            );

            let asset = AssetId::new();
            let mut resolver = crate::asset::InMemoryAssetResolver::new();
            resolver.insert(asset, DecodedImage::new(&pixels, 64, 64));
            let resolver = Arc::new(resolver);
            let mut scene = Scene::new();
            let page = scene
                .insert(CanvasNode::new(NodeData::Group(
                    fanta_doc::GroupNode::default(),
                )))
                .expect("page");
            let mut bitmap = CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
                asset,
                natural_size: [64, 64],
                local_size: [16.0, 16.0],
                crop: None,
                fit: fanta_doc::ImageFitMode::Stretch,
                tint: None,
            }));
            bitmap.parent = Some(page);
            bitmap.transform = fanta_doc::Transform2D::translation(-16.0, -16.0);
            scene.insert(bitmap).expect("bitmap");
            let mut vector =
                fanta_doc::VectorNode::rect_solid(0.0, 0.0, 16.0, 16.0, fanta_doc::Color::WHITE);
            vector.fills = smallvec::smallvec![Fill::Image {
                asset,
                mode: fanta_doc::ImageFitMode::Stretch,
                opacity: 1.0,
                crop: None,
                scale: None,
                rotation: None,
                blend: BlendMode::Normal,
                adjust: Default::default(),
            }];
            let mut painted = CanvasNode::new(NodeData::Vector(vector));
            painted.parent = Some(page);
            painted.transform = fanta_doc::Transform2D::translation(0.0, -16.0);
            scene.insert(painted).expect("image fill");
            let inputs = RenderInputs::empty();
            let mut gpu_renderer =
                super::super::RasterRenderer::new(32, 32).expect("GPU renderer state");
            gpu_renderer.background = fanta_doc::Color::BLACK;
            gpu_renderer.set_asset_resolver(resolver.clone());
            let mut cpu_renderer =
                super::super::RasterRenderer::new(32, 32).expect("CPU reference");
            cpu_renderer.background = fanta_doc::Color::BLACK;
            cpu_renderer.set_asset_resolver(resolver);
            for zoom in [1.0, 0.5, 2.0, 1.0] {
                let viewport = fanta_doc::Viewport {
                    center: [0.0, 0.0],
                    zoom,
                };
                gpu_renderer.render_to_canvas(
                    target.canvas().expect("target"),
                    32,
                    32,
                    &scene,
                    &viewport,
                    Some(page),
                    &inputs,
                );
                owner.submit_and_wait().expect("Fanta raster cache uploads");
                cpu_renderer.render_page_with(&scene, &viewport, Some(page), &inputs);
                let actual = graphite_target_pixels(&mut target);
                let expected = cpu_renderer.copy_rgba();
                assert!(
                    actual
                        .iter()
                        .zip(&expected)
                        .all(|(actual, expected)| actual.abs_diff(*expected) <= 2),
                    "Fanta bitmap/image-fill and warm pyramid cache must match at zoom {zoom}"
                );
            }
        });
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_background_blur_visibly_frosts_the_backdrop_and_preserves_its_boundary() {
        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let (_texture, mut target) = graphite_test_target_sized(&mut owner, &device, (64, 64));
            for zoom in [0.37, 1.0, 2.0] {
                for masked in [false, true] {
                    let render = |blurred: bool,
                                  owner: &mut RetainedGraphiteContext,
                                  target: &mut RetainedGpuTarget| {
                        let mut scene = Scene::new();
                        let page = scene
                            .insert(CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
                                background: Some(Fill::solid(fanta_doc::Color::WHITE)),
                                ..Default::default()
                            })))
                            .expect("page");
                        let mut group =
                            CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
                        group.parent = Some(page);
                        let group = scene.insert(group).expect("group");
                        if masked {
                            let mut mask = CanvasNode::new(NodeData::Vector(
                                fanta_doc::VectorNode::rect_solid(
                                    -24.0 / zoom,
                                    -24.0 / zoom,
                                    48.0 / zoom,
                                    48.0 / zoom,
                                    fanta_doc::Color::WHITE,
                                ),
                            ));
                            mask.parent = Some(group);
                            mask.is_mask = true;
                            scene.insert(mask).expect("mask");
                        }
                        for row in 0..12 {
                            for column in 0..12 {
                                if (row + column) % 2 == 0 {
                                    continue;
                                }
                                let mut cell = CanvasNode::new(NodeData::Vector(
                                    fanta_doc::VectorNode::rect_solid(
                                        (-24.0 + f64::from(column) * 4.0) / zoom,
                                        (-24.0 + f64::from(row) * 4.0) / zoom,
                                        4.0 / zoom,
                                        4.0 / zoom,
                                        fanta_doc::Color::BLACK,
                                    ),
                                ));
                                cell.parent = Some(group);
                                cell.index = scene.next_child_index(Some(group));
                                scene.insert(cell).expect("checker cell");
                            }
                        }
                        let mut vector = fanta_doc::VectorNode::rect_solid(
                            -12.0 / zoom,
                            -12.0 / zoom,
                            24.0 / zoom,
                            24.0 / zoom,
                            fanta_doc::Color::rgba(255, 255, 255, 30),
                        );
                        vector.corner_radius = Some(4.0 / zoom);
                        let mut panel = CanvasNode::new(NodeData::Vector(vector));
                        panel.parent = Some(group);
                        panel.index = scene.next_child_index(Some(group));
                        if blurred {
                            panel.blurs.push(fanta_doc::Blur::background(10.0 / zoom));
                        }
                        scene.insert(panel).expect("panel");
                        let mut renderer =
                            super::super::RasterRenderer::new(64, 64).expect("renderer");
                        let metrics = renderer.render_to_canvas(
                            target.canvas().expect("target"),
                            64,
                            64,
                            &scene,
                            &fanta_doc::Viewport {
                                center: [0.0, 0.0],
                                zoom,
                            },
                            Some(page),
                            &RenderInputs::empty(),
                        );
                        assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
                        owner.submit_and_wait().expect("complete blur render");
                        graphite_target_pixels(target)
                    };
                    let plain = render(false, &mut owner, &mut target);
                    let frosted = render(true, &mut owner, &mut target);
                    let variance = |pixels: &[u8]| {
                        let values: Vec<f64> = (28..36)
                            .flat_map(|row| (28..36).map(move |column| (row, column)))
                            .map(|(row, column)| {
                                f64::from(*pixels.get((row * 64 + column) * 4).expect("pixel"))
                            })
                            .collect();
                        let mean = values.iter().sum::<f64>() / values.len() as f64;
                        values
                            .iter()
                            .map(|value| (value - mean).powi(2))
                            .sum::<f64>()
                            / values.len() as f64
                    };
                    let plain_variance = variance(&plain);
                    let frosted_variance = variance(&frosted);
                    assert!(
                        plain_variance > 1_000.0,
                        "checker remains sharp without blur"
                    );
                    assert!(
                        frosted_variance < plain_variance * 0.6,
                        "zoom {zoom}, masked {masked}: backdrop must visibly frost: {plain_variance} → {frosted_variance}"
                    );
                    for row in 0..64 {
                        for column in 0..64 {
                            if (19..45).contains(&row) && (19..45).contains(&column) {
                                continue;
                            }
                            let offset = (row * 64 + column) * 4;
                            assert_eq!(
                                plain.get(offset..offset + 4),
                                frosted.get(offset..offset + 4),
                                "background blur must leave pixels outside its silhouette sharp"
                            );
                        }
                    }
                }
            }
        });
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    fn graphite_layer_blur_softens_only_the_authored_artwork() {
        metal::objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default().expect("Metal device");
            let queue = device.new_command_queue();
            let mut owner = graphite_test_context(&device, &queue);
            let (_texture, mut target) = graphite_test_target_sized(&mut owner, &device, (64, 64));
            for zoom in [0.37, 1.0, 2.0] {
                let render = |blurred: bool,
                              owner: &mut RetainedGraphiteContext,
                              target: &mut RetainedGpuTarget| {
                    let mut scene = Scene::new();
                    let page = scene
                        .insert(CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
                            background: Some(Fill::solid(fanta_doc::Color::WHITE)),
                            ..Default::default()
                        })))
                        .expect("page");
                    let mut rectangle =
                        CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
                            -12.0 / zoom,
                            -12.0 / zoom,
                            24.0 / zoom,
                            24.0 / zoom,
                            fanta_doc::Color::rgb(255, 0, 0),
                        )));
                    rectangle.parent = Some(page);
                    if blurred {
                        rectangle.blurs.push(fanta_doc::Blur::layer(10.0 / zoom));
                    }
                    scene.insert(rectangle).expect("rectangle");
                    let mut renderer = super::super::RasterRenderer::new(64, 64).expect("renderer");
                    let metrics = renderer.render_to_canvas(
                        target.canvas().expect("target"),
                        64,
                        64,
                        &scene,
                        &fanta_doc::Viewport {
                            center: [0.0, 0.0],
                            zoom,
                        },
                        Some(page),
                        &RenderInputs::empty(),
                    );
                    assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
                    owner.submit_and_wait().expect("complete authored blur");
                    graphite_target_pixels(target)
                };
                let plain = render(false, &mut owner, &mut target);
                let softened = render(true, &mut owner, &mut target);
                let green = |pixels: &[u8], column: usize| {
                    *pixels
                        .get((32 * 64 + column) * 4 + 1)
                        .expect("green channel")
                };
                assert!(green(&plain, 46) >= 254, "unblurred edge remains sharp");
                assert!(green(&softened, 46) < 245, "authored blur extends its edge");
                assert!(green(&softened, 42) > green(&plain, 42) + 20);
                assert!(green(&softened, 0) >= 254, "distant canvas remains sharp");
            }
        });
    }

    #[test]
    fn retained_storage_allocation_failure_preserves_source_and_renderer_surface() {
        let (scene, page_id, moving_id) = storage_fixture();
        let viewport = fanta_doc::Viewport::default();
        let inputs = RenderInputs::empty();
        let original = scene.get(moving_id).expect("moving").clone();
        let revision = scene.revision();
        for failed_allocation in [1, 2] {
            let mut renderer = super::super::RasterRenderer::new(32, 32).expect("renderer");
            renderer.render_page_with(&scene, &viewport, Some(page_id), &inputs);
            let original_pixels = renderer.copy_rgba();
            let mut allocations = 0;
            let result = RetainedTranslationSession::prepare_internal(
                &mut renderer,
                &scene,
                page_id,
                moving_id,
                &viewport,
                &inputs,
                None,
                0,
                RETAINED_PIXEL_AND_PICTURE_BUDGET,
                RetainedBackend::Cpu,
                || {
                    allocations += 1;
                    if allocations == failed_allocation {
                        None
                    } else {
                        skia_safe::surfaces::raster_n32_premul((32, 32))
                    }
                },
                std::time::Instant::now(),
                0,
            );
            assert!(matches!(result, Err(RetainedError::Allocation)));
            assert_eq!(allocations, failed_allocation);
            assert_eq!(scene.revision(), revision);
            assert_eq!(scene.get(moving_id), Some(&original));
            assert_eq!(renderer.copy_rgba(), original_pixels);
        }
    }

    #[test]
    fn retained_backend_rejection_precedes_revision_and_output_mutation() {
        let (mut scene, page_id, moving_id) = storage_fixture();
        let viewport = fanta_doc::Viewport::default();
        let inputs = RenderInputs::empty();
        let mut renderer = super::super::RasterRenderer::new(32, 32).expect("renderer");
        let mut session = RetainedTranslationSession::prepare(
            &mut renderer,
            &scene,
            page_id,
            moving_id,
            &viewport,
            &inputs,
            None,
            0,
        )
        .expect("prepare");
        session
            .render(&mut renderer, &scene, &viewport, &inputs, None, 0)
            .expect("initial frame");
        let original_revision = session.spec.scene_revision;
        let info = skia_safe::ImageInfo::new(
            (32, 32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let mut original_pixels = vec![0; 32 * 32 * 4];
        assert!(
            session
                .output
                .read_pixels(&info, &mut original_pixels, 32 * 4, (0, 0))
        );
        scene
            .set_transform(moving_id, fanta_doc::Transform2D::translation(5.0, 7.0))
            .expect("translation");
        assert!(matches!(
            session.render_with_backend(
                &mut renderer,
                |_| Err(RetainedError::ChangedBackend),
                &scene,
                &viewport,
                &inputs,
                None,
                0,
            ),
            Err(RetainedError::ChangedBackend)
        ));
        assert_eq!(session.spec.scene_revision, original_revision);
        let mut rejected_pixels = vec![0; original_pixels.len()];
        assert!(
            session
                .output
                .read_pixels(&info, &mut rejected_pixels, 32 * 4, (0, 0))
        );
        assert_eq!(rejected_pixels, original_pixels);
        assert!(matches!(
            session.render(&mut renderer, &scene, &viewport, &inputs, None, 0),
            Err(RetainedError::Disabled)
        ));
    }

    #[test]
    fn retained_storage_budget_rejects_before_allocator_runs() {
        let (scene, page_id, moving_id) = storage_fixture();
        let viewport = fanta_doc::Viewport::default();
        let inputs = RenderInputs::empty();
        let mut renderer = super::super::RasterRenderer::new(32, 32).expect("renderer");
        let mut allocations = 0;
        let result = RetainedTranslationSession::prepare_internal(
            &mut renderer,
            &scene,
            page_id,
            moving_id,
            &viewport,
            &inputs,
            None,
            0,
            32 * 32 * 8 - 1,
            RetainedBackend::Cpu,
            || {
                allocations += 1;
                skia_safe::surfaces::raster_n32_premul((32, 32))
            },
            std::time::Instant::now(),
            0,
        );
        assert!(matches!(result, Err(RetainedError::MemoryLimit { .. })));
        assert_eq!(allocations, 0);
    }

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
