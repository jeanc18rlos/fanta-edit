use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use fanta_doc::{
    AnimationClipId, ComponentId, ModeId, NodeData, NodeId, Scene, VarValue, VariableCollectionId,
    VariableId,
};
use serde::{Deserialize, Serialize};

use crate::{ActivePrototypeAnimation, PlaybackSession, PresentError, PresentResponse};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipPlaybackMode {
    #[default]
    Once,
    Repeat,
    PingPong,
}

impl From<fanta_doc::LoopMode> for ClipPlaybackMode {
    fn from(mode: fanta_doc::LoopMode) -> Self {
        match mode {
            fanta_doc::LoopMode::Once => Self::Once,
            fanta_doc::LoopMode::Repeat => Self::Repeat,
            fanta_doc::LoopMode::PingPong => Self::PingPong,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeOccurrence {
    pub layer: u64,
    pub node: NodeId,
    #[serde(default)]
    pub instance_path: Vec<NodeId>,
    #[serde(default)]
    pub definition_path: Vec<NodeId>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipPlaybackState {
    pub clip: AnimationClipId,
    pub playhead_ms: Option<u32>,
    pub mode: ClipPlaybackMode,
    pub paused: bool,
    pub running: bool,
    pub scope: Option<NodeOccurrence>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSchedule {
    pub continuous: bool,
    pub next_deadline_ms: Option<f64>,
    pub paused: bool,
    pub timers_suspended: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub current_frame: NodeId,
    pub current_layer: u64,
    pub overlay_frames: Vec<NodeId>,
    pub clock_ms: f64,
    pub clip: Option<ClipPlaybackState>,
    pub schedule: PlaybackSchedule,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlaybackEvent {
    ClipCompleted { clip: AnimationClipId },
    Diagnostic { diagnostic: PlaybackDiagnostic },
}

#[derive(Default)]
pub(crate) struct OccurrenceCache {
    identities: BTreeMap<NodeOccurrence, NodeId>,
    next_identity: u128,
    explicit_styles: BTreeMap<NodeId, (bool, bool)>,
    pub(super) layers: BTreeMap<(u64, NodeId, bool), (RuntimeTreeKey, Rc<ResolvedLayer>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RuntimeTreeKey {
    mode_generation: u64,
    global_variants: BTreeMap<ComponentId, String>,
    instance_variants: BTreeMap<NodeId, (ComponentId, String)>,
    scroll: BTreeMap<NodeId, [f64; 2]>,
    clip: Option<(AnimationClipId, Option<u32>, Option<NodeOccurrence>)>,
}

impl RuntimeTreeKey {
    fn same_baseline(&self, other: &Self) -> bool {
        self.mode_generation == other.mode_generation
            && self.global_variants == other.global_variants
            && self.instance_variants == other.instance_variants
            && self.scroll == other.scroll
    }
}

pub(crate) struct ResolvedLayer {
    pub scene: Scene,
    pub root: NodeId,
    pub occurrences: BTreeMap<NodeId, NodeOccurrence>,
    pub master_roots: BTreeMap<NodeId, NodeId>,
    target_index: BTreeMap<(Vec<NodeId>, NodeId), NodeId>,
    sampled_nodes: std::collections::BTreeSet<NodeId>,
}

impl PlaybackSession {
    pub fn new_headless(
        doc: &fanta_doc::Doc,
        start: Option<NodeId>,
        size: glam::DVec2,
    ) -> Result<Self, PresentError> {
        Self::new(doc, start, size)
    }

    pub fn advance_by(&mut self, delta_seconds: f64) -> Result<PresentResponse, PresentError> {
        if !delta_seconds.is_finite()
            || delta_seconds < 0.0
            || !((self.clock + delta_seconds) * 1000.0).is_finite()
        {
            return Err(PresentError::InvalidElapsedTime);
        }
        Ok(self.tick(delta_seconds))
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }
    pub fn resume(&mut self) {
        self.paused = false;
    }
    pub fn pause_playback(&mut self) {
        self.pause();
    }
    pub fn resume_playback(&mut self) {
        self.resume();
    }

    pub fn schedule(&self) -> PlaybackSchedule {
        let continuous = !self.paused
            && (self.active_transition.is_some()
                || self.active_overlay_transition.is_some()
                || self.active_overlay_exit.is_some()
                || self.active_variant_transition.is_some()
                || self.active_scroll_animation.is_some()
                || self.active_prototype_animation.is_some_and(|active| {
                    active.playhead_ms(&self.doc).is_some() && active.is_running(&self.doc)
                }));
        let timer_deadline_ms = if self.paused || self.timers_suspended {
            None
        } else {
            self.timers
                .iter()
                .map(|timer| timer.remaining.max(0.0) * 1000.0)
                .min_by(f64::total_cmp)
        };
        let clip_deadline_ms = (!self.paused)
            .then(|| self.active_prototype_animation)
            .flatten()
            .filter(|active| !active.paused && active.elapsed < active.delay)
            .map(|active| (active.delay - active.elapsed) * 1000.0);
        let next_deadline_ms = timer_deadline_ms
            .into_iter()
            .chain(clip_deadline_ms)
            .min_by(f64::total_cmp);
        PlaybackSchedule {
            continuous,
            next_deadline_ms,
            paused: self.paused,
            timers_suspended: self.timers_suspended,
        }
    }

    pub fn playback_state(&self) -> PlaybackState {
        PlaybackState {
            current_frame: self.current,
            current_layer: self.current_layer,
            overlay_frames: self.overlays.iter().map(|entry| entry.root).collect(),
            clock_ms: self.clock * 1000.0,
            clip: self
                .active_prototype_animation
                .map(|active| ClipPlaybackState {
                    clip: active.clip,
                    playhead_ms: active.playhead_ms(&self.doc),
                    mode: active.mode,
                    paused: active.paused,
                    running: active.is_running(&self.doc),
                    scope: self.active_motion_scope.clone(),
                }),
            schedule: self.schedule(),
        }
    }

    pub fn take_playback_events(&mut self) -> Vec<PlaybackEvent> {
        let mut events = std::mem::take(&mut self.playback_events);
        events.extend(self.resolution_errors.take().into_iter().map(|message| {
            PlaybackEvent::Diagnostic {
                diagnostic: PlaybackDiagnostic {
                    code: "resolutionFailed".into(),
                    message,
                },
            }
        }));
        events
    }

    pub fn set_text_context(&mut self, context: fanta_render::TextRenderContext) {
        self.text_context = Some(context);
        self.text_context_generation = self.text_context_generation.wrapping_add(1);
        self.occurrence_cache.borrow_mut().layers.clear();
    }

    pub fn set_variable_value(&mut self, variable: VariableId, value: VarValue) -> bool {
        self.set_variable(self.layer_root(), variable, value)
    }

    pub fn runtime_variables(&self) -> &fanta_doc::VariableRegistry {
        &self.variables
    }

    pub fn runtime_active_modes(&self) -> &BTreeMap<VariableCollectionId, ModeId> {
        &self.active_modes
    }

    pub fn resolution_generation(&self) -> u64 {
        self.mode_generation
    }

    pub fn set_runtime_variable(
        &mut self,
        variable: VariableId,
        value: VarValue,
    ) -> Result<PresentResponse, PresentError> {
        if !self.set_variable_value(variable, value) {
            return Err(PresentError::InvalidVariable);
        }
        Ok(PresentResponse {
            needs_redraw: true,
            animating: self.is_animating(),
            ..Default::default()
        })
    }

    pub fn set_runtime_mode(
        &mut self,
        collection: VariableCollectionId,
        mode: ModeId,
    ) -> Result<PresentResponse, PresentError> {
        if !self.set_active_mode(collection, mode) {
            return Err(PresentError::InvalidMode);
        }
        Ok(PresentResponse {
            needs_redraw: true,
            animating: self.is_animating(),
            ..Default::default()
        })
    }

    pub fn set_active_mode(&mut self, collection: VariableCollectionId, mode: ModeId) -> bool {
        if !self
            .variables
            .collections
            .get(&collection)
            .is_some_and(|collection| {
                collection
                    .modes
                    .iter()
                    .any(|candidate| candidate.id == mode)
            })
        {
            return false;
        }
        self.active_modes.insert(collection, mode);
        self.mode_generation = self.mode_generation.wrapping_add(1);
        self.refresh_layer_timers();
        true
    }

    pub fn navigate(&mut self, frame: NodeId) -> PresentResponse {
        self.execute(
            self.layer_root(),
            &fanta_doc::Action::Navigate { to: frame },
            None,
        )
    }

    pub fn back(&mut self) -> PresentResponse {
        self.execute(self.layer_root(), &fanta_doc::Action::Back, None)
    }

    pub fn play_clip(
        &mut self,
        clip: AnimationClipId,
        scope: Option<NodeOccurrence>,
        mode: ClipPlaybackMode,
    ) -> Result<PresentResponse, PresentError> {
        let targets = self.resolve_motion_targets(clip, scope.as_ref())?;
        self.active_prototype_animation = Some(ActivePrototypeAnimation {
            clip,
            delay: 0.0,
            elapsed: 0.0,
            mode,
            paused: false,
            completion_emitted: false,
        });
        self.active_motion_scope = scope;
        self.active_motion_targets = targets;
        if self
            .doc
            .motion
            .clip(clip)
            .is_some_and(|clip| clip.duration_ms == 0)
        {
            self.playback_events
                .push(PlaybackEvent::ClipCompleted { clip });
            if let Some(active) = &mut self.active_prototype_animation {
                active.completion_emitted = true;
            }
        }
        Ok(PresentResponse {
            needs_redraw: true,
            animating: self.is_animating(),
            ..Default::default()
        })
    }

    pub fn pause_clip(&mut self) {
        if let Some(active) = &mut self.active_prototype_animation {
            active.paused = true;
        }
    }
    pub fn resume_clip(&mut self) {
        if let Some(active) = &mut self.active_prototype_animation {
            active.paused = false;
        }
    }

    pub fn seek_clip(&mut self, playhead_ms: f64) -> Result<PresentResponse, PresentError> {
        if !playhead_ms.is_finite() || playhead_ms < 0.0 {
            return Err(PresentError::InvalidPlayhead);
        }
        if let Some(active) = &mut self.active_prototype_animation {
            let duration = self
                .doc
                .motion
                .clip(active.clip)
                .ok_or(PresentError::ClipNotFound)?
                .duration_ms;
            active.elapsed = active.delay + playhead_ms.min(f64::from(duration)) / 1000.0;
            active.completion_emitted = false;
        }
        Ok(PresentResponse {
            needs_redraw: true,
            animating: self.is_animating(),
            ..Default::default()
        })
    }

    pub fn stop_clip(&mut self) -> PresentResponse {
        let needs_redraw = self.active_prototype_animation.take().is_some();
        self.active_motion_scope = None;
        self.active_motion_targets.clear();
        PresentResponse {
            needs_redraw,
            animating: self.is_animating(),
            ..Default::default()
        }
    }

    pub fn occurrences(&self) -> Vec<NodeOccurrence> {
        let resolved = self.reaction_layer(false);
        resolved
            .scene
            .descendants_of(resolved.root)
            .filter_map(|identity| resolved.occurrences.get(&identity).cloned())
            .collect()
    }

    pub fn visit_effective_nodes(&self, mut visit: impl FnMut(&fanta_doc::CanvasNode)) {
        self.visit_effective_occurrences(|_, node| visit(node));
    }

    pub fn visit_effective_occurrences(
        &self,
        mut visit: impl FnMut(&NodeOccurrence, &fanta_doc::CanvasNode),
    ) {
        for (root, layer) in std::iter::once((self.current, self.current_layer)).chain(
            self.overlays
                .iter()
                .map(|overlay| (overlay.root, overlay.layer)),
        ) {
            let resolved = self.resolved_layer_with_identity(root, layer, true);
            for identity in resolved.scene.descendants_of(root) {
                if let (Some(occurrence), Some(node)) = (
                    resolved.occurrences.get(&identity),
                    resolved.scene.get(identity),
                ) {
                    visit(occurrence, node);
                }
            }
        }
    }

    pub(super) fn resolve_motion_targets(
        &self,
        clip: AnimationClipId,
        scope: Option<&NodeOccurrence>,
    ) -> Result<Vec<NodeOccurrence>, PresentError> {
        let clip = self
            .doc
            .motion
            .clip(clip)
            .ok_or(PresentError::ClipNotFound)?;
        let resolved = match scope {
            Some(scope) if scope.layer == self.current_layer => {
                self.resolved_layer_with_identity(self.current, self.current_layer, false)
            }
            Some(scope) => {
                let overlay = self
                    .overlays
                    .iter()
                    .find(|overlay| overlay.layer == scope.layer)
                    .ok_or(PresentError::MotionTargetNotFound(scope.node))?;
                self.resolved_layer_with_identity(overlay.root, overlay.layer, false)
            }
            None => self.reaction_layer(false),
        };
        if let Some(scope) = scope {
            if !resolved
                .occurrences
                .values()
                .any(|occurrence| occurrence == scope)
            {
                return Err(PresentError::MotionTargetNotFound(scope.node));
            }
        }
        let mut targets = Vec::with_capacity(clip.tracks.len());
        for track in clip.tracks.values() {
            let candidates = self.motion_candidates(&resolved, track.target.node, scope);
            if candidates.len() != 1 {
                return Err(PresentError::MotionTargetNotFound(track.target.node));
            }
            let occurrence = candidates
                .first()
                .and_then(|identity| resolved.occurrences.get(identity))
                .ok_or(PresentError::MotionTargetNotFound(track.target.node))?;
            targets.push(occurrence.clone());
        }
        Ok(targets)
    }

    pub(crate) fn motion_candidates(
        &self,
        resolved: &ResolvedLayer,
        target: NodeId,
        scope: Option<&NodeOccurrence>,
    ) -> Vec<NodeId> {
        let mut path = scope
            .map(|scope| scope.instance_path.clone())
            .unwrap_or_default();
        if let Some(scope) = scope {
            if resolved
                .master_roots
                .iter()
                .any(|(instance, _)| resolved.occurrences.get(instance) == Some(scope))
            {
                path.push(scope.node);
            }
        }
        if scope.is_some_and(|scope| scope.layer != self.layer_identity(resolved.root))
            && !resolved
                .occurrences
                .values()
                .any(|occurrence| Some(occurrence) == scope)
        {
            return Vec::new();
        }
        resolved
            .target_index
            .get(&(path, target))
            .copied()
            .into_iter()
            .collect()
    }

    pub(super) fn resolve_action_target(&self, source: NodeId, target: NodeId) -> Option<NodeId> {
        let resolved = self.reaction_layer(false);
        let scope = resolved.occurrences.get(&source).filter(|occurrence| {
            !occurrence.instance_path.is_empty() || resolved.master_roots.contains_key(&source)
        });
        if scope.is_none() {
            return self.doc.scene.contains(target).then_some(target);
        }
        self.motion_candidates(&resolved, target, scope)
            .into_iter()
            .next()
    }

    pub(super) fn reaction_instance_ancestors(
        &self,
        source: NodeId,
    ) -> Vec<(NodeId, NodeOccurrence)> {
        let resolved = self.reaction_layer(false);
        let mut ancestors = Vec::new();
        let mut cursor = Some(source);
        while let Some(identity) = cursor {
            if resolved.master_roots.contains_key(&identity) {
                if let Some(occurrence) = resolved.occurrences.get(&identity) {
                    ancestors.push((identity, occurrence.clone()));
                }
            }
            cursor = resolved.scene.get(identity).and_then(|node| node.parent);
        }
        ancestors
    }

    pub(super) fn surviving_reaction_source(
        &self,
        source: NodeId,
        original: &NodeOccurrence,
        ancestors: &[(NodeId, NodeOccurrence)],
    ) -> Option<(NodeId, (u64, NodeId))> {
        let root = if original.layer == self.current_layer {
            self.current
        } else {
            self.overlays
                .iter()
                .find(|overlay| overlay.layer == original.layer)?
                .root
        };
        let resolved = self.resolved_layer_with_identity(root, original.layer, false);
        let identity = if resolved.occurrences.get(&source) == Some(original) {
            Some(source)
        } else {
            ancestors.iter().find_map(|(identity, occurrence)| {
                (resolved.occurrences.get(identity) == Some(occurrence)).then_some(*identity)
            })
        }?;
        Some((identity, (original.layer, root)))
    }

    pub(crate) fn layer_identity(&self, root: NodeId) -> u64 {
        self.overlays
            .iter()
            .rev()
            .find(|entry| entry.root == root)
            .map_or(self.current_layer, |entry| entry.layer)
    }

    pub(super) fn reaction_layer(&self, include_motion: bool) -> Rc<ResolvedLayer> {
        match self.dispatch_layer {
            Some((layer, root)) => self.resolved_layer_with_identity(root, layer, include_motion),
            None => self.resolved_layer(self.layer_root(), include_motion),
        }
    }

    pub(super) fn refresh_layer_timers(&mut self) {
        self.schedule_timers_with_identity(self.current, self.current_layer);
        let layers: Vec<_> = self
            .overlays
            .iter()
            .map(|overlay| (overlay.root, overlay.layer))
            .collect();
        for (root, layer) in layers {
            self.schedule_timers_with_identity(root, layer);
        }
        if let Some(active) = self.active_prototype_animation {
            if self
                .resolve_motion_targets(active.clip, self.active_motion_scope.as_ref())
                .map_or(true, |targets| targets != self.active_motion_targets)
            {
                self.stop_clip();
            }
        }
    }

    pub(crate) fn resolved_layer(&self, root: NodeId, include_motion: bool) -> Rc<ResolvedLayer> {
        self.resolved_layer_with_identity(root, self.layer_identity(root), include_motion)
    }

    pub(crate) fn resolved_layer_with_identity(
        &self,
        root: NodeId,
        layer: u64,
        include_motion: bool,
    ) -> Rc<ResolvedLayer> {
        if let Some(context) = &self.text_context {
            return context.with(|| self.resolve_layer_inner(root, layer, include_motion));
        }
        self.resolve_layer_inner(root, layer, include_motion)
    }

    fn resolve_layer_inner(
        &self,
        root: NodeId,
        layer: u64,
        include_motion: bool,
    ) -> Rc<ResolvedLayer> {
        let clip = include_motion
            .then(|| self.active_prototype_animation)
            .flatten()
            .map(|active| {
                (
                    active.clip,
                    active.playhead_ms(&self.doc),
                    self.active_motion_scope.clone(),
                )
            });
        let key = RuntimeTreeKey {
            mode_generation: self.mode_generation,
            global_variants: self
                .variant_overrides
                .iter()
                .map(|(identity, value)| (*identity, value.clone()))
                .collect(),
            instance_variants: self
                .instance_variant_overrides
                .iter()
                .map(|(identity, value)| (*identity, value.clone()))
                .collect(),
            scroll: self
                .scroll_offsets
                .iter()
                .map(|(identity, value)| (*identity, *value))
                .collect(),
            clip,
        };
        if let Some((previous, cached)) =
            self.occurrence_cache
                .borrow()
                .layers
                .get(&(layer, root, include_motion))
        {
            if previous == &key {
                return cached.clone();
            }
        }
        let mut resolved = if include_motion {
            let baseline = self.resolved_layer_with_identity(root, layer, false);
            let previous = self
                .occurrence_cache
                .borrow_mut()
                .layers
                .remove(&(layer, root, true));
            match previous
                .filter(|(previous, _)| previous.same_baseline(&key))
                .and_then(|(_, cached)| Rc::try_unwrap(cached).ok())
            {
                Some(mut scratch) => {
                    for identity in &scratch.sampled_nodes {
                        if let (Some(original), Some(target)) = (
                            baseline.scene.get(*identity),
                            scratch.scene.get_mut(*identity),
                        ) {
                            *target = original.clone();
                        }
                    }
                    scratch.sampled_nodes.clear();
                    scratch
                }
                None => ResolvedLayer {
                    scene: baseline.scene.clone(),
                    root,
                    occurrences: baseline.occurrences.clone(),
                    master_roots: baseline.master_roots.clone(),
                    target_index: baseline.target_index.clone(),
                    sampled_nodes: Default::default(),
                },
            }
        } else {
            let mut scene = self.doc.scene.clone();
            Self::apply_variant_overrides_to_scene(
                &self.variant_overrides,
                &self.instance_variant_overrides,
                &self.doc.components,
                &mut scene,
                root,
            );
            let mut occurrences = BTreeMap::new();
            let mut master_roots = BTreeMap::new();
            let mut baked_geometry = std::collections::BTreeSet::new();
            let mut layout_changes = std::collections::BTreeSet::new();
            let original: Vec<_> = scene.descendants_of(root).collect();
            let mut remap = BTreeMap::new();
            for identity in &original {
                let occurrence = NodeOccurrence {
                    layer,
                    node: *identity,
                    instance_path: Vec::new(),
                    definition_path: Vec::new(),
                };
                let stable = if layer == self.current_layer || *identity == root {
                    *identity
                } else {
                    self.stable_occurrence_identity(&occurrence)
                };
                remap.insert(*identity, stable);
                occurrences.insert(stable, occurrence);
            }
            if layer != self.current_layer {
                let mut nodes: Vec<_> = original
                    .iter()
                    .filter_map(|identity| scene.get(*identity).cloned())
                    .collect();
                rank_occurrence_siblings(nodes.iter_mut());
                for node in &mut nodes {
                    node.id = remap.get(&node.id).copied().unwrap_or(node.id);
                    node.parent = node
                        .parent
                        .map(|parent| remap.get(&parent).copied().unwrap_or(parent));
                }
                if let Err(error) = scene
                    .remove(root)
                    .and_then(|_| scene.insert_many(nodes).map(|_| ()))
                {
                    self.resolution_errors.borrow_mut().push(error.to_string());
                }
            }
            for identity in original {
                if let Some(stable) = remap.get(&identity).copied() {
                    self.occurrence_cache
                        .borrow_mut()
                        .explicit_styles
                        .remove(&stable);
                    self.resolve_occurrence(
                        &mut scene,
                        stable,
                        &mut occurrences,
                        &mut master_roots,
                        &mut baked_geometry,
                        &mut layout_changes,
                        0,
                    );
                }
            }
            let mut affected: std::collections::BTreeSet<_> = scene
                .descendants_of(root)
                .filter(|identity| !baked_geometry.contains(identity))
                .collect();
            for changed in layout_changes {
                affected.insert(changed);
                affected.extend(scene.ancestors_of(changed).map(|node| node.id));
            }
            fanta_doc::solve_auto_layout_scoped(
                &mut scene,
                root,
                &affected,
                &mut fanta_render::measure_text_node,
            );
            Self::apply_scroll_offsets_to_scene(&self.scroll_offsets, &mut scene);
            let mut target_index = BTreeMap::new();
            for (identity, occurrence) in &occurrences {
                target_index.insert(
                    (occurrence.instance_path.clone(), occurrence.node),
                    *identity,
                );
                if let Some(master) = master_roots.get(identity) {
                    let mut path = occurrence.instance_path.clone();
                    path.push(occurrence.node);
                    target_index.insert((path, *master), *identity);
                }
            }
            ResolvedLayer {
                scene,
                root,
                occurrences,
                master_roots,
                target_index,
                sampled_nodes: Default::default(),
            }
        };
        let mut transforms = None;
        if include_motion {
            if let Some(active) = self.active_prototype_animation {
                if let Some(playhead) = active.playhead_ms(&self.doc) {
                    if let Some(mut motion) = self.compiled_motion.evaluate(active.clip, playhead) {
                        let mut mapped = BTreeMap::new();
                        for (target, value) in motion.overrides {
                            for identity in self.motion_candidates(
                                &resolved,
                                target.node,
                                self.active_motion_scope.as_ref(),
                            ) {
                                mapped.insert(
                                    fanta_doc::MotionTarget {
                                        node: identity,
                                        property: target.property,
                                    },
                                    value.clone(),
                                );
                            }
                        }
                        motion.overrides = mapped;
                        let mut mapped_origins = BTreeMap::new();
                        for (node, origin) in motion.transform_origins {
                            for identity in self.motion_candidates(
                                &resolved,
                                node,
                                self.active_motion_scope.as_ref(),
                            ) {
                                mapped_origins.insert(identity, origin);
                            }
                        }
                        motion.transform_origins = mapped_origins;
                        resolved
                            .sampled_nodes
                            .extend(motion.overrides.keys().map(|target| target.node));
                        let mut transform_motion = motion.clone();
                        transform_motion.overrides.retain(|target, _| {
                            !matches!(target.property, fanta_doc::MotionProperty::Bound { .. })
                        });
                        motion.overrides.retain(|target, _| {
                            matches!(target.property, fanta_doc::MotionProperty::Bound { .. })
                        });
                        let mut affected = std::collections::BTreeSet::new();
                        for target in
                            motion
                                .overrides
                                .keys()
                                .filter(|target| match target.property {
                                    fanta_doc::MotionProperty::Bound { prop } => matches!(
                                        prop,
                                        fanta_doc::BoundProp::TextContent
                                            | fanta_doc::BoundProp::TextStyle
                                            | fanta_doc::BoundProp::ClipWidth
                                            | fanta_doc::BoundProp::ClipHeight
                                            | fanta_doc::BoundProp::Visible
                                    ),
                                    _ => false,
                                })
                        {
                            affected.insert(target.node);
                            affected.extend(
                                resolved.scene.ancestors_of(target.node).map(|node| node.id),
                            );
                        }
                        Self::apply_motion_to_hit_scene(&mut resolved.scene, Some(motion));
                        if !affected.is_empty() {
                            for identity in &affected {
                                resolved
                                    .sampled_nodes
                                    .extend(resolved.scene.descendants_of(*identity));
                            }
                            fanta_doc::solve_auto_layout_scoped(
                                &mut resolved.scene,
                                root,
                                &affected,
                                &mut fanta_render::measure_text_node,
                            );
                        }
                        transforms = Some(transform_motion);
                    }
                }
            }
        }
        Self::apply_motion_to_hit_scene(&mut resolved.scene, transforms);
        let resolved = Rc::new(resolved);
        self.occurrence_cache
            .borrow_mut()
            .layers
            .insert((layer, root, include_motion), (key, resolved.clone()));
        self.prune_occurrence_cache(Some((layer, root)));
        resolved
    }

    pub(super) fn prune_occurrence_cache(&self, building: Option<(u64, NodeId)>) {
        let mut active_layers = std::collections::BTreeSet::from([self.current_layer]);
        active_layers.extend(self.overlays.iter().map(|overlay| overlay.layer));
        if let Some(exit) = &self.active_overlay_exit {
            active_layers.insert(exit.entry.layer);
        }
        if let Some((layer, _)) = building {
            active_layers.insert(layer);
        }
        let mut cache = self.occurrence_cache.borrow_mut();
        cache.layers.retain(|(layer, root, _), _| {
            building == Some((*layer, *root))
                || (*layer == self.current_layer
                    && (*root == self.current
                        || self.active_transition.is_some_and(|transition| {
                            transition.from_frame == *root || transition.to_frame == *root
                        })))
                || self
                    .overlays
                    .iter()
                    .any(|overlay| overlay.layer == *layer && overlay.root == *root)
                || self
                    .active_overlay_exit
                    .as_ref()
                    .is_some_and(|exit| exit.entry.layer == *layer && exit.entry.root == *root)
        });
        cache
            .identities
            .retain(|occurrence, _| active_layers.contains(&occurrence.layer));
        let live: std::collections::BTreeSet<_> = cache.identities.values().copied().collect();
        cache
            .explicit_styles
            .retain(|identity, _| self.doc.scene.contains(*identity) || live.contains(identity));
    }

    #[cfg(test)]
    pub(super) fn occurrence_cache_counts(&self) -> (usize, usize) {
        let cache = self.occurrence_cache.borrow();
        (cache.layers.len(), cache.identities.len())
    }

    fn stable_occurrence_identity(&self, occurrence: &NodeOccurrence) -> NodeId {
        let mut cache = self.occurrence_cache.borrow_mut();
        if let Some(identity) = cache.identities.get(occurrence).copied() {
            return identity;
        }
        let identity = loop {
            cache.next_identity = cache.next_identity.saturating_add(1);
            let candidate = NodeId::from_u128(cache.next_identity);
            if !self.doc.scene.contains(candidate) {
                break candidate;
            }
        };
        cache.identities.insert(occurrence.clone(), identity);
        identity
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_occurrence(
        &self,
        scene: &mut Scene,
        identity: NodeId,
        occurrences: &mut BTreeMap<NodeId, NodeOccurrence>,
        master_roots: &mut BTreeMap<NodeId, NodeId>,
        baked_geometry: &mut std::collections::BTreeSet<NodeId>,
        layout_changes: &mut std::collections::BTreeSet<NodeId>,
        depth: usize,
    ) {
        if depth > 64 {
            self.resolution_errors
                .borrow_mut()
                .push("component expansion depth exceeded".into());
            return;
        }
        let Some(mut node) = scene.get(identity).cloned() else {
            return;
        };
        if self.mode_generation > 0 {
            let authored = occurrences
                .get(&identity)
                .and_then(|occurrence| self.doc.scene.get(occurrence.node))
                .unwrap_or(&node);
            if fanta_doc::has_text_tokens(authored)
                || authored.bindings.iter().any(|(property, _)| {
                    matches!(
                        property,
                        fanta_doc::BoundProp::TextStyle
                            | fanta_doc::BoundProp::TextContent
                            | fanta_doc::BoundProp::ClipWidth
                            | fanta_doc::BoundProp::ClipHeight
                            | fanta_doc::BoundProp::Visible
                    )
                })
            {
                layout_changes.insert(identity);
            }
        }
        let anchor = occurrences
            .get(&identity)
            .map(|occurrence| {
                occurrence
                    .instance_path
                    .first()
                    .copied()
                    .unwrap_or(occurrence.node)
            })
            .unwrap_or(identity);
        let bindings = node.bindings.clone();
        for (property, variable) in bindings
            .iter()
            .filter(|(property, _)| **property == fanta_doc::BoundProp::TextStyle)
            .chain(
                bindings
                    .iter()
                    .filter(|(property, _)| **property != fanta_doc::BoundProp::TextStyle),
            )
        {
            if let Some(value) = fanta_doc::resolve_bound_value(
                &self.variables,
                &self.doc.scene,
                anchor,
                &self.active_modes,
                *variable,
            ) {
                property.apply_resolved(&mut node, value);
            }
        }
        fanta_doc::apply_text_tokens(
            &mut node,
            &self.variables,
            &self.doc.scene,
            anchor,
            &self.active_modes,
        );
        node.bindings.clear();
        match self.apply_runtime_style(&node) {
            Ok(patched) => node = patched,
            Err(error) => {
                self.resolution_errors.borrow_mut().push(error.to_string());
            }
        }
        if let Some(target) = scene.get_mut(identity) {
            *target = node.clone();
        }
        let NodeData::Instance(mut instance) = node.data.clone() else {
            return;
        };
        super::apply_variant_override_to_instance(
            &self.variant_overrides,
            self.instance_variant_overrides.get(&identity),
            &self.doc.components,
            &mut instance,
        );
        let context =
            fanta_doc::InstanceExpansionContext::new(&self.variables, &self.active_modes, anchor);
        let Some(component) = fanta_doc::resolved_component_with_context(
            &self.doc.scene,
            &self.doc.components,
            &instance,
            &context,
        ) else {
            return;
        };
        let mut expanded = fanta_doc::expand_instance_with_context(
            &self.doc.scene,
            &self.doc.components,
            &instance,
            &context,
        );
        rank_occurrence_siblings(expanded.iter_mut().map(|entry| &mut entry.node));
        if instance.derived.is_empty() {
            fanta_doc::solve_expanded(&mut expanded, &mut fanta_render::measure_text_node);
        }
        let Some(wrapper_occurrence) = occurrences.get(&identity).cloned() else {
            return;
        };
        let mut instance_path = wrapper_occurrence.instance_path.clone();
        instance_path.push(wrapper_occurrence.node);
        let mut remap = HashMap::new();
        for entry in &expanded {
            let authored = entry
                .def_path
                .last()
                .copied()
                .unwrap_or(component.resolved_root);
            let occurrence = if entry.def_path.is_empty() {
                wrapper_occurrence.clone()
            } else {
                NodeOccurrence {
                    layer: wrapper_occurrence.layer,
                    node: authored,
                    instance_path: instance_path.clone(),
                    definition_path: wrapper_occurrence
                        .definition_path
                        .iter()
                        .copied()
                        .chain(std::iter::once(component.resolved_root))
                        .chain(entry.def_path.iter().copied())
                        .collect(),
                }
            };
            let stable = if entry.def_path.is_empty() {
                identity
            } else {
                self.stable_occurrence_identity(&occurrence)
            };
            remap.insert(entry.node.id, stable);
            let mut typography = false;
            let mut color = false;
            for authored_override in &instance.overrides {
                let path = authored_override.target_path.as_slice();
                let path = path
                    .strip_prefix(&[component.resolved_root])
                    .unwrap_or(path);
                if path == entry.def_path.as_slice() {
                    typography |= authored_override.target_prop == fanta_doc::BoundProp::TextStyle;
                    color |= matches!(
                        authored_override.target_prop,
                        fanta_doc::BoundProp::FillColor { .. }
                    );
                }
            }
            self.occurrence_cache
                .borrow_mut()
                .explicit_styles
                .insert(stable, (typography, color));
            if !instance.derived.is_empty() {
                baked_geometry.insert(stable);
            }
            occurrences.insert(stable, occurrence);
        }
        master_roots.insert(identity, component.resolved_root);
        let mut descendants = Vec::new();
        let mut inserts = Vec::new();
        for entry in expanded {
            let Some(stable) = remap.get(&entry.node.id).copied() else {
                continue;
            };
            let mut expanded_node = entry.node;
            expanded_node.id = stable;
            if entry.def_path.is_empty() {
                match self.apply_runtime_style(&expanded_node) {
                    Ok(patched) => expanded_node = patched,
                    Err(error) => self.resolution_errors.borrow_mut().push(error.to_string()),
                }
            }
            if entry.def_path.is_empty() {
                expanded_node.parent = node.parent;
                expanded_node.index = node.index;
                expanded_node.transform = node.transform;
                expanded_node.opacity =
                    fanta_doc::UnitInterval::new(node.opacity.get() * expanded_node.opacity.get());
                let mut reactions = node.reactions.clone();
                reactions.extend(expanded_node.reactions);
                expanded_node.reactions = reactions;
                if let Some(target) = scene.get_mut(identity) {
                    *target = expanded_node;
                }
            } else {
                expanded_node.parent = expanded_node
                    .parent
                    .and_then(|parent| remap.get(&parent).copied());
                descendants.push(stable);
                inserts.push(expanded_node);
            }
        }
        if let Err(error) = scene.insert_many(inserts) {
            // A malformed component graph is left unexpanded rather than partially published.
            self.occurrence_cache.borrow_mut().layers.clear();
            self.resolution_errors.borrow_mut().push(error.to_string());
            return;
        }
        for child in descendants {
            self.resolve_occurrence(
                scene,
                child,
                occurrences,
                master_roots,
                baked_geometry,
                layout_changes,
                depth + 1,
            );
        }
    }

    fn apply_runtime_style(
        &self,
        node: &fanta_doc::CanvasNode,
    ) -> Result<fanta_doc::CanvasNode, fanta_doc::stylesheet::StyleError> {
        let mut patched = self.doc.styles.apply_local_patch_after_bindings(node)?;
        let (typography, color) = self
            .occurrence_cache
            .borrow()
            .explicit_styles
            .get(&node.id)
            .copied()
            .unwrap_or_default();
        let original = match &node.data {
            NodeData::Text(text) => Some(&text.style),
            NodeData::TextPath(text) => Some(&text.style),
            _ => None,
        };
        let target = match &mut patched.data {
            NodeData::Text(text) => Some(&mut text.style),
            NodeData::TextPath(text) => Some(&mut text.style),
            _ => None,
        };
        if let (Some(original), Some(target)) = (original, target) {
            if typography {
                *target = original.clone();
            } else if color {
                target.color = original.color;
            }
        }
        Ok(patched)
    }
}

fn rank_occurrence_siblings<'node>(
    nodes: impl IntoIterator<Item = &'node mut fanta_doc::CanvasNode>,
) {
    let mut sibling_ranks = HashMap::<Option<NodeId>, f64>::new();
    for node in nodes {
        let rank = sibling_ranks.entry(node.parent).or_default();
        // Equal authored indices have already been ordered by authored identity.
        // Unique transient ranks keep later Scene rebuilds independent of allocated identities.
        node.index = fanta_doc::IndexKey::from_raw(*rank);
        *rank += 1.0;
    }
}

#[cfg(test)]
mod ordering_tests {
    use super::*;
    use crate::{KeyEvent, PointerEvent};
    use fanta_doc::component::{ComponentDef, ComponentSet, ComponentSetMembership, VariantAxis};
    use fanta_doc::{
        Action, CanvasNode, Doc, GroupNode, InstanceNode, Reaction, ReactionId, Trigger,
    };
    use glam::DVec2;

    fn frame(identity: u128, parent: Option<NodeId>) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([20.0, 20.0]),
            ..GroupNode::default()
        }));
        node.id = NodeId::from_u128(identity);
        node.parent = parent;
        node.index = fanta_doc::IndexKey::from_raw(7.0);
        node
    }

    fn reactive_child(identity: u128, parent: NodeId, link: &str) -> CanvasNode {
        let mut node = frame(identity, Some(parent));
        for (offset, trigger) in [
            (
                0,
                Trigger::Key {
                    keys: vec!["K".into()],
                },
            ),
            (1, Trigger::AfterDelay { delay_ms: 10 }),
            (2, Trigger::Click),
        ] {
            node.reactions.push(Reaction {
                id: ReactionId::from_u128(identity + offset),
                trigger,
                action: Action::OpenLink { url: link.into() },
                extra_actions: Vec::new(),
                transition: None,
                animation: None,
            });
        }
        node
    }

    #[test]
    fn variant_sibling_order_ignores_cached_occurrence_allocation_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        let root = NodeId::from_u128(100);
        let set = ComponentId::from_u128(20);
        let default = ComponentId::from_u128(21);
        let selected = ComponentId::from_u128(22);
        let default_root = NodeId::from_u128(200);
        let selected_root = NodeId::from_u128(300);
        let earlier = NodeId::from_u128(310);
        let cached = NodeId::from_u128(320);
        let instance = NodeId::from_u128(400);
        for node in [
            frame(100, None),
            frame(200, None),
            frame(300, None),
            reactive_child(310, selected_root, "earlier"),
            reactive_child(320, selected_root, "later"),
        ] {
            document.scene.insert(node)?;
        }
        for (identity, root, name) in [
            (default, default_root, "Default"),
            (selected, selected_root, "Selected"),
        ] {
            let mut definition = ComponentDef::new(identity, root, name);
            definition.variant_of = Some(ComponentSetMembership {
                set,
                axis_values: BTreeMap::from([("State".into(), name.into())]),
            });
            document.components.defs.insert(identity, definition);
        }
        document.components.sets.insert(
            set,
            ComponentSet {
                id: set,
                root: None,
                name: "Fixture".into(),
                axes: vec![VariantAxis {
                    name: "State".into(),
                    values: vec!["Default".into(), "Selected".into()],
                }],
                members: vec![default, selected],
                default_variant: default,
            },
        );
        let mut placement = CanvasNode::new(NodeData::Instance(InstanceNode {
            component: set,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [20.0, 20.0],
        }));
        placement.id = instance;
        placement.parent = Some(root);
        document.scene.insert(placement)?;
        let authored = document.to_json_string()?;
        let mut playback = PlaybackSession::new(&document, Some(root), DVec2::splat(20.0))?;
        let cached_identity = playback.stable_occurrence_identity(&NodeOccurrence {
            layer: playback.current_layer,
            node: cached,
            instance_path: vec![instance],
            definition_path: vec![selected_root, cached],
        });
        playback.set_variant_override(set, "Selected".into());
        let resolved = playback.reaction_layer(false);
        let ordered: Vec<_> = resolved
            .scene
            .children_of(Some(instance))
            .iter()
            .filter_map(|identity| {
                resolved
                    .occurrences
                    .get(identity)
                    .map(|occurrence| occurrence.node)
            })
            .collect();
        assert_eq!(ordered, vec![earlier, cached]);
        let earlier_identity = resolved
            .occurrences
            .iter()
            .find_map(|(identity, occurrence)| (occurrence.node == earlier).then_some(*identity))
            .ok_or("missing earlier child")?;
        assert!(cached_identity < earlier_identity);
        playback.handle_key("K", KeyEvent::Down);
        assert_eq!(playback.take_open_url().as_deref(), Some("earlier"));
        playback.handle_pointer(DVec2::splat(10.0), PointerEvent::Click);
        assert_eq!(playback.take_open_url().as_deref(), Some("later"));
        playback.advance_by(0.01)?;
        assert_eq!(playback.take_open_url().as_deref(), Some("later"));
        assert_eq!(playback.doc.to_json_string()?, authored);
        assert_eq!(document.to_json_string()?, authored);
        Ok(())
    }

    #[test]
    fn overlay_sibling_order_ignores_cached_occurrence_allocation_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        let root = NodeId::from_u128(500);
        let overlay = NodeId::from_u128(600);
        let earlier = NodeId::from_u128(610);
        let later = NodeId::from_u128(620);
        for node in [
            frame(500, None),
            frame(600, None),
            reactive_child(610, overlay, "earlier"),
            reactive_child(620, overlay, "later"),
        ] {
            document.scene.insert(node)?;
        }
        let mut playback = PlaybackSession::new(&document, Some(root), DVec2::splat(20.0))?;
        playback.stable_occurrence_identity(&NodeOccurrence {
            layer: playback.next_layer,
            node: later,
            instance_path: Vec::new(),
            definition_path: Vec::new(),
        });
        playback.open_overlay(
            overlay,
            &fanta_doc::OverlaySettings {
                position: fanta_doc::OverlayPosition::Center,
                background_dim: false,
                close_on_click_outside: false,
            },
            None,
        );
        let resolved = playback.reaction_layer(false);
        assert_eq!(
            resolved
                .scene
                .children_of(Some(overlay))
                .iter()
                .filter_map(|identity| resolved
                    .occurrences
                    .get(identity)
                    .map(|occurrence| occurrence.node))
                .collect::<Vec<_>>(),
            vec![earlier, later]
        );
        playback.handle_key("K", KeyEvent::Down);
        assert_eq!(playback.take_open_url().as_deref(), Some("earlier"));
        playback.advance_by(0.01)?;
        assert_eq!(playback.take_open_url().as_deref(), Some("later"));
        Ok(())
    }
}
