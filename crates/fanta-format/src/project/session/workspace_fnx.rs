//! Optional `workspace.fnx` + dependency graph index.

use super::error::SessionError;
use super::types::ArtifactId;
use crate::project::layout::WORKSPACE_FNX;
use std::collections::BTreeMap;
use std::path::Path;

/// Lightweight dependency edges between artifacts (DI graph).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyGraph {
    /// `importer` → list of imported artifact labels (paths or ids).
    pub edges: BTreeMap<String, Vec<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.edges.entry(from.into()).or_default().push(to.into());
    }

    pub fn imports_of(&self, from: &str) -> &[String] {
        self.edges.get(from).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Workspace IR: optional source text + parsed dependency hints.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceIr {
    /// Source when `workspace.fnx` exists; empty string means synthesized.
    pub source: String,
    pub graph: DependencyGraph,
    pub present_on_disk: bool,
}

/// Load or synthesize workspace.fnx.
pub fn load_workspace_ir(project_root: &Path) -> Result<WorkspaceIr, SessionError> {
    let path = project_root.join(WORKSPACE_FNX);
    if path.is_file() {
        let source = std::fs::read_to_string(&path)?;
        let graph = parse_dependency_hints(&source);
        return Ok(WorkspaceIr {
            source,
            graph,
            present_on_disk: true,
        });
    }
    Ok(WorkspaceIr {
        source: synthesize_workspace_fnx(&[]),
        graph: DependencyGraph::new(),
        present_on_disk: false,
    })
}

/// Minimal cosmetic workspace.fnx (not a full canvas).
pub fn synthesize_workspace_fnx(entries: &[(ArtifactId, &str)]) -> String {
    let mut out = String::from(
        "// @generated fanta workspace — dependency injector / variables shell\n\
export default function Workspace() {\n\
  return (\n\
    <Workspace name=\"project\">\n",
    );
    for (id, path) in entries {
        out.push_str(&format!(
            "      <Entry kind=\"{}\" path={path:?} id={:?} />\n",
            id.kind().label(),
            id.debug_label()
        ));
    }
    out.push_str("    </Workspace>\n  );\n}\n");
    out
}

/// Extremely small hint parser: lines containing `path="..."` become edges from workspace.
fn parse_dependency_hints(source: &str) -> DependencyGraph {
    let mut graph = DependencyGraph::new();
    for line in source.lines() {
        if let Some(start) = line.find("path=\"") {
            let rest = &line[start + 6..];
            if let Some(end) = rest.find('"') {
                let path = &rest[..end];
                graph.add_edge("workspace", path);
            }
        }
    }
    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesize_contains_entries() {
        let text = synthesize_workspace_fnx(&[]);
        assert!(text.contains("Workspace"));
        assert!(text.contains("export default"));
    }

    #[test]
    fn parse_paths_into_graph() {
        let src = r#"
    <Entry path="pages/home/page.fnx" />
    <Entry path="components/button/master.fnx" />
"#;
        let g = parse_dependency_hints(src);
        assert_eq!(g.imports_of("workspace").len(), 2);
    }
}
