//! Artifact kinds and the import capability matrix (design N / session arch).
//!
//! Kinds describe *what* a design file is allowed to compose. Enforcement
//! happens at materialize / `apply` time in `fanta-format::session`; this
//! module only defines the static matrix.

/// Kind of a design artifact in a Fanta project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Project shell: variables, modes, dependency graph, config.
    Workspace,
    /// Design page — main canvas tab.
    Page,
    /// Component master — isolated editor root.
    Component,
    /// Graphics artboard — vectors/assets only; no live instances.
    Graphics,
    /// Prototype flow (layout stub in v1).
    Prototype,
    /// Motion composition (layout stub in v1).
    Motion,
    /// Audio workspace (layout stub in v1).
    Audio,
}

impl ArtifactKind {
    /// Human-readable label for diagnostics / UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Page => "page",
            Self::Component => "component",
            Self::Graphics => "graphics",
            Self::Prototype => "prototype",
            Self::Motion => "motion",
            Self::Audio => "audio",
        }
    }

    /// Whether this kind is fully implemented for session load/edit (v1).
    pub fn is_fully_specified(self) -> bool {
        matches!(
            self,
            Self::Workspace | Self::Page | Self::Component | Self::Graphics
        )
    }

    /// Whether this kind has a canvas scene root.
    pub fn has_canvas(self) -> bool {
        matches!(
            self,
            Self::Page | Self::Component | Self::Graphics | Self::Prototype | Self::Motion
        )
    }
}

/// What an artifact may import / compose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportTarget {
    /// Live component instance (`NodeData::Instance`).
    LiveComponent,
    /// Nested page tree (forbidden in v1).
    NestedPage,
    /// Binary / media assets by `AssetId`.
    Assets,
    /// Variable bindings (tokens).
    Variables,
    /// Bake / recreate a component into static nodes (Graphics only).
    ComponentRecreation,
    /// Nested prototype (forbidden v1).
    NestedPrototype,
    /// Nested motion (forbidden v1).
    NestedMotion,
}

/// Whether `from` may import `target`.
pub fn import_allowed(from: ArtifactKind, target: ImportTarget) -> bool {
    use ArtifactKind::*;
    use ImportTarget::*;
    match (from, target) {
        (Workspace, _) => false, // workspace indexes; does not compose scene imports
        (Page, LiveComponent | Assets | Variables) => true,
        (Page, NestedPage | ComponentRecreation | NestedPrototype | NestedMotion) => false,
        (Component, LiveComponent | Assets | Variables) => true,
        (Component, NestedPage | ComponentRecreation | NestedPrototype | NestedMotion) => false,
        (Graphics, Assets | Variables | ComponentRecreation) => true,
        (Graphics, LiveComponent | NestedPage | NestedPrototype | NestedMotion) => false,
        (Prototype, LiveComponent | Assets | Variables) => true,
        (Prototype, NestedPage | ComponentRecreation | NestedPrototype | NestedMotion) => false,
        (Motion, LiveComponent | Assets | Variables) => true,
        (Motion, NestedPage | ComponentRecreation | NestedPrototype | NestedMotion) => false,
        (Audio, Assets) => true,
        (Audio, _) => false,
    }
}

/// Relative file names that form one artifact's hashed / written unit, in
/// hash order (design N9 / ArtifactFileSet table).
pub fn artifact_file_names(kind: ArtifactKind) -> &'static [&'static str] {
    match kind {
        ArtifactKind::Page => &["page.fnx", "page.ids.json", "page.json"],
        ArtifactKind::Component => &["master.fnx", "master.ids.json", "def.json"],
        ArtifactKind::Graphics => &["graphics.fnx", "graphics.ids.json", "graphics.json"],
        ArtifactKind::Prototype => &["prototype.fnx", "prototype.ids.json", "prototype.json"],
        ArtifactKind::Motion => &["motion.fnx", "motion.ids.json", "motion.json"],
        ArtifactKind::Audio => &["audio.fnx", "audio.ids.json", "audio.json"],
        ArtifactKind::Workspace => &[], // workspace shared uses a different path list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_may_instance_component() {
        assert!(import_allowed(
            ArtifactKind::Page,
            ImportTarget::LiveComponent
        ));
    }

    #[test]
    fn graphics_forbids_live_instance() {
        assert!(!import_allowed(
            ArtifactKind::Graphics,
            ImportTarget::LiveComponent
        ));
        assert!(import_allowed(
            ArtifactKind::Graphics,
            ImportTarget::ComponentRecreation
        ));
    }

    #[test]
    fn audio_only_assets() {
        assert!(import_allowed(ArtifactKind::Audio, ImportTarget::Assets));
        assert!(!import_allowed(
            ArtifactKind::Audio,
            ImportTarget::LiveComponent
        ));
    }

    #[test]
    fn component_cannot_import_page() {
        assert!(!import_allowed(
            ArtifactKind::Component,
            ImportTarget::NestedPage
        ));
    }
}
