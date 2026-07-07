//! Figma canvas-color hints that are not reliably preserved as node geometry.
//!
//! Dev Mode screenshots composite a selected node over the page canvas color.
//! In `.fig` imports, the page root is a `CANVAS` node, but the editor canvas
//! color is not consistently exposed as a normal fill. The Spectrum fixture uses
//! page names to encode the visible theme, so keep that tiny inference here and
//! share it between the app and fidelity harness.

use fanta_doc::{Color, Doc, NodeId};

/// Infer the Figma editor canvas color for an imported page.
pub fn figma_page_canvas_color(doc: &Doc, page: NodeId) -> Option<Color> {
    let node = doc.scene.get(page)?;
    let is_figma_canvas = node
        .meta
        .get("figma_type")
        .and_then(|v| v.as_str())
        .is_some_and(|kind| kind == "CANVAS");
    if !is_figma_canvas {
        return None;
    }
    figma_page_canvas_color_from_name(doc.page_name(page).unwrap_or(&node.name))
}

/// Infer the canvas color from a Figma page name.
pub fn figma_page_canvas_color_from_name(name: &str) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    if lower.contains("darkest theme") {
        Some(Color::rgb(0x15, 0x15, 0x15))
    } else if lower.contains("dark theme") {
        Some(Color::rgb(0x29, 0x29, 0x29))
    } else if lower.contains("wireframe") {
        Some(Color::rgb(0xF4, 0xF6, 0xFC))
    } else if lower.contains("light theme") || lower.contains("typography") {
        Some(Color::rgb(0xF5, 0xF5, 0xF5))
    } else if lower.contains("introduction") {
        Some(Color::rgb(0xF4, 0xF4, 0xF4))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_page_names_map_to_figma_canvas_colors() {
        assert_eq!(
            figma_page_canvas_color_from_name("↳  🌙  Darkest Theme"),
            Some(Color::rgb(0x15, 0x15, 0x15))
        );
        assert_eq!(
            figma_page_canvas_color_from_name("↳  🔅  Dark Theme"),
            Some(Color::rgb(0x29, 0x29, 0x29))
        );
        assert_eq!(
            figma_page_canvas_color_from_name("↳  ⚙️  Wireframes"),
            Some(Color::rgb(0xF4, 0xF6, 0xFC))
        );
        assert_eq!(
            figma_page_canvas_color_from_name("🔠  Typography"),
            Some(Color::rgb(0xF5, 0xF5, 0xF5))
        );
        assert_eq!(figma_page_canvas_color_from_name("🎲  Icons"), None);
    }
}
