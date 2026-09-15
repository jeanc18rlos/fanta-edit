//! Lucide mappings used by the Layers tree.

use gpui::{AnyElement, Hsla};

use crate::{
    atoms::{LucideIcon, render_lucide_icon},
    toolbar::{ToolbarTool, render_tool_icon},
};

use super::super::LayersPanelNodeKind;

pub(super) fn layer_kind_icon(kind: LayersPanelNodeKind, color: Hsla, size: f32) -> AnyElement {
    use LayersPanelNodeKind as Kind;

    let icon = match kind {
        Kind::Frame => LucideIcon::Frame,
        Kind::Group => LucideIcon::Group,
        Kind::Section => LucideIcon::PanelTop,
        Kind::Component => LucideIcon::Component,
        Kind::ComponentSet => LucideIcon::LayoutGrid,
        Kind::Instance => LucideIcon::Diamond,
        Kind::Text => LucideIcon::TypeIcon,
        Kind::Image => LucideIcon::Image,
        Kind::Video => LucideIcon::Video,
        Kind::Mask => LucideIcon::Scan,
        Kind::BooleanOperation => LucideIcon::Combine,
        Kind::Star => LucideIcon::Star,
        Kind::Other => LucideIcon::Ellipsis,
        Kind::Rectangle => {
            return render_tool_icon(ToolbarTool::Rectangle, color, size);
        }
        Kind::Ellipse => return render_tool_icon(ToolbarTool::Ellipse, color, size),
        Kind::Polygon => return render_tool_icon(ToolbarTool::Polygon, color, size),
        Kind::Line => return render_tool_icon(ToolbarTool::Line, color, size),
        Kind::Arrow => return render_tool_icon(ToolbarTool::Arrow, color, size),
        Kind::Vector => return render_tool_icon(ToolbarTool::NodeEdit, color, size),
        Kind::Slice => return render_tool_icon(ToolbarTool::Slice, color, size),
        Kind::Pen => return render_tool_icon(ToolbarTool::Pen, color, size),
        Kind::Pencil => return render_tool_icon(ToolbarTool::Pencil, color, size),
    };
    render_lucide_icon(icon, color, size)
}

pub(super) fn render_lock_icon(locked: bool, color: Hsla, size: f32) -> AnyElement {
    render_lucide_icon(
        if locked {
            LucideIcon::Lock
        } else {
            LucideIcon::LockOpen
        },
        color,
        size,
    )
}
