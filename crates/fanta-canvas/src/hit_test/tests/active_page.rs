//! Active-page scoping across overlapping shared-origin pages (the `.fig`
//! multi-page layout) for click, marquee, and deep queries.

use super::*;
// ---- active-page scoping ------------------------------------------------

/// Build a two-page doc where each page is a root group holding a single
/// rect that overlaps the other page's rect in world space (the `.fig`
/// shared-origin layout). Returns `(doc, page_a, rect_a, page_b, rect_b)`.
fn two_overlapping_pages() -> (Doc, NodeId, NodeId, NodeId, NodeId) {
    let mut doc = Doc::new();

    // Page A: group with a rect covering [-20,-20 .. 20,20].
    let ga = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_a = ga.id;
    doc.apply(Operation::create_node(ga)).unwrap();
    let mut ra = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -20.0,
        -20.0,
        40.0,
        40.0,
        Color::WHITE,
    )));
    ra.parent = Some(page_a);
    let rect_a = ra.id;
    doc.apply(Operation::create_node(ra)).unwrap();

    // Page B: created AFTER A so it paints on top — a different group with a
    // rect at the SAME world region. Without page scoping this rect would
    // win every hit even when A is the active page.
    let gb = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_b = gb.id;
    doc.apply(Operation::create_node(gb)).unwrap();
    let mut rb = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -20.0,
        -20.0,
        40.0,
        40.0,
        Color::BLACK,
    )));
    rb.parent = Some(page_b);
    let rect_b = rb.id;
    doc.apply(Operation::create_node(rb)).unwrap();

    doc.add_page(page_a);
    doc.add_page(page_b);
    (doc, page_a, rect_a, page_b, rect_b)
}

#[test]
fn on_active_page_helper_classifies_by_root_ancestor() {
    let (doc, page_a, rect_a, page_b, rect_b) = two_overlapping_pages();
    // The page node itself counts as on-page.
    assert!(on_active_page(&doc.scene, page_a, page_a));
    // A descendant's root ancestor is its page.
    assert!(on_active_page(&doc.scene, rect_a, page_a));
    assert!(on_active_page(&doc.scene, rect_b, page_b));
    // Cross-page rejection.
    assert!(!on_active_page(&doc.scene, rect_b, page_a));
    assert!(!on_active_page(&doc.scene, rect_a, page_b));
    assert!(!on_active_page(&doc.scene, page_b, page_a));
}

#[test]
fn nested_focus_root_scopes_to_its_subtree() {
    // A component master is a nested subtree (under a hidden Components page),
    // not a top-level page. Scoping hits to that master (the focus root in a
    // component-edit tab) must include its descendants — so its inner layers
    // are selectable, not just the master root.
    let mut doc = Doc::new();
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    doc.apply(Operation::create_node(page)).unwrap();
    doc.add_page(page_id);

    let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    master.parent = Some(page_id);
    let master_id = master.id;
    doc.apply(Operation::create_node(master)).unwrap();

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::WHITE,
    )));
    child.parent = Some(master_id);
    let child_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();

    // A sibling directly under the page (not under the master).
    let mut sibling = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        50.0,
        0.0,
        10.0,
        10.0,
        Color::BLACK,
    )));
    sibling.parent = Some(page_id);
    let sibling_id = sibling.id;
    doc.apply(Operation::create_node(sibling)).unwrap();

    // Scoped to the master: self + descendant in scope, page-sibling out.
    assert!(on_active_page(&doc.scene, master_id, master_id));
    assert!(on_active_page(&doc.scene, child_id, master_id));
    assert!(!on_active_page(&doc.scene, sibling_id, master_id));
    // Scoped to the page: everything under it (incl. the nested child) in scope.
    assert!(on_active_page(&doc.scene, child_id, page_id));
    assert!(on_active_page(&doc.scene, sibling_id, page_id));
}

#[test]
fn click_scoped_to_active_page_never_hits_other_page() {
    let (doc, page_a, rect_a, page_b, rect_b) = two_overlapping_pages();
    let p = DVec2::ZERO; // inside both rects (they overlap).

    // Active page A → must select A's rect, even though B paints on top.
    assert_eq!(
        hit_test(&doc.scene, p, HitPrecision::Bounds, Some(page_a)),
        Some(rect_a)
    );
    // Active page B → must select B's rect.
    assert_eq!(
        hit_test(&doc.scene, p, HitPrecision::Bounds, Some(page_b)),
        Some(rect_b)
    );
    // No scoping → a hit from either page (both rects overlap the point);
    // the point is which page-scope changes the answer, above. Unscoped is
    // back-compat: it just resolves the topmost across all roots.
    let unscoped = hit_test(&doc.scene, p, HitPrecision::Bounds, None);
    assert!(
        unscoped == Some(rect_a) || unscoped == Some(rect_b),
        "unscoped hit must be one of the page rects, got {unscoped:?}"
    );
}

#[test]
fn marquee_scoped_to_active_page_collects_only_that_page() {
    let (doc, page_a, rect_a, page_b, rect_b) = two_overlapping_pages();
    // A marquee fully containing both overlapping rects.
    let r = Bounds::from_xywh(-50.0, -50.0, 100.0, 100.0);

    let hits_a = hit_test_within(&doc.scene, r, MarqueeMode::Contains, Some(page_a));
    assert_eq!(hits_a.as_slice(), &[rect_a]);

    let hits_b = hit_test_within(&doc.scene, r, MarqueeMode::Contains, Some(page_b));
    assert_eq!(hits_b.as_slice(), &[rect_b]);

    // No scoping collects both leaves (groups are never reported).
    let hits_all = hit_test_within(&doc.scene, r, MarqueeMode::Contains, None);
    assert_eq!(hits_all.len(), 2);
    assert!(hits_all.contains(&rect_a));
    assert!(hits_all.contains(&rect_b));
}

#[test]
fn deep_hits_scoped_to_active_page() {
    let (doc, page_a, rect_a, page_b, rect_b) = two_overlapping_pages();
    let p = DVec2::ZERO;

    let deep_a = hit_test_deep(&doc.scene, p, HitPrecision::Bounds, Some(page_a));
    assert_eq!(deep_a.as_slice(), &[rect_a]);

    let deep_b = hit_test_deep(&doc.scene, p, HitPrecision::Bounds, Some(page_b));
    assert_eq!(deep_b.as_slice(), &[rect_b]);

    // Unscoped sees both rects (z-order between the two pages is
    // immaterial here — page scoping, above, is what this test guards).
    let deep_all = hit_test_deep(&doc.scene, p, HitPrecision::Bounds, None);
    assert_eq!(deep_all.len(), 2);
    assert!(deep_all.contains(&rect_a));
    assert!(deep_all.contains(&rect_b));
}

#[test]
fn page_root_with_canvas_background_is_not_a_hit_target() {
    use fanta_doc::{Fill, GroupNode, Transform2D};
    // Imported pages carry the Figma canvas background as the page group's
    // fill, which makes them "frame surfaces". Clicking the backdrop must
    // still deselect (hit nothing), like Figma.
    let mut doc = Doc::new();
    let mut page_node = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([1000.0, 1000.0]),
        background: Some(Fill::solid(Color::WHITE)),
        ..GroupNode::default()
    }));
    page_node.transform = Transform2D::translation(-500.0, -500.0);
    let page = page_node.id;
    doc.apply(Operation::create_node(page_node)).unwrap();
    let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        40.0,
        40.0,
        Color::BLACK,
    )));
    rect.parent = Some(page);
    let rect_id = rect.id;
    doc.apply(Operation::create_node(rect)).unwrap();
    doc.add_page(page);

    // Over the child: the child wins.
    assert_eq!(
        hit_test(
            &doc.scene,
            DVec2::new(-480.0, -480.0),
            HitPrecision::Bounds,
            Some(page)
        ),
        Some(rect_id)
    );
    // Over empty page background: nothing, even though the page group has a
    // background fill.
    assert_eq!(
        hit_test(
            &doc.scene,
            DVec2::new(300.0, 300.0),
            HitPrecision::Bounds,
            Some(page)
        ),
        None
    );
    // A NESTED focus root (component-master editing) is not excluded: its
    // subtree stays hittable when it is the scope.
    let mut master = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        background: Some(Fill::solid(Color::WHITE)),
        ..GroupNode::default()
    }));
    master.parent = Some(page);
    master.transform = Transform2D::translation(200.0, 200.0);
    let master_id = master.id;
    doc.apply(Operation::create_node(master)).unwrap();
    let mut inner = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        10.0,
        10.0,
        30.0,
        30.0,
        Color::BLACK,
    )));
    inner.parent = Some(master_id);
    let inner_id = inner.id;
    doc.apply(Operation::create_node(inner)).unwrap();
    assert_eq!(
        hit_test(
            &doc.scene,
            DVec2::new(-280.0, -280.0),
            HitPrecision::Bounds,
            Some(master_id)
        ),
        Some(inner_id)
    );
}
