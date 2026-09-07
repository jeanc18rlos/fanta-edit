//! Code-native monochrome vector icons.
//!
//! ARCHITECTURE.md §5 forbids Unicode characters as icon sources: their
//! outline, alignment, and availability vary by platform. Icons here are drawn
//! as theme-colored stroke paths on a 16-unit design grid so they inherit the
//! active theme color, stay sharp at compact sizes, and never depend on
//! platform fonts or host-provided assets. The toolbar's tool/mode icons use
//! the same machinery via [`render_icon_canvas`].

use gpui::{
    AnyElement, Hsla, IntoElement as _, Path, PathBuilder, PathStyle, Pixels, Point, StrokeOptions,
    Styled as _, canvas, point, px,
};
use lyon::tessellation::{LineCap, LineJoin};

/// The shared icon design grid, in logical units.
pub(crate) const ICON_SIZE: f32 = 16.;
const STROKE_WIDTH: f32 = 1.25;
/// Lucide's design grid: icons are authored on 24 units with a 2-unit stroke
/// and round caps/joins, then scaled to the rendered size. Toolbar drawings
/// that have no Lucide asset use this idiom so they read as one set next to
/// the `IconName` SVGs (§5).
pub(crate) const LUCIDE_GRID: f32 = 24.;
const LUCIDE_STROKE_WIDTH: f32 = 2.;

/// Semantic icons shared by Fanta control surfaces.
///
/// §16 contract: the atom owns the stroke-path drawings; the caller picks a
/// variant and renders it via [`render_control_icon`], never assembles glyph
/// or div art.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlIcon {
    Play,
    Pause,
    KeyframeDiamond,
    Loop,
    Sparkle,
    Help,
    Close,
    JumpArrow,
    VariableNumber,
    VariableText,
    VariableColor,
    VariableBoolean,
    // Node-kind icons. Node kinds whose drawing a toolbar tool already owns
    // (Rectangle, Ellipse, Polygon, Star, Line, Arrow, Pen, Pencil, Slice)
    // render via `crate::toolbar::render_tool_icon` instead of duplicating
    // paths here.
    Component,
    Instance,
    Group,
    Frame,
    Section,
    Text,
    Image,
    Video,
    Mask,
    BooleanOperation,
    Lock,
    Unlock,
    Eye,
    EyeClosed,
}

impl ControlIcon {
    #[cfg(test)]
    pub(crate) const ALL: &'static [Self] = &[
        Self::Play,
        Self::Pause,
        Self::KeyframeDiamond,
        Self::Loop,
        Self::Sparkle,
        Self::Help,
        Self::Close,
        Self::JumpArrow,
        Self::VariableNumber,
        Self::VariableText,
        Self::VariableColor,
        Self::VariableBoolean,
        Self::Component,
        Self::Instance,
        Self::Group,
        Self::Frame,
        Self::Section,
        Self::Text,
        Self::Image,
        Self::Video,
        Self::Mask,
        Self::BooleanOperation,
        Self::Lock,
        Self::Unlock,
        Self::Eye,
        Self::EyeClosed,
    ];
}

/// Renders a shared semantic icon at `size` in the given color.
///
/// §16 contract: the atom owns theme-colored stroke rendering on the 16-unit
/// grid; the caller supplies color and size and places the element.
pub fn render_control_icon(icon: ControlIcon, color: Hsla, size: f32) -> AnyElement {
    render_icon_canvas(color, size, move |path| draw_control_icon(path, icon))
}

/// Renders a custom stroke-path icon drawn on the shared 16-unit grid.
pub(crate) fn render_icon_canvas(
    color: Hsla,
    size: f32,
    draw: impl Fn(&mut IconPath) + 'static,
) -> AnyElement {
    render_wide_icon_canvas(color, size, ICON_SIZE, draw)
}

/// Renders a stroke-path icon authored on Lucide's 24-unit grid with the
/// Lucide stroke idiom (2-unit stroke scaled, round caps and joins).
pub(crate) fn render_lucide_icon_canvas(
    color: Hsla,
    size: f32,
    draw: impl Fn(&mut IconPath) + 'static,
) -> AnyElement {
    let size = size.max(1.);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let mut path = IconPath::lucide(bounds.origin, size);
            draw(&mut path);
            if let Some(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(size))
    .into_any_element()
}

/// Renders a non-square stroke-path icon: `grid_width` columns of the shared
/// grid at the standard 16-unit height. `size` remains the rendered height.
pub(crate) fn render_wide_icon_canvas(
    color: Hsla,
    size: f32,
    grid_width: f32,
    draw: impl Fn(&mut IconPath) + 'static,
) -> AnyElement {
    let size = size.max(1.);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let mut path = IconPath::new(bounds.origin, size);
            draw(&mut path);
            if let Some(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .w(px(size / ICON_SIZE * grid_width))
    .h(px(size))
    .into_any_element()
}

/// Stroke-path builder over the shared 16-unit icon grid.
pub(crate) struct IconPath {
    builder: PathBuilder,
    origin: Point<Pixels>,
    scale: f32,
}

impl IconPath {
    pub(crate) fn new(origin: Point<Pixels>, size: f32) -> Self {
        let scale = size / ICON_SIZE;
        Self {
            builder: PathBuilder::stroke(px(STROKE_WIDTH * scale)),
            origin,
            scale,
        }
    }

    /// A builder over Lucide's 24-unit grid: coordinates are Lucide's own,
    /// the stroke is 2 units scaled to `size`, and caps/joins are round.
    pub(crate) fn lucide(origin: Point<Pixels>, size: f32) -> Self {
        let scale = size / LUCIDE_GRID;
        let options = StrokeOptions::default()
            .with_line_width(LUCIDE_STROKE_WIDTH * scale)
            .with_line_cap(LineCap::Round)
            .with_line_join(LineJoin::Round);
        Self {
            builder: PathBuilder::default().with_style(PathStyle::Stroke(options)),
            origin,
            scale,
        }
    }

    fn point(&self, x: f32, y: f32) -> Point<Pixels> {
        point(
            self.origin.x + px(x * self.scale),
            self.origin.y + px(y * self.scale),
        )
    }

    pub(crate) fn move_to(&mut self, x: f32, y: f32) {
        let point = self.point(x, y);
        self.builder.move_to(point);
    }

    pub(crate) fn line_to(&mut self, x: f32, y: f32) {
        let point = self.point(x, y);
        self.builder.line_to(point);
    }

    pub(crate) fn line(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.move_to(from.0, from.1);
        self.line_to(to.0, to.1);
    }

    pub(crate) fn curve_to(&mut self, to: (f32, f32), control: (f32, f32)) {
        let to = self.point(to.0, to.1);
        let control = self.point(control.0, control.1);
        self.builder.curve_to(to, control);
    }

    pub(crate) fn cubic_to(
        &mut self,
        to: (f32, f32),
        control_a: (f32, f32),
        control_b: (f32, f32),
    ) {
        let to = self.point(to.0, to.1);
        let control_a = self.point(control_a.0, control_a.1);
        let control_b = self.point(control_b.0, control_b.1);
        self.builder.cubic_bezier_to(to, control_a, control_b);
    }

    pub(crate) fn arc_to(
        &mut self,
        radius_x: f32,
        radius_y: f32,
        large_arc: bool,
        sweep: bool,
        to: (f32, f32),
    ) {
        let to = self.point(to.0, to.1);
        self.builder.arc_to(
            point(px(radius_x * self.scale), px(radius_y * self.scale)),
            px(0.),
            large_arc,
            sweep,
            to,
        );
    }

    pub(crate) fn poly<const N: usize>(&mut self, points: [(f32, f32); N], closed: bool) {
        let points = points.map(|(x, y)| self.point(x, y));
        self.builder.add_polygon(&points, closed);
    }

    pub(crate) fn rect(&mut self, left: f32, top: f32, right: f32, bottom: f32) {
        self.poly(
            [(left, top), (right, top), (right, bottom), (left, bottom)],
            true,
        );
    }

    /// An axis-aligned rectangle with `radius` corners, drawn clockwise from
    /// the top edge (Lucide's `rx` idiom).
    pub(crate) fn rounded_rect(
        &mut self,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
        radius: f32,
    ) {
        let radius = radius
            .min((right - left) / 2.)
            .min((bottom - top) / 2.)
            .max(0.);
        self.move_to(left + radius, top);
        self.line_to(right - radius, top);
        self.arc_to(radius, radius, false, true, (right, top + radius));
        self.line_to(right, bottom - radius);
        self.arc_to(radius, radius, false, true, (right - radius, bottom));
        self.line_to(left + radius, bottom);
        self.arc_to(radius, radius, false, true, (left, bottom - radius));
        self.line_to(left, top + radius);
        self.arc_to(radius, radius, false, true, (left + radius, top));
        self.builder.close();
    }

    pub(crate) fn diamond(&mut self, center_x: f32, center_y: f32, radius: f32) {
        self.poly(
            [
                (center_x, center_y - radius),
                (center_x + radius, center_y),
                (center_x, center_y + radius),
                (center_x - radius, center_y),
            ],
            true,
        );
    }

    pub(crate) fn ellipse(&mut self, center_x: f32, center_y: f32, radius_x: f32, radius_y: f32) {
        let radii = point(px(radius_x * self.scale), px(radius_y * self.scale));
        self.move_to(center_x, center_y - radius_y);
        let bottom = self.point(center_x, center_y + radius_y);
        self.builder.arc_to(radii, px(0.), false, true, bottom);
        let top = self.point(center_x, center_y - radius_y);
        self.builder.arc_to(radii, px(0.), false, true, top);
        self.builder.close();
    }

    pub(crate) fn circle(&mut self, center_x: f32, center_y: f32, radius: f32) {
        self.ellipse(center_x, center_y, radius, radius);
    }

    pub(crate) fn plus(&mut self, center_x: f32, center_y: f32, radius: f32) {
        self.line((center_x - radius, center_y), (center_x + radius, center_y));
        self.line((center_x, center_y - radius), (center_x, center_y + radius));
    }

    pub(crate) fn close(&mut self) {
        self.builder.close();
    }

    pub(crate) fn build(self) -> Option<Path<Pixels>> {
        self.builder.build().ok()
    }
}

fn draw_control_icon(path: &mut IconPath, icon: ControlIcon) {
    match icon {
        ControlIcon::Play => {
            path.poly([(5.4, 3.4), (12.5, 8.), (5.4, 12.6)], true);
        }
        ControlIcon::Pause => {
            path.line((5.9, 3.8), (5.9, 12.2));
            path.line((10.1, 3.8), (10.1, 12.2));
        }
        ControlIcon::KeyframeDiamond => {
            path.diamond(8., 8., 4.2);
        }
        ControlIcon::Loop => {
            path.move_to(12.9, 8.);
            path.arc_to(4.9, 4.9, true, true, (8., 3.1));
            path.poly([(6.1, 1.5), (8., 3.1), (6.3, 5.)], false);
        }
        ControlIcon::Sparkle => {
            path.poly(
                [
                    (8., 2.5),
                    (9.6, 6.4),
                    (13.5, 8.),
                    (9.6, 9.6),
                    (8., 13.5),
                    (6.4, 9.6),
                    (2.5, 8.),
                    (6.4, 6.4),
                ],
                true,
            );
        }
        ControlIcon::Help => {
            path.move_to(5.6, 5.4);
            path.cubic_to((10.4, 5.4), (5.6, 2.2), (10.4, 2.2));
            path.cubic_to((8., 8.6), (10.4, 7.2), (8., 7.2));
            path.line_to(8., 9.8);
            path.circle(8., 12.6, 0.6);
        }
        ControlIcon::Close => {
            path.line((4.4, 4.4), (11.6, 11.6));
            path.line((11.6, 4.4), (4.4, 11.6));
        }
        ControlIcon::JumpArrow => {
            path.move_to(3.2, 12.);
            path.line_to(3.2, 8.4);
            path.curve_to((5.6, 6.), (3.2, 6.));
            path.line_to(12.6, 6.);
            path.poly([(9.9, 3.3), (12.8, 6.), (9.9, 8.7)], false);
        }
        ControlIcon::VariableNumber => {
            path.line((6.1, 3.6), (5.2, 12.4));
            path.line((10.8, 3.6), (9.9, 12.4));
            path.line((3.9, 6.4), (12.6, 6.4));
            path.line((3.4, 9.8), (12.1, 9.8));
        }
        ControlIcon::VariableText => {
            path.line((4., 4.), (12., 4.));
            path.line((8., 4.), (8., 12.2));
        }
        ControlIcon::VariableColor => {
            path.circle(8., 8., 4.9);
            path.line((8., 3.1), (8., 12.9));
        }
        ControlIcon::VariableBoolean => {
            path.ellipse(8., 8., 6., 3.6);
            path.circle(10.4, 8., 2.);
        }
        ControlIcon::Component => {
            path.diamond(8., 8., 5.4);
        }
        ControlIcon::Instance => {
            path.diamond(8., 8., 5.4);
            path.diamond(8., 8., 2.6);
        }
        ControlIcon::Group => {
            for (x, y, dx, dy) in [
                (2.5, 2.5, 3., 3.),
                (13.5, 2.5, -3., 3.),
                (2.5, 13.5, 3., -3.),
                (13.5, 13.5, -3., -3.),
            ] {
                path.move_to(x + dx, y);
                path.line_to(x, y);
                path.line_to(x, y + dy);
            }
        }
        ControlIcon::Frame => {
            path.line((5.5, 2.), (5.5, 14.));
            path.line((10.5, 2.), (10.5, 14.));
            path.line((2., 5.5), (14., 5.5));
            path.line((2., 10.5), (14., 10.5));
        }
        ControlIcon::Section => {
            path.poly(
                [
                    (2., 13.5),
                    (2., 3.),
                    (6.8, 3.),
                    (8., 4.8),
                    (14., 4.8),
                    (14., 13.5),
                ],
                true,
            );
        }
        ControlIcon::Text => {
            path.line((3.5, 3.), (12.5, 3.));
            path.line((8., 3.), (8., 13.));
            path.line((6., 13.), (10., 13.));
        }
        ControlIcon::Image => {
            path.rect(2., 3., 14., 13.);
            path.circle(5.4, 6.4, 1.1);
            path.poly(
                [
                    (3.2, 11.8),
                    (6.8, 8.2),
                    (9.2, 10.4),
                    (11.2, 8.6),
                    (12.9, 10.3),
                ],
                false,
            );
        }
        ControlIcon::Video => {
            path.rect(2., 3.5, 14., 12.5);
            path.poly([(6.8, 5.9), (10.6, 8.), (6.8, 10.1)], true);
        }
        ControlIcon::Mask => {
            path.circle(8., 8., 5.2);
            path.line((4.3, 11.7), (11.7, 4.3));
        }
        ControlIcon::BooleanOperation => {
            path.circle(6.2, 8., 3.9);
            path.circle(9.8, 8., 3.9);
        }
        ControlIcon::Lock => {
            path.rect(3.8, 7.4, 12.2, 13.6);
            path.move_to(5.6, 7.4);
            path.line_to(5.6, 5.2);
            path.arc_to(2.4, 2.4, false, true, (10.4, 5.2));
            path.line_to(10.4, 7.4);
        }
        ControlIcon::Unlock => {
            path.rect(3.8, 7.4, 12.2, 13.6);
            path.move_to(5.6, 7.4);
            path.line_to(5.6, 4.6);
            path.arc_to(2.4, 2.4, false, true, (10.4, 4.6));
            path.line_to(10.4, 6.);
        }
        ControlIcon::Eye => {
            path.move_to(2., 8.);
            path.curve_to((14., 8.), (8., 3.2));
            path.move_to(2., 8.);
            path.curve_to((14., 8.), (8., 12.8));
            path.circle(8., 8., 2.);
        }
        ControlIcon::EyeClosed => {
            path.move_to(2., 7.);
            path.curve_to((14., 7.), (8., 11.8));
            path.line((4.2, 8.5), (3., 10.2));
            path.line((8., 9.4), (8., 11.4));
            path.line((11.8, 8.5), (13., 10.2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_icon_builds_a_valid_path() {
        let origin = point(px(0.), px(0.));

        for &icon in ControlIcon::ALL {
            let mut path = IconPath::new(origin, ICON_SIZE);
            draw_control_icon(&mut path, icon);
            assert!(
                path.build().is_some(),
                "failed to build the {icon:?} control icon"
            );
        }
    }
}
