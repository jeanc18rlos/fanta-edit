//! The pure Design-panel snapshot: `FigDocument` state in, one
//! `DesignPanelViewData` out.
//!
//! `DesignPanel` accepts either a stream of granular setters or one complete
//! snapshot through `set_view_data`. Building the snapshot is the form this
//! seam wants either way: every future catalog becomes one more field
//! assignment here instead of one more setter call inside
//! `refresh_gpui_design`.
//!
//! Only the fields this adapter actually owns are populated — the inspection
//! context, the property value states and the Page projection. Everything
//! else is panel-owned state the adapter never wrote, which is why
//! `FigView::refresh_gpui_design` still *applies* this snapshot through the
//! granular setters instead of `set_view_data`: a complete snapshot clears
//! whatever it omits, and two of those omissions are live state this adapter
//! would be throwing away.
//!
//! `navigation` is one: the active surface is written straight to the panel by
//! `SurfaceChangeRequested`, so a snapshot carrying the default would send the
//! user back to Properties on their next selection change.
//! `page_node_fallback` is the other: this adapter seeds it as
//! `fig-gpui-design-empty` and parses node ids back out of it, while
//! `DesignPanelViewData::new` substitutes the library's own `design-panel-page`
//! placeholder.
//!
//! The flip to one `set_view_data` call belongs with the work that makes this
//! adapter own those fields, not before it. Building the snapshot already buys
//! the thing that motivated the change: a new catalog is a field assignment
//! here rather than another setter call in `refresh_gpui_design`.
//!
//! Nothing in this module touches gpui: no `Context`, no entity, no `notify`.

use fanta_doc::NodeId;
use fanta_gpui::design::{
    DesignColor, DesignPageBackground, DesignPageViewData, DesignPanelInspectionContext,
    DesignPanelMultipleSelection, DesignPanelParentLayout, DesignPanelPermissions,
    DesignPanelTarget, DesignPanelTargetedSelectionHeader, DesignPanelViewData,
};

use super::design::{
    aggregate_selection, bound_states, design_color, design_node, member_node, parent_layout_for,
    selection_header_for_doc,
};
use crate::document::FigDocument;
use crate::properties_snapshot::{master_roots, page_section};

/// Builds the Design-panel snapshot for one document state.
///
/// `previous_page` is the panel's current Page projection, read by the caller
/// from `DesignPanel::page_view_data()`. Page view data is only derivable for
/// an empty selection: the granular echo simply did not call
/// `set_page_view_data` while a node was selected, so the panel kept the
/// projection it already had. Threading the live value back in keeps that
/// retention a property of the snapshot rather than of the caller's control
/// flow — re-applying an unchanged projection is a no-op in the panel, and a
/// complete-snapshot handoff (which clears whatever it omits) would need the
/// value present to behave the same way.
pub(crate) fn build_design_view_data(
    document: &FigDocument,
    selection: &[NodeId],
    page_index: Option<usize>,
    editable: bool,
    previous_page: Option<DesignPageViewData>,
) -> DesignPanelViewData {
    let doc = &document.doc;
    let masters = master_roots(&doc.components);
    let permissions = if editable {
        DesignPanelPermissions::editor()
    } else {
        DesignPanelPermissions::viewer()
    };
    let (context, states) = match selection {
        [] => (DesignPanelInspectionContext::page(permissions), Vec::new()),
        [id] => match design_node(document, *id, &masters) {
            Some((node, mut states)) => {
                // Bindings last: a bound property keeps its binding
                // state even where the geometry states also speak.
                states.extend(bound_states(doc, *id));
                (
                    DesignPanelInspectionContext::single(
                        node,
                        parent_layout_for(doc, *id),
                        permissions,
                    ),
                    states,
                )
            }
            None => (DesignPanelInspectionContext::page(permissions), Vec::new()),
        },
        ids => {
            let (aggregate, states) = aggregate_selection(document, ids, &masters);
            let mut members = ids.iter().skip(1).map(|id| member_node(doc, *id, &masters));
            let second = members
                .next()
                .unwrap_or_else(|| member_node(doc, ids[0], &masters));
            // Every member's parent layout must agree or the context
            // degrades to Mixed, matching the panel's contract.
            let mut layouts = ids.iter().map(|id| parent_layout_for(doc, *id));
            let first_layout = layouts.next().unwrap_or(DesignPanelParentLayout::Canvas);
            let parent_layout = if layouts.all(|layout| layout == first_layout) {
                first_layout
            } else {
                DesignPanelParentLayout::Mixed
            };
            (
                DesignPanelInspectionContext::multiple(
                    DesignPanelMultipleSelection::with_remaining(aggregate, second, members),
                    parent_layout,
                    permissions,
                ),
                states,
            )
        }
    };
    let page_data = selection.is_empty().then(|| {
        let page = page_section(document, page_index);
        let background = match page.background {
            Some(crate::properties_snapshot::PageBackgroundValue::Solid(color)) => {
                DesignPageBackground::new(design_color(color))
            }
            Some(crate::properties_snapshot::PageBackgroundValue::Other(_)) => {
                DesignPageBackground::new(DesignColor::WHITE)
                    .read_only("Non-solid page backgrounds are edited on canvas")
            }
            Some(crate::properties_snapshot::PageBackgroundValue::None) | None => {
                DesignPageBackground::new(design_color(
                    crate::properties_ops::DEFAULT_PAGE_BACKGROUND,
                ))
            }
        };
        let page_id = page
            .id
            .map(|root| root.to_string())
            .unwrap_or_else(|| format!("page-{}", page_index.unwrap_or(0)));
        DesignPageViewData::canonical(page_id, background)
    });

    let mut view_data = DesignPanelViewData::new(context);
    view_data.property_states = states.into_iter().collect();
    view_data.projections.page = page_data.or(previous_page);
    view_data.projections.selection_header = selection_header_for_doc(
        doc,
        selection,
        page_index
            .and_then(|index| doc.pages().get(index).copied())
            .or_else(|| doc.active_page()),
        editable,
    )
    .map(|header| {
        DesignPanelTargetedSelectionHeader::new(
            DesignPanelTarget::Nodes {
                node_ids: selection
                    .iter()
                    .map(ToString::to_string)
                    .map(Into::into)
                    .collect(),
            },
            header,
        )
    });
    view_data
}
