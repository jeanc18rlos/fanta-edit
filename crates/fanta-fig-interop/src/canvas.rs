//! The Figma editor canvas color of an imported page.
//!
//! Dev Mode screenshots composite a selected node over the page canvas color.
//! Figma stores that color on the `CANVAS` NodeChange itself
//! (`backgroundColor` / `backgroundOpacity` / `backgroundEnabled`), which the
//! importer reads into the page group's `background` fill — so this helper just
//! surfaces the imported value. (An earlier version *guessed* the color from
//! hardcoded Spectrum page-name substrings, which painted every non-Spectrum
//! file's pages on the wrong backdrop.)

use fanta_doc::{Color, Doc, NodeData, NodeId, style::Fill};

/// The Figma editor canvas color for an imported page: the solid background
/// fill the importer read from the page's `CANVAS.backgroundColor`. `None` for
/// a non-CANVAS node, a page with no background, or a non-solid background.
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
    let NodeData::Group(group) = &node.data else {
        return None;
    };
    match group.background.as_ref()? {
        Fill::Solid { color } => Some(*color),
        _ => None,
    }
}
