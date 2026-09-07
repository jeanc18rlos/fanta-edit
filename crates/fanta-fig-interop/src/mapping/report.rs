//! The [`MapReport`] summary of what the mapping recognized and skipped.

use super::HashMap;

/// A summary of what the mapping recognized and what it skipped. Returned
/// alongside the [`Doc`] so callers can surface "imported 42 of 50 nodes; 8
/// unsupported (3 INSTANCE, 5 TEXT_PATH)" instead of silently losing content.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MapReport {
    /// Number of `nodeChanges` that became Fantaisa scene nodes.
    pub mapped: usize,
    /// Number skipped because their `NodeType` is not yet supported, keyed by
    /// the Figma type name.
    pub skipped_by_type: HashMap<String, usize>,
    /// Number skipped because the change had no usable `type`/`guid`.
    pub malformed: usize,
    /// VECTOR-family nodes (VECTOR/STAR/LINE/BOOLEAN_OPERATION/REGULAR_POLYGON)
    /// recovered (real or bbox-fallback geometry; previously skipped). Subset of
    /// `mapped`, surfaced separately so the fixture test can prove the recovery.
    pub vectors_recovered: usize,
    /// VECTOR-family nodes whose **real** path geometry was decoded from the
    /// file's `fillGeometry` command blobs (STEP 2) OR from a
    /// `vectorData.vectorNetworkBlob` fallback (STEP 2b), as opposed to the bbox
    /// fallback. Subset of `vectors_recovered`; the rest kept `meta.geometry =
    /// "bbox_fallback"`. Lets the fixture test report decoded-vs-fallback.
    pub geometry_decoded: usize,
    /// Number of nodes with constraints imported (responsive layout data).
    pub constraints_imported: usize,
    /// Of `geometry_decoded`, how many recovered their path from the
    /// `vectorData.vectorNetworkBlob` fallback (STEP 2b) rather than the
    /// pre-flattened `fillGeometry` command blobs — i.e. the vector nodes that
    /// ship only their editable VectorNetwork. Before this they regressed to a
    /// bbox rectangle (`meta.geometry = "bbox_fallback"`).
    pub vector_network_decoded: usize,
    /// Number of [`ComponentDef`]s built from SYMBOL changes.
    pub components: usize,
    /// Number of [`ComponentSet`]s built from state-group SYMBOL / COMPONENT_SET.
    pub component_sets: usize,
    /// Number of exposed component **properties** parsed from masters'
    /// `componentPropDefs` (TEXT / BOOL / INSTANCE_SWAP / VARIANT) onto
    /// [`ComponentDef::props`]. Each backs a prop the instance can assign — and
    /// whose DEFAULT applies when the instance leaves it unset. Previously 0 (the
    /// prop schema was never imported), so an instance relying on a prop default
    /// fell back to the master's authored placeholder.
    pub component_props: usize,
    /// Number of instances whose [`InstanceNode::prop_values`] were populated from
    /// `componentPropAssignments` (the typed, schema-keyed view of an instance's
    /// chosen prop values — distinct from the override list).
    pub instances_with_prop_values: usize,
    /// Number of component-property **defaults** applied to a bound descendant
    /// because the instance left that prop unset (so the descendant shows the
    /// prop default rather than the master's authored placeholder).
    pub prop_defaults_applied: usize,
    /// Number of instances of a component **set** whose variant selection (its
    /// variant `prop_values`) picked a specific member def — i.e. variant
    /// selection actually ran (vs. falling back to the set's default variant).
    pub variant_selections: usize,
    /// Number of [`InstanceNode`]s built from INSTANCE changes.
    pub instances: usize,
    /// Number of instances that carry ≥1 resolved [`Override`] after import.
    pub instances_with_overrides: usize,
    /// Total number of resolved [`Override`]s pushed onto instances (text, fill,
    /// visibility) — across both `symbolOverrides` and `componentPropAssignments`.
    pub overrides_applied: usize,
    /// Number of instances that carry ≥1 baked [`fanta_doc::node::DerivedOverride`]
    /// after import (Figma's `derivedSymbolData`, field 125 — the fully-resolved
    /// per-descendant render data).
    pub instances_with_derived: usize,
    /// Total number of [`fanta_doc::node::DerivedOverride`] entries pushed onto
    /// instances (one per resolved `derivedSymbolData` entry). Subset that
    /// resolved a `guidPath` to a real master descendant; unresolved entries are
    /// skipped, not counted.
    pub derived_overrides_applied: usize,
    /// Number of derived entries whose `fillGeometry` decoded to a real
    /// [`PathData`] (subset of `derived_overrides_applied`).
    pub derived_geometry_decoded: usize,
    /// Number of derived entries that carried a resolved `size`.
    pub derived_with_size: usize,
    /// Number of derived entries that carried a resolved `transform`.
    pub derived_with_transform: usize,
    /// Number of derived entries that carried baked text layout
    /// (`derivedTextData` or scalar font fields).
    pub derived_with_text: usize,
    /// Number of derived text entries that carried a resolved per-instance glyph
    /// color (the entry's `fillPaints` first solid). This is what lets a label on
    /// a dark page render in its real (light) color rather than the master's
    /// near-black default. Mirrors op2's derived-fill swap.
    pub derived_text_color: usize,
    /// Number of derived text entries that carried a resolved per-instance font
    /// weight (from the entry's `derivedTextData`/`fontName` style).
    pub derived_text_weight: usize,
    /// Number of [`Variable`]s built from VARIABLE changes.
    pub variables: usize,
    /// Number of [`VariableCollection`]s built from VARIABLE_SET changes.
    pub variable_collections: usize,
    /// Number of prototype reactions attached to nodes.
    pub reactions: usize,
    /// Number of prototype interactions whose `actions` array parsed to MORE
    /// than one action (Figma's "set variable then navigate" pairs). Before
    /// PR-5 these collapsed to their first action; now the first is
    /// [`fanta_doc::Reaction::action`] and the rest ride in
    /// `Reaction::extra_actions`, executed sequentially in one gesture.
    pub reactions_multi_action: usize,
    /// Number of prototype interactions DROPPED because their event's
    /// `interactionType` is not one we recognize (ON_MEDIA_END and friends).
    /// These previously defaulted to Click — firing a media-end navigation on
    /// click is worse than not firing — so the reaction is skipped and the
    /// loss surfaced here instead (R3: loud, counted degradation).
    pub reactions_dropped_unknown_trigger: usize,
    /// Number of variable bindings attached to nodes.
    pub bindings: usize,
    /// Number of per-frame variable-mode pins applied (`explicitVariableModes`):
    /// each is one `(collection -> mode)` entry resolved onto a group's
    /// [`GroupNode::explicit_modes`], so a frame that pins a non-default mode
    /// (e.g. a card forced to the Dark theme) resolves its bound values in that
    /// mode regardless of the document's active mode.
    ///
    /// [`GroupNode::explicit_modes`]: fanta_doc::node::GroupNode::explicit_modes
    pub explicit_modes_imported: usize,
    /// Number of nodes dropped because they lived inside an instance subtree
    /// (virtual content — produced by `expand_instance`, never stored).
    pub instance_children_dropped: usize,
    /// Number of component masters (SYMBOL/COMPONENT) NOT relocated to the hidden
    /// Components page because they are embedded inside design content (a
    /// FRAME/SECTION/GROUP on a real page) and therefore render in place — Figma
    /// fidelity. They still back a `ComponentDef` for instance expansion. A master
    /// whose parent is a page root or another master container is still relocated.
    pub masters_kept_in_place: usize,
    /// Number of gradient paints (LINEAR/RADIAL/ANGULAR/DIAMOND) mapped to a
    /// [`Fill::Gradient`]. Previously these were dropped (only SOLID was read),
    /// so a gradient-filled element rendered with no fill.
    pub gradients_imported: usize,
    /// Of `gradients_imported`, how many were ANGULAR (conic) gradients now
    /// mapped to a native [`Gradient::Angular`] (sweep) rather than approximated
    /// to linear.
    pub conic_gradients_imported: usize,
    /// Of `gradients_imported`, how many were DIAMOND gradients now mapped to a
    /// native [`Gradient::Diamond`] rather than approximated to radial.
    pub diamond_gradients_imported: usize,
    /// Number of image paints recognized and mapped to a [`Fill::Image`] keyed
    /// by a deterministic `AssetId` (the bitmap bytes are returned by
    /// [`fig_to_doc`] for the renderer's resolver to decode lazily).
    pub images_imported: usize,
    /// Number of distinct embedded bitmaps actually resolved to bytes — i.e.
    /// referenced `AssetId`s whose `images/<hash>` entry was present in the ZIP
    /// and handed back for decoding. `<= images_imported` (one bitmap can back
    /// many paints, and a referenced hash with no ZIP entry is not counted).
    pub image_assets_extracted: usize,
    /// Number of nodes that received ≥1 imported stroke (paint + weight + align).
    /// Counts shape (VECTOR) strokes AND frame (GROUP) border strokes.
    pub strokes_imported: usize,
    /// Number of nodes that received per-corner radius (independent corners).
    pub per_corner_radius: usize,
    /// Number of FRAME/SECTION-style groups (a group with a `clip_size`) that
    /// gained a border (≥1 stroke) on import. The frame-border fidelity fix:
    /// previously every frame's stroke was dropped at import.
    pub frames_with_stroke: usize,
    /// Number of FRAME/SECTION-style groups that gained corner rounding (uniform
    /// or per-corner) on import. Previously a frame's corner radius was dropped.
    pub frames_rounded: usize,
    /// Number of FRAME-style containers (FRAME + the SYMBOL/COMPONENT/
    /// COMPONENT_SET/INSTANCE-as-group cases) whose "clip content" toggle is OFF
    /// (`frameMaskDisabled == true`), so they keep their declared box but set
    /// `meta.clip_content=false`; overflowing children/badges/bleed content are
    /// not cropped. Previously the importer ignored the flag and clipped every
    /// frame unconditionally; 68% of frames in the Spectrum fixture set this.
    /// SECTION is not counted: a Figma section is an unclipped organizational
    /// container that keeps its declared box regardless of the flag.
    pub frames_clip_disabled: usize,
    /// Number of nodes carrying a sub-1.0 node-level opacity after import.
    pub node_opacity_imported: usize,
    /// Number of node-level effects (drop/inner shadow) imported.
    pub effects_imported: usize,
    /// Number of node-level **blur** effects (layer + background) imported into
    /// the node `blurs` list.
    pub blurs_imported: usize,
    /// Number of nodes given a non-Normal blend mode.
    pub blend_modes_imported: usize,
    /// Number of shared-style definition NodeChanges (those carrying a
    /// `styleType` field: FILL / TEXT / EFFECT). The pool the style-reference
    /// pre-pass resolves against (op2's `resolveStyleReferences`).
    pub style_def_count: usize,
    /// Number of nodes (and override entries) that carried a `styleIdForFill`
    /// ref while having an empty/absent `fillPaints` — the fills we silently
    /// dropped before the style pre-pass existed.
    pub style_ref_empty_fill: usize,
    /// Of `style_ref_empty_fill`, how many were resolved to a real paint set by
    /// inlining the referenced FILL/TEXT style's `fillPaints`.
    pub style_ref_resolved_fill: usize,
    /// Gap A — stroke-only icon vectors detected (no visible fills + visible
    /// strokes + decoded outline): their already-expanded `fillGeometry` outline
    /// was painted as a FILL (the stroke paint) instead of being decoded into a
    /// path AND re-stroked with a width, which doubled/blobbed the glyph.
    pub stroke_only_vectors: usize,
    /// Gap B — TEXT nodes whose `fontName.style` parsed to a non-400/700 weight
    /// (Thin/ExtraLight/Light/Medium/SemiBold/ExtraBold/Black). The old mapper
    /// collapsed every weight to 400 or 700.
    pub non_default_weights: usize,
    /// Gap C — nodes whose imported stroke carries a non-default cap, join, or a
    /// dash pattern (i.e. data the old `build_stroke` dropped).
    pub strokes_with_cap_join_dash: usize,
    /// Gap D — TEXT nodes whose rendered string was transformed by a non-ORIGINAL
    /// `textCase` (UPPER/LOWER/TITLE) — and the transform actually changed the
    /// string.
    pub text_case_transformed: usize,
    /// Override entries (`symbolData.symbolOverrides` + `derivedSymbolData`)
    /// whose `guidPath` had length 1 — they target a direct master descendant
    /// and resolve to a def-local path at this level (the common case, handled
    /// before and after this port). Surfaced to quantify the nested split.
    pub override_path_len1: usize,
    /// Override entries whose `guidPath` had length > 1 — they cross a
    /// nested-instance boundary. Before this port these were keyed by the
    /// terminal guid only, so they were mis-routed or dropped; now the full path
    /// is resolved across masters and the remainder routed onto the nested
    /// instance. The count we used to lose.
    pub override_path_nested: usize,
    /// Of `override_path_nested`, how many resolved their FULL cross-master path
    /// to a multi-level [`fanta_doc::node::OverridePath`] and were attached to
    /// the instance (so `expand_instance` routes them onto the nested instance).
    /// The rest had a guid that didn't resolve to a master descendant and were
    /// skipped (tolerated, never fatal).
    pub override_nested_resolved: usize,
    /// `overriddenSymbolID` swaps found on override entries (an override that
    /// SWAPS which component a nested instance points to). Previously never read.
    pub overridden_symbol_swaps: usize,
    /// Of `overridden_symbol_swaps`, how many resolved both the swapped
    /// `ComponentId` and a path to the nested instance, and were emitted as a
    /// [`fanta_doc::node::OverrideValue::SwapInstance`] override.
    pub overridden_symbol_resolved: usize,
    /// Generic [`fanta_doc::node::OverrideValue::Field`] KEYS emitted from
    /// override entries' remaining renderable fields — the ones the typed
    /// swap/text/fill/stroke/visible arms don't consume (OV-1..OV-8):
    /// `opacity`, corner radius/radii/smoothing, `effects`/`blurs`,
    /// `blend_mode`, `size`/`transform` (only when no `derivedSymbolData` entry
    /// already covers that path), text `style` scalars, per-run `style_runs`.
    /// Counted per surviving key (one Field override can carry several), so a
    /// real file's generic-override coverage becomes a number (R3).
    pub override_fields_applied: usize,
    /// Candidate Field keys DROPPED because they merely RESTATE the master's
    /// own serialized value at the target node. Figma bakes each instance's
    /// fully-resolved state into its override entries, so these are snapshots,
    /// not authored deltas — keeping them would pin the instance against master
    /// edits (the same snapshot-vs-authored rule the fills/strokes arms apply).
    pub override_fields_dropped_snapshot: usize,
    /// Component-property **instance-swap** assignments resolved: a
    /// `componentPropAssignments` entry with a `guidValue`, matched to a master
    /// descendant whose `componentPropRefs` bind `OVERRIDDEN_SYMBOL_ID` to that
    /// prop-def, and emitted as a nested
    /// [`fanta_doc::node::OverrideValue::SwapInstance`]. This is the per-instance
    /// ICON SWAP on a shared Button master (Edit vs Copy vs Delete) — without it
    /// every instance of the master shows the master's default icon.
    pub prop_instance_swap_resolved: usize,
    /// Component-property **visibility** assignments resolved: a
    /// `componentPropAssignments` entry with a `boolValue`, matched to a master
    /// descendant whose `componentPropRefs` bind `VISIBLE` to that prop-def, and
    /// emitted as an [`fanta_doc::node::OverrideValue::Visible`]. This is what
    /// hides a button's "Hold Icon" placeholder vector (the spurious ▪ when left
    /// visible).
    pub prop_visible_resolved: usize,
    /// Instances whose expansion root gained a MERGED surface background from
    /// their master root (the mergeSymbolProps surface merge in
    /// [`fanta_doc::resolve::expand_instance`]). Counted here at import time as
    /// the number of instances whose resolved master root carries a background
    /// fill — i.e. instances that now render their inherited surface.
    pub instances_with_merged_surface: usize,
    /// Descendants of an **in-place** component-set variant master (a SYMBOL
    /// whose name carries `Axis=Value` pairs and that renders directly on a
    /// design page, not via an instance) that were hidden because their
    /// `VISIBLE` is driven by a component-property `PROP_REF` whose value this
    /// static master render does not supply.
    ///
    /// Figma renders such a master with its property-bound visibility resolved:
    /// a `VISIBLE`-bound child whose prop is a **variant axis** present in the
    /// member name shows iff that axis is `True` (e.g. `Label ?=True` shows the
    /// Label, `Icon ?=False` hides the Icon); a `VISIBLE`-bound child whose prop
    /// is **not** a variant axis (a plain bool placeholder like `Hold Icon ?` /
    /// `Asterisk ?`) has no per-variant supplier and is hidden. Un-applied, every
    /// such placeholder painted its authored `visible:true` geometry — the
    /// spurious small ▪ square / ▾ triangle on the Action-Button documentation
    /// grid (and the analogous Asterisk/Help-Text/Checkmark affordances).
    pub master_placeholders_hidden: usize,

    // ---- Stage 1: auto-layout import (model+import only; no layout pass yet) ----
    /// Nodes given a HORIZONTAL [`fanta_doc::node::AutoLayout`] (`stackMode ==
    /// HORIZONTAL`). The data a future flexbox pass consumes; not yet applied.
    pub auto_layout_horizontal: usize,
    /// Nodes given a VERTICAL [`fanta_doc::node::AutoLayout`].
    pub auto_layout_vertical: usize,
    /// Of the auto-layout frames, how many HUG (RESIZE_TO_FIT) on their primary
    /// axis — the extent a layout pass would have to compute from content.
    pub auto_layout_primary_hug: usize,
    /// Of the auto-layout frames, how many HUG on their counter axis.
    pub auto_layout_counter_hug: usize,
    /// Children given a [`fanta_doc::node::LayoutChild`] with `grow > 0` (FILL on
    /// the parent's primary axis).
    pub layout_children_grow: usize,
    /// Children given a [`fanta_doc::node::LayoutChild`] with `absolute == true`
    /// (taken out of the auto-layout flow).
    pub layout_children_absolute: usize,
    /// TEXT nodes imported with `auto_resize == WidthAndHeight` (Figma auto-width
    /// — the label-never-wraps case behind truncated button labels).
    pub text_auto_width: usize,
    /// TEXT nodes imported with `auto_resize == Height` (Figma auto-height).
    pub text_auto_height: usize,
    /// TEXT nodes imported with `auto_resize == None` (fixed box, wraps).
    pub text_fixed_box: usize,
    /// Nodes whose stroke carries per-side border weights (Figma
    /// `borderStrokeWeightsIndependent` with differing
    /// `borderTop/Right/Bottom/LeftWeight`). Each such node has one or more
    /// strokes with a `Some(per_side)` field.
    pub per_side_borders: usize,
    /// Auto-layout frames imported with `wrap == true` (Figma `stackWrap` →
    /// children flow onto multiple rows/columns).
    pub auto_layout_wrap: usize,
    /// ELLIPSE nodes imported as a real arc/pie/donut/ring path (non-default
    /// `arcData`: a partial sweep and/or a non-zero inner radius), rather than the
    /// full ellipse.
    pub arc_ellipses: usize,
    /// Nodes imported with `is_mask == true` (Figma `mask` / `isMask`): each
    /// masks its following siblings within its parent. The renderer composites
    /// the masked siblings against the mask with `DstIn`.
    pub masks_imported: usize,
    /// Nodes whose GLASS effect (Figma's liquid-glass) was approximated as a
    /// backdrop blur — the refraction/specular components are not modeled.
    pub effects_glass_approximated: usize,
    /// Effect entries whose `type` is a member we neither map nor approximate
    /// (NOISE, TEXTURE, …): the effect is dropped, and that loss is counted
    /// here instead of disappearing silently.
    pub effects_dropped_unknown: usize,
    /// Frames whose authored prototype overflow (`scrollDirection`) enables at
    /// least one scroll axis. Distinguishes real scrolling containers from the
    /// legacy every-frame `scrollable` heuristic this replaced.
    pub scroll_frames: usize,
    /// Nodes carrying a non-default `scrollBehavior` (fixed headers / sticky
    /// rows) — the children the present runtime pins while content scrolls.
    pub scroll_pinned_children: usize,
    /// Of `masks_imported`, how many use [`fanta_doc::node::MaskType::Luminance`]
    /// (mask by the mask's painted luminance) rather than the default ALPHA
    /// coverage.
    pub masks_luminance: usize,

    // ---- Figma keyframe motion (ANIMATION_PRESET_INSTANCE / KEYFRAME_TRACK /
    // KEYFRAME → doc.motion; see mapping/motion.rs) ----
    /// Number of [`fanta_doc::motion::AnimationClip`]s built from
    /// ANIMATION_PRESET_INSTANCE groupings (one clip per preset instance whose
    /// tracks resolved; a not-yet-observed preset-less track grouping also
    /// lands here). Previously all three motion NodeChange types were skipped
    /// outright, so an authored animation imported as nothing.
    pub motion_clips_imported: usize,
    /// Number of [`fanta_doc::motion::AnimationTrack`]s built — one per
    /// (KEYFRAME_TRACK, bound scene node, motion channel) triple resolved
    /// through the consumers' KEYFRAME-expression bindings.
    pub motion_tracks_imported: usize,
    /// Number of [`fanta_doc::motion::Keyframe`]s imported onto those tracks
    /// (time µs→ms, float value composed per the track's `keyframeOperation`,
    /// easing mapped onto [`fanta_doc::node::Easing`] /
    /// [`fanta_doc::motion::Interpolation`]).
    pub motion_keyframes_imported: usize,
    /// Number of KEYFRAME changes DROPPED because they could not be expressed:
    /// a non-FLOAT `keyframeValue`, a track no consumer binds, a binding whose
    /// `VariableField` channel the motion model can't address (MOTION_SHEAR,
    /// 3D / vector-valued channels), a consumer node that was dropped, or a
    /// committed transform with no channel decomposition (shear). Loud,
    /// counted degradation — never silent.
    pub motion_keyframes_dropped: usize,
    /// ANIMATION_PRESET_INSTANCEs that yielded NO clip (they own no tracks, or
    /// every track failed to resolve a binding). The preset reference itself
    /// (`name` + parameters) has no home in the motion model beyond the clip
    /// name, so an unresolvable preset is counted here rather than invented.
    pub motion_presets_dropped: usize,
}

impl MapReport {
    /// Total skipped (unsupported + malformed). Does NOT include
    /// `instance_children_dropped`, which is intentional virtual-subtree pruning
    /// rather than an unrecognized node.
    pub fn skipped(&self) -> usize {
        self.skipped_by_type.values().sum::<usize>() + self.malformed
    }
}
