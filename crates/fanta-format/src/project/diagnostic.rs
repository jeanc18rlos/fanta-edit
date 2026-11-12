//! Structured findings from reading a project: what a load or an authoring
//! pass noticed, in a form a tool can act on (spec 0009 R2). Not gated on
//! `native`: the wasm engine reads projects too.

/// The diagnostic code every unknown-attribute warning carries.
pub(crate) const UNKNOWN_ATTRIBUTE: &str = "source.unknown_attribute";
/// A hand edit the web loader read but could not apply to the node.
pub(crate) const EDIT_NOT_KEPT: &str = "edit.not_kept";
/// A CSS selector outside the emitted one-id-per-rule dialect.
pub(crate) const EDIT_SELECTOR_OUTSIDE_DIALECT: &str = "edit.selector_outside_dialect";
/// An `id` in the SVG or CSS that is not a node id.
pub(crate) const ID_NOT_A_NODE: &str = "id.not_a_node";
/// A design SVG that stopped parsing part-way.
pub(crate) const PARSE_SVG: &str = "parse.svg";
/// An element or rule whose node has no record in `nodes.json`.
pub(crate) const RECORD_MISSING: &str = "record.missing";
/// A `nodes.json` record that does not decode as a node.
pub(crate) const RECORD_UNREADABLE: &str = "record.unreadable";
/// Two variables that produce the same CSS token.
pub(crate) const REF_AMBIGUOUS_TOKEN: &str = "ref.ambiguous_token";
/// A page or component left out of the loaded document because one of its
/// files does not parse. `file` is its directory, which a save leaves alone.
pub const DESIGN_UNAVAILABLE: &str = "design.unavailable";
/// The same `id` on two elements of one design SVG.
pub(crate) const ID_DUPLICATE: &str = "id.duplicate";
/// The project does not load, for a reason no other finding explains.
pub(crate) const LOAD_FAILED: &str = "load.failed";
/// A file every design needs, or the writer would emit, is not on disk.
pub(crate) const MANIFEST_FILE_MISSING: &str = "manifest.file_missing";
/// A node the writer draws has no element in its design's SVG.
pub(crate) const RECORD_NO_ELEMENT: &str = "record.no_element";
/// A node paints an image asset the project does not hold.
pub(crate) const REF_MISSING_ASSET: &str = "ref.missing_asset";
/// An instance of a component the project does not define.
pub(crate) const REF_MISSING_COMPONENT: &str = "ref.missing_component";
/// A `var()` naming a token no variable produces.
pub(crate) const REF_MISSING_TOKEN: &str = "ref.missing_token";
/// Unresolved merge-conflict markers in a project file.
pub(crate) const PARSE_CONFLICT_MARKER: &str = "parse.conflict_marker";
/// A stylesheet with unbalanced braces.
pub(crate) const PARSE_CSS: &str = "parse.css";
/// A project JSON file that is not valid JSON.
pub(crate) const PARSE_JSON: &str = "parse.json";
/// Auto-layout geometry omitted from the source could not be solved on load.
pub const LOAD_OMITTED_GEOMETRY: &str = "load.omitted_geometry";

/// Every diagnostic code this crate and the engine hosts emit. Codes are
/// permanent: add to this list, never repurpose an entry (spec 0009 R4).
pub const DIAGNOSTIC_CODES: &[&str] = &[
    DESIGN_UNAVAILABLE,
    EDIT_NOT_KEPT,
    EDIT_SELECTOR_OUTSIDE_DIALECT,
    ID_DUPLICATE,
    ID_NOT_A_NODE,
    LOAD_FAILED,
    LOAD_OMITTED_GEOMETRY,
    MANIFEST_FILE_MISSING,
    PARSE_CONFLICT_MARKER,
    PARSE_CSS,
    PARSE_JSON,
    PARSE_SVG,
    RECORD_MISSING,
    RECORD_NO_ELEMENT,
    RECORD_UNREADABLE,
    REF_AMBIGUOUS_TOKEN,
    REF_MISSING_ASSET,
    REF_MISSING_COMPONENT,
    REF_MISSING_TOKEN,
    UNKNOWN_ATTRIBUTE,
];


/// How bad a finding is. The loader itself only ever warns, because anything
/// fatal fails its parse; `Error` comes from the whole-project check, which
/// reports what would stop a load or lose data instead of stopping at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSeverity {
    Error,
    Warning,
    Info,
}

/// One non-fatal diagnostic from ingesting hand-written `.fnx` source (e.g. a
/// typo'd attribute that serde would otherwise swallow silently). These never
/// block a commit — unknown FUTURE fields must keep riding through — the
/// caller decides whether a policy (such as the harness `--deny`) promotes
/// them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct SourceDiagnostic {
    pub severity: SourceSeverity,
    /// Stable machine code, e.g. `source.unknown_attribute`. Every code is
    /// listed in [`DIAGNOSTIC_CODES`] and documented in
    /// `docs/fanta/project-schema.md`; a code is never reused.
    pub code: &'static str,
    /// Project-relative path with forward slashes, where the finding has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// 1-based line and column in `file`, where the reader knows them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    pub message: String,
}

impl SourceDiagnostic {
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: SourceSeverity::Warning,
            code,
            file: None,
            line: None,
            column: None,
            node_id: None,
            node_name: None,
            message: message.into(),
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: SourceSeverity::Error,
            ..Self::warning(code, message)
        }
    }

    pub fn at(mut self, line: usize, column: usize) -> Self {
        self.line = u32::try_from(line).ok();
        self.column = u32::try_from(column).ok();
        self
    }

    pub fn for_node(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    pub fn in_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Set the line and column of byte `offset` in `text`, the file's content.
    pub fn at_offset(mut self, text: &str, offset: usize) -> Self {
        let before = text.get(..offset.min(text.len())).unwrap_or(text);
        let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
        self.line = u32::try_from(before.matches('\n').count() + 1).ok();
        self.column = u32::try_from(before[line_start..].chars().count() + 1).ok();
        self
    }
}

/// The message alone: what the free-text warnings used to say, for callers
/// that print or forward one line per finding.
impl std::fmt::Display for SourceDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_offset_counts_lines_and_columns_from_one() {
        let text = "ab\ncdé f\n";
        let at = |offset| {
            let finding = SourceDiagnostic::warning(PARSE_SVG, "x").at_offset(text, offset);
            (finding.line, finding.column)
        };
        assert_eq!(at(0), (Some(1), Some(1)));
        assert_eq!(at(3), (Some(2), Some(1)));
        assert_eq!(at(text.find('f').unwrap()), (Some(2), Some(5)));
        assert_eq!(at(10_000), (Some(3), Some(1)), "past the end clamps");
    }

    /// Spec 0009 A9: the documented code list and the codes in the source match.
    #[test]
    fn every_diagnostic_code_is_documented_and_no_other() {
        let doc = include_str!("../../../../docs/fanta/project-schema.md");
        let section = doc
            .split("## Diagnostic codes")
            .nth(1)
            .expect("project-schema.md has a Diagnostic codes section");
        let section = section.split("\n## ").next().unwrap_or(section);
        let documented: std::collections::BTreeSet<&str> = section
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|rest| rest.split('`').next())
            .collect();
        let emitted: std::collections::BTreeSet<&str> = DIAGNOSTIC_CODES.iter().copied().collect();
        assert_eq!(emitted.len(), DIAGNOSTIC_CODES.len(), "a code is listed twice");
        assert_eq!(documented, emitted);
    }
}
