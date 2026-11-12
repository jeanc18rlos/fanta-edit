//! `fanta-site --plain`: export a project as plain web files (spec 0007).
//! The projection and writer live in `fanta-format`
//! (`write_web_project_tree`); this caller supplies what the pure writer
//! cannot compute itself — renderer-accurate design bounds and PNG fallbacks
//! for nodes outside the SVG subset.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use fanta_format::{
    WebRaster, WebReport, WebWriteInputs, WebWriteOutcome, write_web_project_tree,
};
use fanta_headless::HeadlessProject;
use fanta_headless::export::{ExportEncoding, ExportSizing, export_node, page_visual_bounds};

/// Project `project` into `output` (default `<project>/web`). The output must
/// be missing, empty, or a previous web projection; anything else is refused
/// rather than overwritten.
pub fn export_web(project: &Path, output: Option<&Path>) -> Result<WebReport> {
    let opened = HeadlessProject::open(project)
        .with_context(|| format!("opening Fanta project at {}", project.display()))?;
    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| opened.root().join("web"));
    check_output(&output)?;
    fs::create_dir_all(&output).with_context(|| format!("creating {}", output.display()))?;

    let doc = opened.doc();
    let resolver = opened.document().asset_resolver();
    let mut inputs = WebWriteInputs {
        warnings: opened.warnings().to_vec(),
        ..Default::default()
    };
    let design_roots = doc
        .pages()
        .to_vec()
        .into_iter()
        .chain(doc.components.defs.values().map(|def| def.root));
    for root in design_roots {
        if let Some(bounds) = page_visual_bounds(doc, Some(root)) {
            inputs.bounds.insert(root, bounds);
        }
    }

    loop {
        match write_web_project_tree(&output, doc, opened.document().raw_assets(), &inputs)? {
            WebWriteOutcome::Written { report, .. } => return Ok(report),
            WebWriteOutcome::NeedsRasters(nodes) => {
                for node in nodes {
                    if inputs.rasters.contains_key(&node) {
                        bail!("the writer rejected the raster rendered for {node}");
                    }
                    let image = export_node(
                        doc,
                        resolver.clone(),
                        node,
                        ExportEncoding::Png,
                        ExportSizing::Scale(2.0),
                    )
                    .with_context(|| format!("rendering the PNG fallback for {node}"))?;
                    inputs.rasters.insert(
                        node,
                        WebRaster {
                            bytes: image.bytes,
                            bounds: image.bounds,
                        },
                    );
                }
            }
        }
    }
}

/// The output may be new, empty, or a previous projection, recognized by its
/// report file — never an unrelated folder.
fn check_output(output: &Path) -> Result<()> {
    match fs::read_dir(output) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("reading {}", output.display())),
        Ok(mut entries) => {
            if entries.next().is_some() && !output.join("report.json").is_file() {
                bail!(
                    "{} is not empty and not a previous web projection; refusing to overwrite it",
                    output.display()
                );
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use fanta_doc::color::Color;
    use fanta_doc::value::VarValue;
    use fanta_doc::variables::{Mode, Variable, VariableCollection};
    use fanta_doc::{
        CanvasNode, Doc, GroupNode, ModeId, NodeData, NodeId, Operation, Transform2D, VectorNode,
    };

    fn write_fixture_project() -> (tempfile::TempDir, NodeId) {
        let mut doc = Doc::new();
        let mut roots = Vec::new();
        for name in ["Home", "About"] {
            let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
            page.name = name.into();
            let id = page.id;
            doc.apply(Operation::create_node(page)).expect("page");
            doc.add_page(id);
            roots.push(id);
        }
        let collection_id = fanta_doc::VariableCollectionId::new();
        let variable_id = fanta_doc::VariableId::new();
        let mut card_id = None;
        for (page, name, color, x) in [
            (roots[0], "Card", Color::WHITE, 40.0),
            (roots[1], "Panel", Color::BLACK, 0.0),
        ] {
            let mut layer = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                0.0, 0.0, 120.0, 80.0, color,
            )));
            layer.name = name.into();
            layer.parent = Some(page);
            layer.transform = Transform2D::translation(x, 30.0);
            if name == "Card" {
                card_id = Some(layer.id);
                // Bound to the Surface variable inserted below; ids are the
                // defaults, so the forward reference resolves.
                layer
                    .bindings
                    .insert(fanta_doc::BoundProp::FillColor { index: 0 }, variable_id);
            }
            doc.apply(Operation::create_node(layer)).expect("layer");
        }
        let card_id = card_id.expect("card id");

        let (light, dark) = (ModeId::new(), ModeId::new());
        doc.variables.collections.insert(
            collection_id,
            VariableCollection {
                id: collection_id,
                name: "Theme".into(),
                modes: vec![
                    Mode {
                        id: light,
                        name: "Light".into(),
                    },
                    Mode {
                        id: dark,
                        name: "Dark".into(),
                    },
                ],
                default_mode: light,
                variable_order: Vec::new(),
            },
        );
        doc.variables.variables.insert(
            variable_id,
            Variable {
                id: variable_id,
                collection: collection_id,
                name: "Surface".into(),
                ty: fanta_doc::value::VariableType::Color,
                values_by_mode: [
                    (
                        light,
                        VarValue::Color {
                            value: Color::WHITE,
                        },
                    ),
                    (
                        dark,
                        VarValue::Color {
                            value: Color::BLACK,
                        },
                    ),
                ]
                .into(),
                scopes: Vec::new(),
                options: Vec::new(),
            },
        );

        let directory = tempfile::tempdir().expect("project directory");
        fanta_format::write_project_tree(directory.path(), &doc, &BTreeMap::new())
            .expect("write the project");
        (directory, card_id)
    }

    /// A1 and A3 (T1 scope): the full web-format file set appears (R1), with
    /// no FNX, no FSS and no wasm, and a second export over the first is
    /// byte-identical (R5).
    #[test]
    fn exports_the_file_set_deterministically() {
        let (project, _) = write_fixture_project();
        let report = export_web(project.path(), None).expect("export");
        assert_eq!(report.pages.len(), 2);
        assert!(report.pages.iter().all(|page| page.error.is_none()));

        let output = project.path().join("web");
        for file in [
            "fanta.json",
            "index.html",
            "viewer.js",
            "tokens.css",
            "design.css",
            "pages.js",
            "report.json",
            "pages/home/page.svg",
            "pages/home/map.json",
            "pages/home/nodes.json",
            "pages/about/page.svg",
            "pages/about/map.json",
            "pages/about/nodes.json",
        ] {
            assert!(output.join(file).is_file(), "missing {file}");
        }
        let manifest = fs::read_to_string(output.join("fanta.json")).expect("manifest");
        assert!(
            manifest.contains("\"storage\": \"web\""),
            "the manifest must mark web storage:\n{manifest}"
        );
        let home = fs::read_to_string(output.join("pages/home/page.svg")).expect("home svg");
        assert!(
            home.contains("data-name=\"Card\""),
            "nodes keep their names:\n{home}"
        );
        assert!(home.contains("<rect"), "a rectangle emits a <rect>:\n{home}");
        let css = fs::read_to_string(output.join("design.css")).expect("design css");
        assert!(
            css.contains("fill: var(--theme-surface, #FFFFFF);"),
            "a bound fill styles through the token:\n{css}"
        );
        let mut snapshot = BTreeMap::new();
        for entry in walk(&output) {
            snapshot.insert(entry.clone(), fs::read(&entry).expect("read"));
        }
        assert!(
            !snapshot.keys().any(|path| {
                path.extension()
                    .is_some_and(|e| e == "wasm" || e == "fnx" || e == "fss")
            }),
            "the projection must not ship WebAssembly or FNX sources"
        );

        let second = export_web(project.path(), None).expect("re-export");
        assert_eq!(second.pages.len(), 2);
        for (path, bytes) in &snapshot {
            assert_eq!(
                &fs::read(path).expect("re-read"),
                bytes,
                "{} changed across identical exports",
                path.display()
            );
        }
    }

    /// A2 (T1 scope): the id map names every node with its display-form id.
    #[test]
    fn page_map_lists_nodes_by_display_id() {
        let (project, card) = write_fixture_project();
        export_web(project.path(), None).expect("export");
        let map: serde_json::Value = serde_json::from_slice(
            &fs::read(project.path().join("web/pages/home/map.json")).expect("map"),
        )
        .expect("parse");
        let id = card.to_string();
        assert!(id.starts_with("n_"), "display ids are prefixed, got {id}");
        assert_eq!(map["nodes"][&id]["name"], "Card");
        let bounds = map["nodes"][&id]["bounds"].as_array().expect("bounds");
        assert_eq!(bounds[2].as_f64(), Some(120.0));
        assert_eq!(bounds[3].as_f64(), Some(80.0));
    }

    /// R6 groundwork: `nodes.json` carries the complete node records keyed by
    /// the persisted (bare ULID) id form.
    #[test]
    fn nodes_json_holds_full_records_by_persisted_id() {
        let (project, card) = write_fixture_project();
        export_web(project.path(), None).expect("export");
        let nodes: serde_json::Value = serde_json::from_slice(
            &fs::read(project.path().join("web/pages/home/nodes.json")).expect("nodes"),
        )
        .expect("parse");
        let bare = serde_json::to_value(card).expect("key");
        let bare = bare.as_str().expect("string key");
        assert!(
            nodes.get(bare).is_some(),
            "nodes.json keys are bare ULIDs; missing {bare} in:\n{nodes}"
        );
        assert_eq!(nodes[bare]["name"], "Card");
    }

    /// A5: every mode appears as a class and the default mode fills `:root`.
    #[test]
    fn tokens_css_carries_modes_as_classes() {
        let (project, _) = write_fixture_project();
        export_web(project.path(), None).expect("export");
        let css = fs::read_to_string(project.path().join("web/tokens.css")).expect("css");
        assert!(css.contains(".theme--light {"), "missing light scope in:\n{css}");
        assert!(css.contains(".theme--dark {"), "missing dark scope in:\n{css}");
        assert!(
            css.contains("--theme-surface: #FFFFFF;"),
            "missing light value in:\n{css}"
        );
        assert!(
            css.contains("--theme-surface: #000000;"),
            "missing dark value in:\n{css}"
        );
        assert!(css.contains(":root {"), "missing :root in:\n{css}");
    }

    #[test]
    fn refuses_an_unrelated_output_directory() {
        let (project, _) = write_fixture_project();
        let output = project.path().join("not-a-projection");
        fs::create_dir(&output).expect("dir");
        fs::write(output.join("keep.txt"), "user file").expect("file");
        let error = export_web(project.path(), Some(&output)).expect_err("must refuse");
        assert!(error.to_string().contains("refusing"), "{error:#}");
        assert_eq!(
            fs::read_to_string(output.join("keep.txt")).expect("kept"),
            "user file"
        );
    }

    fn walk(directory: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![directory.to_path_buf()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }
}
