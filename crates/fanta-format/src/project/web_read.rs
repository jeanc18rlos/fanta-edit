//! Loading hand edits from the web projection (spec 0007 T3, R7): the SVG and
//! CSS are authoritative for what they express, so fields parsed from them
//! win over the `nodes.json` records. Parsing is tolerant of formatting — the
//! next save canonicalizes. An edit the loader cannot keep is reported per
//! node through the loader's warnings (which the next save writes into
//! `report.json`); nothing is silently dropped.
//!
//! Only differences change the document: a pristine projection loads with no
//! record touched and no warning, so load-then-save stays byte-identical.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use fanta_doc::{
    BlendMode, BlurKind, BoundProp, CanvasNode, Color, Fill, NodeData, NodeId, PathData,
    ShadowKind, TextAlign, Transform2D, UnitInterval, VariableId,
};
use serde_json::{Map, Value};

use crate::error::Result;

use super::layout::{COMPONENTS_DIR, PAGES_DIR, id_from_key, json_key, slugify};
use super::diagnostic::{
    EDIT_NOT_KEPT, EDIT_SELECTOR_OUTSIDE_DIALECT, ID_NOT_A_NODE, PARSE_SVG, RECORD_MISSING,
    RECORD_UNREADABLE, REF_AMBIGUOUS_TOKEN, REF_MISSING_TOKEN, SourceDiagnostic,
};
use super::tree_source::TreeSource;
use super::web::{DESIGN_CSS, MASTER_SVG, PAGE_SVG, css_blend_mode, css_color, number};

/// Parse the project's `design.css` and every design SVG, and apply what they
/// say onto the node records. `variables` is the registry JSON (for `var()`
/// token lookup); groups whose auto-layout intent changed land in
/// `pending_layout` (with their children) so the solver regenerates the
/// derived geometry (R8).
pub(crate) fn apply_web_overlay(
    source: &TreeSource<'_>,
    dir: &Path,
    nodes: &mut Map<String, Value>,
    variables: &Value,
    pending_layout: &mut BTreeSet<NodeId>,
    warnings: &mut Vec<SourceDiagnostic>,
    unavailable: &BTreeSet<PathBuf>,
) -> Result<()> {
    let tokens = TokenTable::new(variables, warnings);
    let mut rules = match source.read_to_string(&dir.join(DESIGN_CSS)) {
        Ok(css) => parse_rules(&css, DESIGN_CSS, warnings),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error.into()),
    };
    let mut elements = BTreeMap::new();
    // (project-relative name, path) of each design SVG; `SvgElement::file`
    // indexes it.
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for path in design_svgs(source, dir)? {
        if path.parent().is_some_and(|design| unavailable.contains(design)) {
            continue;
        }
        let svg = source.read_to_string(&path)?;
        let context = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let file = u32::try_from(files.len()).unwrap_or(u32::MAX);
        parse_design_svg(&svg, &context, file, &mut elements, warnings);
        let scoped_context = Path::new(&context)
            .with_extension("css")
            .to_string_lossy()
            .into_owned();
        files.push((context, path.clone()));
        // The design's own stylesheet wins over a duplicate declaration in
        // the root design.css (scoped is more specific — spec 0007 R3 as
        // amended), but a root-only declaration still applies: append the
        // scoped declarations so the later value wins per property.
        let scoped = path.with_extension("css");
        match source.read_to_string(&scoped) {
            Ok(css) => {
                for (id, declarations) in parse_rules(&css, &scoped_context, warnings) {
                    rules.entry(id).or_default().extend(declarations);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    let mut ids: BTreeSet<&String> = elements.keys().collect();
    ids.extend(rules.keys());
    let mut relayout = Vec::new();
    for raw in ids {
        let (display, slot) = split_slot(raw);
        if matches!(slot, Slot::Clip | Slot::Skip) {
            continue;
        }
        let Ok(id) = display.parse::<NodeId>() else {
            let finding = SourceDiagnostic::warning(
                ID_NOT_A_NODE,
                format!("#{raw}: the id does not name a node; its rule is regenerated on save"),
            );
            warnings.push(locate(finding, raw, elements.get(raw), &files, source));
            continue;
        };
        let key = json_key(&id)?;
        let Some(record) = nodes.get(&key) else {
            let finding = SourceDiagnostic::warning(
                RECORD_MISSING,
                format!("{display}: appears in the projection but not in nodes.json; ignored"),
            )
            .for_node(display);
            warnings.push(locate(finding, raw, elements.get(raw), &files, source));
            continue;
        };
        let mut node: CanvasNode = match serde_json::from_value(record.clone()) {
            Ok(node) => node,
            Err(error) => {
                let finding = SourceDiagnostic::warning(
                    RECORD_UNREADABLE,
                    format!("{display}: unreadable record: {error}"),
                )
                .for_node(display);
                warnings.push(locate(finding, raw, elements.get(raw), &files, source));
                continue;
            }
        };
        let reported = warnings.len();
        let mut edit = Edit {
            node: &mut node,
            display,
            changed: false,
            layout_changed: false,
            tokens: &tokens,
            warnings,
        };
        if let Some(element) = elements.get(raw) {
            edit.apply_svg(&slot, element);
        }
        if let Some(declarations) = rules.get(raw) {
            for (property, value) in declarations {
                edit.apply_declaration(&slot, property, value);
            }
        }
        let (changed, layout_changed) = (edit.changed, edit.layout_changed);
        if warnings.len() > reported {
            let located: Vec<_> = warnings
                .drain(reported..)
                .map(|finding| {
                    locate(finding.for_node(display), raw, elements.get(raw), &files, source)
                })
                .collect();
            warnings.extend(located);
        }
        if layout_changed {
            relayout.push(id);
        }
        if changed {
            nodes.insert(key, serde_json::to_value(&node)?);
        }
    }

    // R8: regenerate the derived geometry under every group whose intent
    // changed — the stale solved coordinates in the SVG must not win.
    if !relayout.is_empty() {
        let parents: BTreeSet<String> = relayout
            .iter()
            .map(json_key)
            .collect::<Result<_>>()?;
        for (key, record) in nodes.iter() {
            let parent_inside = record
                .get("parent")
                .and_then(Value::as_str)
                .is_some_and(|parent| parents.contains(parent));
            if parent_inside {
                pending_layout.insert(id_from_key::<NodeId>(key)?);
            }
        }
        pending_layout.extend(relayout);
    }
    Ok(())
}

/// What an element id addresses: the node itself, one of a frame's background
/// rects, a stacked-fill clone element, or a projection-only def (clip path,
/// mask, gradient).
enum Slot {
    Node,
    Background(usize),
    Fill(usize),
    Clip,
    /// Mask and gradient defs: regenerated on save, never node edits.
    Skip,
}

fn split_slot(raw: &str) -> (&str, Slot) {
    if raw.ends_with("-mask") || raw.ends_with("-grad") {
        return (raw, Slot::Skip);
    }
    if let Some(display) = raw.strip_suffix("-clip") {
        return (display, Slot::Clip);
    }
    if let Some(display) = raw.strip_suffix("-bg") {
        return (display, Slot::Background(0));
    }
    if let Some(position) = raw.rfind("-bg") {
        let (display, index) = raw.split_at(position);
        if let Ok(index) = index["-bg".len()..].parse::<usize>() {
            return (display, Slot::Background(index));
        }
    }
    if let Some(position) = raw.rfind("-f") {
        let (display, index) = raw.split_at(position);
        if let Ok(index) = index["-f".len()..].parse::<usize>() {
            return (display, Slot::Fill(index));
        }
    }
    (raw, Slot::Node)
}

/// Say where a finding about the element with raw id `raw` sits: its SVG
/// file and the position of its `id` attribute. The file is read again here
/// because findings are rare and tracking a position for every element would
/// tax every load.
fn locate(
    finding: SourceDiagnostic,
    raw: &str,
    element: Option<&SvgElement>,
    files: &[(String, PathBuf)],
    source: &TreeSource<'_>,
) -> SourceDiagnostic {
    let Some((context, path)) = element.and_then(|element| files.get(element.file as usize))
    else {
        return finding;
    };
    let finding = finding.in_file(context.clone());
    let Ok(text) = source.read_to_string(path) else {
        return finding;
    };
    match text.find(&format!("id=\"{raw}\"")) {
        Some(offset) => finding.at_offset(&text, offset),
        None => finding,
    }
}

/// One parsed SVG element, keyed by its raw `id`.
#[derive(Default)]
struct SvgElement {
    /// Index into the overlay's list of design SVG files.
    file: u32,
    tag: String,
    name: Option<String>,
    transform: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    d: Option<String>,
    href: Option<String>,
    /// `<tspan>` contents of a `<text>` element, one entry per line.
    lines: Option<Vec<String>>,
}

/// Every design SVG in the tree: `pages/*/page.svg` and each `master.svg`
/// under `components/` (variants nest one level deeper).
fn design_svgs(source: &TreeSource<'_>, dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let pages = dir.join(PAGES_DIR);
    if source.is_dir(&pages) {
        for entry in source.sorted_entries(&pages)? {
            let candidate = entry.join(PAGE_SVG);
            if source.is_file(&candidate) {
                found.push(candidate);
            }
        }
    }
    let mut stack = vec![dir.join(COMPONENTS_DIR)];
    while let Some(directory) = stack.pop() {
        if !source.is_dir(&directory) {
            continue;
        }
        for path in source.sorted_entries(&directory)? {
            if source.is_dir(&path) {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == MASTER_SVG) {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// The attributes of a tag that can be read, the first of each name.
/// quick-xml's own duplicate check compares every pair of names, so a tag
/// with very many attributes stalled the load (RUSTSEC-2026-0194).
pub(super) fn first_attributes<'a>(
    start: &'a quick_xml::events::BytesStart<'_>,
) -> impl Iterator<Item = quick_xml::events::attributes::Attribute<'a>> {
    // Tags are small and this runs for every one, so names are compared
    // directly until there are too many for that to stay cheap.
    const SCANNED_NAMES: usize = 32;
    let mut first_names: Vec<&[u8]> = Vec::new();
    let mut later_names = HashSet::new();
    let mut attributes = start.attributes();
    attributes.with_checks(false);
    attributes.flatten().filter(move |attribute| {
        let name = attribute.key.into_inner();
        if first_names.contains(&name) {
            false
        } else if first_names.len() < SCANNED_NAMES {
            first_names.push(name);
            true
        } else {
            later_names.insert(name)
        }
    })
}

fn parse_design_svg(
    src: &str,
    context: &str,
    file: u32,
    elements: &mut BTreeMap<String, SvgElement>,
    warnings: &mut Vec<SourceDiagnostic>,
) {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(src);
    // (text element id, lines) while inside a `<text>`.
    let mut text: Option<(String, Vec<String>)> = None;
    let mut in_tspan = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) | Ok(Event::Empty(start)) => {
                let tag = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                if tag == "tspan" {
                    if let Some((_, lines)) = &mut text {
                        lines.push(String::new());
                        in_tspan = true;
                    }
                    continue;
                }
                let mut element = SvgElement {
                    file,
                    tag: tag.clone(),
                    ..SvgElement::default()
                };
                let mut id = None;
                let mut styled = false;
                for attribute in first_attributes(&start) {
                    let value = match attribute.unescape_value() {
                        Ok(value) => value.into_owned(),
                        Err(_) => continue,
                    };
                    match attribute.key.as_ref() {
                        b"id" => id = Some(value),
                        b"data-name" => element.name = Some(value),
                        b"transform" => element.transform = Some(value),
                        b"x" => element.x = value.trim().parse().ok(),
                        b"y" => element.y = value.trim().parse().ok(),
                        b"width" => element.width = value.trim().parse().ok(),
                        b"height" => element.height = value.trim().parse().ok(),
                        b"d" => element.d = Some(value),
                        b"href" => element.href = Some(value),
                        b"style" => styled = true,
                        _ => {}
                    }
                }
                let Some(id) = id else { continue };
                // The writer never puts `style` on a node's own element, so
                // one there is a hand edit (vector applications fold paint
                // into it on save), and only the stylesheet is read.
                if styled && id.parse::<NodeId>().is_ok() {
                    let finding = SourceDiagnostic::warning(
                        EDIT_NOT_KEPT,
                        format!(
                            "{id}: the style attribute is not read; styling belongs in the design's stylesheet, and the save removes the attribute"
                        ),
                    )
                    .for_node(&id)
                    .in_file(context);
                    warnings.push(match src.find(&format!("id=\"{id}\"")) {
                        Some(offset) => finding.at_offset(src, offset),
                        None => finding,
                    });
                }
                if tag == "text" {
                    text = Some((id.clone(), Vec::new()));
                }
                elements.insert(id, element);
            }
            Ok(Event::Text(content)) => {
                if in_tspan
                    && let Some((_, lines)) = &mut text
                    && let Some(line) = lines.last_mut()
                    && let Ok(content) = content.xml10_content()
                {
                    line.push_str(&content);
                }
            }
            Ok(Event::End(end)) => match end.name().as_ref() {
                b"tspan" => in_tspan = false,
                b"text" => {
                    if let Some((id, lines)) = text.take()
                        && let Some(element) = elements.get_mut(&id)
                    {
                        element.lines = Some(lines);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => {
                let offset = usize::try_from(reader.error_position()).unwrap_or(src.len());
                warnings.push(
                    SourceDiagnostic::warning(
                        PARSE_SVG,
                        format!(
                            "{context}: SVG parse error ({error}); edits beyond this point are ignored"
                        ),
                    )
                    .in_file(context)
                    .at_offset(src, offset),
                );
                break;
            }
            Ok(_) => {}
        }
    }
}

/// Tolerant CSS: comments stripped, blocks split on braces, one `#id`
/// selector per rule (the emitted dialect). Non-id selectors (`:root`, mode
/// classes) are skipped; a grouped selector is reported, not guessed at.
fn parse_rules(
    css: &str,
    context: &str,
    warnings: &mut Vec<SourceDiagnostic>,
) -> BTreeMap<String, Vec<(String, String)>> {
    let mut source = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        source.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    source.push_str(rest);

    let mut rules: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for block in source.split('}') {
        let Some((selector, body)) = block.split_once('{') else {
            continue;
        };
        // Statements before the selector (the aggregator's `@import` lines)
        // end with `;`; the selector is what follows the last one.
        let selector = selector.rsplit(';').next().unwrap_or(selector).trim();
        let Some(id) = selector.strip_prefix('#') else {
            continue;
        };
        if id.contains([',', ' ', '.', ':', '#']) {
            let finding = SourceDiagnostic::warning(
                EDIT_SELECTOR_OUTSIDE_DIALECT,
                format!(
                    "selector {selector:?} is outside the emitted dialect; its rule is regenerated on save"
                ),
            )
            .in_file(context);
            warnings.push(match css.find(selector) {
                Some(offset) => finding.at_offset(css, offset),
                None => finding,
            });
            continue;
        }
        let declarations = rules.entry(id.to_string()).or_default();
        for declaration in body.split(';') {
            let Some((property, value)) = declaration.split_once(':') else {
                continue;
            };
            declarations.push((property.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    rules
}

/// `--token` name → variable id, from the registry JSON, via the same slugs
/// the emitter uses.
struct TokenTable {
    by_token: BTreeMap<String, String>,
}

impl TokenTable {
    fn new(variables: &Value, warnings: &mut Vec<SourceDiagnostic>) -> Self {
        let mut by_token = BTreeMap::new();
        let collections = variables.get("collections").and_then(Value::as_object);
        let variables = variables.get("variables").and_then(Value::as_object);
        if let (Some(collections), Some(variables)) = (collections, variables) {
            for (key, variable) in variables {
                let Some(collection) = variable
                    .get("collection")
                    .and_then(Value::as_str)
                    .and_then(|collection| collections.get(collection))
                else {
                    continue;
                };
                let (Some(collection_name), Some(variable_name)) = (
                    collection.get("name").and_then(Value::as_str),
                    variable.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                let token = format!(
                    "--{}-{}",
                    slugify(collection_name, "collection"),
                    slugify(variable_name, "variable")
                );
                if let Some(first) = by_token.get(&token) {
                    if first != key {
                        warnings.push(SourceDiagnostic::warning(
                            REF_AMBIGUOUS_TOKEN,
                            format!(
                                "variables {collection_name}/{variable_name} and another share the token {token}; var() edits resolve to the first"
                            ),
                        ));
                    }
                    continue;
                }
                by_token.insert(token, key.clone());
            }
        }
        Self { by_token }
    }

    fn variable(&self, token: &str) -> Option<VariableId> {
        id_from_key(self.by_token.get(token)?).ok()
    }
}

/// A parsed CSS paint value in the emitted dialect.
enum Paint {
    None,
    Literal(Color),
    Var { token: String, literal: Color },
}

fn parse_paint(value: &str) -> Option<Paint> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some(Paint::None);
    }
    if let Some(inner) = value
        .strip_prefix("var(")
        .and_then(|inner| inner.strip_suffix(')'))
    {
        let (token, fallback) = inner.split_once(',')?;
        return Some(Paint::Var {
            token: token.trim().to_string(),
            literal: parse_color(fallback.trim())?,
        });
    }
    parse_color(value).map(Paint::Literal)
}

fn parse_color(value: &str) -> Option<Color> {
    if value.starts_with('#') {
        return Color::from_hex(&value.to_ascii_uppercase());
    }
    let inner = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let mut parts = inner.split(',').map(str::trim);
    let red = parts.next()?.parse().ok()?;
    let green = parts.next()?.parse().ok()?;
    let blue = parts.next()?.parse().ok()?;
    let alpha = match parts.next() {
        Some(alpha) => {
            let alpha: f64 = alpha.parse().ok()?;
            (alpha * 255.0).round().clamp(0.0, 255.0) as u8
        }
        None => 255,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(Color {
        r: red,
        g: green,
        b: blue,
        a: alpha,
    })
}

fn parse_px(value: &str) -> Option<f64> {
    value.trim().trim_end_matches("px").trim().parse().ok()
}

/// One node being edited: applies observations, flips `changed` only on a
/// real difference, and reports what it cannot keep.
struct Edit<'a> {
    node: &'a mut CanvasNode,
    display: &'a str,
    changed: bool,
    layout_changed: bool,
    tokens: &'a TokenTable,
    warnings: &'a mut Vec<SourceDiagnostic>,
}

impl Edit<'_> {
    fn keep(&mut self, what: &str) {
        self.keep_as(EDIT_NOT_KEPT, what);
    }

    fn keep_as(&mut self, code: &'static str, what: &str) {
        self.warnings.push(SourceDiagnostic::warning(
            code,
            format!("{}: {what}", self.display),
        ));
    }

    fn apply_svg(&mut self, slot: &Slot, element: &SvgElement) {
        if matches!(slot, Slot::Fill(_)) {
            // Clone elements mirror the base element's geometry; geometry
            // edits come from the base, only the paint belongs to the clone.
            return;
        }
        if let Slot::Background(index) = slot {
            // The background rect mirrors the frame's size; its paint comes
            // from CSS. Resizing bg0 resizes the frame.
            if *index == 0
                && let NodeData::Group(group) = &mut self.node.data
                && let (Some(width), Some(height)) = (element.width, element.height)
            {
                let size = Some([width, height]);
                if group.clip_size.is_some() {
                    if group.clip_size != size {
                        group.clip_size = size;
                        self.changed = true;
                    }
                } else if group.local_size.is_some() && group.local_size != size {
                    group.local_size = size;
                    self.changed = true;
                }
            }
            return;
        }
        if let Some(name) = &element.name
            && *name != self.node.name
        {
            self.node.name = name.clone();
            self.changed = true;
        }
        if let Some(transform) = &element.transform {
            match parse_transform(transform) {
                Some(parsed) => {
                    if parsed != self.node.transform {
                        self.node.transform = parsed;
                        self.changed = true;
                    }
                }
                None => self.keep(&format!(
                    "transform {transform:?} is outside the emitted dialect; the save regenerates it"
                )),
            }
        }
        match &mut self.node.data {
            NodeData::Vector(vector) => match element.tag.as_str() {
                "rect" => {
                    let desired = PathData::rect(
                        element.x.unwrap_or(0.0),
                        element.y.unwrap_or(0.0),
                        element.width.unwrap_or(0.0),
                        element.height.unwrap_or(0.0),
                    );
                    if element.width.unwrap_or(0.0) > 0.0
                        && element.height.unwrap_or(0.0) > 0.0
                        && vector.path.to_svg_d() != desired.to_svg_d()
                    {
                        vector.path = desired;
                        self.changed = true;
                    }
                }
                "path" => {
                    if let Some(d) = &element.d
                        && normalized(d) != normalized(&vector.path.to_svg_d())
                    {
                        self.keep(
                            "edited path data is not applied yet; the save regenerates it",
                        );
                    }
                }
                _ => {}
            },
            NodeData::Text(text) => {
                if let Some(lines) = &element.lines {
                    let content = lines.join("\n");
                    if content != text.content {
                        text.content = content;
                        self.changed = true;
                    }
                }
            }
            NodeData::Bitmap(bitmap) => {
                if let (Some(width), Some(height)) = (element.width, element.height) {
                    let size = [width, height];
                    if bitmap.local_size != size {
                        bitmap.local_size = size;
                        self.changed = true;
                    }
                }
                if let Some(href) = &element.href {
                    let expected = format!("{}", bitmap.asset);
                    if !href.contains(&expected) {
                        self.keep(
                            "an edited image href does not rebind the asset yet; the save regenerates it",
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn apply_declaration(&mut self, slot: &Slot, property: &str, value: &str) {
        // On a stacked-fill clone only the paint is authoritative; the other
        // declarations (rx, opacity) mirror the base element as plain
        // literals, and applying them would unbind the base's variables.
        if matches!(slot, Slot::Fill(_)) && property != "fill" {
            return;
        }
        match property {
            "fill" => self.apply_fill(slot, value),
            "stroke" => self.apply_stroke_paint(value),
            "stroke-width" => {
                let Some(width) = parse_px(value) else {
                    return self.keep(&format!("stroke-width {value:?} is not a number"));
                };
                self.apply_binding_from_value(value, BoundProp::StrokeWidth { index: 0 });
                if let NodeData::Vector(vector) = &mut self.node.data
                    && let Some(stroke) = vector.strokes.first_mut()
                    && stroke.width != width
                {
                    stroke.width = width;
                    self.changed = true;
                }
            }
            "stroke-dasharray" => {
                let dash: Option<Vec<f64>> = value
                    .split_whitespace()
                    .map(|part| part.parse().ok())
                    .collect();
                let Some(dash) = dash else {
                    return self.keep(&format!("stroke-dasharray {value:?} is not numbers"));
                };
                if let NodeData::Vector(vector) = &mut self.node.data
                    && let Some(stroke) = vector.strokes.first_mut()
                    && stroke.dash != dash
                {
                    stroke.dash = dash;
                    self.changed = true;
                }
            }
            "opacity" => {
                let Some(opacity) = parse_px(value) else {
                    return self.keep(&format!("opacity {value:?} is not a number"));
                };
                self.apply_binding_from_value(value, BoundProp::Opacity);
                let opacity = UnitInterval::new(opacity as f32);
                if self.node.opacity != opacity {
                    self.node.opacity = opacity;
                    self.changed = true;
                }
            }
            "rx" => {
                let Some(radius) = parse_px(value) else {
                    return self.keep(&format!("rx {value:?} is not a number"));
                };
                self.apply_binding_from_value(value, BoundProp::CornerRadius);
                let (corner_radius, corner_radii) = match &mut self.node.data {
                    NodeData::Vector(vector) => (&mut vector.corner_radius, &mut vector.corner_radii),
                    NodeData::Group(group) => (&mut group.corner_radius, &mut group.corner_radii),
                    _ => return,
                };
                let same = match corner_radii {
                    Some(radii) => radii.iter().all(|corner| *corner == radius),
                    None => *corner_radius == Some(radius),
                };
                if !same {
                    *corner_radius = Some(radius);
                    *corner_radii = None;
                    self.changed = true;
                }
            }
            "font-family" => {
                if let NodeData::Text(text) = &mut self.node.data {
                    let family = value.trim().trim_matches(['"', '\'']).to_string();
                    if family != text.style.font_family {
                        text.style.font_family = family;
                        self.changed = true;
                    }
                }
            }
            "font-size" => {
                if let NodeData::Text(text) = &mut self.node.data
                    && let Some(size) = parse_px(value)
                    && text.style.size_px != size
                {
                    text.style.size_px = size;
                    self.changed = true;
                }
            }
            "font-weight" => {
                if let NodeData::Text(text) = &mut self.node.data
                    && let Ok(weight) = value.trim().parse()
                    && text.style.weight != weight
                {
                    text.style.weight = weight;
                    self.changed = true;
                }
            }
            "font-style" => {
                if let NodeData::Text(text) = &mut self.node.data {
                    let italic = value.trim() == "italic";
                    if text.style.italic != italic {
                        text.style.italic = italic;
                        self.changed = true;
                    }
                }
            }
            "letter-spacing" => {
                if let NodeData::Text(text) = &mut self.node.data
                    && let Some(spacing) = parse_px(value)
                    && text.style.letter_spacing != spacing
                {
                    text.style.letter_spacing = spacing;
                    self.changed = true;
                }
            }
            "text-decoration" => {
                if let NodeData::Text(text) = &mut self.node.data {
                    let underline = value.contains("underline");
                    let strikethrough = value.contains("line-through");
                    if text.style.underline != underline
                        || text.style.strikethrough != strikethrough
                    {
                        text.style.underline = underline;
                        text.style.strikethrough = strikethrough;
                        self.changed = true;
                    }
                }
            }
            "text-anchor" => {
                if let NodeData::Text(text) = &mut self.node.data {
                    let align = match value.trim() {
                        "middle" => TextAlign::Center,
                        "end" => TextAlign::Right,
                        _ => TextAlign::Left,
                    };
                    if text.align != align {
                        text.align = align;
                        self.changed = true;
                    }
                }
            }
            "mix-blend-mode" => match blend_mode_from_css(value.trim()) {
                Some(blend) => {
                    if self.node.blend_mode != blend {
                        self.node.blend_mode = blend;
                        self.changed = true;
                    }
                }
                None => self.keep(&format!(
                    "mix-blend-mode {value:?} has no canvas equivalent; the save regenerates it"
                )),
            },
            "filter" => {
                if normalized(value) != normalized(&canonical_filter(self.node)) {
                    self.keep(
                        "edited filter effects are not applied yet; the save regenerates them",
                    );
                }
            }
            "--fanta-layout" | "--fanta-gap" | "--fanta-padding" => {
                self.apply_layout_intent(property, value);
            }
            _ => self.keep(&format!(
                "{property} is outside the hand-edit subset; the save regenerates it"
            )),
        }
    }

    fn apply_fill(&mut self, slot: &Slot, value: &str) {
        if value.trim_start().starts_with("url(") {
            // A canonical gradient paint: its def lives in the SVG; editing
            // gradients by hand goes through nodes.json.
            return;
        }
        let Some(paint) = parse_paint(value) else {
            return self.keep(&format!(
                "fill {value:?} is outside the emitted dialect; the save regenerates it"
            ));
        };
        let binding_prop = match (&slot, &self.node.data) {
            (Slot::Node, NodeData::Text(_)) => BoundProp::TextStyle,
            (Slot::Background(index), _) => BoundProp::FillColor {
                index: u16::try_from(*index).unwrap_or(u16::MAX),
            },
            (Slot::Fill(index), _) => BoundProp::FillColor {
                index: u16::try_from(*index).unwrap_or(u16::MAX),
            },
            _ => BoundProp::FillColor { index: 0 },
        };
        let color = match paint {
            Paint::None => {
                if let NodeData::Vector(vector) = &mut self.node.data
                    && matches!(slot, Slot::Node)
                    && !vector.fills.is_empty()
                {
                    vector.fills.clear();
                    self.changed = true;
                } else if matches!(slot, Slot::Fill(_)) {
                    self.keep(
                        "removing a stacked fill is not applied from CSS; edit nodes.json or the editor",
                    );
                }
                return;
            }
            Paint::Literal(color) => {
                self.set_binding(binding_prop, None);
                color
            }
            Paint::Var { token, literal } => {
                match self.tokens.variable(&token) {
                    Some(variable) => self.set_binding(binding_prop, Some(variable)),
                    None => self.keep_as(
                        REF_MISSING_TOKEN,
                        &format!(
                            "var({token}) names no variable; the literal fallback is kept unbound"
                        ),
                    ),
                }
                literal
            }
        };
        let mut note = None;
        let mut changed = false;
        {
            let target = match (slot, &mut self.node.data) {
                (Slot::Node, NodeData::Vector(vector)) => match vector.fills.first_mut() {
                    Some(Fill::Solid { color, .. }) => Some(color),
                    Some(_) => {
                        note = Some(
                            "a fill edit on a non-solid paint is not applied; the save regenerates it",
                        );
                        None
                    }
                    None => {
                        vector.fills.push(Fill::Solid {
                            color,
                            blend: BlendMode::Normal,
                        });
                        changed = true;
                        None
                    }
                },
                (Slot::Fill(index), NodeData::Vector(vector)) => {
                    match vector.fills.get_mut(*index) {
                        Some(Fill::Solid { color, .. }) => Some(color),
                        Some(_) => {
                            note = Some(
                                "a fill edit on a non-solid paint is not applied; the save regenerates it",
                            );
                            None
                        }
                        None => {
                            note = Some(
                                "the fill index does not exist; the declaration is regenerated on save",
                            );
                            None
                        }
                    }
                }
                (Slot::Node | Slot::Fill(_), NodeData::Boolean(boolean)) => {
                    let index = match slot {
                        Slot::Fill(index) => *index,
                        _ => 0,
                    };
                    match boolean.fills.get_mut(index) {
                        Some(Fill::Solid { color, .. }) => Some(color),
                        Some(_) => {
                            note = Some(
                                "a fill edit on a non-solid paint is not applied; the save regenerates it",
                            );
                            None
                        }
                        None => {
                            note = Some(
                                "the fill index does not exist; the declaration is regenerated on save",
                            );
                            None
                        }
                    }
                }
                (Slot::Node, NodeData::Text(text)) => Some(&mut text.style.color),
                (Slot::Background(index), NodeData::Group(group)) => {
                    let fill = group
                        .background
                        .iter_mut()
                        .chain(group.background_fills.iter_mut())
                        .nth(*index);
                    match fill {
                        Some(Fill::Solid { color, .. }) => Some(color),
                        Some(_) => {
                            note = Some(
                                "a fill edit on a non-solid background is not applied; the save regenerates it",
                            );
                            None
                        }
                        None => {
                            note = Some(
                                "the background index does not exist; the declaration is regenerated on save",
                            );
                            None
                        }
                    }
                }
                _ => None,
            };
            if let Some(target) = target
                && *target != color
            {
                *target = color;
                changed = true;
            }
        }
        self.changed |= changed;
        if let Some(note) = note {
            self.keep(note);
        }
    }

    fn apply_stroke_paint(&mut self, value: &str) {
        if value.trim_start().starts_with("url(") {
            // A canonical gradient paint: its def lives in the SVG; editing
            // gradients by hand goes through nodes.json.
            return;
        }
        let Some(paint) = parse_paint(value) else {
            return self.keep(&format!(
                "stroke {value:?} is outside the emitted dialect; the save regenerates it"
            ));
        };
        if !matches!(self.node.data, NodeData::Vector(_)) {
            return;
        }
        let color = match paint {
            Paint::None => {
                if let NodeData::Vector(vector) = &mut self.node.data
                    && !vector.strokes.is_empty()
                {
                    vector.strokes.clear();
                    self.changed = true;
                }
                return;
            }
            Paint::Literal(color) => {
                self.set_binding(BoundProp::StrokeColor { index: 0 }, None);
                color
            }
            Paint::Var { token, literal } => {
                match self.tokens.variable(&token) {
                    Some(variable) => {
                        self.set_binding(BoundProp::StrokeColor { index: 0 }, Some(variable))
                    }
                    None => self.keep_as(
                        REF_MISSING_TOKEN,
                        &format!(
                            "var({token}) names no variable; the literal fallback is kept unbound"
                        ),
                    ),
                }
                literal
            }
        };
        let NodeData::Vector(vector) = &mut self.node.data else {
            return;
        };
        let mut note = None;
        let mut changed = false;
        match vector.strokes.first_mut() {
            Some(stroke) => match &mut stroke.paint {
                Fill::Solid { color: current, .. } => {
                    if *current != color {
                        *current = color;
                        changed = true;
                    }
                }
                _ => {
                    note = Some(
                        "a stroke edit on a non-solid paint is not applied; the save regenerates it",
                    );
                }
            },
            None => {
                note = Some(
                    "adding a stroke by hand is not supported yet; the declaration is regenerated on save",
                );
            }
        }
        self.changed |= changed;
        if let Some(note) = note {
            self.keep(note);
        }
    }

    /// Keeps `prop`'s binding in step with a `var()`/literal swap. `None`
    /// removes the binding; `Some` points it at the variable.
    fn set_binding(&mut self, prop: BoundProp, variable: Option<VariableId>) {
        match variable {
            None => {
                if self.node.bindings.remove(&prop).is_some() {
                    self.changed = true;
                }
            }
            Some(variable) => {
                if self.node.bindings.get(&prop) != Some(&variable) {
                    self.node.bindings.insert(prop, variable);
                    self.changed = true;
                }
            }
        }
    }

    /// A `var()` on a numeric declaration keeps the binding; a literal drops
    /// it (the var-swap half of R7 for non-paint properties).
    fn apply_binding_from_value(&mut self, value: &str, prop: BoundProp) {
        match parse_var_token(value) {
            Some(token) => match self.tokens.variable(&token) {
                Some(variable) => self.set_binding(prop, Some(variable)),
                None => self.keep_as(
                    REF_MISSING_TOKEN,
                    &format!("var({token}) names no variable; the literal fallback is kept unbound"),
                ),
            },
            None => self.set_binding(prop, None),
        }
    }

    fn apply_layout_intent(&mut self, property: &str, value: &str) {
        let NodeData::Group(group) = &mut self.node.data else {
            return self.keep(&format!("{property} applies only to frames"));
        };
        let Some(layout) = &mut group.auto_layout else {
            return self.keep(&format!(
                "{property} on a frame without auto-layout is not applied; enable auto-layout in the editor first"
            ));
        };
        match property {
            "--fanta-layout" => {
                match serde_json::from_value(Value::String(value.trim().to_string())) {
                    Ok(mode) => {
                        if layout.mode != mode {
                            layout.mode = mode;
                            self.changed = true;
                            self.layout_changed = true;
                        }
                    }
                    Err(_) => self.keep(&format!("--fanta-layout {value:?} is not a layout mode")),
                }
            }
            "--fanta-gap" => match parse_px(value) {
                Some(gap) => {
                    if layout.spacing != gap {
                        layout.spacing = gap;
                        self.changed = true;
                        self.layout_changed = true;
                    }
                }
                None => self.keep(&format!("--fanta-gap {value:?} is not a number")),
            },
            "--fanta-padding" => {
                let padding: Option<Vec<f64>> = value
                    .split_whitespace()
                    .map(|part| parse_px(part))
                    .collect();
                match padding.as_deref() {
                    Some([all]) => {
                        let padding = [*all; 4];
                        if layout.padding != padding {
                            layout.padding = padding;
                            self.changed = true;
                            self.layout_changed = true;
                        }
                    }
                    Some([a, b, c, d]) => {
                        let padding = [*a, *b, *c, *d];
                        if layout.padding != padding {
                            layout.padding = padding;
                            self.changed = true;
                            self.layout_changed = true;
                        }
                    }
                    _ => self.keep(&format!(
                        "--fanta-padding {value:?} needs one or four numbers"
                    )),
                }
            }
            _ => {}
        }
    }
}

fn parse_var_token(value: &str) -> Option<String> {
    let inner = value
        .trim()
        .strip_prefix("var(")?
        .strip_suffix(')')?;
    let token = match inner.split_once(',') {
        Some((token, _)) => token,
        None => inner,
    };
    Some(token.trim().to_string())
}

/// `translate(x y)` or `matrix(a b c d e f)`, commas or spaces.
fn parse_transform(value: &str) -> Option<Transform2D> {
    let value = value.trim();
    let (kind, arguments) = value.split_once('(')?;
    let arguments = arguments.strip_suffix(')')?;
    let numbers: Option<Vec<f64>> = arguments
        .split(|separator: char| separator == ',' || separator.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse().ok())
        .collect();
    let numbers = numbers?;
    match (kind.trim(), numbers.as_slice()) {
        ("translate", [x]) => Some(Transform2D::translation(*x, 0.0)),
        ("translate", [x, y]) => Some(Transform2D::translation(*x, *y)),
        ("matrix", [a, b, c, d, e, f]) => Some(Transform2D(glam::DAffine2::from_cols_array(&[
            *a, *b, *c, *d, *e, *f,
        ]))),
        _ => None,
    }
}

/// The filter string the emitter would write for this node, for edit
/// detection. Kept in step with the emitter's `start_element`.
fn canonical_filter(node: &CanvasNode) -> String {
    let mut filters = Vec::new();
    for shadow in &node.effects {
        if matches!(shadow.kind, ShadowKind::Drop) {
            filters.push(format!(
                "drop-shadow({}px {}px {}px {})",
                number(shadow.offset[0]),
                number(shadow.offset[1]),
                number(shadow.blur),
                css_color(shadow.color),
            ));
        }
    }
    for blur in &node.blurs {
        if matches!(blur.kind, BlurKind::Layer) {
            filters.push(format!("blur({}px)", number(blur.radius)));
        }
    }
    filters.join(" ")
}

/// The reverse of the emitter's blend table.
fn blend_mode_from_css(value: &str) -> Option<BlendMode> {
    [
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ]
    .into_iter()
    .find(|mode| css_blend_mode(*mode) == Some(value))
}

fn normalized(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_svg_is_reported_with_its_file_and_position() {
        let mut elements = BTreeMap::new();
        let mut warnings = Vec::new();
        parse_design_svg(
            "<svg>\n  <g id=\"a\"></g>\n  <rect id=\"b\"></svg>",
            "pages/home/page.svg",
            0,
            &mut elements,
            &mut warnings,
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].code, PARSE_SVG);
        assert_eq!(warnings[0].file.as_deref(), Some("pages/home/page.svg"));
        assert_eq!(warnings[0].line, Some(3), "{warnings:?}");
        assert!(elements.contains_key("a"), "elements before the error are kept");
    }

    #[test]
    fn a_repeated_attribute_keeps_its_first_value() {
        let mut elements = BTreeMap::new();
        let mut warnings = Vec::new();
        parse_design_svg(
            "<svg><rect id=\"a\" x=\"1\" x=\"2\" id=\"b\" y=\"3\"/></svg>",
            "pages/home/page.svg",
            0,
            &mut elements,
            &mut warnings,
        );
        assert_eq!(warnings.len(), 0, "{warnings:?}");
        assert_eq!(elements.keys().collect::<Vec<_>>(), ["a"]);
        let element = elements.get("a").expect("the element is read");
        assert_eq!((element.x, element.y), (Some(1.0), Some(3.0)));

        // An unquoted value is not read and does not take the name: a later
        // quoted attribute of that name is the first one that can be read.
        elements.clear();
        parse_design_svg(
            "<svg><rect id=\"a\" x = 1 x=\"2\" width=\"4\"y=5 y=\"3\"/></svg>",
            "pages/home/page.svg",
            0,
            &mut elements,
            &mut warnings,
        );
        let element = elements.get("a").expect("the element is read");
        assert_eq!(
            (element.x, element.y, element.width),
            (Some(2.0), Some(3.0), Some(4.0))
        );
    }

    #[test]
    fn a_tag_with_very_many_attributes_is_read_in_linear_time() {
        let mut src = String::from("<svg><rect id=\"a\"");
        for index in 0..200_000 {
            src.push_str(&format!(" a{index}=\"\""));
        }
        src.push_str(" y=\"3\"/></svg>");
        let mut elements = BTreeMap::new();
        let mut warnings = Vec::new();
        let started = std::time::Instant::now();
        parse_design_svg(&src, "pages/home/page.svg", 0, &mut elements, &mut warnings);
        let elapsed = started.elapsed();
        assert_eq!(elements.get("a").and_then(|element| element.y), Some(3.0));
        // A duplicate check that compares every pair takes minutes here.
        assert!(elapsed < std::time::Duration::from_secs(30), "{elapsed:?}");
    }

    #[test]
    fn a_grouped_selector_is_reported_with_its_file_and_position() {
        let mut warnings = Vec::new();
        let rules = parse_rules(
            "#a { fill: red; }\n#b, #c { fill: blue; }\n",
            "design.css",
            &mut warnings,
        );
        assert!(rules.contains_key("a"));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].code, EDIT_SELECTOR_OUTSIDE_DIALECT);
        assert_eq!(warnings[0].file.as_deref(), Some("design.css"));
        assert_eq!((warnings[0].line, warnings[0].column), (Some(2), Some(1)));
    }
}
