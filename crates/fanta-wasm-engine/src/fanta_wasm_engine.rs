//! The design engine for browsers and agent hosts (spec 0008 T1, R1/R2/R3):
//! the same document model, design-op vocabulary and file emitter the desktop
//! app uses, compiled to `wasm32-unknown-unknown` with no filesystem — files
//! come in as bytes, edits are `batch_design` op JSON, and a save hands back
//! the exact bytes the desktop writer would put on disk (minus the stateful
//! editor-support seeds; the host must not delete files the emitted map does
//! not name).
//!
//! Build: `RUSTFLAGS='--cfg getrandom_backend="wasm_js"' cargo build -p
//! fanta-wasm-engine --target wasm32-unknown-unknown --release`, then
//! `wasm-bindgen` (the CLI must match the workspace's pinned version).
//!
//! [`EngineCore`] is the plain-Rust engine, tested natively;
//! [`FantaEngine`] is its `wasm_bindgen` skin. Renders are compiled out
//! (`fanta-headless` without its `render` feature), so SVG parts the pure
//! projection cannot compute draw nothing and the report says so.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use fanta_design_ops::DesignOp;
use fanta_headless::document::HeadlessDocument;
use fanta_headless::layout;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

pub struct EngineCore {
    staged: BTreeMap<PathBuf, Vec<u8>>,
    document: HeadlessDocument,
    warnings: Vec<String>,
    emitted: BTreeMap<PathBuf, Arc<Vec<u8>>>,
}

impl EngineCore {
    /// An engine over a new design with one empty page.
    pub fn new() -> Result<Self> {
        Ok(Self {
            staged: BTreeMap::new(),
            document: HeadlessDocument::empty()?,
            warnings: Vec::new(),
            emitted: BTreeMap::new(),
        })
    }

    /// Stage one project file for [`load`](Self::load). Paths are the
    /// project-relative form the emitter uses (`pages/home/page.svg`).
    pub fn add_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<()> {
        let path = Path::new(path);
        if path.as_os_str().is_empty()
            || !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            bail!(
                "{} is not a plain relative project path",
                path.display()
            );
        }
        self.staged.insert(path.to_path_buf(), bytes);
        Ok(())
    }

    /// Assemble the staged files into an open document, the way the editor
    /// opens a project directory. Returns [`inspect`](Self::inspect).
    pub fn load(&mut self) -> Result<Value> {
        let (mut doc, assets, mut warnings) =
            fanta_format::read_project_files(&self.staged).context("reading the project files")?;
        fanta_doc::strip_redundant_instance_overrides(&mut doc.scene, &doc.components);
        if let Err(error) = layout::solve_pending(&mut doc) {
            warnings.push(format!("computing omitted source geometry: {error:#}"));
        }
        self.document = HeadlessDocument::new(doc, assets);
        self.warnings = warnings;
        self.emitted.clear();
        Ok(self.inspect())
    }

    /// Apply a `batch_design` ops list (the JSON array every Fanta agent
    /// surface takes) as one undo step. `commit` false checks and rolls back.
    /// Returns the batch result: `applied`, per-op status, created ids.
    pub fn apply(&mut self, ops_json: &str, label: &str, commit: bool) -> Result<Value> {
        let ops: Vec<DesignOp> =
            serde_json::from_str(ops_json).context("parsing the design ops JSON")?;
        let active_page = self.document.doc().active_page();
        self.document
            .apply_design_ops(active_page, label, &ops, commit)
    }

    /// Undo the last step. Returns whether a step was undone.
    pub fn undo(&mut self) -> Result<bool> {
        self.document.undo()
    }

    /// Redo the last undone step. Returns whether a step was redone.
    pub fn redo(&mut self) -> Result<bool> {
        self.document.redo()
    }

    /// Project the document into the files a save writes — the managed
    /// files only, byte-identical to a desktop save (spec 0008 R3). The
    /// paths come back as JSON; read each file with
    /// [`take_file`](Self::take_file).
    pub fn emit(&mut self) -> Result<Value> {
        let (files, report) = fanta_format::emit_web_project_tree(
            self.document.doc(),
            self.document.raw_assets(),
            None,
            &self.warnings,
        )
        .context("projecting the project files")?;
        let paths: Vec<String> = files
            .keys()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        self.emitted = files;
        Ok(json!({
            "files": paths,
            "report": serde_json::to_value(&report)?,
        }))
    }

    /// The bytes of one file from the last [`emit`](Self::emit).
    pub fn take_file(&self, path: &str) -> Option<Vec<u8>> {
        self.emitted.get(Path::new(path)).map(|bytes| bytes.to_vec())
    }

    /// The open document's editor state: title, pages, active page, history
    /// depth and the load warnings.
    pub fn inspect(&self) -> Value {
        let doc = self.document.doc();
        let pages: Vec<Value> = doc
            .pages()
            .iter()
            .map(|root| {
                json!({
                    "id": root.to_string(),
                    "name": doc.scene.get(*root).map(|node| node.name.clone()),
                })
            })
            .collect();
        json!({
            "title": doc.metadata.title,
            "pages": pages,
            "active_page": doc.active_page().map(|root| root.to_string()),
            "undo_depth": doc.history.undo_depth(),
            "warnings": self.warnings,
        })
    }
}

/// [`EngineCore`] for JavaScript: strings carry JSON, byte slices carry file
/// contents, and every error surfaces as a thrown `Error`.
#[wasm_bindgen]
pub struct FantaEngine {
    core: EngineCore,
}

#[wasm_bindgen]
impl FantaEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<FantaEngine, JsError> {
        Ok(FantaEngine {
            core: EngineCore::new().map_err(to_js_error)?,
        })
    }

    pub fn add_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), JsError> {
        self.core.add_file(path, bytes.to_vec()).map_err(to_js_error)
    }

    pub fn load(&mut self) -> Result<String, JsError> {
        Ok(self.core.load().map_err(to_js_error)?.to_string())
    }

    pub fn apply(&mut self, ops_json: &str, label: &str, commit: bool) -> Result<String, JsError> {
        Ok(self
            .core
            .apply(ops_json, label, commit)
            .map_err(to_js_error)?
            .to_string())
    }

    pub fn undo(&mut self) -> Result<bool, JsError> {
        self.core.undo().map_err(to_js_error)
    }

    pub fn redo(&mut self) -> Result<bool, JsError> {
        self.core.redo().map_err(to_js_error)
    }

    pub fn emit(&mut self) -> Result<String, JsError> {
        Ok(self.core.emit().map_err(to_js_error)?.to_string())
    }

    pub fn take_file(&self, path: &str) -> Option<Vec<u8>> {
        self.core.take_file(path)
    }

    pub fn inspect(&self) -> String {
        self.core.inspect().to_string()
    }
}

fn to_js_error(error: anyhow::Error) -> JsError {
    JsError::new(&format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R1/R2/R3 end to end, natively: create → emit → load the emitted bytes
    /// in a second engine → the design round-trips, ops apply with the
    /// `batch_design` vocabulary, and undo steps back.
    #[test]
    fn edits_round_trip_through_emitted_files() {
        let mut first = EngineCore::new().expect("new engine");
        let result = first
            .apply(
                r##"[{"op":"create_node","node_type":"rectangle","name":"Panel",
                     "x":10.0,"y":20.0,"width":120.0,"height":80.0,"fill":"#123456"}]"##,
                "create panel",
                true,
            )
            .expect("apply create");
        assert_eq!(result["applied"], Value::Bool(true), "{result}");
        let emitted = first.emit().expect("emit");
        let paths = emitted["files"].as_array().expect("paths");
        assert!(!paths.is_empty());

        let mut second = EngineCore::new().expect("second engine");
        for path in paths {
            let path = path.as_str().expect("path string");
            let bytes = first.take_file(path).expect("emitted bytes");
            second.add_file(path, bytes).expect("stage file");
        }
        let state = second.load().expect("load emitted files");
        assert_eq!(state["warnings"], json!([]), "{state}");
        assert_eq!(state["pages"].as_array().map(Vec::len), Some(1));
        let page_svg = state["pages"][0]["name"].as_str().map(str::to_owned);
        assert_eq!(page_svg.as_deref(), Some("Page 1"));
        let reloaded = second
            .emit()
            .expect("emit after load")
            .to_string();
        assert!(reloaded.contains("page.svg"));

        assert!(second.apply(
            r#"[{"op":"create_node","node_type":"text","x":0.0,"y":0.0,
                 "width":100.0,"height":20.0,"text":"Hello"}]"#,
            "add label",
            true,
        )
        .expect("apply text")["applied"]
            .as_bool()
            .unwrap_or(false));
        assert_eq!(second.inspect()["undo_depth"], json!(1));
        assert!(second.undo().expect("undo"));
        assert_eq!(second.inspect()["undo_depth"], json!(0));
        assert!(!second.undo().expect("undo at bottom"), "nothing left");
        assert!(second.redo().expect("redo"));

        // Paths from outside the project tree are refused at the boundary.
        assert!(second.add_file("../escape.svg", Vec::new()).is_err());
        assert!(second.add_file("/absolute.svg", Vec::new()).is_err());
    }
}
