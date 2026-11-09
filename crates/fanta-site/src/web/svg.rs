//! The CSS-first SVG emitter: a page becomes readable SVG elements whose
//! styling lives in CSS rules, with `var(--token)` wherever the design binds a
//! variable. Node kinds and features outside the supported subset fall back to
//! a rasterized `<image>` and are named in the report, so pages stay correct
//! while the subset grows.

use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use fanta_doc::variables::VariableRegistry;
use fanta_doc::{
    BlendMode, BlurKind, BoundProp, CanvasNode, Color, Doc, Fill, NodeData, NodeFlags, NodeId,
    PathData, PathSegment, ShadowKind, Stroke, TextAlign, TextStyle, Transform2D,
};
use fanta_headless::export::{
    ExportEncoding, ExportSizing, export_node, page_visual_bounds, resolve_export_bindings,
};
use fanta_render::AssetResolver;
use serde::Serialize;

/// Raster hrefs carry this placeholder because the page's directory name is
/// not known inside the emitter; the caller substitutes the slug.
pub(crate) const RASTER_PLACEHOLDER: &str = "__RASTER_DIR__";

/// One page, emitted. The caller writes the files.
pub(crate) struct EmittedPage {
    pub svg: String,
    /// CSS rules for this page's nodes, appended to the shared design.css.
    pub css: String,
    pub width: u32,
    pub height: u32,
    /// Rasterized fallback nodes: file name under `pages/<slug>/` and bytes.
    pub rasters: Vec<(String, Vec<u8>)>,
    /// Asset ids referenced by `<image href="assets/…">` elements.
    pub assets: Vec<fanta_doc::AssetId>,
    pub approximations: Vec<Approximation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Approximation {
    pub node: String,
    pub name: String,
    pub what: String,
}

pub(crate) fn emit_page(
    authored: &Doc,
    resolver: Option<Arc<dyn AssetResolver>>,
    page: NodeId,
) -> Result<EmittedPage> {
    let resolved = resolve_export_bindings(authored);
    let bounds = page_visual_bounds(&resolved, Some(page))
        .filter(|bounds| bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0)
        .context("the page draws nothing to export")?;
    let mut emitter = Emitter {
        resolved: &resolved,
        authored,
        registry: &authored.variables,
        resolver,
        css: String::new(),
        rasters: Vec::new(),
        assets: Vec::new(),
        approximations: Vec::new(),
    };
    let mut body = String::new();
    emitter.emit_node(page, &mut body, 1)?;
    let width = bounds.width().ceil();
    let height = bounds.height().ceil();
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"{} {} {width} {height}\">\n{body}</svg>\n",
        number(bounds.min_x),
        number(bounds.min_y),
    );
    Ok(EmittedPage {
        svg,
        css: emitter.css,
        width: width as u32,
        height: height as u32,
        rasters: emitter.rasters,
        assets: emitter.assets,
        approximations: emitter.approximations,
    })
}

struct Emitter<'a> {
    /// Bound values substituted, so literals match the active modes.
    resolved: &'a Doc,
    /// The authored document: bindings live here.
    authored: &'a Doc,
    registry: &'a VariableRegistry,
    resolver: Option<Arc<dyn AssetResolver>>,
    css: String,
    rasters: Vec<(String, Vec<u8>)>,
    assets: Vec<fanta_doc::AssetId>,
    approximations: Vec<Approximation>,
}

impl Emitter<'_> {
    fn emit_node(&mut self, id: NodeId, out: &mut String, depth: usize) -> Result<()> {
        let Some(node) = self.resolved.scene.get(id).cloned() else {
            return Ok(());
        };
        if node.flags.contains(NodeFlags::HIDDEN) {
            return Ok(());
        }
        // Siblings after a mask node are clipped by it; the subset cannot
        // express that yet, so a parent holding a mask rasterizes whole.
        let has_mask_child = self
            .resolved
            .scene
            .children_of(Some(id))
            .iter()
            .any(|child| self.resolved.scene.get(*child).is_some_and(|c| c.is_mask));
        if has_mask_child {
            return self.raster_fallback(&node, out, depth, "a mask clips this subtree");
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
                        self.fill_declaration(&node, fill, index, &mut rule);
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
                for child in self.resolved.scene.children_of(Some(id)).to_vec() {
                    self.emit_node(child, out, depth + 1)?;
                }
                indent(out, depth);
                out.push_str("</g>\n");
            }
            NodeData::Vector(vector) => {
                let vector = vector.clone();
                let single_solid = vector.fills.len() <= 1
                    && vector
                        .fills
                        .first()
                        .is_none_or(|fill| matches!(fill, Fill::Solid { .. }));
                if !single_solid {
                    return self.raster_fallback(&node, out, depth, "gradient or stacked fills");
                }
                let mut rule = Declarations::default();
                match vector.fills.first() {
                    Some(fill) => self.fill_declaration(&node, fill, 0, &mut rule),
                    None => rule.push("fill", "none".to_string()),
                }
                self.stroke_declarations(&node, &vector.strokes, &mut rule);
                match vector.path.is_rect().then(|| rect_of(&vector.path)).flatten() {
                    Some((x, y, width, height)) => {
                        let radius = vector
                            .corner_radii
                            .map(|radii| radii[0])
                            .or(vector.corner_radius);
                        if vector
                            .corner_radii
                            .is_some_and(|radii| radii.windows(2).any(|pair| pair[0] != pair[1]))
                        {
                            self.approximate(&node, "mixed corner radii use the first corner");
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
                        self.start_element(&node, out, depth, "rect");
                        let _ = write!(
                            out,
                            " x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                            number(x),
                            number(y),
                            number(width),
                            number(height)
                        );
                        out.push_str("/>\n");
                    }
                    None => {
                        self.start_element(&node, out, depth, "path");
                        let _ = write!(out, " d=\"{}\"", vector.path.to_svg_d());
                        out.push_str("/>\n");
                    }
                }
                self.rule(&id.to_string(), rule);
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
                self.assets.push(bitmap.asset);
                self.start_element(&node, out, depth, "image");
                let _ = write!(
                    out,
                    " href=\"assets/{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\"",
                    bitmap.asset,
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
        let class = crate::slug(&node.name);
        if !class.is_empty() {
            let _ = write!(out, " class=\"{class}\"");
        }
        let _ = write!(out, " data-name=\"{}\"", xml_escape(&node.name));
        if let Some(transform) = transform_attribute(&node.transform) {
            let _ = write!(out, " transform=\"{transform}\"");
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
                let _ = write!(out, " clip-path=\"url(#{parent}-clip)\"");
            }
        }

        let mut rule = Declarations::default();
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
        self.rule(&node.id.to_string(), rule);
    }

    fn fill_declaration(
        &mut self,
        node: &CanvasNode,
        fill: &Fill,
        index: usize,
        rule: &mut Declarations,
    ) {
        match fill {
            Fill::Solid { color, blend } => {
                if !blend.is_normal() {
                    self.approximate(node, "per-paint blend modes are dropped");
                }
                let literal = css_color(*color);
                let index = u16::try_from(index).unwrap_or(u16::MAX);
                match self.bound_variable(node, BoundProp::FillColor { index }) {
                    Some(token) => rule.push("fill", format!("var({token}, {literal})")),
                    None => rule.push("fill", literal),
                }
            }
            _ => self.approximate(node, "non-solid background fills are dropped"),
        }
    }

    fn stroke_declarations(
        &mut self,
        node: &CanvasNode,
        strokes: &[Stroke],
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
            _ => self.approximate(node, "non-solid strokes are dropped"),
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
            crate::slug(&collection.name),
            crate::slug(&variable.name)
        ))
    }

    fn raster_fallback(
        &mut self,
        node: &CanvasNode,
        out: &mut String,
        depth: usize,
        what: &str,
    ) -> Result<()> {
        self.approximate(node, &format!("rasterized: {what}"));
        let image = export_node(
            self.resolved,
            self.resolver.clone(),
            node.id,
            ExportEncoding::Png,
            ExportSizing::Scale(2.0),
        )
        .with_context(|| format!("rasterizing {} ({})", node.id, node.name))?;
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
        let low = inverse.transform_point2(glam::DVec2::new(image.bounds.min_x, image.bounds.min_y));
        let high = inverse.transform_point2(glam::DVec2::new(image.bounds.max_x, image.bounds.max_y));
        let file = format!("{}.png", node.id);
        indent(out, depth);
        let _ = writeln!(
            out,
            "<image id=\"{}\" data-name=\"{}\" href=\"{RASTER_PLACEHOLDER}/{file}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
            node.id,
            xml_escape(&node.name),
            number(low.x.min(high.x)),
            number(low.y.min(high.y)),
            number((high.x - low.x).abs()),
            number((high.y - low.y).abs()),
        );
        self.rasters.push((file, image.bytes));
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
        NodeData::Instance(_) => "component instances are rasterized for now",
        NodeData::Boolean(_) => "boolean shapes are rasterized for now",
        NodeData::TextPath(_) => "text on a path is rasterized for now",
        NodeData::Video(_) | NodeData::Audio(_) => "media nodes export their poster",
        _ => "this node kind is rasterized for now",
    }
}

fn css_blend_mode(mode: BlendMode) -> Option<&'static str> {
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
    format!("\"{}\"", value.replace('"', ""))
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Shortest-round-trip float formatting; deterministic across platforms.
fn number(value: f64) -> String {
    format!("{value}")
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}
