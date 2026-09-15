//! Lucide icon mapping for toolbar tools and modes.
//!
//! This module contains semantic mappings only. Geometry is owned by the
//! pinned Lucide catalog and rendered by the shared atom.

use gpui::{AnyElement, Hsla, IntoElement as _, Styled as _, px};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::atoms::{LucideIcon, render_lucide_icon};

use super::{ToolbarMode, ToolbarTool};

pub(crate) fn render_tool_icon(tool: ToolbarTool, color: Hsla, size: f32) -> AnyElement {
    render_lucide_icon(tool_icon(tool), color, size)
}

pub(crate) fn render_mode_icon(mode: ToolbarMode, color: Hsla, size: f32) -> AnyElement {
    render_lucide_icon(mode_icon(mode), color, size)
}

/// Renders one of gpui-component's bundled Lucide assets.
pub(crate) fn render_icon_asset(icon: IconName, color: Hsla, size: f32) -> AnyElement {
    Icon::new(icon)
        .with_size(px(size))
        .text_color(color)
        .into_any_element()
}

pub(crate) const fn mode_icon(mode: ToolbarMode) -> LucideIcon {
    match mode {
        ToolbarMode::Design => LucideIcon::SquareDashedMousePointer,
        ToolbarMode::Motion => LucideIcon::Clapperboard,
        ToolbarMode::Dev => LucideIcon::CodeXml,
    }
}

pub(crate) const fn tool_icon(tool: ToolbarTool) -> LucideIcon {
    match tool {
        ToolbarTool::Move => LucideIcon::MousePointer2,
        ToolbarTool::Hand => LucideIcon::Hand,
        ToolbarTool::Scale => LucideIcon::Scaling,
        ToolbarTool::Frame => LucideIcon::Frame,
        ToolbarTool::Section => LucideIcon::PanelTop,
        ToolbarTool::Slice => LucideIcon::ScanLine,
        ToolbarTool::Rectangle => LucideIcon::Square,
        ToolbarTool::Line => LucideIcon::Slash,
        ToolbarTool::Arrow => LucideIcon::ArrowUpRight,
        ToolbarTool::Ellipse => LucideIcon::Circle,
        ToolbarTool::Polygon => LucideIcon::Pentagon,
        ToolbarTool::Star => LucideIcon::Star,
        ToolbarTool::ImageVideo => LucideIcon::Image,
        ToolbarTool::Pen => LucideIcon::PenTool,
        ToolbarTool::Pencil => LucideIcon::Pencil,
        ToolbarTool::PathSelect => LucideIcon::MousePointerClick,
        ToolbarTool::NodeEdit => LucideIcon::Spline,
        ToolbarTool::Text => LucideIcon::TypeIcon,
        ToolbarTool::TextPath => LucideIcon::Baseline,
        ToolbarTool::Comment => LucideIcon::MessageSquare,
        ToolbarTool::Annotation => LucideIcon::StickyNote,
        ToolbarTool::Measure => LucideIcon::Ruler,
        ToolbarTool::Resources => LucideIcon::LayoutDashboard,
        ToolbarTool::Actions => LucideIcon::Sparkles,
        ToolbarTool::Inspect => LucideIcon::Search,
        ToolbarTool::ColorPicker => LucideIcon::Pipette,
        ToolbarTool::Code => LucideIcon::CodeXml,
        ToolbarTool::Variables => LucideIcon::Variable,
        ToolbarTool::ReadyForDev => LucideIcon::CircleCheck,
        ToolbarTool::MotionSelect => LucideIcon::MousePointer2,
        ToolbarTool::AddKeyframe => LucideIcon::DiamondPlus,
        ToolbarTool::MotionPath => LucideIcon::Route,
        ToolbarTool::AnimationStyle => LucideIcon::WandSparkles,
        ToolbarTool::TimeComment => LucideIcon::Clock,
        ToolbarTool::AutoKeyframe => LucideIcon::RefreshCw,
        ToolbarTool::PlayPreview => LucideIcon::CirclePlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_toolbar_item_maps_to_an_official_lucide_icon() {
        for &tool in ToolbarTool::ALL {
            assert!(!tool_icon(tool).svg_str().is_empty());
        }
        for &mode in ToolbarMode::ALL {
            assert!(!mode_icon(mode).svg_str().is_empty());
        }
    }
}
