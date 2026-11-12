//! The plain-web-files project format (spec 0007, ADR 0027): one CSS-first
//! SVG per page and per component, all styling in `design.css` (with
//! `var(--token)` wherever the design binds a variable), variables and modes
//! in `tokens.css`, a dependency-free HTML+JS viewer, and the shared JSON
//! sidecars. No `.fnx`, no `.fss`, no WebAssembly, no build step.
//!
//! The SVG and CSS are the readable projection of structure and styling;
//! `nodes.json` beside each SVG carries the complete node records, so the
//! format is lossless from day one while native SVG/CSS coverage grows
//! (T4 shrinks the sidecar as coverage lands — see the spec's design notes).
//!
//! Native coverage (R11): solid and gradient paints (linear and axis-aligned
//! radial), stacked fills as `-f<n>` clone elements, masks as SVG `<mask>`,
//! booleans as their flattened outline, and instances as inline-styled
//! derived content without element ids (expansion ids are not stable across
//! saves, so derived content never enters `design.css` or the loader).
//!
//! This module is pure over the document (the persistence layer forbids
//! Skia): node kinds outside the SVG subset need a pre-rendered PNG, and an
//! unbaked boolean needs its flattened outline, from the caller's
//! [`WebRenderSource`] (`fanta-headless` implements it over its exporter).
//! With no render source, those nodes keep their full record in `nodes.json`
//! but draw nothing in the SVG, and the report says so per node — the write
//! still succeeds, so renderer-less library callers (tests, migration
//! tooling) stay usable.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fanta_doc::value::VarValue;
use fanta_doc::variables::{Variable, VariableRegistry};
use fanta_doc::{
    AssetId, BlendMode, BlurKind, BoundProp, Bounds, CanvasNode, Color, Doc, Fill, Gradient,
    GradientStop, InstanceNode, ModeId, NodeData, NodeFlags, NodeId, PathData, PathSegment,
    ShadowKind, Stroke, TextAlign, TextStyle, Transform2D, VectorNode, expand_instance,
    resolve_export_bindings,
};
use serde::Serialize;
use serde_json::Value;

use crate::error::{FormatError, Result};

use super::layout::{
    ASSETS_DIR, COMPONENTS_DIR, FANTA_JSON, PAGES_DIR, ProjectManifest, STORAGE_WEB, json_bytes,
    json_key, slugify,
};
use super::diagnostic::SourceDiagnostic;
use super::media::{ASSET_INDEX_FILE, asset_index_bytes, sniff_media};
#[cfg(feature = "native")]
use super::write::{WriteReport, sync_project_files};
use super::write::{
    ProjectedFiles, design_grouping, project_asset_files, project_design_headers,
    project_doc_singletons, project_loose_nodes,
};

pub(crate) const PAGE_SVG: &str = "page.svg";
pub(crate) const MASTER_SVG: &str = "master.svg";
pub(crate) const DESIGN_CSS: &str = "design.css";
pub(crate) const NODES_JSON: &str = "nodes.json";
pub(crate) const MAP_JSON: &str = "map.json";
pub(crate) const PAGE_CSS: &str = "page.css";
pub(crate) const MASTER_CSS: &str = "master.css";
const INDEX_TEMPLATE: &str = include_str!("../../assets/web/index.html");
const VIEWER_JS: &str = include_str!("../../assets/web/viewer.js");
// The host-neutral canvas component (spec 0008 T2), emitted verbatim so the
// served project and agent-host panes run the same canvas.
const CANVAS_JS: &str = include_str!("../../../../web/fanta-canvas-web/src/fanta-canvas.js");
const EDITOR_JS: &str = include_str!("../../assets/web/editor.js");
/// Aliases may chain; past this many hops the chain is reported, not followed.
const MAX_ALIAS_HOPS: usize = 8;

/// A pre-rendered PNG fallback for one node outside the SVG subset.
#[derive(Debug, Clone)]
pub struct WebRaster {
    pub bytes: Vec<u8>,
    /// The world rectangle the image shows.
    pub bounds: Bounds,
}

/// Renderer-derived data the pure projection cannot compute itself. The
/// writer queries it on demand during a save; `fanta-headless` provides the
/// shared implementation over its exporter, so the editor, the engine and
/// `fanta-site` project identically (R10). Implementations must be
/// deterministic over an unchanged document — the written bytes are a pure
/// function of the document (R5), and a save of an unchanged project must
/// not rewrite files.
pub trait WebRenderSource {
    /// viewBox bounds for a design root (page or component root), effects
    /// included. `None` falls back to the scene's world bounds.
    fn design_bounds(&self, root: NodeId) -> Option<Bounds>;
    /// A PNG covering `node`'s visual bounds in world space, for node kinds
    /// beyond the SVG subset.
    fn raster(&self, node: NodeId) -> Option<WebRaster>;
    /// The flattened outline of a boolean with no baked geometry.
    fn outline(&self, node: NodeId) -> Option<PathData>;
}

/// What the projection wrote and what it had to approximate.
#[derive(Debug, Serialize)]
pub struct WebReport {
    pub pages: Vec<WebDesignReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<WebDesignReport>,
    /// Variables that have no CSS form, with the reason.
    pub skipped_variables: Vec<SkippedVariable>,
    /// Findings from opening the project (spec 0009 R2).
    pub warnings: Vec<SourceDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct WebDesignReport {
    pub slug: String,
    pub name: String,
    /// Design units.
    pub width: u32,
    pub height: u32,
    /// Where the emitted SVG diverges from the canvas (rasterized nodes,
    /// browser-laid-out text, dropped features).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub approximations: Vec<Approximation>,
    /// Set when no SVG could be emitted (`nodes.json` still has the design).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkippedVariable {
    pub collection: String,
    pub variable: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Approximation {
    pub node: String,
    pub name: String,
    pub what: String,
}

/// The complete in-memory web projection of `doc`, before any disk I/O.
pub struct WebProjection {
    pub files: BTreeMap<PathBuf, Arc<Vec<u8>>>,
    pub report: WebReport,
}

/// Write `doc` to `dir` in the web format, through the same transactional
/// sync as the legacy FNX writer (locking, symlink defense, stale pruning —
/// which also removes `.fnx`/`.fss` files when converting a project in
/// place). `render` supplies what the pure projection cannot compute; with
/// `None`, nodes that need it draw nothing and the report says so.
#[cfg(feature = "native")]
pub fn write_web_project_tree(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    render: Option<&dyn WebRenderSource>,
    warnings: &[SourceDiagnostic],
) -> Result<(WriteReport, WebReport)> {
    let asset_hrefs = asset_hrefs(assets);
    let projection = web_project_files(doc, &asset_hrefs, render, warnings)?;
    let mut files = projection.files;
    files.insert(
        PathBuf::from(ASSETS_DIR).join(ASSET_INDEX_FILE),
        Arc::new(asset_index_bytes(assets)?),
    );
    let asset_files = project_asset_files(assets, None)?;
    // Designs the load left out are not in `doc`. Their files are not stale:
    // they are what the user has to repair (spec 0009 R11).
    let unavailable: Vec<PathBuf> = warnings
        .iter()
        .filter(|finding| finding.code == super::diagnostic::DESIGN_UNAVAILABLE)
        .filter_map(|finding| finding.file.as_deref().map(PathBuf::from))
        .collect();
    if let Some(taken) = files
        .keys()
        .find(|path| unavailable.iter().any(|design| path.starts_with(design)))
    {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} would overwrite a design that could not be loaded; repair its files or rename the new design",
            taken.display()
        )));
    }
    let write = sync_project_files(dir, &files, &asset_files, None, &unavailable)?;
    Ok((write, projection.report))
}

/// Everything [`write_web_project_tree`] writes, as one in-memory map — the
/// asset index and the asset bytes included — with no disk I/O. The browser
/// engine emits through this, so a browser save and a native save are the
/// same byte path by construction (spec 0008 R3). The one difference: the
/// editor-support seeds (`AGENTS.md`, `.gitignore`, …) are stateful on disk
/// — seeded only when absent or still pristine — so they belong to the disk
/// sync, and a host writing this map back must not delete files it does not
/// contain.
pub fn emit_web_project_tree(
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    render: Option<&dyn WebRenderSource>,
    warnings: &[SourceDiagnostic],
) -> Result<(BTreeMap<PathBuf, Arc<Vec<u8>>>, WebReport)> {
    let asset_hrefs = asset_hrefs(assets);
    let projection = web_project_files(doc, &asset_hrefs, render, warnings)?;
    let mut files = projection.files;
    files.insert(
        PathBuf::from(ASSETS_DIR).join(ASSET_INDEX_FILE),
        Arc::new(asset_index_bytes(assets)?),
    );
    for (path, bytes) in project_asset_files(assets, None)? {
        files.insert(path, Arc::new(bytes.to_vec()));
    }
    Ok((files, projection.report))
}

/// `assets/<family>/<id>.<ext>` for every asset — the same naming
/// [`project_asset_files`] uses, as href strings for `<image>` elements.
pub(crate) fn asset_hrefs(assets: &BTreeMap<AssetId, Vec<u8>>) -> BTreeMap<AssetId, String> {
    assets
        .iter()
        .map(|(id, bytes)| {
            let (family, ext) = sniff_media(bytes);
            (*id, format!("{ASSETS_DIR}/{family}/{id}.{ext}"))
        })
        .collect()
}

/// The complete in-memory web projection: every non-asset file the tree
/// should contain, with the exact bytes, as a pure function of the document
/// and the inputs (the byte-determinism contract).
pub fn web_project_files(
    doc: &Doc,
    asset_hrefs: &BTreeMap<AssetId, String>,
    render: Option<&dyn WebRenderSource>,
    warnings: &[SourceDiagnostic],
) -> Result<WebProjection> {
    let resolved = resolve_export_bindings(doc);
    let grouping = design_grouping(doc)?;
    let mut files = ProjectedFiles::new();

    let mut manifest = ProjectManifest::for_doc(doc);
    manifest.storage = Some(STORAGE_WEB.to_owned());
    files.insert(
        PathBuf::from(FANTA_JSON),
        Arc::new(json_bytes(&serde_json::to_value(manifest)?)?),
    );
    project_doc_singletons(&mut files, doc)?;
    project_design_headers(&mut files, doc, &grouping)?;
    project_loose_nodes(&mut files, doc, &grouping.loose)?;

    let mut design_css = String::from(
        "/* Generated by Fanta: imports each design's stylesheet. Project-wide rules added here apply on load; per-node rules are canonicalized into the owning design's file on save. */\n",
    );
    let mut pages = Vec::new();
    let mut components = Vec::new();

    // Pages in document order.
    for page in &doc.pages {
        let slug = grouping.page_slugs.get(page).ok_or_else(|| {
            FormatError::InvalidProjectTree(format!("page {page} has no directory slug"))
        })?;
        let design_dir = PathBuf::from(PAGES_DIR).join(slug);
        let name = doc
            .scene
            .get(*page)
            .map(|node| node.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Untitled")
            .to_owned();
        let nodes = grouping.page_nodes.get(slug).map_or(&[][..], Vec::as_slice);
        design_nodes_json(&mut files, doc, &design_dir, nodes)?;
        let report = emit_design(
            doc,
            &resolved,
            *page,
            &design_dir,
            PAGE_SVG,
            slug,
            &name,
            asset_hrefs,
            render,
            &mut files,
            &mut design_css,
        )?;
        files.insert(
            design_dir.join(MAP_JSON),
            Arc::new(json_bytes(&serde_json::to_value(page_map(
                doc, *page, &name,
            ))?)?),
        );
        pages.push(report);
    }

    // Component designs, sorted by directory.
    let root_of_dir: BTreeMap<&String, NodeId> = grouping
        .component_roots
        .iter()
        .map(|(root, cdir)| (cdir, *root))
        .collect();
    for (cdir, nodes) in &grouping.comp_nodes {
        let design_dir = PathBuf::from(COMPONENTS_DIR).join(cdir);
        design_nodes_json(&mut files, doc, &design_dir, nodes)?;
        let Some(root) = root_of_dir.get(cdir).copied() else {
            continue;
        };
        let name = doc
            .scene
            .get(root)
            .map(|node| node.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Component")
            .to_owned();
        let report = emit_design(
            doc,
            &resolved,
            root,
            &design_dir,
            MASTER_SVG,
            cdir,
            &name,
            asset_hrefs,
            render,
            &mut files,
            &mut design_css,
        )?;
        components.push(report);
    }

    let mut skipped_variables = Vec::new();
    files.insert(
        PathBuf::from("tokens.css"),
        Arc::new(tokens_css(doc, &mut skipped_variables).into_bytes()),
    );
    files.insert(PathBuf::from(DESIGN_CSS), Arc::new(design_css.into_bytes()));

    let title = match doc.metadata.title.trim() {
        "" => "Fanta project",
        title => title,
    };
    files.insert(
        PathBuf::from("index.html"),
        Arc::new(
            INDEX_TEMPLATE
                .replace("{{title}}", &html_escape(title))
                .into_bytes(),
        ),
    );
    files.insert(
        PathBuf::from("viewer.js"),
        Arc::new(VIEWER_JS.as_bytes().to_vec()),
    );
    files.insert(
        PathBuf::from("canvas.js"),
        Arc::new(CANVAS_JS.as_bytes().to_vec()),
    );
    files.insert(
        PathBuf::from("editor.js"),
        Arc::new(EDITOR_JS.as_bytes().to_vec()),
    );

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
    pages_js.push_str(&serde_json::to_string_pretty(&manifest)?);
    pages_js.push_str(";\n");
    files.insert(PathBuf::from("pages.js"), Arc::new(pages_js.into_bytes()));

    let report = WebReport {
        pages,
        components,
        skipped_variables,
        warnings: warnings.to_vec(),
    };
    files.insert(
        PathBuf::from("report.json"),
        Arc::new(json_bytes(&serde_json::to_value(&report)?)?),
    );

    Ok(WebProjection { files, report })
}

/// `nodes.json` beside a design's SVG: the complete record of every node in
/// the design, keyed by the persisted (serde) id form, in sorted key order.
/// This is what makes the format lossless while the SVG/CSS subset grows.
fn design_nodes_json(
    files: &mut ProjectedFiles,
    doc: &Doc,
    design_dir: &Path,
    nodes: &[NodeId],
) -> Result<()> {
    let mut table = serde_json::Map::new();
    for id in nodes {
        let node = doc
            .scene
            .get(*id)
            .ok_or_else(|| FormatError::InvalidProjectTree(format!("node {id} vanished")))?;
        let mut value = serde_json::to_value(node)?;
        crate::project::read::repair_non_finite_geometry(&mut value);
        table.insert(json_key(id)?, value);
    }
    files.insert(
        design_dir.join(NODES_JSON),
        Arc::new(json_bytes(&Value::Object(table))?),
    );
    Ok(())
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

/// Emit one design's SVG into `files` and its CSS into `design_css`.
#[expect(clippy::too_many_arguments, reason = "internal assembly step")]
fn emit_design(
    authored: &Doc,
    resolved: &Doc,
    root: NodeId,
    design_dir: &Path,
    svg_name: &str,
    slug: &str,
    name: &str,
    asset_hrefs: &BTreeMap<AssetId, String>,
    render: Option<&dyn WebRenderSource>,
    files: &mut ProjectedFiles,
    design_css: &mut String,
) -> Result<WebDesignReport> {
    let design_dir_href = design_dir.to_string_lossy().replace('\\', "/");
    // A design that draws nothing (an empty page) still gets its SVG: the
    // root element is where a hand edit adds the first content.
    let bounds = render
        .and_then(|render| render.design_bounds(root))
        .or_else(|| resolved.scene.world_bounds(root))
        .filter(|bounds| {
            bounds.min_x.is_finite()
                && bounds.min_y.is_finite()
                && bounds.max_x >= bounds.min_x
                && bounds.max_y >= bounds.min_y
        })
        .unwrap_or(Bounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 0.0,
            max_y: 0.0,
        });
    let mut emitter = Emitter {
        resolved,
        authored,
        registry: &authored.variables,
        asset_hrefs,
        render,
        raster_dir: &design_dir_href,
        css: String::new(),
        defs: String::new(),
        rasters: Vec::new(),
        approximations: Vec::new(),
        virtual_defs: 0,
    };
    let mut body = String::new();
    emitter.emit_node(root, &mut body, 1)?;
    let width = (bounds.max_x - bounds.min_x).ceil();
    let height = (bounds.max_y - bounds.min_y).ceil();
    let defs = if emitter.defs.is_empty() {
        String::new()
    } else {
        format!("  <defs>\n{}  </defs>\n", emitter.defs)
    };
    // The styling lives in the root css files, so a standalone open of the
    // SVG (browser, file manager preview) must reference them or it renders
    // unstyled. Browsers honor xml-stylesheet on an SVG *document*; the
    // loader's XML reader skips processing instructions.
    let to_root = "../".repeat(design_dir.components().count());
    let css_name = svg_name.replace(".svg", ".css");
    // data-fanta marks the file as Fanta source and says which design kind it
    // is, so a tool handed the bare file can route it without relying on the
    // directory shape. The loader ignores unknown root attributes.
    let kind = svg_name.strip_suffix(".svg").unwrap_or(svg_name);
    let svg = format!(
        "<?xml-stylesheet type=\"text/css\" href=\"{to_root}tokens.css\"?>\n\
         <?xml-stylesheet type=\"text/css\" href=\"{css_name}\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" data-fanta=\"{kind}\" \
         width=\"{width}\" height=\"{height}\" \
         viewBox=\"{} {} {width} {height}\">\n{defs}{body}</svg>\n",
        number(bounds.min_x),
        number(bounds.min_y),
    );
    // Every design gets its stylesheet, even an empty one: it is where a hand
    // edit adds the first rule, and the SVG's xml-stylesheet reference stays
    // valid.
    let scoped_css = format!(
        "/* Generated by Fanta: {design_dir_href} per-node styles, selected by node id. Edit values here or in the editor; the writer canonicalizes. */\n{}",
        emitter.css
    );
    files.insert(
        design_dir.join(&css_name),
        Arc::new(scoped_css.into_bytes()),
    );
    design_css.push_str(&format!("@import url(\"{design_dir_href}/{css_name}\");\n"));
    files.insert(design_dir.join(svg_name), Arc::new(svg.into_bytes()));
    for (file, bytes) in emitter.rasters {
        files.insert(design_dir.join(file), Arc::new(bytes));
    }
    Ok(WebDesignReport {
        slug: slug.to_owned(),
        name: name.to_owned(),
        width: width as u32,
        height: height as u32,
        approximations: emitter.approximations,
        error: None,
    })
}

struct Emitter<'a> {
    /// Bound values substituted, so literals match the active modes.
    resolved: &'a Doc,
    /// The authored document: bindings live here.
    authored: &'a Doc,
    registry: &'a VariableRegistry,
    asset_hrefs: &'a BTreeMap<AssetId, String>,
    render: Option<&'a dyn WebRenderSource>,
    /// Project-relative directory raster hrefs point into.
    raster_dir: &'a str,
    css: String,
    /// `<defs>` content: gradient paint servers and instance clip paths.
    defs: String,
    rasters: Vec<(String, Vec<u8>)>,
    approximations: Vec<Approximation>,
    /// Counter for def ids inside derived (instance) content, which emits no
    /// node ids because expansion ids are not stable across saves.
    virtual_defs: usize,
}

impl Emitter<'_> {
    fn emit_node(&mut self, id: NodeId, out: &mut String, depth: usize) -> Result<()> {
        let Some(node) = self.resolved.scene.get(id).cloned() else {
            return Ok(());
        };
        if node.flags.contains(NodeFlags::HIDDEN) {
            return Ok(());
        }

        match &node.data {
            NodeData::Group(group) => {
                let group = group.clone();
                self.start_element(&node, out, depth, "g");
                out.push_str(">\n");
                let size = group.clip_size.or(group.local_size);
                let radius = group
                    .corner_radii
                    .map(|radii| radii[0])
                    .or(group.corner_radius);
                if group
                    .corner_radii
                    .is_some_and(|radii| radii.windows(2).any(|pair| pair[0] != pair[1]))
                {
                    self.approximate(&node, "mixed corner radii use the first corner");
                }
                if let Some([width, height]) = size {
                    let backgrounds: Vec<Fill> = group
                        .background
                        .iter()
                        .chain(group.background_fills.iter())
                        .cloned()
                        .collect();
                    for (index, fill) in backgrounds.iter().enumerate() {
                        let element_id = if index == 0 {
                            format!("{id}-bg")
                        } else {
                            format!("{id}-bg{index}")
                        };
                        indent(out, depth + 1);
                        let _ = writeln!(
                            out,
                            "<rect id=\"{element_id}\" width=\"{}\" height=\"{}\"/>",
                            number(width),
                            number(height),
                        );
                        let mut rule = Declarations::default();
                        match self.paint_value(&node, fill, index, &element_id) {
                            Some(paint) => rule.push("fill", paint),
                            None => self.approximate(
                                &node,
                                "a background paint beyond the SVG subset is dropped",
                            ),
                        }
                        if let Some(radius) = radius {
                            rule.push_bound(
                                self,
                                &node,
                                BoundProp::CornerRadius,
                                "rx",
                                number(radius),
                            );
                        }
                        self.rule(&element_id, rule);
                    }
                    if !group.strokes.is_empty() {
                        self.approximate(&node, "frame borders are not emitted yet");
                    }
                    if group.clip_size.is_some() {
                        indent(out, depth + 1);
                        let _ = writeln!(
                            out,
                            "<clipPath id=\"{id}-clip\"><rect width=\"{}\" height=\"{}\"{}/></clipPath>",
                            number(width),
                            number(height),
                            radius
                                .map(|radius| format!(" rx=\"{}\"", number(radius)))
                                .unwrap_or_default(),
                        );
                    }
                }
                self.emit_children(id, out, depth)?;
                indent(out, depth);
                out.push_str("</g>\n");
            }
            NodeData::Vector(vector) => {
                let vector = vector.clone();
                self.emit_vector(&node, &vector, out, depth)?;
            }
            NodeData::Boolean(boolean) => {
                // The flattened outline is the visual projection (R11); the
                // operands stay in nodes.json, where the editor reads them.
                let outline = boolean
                    .baked
                    .as_ref()
                    .map(|baked| baked.vector.path.clone())
                    .or_else(|| self.render.and_then(|render| render.outline(id)));
                let Some(outline) = outline else {
                    self.approximate(
                        &node,
                        "not drawn: flattening this boolean needs a renderer",
                    );
                    return Ok(());
                };
                let vector = VectorNode {
                    path: outline,
                    fills: boolean.fills.clone(),
                    strokes: boolean.strokes.clone(),
                    ..VectorNode::default()
                };
                self.emit_vector(&node, &vector, out, depth)?;
            }
            NodeData::Instance(instance) => {
                let instance = instance.clone();
                self.start_element(&node, out, depth, "g");
                out.push_str(">\n");
                // Like the canvas, the expansion clips to the instance size.
                let [width, height] = instance.local_size;
                if width > 0.0 && height > 0.0 {
                    indent(out, depth + 1);
                    let _ = writeln!(
                        out,
                        "<clipPath id=\"{id}-clip\"><rect width=\"{}\" height=\"{}\"/></clipPath>",
                        number(width),
                        number(height),
                    );
                    indent(out, depth + 1);
                    let _ = writeln!(out, "<g clip-path=\"url(#{id}-clip)\">");
                    self.emit_expansion(&node, &instance, out, depth + 2)?;
                    indent(out, depth + 1);
                    out.push_str("</g>\n");
                } else {
                    self.emit_expansion(&node, &instance, out, depth + 1)?;
                }
                indent(out, depth);
                out.push_str("</g>\n");
            }
            NodeData::Text(text) => {
                let text = text.clone();
                let style = &text.style;
                let mut rule = Declarations::default();
                rule.push("font-family", css_font_family(&style.font_family));
                rule.push("font-size", format!("{}px", number(style.size_px)));
                if style.weight != 400 {
                    rule.push("font-weight", style.weight.to_string());
                }
                if style.italic {
                    rule.push("font-style", "italic".to_string());
                }
                if style.letter_spacing != 0.0 {
                    rule.push(
                        "letter-spacing",
                        format!("{}px", number(style.letter_spacing)),
                    );
                }
                match (style.underline, style.strikethrough) {
                    (true, true) => {
                        rule.push("text-decoration", "underline line-through".to_string())
                    }
                    (true, false) => rule.push("text-decoration", "underline".to_string()),
                    (false, true) => rule.push("text-decoration", "line-through".to_string()),
                    (false, false) => {}
                }
                let color = css_color(style.color);
                match self.bound_variable(&node, BoundProp::TextStyle) {
                    Some(token) => rule.push("fill", format!("var({token}, {color})")),
                    None => rule.push("fill", color),
                }
                if !text.style_runs.is_empty() {
                    self.approximate(&node, "per-run rich text styling is flattened");
                }
                let (anchor, x) = match text.align {
                    TextAlign::Left => (None, 0.0),
                    TextAlign::Center => (Some("middle"), text.local_size[0] / 2.0),
                    TextAlign::Right => (Some("end"), text.local_size[0]),
                    _ => {
                        self.approximate(&node, "justified text renders left-aligned");
                        (None, 0.0)
                    }
                };
                if let Some(anchor) = anchor {
                    rule.push("text-anchor", anchor.to_string());
                }
                self.approximate(&node, "text is laid out by the browser, not the engine shaper");
                let line_height = style.size_px * line_height_factor(style);
                self.start_element(&node, out, depth, "text");
                let _ = write!(out, " y=\"{}\"", number(style.size_px));
                out.push_str(">\n");
                for (index, line) in text.content.split('\n').enumerate() {
                    indent(out, depth + 1);
                    let _ = writeln!(
                        out,
                        "<tspan x=\"{}\"{}>{}</tspan>",
                        number(x),
                        if index == 0 {
                            String::new()
                        } else {
                            format!(" dy=\"{}\"", number(line_height))
                        },
                        xml_escape(line),
                    );
                }
                indent(out, depth);
                out.push_str("</text>\n");
                self.rule(&id.to_string(), rule);
            }
            NodeData::Bitmap(bitmap) => {
                let bitmap = bitmap.clone();
                if bitmap.crop.is_some() {
                    return self.raster_fallback(&node, out, depth, "cropped image");
                }
                let href = match self.asset_hrefs.get(&bitmap.asset) {
                    Some(href) => href.clone(),
                    None => {
                        self.approximate(&node, "image asset has no file under assets/");
                        format!("{ASSETS_DIR}/{}", bitmap.asset)
                    }
                };
                self.start_element(&node, out, depth, "image");
                let _ = write!(
                    out,
                    " href=\"{href}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\"",
                    number(bitmap.local_size[0]),
                    number(bitmap.local_size[1]),
                );
                out.push_str("/>\n");
            }
            data => {
                let what = unsupported_kind(data);
                return self.raster_fallback(&node, out, depth, what);
            }
        }
        Ok(())
    }

    /// Writes `<tag id class data-name transform clip-path` without closing,
    /// so the caller appends shape attributes; also queues the node-level CSS
    /// shared by every kind (opacity, blend, shadows, blurs).
    fn start_element(&mut self, node: &CanvasNode, out: &mut String, depth: usize, tag: &str) {
        indent(out, depth);
        let _ = write!(out, "<{tag} id=\"{}\"", node.id);
        let class = slugify(&node.name, "");
        if !class.is_empty() {
            let _ = write!(out, " class=\"{class}\"");
        }
        let _ = write!(out, " data-name=\"{}\"", xml_escape(&node.name));
        out.push_str(&self.shared_attrs(node));

        let mut rule = Declarations::default();
        self.node_level_declarations(node, &mut rule);
        // R8: auto-layout intent lives in CSS; the solved child geometry in
        // the SVG is derived from it and regenerated when the intent changes.
        if let NodeData::Group(group) = &node.data
            && let Some(layout) = &group.auto_layout
        {
            if let Some(mode) = serde_json::to_value(layout.mode)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
            {
                rule.push("--fanta-layout", mode);
            }
            rule.push("--fanta-gap", number(layout.spacing));
            if layout.padding != [0.0; 4] {
                rule.push("--fanta-padding", layout.padding.map(number).join(" "));
            }
        }
        self.rule(&node.id.to_string(), rule);
    }

    /// Opacity, blend mode and filter effects for one node — the node-level
    /// CSS shared by real elements (`design.css` rules) and derived instance
    /// content (inline styles; its ids never bind, so literals come out).
    fn node_level_declarations(&mut self, node: &CanvasNode, rule: &mut Declarations) {
        let opacity = f64::from(node.opacity.get());
        if opacity < 1.0 {
            rule.push_bound(self, node, BoundProp::Opacity, "opacity", number(opacity));
        }
        if !node.blend_mode.is_normal() {
            match css_blend_mode(node.blend_mode) {
                Some(mode) => rule.push("mix-blend-mode", mode.to_string()),
                None => self.approximate(node, "blend mode has no CSS equivalent"),
            }
        }
        let mut filters = Vec::new();
        for shadow in &node.effects {
            if matches!(shadow.kind, ShadowKind::Drop) {
                if shadow.spread != 0.0 {
                    self.approximate(node, "shadow spread is dropped");
                }
                filters.push(format!(
                    "drop-shadow({}px {}px {}px {})",
                    number(shadow.offset[0]),
                    number(shadow.offset[1]),
                    number(shadow.blur),
                    css_color(shadow.color),
                ));
            } else {
                self.approximate(node, "inner shadows are dropped");
            }
        }
        for blur in &node.blurs {
            if matches!(blur.kind, BlurKind::Layer) {
                filters.push(format!("blur({}px)", number(blur.radius)));
            } else {
                self.approximate(node, "background blur is dropped");
            }
        }
        if !filters.is_empty() {
            rule.push("filter", filters.join(" "));
        }
    }

    /// Emit a group's children, folding mask nodes into SVG `<mask>`: a mask
    /// node clips its following siblings, so they nest in a
    /// `<g mask="url(#…)">` wrapper (one per mask; later masks nest deeper).
    fn emit_children(&mut self, id: NodeId, out: &mut String, depth: usize) -> Result<()> {
        let mut open_masks = 0usize;
        for child in self.resolved.scene.children_of(Some(id)).to_vec() {
            let child_depth = depth + 1 + open_masks;
            let is_mask = self.resolved.scene.get(child).is_some_and(|child_node| {
                child_node.is_mask && !child_node.flags.contains(NodeFlags::HIDDEN)
            });
            if is_mask {
                indent(out, child_depth);
                let _ = writeln!(out, "<mask id=\"{child}-mask\" style=\"mask-type:alpha\">");
                self.emit_node(child, out, child_depth + 1)?;
                indent(out, child_depth);
                out.push_str("</mask>\n");
                indent(out, child_depth);
                let _ = writeln!(out, "<g mask=\"url(#{child}-mask)\">");
                open_masks += 1;
            } else {
                self.emit_node(child, out, child_depth)?;
            }
        }
        for closing in (0..open_masks).rev() {
            indent(out, depth + 1 + closing);
            out.push_str("</g>\n");
        }
        Ok(())
    }

    /// Emit a vector-shaped node: the base element carries the geometry, the
    /// first fill and the strokes; additional fills stack as `-f<n>` clones of
    /// the same geometry. A paint outside the subset rasterizes the node.
    fn emit_vector(
        &mut self,
        node: &CanvasNode,
        vector: &VectorNode,
        out: &mut String,
        depth: usize,
    ) -> Result<()> {
        let paintable = vector.fills.iter().all(paint_expressible)
            && vector
                .strokes
                .first()
                .is_none_or(|stroke| paint_expressible(&stroke.paint));
        if !paintable {
            return self.raster_fallback(node, out, depth, "paints beyond the SVG subset");
        }
        let id = node.id;
        let radius = vector
            .corner_radii
            .map(|radii| radii[0])
            .or(vector.corner_radius);
        if vector
            .corner_radii
            .is_some_and(|radii| radii.windows(2).any(|pair| pair[0] != pair[1]))
        {
            self.approximate(node, "mixed corner radii use the first corner");
        }
        let (tag, geometry) = match vector.path.is_rect().then(|| rect_of(&vector.path)).flatten()
        {
            Some((x, y, width, height)) => (
                "rect",
                format!(
                    " x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    number(x),
                    number(y),
                    number(width),
                    number(height)
                ),
            ),
            None => ("path", format!(" d=\"{}\"", vector.path.to_svg_d())),
        };

        let base_id = id.to_string();
        let mut rule = Declarations::default();
        match vector.fills.first() {
            Some(fill) => match self.paint_value(node, fill, 0, &base_id) {
                Some(paint) => rule.push("fill", paint),
                None => rule.push("fill", "none".to_string()),
            },
            None => rule.push("fill", "none".to_string()),
        }
        self.stroke_declarations(node, &vector.strokes, &base_id, &mut rule);
        if tag == "rect"
            && let Some(radius) = radius
        {
            rule.push_bound(self, node, BoundProp::CornerRadius, "rx", number(radius));
        }
        self.start_element(node, out, depth, tag);
        out.push_str(&geometry);
        out.push_str("/>\n");
        self.rule(&base_id, rule);

        for (index, fill) in vector.fills.iter().enumerate().skip(1) {
            let element_id = format!("{id}-f{index}");
            let mut rule = Declarations::default();
            match self.paint_value(node, fill, index, &element_id) {
                Some(paint) => rule.push("fill", paint),
                None => rule.push("fill", "none".to_string()),
            }
            if tag == "rect"
                && let Some(radius) = radius
            {
                rule.push("rx", number(radius));
            }
            indent(out, depth);
            let _ = writeln!(
                out,
                "<{tag} id=\"{element_id}\"{}{geometry}/>",
                self.shared_attrs(node),
            );
            self.rule(&element_id, rule);
        }
        Ok(())
    }

    /// Expand an instance and emit its derived content: same drawing
    /// vocabulary, but no element ids (expansion mints fresh ids every call)
    /// and inline styles instead of `design.css` rules, so the output stays
    /// byte-deterministic and invisible to the loader.
    fn emit_expansion(
        &mut self,
        node: &CanvasNode,
        instance: &InstanceNode,
        out: &mut String,
        depth: usize,
    ) -> Result<()> {
        let expanded = expand_instance(&self.resolved.scene, &self.resolved.components, instance);
        if expanded.is_empty() {
            self.approximate(node, "the instance's component has no content");
            return Ok(());
        }
        let nodes: Vec<CanvasNode> = expanded.into_iter().map(|entry| entry.node).collect();
        let ids: std::collections::BTreeSet<NodeId> =
            nodes.iter().map(|virtual_node| virtual_node.id).collect();
        let mut children: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
        let mut roots = Vec::new();
        for (position, virtual_node) in nodes.iter().enumerate() {
            match virtual_node.parent {
                Some(parent) if ids.contains(&parent) => {
                    children.entry(parent).or_default().push(position);
                }
                _ => roots.push(position),
            }
        }
        let instance_display = node.id.to_string();
        for root in roots {
            self.emit_virtual(&nodes, root, &children, &instance_display, out, depth)?;
        }
        Ok(())
    }

    fn emit_virtual(
        &mut self,
        nodes: &[CanvasNode],
        position: usize,
        children: &BTreeMap<NodeId, Vec<usize>>,
        instance: &str,
        out: &mut String,
        depth: usize,
    ) -> Result<()> {
        let node = &nodes[position];
        if node.flags.contains(NodeFlags::HIDDEN) {
            return Ok(());
        }
        let mut level = Declarations::default();
        self.node_level_declarations(node, &mut level);
        let transform = transform_attribute(&node.transform)
            .map(|transform| format!(" transform=\"{transform}\""))
            .unwrap_or_default();
        let level_inline = level.inline();
        let style = match level.0.is_empty() {
            true => String::new(),
            false => format!(" style=\"{level_inline}\""),
        };
        match &node.data {
            NodeData::Group(group) => {
                let clip = group.clip_size.map(|[width, height]| {
                    self.virtual_defs += 1;
                    let clip_id = format!("{instance}-v{}-clip", self.virtual_defs);
                    let _ = writeln!(
                        self.defs,
                        "    <clipPath id=\"{clip_id}\"><rect width=\"{}\" height=\"{}\"/></clipPath>",
                        number(width),
                        number(height),
                    );
                    format!(" clip-path=\"url(#{clip_id})\"")
                });
                indent(out, depth);
                let _ = writeln!(
                    out,
                    "<g data-virtual=\"true\"{transform}{}{style}>",
                    clip.unwrap_or_default(),
                );
                if let Some([width, height]) = group.clip_size.or(group.local_size) {
                    for fill in group.background.iter().chain(group.background_fills.iter()) {
                        let Some(paint) = self.virtual_paint(fill, instance) else {
                            self.approximate(
                                node,
                                "a paint beyond the SVG subset inside an instance is dropped",
                            );
                            continue;
                        };
                        let radius = group
                            .corner_radii
                            .map(|radii| radii[0])
                            .or(group.corner_radius)
                            .map(|radius| format!(" rx=\"{}\"", number(radius)))
                            .unwrap_or_default();
                        indent(out, depth + 1);
                        let _ = writeln!(
                            out,
                            "<rect width=\"{}\" height=\"{}\"{radius} style=\"fill: {paint}\"/>",
                            number(width),
                            number(height),
                        );
                    }
                }
                let mut open_masks = 0usize;
                for child in children.get(&node.id).map_or(&[][..], Vec::as_slice) {
                    let child_depth = depth + 1 + open_masks;
                    let child_node = &nodes[*child];
                    if child_node.is_mask && !child_node.flags.contains(NodeFlags::HIDDEN) {
                        self.virtual_defs += 1;
                        let mask_id = format!("{instance}-v{}-mask", self.virtual_defs);
                        indent(out, child_depth);
                        let _ =
                            writeln!(out, "<mask id=\"{mask_id}\" style=\"mask-type:alpha\">");
                        self.emit_virtual(nodes, *child, children, instance, out, child_depth + 1)?;
                        indent(out, child_depth);
                        out.push_str("</mask>\n");
                        indent(out, child_depth);
                        let _ = writeln!(out, "<g mask=\"url(#{mask_id})\">");
                        open_masks += 1;
                    } else {
                        self.emit_virtual(nodes, *child, children, instance, out, child_depth)?;
                    }
                }
                for closing in (0..open_masks).rev() {
                    indent(out, depth + 1 + closing);
                    out.push_str("</g>\n");
                }
                indent(out, depth);
                out.push_str("</g>\n");
            }
            NodeData::Vector(vector) => {
                self.emit_virtual_vector(
                    node,
                    vector,
                    instance,
                    &transform,
                    &level_inline,
                    out,
                    depth,
                );
            }
            NodeData::Boolean(boolean) => match &boolean.baked {
                Some(baked) => {
                    let vector = VectorNode {
                        path: baked.vector.path.clone(),
                        fills: boolean.fills.clone(),
                        strokes: boolean.strokes.clone(),
                        ..VectorNode::default()
                    };
                    self.emit_virtual_vector(
                        node,
                        &vector,
                        instance,
                        &transform,
                        &level_inline,
                        out,
                        depth,
                    );
                }
                None => {
                    self.approximate(node, "an unbaked boolean inside an instance is dropped")
                }
            },
            NodeData::Instance(nested) => {
                indent(out, depth);
                let _ = writeln!(out, "<g data-virtual=\"true\"{transform}{style}>");
                let nested = nested.clone();
                self.emit_expansion(node, &nested, out, depth + 1)?;
                indent(out, depth);
                out.push_str("</g>\n");
            }
            NodeData::Text(text) => {
                let text_style = &text.style;
                let mut declarations = level;
                declarations.push("font-family", css_font_family(&text_style.font_family));
                declarations.push("font-size", format!("{}px", number(text_style.size_px)));
                if text_style.weight != 400 {
                    declarations.push("font-weight", text_style.weight.to_string());
                }
                declarations.push("fill", css_color(text_style.color));
                let (anchor, x) = match text.align {
                    TextAlign::Left => (None, 0.0),
                    TextAlign::Center => (Some("middle"), text.local_size[0] / 2.0),
                    TextAlign::Right => (Some("end"), text.local_size[0]),
                    _ => (None, 0.0),
                };
                if let Some(anchor) = anchor {
                    declarations.push("text-anchor", anchor.to_string());
                }
                indent(out, depth);
                let _ = writeln!(
                    out,
                    "<text data-virtual=\"true\"{transform} y=\"{}\" style=\"{}\">",
                    number(text_style.size_px),
                    declarations.inline(),
                );
                let line_height = text_style.size_px * line_height_factor(text_style);
                for (index, line) in text.content.split('\n').enumerate() {
                    indent(out, depth + 1);
                    let _ = writeln!(
                        out,
                        "<tspan x=\"{}\"{}>{}</tspan>",
                        number(x),
                        if index == 0 {
                            String::new()
                        } else {
                            format!(" dy=\"{}\"", number(line_height))
                        },
                        xml_escape(line),
                    );
                }
                indent(out, depth);
                out.push_str("</text>\n");
            }
            NodeData::Bitmap(bitmap) => {
                if bitmap.crop.is_some() {
                    self.approximate(node, "a cropped image inside an instance is dropped");
                    return Ok(());
                }
                let href = match self.asset_hrefs.get(&bitmap.asset) {
                    Some(href) => href.clone(),
                    None => {
                        self.approximate(node, "image asset has no file under assets/");
                        format!("{ASSETS_DIR}/{}", bitmap.asset)
                    }
                };
                indent(out, depth);
                let _ = writeln!(
                    out,
                    "<image data-virtual=\"true\"{transform}{style} href=\"{href}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\"/>",
                    number(bitmap.local_size[0]),
                    number(bitmap.local_size[1]),
                );
            }
            _ => self.approximate(
                node,
                "content beyond the SVG subset inside an instance is dropped",
            ),
        }
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "internal assembly step")]
    fn emit_virtual_vector(
        &mut self,
        node: &CanvasNode,
        vector: &VectorNode,
        instance: &str,
        transform: &str,
        level: &str,
        out: &mut String,
        depth: usize,
    ) {
        let radius = vector
            .corner_radii
            .map(|radii| radii[0])
            .or(vector.corner_radius);
        let (tag, geometry) = match vector.path.is_rect().then(|| rect_of(&vector.path)).flatten()
        {
            Some((x, y, width, height)) => (
                "rect",
                format!(
                    " x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{}",
                    number(x),
                    number(y),
                    number(width),
                    number(height),
                    radius
                        .map(|radius| format!(" rx=\"{}\"", number(radius)))
                        .unwrap_or_default(),
                ),
            ),
            None => ("path", format!(" d=\"{}\"", vector.path.to_svg_d())),
        };
        let mut paints = Vec::new();
        for fill in &vector.fills {
            match self.virtual_paint(fill, instance) {
                Some(paint) => paints.push(paint),
                None => self.approximate(
                    node,
                    "a paint beyond the SVG subset inside an instance is dropped",
                ),
            }
        }
        if paints.is_empty() {
            paints.push("none".to_string());
        }
        let stroke = vector.strokes.first().and_then(|stroke| {
            let paint = self.virtual_paint(&stroke.paint, instance)?;
            Some(format!(
                "; stroke: {paint}; stroke-width: {}",
                number(stroke.width)
            ))
        });
        for (index, paint) in paints.iter().enumerate() {
            let stroke_part = if index == 0 {
                stroke.as_deref().unwrap_or_default()
            } else {
                ""
            };
            // The level declarations (opacity, filter, blend) share the one
            // style attribute with the fill: a second style="" on the same
            // element is malformed XML.
            let level_part = if index == 0 && !level.is_empty() {
                format!("{level}; ")
            } else {
                String::new()
            };
            indent(out, depth);
            let _ = writeln!(
                out,
                "<{tag} data-virtual=\"true\"{transform}{geometry} style=\"{level_part}fill: {paint}{stroke_part}\"/>",
            );
        }
    }

    /// The CSS paint for derived content: literals only (bindings resolve
    /// before expansion), gradients through counter-keyed defs.
    fn virtual_paint(&mut self, fill: &Fill, instance: &str) -> Option<String> {
        match fill {
            Fill::Solid { color, .. } => Some(css_color(*color)),
            Fill::Gradient { gradient, .. } => {
                self.virtual_defs += 1;
                let def_id = format!("{instance}-v{}-grad", self.virtual_defs);
                self.gradient_def(&def_id, gradient)
            }
            _ => None,
        }
    }

    /// The CSS paint for one fill of a real element: a literal or `var()` for
    /// solids, a `url(#…)` def reference for expressible gradients, `None`
    /// beyond the subset.
    fn paint_value(
        &mut self,
        node: &CanvasNode,
        fill: &Fill,
        index: usize,
        element_id: &str,
    ) -> Option<String> {
        match fill {
            Fill::Solid { color, blend } => {
                if !blend.is_normal() {
                    self.approximate(node, "per-paint blend modes are dropped");
                }
                let literal = css_color(*color);
                let index = u16::try_from(index).unwrap_or(u16::MAX);
                Some(
                    match self.bound_variable(node, BoundProp::FillColor { index }) {
                        Some(token) => format!("var({token}, {literal})"),
                        None => literal,
                    },
                )
            }
            Fill::Gradient { gradient, .. } => self.gradient_def(&format!("{element_id}-grad"), gradient),
            _ => None,
        }
    }

    /// Emit a gradient def and return its `url(#…)` paint. Coordinates are
    /// node-local 0–1, which is SVG's default `objectBoundingBox` space.
    /// Linear and axis-aligned radial map to SVG; angular, diamond and
    /// handle-rotated radial do not and return `None`.
    fn gradient_def(&mut self, def_id: &str, gradient: &Gradient) -> Option<String> {
        match gradient {
            Gradient::Linear { start, end, stops } => {
                let _ = writeln!(
                    self.defs,
                    "    <linearGradient id=\"{def_id}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">",
                    number(f64::from(start[0])),
                    number(f64::from(start[1])),
                    number(f64::from(end[0])),
                    number(f64::from(end[1])),
                );
                write_stops(&mut self.defs, stops);
                self.defs.push_str("    </linearGradient>\n");
            }
            Gradient::Radial {
                center,
                radius,
                handles: None,
                stops,
            } => {
                let _ = writeln!(
                    self.defs,
                    "    <radialGradient id=\"{def_id}\" cx=\"{}\" cy=\"{}\" r=\"{}\">",
                    number(f64::from(center[0])),
                    number(f64::from(center[1])),
                    number(f64::from(*radius)),
                );
                write_stops(&mut self.defs, stops);
                self.defs.push_str("    </radialGradient>\n");
            }
            _ => return None,
        }
        Some(format!("url(#{def_id})"))
    }

    /// ` transform=".." clip-path=".."` — shared by a node's base element and
    /// its stacked-fill clones.
    fn shared_attrs(&self, node: &CanvasNode) -> String {
        let mut attrs = String::new();
        if let Some(transform) = transform_attribute(&node.transform) {
            let _ = write!(attrs, " transform=\"{transform}\"");
        }
        if let Some(parent) = node.parent {
            let parent_clips = self
                .resolved
                .scene
                .get(parent)
                .is_some_and(|parent_node| match &parent_node.data {
                    NodeData::Group(group) => group.clip_size.is_some(),
                    _ => false,
                });
            if parent_clips {
                let _ = write!(attrs, " clip-path=\"url(#{parent}-clip)\"");
            }
        }
        attrs
    }

    fn stroke_declarations(
        &mut self,
        node: &CanvasNode,
        strokes: &[Stroke],
        element_id: &str,
        rule: &mut Declarations,
    ) {
        let Some(stroke) = strokes.first() else {
            return;
        };
        if strokes.len() > 1 {
            self.approximate(node, "stacked strokes keep the first");
        }
        match &stroke.paint {
            Fill::Solid { color, .. } => {
                let literal = css_color(*color);
                match self.bound_variable(node, BoundProp::StrokeColor { index: 0 }) {
                    Some(token) => rule.push("stroke", format!("var({token}, {literal})")),
                    None => rule.push("stroke", literal),
                }
            }
            Fill::Gradient { gradient, .. } => {
                match self.gradient_def(&format!("{element_id}-stroke-grad"), gradient) {
                    Some(paint) => rule.push("stroke", paint),
                    None => {
                        self.approximate(node, "a stroke gradient beyond the SVG subset is dropped");
                        return;
                    }
                }
            }
            _ => {
                self.approximate(node, "non-solid strokes are dropped");
                return;
            }
        }
        rule.push_bound(
            self,
            node,
            BoundProp::StrokeWidth { index: 0 },
            "stroke-width",
            number(stroke.width),
        );
        if !stroke.dash.is_empty() {
            rule.push(
                "stroke-dasharray",
                stroke
                    .dash
                    .iter()
                    .map(|dash| number(*dash))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }

    /// `--token` name for `prop` when the authored node binds it.
    fn bound_variable(&self, node: &CanvasNode, prop: BoundProp) -> Option<String> {
        let authored = self.authored.scene.get(node.id)?;
        let variable = authored.bindings.get(&prop)?;
        let variable = self.registry.variable(*variable)?;
        let collection = self.registry.collections.get(&variable.collection)?;
        Some(format!(
            "--{}-{}",
            slugify(&collection.name, "collection"),
            slugify(&variable.name, "variable")
        ))
    }

    fn raster_fallback(
        &mut self,
        node: &CanvasNode,
        out: &mut String,
        depth: usize,
        what: &str,
    ) -> Result<()> {
        let Some(raster) = self.render.and_then(|render| render.raster(node.id)) else {
            self.approximate(node, &format!("not drawn: {what}, and no renderer is available"));
            return Ok(());
        };
        self.approximate(node, &format!("rasterized: {what}"));
        // The PNG covers the node's visual bounds in world space; place it in
        // the parent's space through the inverse of the parent's world
        // transform.
        let parent_world = node
            .parent
            .and_then(|parent| self.resolved.scene.world_transform(parent))
            .unwrap_or(Transform2D::IDENTITY);
        if !is_axis_aligned(&parent_world) {
            self.approximate(node, "rasterized under a rotated ancestor; placement is approximate");
        }
        let inverse = parent_world.0.inverse();
        let low = inverse.transform_point2(glam::DVec2::new(
            raster.bounds.min_x,
            raster.bounds.min_y,
        ));
        let high = inverse.transform_point2(glam::DVec2::new(
            raster.bounds.max_x,
            raster.bounds.max_y,
        ));
        let file = format!("{}.png", node.id);
        indent(out, depth);
        let _ = writeln!(
            out,
            "<image id=\"{}\" data-name=\"{}\" href=\"{}/{file}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
            node.id,
            xml_escape(&node.name),
            self.raster_dir,
            number(low.x.min(high.x)),
            number(low.y.min(high.y)),
            number((high.x - low.x).abs()),
            number((high.y - low.y).abs()),
        );
        self.rasters.push((file, raster.bytes));
        Ok(())
    }

    fn approximate(&mut self, node: &CanvasNode, what: &str) {
        self.approximations.push(Approximation {
            node: node.id.to_string(),
            name: node.name.clone(),
            what: what.to_string(),
        });
    }

    fn rule(&mut self, element_id: &str, declarations: Declarations) {
        if declarations.0.is_empty() {
            return;
        }
        let _ = writeln!(self.css, "#{element_id} {{");
        for (property, value) in declarations.0 {
            let _ = writeln!(self.css, "  {property}: {value};");
        }
        let _ = writeln!(self.css, "}}");
    }
}

#[derive(Default)]
struct Declarations(Vec<(&'static str, String)>);

impl Declarations {
    fn push(&mut self, property: &'static str, value: String) {
        self.0.push((property, value));
    }

    /// Pushes `property: value`, wrapped in `var(--token, value)` when the
    /// authored node binds `prop` to a variable.
    fn push_bound(
        &mut self,
        emitter: &Emitter<'_>,
        node: &CanvasNode,
        prop: BoundProp,
        property: &'static str,
        literal: String,
    ) {
        match emitter.bound_variable(node, prop) {
            Some(token) => self.push(property, format!("var({token}, {literal})")),
            None => self.push(property, literal),
        }
    }

    /// `p1: v1; p2: v2` — the inline-style form for derived content.
    fn inline(&self) -> String {
        self.0
            .iter()
            .map(|(property, value)| format!("{property}: {value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Whether a paint stays vector in the projection: solid, or a gradient SVG
/// can express.
fn paint_expressible(fill: &Fill) -> bool {
    match fill {
        Fill::Solid { .. } => true,
        Fill::Gradient { gradient, .. } => matches!(
            gradient,
            Gradient::Linear { .. } | Gradient::Radial { handles: None, .. }
        ),
        _ => false,
    }
}

fn write_stops(defs: &mut String, stops: &[GradientStop]) {
    for stop in stops {
        let _ = write!(
            defs,
            "      <stop offset=\"{}\" stop-color=\"#{:02X}{:02X}{:02X}\"",
            number(f64::from(stop.position)),
            stop.color.r,
            stop.color.g,
            stop.color.b,
        );
        if stop.color.a != 255 {
            let _ = write!(
                defs,
                " stop-opacity=\"{}\"",
                number(f64::from(stop.color.a) / 255.0)
            );
        }
        defs.push_str("/>\n");
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
        let collection_slug = slugify(&collection.name, "collection");
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
                            slugify(&variable.name, "variable")
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
                slugify(&mode.name, "mode")
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
) -> std::result::Result<String, String> {
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
                        .ok_or_else(|| {
                            format!("alias into a missing collection from {}", current.name)
                        })?
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

fn rect_of(path: &PathData) -> Option<(f64, f64, f64, f64)> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for segment in &path.segments {
        let point = match segment {
            PathSegment::Move { to } | PathSegment::Line { to } => to,
            PathSegment::Close => continue,
            _ => return None,
        };
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    (min[0].is_finite() && max[0] > min[0] && max[1] > min[1])
        .then(|| (min[0], min[1], max[0] - min[0], max[1] - min[1]))
}

fn transform_attribute(transform: &Transform2D) -> Option<String> {
    let affine = transform.0;
    if affine == glam::DAffine2::IDENTITY {
        return None;
    }
    let x_axis = affine.matrix2.x_axis;
    let y_axis = affine.matrix2.y_axis;
    if x_axis.x == 1.0 && x_axis.y == 0.0 && y_axis.x == 0.0 && y_axis.y == 1.0 {
        return Some(format!(
            "translate({} {})",
            number(affine.translation.x),
            number(affine.translation.y)
        ));
    }
    Some(format!(
        "matrix({} {} {} {} {} {})",
        number(x_axis.x),
        number(x_axis.y),
        number(y_axis.x),
        number(y_axis.y),
        number(affine.translation.x),
        number(affine.translation.y),
    ))
}

fn is_axis_aligned(transform: &Transform2D) -> bool {
    let matrix = transform.0.matrix2;
    matrix.x_axis.y == 0.0 && matrix.y_axis.x == 0.0
}

fn line_height_factor(style: &TextStyle) -> f64 {
    if style.line_height > 0.0 {
        style.line_height
    } else {
        1.2
    }
}

fn unsupported_kind(data: &NodeData) -> &'static str {
    match data {
        NodeData::TextPath(_) => "text on a path is rasterized for now",
        NodeData::Video(_) | NodeData::Audio(_) => "media nodes export their poster",
        _ => "this node kind is rasterized for now",
    }
}

pub(crate) fn css_blend_mode(mode: BlendMode) -> Option<&'static str> {
    Some(match mode {
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Overlay => "overlay",
        BlendMode::Darken => "darken",
        BlendMode::Lighten => "lighten",
        BlendMode::ColorDodge => "color-dodge",
        BlendMode::ColorBurn => "color-burn",
        BlendMode::HardLight => "hard-light",
        BlendMode::SoftLight => "soft-light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
        _ => return None,
    })
}

pub(crate) fn css_color(color: Color) -> String {
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

fn css_font_family(value: &str) -> String {
    // Single quotes: this value also lands in inline style="…" attributes,
    // where a double quote breaks the SVG's XML well-formedness.
    format!("'{}'", value.replace(['"', '\''], ""))
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Shortest-round-trip float formatting; deterministic across platforms.
pub(crate) fn number(value: f64) -> String {
    format!("{value}")
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}
