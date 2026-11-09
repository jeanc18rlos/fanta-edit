//! The plain web projection (spec 0005, T1): one Skia-rendered SVG per page,
//! design tokens as CSS custom properties, an id map per page, and a
//! dependency-free HTML+JS viewer. No WebAssembly and no build step: serving
//! the directory with any static file server shows the design.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use fanta_doc::value::VarValue;
use fanta_doc::variables::{Variable, VariableRegistry};
use fanta_doc::{Doc, ModeId, NodeId};
use fanta_headless::HeadlessProject;
use fanta_headless::export::{ExportEncoding, ExportSizing, export_node};
use serde::Serialize;

const INDEX_TEMPLATE: &str = include_str!("../assets/web/index.html");
const VIEWER_JS: &str = include_str!("../assets/web/viewer.js");
/// Aliases may chain; past this many hops the chain is reported, not followed.
const MAX_ALIAS_HOPS: usize = 8;

/// What `export_web` wrote and what it had to leave out.
#[derive(Debug, Serialize)]
pub struct WebReport {
    pub pages: Vec<WebPageReport>,
    /// Variables that have no CSS form, with the reason.
    pub skipped_variables: Vec<SkippedVariable>,
    /// Warnings from opening the project, verbatim.
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WebPageReport {
    pub slug: String,
    pub name: String,
    /// Design units.
    pub width: u32,
    pub height: u32,
    /// Set when the page could not be rendered; no SVG was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkippedVariable {
    pub collection: String,
    pub variable: String,
    pub reason: String,
}

#[derive(Serialize)]
struct PageMap {
    /// The page root's id in display form and the world rectangle the SVG
    /// shows; node bounds below are in the same world coordinates.
    page: MapNode,
    nodes: BTreeMap<String, MapNode>,
}

#[derive(Serialize)]
struct MapNode {
    name: String,
    /// `[min_x, min_y, width, height]` in world units.
    bounds: [f64; 4],
}

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
    let pages_directory = output.join("pages");
    fs::create_dir_all(&pages_directory)
        .with_context(|| format!("creating {}", pages_directory.display()))?;

    let doc = opened.doc();
    let resolver = opened.document().asset_resolver();
    let mut used_slugs = BTreeSet::new();
    let mut pages = Vec::new();
    let mut page_maps: BTreeMap<String, PageMap> = BTreeMap::new();
    for page in doc.pages().to_vec() {
        let name = match doc.scene.get(page) {
            Some(node) if !node.name.is_empty() => node.name.clone(),
            Some(_) => "Untitled".to_string(),
            None => continue,
        };
        let slug = unique_slug(&name, &mut used_slugs);
        match export_node(
            doc,
            resolver.clone(),
            page,
            ExportEncoding::Svg,
            ExportSizing::Scale(1.0),
        ) {
            Ok(image) => {
                fs::write(pages_directory.join(format!("{slug}.svg")), &image.bytes)
                    .with_context(|| format!("writing pages/{slug}.svg"))?;
                page_maps.insert(slug.clone(), page_map(doc, page, &name));
                pages.push(WebPageReport {
                    slug,
                    name,
                    width: image.width,
                    height: image.height,
                    error: None,
                });
            }
            Err(error) => pages.push(WebPageReport {
                slug,
                name,
                width: 0,
                height: 0,
                error: Some(format!("{error:#}")),
            }),
        }
    }
    for (slug, map) in &page_maps {
        write_json(&pages_directory.join(format!("{slug}.map.json")), map)?;
    }

    let mut skipped_variables = Vec::new();
    let tokens = tokens_css(doc, &mut skipped_variables);
    fs::write(output.join("tokens.css"), tokens).context("writing tokens.css")?;

    let title = project
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Fanta project".to_string());
    fs::write(
        output.join("index.html"),
        INDEX_TEMPLATE.replace("{{title}}", &html_escape(&title)),
    )
    .context("writing index.html")?;
    fs::write(output.join("viewer.js"), VIEWER_JS).context("writing viewer.js")?;

    let manifest: Vec<_> = pages
        .iter()
        .filter(|page| page.error.is_none())
        .map(|page| {
            serde_json::json!({
                "slug": page.slug,
                "name": page.name,
                "width": page.width,
                "height": page.height,
            })
        })
        .collect();
    let mut pages_js = String::from("const FANTA_PAGES = ");
    pages_js.push_str(&serde_json::to_string_pretty(&manifest).context("encoding pages.js")?);
    pages_js.push_str(";\n");
    fs::write(output.join("pages.js"), pages_js).context("writing pages.js")?;

    let report = WebReport {
        pages,
        skipped_variables,
        warnings: opened.warnings().to_vec(),
    };
    write_json(&output.join("report.json"), &report)?;
    Ok(report)
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

fn unique_slug(name: &str, used: &mut BTreeSet<String>) -> String {
    let base = match crate::slug(name) {
        slug if slug.is_empty() => "page".to_string(),
        slug => slug,
    };
    let mut candidate = base.clone();
    let mut counter = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}-{counter}");
        counter += 1;
    }
    candidate
}

fn page_map(doc: &Doc, page: NodeId, page_name: &str) -> PageMap {
    let mut nodes = BTreeMap::new();
    for id in doc.scene.descendants_of(page) {
        let Some(node) = doc.scene.get(id) else {
            continue;
        };
        let Some(bounds) = doc.scene.world_bounds(id) else {
            continue;
        };
        // Display form (`n_…`): this file is for outside consumers, which per
        // ADR 0014 see prefixed ids, like MCP and logs do.
        nodes.insert(
            id.to_string(),
            MapNode {
                name: node.name.clone(),
                bounds: [
                    bounds.min_x,
                    bounds.min_y,
                    bounds.max_x - bounds.min_x,
                    bounds.max_y - bounds.min_y,
                ],
            },
        );
    }
    let page_bounds = doc
        .scene
        .world_bounds(page)
        .map(|bounds| {
            [
                bounds.min_x,
                bounds.min_y,
                bounds.max_x - bounds.min_x,
                bounds.max_y - bounds.min_y,
            ]
        })
        .unwrap_or([0.0, 0.0, 0.0, 0.0]);
    PageMap {
        page: MapNode {
            name: page_name.to_string(),
            bounds: page_bounds,
        },
        nodes,
    }
}

/// Variables and modes as CSS custom properties: `:root` carries the active
/// (or default) mode of every collection, and each mode gets a class scope so
/// the page can switch by toggling one class.
fn tokens_css(doc: &Doc, skipped: &mut Vec<SkippedVariable>) -> String {
    let registry = &doc.variables;
    let mut by_collection: BTreeMap<_, Vec<&Variable>> = BTreeMap::new();
    for variable in registry.variables.values() {
        by_collection
            .entry(variable.collection)
            .or_default()
            .push(variable);
    }

    let mut css = String::from(
        "/* Generated by Fanta from doc/variables.json; edit the project, not this file. */\n",
    );
    let mut root_block = String::new();
    for (collection_id, variables) in &by_collection {
        let Some(collection) = registry.collections.get(collection_id) else {
            continue;
        };
        let collection_slug = crate::slug(&collection.name);
        let active_mode = doc
            .active_modes
            .get(collection_id)
            .copied()
            .unwrap_or(collection.default_mode);
        for mode in &collection.modes {
            let mut block = String::new();
            for variable in variables {
                match css_value_in_mode(registry, doc, variable, mode.id) {
                    Ok(value) => {
                        let _ = writeln!(
                            block,
                            "  --{collection_slug}-{}: {value};",
                            crate::slug(&variable.name)
                        );
                    }
                    Err(reason) => {
                        // Report a variable once, from its collection's first mode.
                        if mode.id == collection.modes[0].id {
                            skipped.push(SkippedVariable {
                                collection: collection.name.clone(),
                                variable: variable.name.clone(),
                                reason,
                            });
                        }
                    }
                }
            }
            if block.is_empty() {
                continue;
            }
            let _ = writeln!(
                css,
                ".{collection_slug}--{} {{\n{block}}}",
                crate::slug(&mode.name)
            );
            if mode.id == active_mode {
                root_block.push_str(&block);
            }
        }
    }
    if !root_block.is_empty() {
        css.push_str(":root {\n");
        css.push_str(&root_block);
        css.push_str("}\n");
    }
    css
}

fn css_value_in_mode(
    registry: &VariableRegistry,
    doc: &Doc,
    variable: &Variable,
    mode: ModeId,
) -> Result<String, String> {
    let mut current = variable;
    let mut current_mode = mode;
    for _ in 0..MAX_ALIAS_HOPS {
        let Some(value) = current.value_for_mode(current_mode) else {
            return Err(format!("no value for mode of {}", current.name));
        };
        match value {
            VarValue::Alias { variable: target } => {
                let Some(target) = registry.variable(*target) else {
                    return Err(format!("alias from {} to a missing variable", current.name));
                };
                // An alias into another collection resolves under that
                // collection's own active (or default) mode; within the same
                // collection the mode axis carries over.
                current_mode = if target.collection == current.collection {
                    current_mode
                } else {
                    doc.active_modes
                        .get(&target.collection)
                        .copied()
                        .or_else(|| {
                            registry
                                .collections
                                .get(&target.collection)
                                .map(|collection| collection.default_mode)
                        })
                        .ok_or_else(|| format!("alias into a missing collection from {}", current.name))?
                };
                current = target;
            }
            VarValue::Color { value } => return Ok(css_color(*value)),
            VarValue::Float { value } => return Ok(format!("{value}")),
            VarValue::Boolean { value } => return Ok(value.to_string()),
            VarValue::String { value } | VarValue::Enum { value } => {
                if value.contains([';', '{', '}']) || value.contains('\n') {
                    return Err("string holds CSS delimiters".to_string());
                }
                return Ok(value.clone());
            }
            VarValue::TextStyle { .. } => {
                return Err("typography tokens have no single CSS value yet".to_string());
            }
            VarValue::Image { .. } => {
                return Err("image tokens have no CSS projection yet".to_string());
            }
        }
    }
    Err(format!("alias chain from {} is too deep", variable.name))
}

fn css_color(color: fanta_doc::color::Color) -> String {
    if color.a == 255 {
        color.to_hex()
    } else {
        format!(
            "rgba({}, {}, {}, {:.3})",
            color.r,
            color.g,
            color.b,
            f64::from(color.a) / 255.0
        )
    }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("encoding {}", path.display()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use fanta_doc::color::Color;
    use fanta_doc::variables::{Mode, VariableCollection};
    use fanta_doc::{CanvasNode, GroupNode, NodeData, Operation, Transform2D, VectorNode};

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
            }
            doc.apply(Operation::create_node(layer)).expect("layer");
        }
        let card_id = card_id.expect("card id");

        let collection_id = Default::default();
        let (light, dark) = (ModeId::new(), ModeId::new());
        doc.variables.collections.insert(
            collection_id,
            VariableCollection {
                id: collection_id,
                name: "Theme".into(),
                modes: vec![
                    Mode { id: light, name: "Light".into() },
                    Mode { id: dark, name: "Dark".into() },
                ],
                default_mode: light,
                variable_order: Vec::new(),
            },
        );
        let variable_id = Default::default();
        doc.variables.variables.insert(
            variable_id,
            Variable {
                id: variable_id,
                collection: collection_id,
                name: "Surface".into(),
                ty: fanta_doc::value::VariableType::Color,
                values_by_mode: [
                    (light, VarValue::Color { value: Color::WHITE }),
                    (dark, VarValue::Color { value: Color::BLACK }),
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

    /// A1 and A3 (T1 scope): the full file set appears, with no wasm, and a
    /// second export over the first is byte-identical.
    #[test]
    fn exports_the_file_set_deterministically() {
        let (project, _) = write_fixture_project();
        let report = export_web(project.path(), None).expect("export");
        assert_eq!(report.pages.len(), 2);
        assert!(report.pages.iter().all(|page| page.error.is_none()));

        let output = project.path().join("web");
        for file in [
            "index.html",
            "viewer.js",
            "tokens.css",
            "pages.js",
            "report.json",
            "pages/home.svg",
            "pages/home.map.json",
            "pages/about.svg",
            "pages/about.map.json",
        ] {
            assert!(output.join(file).is_file(), "missing {file}");
        }
        let mut snapshot = BTreeMap::new();
        for entry in walk(&output) {
            snapshot.insert(entry.clone(), fs::read(&entry).expect("read"));
        }
        assert!(
            !snapshot.keys().any(|path| path.extension().is_some_and(|e| e == "wasm")),
            "the projection must not ship WebAssembly"
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
            &fs::read(project.path().join("web/pages/home.map.json")).expect("map"),
        )
        .expect("parse");
        let id = card.to_string();
        assert!(id.starts_with("n_"), "display ids are prefixed, got {id}");
        assert_eq!(map["nodes"][&id]["name"], "Card");
        let bounds = map["nodes"][&id]["bounds"].as_array().expect("bounds");
        assert_eq!(bounds[2].as_f64(), Some(120.0));
        assert_eq!(bounds[3].as_f64(), Some(80.0));
    }

    /// A5: every mode appears as a class and the default mode fills `:root`.
    #[test]
    fn tokens_css_carries_modes_as_classes() {
        let (project, _) = write_fixture_project();
        export_web(project.path(), None).expect("export");
        let css = fs::read_to_string(project.path().join("web/tokens.css")).expect("css");
        assert!(css.contains(".theme--light {"), "missing light scope in:\n{css}");
        assert!(css.contains(".theme--dark {"), "missing dark scope in:\n{css}");
        assert!(css.contains("--theme-surface: #FFFFFF;"), "missing light value in:\n{css}");
        assert!(css.contains("--theme-surface: #000000;"), "missing dark value in:\n{css}");
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
