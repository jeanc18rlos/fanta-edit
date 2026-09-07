use gpui::SharedString;

use super::*;

/// One immutable Figma inspector-menu hover candidate.
///
/// The payload deliberately repeats the original and candidate values on the
/// matching [`DesignMenuPreviewPhase::End`] event. Hosts can therefore unwind
/// exactly the preview that began even after a controlled selection, effect,
/// or paint echo has invalidated the panel's current indices.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignMenuPreview {
    /// A node property resolved against the exact ordered page selection.
    NodeProperty {
        target: DesignPanelTarget,
        property: DesignPanelProperty,
        original: DesignPanelValue,
        candidate: DesignPanelValue,
    },
    /// An effect property resolved by stable effect ID with an index fallback
    /// for legacy hosts that do not yet supply IDs.
    EffectProperty {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
        property: DesignPanelProperty,
        original: DesignPanelValue,
        candidate: DesignPanelValue,
    },
    /// A paint property resolved by exact text/object scope, stable paint ID,
    /// collection, and compatibility index.
    PaintProperty {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        property: DesignPaintProperty,
        original: DesignPaintValue,
        candidate: DesignPaintValue,
    },
}

/// Balanced lifecycle for an inspector-menu hover preview.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignMenuPreviewPhase {
    Begin,
    End,
}

/// Typed intent emitted by [`super::DesignPanel`].
#[derive(Clone, Debug, PartialEq)]
pub enum DesignPanelAction {
    /// Compatibility commit-only replacement for `PageNode.backgrounds`.
    ///
    /// The retained color picker emits [`Self::PageBackgroundEditRequested`].
    PageBackgroundChangeRequested {
        page_id: SharedString,
        color: DesignColor,
    },
    /// Phased replacement for the one solid paint exposed by
    /// `PageNode.backgrounds`.
    PageBackgroundEditRequested {
        page_id: SharedString,
        color: DesignColor,
        phase: DesignPanelEditPhase,
    },
    /// Opens one host-supplied Page resource category. Browsing is
    /// non-mutating and remains available in view-only contexts.
    #[deprecated(note = "canonical Page styles render DesignPageLocalStylesViewData directly")]
    LocalResourceBrowseRequested {
        page_id: SharedString,
        category: DesignLocalResourceCategory,
    },
    /// Opens an already-local style or variable collection by stable source
    /// and resource ID.
    #[deprecated(note = "use LocalStyleCommandRequested")]
    LocalResourceOpenRequested {
        page_id: SharedString,
        resource: DesignLocalResourceSelection,
    },
    /// Creates one exact local style or variable-collection kind.
    #[deprecated(note = "use LocalStyleCreateRequested")]
    LocalResourceCreateRequested {
        page_id: SharedString,
        kind: DesignLocalResourceKind,
    },
    /// Imports one exact discoverable library resource.
    #[deprecated(note = "library importing is not part of the canonical Page Styles surface")]
    LocalResourceImportRequested {
        page_id: SharedString,
        resource: DesignLocalResourceSelection,
    },
    /// Runs one command against a style whose Page, family, direct parent,
    /// stable ID, and current sibling index all still match the host snapshot.
    LocalStyleCommandRequested {
        target: DesignLocalStyleTarget,
        command: DesignLocalStyleCommand,
    },
    /// Creates one local style in the exact current-file family/folder.
    LocalStyleCreateRequested {
        page_id: SharedString,
        kind: DesignLocalStyleKind,
        parent_folder_id: Option<SharedString>,
    },
    /// Creates a folder and optionally places an exact ordered style
    /// selection into it. An empty selection requests an empty folder.
    LocalStyleFolderCreateRequested {
        page_id: SharedString,
        kind: DesignLocalStyleKind,
        selected: Vec<DesignLocalStyleTarget>,
    },
    /// Deletes one or more exact current style leaves atomically.
    LocalStylesDeleteRequested {
        targets: Vec<DesignLocalStyleTarget>,
    },
    /// Moves exact current style leaves to an exact, neighbor-validated
    /// insertion point.
    LocalStylesMoveRequested {
        targets: Vec<DesignLocalStyleTarget>,
        destination: DesignLocalStyleInsertion,
    },
    /// Opens the host's Variables surface from the explicitly enabled legacy
    /// right-sidebar entry point. This is navigation only; it never imports or
    /// creates a collection.
    VariablesViewOpenRequested {
        page_id: SharedString,
    },
    /// Sets an explicit mode for one collection on the exact page or
    /// single-node target. The panel never updates its resolved snapshot.
    VariableModeApplyRequested {
        target: DesignPanelTarget,
        collection_id: SharedString,
        mode_id: SharedString,
    },
    /// Clears the exact current explicit mode. Carrying the old mode ID lets
    /// hosts reject stale intents after a concurrent variable-mode echo.
    VariableModeClearRequested {
        target: DesignPanelTarget,
        collection_id: SharedString,
        explicit_mode_id: SharedString,
    },
    /// Requests a host-controlled right-sidebar navigation change.
    ///
    /// This is a non-document intent. The panel keeps rendering `current`
    /// until the host validates `requested` and echoes it through
    /// `DesignPanel::set_active_surface`.
    SurfaceChangeRequested {
        current: DesignPanelSurface,
        requested: DesignPanelSurface,
    },
    SelectionHeaderCommandRequested {
        target: DesignPanelTarget,
        command: DesignSelectionHeaderCommand,
    },
    /// Requests Figma's structural “Add auto layout” operation for the exact
    /// ordered current selection.
    ///
    /// The host may convert a group/frame-like node or create a new wrapping
    /// frame around arbitrary selected layers. The reusable panel never
    /// predicts a wrapper identity, node kind, layout direction, or selection;
    /// all of those arrive in a fresh inspection-context echo.
    AddAutoLayoutRequested {
        target: DesignPanelTarget,
    },
    /// Carries the exact ordered node selection for one ordinary inspector
    /// action whose leaf payload historically named only the aggregate node.
    ///
    /// `target` is authoritative. The nested action retains its single
    /// `node_id` compatibility hint so existing single-node host reducers can
    /// be reused per target while migrating to an atomic multi-node operation.
    /// The panel emits this envelope whenever an ordinary document-facing
    /// action is triggered for a multiple selection.
    TargetedNodeActionRequested {
        target: DesignPanelTarget,
        action: Box<DesignPanelAction>,
    },
    TypographyPropertyChangeRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    },
    TypographyPropertyEditRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        phase: DesignPanelEditPhase,
    },
    /// Changes one exact host-enumerated variable-font axis.
    ///
    /// `tag` is authoritative; a row index is intentionally absent so a host
    /// may reorder its axis snapshot while an edit is in flight. Continuous
    /// slider, keyboard, and numeric-input interactions use the standard
    /// Begin/Preview/Commit/Cancel lifecycle.
    TypographyVariableAxisEditRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        tag: SharedString,
        value: f32,
        phase: DesignPanelEditPhase,
    },
    /// Applies one exact host-supplied page or library text style. The host
    /// resolves the stable selection and echoes a fresh typography snapshot.
    TypographyStyleApplyRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        style: DesignTypographyStyleSelection,
    },
    /// Detaches the exact current style binding while preserving its resolved
    /// typography values in host-owned document state.
    TypographyStyleDetachRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        style: DesignTypographyStyleSelection,
    },
    /// Applies an exact host-enumerated font family/style pair.
    TypographyFontApplyRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        font: DesignFontSelection,
    },
    /// Imports an exact discoverable font. Import and apply are separate
    /// operations so the host can echo loading or failure state.
    TypographyFontImportRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        font: DesignFontSelection,
    },
    /// Changes one exact host-supplied OpenType feature tag.
    TypographyOpenTypeFeatureChangeRequested {
        node_id: SharedString,
        target: DesignTypographyTarget,
        tag: DesignOpenTypeFeatureTag,
        enabled: bool,
    },
    /// Requests Figma's native whole-node "Flip text orientation" command.
    ///
    /// Orientation is intentionally not predicted. The host resolves the
    /// command for this exact TextPath identity and echoes fresh
    /// [`DesignTextPathViewData`].
    TextPathFlipOrientationRequested {
        node_id: SharedString,
    },
    /// Replaces the complete `TextPathNode.textPathStartData` record.
    TextPathStartChangeRequested {
        node_id: SharedString,
        data: DesignTextPathStartData,
        phase: DesignPanelEditPhase,
    },
    /// Replaces the host-owned vector sub-selection by opaque vertex identity.
    VectorVertexSelectionEditRequested {
        node_id: SharedString,
        selected_vertex_ids: Vec<SharedString>,
        phase: DesignPanelEditPhase,
    },
    /// Sets one coordinate for every exact selected vertex identity.
    VectorVertexPositionEditRequested {
        node_id: SharedString,
        vertex_ids: Vec<SharedString>,
        axis: DesignVectorCoordinateAxis,
        value: f32,
        phase: DesignPanelEditPhase,
    },
    /// Sets Figma's per-vertex `cornerRadius` for exact selected identities.
    VectorVertexCornerRadiusEditRequested {
        node_id: SharedString,
        vertex_ids: Vec<SharedString>,
        radius: f32,
        phase: DesignPanelEditPhase,
    },
    /// Sets Figma's writable `HandleMirroring` for exact selected identities.
    VectorHandleMirroringEditRequested {
        node_id: SharedString,
        vertex_ids: Vec<SharedString>,
        mirroring: DesignHandleMirroring,
        phase: DesignPanelEditPhase,
    },
    PropertyChangeRequested {
        node_id: SharedString,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    },
    PropertyEditRequested {
        node_id: SharedString,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        phase: DesignPanelEditPhase,
    },
    /// Starts or ends Figma's on-canvas min/max bounds preview for one axis.
    ///
    /// The values are the exact immutable host snapshot shown by the
    /// inspector. `preview=false` carries the same captured values as the
    /// matching start so a host can safely unwind a preview after a selection
    /// or property echo.
    DimensionLimitsPreviewRequested {
        node_id: SharedString,
        axis: DesignLayoutDimensionAxis,
        minimum: Option<f32>,
        maximum: Option<f32>,
        preview: bool,
    },
    /// Starts or ends a non-mutating preview for one exact inspector-menu
    /// candidate. A matching `End` always repeats the immutable `Begin`
    /// payload; committed document state changes only through the ordinary
    /// property, paint, or effect action emitted after preview cleanup.
    MenuPreviewRequested {
        preview: DesignMenuPreview,
        phase: DesignMenuPreviewPhase,
    },
    /// Requests that the host copy one generic inspector readout exactly as
    /// displayed. This is a non-mutating viewer action: the reusable panel
    /// never writes to the clipboard itself.
    PropertyCopyRequested {
        target: DesignPanelTarget,
        property: DesignPanelProperty,
        displayed_value: SharedString,
    },
    /// Requests that the host copy one exact view-only Properties section.
    ///
    /// The reusable panel never writes to the clipboard. `copy_value` is
    /// supplied by the host projection so CSS and color conversions stay
    /// lossless and host controlled.
    ViewerSectionCopyRequested {
        target: DesignPanelTarget,
        section_id: SharedString,
        copy_value: SharedString,
    },
    /// Requests a different host-owned representation for one view-only
    /// Properties section. The visible rows do not change until the host
    /// echoes a fresh [`DesignViewerPropertiesViewData`] snapshot.
    ViewerSectionRepresentationChangeRequested {
        target: DesignPanelTarget,
        section_id: SharedString,
        representation: DesignViewerColorRepresentation,
    },
    /// Applies an imported/page variable to every exact API field represented
    /// by one inspector property. The panel never changes its controlled
    /// property snapshot while waiting for the host echo.
    PropertyVariableApplyRequested {
        node_id: SharedString,
        target: DesignPropertyVariableTarget,
        variable_id: SharedString,
    },
    /// Requests import of one discoverable library variable. Import and apply
    /// are intentionally separate host operations.
    PropertyVariableImportRequested {
        node_id: SharedString,
        target: DesignPropertyVariableTarget,
        variable_id: SharedString,
    },
    /// Detaches the exact current variable from all API fields represented by
    /// the property while preserving the host-resolved raw value.
    PropertyVariableDetachRequested {
        node_id: SharedString,
        target: DesignPropertyVariableTarget,
        variable_id: SharedString,
    },
    SectionShareRequested {
        node_id: SharedString,
    },
    SectionResolveChangedStatusRequested {
        node_id: SharedString,
    },
    TransformModifierAddRequested {
        node_id: SharedString,
        repeat_type: DesignRepeatType,
    },
    TransformModifierRemoveRequested {
        node_id: SharedString,
        modifier_id: SharedString,
        index: usize,
    },
    TransformModifierChangeRequested {
        node_id: SharedString,
        modifier_id: SharedString,
        index: usize,
        change: DesignTransformModifierChange,
        phase: DesignPanelEditPhase,
    },
    ApplyTransformModifiersRequested {
        node_id: SharedString,
    },
    /// Creates a new definition at the end of Figma's Variant or regular
    /// component-property partition.
    ///
    /// The host owns the generated property identity. `expected_property_order`
    /// and `after_property_id` are stable stale-intent guards captured when the
    /// modal is submitted; neither value is a parsed document identifier.
    ComponentPropertyDefinitionCreateRequested {
        node_id: SharedString,
        kind: DesignComponentPropertyKind,
        name: SharedString,
        /// Slot descriptions are authored in the create modal. Other kinds
        /// currently carry `None`, matching Figma's property schema.
        description: Option<SharedString>,
        documentation_links: Vec<DesignDocumentationLink>,
        definition: DesignComponentPropertyDefinition,
        /// Stable variable catalog identity for Boolean/Text default values.
        default_variable_id: Option<SharedString>,
        partition: DesignComponentPropertyPartition,
        expected_property_order: Vec<SharedString>,
        after_property_id: Option<SharedString>,
    },
    /// Edits one exact definition name through the panel's inline editor.
    ///
    /// `original_name` is the value at `Begin`; `expected_name` is the latest
    /// controlled host echo. Together with `phase`, these fields let a host
    /// balance preview/commit/cancel without accepting a stale overwrite.
    ComponentPropertyDefinitionRenameRequested {
        node_id: SharedString,
        property_id: SharedString,
        original_name: SharedString,
        expected_name: SharedString,
        name: SharedString,
        phase: DesignPanelEditPhase,
    },
    /// Commits one exact description from the Edit property modal.
    ComponentPropertyDefinitionMetadataEditRequested {
        node_id: SharedString,
        property_id: SharedString,
        expected_description: Option<SharedString>,
        description: Option<SharedString>,
        expected_documentation_links: Vec<DesignDocumentationLink>,
        documentation_links: Vec<DesignDocumentationLink>,
    },
    /// Atomically commits Figma's Edit property modal for one exact property.
    ///
    /// Metadata and the typed definition share one stale guard and one host
    /// transaction so a combined confirmation cannot be partially accepted or
    /// split across undo entries. Slot settings live inside `definition`.
    ComponentPropertyDefinitionEditRequested {
        node_id: SharedString,
        property_id: SharedString,
        expected_description: Option<SharedString>,
        description: Option<SharedString>,
        expected_documentation_links: Vec<DesignDocumentationLink>,
        documentation_links: Vec<DesignDocumentationLink>,
        expected_definition: DesignComponentPropertyDefinition,
        definition: DesignComponentPropertyDefinition,
    },
    ComponentPropertyDefinitionDeleteRequested {
        node_id: SharedString,
        property_id: SharedString,
        expected_name: SharedString,
    },
    /// Reorders within one Figma definition partition.
    ///
    /// `before_property_id` is `None` when moving to the end. The original and
    /// expected stable-ID orders keep a drag cancelable and reject stale or
    /// cross-partition moves without relying on display indexes.
    ComponentPropertyDefinitionReorderRequested {
        node_id: SharedString,
        property_id: SharedString,
        partition: DesignComponentPropertyPartition,
        original_property_order: Vec<SharedString>,
        expected_property_order: Vec<SharedString>,
        before_property_id: Option<SharedString>,
        phase: DesignPanelEditPhase,
    },
    ComponentVariantOptionCreateRequested {
        node_id: SharedString,
        property_id: SharedString,
        name: SharedString,
        expected_option_order: Vec<SharedString>,
        after_option_id: Option<SharedString>,
    },
    ComponentVariantOptionRenameRequested {
        node_id: SharedString,
        property_id: SharedString,
        option_id: SharedString,
        original_name: SharedString,
        expected_name: SharedString,
        name: SharedString,
        phase: DesignPanelEditPhase,
    },
    ComponentVariantOptionDeleteRequested {
        node_id: SharedString,
        property_id: SharedString,
        option_id: SharedString,
        expected_name: SharedString,
    },
    ComponentVariantOptionReorderRequested {
        node_id: SharedString,
        property_id: SharedString,
        option_id: SharedString,
        original_option_order: Vec<SharedString>,
        expected_option_order: Vec<SharedString>,
        before_option_id: Option<SharedString>,
        phase: DesignPanelEditPhase,
    },
    ComponentPropertyApplyToLayerRequested {
        node_id: SharedString,
        control_id: SharedString,
        layer_id: SharedString,
        surface: DesignComponentPropertyApplicationSurface,
        property_id: SharedString,
    },
    ComponentPropertySwitchOnLayerRequested {
        node_id: SharedString,
        control_id: SharedString,
        layer_id: SharedString,
        surface: DesignComponentPropertyApplicationSurface,
        from_property_id: SharedString,
        to_property_id: SharedString,
    },
    ComponentPropertyDetachFromLayerRequested {
        node_id: SharedString,
        control_id: SharedString,
        layer_id: SharedString,
        surface: DesignComponentPropertyApplicationSurface,
        property_id: SharedString,
    },
    NestedComponentPropertyExposeRequested {
        node_id: SharedString,
        candidate_id: SharedString,
        nested_instance_id: SharedString,
        nested_property_id: SharedString,
    },
    NestedComponentPropertyUnexposeRequested {
        node_id: SharedString,
        candidate_id: SharedString,
        nested_instance_id: SharedString,
        nested_property_id: SharedString,
        exposed_property_id: SharedString,
    },
    /// `preview=true` highlights one nested source; `false` restores the
    /// host's prior canvas presentation without changing the document.
    NestedComponentPropertyPreviewRequested {
        node_id: SharedString,
        candidate_id: SharedString,
        nested_instance_id: SharedString,
        nested_property_id: SharedString,
        preview: bool,
    },
    ComponentPropertyChangeRequested {
        node_id: SharedString,
        property_id: SharedString,
        value: DesignComponentPropertyValue,
    },
    ComponentPropertyEditRequested {
        node_id: SharedString,
        property_id: SharedString,
        value: DesignComponentPropertyValue,
        phase: DesignPanelEditPhase,
    },
    ComponentPropertyResetRequested {
        node_id: SharedString,
        property_id: SharedString,
    },
    /// Applies one page/imported variable to the exact `defaultValue` or
    /// instance `value` field represented by `target`.
    ComponentPropertyVariableApplyRequested {
        node_id: SharedString,
        target: DesignComponentPropertyVariableTarget,
        variable_id: SharedString,
    },
    ComponentPropertyVariableImportRequested {
        node_id: SharedString,
        target: DesignComponentPropertyVariableTarget,
        variable_id: SharedString,
    },
    ComponentPropertyVariableDetachRequested {
        node_id: SharedString,
        target: DesignComponentPropertyVariableTarget,
        variable_id: SharedString,
    },
    /// Accepts one exact all-components browser selection. `None` clears an
    /// optional instance-swap property.
    ComponentSwapApplyRequested {
        node_id: SharedString,
        property_id: SharedString,
        selection: Option<DesignComponentSwapSelection>,
    },
    ComponentSwapImportRequested {
        node_id: SharedString,
        property_id: SharedString,
        selection: DesignComponentSwapSelection,
    },
    /// `Some` starts/changes hover preview and `None` restores host state.
    ComponentSwapPreviewRequested {
        node_id: SharedString,
        property_id: SharedString,
        selection: Option<DesignComponentSwapSelection>,
    },
    ComponentPropertyNestedInstanceSelectRequested {
        node_id: SharedString,
        property_id: SharedString,
        instance_id: SharedString,
    },
    ComponentPropertyNestedInstanceGoToMainRequested {
        node_id: SharedString,
        property_id: SharedString,
        instance_id: SharedString,
        main_component_id: SharedString,
    },
    SlotSettingsChangeRequested {
        node_id: SharedString,
        property_id: SharedString,
        expected_settings: DesignSlotSettings,
        change: DesignSlotSettingsChange,
        phase: DesignPanelEditPhase,
    },
    SlotResetRequested {
        node_id: SharedString,
        property_id: SharedString,
    },
    SlotClearRequested {
        node_id: SharedString,
        property_id: SharedString,
    },
    SlotAddInstanceRequested {
        node_id: SharedString,
        property_id: SharedString,
        preferred_component: Option<DesignComponentReference>,
    },
    /// Selects/navigates to one stable layer contained by a Slot.
    SlotChildSelectRequested {
        node_id: SharedString,
        property_id: SharedString,
        child_node_id: SharedString,
    },
    /// Selects every current layer that violates a Slot's preferred-instance
    /// guideline.
    ///
    /// The ordered IDs are an exact stale-intent guard captured from the
    /// host-controlled Slot value and violation snapshot. Hosts must reject
    /// the request if the Slot contents or violations no longer resolve to
    /// this same ordered set.
    SlotLimitLayersSelectRequested {
        node_id: SharedString,
        property_id: SharedString,
        child_node_ids: Vec<SharedString>,
    },
    /// Removes one stable Slot child. `index` is only an ordering hint.
    SlotChildRemoveRequested {
        node_id: SharedString,
        property_id: SharedString,
        child_node_id: SharedString,
        index: usize,
    },
    /// Reorders one stable Slot child within the Slot's back-to-front list.
    SlotChildReorderRequested {
        node_id: SharedString,
        property_id: SharedString,
        child_node_id: SharedString,
        from_index: usize,
        to_index: usize,
    },
    /// Replaces an instance child while preserving its stable Slot position.
    SlotChildReplaceRequested {
        node_id: SharedString,
        property_id: SharedString,
        child_node_id: SharedString,
        replacement: DesignComponentReference,
    },
    CollectionItemAddRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
    },
    CollectionItemRemoveRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        index: usize,
    },
    /// Adds the first exact effect kind selected by the panel from the host's
    /// availability and Figma's per-kind count limits.
    EffectAddRequested {
        node_id: SharedString,
        kind: DesignEffectKind,
    },
    /// Removes one stable effect row. `index` is a compatibility hint only.
    EffectRemoveRequested {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
    },
    /// Reorders one effect without invalidating its anchored settings popup.
    EffectReorderRequested {
        node_id: SharedString,
        effect_id: SharedString,
        from_index: usize,
        to_index: usize,
    },
    /// Edits one stable effect row. Shader leaves additionally carry their
    /// property-definition ID so the host never resolves a stale property
    /// index after a shader metadata echo.
    EffectEditRequested {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
        property: DesignPanelProperty,
        shader_property_id: Option<SharedString>,
        value: DesignPanelValue,
        phase: DesignPanelEditPhase,
    },
    /// Opens the host's imported/available shader chooser.
    EffectShaderChooseRequested {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
    },
    /// Opens a host-owned resource or variable chooser for one stable Shader
    /// property-definition. The panel never invents asset or variable IDs.
    EffectShaderPropertyEditorRequested {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
        shader_property_id: SharedString,
        property_index: usize,
        property_kind: DesignShaderPropertyKind,
        target: DesignShaderPropertyEditorTarget,
        editor: DesignShaderPropertyEditorKind,
        current_value: DesignShaderPropertyValue,
    },
    /// Detaches an echoed Shader-property variable binding. `variable_id`
    /// lets a host reject a stale request.
    EffectShaderPropertyVariableDetachRequested {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
        shader_property_id: SharedString,
        property_index: usize,
        target: DesignShaderPropertyEditorTarget,
        variable_id: SharedString,
    },
    EffectStyleApplyRequested {
        node_id: SharedString,
        style: DesignEffectStyleSelection,
    },
    EffectStyleCreateRequested {
        node_id: SharedString,
        effects: Vec<DesignEffect>,
    },
    EffectStyleDetachRequested {
        node_id: SharedString,
        style: DesignEffectStyleSelection,
    },
    EffectVariableApplyRequested {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
        field: DesignEffectVariableField,
        variable_id: SharedString,
    },
    EffectVariableDetachRequested {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
        field: DesignEffectVariableField,
        variable_id: SharedString,
    },
    /// Atomically requests positive column and row counts for one exact Grid
    /// container. The host decides how existing tracks and children relocate,
    /// then echoes complete authoritative track vectors.
    ///
    /// When rows are host-derived through [`DesignGridAutoTracks::Rows`], the
    /// panel preserves the echoed row count and emits column-only changes with
    /// that exact current row count.
    GridDimensionsEditRequested {
        node_id: SharedString,
        dimensions: DesignGridDimensions,
        phase: DesignPanelEditPhase,
    },
    /// Inserts one explicit Grid track. The host owns the resulting track
    /// vector and echoes it through [`DesignPanelNode`].
    GridTrackAddRequested {
        node_id: SharedString,
        axis: DesignGridTrackAxis,
        insertion_index: usize,
    },
    /// Deletes one explicit Grid track. The panel suppresses this intent when
    /// it would remove the final track or an auto-managed row.
    GridTrackDeleteRequested {
        node_id: SharedString,
        axis: DesignGridTrackAxis,
        index: usize,
    },
    /// Reorders explicit Grid tracks using Figma's `fromIndices` and
    /// `insertionIndex` semantics.
    GridTracksReorderRequested {
        node_id: SharedString,
        axis: DesignGridTrackAxis,
        from_indices: Vec<usize>,
        insertion_index: usize,
    },
    LayoutGridStyleApplyRequested {
        node_id: SharedString,
        style: DesignLayoutGridStyleSelection,
    },
    LayoutGridStyleCreateRequested {
        node_id: SharedString,
        layout_grids: Vec<DesignLayoutGrid>,
    },
    LayoutGridStyleDetachRequested {
        node_id: SharedString,
        style: DesignLayoutGridStyleSelection,
    },
    LayoutGridStyleImportRequested {
        node_id: SharedString,
        style: DesignLayoutGridStyleSelection,
    },
    /// Edits one ordinary layout guide by stable host identity.
    ///
    /// `index` is only a compatibility hint for legacy guides whose `id` is
    /// empty. Hosts must resolve non-empty `guide_id` values against their
    /// latest ordered guide snapshot on every phase.
    LayoutGridPropertyEditRequested {
        node_id: SharedString,
        guide_id: SharedString,
        index: usize,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        phase: DesignPanelEditPhase,
    },
    /// Removes one ordinary layout guide by stable host identity.
    ///
    /// `index` is only a compatibility hint when `guide_id` is empty.
    LayoutGridRemoveRequested {
        node_id: SharedString,
        guide_id: SharedString,
        index: usize,
    },
    /// Applies one already-local Number variable to an exact, stable
    /// layout-guide leaf.
    LayoutGridVariableApplyRequested {
        node_id: SharedString,
        target: DesignLayoutGridVariableTarget,
        variable_id: SharedString,
    },
    /// Imports one available library Number variable. Import and apply remain
    /// separate host operations.
    LayoutGridVariableImportRequested {
        node_id: SharedString,
        target: DesignLayoutGridVariableTarget,
        variable_id: SharedString,
    },
    /// Detaches the exact echoed binding while preserving the resolved raw
    /// guide value.
    LayoutGridVariableDetachRequested {
        node_id: SharedString,
        target: DesignLayoutGridVariableTarget,
        variable_id: SharedString,
    },
    /// Opens the host's Number-variable creation flow for one exact guide
    /// field. The panel never manufactures a collection or variable ID.
    LayoutGridVariableCreateRequested {
        node_id: SharedString,
        target: DesignLayoutGridVariableTarget,
        value: DesignLayoutGridVariableValue,
    },
    #[deprecated(note = "use LayoutGridVariableApplyRequested with a Count target")]
    LayoutGridCountVariableApplyRequested {
        node_id: SharedString,
        guide_id: SharedString,
        index: usize,
        variable_id: SharedString,
    },
    #[deprecated(note = "use LayoutGridVariableDetachRequested with a Count target")]
    LayoutGridCountVariableDetachRequested {
        node_id: SharedString,
        guide_id: SharedString,
        index: usize,
        variable_id: SharedString,
    },
    #[deprecated(note = "use DesignPanelAction::PaintEditRequested")]
    PaintChangeRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        index: usize,
        paint: DesignPaint,
    },
    /// Typed paint edit keyed by stable paint identity with a legacy index
    /// fallback. The host applies the edit and echoes a fresh paint snapshot.
    PaintEditRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        edit: DesignPaintEdit,
        phase: DesignPanelEditPhase,
    },
    /// Reorders one paint without invalidating an open picker for a stable ID.
    PaintReorderRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        from_index: usize,
        to_index: usize,
    },
    /// Applies an imported/page Paint style to the complete ordered Fill or
    /// Stroke paint collection.
    PaintStyleApplyRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        style: DesignPaintStyleSelection,
    },
    /// Imports one discoverable library Paint style. Import and apply are
    /// intentionally separate host operations.
    PaintStyleImportRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        style: DesignPaintStyleSelection,
    },
    /// Opens the host's Paint-style creation flow with the exact ordered
    /// collection snapshot.
    PaintStyleCreateRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paints: Vec<DesignPaint>,
    },
    /// Detaches the exact current `fillStyleId`/`strokeStyleId` while
    /// preserving the host-resolved ordered paints.
    PaintStyleDetachRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        style: DesignPaintStyleSelection,
    },
    /// Opens the host's source-node chooser for a Pattern paint.
    PaintSourceReplaceRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
    },
    /// Opens one exact host-owned image/video source workflow. `source_id`
    /// snapshots the currently controlled source so an asynchronous chooser,
    /// generation, or editing result cannot be applied to a subsequently
    /// replaced source.
    PaintMediaSourceActionRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        source_id: SharedString,
        action: DesignMediaSourceAction,
    },
    /// Replaces one exact image/video source from a validated external file.
    ///
    /// `expected_source_id` and `expected_media_kind` snapshot the controlled
    /// media payload. The host rejects the intent if either changed before it
    /// resolves the stable paint identity. The reusable panel never opens,
    /// reads, uploads, or retains the file.
    PaintMediaSourceDropRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        expected_source_id: SharedString,
        expected_media_kind: DesignMediaKind,
        file: DesignMediaDroppedFile,
    },
    /// Drives a host-controlled crop-tool session for one stable image/video
    /// paint. Preview/commit payloads carry the candidate affine crop without
    /// mutating the controlled paint snapshot in the panel.
    PaintMediaCropActionRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        action: DesignMediaCropAction,
    },
    /// Controls Design-tab video preview only. Playback never becomes a paint
    /// document property.
    PaintVideoPreviewActionRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        action: DesignVideoPreviewAction,
    },
    /// Imports one discoverable fill shader into the file. The host performs
    /// the async import and echoes refreshed [`DesignShaderViewData`].
    PaintShaderImportRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        shader: DesignShaderSelection,
    },
    /// Applies one already-imported fill shader to this stable paint row.
    /// The host resolves author defaults and echoes the resulting payload.
    PaintShaderApplyRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        shader: DesignShaderSelection,
    },
    /// Opens the host's variable picker for one shader property-definition id.
    PaintShaderPropertyBindRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        definition_id: SharedString,
    },
    /// Opens the host's type-appropriate editor for a complex shader value
    /// (resource, geometry, gradient, or author-specific text control).
    PaintShaderPropertyEditorRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        definition_id: SharedString,
    },
    /// Detaches the exact current variable alias from one shader property.
    PaintShaderPropertyDetachRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        definition_id: SharedString,
        variable_id: SharedString,
    },
    /// Applies one imported/page Color variable to a solid paint or a specific
    /// stable gradient stop.
    PaintColorVariableApplyRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
        variable_id: SharedString,
    },
    /// Imports one discoverable Color variable. Import and apply are separate
    /// so an available library value never appears bound before a host echo.
    PaintColorVariableImportRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
        variable_id: SharedString,
    },
    /// Detaches the exact current Color-variable identity from one paint leaf
    /// while preserving its host-resolved color.
    PaintColorVariableDetachRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
        variable_id: SharedString,
    },
    /// Opens the host's Color-variable creation flow using the resolved leaf
    /// value; the panel does not choose a collection or manufacture an ID.
    PaintColorVariableCreateRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
        color: DesignColor,
    },
    /// Samples one host-supplied Color-style value into the exact active color
    /// leaf without attaching a whole Fill/Stroke Paint style or Color
    /// variable binding.
    PaintColorStyleSampleRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
        sample: DesignColorStyleSampleSelection,
    },
    /// Compatibility-only leaf color preset action.
    #[deprecated(
        note = "use PaintStyleApplyRequested for whole styles or PaintColorVariableApplyRequested for leaf variables"
    )]
    PaintColorStyleApplyRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
        style: DesignColorStyleSelection,
    },
    /// Compatibility-only leaf color preset creation action.
    #[deprecated(note = "use PaintColorVariableCreateRequested")]
    PaintColorStyleCreateRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
        color: DesignColor,
    },
    /// Opens the host's eyedropper for an editable, unbound solid color or
    /// stable gradient stop. Applying a sampled value remains host-owned.
    PaintEyedropperRequested {
        node_id: SharedString,
        collection: DesignPanelCollection,
        target: DesignPaintTarget,
        paint_id: SharedString,
        index: usize,
        color_target: DesignPaintColorTarget,
    },
    #[deprecated(note = "use DesignPanelAction::ExportAllRequested")]
    ExportRequested {
        node_id: SharedString,
        index: usize,
    },
    ExportConfigurationAddRequested {
        target: DesignPanelTarget,
    },
    ExportConfigurationRemoveRequested {
        target: DesignPanelTarget,
        configuration_id: SharedString,
    },
    ExportConfigurationChangeRequested {
        target: DesignPanelTarget,
        configuration_id: SharedString,
        change: DesignExportConfigurationChange,
        phase: DesignPanelEditPhase,
    },
    ExportModeChangeRequested {
        target: DesignPanelTarget,
        mode: DesignExportMode,
    },
    AnimatedExportChangeRequested {
        target: DesignPanelTarget,
        change: DesignAnimatedExportChange,
        phase: DesignPanelEditPhase,
    },
    AnimatedExportRequested {
        target: DesignPanelTarget,
        settings: DesignAnimatedExportSettings,
    },
    ExportAllRequested {
        target: DesignPanelTarget,
    },
    ExportPreviewRequested {
        target: DesignPanelTarget,
    },
    /// Continuously updates one axis of the exact ordered Smart Selection.
    ///
    /// The panel retains only the draft. The host snapshots spacing at Begin,
    /// previews/commits the candidate atomically for the complete target, and
    /// restores its snapshot at Cancel.
    SmartSelectionSpacingEditRequested {
        target: DesignPanelTarget,
        axis: DesignSmartSelectionAxis,
        value: f32,
        phase: DesignPanelEditPhase,
    },
    /// Runs one host-authorized distribute or Tidy up operation for the exact
    /// ordered Smart Selection snapshot.
    SmartSelectionArrangeRequested {
        target: DesignPanelTarget,
        operation: DesignSmartSelectionOperation,
    },
    ArrangeRequested {
        target: DesignPanelTarget,
        operation: DesignArrangeOperation,
    },
    TransformRequested {
        target: DesignPanelTarget,
        operation: DesignTransformOperation,
    },
    ResizeToFitRequested {
        target: DesignPanelTarget,
    },
    /// Applies one exact host-enumerated Frame preset.
    ///
    /// Stable group/preset identity lets the host reject a stale catalog
    /// activation. Dimensions are the exact immutable values displayed by the
    /// triggering row and let reducers validate that identity against their
    /// latest catalog before resizing the Frame.
    FramePresetApplyRequested {
        node_id: SharedString,
        selection: DesignFramePresetSelection,
        width: f32,
        height: f32,
    },
    /// Atomically exchanges the start and end cap of an open line/path.
    ///
    /// The host owns the stroke and echoes the resulting cap values.
    SwapStrokeEndpointsRequested {
        node_id: SharedString,
    },
    /// Recolors every stable paint occurrence represented by one Selection
    /// colors aggregate row.
    SelectionColorEditRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        color: DesignColor,
        paint_references: Vec<DesignSelectionPaintReference>,
        phase: DesignPanelEditPhase,
    },
    /// Applies one typed normal-paint edit to every exact occurrence behind a
    /// stable Selection-colors aggregate row.
    ///
    /// The ordered selection and occurrence references are authoritative.
    /// `DesignPaintEdit` retains the same typed property/value contract used
    /// by ordinary Fill and Stroke paint editors.
    SelectionColorPaintEditRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        edit: DesignPaintEdit,
        phase: DesignPanelEditPhase,
    },
    /// Asks the host to select every exact paint occurrence represented by one
    /// aggregate row. This changes selection only and remains available to
    /// viewers when every occurrence has a stable supplied reference.
    SelectionColorOccurrencesSelectRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
    },
    /// Applies one imported/page Paint style to every exact Fill/Stroke
    /// collection represented by a stable Selection-colors aggregate row.
    SelectionColorPaintStyleApplyRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        style: DesignPaintStyleSelection,
    },
    /// Imports one discoverable library Paint style without applying it.
    SelectionColorPaintStyleImportRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        style: DesignPaintStyleSelection,
    },
    /// Opens whole Paint-style creation with the row's exact ordered
    /// collection snapshot.
    SelectionColorPaintStyleCreateRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        paints: Vec<DesignPaint>,
    },
    /// Detaches the exact uniform collection-level style binding while
    /// preserving its resolved paints.
    SelectionColorPaintStyleDetachRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        style: DesignPaintStyleSelection,
    },
    /// Applies one imported/page Color variable to every exact selected leaf.
    SelectionColorVariableApplyRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        variable_id: SharedString,
    },
    /// Imports one available library Color variable without binding it.
    SelectionColorVariableImportRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        variable_id: SharedString,
    },
    /// Opens Color-variable creation for the row's current resolved color.
    SelectionColorVariableCreateRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        color: DesignColor,
    },
    /// Detaches the exact uniform leaf-level variable binding.
    SelectionColorVariableDetachRequested {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
        paint_references: Vec<DesignSelectionPaintReference>,
        variable_id: SharedString,
    },
    /// Compatibility intent retained for old node-level media adapters.
    ///
    /// The panel no longer emits this variant. Hosts should handle
    /// [`Self::PaintMediaSourceActionRequested`] instead.
    #[deprecated(note = "use DesignPanelAction::PaintMediaSourceActionRequested")]
    ReplaceMediaRequested {
        node_id: SharedString,
    },
    ResetInstanceOverridesRequested {
        node_id: SharedString,
    },
    GoToMainComponentRequested {
        node_id: SharedString,
    },
    DetachInstanceRequested {
        node_id: SharedString,
    },
}

#[allow(deprecated)]
impl DesignPanelAction {
    /// Wraps a legacy single-node leaf payload in its authoritative ordered
    /// selection target.
    ///
    /// Hosts should treat the target as one atomic operation. The nested
    /// `node_id` is only a compatibility hint and must not be used to discard
    /// the remaining selected IDs.
    pub fn for_selection_target(target: DesignPanelTarget, action: Self) -> Self {
        Self::TargetedNodeActionRequested {
            target,
            action: Box::new(action),
        }
    }

    /// Returns the authoritative ordered target and nested leaf payload for a
    /// targeted ordinary inspector action.
    pub fn targeted_node_action(&self) -> Option<(&DesignPanelTarget, &Self)> {
        match self {
            Self::TargetedNodeActionRequested { target, action } => Some((target, action.as_ref())),
            _ => None,
        }
    }

    /// Returns the exact Fill/Stroke scope captured by a paint interaction.
    ///
    /// Hosts can compare a selected-range target with their current text
    /// selection before resolving the stable paint ID. Import-only actions
    /// retain the same target so an asynchronous chooser cannot be replayed
    /// against a later range.
    pub const fn paint_target(&self) -> Option<(DesignPanelCollection, DesignPaintTarget)> {
        match self {
            Self::CollectionItemAddRequested {
                collection, target, ..
            }
            | Self::CollectionItemRemoveRequested {
                collection, target, ..
            }
            | Self::PaintChangeRequested {
                collection, target, ..
            }
            | Self::PaintEditRequested {
                collection, target, ..
            }
            | Self::PaintReorderRequested {
                collection, target, ..
            }
            | Self::PaintStyleApplyRequested {
                collection, target, ..
            }
            | Self::PaintStyleImportRequested {
                collection, target, ..
            }
            | Self::PaintStyleCreateRequested {
                collection, target, ..
            }
            | Self::PaintStyleDetachRequested {
                collection, target, ..
            }
            | Self::PaintSourceReplaceRequested {
                collection, target, ..
            }
            | Self::PaintMediaSourceActionRequested {
                collection, target, ..
            }
            | Self::PaintMediaSourceDropRequested {
                collection, target, ..
            }
            | Self::PaintMediaCropActionRequested {
                collection, target, ..
            }
            | Self::PaintVideoPreviewActionRequested {
                collection, target, ..
            }
            | Self::PaintShaderImportRequested {
                collection, target, ..
            }
            | Self::PaintShaderApplyRequested {
                collection, target, ..
            }
            | Self::PaintShaderPropertyBindRequested {
                collection, target, ..
            }
            | Self::PaintShaderPropertyEditorRequested {
                collection, target, ..
            }
            | Self::PaintShaderPropertyDetachRequested {
                collection, target, ..
            }
            | Self::PaintColorVariableApplyRequested {
                collection, target, ..
            }
            | Self::PaintColorVariableImportRequested {
                collection, target, ..
            }
            | Self::PaintColorVariableDetachRequested {
                collection, target, ..
            }
            | Self::PaintColorVariableCreateRequested {
                collection, target, ..
            }
            | Self::PaintColorStyleSampleRequested {
                collection, target, ..
            }
            | Self::PaintColorStyleApplyRequested {
                collection, target, ..
            }
            | Self::PaintColorStyleCreateRequested {
                collection, target, ..
            }
            | Self::PaintEyedropperRequested {
                collection, target, ..
            } => Some((*collection, *target)),
            _ => None,
        }
    }

    /// Returns a copy of a legacy leaf payload with its compatibility
    /// `node_id` replaced.
    ///
    /// This is primarily useful to small reference hosts that intentionally
    /// replay one atomic targeted action through an existing single-node
    /// reducer. Production hosts can consume [`Self::targeted_node_action`]
    /// directly and apply one multi-node document operation.
    pub fn retargeted_legacy_node_action(&self, node_id: impl Into<SharedString>) -> Option<Self> {
        let mut action = self.clone();
        *action.legacy_node_id_mut()? = node_id.into();
        Some(action)
    }

    pub(crate) fn legacy_node_id_mut(&mut self) -> Option<&mut SharedString> {
        match self {
            Self::TypographyPropertyChangeRequested { node_id, .. }
            | Self::TypographyPropertyEditRequested { node_id, .. }
            | Self::TypographyVariableAxisEditRequested { node_id, .. }
            | Self::TypographyStyleApplyRequested { node_id, .. }
            | Self::TypographyStyleDetachRequested { node_id, .. }
            | Self::TypographyFontApplyRequested { node_id, .. }
            | Self::TypographyFontImportRequested { node_id, .. }
            | Self::TypographyOpenTypeFeatureChangeRequested { node_id, .. }
            | Self::TextPathFlipOrientationRequested { node_id }
            | Self::TextPathStartChangeRequested { node_id, .. }
            | Self::VectorVertexSelectionEditRequested { node_id, .. }
            | Self::VectorVertexPositionEditRequested { node_id, .. }
            | Self::VectorVertexCornerRadiusEditRequested { node_id, .. }
            | Self::VectorHandleMirroringEditRequested { node_id, .. }
            | Self::PropertyChangeRequested { node_id, .. }
            | Self::PropertyEditRequested { node_id, .. }
            | Self::DimensionLimitsPreviewRequested { node_id, .. }
            | Self::PropertyVariableApplyRequested { node_id, .. }
            | Self::PropertyVariableImportRequested { node_id, .. }
            | Self::PropertyVariableDetachRequested { node_id, .. }
            | Self::SectionShareRequested { node_id }
            | Self::SectionResolveChangedStatusRequested { node_id }
            | Self::TransformModifierAddRequested { node_id, .. }
            | Self::TransformModifierRemoveRequested { node_id, .. }
            | Self::TransformModifierChangeRequested { node_id, .. }
            | Self::ApplyTransformModifiersRequested { node_id }
            | Self::ComponentPropertyDefinitionCreateRequested { node_id, .. }
            | Self::ComponentPropertyDefinitionRenameRequested { node_id, .. }
            | Self::ComponentPropertyDefinitionMetadataEditRequested { node_id, .. }
            | Self::ComponentPropertyDefinitionEditRequested { node_id, .. }
            | Self::ComponentPropertyDefinitionDeleteRequested { node_id, .. }
            | Self::ComponentPropertyDefinitionReorderRequested { node_id, .. }
            | Self::ComponentVariantOptionCreateRequested { node_id, .. }
            | Self::ComponentVariantOptionRenameRequested { node_id, .. }
            | Self::ComponentVariantOptionDeleteRequested { node_id, .. }
            | Self::ComponentVariantOptionReorderRequested { node_id, .. }
            | Self::ComponentPropertyApplyToLayerRequested { node_id, .. }
            | Self::ComponentPropertySwitchOnLayerRequested { node_id, .. }
            | Self::ComponentPropertyDetachFromLayerRequested { node_id, .. }
            | Self::NestedComponentPropertyExposeRequested { node_id, .. }
            | Self::NestedComponentPropertyUnexposeRequested { node_id, .. }
            | Self::NestedComponentPropertyPreviewRequested { node_id, .. }
            | Self::ComponentPropertyChangeRequested { node_id, .. }
            | Self::ComponentPropertyEditRequested { node_id, .. }
            | Self::ComponentPropertyResetRequested { node_id, .. }
            | Self::ComponentPropertyVariableApplyRequested { node_id, .. }
            | Self::ComponentPropertyVariableImportRequested { node_id, .. }
            | Self::ComponentPropertyVariableDetachRequested { node_id, .. }
            | Self::ComponentSwapApplyRequested { node_id, .. }
            | Self::ComponentSwapImportRequested { node_id, .. }
            | Self::ComponentSwapPreviewRequested { node_id, .. }
            | Self::ComponentPropertyNestedInstanceSelectRequested { node_id, .. }
            | Self::ComponentPropertyNestedInstanceGoToMainRequested { node_id, .. }
            | Self::SlotSettingsChangeRequested { node_id, .. }
            | Self::SlotResetRequested { node_id, .. }
            | Self::SlotClearRequested { node_id, .. }
            | Self::SlotAddInstanceRequested { node_id, .. }
            | Self::SlotChildSelectRequested { node_id, .. }
            | Self::SlotLimitLayersSelectRequested { node_id, .. }
            | Self::SlotChildRemoveRequested { node_id, .. }
            | Self::SlotChildReorderRequested { node_id, .. }
            | Self::SlotChildReplaceRequested { node_id, .. }
            | Self::CollectionItemAddRequested { node_id, .. }
            | Self::CollectionItemRemoveRequested { node_id, .. }
            | Self::EffectAddRequested { node_id, .. }
            | Self::EffectRemoveRequested { node_id, .. }
            | Self::EffectReorderRequested { node_id, .. }
            | Self::EffectEditRequested { node_id, .. }
            | Self::EffectShaderChooseRequested { node_id, .. }
            | Self::EffectShaderPropertyEditorRequested { node_id, .. }
            | Self::EffectShaderPropertyVariableDetachRequested { node_id, .. }
            | Self::EffectStyleApplyRequested { node_id, .. }
            | Self::EffectStyleCreateRequested { node_id, .. }
            | Self::EffectStyleDetachRequested { node_id, .. }
            | Self::EffectVariableApplyRequested { node_id, .. }
            | Self::EffectVariableDetachRequested { node_id, .. }
            | Self::GridDimensionsEditRequested { node_id, .. }
            | Self::GridTrackAddRequested { node_id, .. }
            | Self::GridTrackDeleteRequested { node_id, .. }
            | Self::GridTracksReorderRequested { node_id, .. }
            | Self::LayoutGridStyleApplyRequested { node_id, .. }
            | Self::LayoutGridStyleCreateRequested { node_id, .. }
            | Self::LayoutGridStyleDetachRequested { node_id, .. }
            | Self::LayoutGridStyleImportRequested { node_id, .. }
            | Self::LayoutGridPropertyEditRequested { node_id, .. }
            | Self::LayoutGridRemoveRequested { node_id, .. }
            | Self::LayoutGridVariableApplyRequested { node_id, .. }
            | Self::LayoutGridVariableImportRequested { node_id, .. }
            | Self::LayoutGridVariableDetachRequested { node_id, .. }
            | Self::LayoutGridVariableCreateRequested { node_id, .. }
            | Self::LayoutGridCountVariableApplyRequested { node_id, .. }
            | Self::LayoutGridCountVariableDetachRequested { node_id, .. }
            | Self::PaintChangeRequested { node_id, .. }
            | Self::PaintEditRequested { node_id, .. }
            | Self::PaintReorderRequested { node_id, .. }
            | Self::PaintStyleApplyRequested { node_id, .. }
            | Self::PaintStyleImportRequested { node_id, .. }
            | Self::PaintStyleCreateRequested { node_id, .. }
            | Self::PaintStyleDetachRequested { node_id, .. }
            | Self::PaintSourceReplaceRequested { node_id, .. }
            | Self::PaintMediaSourceActionRequested { node_id, .. }
            | Self::PaintMediaSourceDropRequested { node_id, .. }
            | Self::PaintMediaCropActionRequested { node_id, .. }
            | Self::PaintVideoPreviewActionRequested { node_id, .. }
            | Self::PaintShaderImportRequested { node_id, .. }
            | Self::PaintShaderApplyRequested { node_id, .. }
            | Self::PaintShaderPropertyBindRequested { node_id, .. }
            | Self::PaintShaderPropertyEditorRequested { node_id, .. }
            | Self::PaintShaderPropertyDetachRequested { node_id, .. }
            | Self::PaintColorVariableApplyRequested { node_id, .. }
            | Self::PaintColorVariableImportRequested { node_id, .. }
            | Self::PaintColorVariableDetachRequested { node_id, .. }
            | Self::PaintColorVariableCreateRequested { node_id, .. }
            | Self::PaintColorStyleSampleRequested { node_id, .. }
            | Self::PaintColorStyleApplyRequested { node_id, .. }
            | Self::PaintColorStyleCreateRequested { node_id, .. }
            | Self::PaintEyedropperRequested { node_id, .. }
            | Self::ExportRequested { node_id, .. }
            | Self::FramePresetApplyRequested { node_id, .. }
            | Self::SwapStrokeEndpointsRequested { node_id }
            | Self::ReplaceMediaRequested { node_id }
            | Self::ResetInstanceOverridesRequested { node_id }
            | Self::GoToMainComponentRequested { node_id }
            | Self::DetachInstanceRequested { node_id } => Some(node_id),
            Self::PageBackgroundChangeRequested { .. }
            | Self::PageBackgroundEditRequested { .. }
            | Self::LocalResourceBrowseRequested { .. }
            | Self::LocalResourceOpenRequested { .. }
            | Self::LocalResourceCreateRequested { .. }
            | Self::LocalResourceImportRequested { .. }
            | Self::LocalStyleCommandRequested { .. }
            | Self::LocalStyleCreateRequested { .. }
            | Self::LocalStyleFolderCreateRequested { .. }
            | Self::LocalStylesDeleteRequested { .. }
            | Self::LocalStylesMoveRequested { .. }
            | Self::VariablesViewOpenRequested { .. }
            | Self::VariableModeApplyRequested { .. }
            | Self::VariableModeClearRequested { .. }
            | Self::SurfaceChangeRequested { .. }
            | Self::SelectionHeaderCommandRequested { .. }
            | Self::AddAutoLayoutRequested { .. }
            | Self::TargetedNodeActionRequested { .. }
            | Self::MenuPreviewRequested { .. }
            | Self::PropertyCopyRequested { .. }
            | Self::ViewerSectionCopyRequested { .. }
            | Self::ViewerSectionRepresentationChangeRequested { .. }
            | Self::ExportConfigurationAddRequested { .. }
            | Self::ExportConfigurationRemoveRequested { .. }
            | Self::ExportConfigurationChangeRequested { .. }
            | Self::ExportModeChangeRequested { .. }
            | Self::AnimatedExportChangeRequested { .. }
            | Self::AnimatedExportRequested { .. }
            | Self::ExportAllRequested { .. }
            | Self::ExportPreviewRequested { .. }
            | Self::SmartSelectionSpacingEditRequested { .. }
            | Self::SmartSelectionArrangeRequested { .. }
            | Self::ArrangeRequested { .. }
            | Self::TransformRequested { .. }
            | Self::ResizeToFitRequested { .. }
            | Self::SelectionColorEditRequested { .. }
            | Self::SelectionColorPaintEditRequested { .. }
            | Self::SelectionColorOccurrencesSelectRequested { .. }
            | Self::SelectionColorPaintStyleApplyRequested { .. }
            | Self::SelectionColorPaintStyleImportRequested { .. }
            | Self::SelectionColorPaintStyleCreateRequested { .. }
            | Self::SelectionColorPaintStyleDetachRequested { .. }
            | Self::SelectionColorVariableApplyRequested { .. }
            | Self::SelectionColorVariableImportRequested { .. }
            | Self::SelectionColorVariableCreateRequested { .. }
            | Self::SelectionColorVariableDetachRequested { .. } => None,
        }
    }

    /// Returns the compatibility classification for actions the current panel
    /// never emits as canonical behavior.
    pub const fn compatibility_path(&self) -> Option<DesignPanelCompatibilityPath> {
        match self {
            Self::PaintChangeRequested { .. } => {
                Some(DesignPanelCompatibilityPath::WholePaintAction)
            }
            Self::PaintColorStyleApplyRequested { .. }
            | Self::PaintColorStyleCreateRequested { .. } => {
                Some(DesignPanelCompatibilityPath::ConflatedColorStyleAction)
            }
            Self::TargetedNodeActionRequested { .. } => None,
            Self::ReplaceMediaRequested { .. } => {
                Some(DesignPanelCompatibilityPath::NodeMediaReplaceAction)
            }
            Self::ExportRequested { .. } => Some(DesignPanelCompatibilityPath::SingleExportAction),
            Self::PropertyChangeRequested { property, .. }
            | Self::PropertyEditRequested { property, .. }
                if property.layout_grid_index().is_some() =>
            {
                Some(DesignPanelCompatibilityPath::IndexedLayoutGridPropertyAction)
            }
            Self::CollectionItemRemoveRequested {
                collection: DesignPanelCollection::LayoutGrid,
                ..
            } => Some(DesignPanelCompatibilityPath::IndexedLayoutGridRemoveAction),
            Self::LayoutGridCountVariableApplyRequested { .. }
            | Self::LayoutGridCountVariableDetachRequested { .. } => {
                Some(DesignPanelCompatibilityPath::CountOnlyLayoutGridVariableAction)
            }
            Self::PropertyChangeRequested { property, .. }
            | Self::PropertyEditRequested { property, .. }
            | Self::EffectEditRequested { property, .. } => property.compatibility_path(),
            _ => None,
        }
    }
}
