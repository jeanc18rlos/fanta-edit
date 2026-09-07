//! Toolbar tool and mode icons.
//!
//! Tools and modes with a fitting Lucide asset in gpui-component's `IconName`
//! set render that SVG (it resolves against the host's asset source, §5).
//! The rest are stroke paths authored on Lucide's own 24-unit grid with its
//! 2-unit round-capped stroke, so both kinds read as one set at the compact
//! toolbar sizes and inherit the active theme color.

use gpui::{AnyElement, Hsla, IntoElement as _, Styled as _, px};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::atoms::vector_icon::{IconPath, render_lucide_icon_canvas};

use super::{ToolbarMode, ToolbarTool};

/// How one toolbar tool or mode is drawn.
pub(crate) enum ToolbarIcon {
    /// A Lucide asset from the `IconName` set.
    Asset(IconName),
    /// A stroke path authored on Lucide's 24-unit grid.
    Drawn(fn(&mut IconPath)),
}

/// Renders a compact semantic icon for a toolbar tool.
pub(crate) fn render_tool_icon(tool: ToolbarTool, color: Hsla, size: f32) -> AnyElement {
    render_toolbar_icon(tool_icon(tool), color, size)
}

/// Renders a compact semantic icon for a toolbar mode.
pub(crate) fn render_mode_icon(mode: ToolbarMode, color: Hsla, size: f32) -> AnyElement {
    render_toolbar_icon(mode_icon(mode), color, size)
}

/// Renders an `IconName` asset at `size` in `color`, the same treatment the
/// drawn icons receive.
pub(crate) fn render_icon_asset(icon: IconName, color: Hsla, size: f32) -> AnyElement {
    Icon::new(icon)
        .with_size(px(size))
        .text_color(color)
        .into_any_element()
}

fn render_toolbar_icon(icon: ToolbarIcon, color: Hsla, size: f32) -> AnyElement {
    match icon {
        ToolbarIcon::Asset(name) => render_icon_asset(name, color, size),
        ToolbarIcon::Drawn(draw) => render_lucide_icon_canvas(color, size, draw),
    }
}

/// The icon for one mode tile.
pub(crate) fn mode_icon(mode: ToolbarMode) -> ToolbarIcon {
    match mode {
        // Lucide `square-dashed-mouse-pointer`: select and edit a frame.
        ToolbarMode::Design => ToolbarIcon::Asset(IconName::Inspector),
        // Lucide `clapperboard`.
        ToolbarMode::Motion => ToolbarIcon::Drawn(|path| {
            path.move_to(3., 11.);
            path.line_to(21., 11.);
            path.line_to(21., 19.);
            path.arc_to(2., 2., false, true, (19., 21.));
            path.line_to(5., 21.);
            path.arc_to(2., 2., false, true, (3., 19.));
            path.close();
            path.poly(
                [
                    (20.2, 6.),
                    (3., 11.),
                    (2.1, 8.6),
                    (3.2, 6.4),
                    (17., 2.3),
                    (19.5, 3.5),
                ],
                true,
            );
            path.line((6.2, 5.3), (9.3, 9.2));
            path.line((12.4, 3.4), (15.5, 7.4));
        }),
        // Lucide `code-xml`.
        ToolbarMode::Dev => ToolbarIcon::Drawn(draw_code_xml),
    }
}

/// The icon for one tool.
pub(crate) fn tool_icon(tool: ToolbarTool) -> ToolbarIcon {
    match tool {
        ToolbarTool::Frame => ToolbarIcon::Asset(IconName::Frame),
        ToolbarTool::Star => ToolbarIcon::Asset(IconName::Star),
        ToolbarTool::Resources => ToolbarIcon::Asset(IconName::LayoutDashboard),
        ToolbarTool::Inspect => ToolbarIcon::Asset(IconName::Search),
        ToolbarTool::ReadyForDev => ToolbarIcon::Asset(IconName::CircleCheck),
        // Lucide `mouse-pointer-2`.
        ToolbarTool::Move => ToolbarIcon::Drawn(|path| draw_pointer(path, 1., 0., 0.)),
        // Lucide `hand`.
        ToolbarTool::Hand => ToolbarIcon::Drawn(|path| {
            path.move_to(18., 11.);
            path.line_to(18., 6.);
            path.arc_to(2., 2., false, false, (14., 6.));
            path.line_to(14., 7.);
            path.move_to(14., 10.);
            path.line_to(14., 4.);
            path.arc_to(2., 2., false, false, (10., 4.));
            path.line_to(10., 6.);
            path.move_to(10., 10.5);
            path.line_to(10., 6.);
            path.arc_to(2., 2., false, false, (6., 6.));
            path.line_to(6., 14.);
            path.move_to(18., 8.);
            path.arc_to(2., 2., true, true, (22., 8.));
            path.line_to(22., 14.);
            path.arc_to(8., 8., false, true, (14., 22.));
            path.line_to(12., 22.);
            path.cubic_to((6.01, 19.66), (9.2, 22.), (7.5, 21.14));
            path.line_to(2.41, 16.06);
            path.arc_to(2., 2., false, true, (5.24, 13.24));
            path.line_to(7., 15.);
        }),
        // Lucide `scaling`.
        ToolbarTool::Scale => ToolbarIcon::Drawn(|path| {
            path.move_to(12., 3.);
            path.line_to(5., 3.);
            path.arc_to(2., 2., false, false, (3., 5.));
            path.line_to(3., 19.);
            path.arc_to(2., 2., false, false, (5., 21.));
            path.line_to(19., 21.);
            path.arc_to(2., 2., false, false, (21., 19.));
            path.line_to(21., 12.);
            path.poly([(14., 15.), (9., 15.), (9., 10.)], false);
            path.poly([(16., 3.), (21., 3.), (21., 8.)], false);
            path.line((21., 3.), (9., 15.));
        }),
        // A tabbed frame in the Lucide idiom (Figma's section glyph).
        ToolbarTool::Section => ToolbarIcon::Drawn(|path| {
            path.poly(
                [
                    (3., 6.),
                    (8., 6.),
                    (10., 3.),
                    (21., 3.),
                    (21., 21.),
                    (3., 21.),
                ],
                true,
            );
            path.line((8., 6.), (21., 6.));
        }),
        // Lucide `scan` corners with an export-cut diagonal.
        ToolbarTool::Slice => ToolbarIcon::Drawn(|path| {
            path.move_to(3., 7.);
            path.line_to(3., 5.);
            path.arc_to(2., 2., false, true, (5., 3.));
            path.line_to(7., 3.);
            path.move_to(17., 3.);
            path.line_to(19., 3.);
            path.arc_to(2., 2., false, true, (21., 5.));
            path.line_to(21., 7.);
            path.move_to(21., 17.);
            path.line_to(21., 19.);
            path.arc_to(2., 2., false, true, (19., 21.));
            path.line_to(17., 21.);
            path.move_to(7., 21.);
            path.line_to(5., 21.);
            path.arc_to(2., 2., false, true, (3., 19.));
            path.line_to(3., 17.);
            path.line((7.5, 16.5), (16.5, 7.5));
        }),
        // Lucide `square`.
        ToolbarTool::Rectangle => {
            ToolbarIcon::Drawn(|path| path.rounded_rect(3., 3., 21., 21., 2.))
        }
        ToolbarTool::Line => ToolbarIcon::Drawn(|path| path.line((4., 20.), (20., 4.))),
        // Lucide `arrow-up-right`.
        ToolbarTool::Arrow => ToolbarIcon::Drawn(|path| {
            path.poly([(7., 7.), (17., 7.), (17., 17.)], false);
            path.line((7., 17.), (17., 7.));
        }),
        // Lucide `circle`.
        ToolbarTool::Ellipse => ToolbarIcon::Drawn(|path| path.circle(12., 12., 10.)),
        // Lucide `pentagon`.
        ToolbarTool::Polygon => ToolbarIcon::Drawn(|path| {
            path.poly(
                [(12., 2.2), (21.4, 9.), (17.9, 20.1), (6.1, 20.1), (2.6, 9.)],
                true,
            );
        }),
        // Lucide `image`.
        ToolbarTool::ImageVideo => ToolbarIcon::Drawn(|path| {
            path.rounded_rect(3., 3., 21., 21., 2.);
            path.circle(9., 9., 2.);
            path.move_to(21., 15.);
            path.line_to(17.91, 11.91);
            path.arc_to(2., 2., false, false, (15.09, 11.91));
            path.line_to(6., 21.);
        }),
        // Lucide `pen-tool`.
        ToolbarTool::Pen => ToolbarIcon::Drawn(|path| {
            path.poly([(15., 21.3), (12.7, 19.), (19., 12.7), (21.3, 15.)], true);
            path.poly(
                [(18., 13.), (16.4, 5.6), (2.5, 2.5), (5.6, 16.4), (13., 18.)],
                false,
            );
            path.line((2.3, 2.3), (7.29, 7.29));
            path.circle(11., 11., 2.);
        }),
        // Lucide `pencil`.
        ToolbarTool::Pencil => ToolbarIcon::Drawn(|path| {
            path.move_to(21.17, 6.81);
            path.arc_to(2.82, 2.82, false, false, (17.19, 2.83));
            path.line_to(3.5, 16.5);
            path.line_to(2.2, 21.8);
            path.line_to(7.5, 20.5);
            path.close();
            path.line((15., 5.), (19., 9.));
        }),
        // Pointer plus a path handle.
        ToolbarTool::PathSelect => ToolbarIcon::Drawn(|path| {
            draw_pointer(path, 0.72, 0., 0.);
            path.line((12.4, 12.4), (15.6, 15.6));
            draw_handle(path, 18.5, 18.5);
        }),
        // Lucide `spline`.
        ToolbarTool::NodeEdit => ToolbarIcon::Drawn(|path| {
            draw_handle(path, 19., 5.);
            draw_handle(path, 5., 19.);
            path.move_to(5., 17.);
            path.arc_to(12., 12., false, true, (17., 5.));
        }),
        // Lucide `type`.
        ToolbarTool::Text => ToolbarIcon::Drawn(|path| {
            path.poly([(4., 7.), (4., 4.), (20., 4.), (20., 7.)], false);
            path.line((9., 20.), (15., 20.));
            path.line((12., 4.), (12., 20.));
        }),
        // A `type` glyph flowing onto a spline.
        ToolbarTool::TextPath => ToolbarIcon::Drawn(|path| {
            path.line((3.5, 4.), (12.5, 4.));
            path.line((8., 4.), (8., 11.5));
            path.move_to(3., 19.);
            path.cubic_to((21., 14.), (8., 10.), (14., 22.));
            draw_handle(path, 19.5, 11.5);
        }),
        // Lucide `message-square`.
        ToolbarTool::Comment => ToolbarIcon::Drawn(|path| {
            path.move_to(21., 15.);
            path.arc_to(2., 2., false, true, (19., 17.));
            path.line_to(7., 17.);
            path.line_to(3., 21.);
            path.line_to(3., 5.);
            path.arc_to(2., 2., false, true, (5., 3.));
            path.line_to(19., 3.);
            path.arc_to(2., 2., false, true, (21., 5.));
            path.close();
        }),
        // Lucide `sticky-note`.
        ToolbarTool::Annotation => ToolbarIcon::Drawn(|path| {
            path.move_to(16., 3.);
            path.line_to(5., 3.);
            path.arc_to(2., 2., false, false, (3., 5.));
            path.line_to(3., 19.);
            path.arc_to(2., 2., false, false, (5., 21.));
            path.line_to(19., 21.);
            path.arc_to(2., 2., false, false, (21., 19.));
            path.line_to(21., 9.);
            path.close();
            path.move_to(15., 3.);
            path.line_to(15., 7.);
            path.arc_to(2., 2., false, false, (17., 9.));
            path.line_to(21., 9.);
        }),
        // Lucide `ruler`.
        ToolbarTool::Measure => ToolbarIcon::Drawn(|path| {
            path.move_to(21.3, 15.3);
            path.arc_to(2.4, 2.4, false, true, (21.3, 18.7));
            path.line_to(18.7, 21.3);
            path.arc_to(2.4, 2.4, false, true, (15.3, 21.3));
            path.line_to(2.7, 8.7);
            path.arc_to(2.4, 2.4, false, true, (2.7, 5.3));
            path.line_to(5.3, 2.7);
            path.arc_to(2.4, 2.4, false, true, (8.7, 2.7));
            path.close();
            path.line((14.5, 12.5), (16.5, 10.5));
            path.line((11.5, 9.5), (13.5, 7.5));
            path.line((8.5, 6.5), (10.5, 4.5));
            path.line((17.5, 15.5), (19.5, 13.5));
        }),
        // Lucide `sparkles`.
        ToolbarTool::Actions => ToolbarIcon::Drawn(|path| {
            draw_sparkle(path, 12., 12.5, 9.);
            path.plus(20., 4.5, 1.5);
            path.plus(4., 19.5, 1.5);
        }),
        // Lucide `pipette`.
        ToolbarTool::ColorPicker => ToolbarIcon::Drawn(|path| {
            path.poly([(2., 22.), (3., 21.), (6., 21.), (15., 12.)], false);
            path.poly([(3., 21.), (3., 18.), (12., 9.)], false);
            path.move_to(15., 6.);
            path.line_to(18.4, 2.6);
            path.arc_to(2.1, 2.1, true, true, (21.4, 5.6));
            path.line_to(18., 9.);
            path.line_to(18.4, 9.4);
            path.arc_to(2.1, 2.1, true, true, (15.4, 12.4));
            path.line_to(11.6, 8.6);
            path.arc_to(2.1, 2.1, true, true, (14.6, 5.6));
            path.close();
        }),
        // Lucide `code-xml`.
        ToolbarTool::Code => ToolbarIcon::Drawn(draw_code_xml),
        // Lucide `variable`.
        ToolbarTool::Variables => ToolbarIcon::Drawn(|path| {
            path.move_to(8., 21.);
            path.cubic_to((4., 12.), (8., 21.), (4., 18.));
            path.cubic_to((8., 3.), (4., 6.), (8., 3.));
            path.move_to(16., 3.);
            path.cubic_to((20., 12.), (16., 3.), (20., 6.));
            path.cubic_to((16., 21.), (20., 18.), (16., 21.));
            path.line((15., 9.), (9., 15.));
            path.line((9., 9.), (15., 15.));
        }),
        // Pointer plus a keyframe diamond.
        ToolbarTool::MotionSelect => ToolbarIcon::Drawn(|path| {
            draw_pointer(path, 0.72, 0., 0.);
            path.diamond(18.5, 18.5, 3.2);
        }),
        // Keyframe diamond plus.
        ToolbarTool::AddKeyframe => ToolbarIcon::Drawn(|path| {
            path.diamond(9.5, 12.5, 7.);
            path.plus(19., 6., 3.);
        }),
        // A spline from a start dot to a keyframe diamond.
        ToolbarTool::MotionPath => ToolbarIcon::Drawn(|path| {
            path.circle(4.5, 18.5, 2.2);
            path.diamond(19.5, 5.5, 2.8);
            path.move_to(6., 17.);
            path.cubic_to((18., 7.), (4., 7.), (20., 17.));
        }),
        // A sparkle over an eased-motion arrow.
        ToolbarTool::AnimationStyle => ToolbarIcon::Drawn(|path| {
            draw_sparkle(path, 8., 8., 6.);
            path.move_to(4., 21.);
            path.cubic_to((21., 15.), (9., 21.), (15., 15.));
            path.poly([(18., 12.), (21., 15.), (18., 18.)], false);
        }),
        // Lucide `clock` with a comment bubble.
        ToolbarTool::TimeComment => ToolbarIcon::Drawn(|path| {
            path.circle(9., 9., 7.);
            path.poly([(9., 5.), (9., 9.), (11.5, 10.5)], false);
            path.poly(
                [
                    (15., 15.),
                    (22., 15.),
                    (22., 20.),
                    (18., 20.),
                    (16., 22.),
                    (16., 20.),
                    (15., 20.),
                ],
                true,
            );
        }),
        // Lucide `refresh-cw` around a keyframe diamond.
        ToolbarTool::AutoKeyframe => ToolbarIcon::Drawn(|path| {
            path.move_to(3., 12.);
            path.arc_to(9., 9., false, true, (12., 3.));
            path.arc_to(9.75, 9.75, false, true, (18.74, 5.74));
            path.line_to(21., 8.);
            path.poly([(21., 3.), (21., 8.), (16., 8.)], false);
            path.move_to(21., 12.);
            path.arc_to(9., 9., false, true, (12., 21.));
            path.arc_to(9.75, 9.75, false, true, (5.26, 18.26));
            path.line_to(3., 16.);
            path.poly([(8., 16.), (3., 16.), (3., 21.)], false);
            path.diamond(12., 12., 3.);
        }),
        // Lucide `circle-play`.
        ToolbarTool::PlayPreview => ToolbarIcon::Drawn(|path| {
            path.circle(12., 12., 10.);
            path.poly([(10., 8.), (16., 12.), (10., 16.)], true);
        }),
    }
}

/// Lucide `code-xml`.
fn draw_code_xml(path: &mut IconPath) {
    path.poly([(18., 16.), (22., 12.), (18., 8.)], false);
    path.poly([(6., 8.), (2., 12.), (6., 16.)], false);
    path.line((14.5, 4.), (9.5, 20.));
}

/// Lucide `mouse-pointer-2`, scaled by `scale` about the grid origin and
/// offset by (`dx`, `dy`).
fn draw_pointer(path: &mut IconPath, scale: f32, dx: f32, dy: f32) {
    let p = |x: f32, y: f32| (x * scale + dx, y * scale + dy);
    let (sx, sy) = p(4.3, 4.3);
    path.move_to(sx, sy);
    let (x, y) = p(20.7, 10.9);
    path.line_to(x, y);
    let (x, y) = p(14.55, 12.95);
    path.line_to(x, y);
    path.arc_to(2. * scale, 2. * scale, false, false, p(12.95, 14.55));
    let (x, y) = p(10.9, 20.7);
    path.line_to(x, y);
    path.close();
}

/// A Lucide `spline`-style path handle: a 4-unit rounded square.
fn draw_handle(path: &mut IconPath, center_x: f32, center_y: f32) {
    path.rounded_rect(
        center_x - 2.,
        center_y - 2.,
        center_x + 2.,
        center_y + 2.,
        1.,
    );
}

/// A four-point Lucide `sparkles` star of `radius` about (`cx`, `cy`).
fn draw_sparkle(path: &mut IconPath, cx: f32, cy: f32, radius: f32) {
    let waist = radius * 0.22;
    path.move_to(cx, cy - radius);
    path.curve_to((cx + radius, cy), (cx + waist, cy - waist));
    path.curve_to((cx, cy + radius), (cx + waist, cy + waist));
    path.curve_to((cx - radius, cy), (cx - waist, cy + waist));
    path.curve_to((cx, cy - radius), (cx - waist, cy - waist));
    path.close();
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};
    use gpui_component::IconNamed as _;

    use super::*;

    fn assert_renders(label: &str, icon: ToolbarIcon) {
        match icon {
            ToolbarIcon::Asset(name) => {
                assert!(
                    name.path().ends_with(".svg"),
                    "{label} should map to an SVG asset"
                );
            }
            ToolbarIcon::Drawn(draw) => {
                let mut path = IconPath::lucide(point(px(0.), px(0.)), 16.);
                draw(&mut path);
                assert!(path.build().is_some(), "failed to build the {label} icon");
            }
        }
    }

    #[test]
    fn every_tool_and_mode_builds_a_valid_path_or_names_an_asset() {
        for &tool in ToolbarTool::ALL {
            assert_renders(tool.label(), tool_icon(tool));
        }
        for &mode in ToolbarMode::ALL {
            assert_renders(mode.label(), mode_icon(mode));
        }
    }

    #[test]
    fn lucide_assets_back_the_tools_that_have_one() {
        for (tool, asset) in [
            (ToolbarTool::Frame, "icons/frame.svg"),
            (ToolbarTool::Star, "icons/star.svg"),
            (ToolbarTool::Resources, "icons/layout-dashboard.svg"),
            (ToolbarTool::Inspect, "icons/search.svg"),
            (ToolbarTool::ReadyForDev, "icons/circle-check.svg"),
        ] {
            match tool_icon(tool) {
                ToolbarIcon::Asset(name) => assert_eq!(name.path(), asset),
                ToolbarIcon::Drawn(_) => panic!("{} should use {asset}", tool.label()),
            }
        }
        assert!(matches!(
            mode_icon(ToolbarMode::Design),
            ToolbarIcon::Asset(IconName::Inspector)
        ));
    }
}
