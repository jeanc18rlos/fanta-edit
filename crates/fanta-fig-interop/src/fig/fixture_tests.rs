//! Opt-in diagnostics that import a *real* Figma `.fig` export. Both are
//! `#[ignore]`d unless `FANTA_FIG_FIXTURE` points at a `.fig` on disk (we
//! cannot commit the 20 MB copyrighted Spectrum fixture), so they don't run in
//! the default suite. Split out of the container `tests` module because each is
//! a large, self-contained measurement harness over the public import pipeline.

use super::*;

/// Opt-in instance-fidelity diagnostic against a real Figma export.
/// Reports the STEP-1 measurement numbers for the component-instance
/// rendering work: how many instances resolve to a def, how many of those
/// defs have a non-empty master subtree in the scene, and how many
/// `expand_instance` calls produce >1 node. Skipped unless
/// `FANTA_FIG_FIXTURE` points at a `.fig`. Run with:
///   FANTA_FIG_FIXTURE=/path/to/file.fig \
///     cargo test -p fanta-fig-interop instance_resolution -- --ignored --nocapture
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn instance_resolution_diagnostic() {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping");
        return;
    };
    let bytes = std::fs::read(&path).expect("read fixture");
    let doc = read_fig(&bytes).expect("real .fig must parse");
    let (mapped, report, _assets) = crate::mapping::fig_to_doc(&doc).expect("map to doc");

    use fanta_doc::node::NodeData;
    use fanta_doc::resolve::expand_instance;

    // Walk every node in every root subtree and tally instances.
    let mut total_instances = 0usize;
    let mut resolved_to_def = 0usize; // instance.component is in defs
    let mut resolved_to_set = 0usize; // instance.component is a set id
    let mut dangling = 0usize; // neither def nor set (placeholder/0)
    let mut resolved_with_nonempty_master = 0usize;
    let mut resolved_with_empty_master = 0usize;
    let mut expand_gt1 = 0usize; // expand_instance returned > 1 node
    let mut expand_eq1 = 0usize; // exactly 1 (root only)
    let mut expand_eq0 = 0usize; // empty

    for root in mapped.scene.roots().to_vec() {
        for id in mapped.scene.descendants_of(root) {
            let Some(node) = mapped.scene.get(id) else {
                continue;
            };
            let NodeData::Instance(inst) = &node.data else {
                continue;
            };
            total_instances += 1;
            let is_def = mapped.components.defs.contains_key(&inst.component);
            let is_set = mapped.components.sets.contains_key(&inst.component);
            if is_def {
                resolved_to_def += 1;
                // Does the def's master root exist and have >= 1 descendant?
                if let Some(def) = mapped.components.def(inst.component) {
                    let descendant_count = mapped.scene.descendants_of(def.root).count();
                    // descendants_of yields the root itself first, so a
                    // non-empty master has count >= 2.
                    if mapped.scene.get(def.root).is_some() && descendant_count >= 2 {
                        resolved_with_nonempty_master += 1;
                    } else {
                        resolved_with_empty_master += 1;
                    }
                }
            } else if is_set {
                resolved_to_set += 1;
            } else {
                dangling += 1;
            }

            let expanded = expand_instance(&mapped.scene, &mapped.components, inst);
            match expanded.len() {
                0 => expand_eq0 += 1,
                1 => expand_eq1 += 1,
                _ => expand_gt1 += 1,
            }
        }
    }

    // Also report master-subtree health across all defs (independent of
    // whether an instance points at them).
    let mut defs_total = 0usize;
    let mut defs_nonempty = 0usize;
    for def in mapped.components.defs.values() {
        defs_total += 1;
        if mapped.scene.get(def.root).is_some()
            && mapped.scene.descendants_of(def.root).count() >= 2
        {
            defs_nonempty += 1;
        }
    }

    eprintln!("=== INSTANCE RESOLUTION DIAGNOSTIC ===");
    eprintln!(
        "report: {} instances, {} components(defs), {} component_sets, \
             {} instance-children dropped",
        report.instances,
        report.components,
        report.component_sets,
        report.instance_children_dropped,
    );
    eprintln!(
        "scene instances: {total_instances} total | {resolved_to_def} resolved->def | \
             {resolved_to_set} resolved->set | {dangling} dangling(ComponentId(0)/unknown)",
    );
    eprintln!(
        "resolved->def masters: {resolved_with_nonempty_master} NON-EMPTY (>=1 descendant) | \
             {resolved_with_empty_master} EMPTY",
    );
    eprintln!(
        "expand_instance: {expand_gt1} returned >1 node | {expand_eq1} returned ==1 (root only) | \
             {expand_eq0} returned 0 (empty)",
    );
    eprintln!(
        "defs library: {defs_total} defs total | {defs_nonempty} have non-empty master subtree",
    );

    // Recursive expansion probe: mirror the renderer, which recurses into
    // nested `NodeData::Instance` clones produced by `expand_instance`. A
    // nested instance with a dangling component returns empty -> blue block.
    // Walk top-level scene instances, expand, then recurse into expanded
    // nested instances, counting how many expansions come back empty.
    fn probe(
        scene: &fanta_doc::scene::Scene,
        comps: &fanta_doc::component::ComponentLibrary,
        inst: &fanta_doc::node::InstanceNode,
        depth: usize,
        empty: &mut usize,
        nonempty: &mut usize,
        nested_total: &mut usize,
    ) {
        if depth > 12 {
            return;
        }
        let expanded = expand_instance(scene, comps, inst);
        if expanded.is_empty() {
            *empty += 1;
            return;
        }
        *nonempty += 1;
        for e in &expanded {
            if let NodeData::Instance(nested) = &e.node.data {
                *nested_total += 1;
                probe(
                    scene,
                    comps,
                    nested,
                    depth + 1,
                    empty,
                    nonempty,
                    nested_total,
                );
            }
        }
    }
    let mut rec_empty = 0usize;
    let mut rec_nonempty = 0usize;
    let mut nested_total = 0usize;
    for root in mapped.scene.roots().to_vec() {
        // Skip the hidden Components page so we measure what a DESIGN page
        // actually renders (top-level instances + their nested expansion).
        let is_components = mapped
            .scene
            .get(root)
            .map(|n| n.name == "Components")
            .unwrap_or(false);
        if is_components {
            continue;
        }
        for id in mapped.scene.descendants_of(root) {
            if let Some(NodeData::Instance(inst)) = mapped.scene.get(id).map(|n| &n.data) {
                probe(
                    &mapped.scene,
                    &mapped.components,
                    inst,
                    0,
                    &mut rec_empty,
                    &mut rec_nonempty,
                    &mut nested_total,
                );
            }
        }
    }
    eprintln!(
        "RECURSIVE expansion (design pages, like the renderer): \
             {rec_nonempty} non-empty expansions | {rec_empty} EMPTY expansions \
             (-> blue blocks) | {nested_total} nested instances encountered",
    );

    eprintln!("=== END DIAGNOSTIC ===");
}

/// Loaded + mapped real-fixture state shared by the `imports_real_fig_fixture_*`
/// tests. `None` when `FANTA_FIG_FIXTURE` is unset, so each test skips cleanly.
struct LoadedFixture {
    doc: FigDocument,
    mapped: fanta_doc::Doc,
    report: crate::mapping::MapReport,
    assets: std::collections::HashMap<fanta_doc::AssetId, Vec<u8>>,
}

/// Read + map the `FANTA_FIG_FIXTURE` `.fig`, or `None` (skip) when it is unset.
fn load_real_fixture() -> Option<LoadedFixture> {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping");
        return None;
    };
    let bytes = std::fs::read(&path).expect("read fixture");
    let doc = read_fig(&bytes).expect("real .fig must parse");
    let (mapped, report, assets) = crate::mapping::fig_to_doc(&doc).expect("map to doc");
    Some(LoadedFixture {
        doc,
        mapped,
        report,
        assets,
    })
}

/// Opt-in integration test against a real Figma export — schema, node counts,
/// and the core recovery assertions. Skipped unless `FANTA_FIG_FIXTURE` points
/// at a `.fig` file (we can't commit a 20 MB copyrighted fixture). Run with:
///   FANTA_FIG_FIXTURE=/path/to/file.fig cargo test -p fanta-fig-interop -- --ignored
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn imports_real_fig_fixture_basic() {
    let Some(fx) = load_real_fixture() else {
        return;
    };
    let (doc, mapped_doc, report) = (&fx.doc, &fx.mapped, &fx.report);
    // The schema should be substantial and the root a message with content.
    assert!(doc.schema.defs.len() > 50, "real fig schema has many defs");

    use fanta_doc::node::NodeData;
    let mut text_nodes = 0usize;
    let mut nonempty_text = 0usize;
    let mut instances = 0usize;
    let mut vectors = 0usize;
    for root in mapped_doc.scene.roots().to_vec() {
        for id in mapped_doc.scene.descendants_of(root) {
            match mapped_doc.scene.get(id).map(|n| &n.data) {
                Some(NodeData::Text(t)) => {
                    text_nodes += 1;
                    if !t.content.trim().is_empty() {
                        nonempty_text += 1;
                    }
                }
                Some(NodeData::Instance(_)) => instances += 1,
                Some(NodeData::Vector(_)) => vectors += 1,
                _ => {}
            }
        }
    }

    let mut by_type: Vec<_> = report.skipped_by_type.iter().collect();
    by_type.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    eprintln!(
        "real fig: {} schema defs, mapped {} scene nodes, {} skipped, \
             {} instance-children dropped; {text_nodes} text nodes \
             ({nonempty_text} with content), {} pages",
        doc.schema.defs.len(),
        mapped_doc.scene.len(),
        report.skipped(),
        report.instance_children_dropped,
        mapped_doc.pages().len(),
    );
    eprintln!(
        "  P1 side-tables: {} components, {} sets, {} variables, \
             {} variable collections, {} instances, {} vectors-recovered \
             ({vectors} vector nodes total), {} reactions, {} bindings",
        report.components,
        report.component_sets,
        report.variables,
        report.variable_collections,
        report.instances,
        report.vectors_recovered,
        report.reactions,
        report.bindings,
    );
    eprintln!("  top skipped types: {by_type:?}");

    assert!(!mapped_doc.scene.is_empty(), "should map at least one node");
    // VECTOR-family geometry is no longer skipped (STEP 1 bbox fallback), so
    // the skip count must drop far below the ~6,930-node baseline that the
    // pre-P1 importer left behind from skipping VECTOR/STAR/LINE/BOOLEAN.
    assert!(
        report.skipped() < 1_000,
        "skipped count ({}) should drop dramatically once VECTOR is recovered \
             (baseline was ~6,930)",
        report.skipped()
    );
    assert!(
        report.vectors_recovered > 5_000,
        "thousands of VECTOR nodes recovered"
    );
    assert!(
        report.components > 0,
        "real design system must yield components"
    );
    assert!(
        report.instances > 0,
        "real file must place component instances"
    );
    assert!(instances > 0, "instances present in the scene");
    assert!(
        nonempty_text > 0,
        "real file must yield TEXT nodes with content"
    );
    assert!(
        !mapped_doc.pages().is_empty(),
        "at least one CANVAS page registered"
    );
    mapped_doc
        .scene
        .validate()
        .expect("imported scene validates");
}

/// Opt-in: instance OVERRIDE + baked `derivedSymbolData` resolution against a
/// real Figma export. Skipped unless `FANTA_FIG_FIXTURE` is set.
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn imports_real_fig_fixture_overrides() {
    let Some(fx) = load_real_fixture() else {
        return;
    };
    let report = &fx.report;
    eprintln!(
        "  instance OVERRIDES: {} instances carry ≥1 override; {} overrides applied total",
        report.instances_with_overrides, report.overrides_applied,
    );
    eprintln!(
        "  instance DERIVED (derivedSymbolData): {} instances carry ≥1 derived entry; \
             {} entries applied total ({} geometry-decoded, {} with size, {} with transform, \
             {} with text)",
        report.instances_with_derived,
        report.derived_overrides_applied,
        report.derived_geometry_decoded,
        report.derived_with_size,
        report.derived_with_transform,
        report.derived_with_text,
    );

    // Instance overrides must resolve: the Spectrum file is built from
    // instances whose text/fills are overridden per placement. We must carry
    // hundreds of resolved overrides (text + fill + visibility), not zero.
    assert!(
        report.instances_with_overrides > 100,
        "expected many instances to carry resolved overrides, got {}",
        report.instances_with_overrides
    );
    assert!(
        report.overrides_applied > report.instances_with_overrides,
        "override count ({}) should exceed instances-with-overrides ({})",
        report.overrides_applied,
        report.instances_with_overrides
    );
    // Baked derivedSymbolData must resolve broadly: the Spectrum file bakes
    // resolved render data onto ~13.5k of its ~13.8k instances, so we must
    // carry baked DerivedOverrides onto thousands of instances, with a
    // healthy share decoding real per-instance geometry.
    assert!(
        report.instances_with_derived > 1_000,
        "expected thousands of instances to carry baked derivedSymbolData, got {}",
        report.instances_with_derived
    );
    assert!(
        report.derived_overrides_applied > report.instances_with_derived,
        "derived entry count ({}) should exceed instances-with-derived ({})",
        report.derived_overrides_applied,
        report.instances_with_derived
    );
    assert!(
        report.derived_geometry_decoded > 1_000,
        "expected thousands of derived entries to decode real geometry, got {}",
        report.derived_geometry_decoded
    );
}

/// Opt-in: STEP-2 vector geometry decode (real paths vs bbox fallback) against a
/// real Figma export. Skipped unless `FANTA_FIG_FIXTURE` is set.
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn imports_real_fig_fixture_geometry() {
    let Some(fx) = load_real_fixture() else {
        return;
    };
    let (doc, mapped_doc, report) = (&fx.doc, &fx.mapped, &fx.report);
    use fanta_doc::node::NodeData;

    // Tally VECTOR-family nodes by geometry tag (decoded vs fallback) and
    // sanity-check that decoded paths stay within a sane multiple of the node's
    // own bbox (real local-space coords land near [0, size]).
    let mut decoded_in_scene = 0usize;
    let mut fallback_in_scene = 0usize;
    let mut decoded_segments_total = 0usize;
    let mut decoded_out_of_box = 0usize;
    for root in mapped_doc.scene.roots().to_vec() {
        for id in mapped_doc.scene.descendants_of(root) {
            let Some(node) = mapped_doc.scene.get(id) else {
                continue;
            };
            let NodeData::Vector(v) = &node.data else {
                continue;
            };
            match node.meta.get("geometry").and_then(|g| g.as_str()) {
                Some("decoded") => {
                    decoded_in_scene += 1;
                    decoded_segments_total += v.path.segments.len();
                    if let Some(b) = v.path.rough_bounds() {
                        let span = b.width().max(b.height());
                        // A decoded path with a span > 100k px is almost
                        // certainly misdecoded coordinates.
                        if span > 100_000.0 || !span.is_finite() {
                            decoded_out_of_box += 1;
                        }
                    }
                }
                Some("bbox_fallback") => fallback_in_scene += 1,
                _ => {}
            }
        }
    }
    let avg_segs = if decoded_in_scene > 0 {
        decoded_segments_total as f64 / decoded_in_scene as f64
    } else {
        0.0
    };
    eprintln!(
        "  STEP-2 geometry: {} blobs in table; {} vectors DECODED real paths, \
             {} kept bbox_fallback (report.geometry_decoded={}); \
             avg {avg_segs:.1} segments/decoded path; {decoded_out_of_box} decoded \
             paths out-of-box",
        doc.blobs.len(),
        decoded_in_scene,
        fallback_in_scene,
        report.geometry_decoded,
    );

    // The file carries a blob table, and a *healthy* fraction of vector-family
    // nodes decoded their real path geometry (rather than every one falling back
    // to a bbox rect). This fixture has ~thousands of icon vectors with
    // fillGeometry.
    assert!(!doc.blobs.is_empty(), "real file exposes a blobs table");
    assert!(
        report.geometry_decoded > 3_000,
        "expected thousands of vectors to decode real geometry, got {}",
        report.geometry_decoded
    );
    let decodable = decoded_in_scene + fallback_in_scene;
    assert!(decodable > 0, "vector-family nodes present");
    assert!(
        decoded_in_scene * 2 > decodable,
        "a healthy majority of vectors should decode ({decoded_in_scene} decoded / \
             {decodable} total)"
    );
    // Decoded paths must be sane: coords land in a reasonable local-space box,
    // so essentially none should blow past the out-of-box guard.
    assert!(
        decoded_out_of_box * 100 < decoded_in_scene.max(1),
        "decoded paths should have in-range coords ({decoded_out_of_box} of \
             {decoded_in_scene} out of box)"
    );
}

/// Scene-walk fidelity tally: gradient-filled nodes, rounded-rect nodes, image
/// fills, and the distinct image asset ids the scene actually references.
struct SceneFillTally {
    scene_gradient_fills: usize,
    scene_corner_rounded: usize,
    scene_image_fills: usize,
    scene_image_assets: std::collections::HashSet<fanta_doc::AssetId>,
}

/// Walk every scene node and tally the gradient/image/rounded-rect fills that
/// actually landed (vs the read-time report counters).
fn tally_scene_fills(mapped_doc: &fanta_doc::Doc) -> SceneFillTally {
    use fanta_doc::node::NodeData;
    use fanta_doc::style::Fill;
    let mut t = SceneFillTally {
        scene_gradient_fills: 0,
        scene_corner_rounded: 0,
        scene_image_fills: 0,
        scene_image_assets: std::collections::HashSet::new(),
    };
    for root in mapped_doc.scene.roots().to_vec() {
        for id in mapped_doc.scene.descendants_of(root) {
            let Some(node) = mapped_doc.scene.get(id) else {
                continue;
            };
            match &node.data {
                NodeData::Vector(v) => {
                    if v.fills.iter().any(|f| matches!(f, Fill::Gradient { .. })) {
                        t.scene_gradient_fills += 1;
                    }
                    for f in &v.fills {
                        if let Fill::Image { asset, .. } = f {
                            t.scene_image_fills += 1;
                            t.scene_image_assets.insert(*asset);
                        }
                    }
                    if v.corner_radius.is_some() {
                        t.scene_corner_rounded += 1;
                    }
                }
                NodeData::Group(g) => {
                    if matches!(&g.background, Some(Fill::Gradient { .. })) {
                        t.scene_gradient_fills += 1;
                    }
                    if let Some(Fill::Image { asset, .. }) = &g.background {
                        t.scene_image_fills += 1;
                        t.scene_image_assets.insert(*asset);
                    }
                }
                _ => {}
            }
        }
    }
    t
}

/// Opt-in: element-fidelity counters (gradients, images, strokes, corners,
/// opacity, effects) + the embedded-image asset flow, against a real Figma
/// export. Skipped unless `FANTA_FIG_FIXTURE` is set.
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn imports_real_fig_fixture_element_fidelity() {
    let Some(fx) = load_real_fixture() else {
        return;
    };
    let (doc, mapped_doc, report, assets) = (&fx.doc, &fx.mapped, &fx.report, &fx.assets);

    let SceneFillTally {
        scene_gradient_fills,
        scene_corner_rounded,
        scene_image_fills,
        scene_image_assets,
    } = tally_scene_fills(mapped_doc);
    eprintln!(
        "  ELEMENT FIDELITY (this work): {} gradients imported, {} image fills, \
             {} nodes stroked, {} per-corner-radius shapes, {} node opacity<1, \
             {} effects (drop/inner shadow), {} blend modes; \
             scene has {scene_gradient_fills} gradient-filled nodes, \
             {scene_corner_rounded} rounded-rect nodes",
        report.gradients_imported,
        report.images_imported,
        report.strokes_imported,
        report.per_corner_radius,
        report.node_opacity_imported,
        report.effects_imported,
        report.blend_modes_imported,
    );
    eprintln!(
        "  IMAGES: {} image paints recognized; {} embedded bitmaps in ZIP; \
             {} assets extracted (referenced+present); scene has {scene_image_fills} \
             image-fill nodes across {} distinct assets",
        report.images_imported,
        doc.images.len(),
        report.image_assets_extracted,
        scene_image_assets.len(),
    );

    // The Spectrum file uses gradients (color-area pickers, accent fills),
    // strokes (dividers, outlines), and rounded rects everywhere. These must all
    // be non-zero now (they were dropped before this work).
    assert!(
        report.gradients_imported > 100,
        "expected hundreds of gradient paints imported, got {}",
        report.gradients_imported
    );
    assert!(
        scene_gradient_fills > 0,
        "gradient fills must reach actual scene nodes"
    );
    assert!(
        report.strokes_imported > 1_000,
        "expected thousands of stroked nodes, got {}",
        report.strokes_imported
    );
    assert!(
        scene_corner_rounded > 1_000,
        "expected thousands of rounded-rect nodes, got {scene_corner_rounded}"
    );
    assert!(
        report.node_opacity_imported > 0,
        "expected some sub-1.0 node opacities imported"
    );
    assert!(
        report.effects_imported > 0,
        "expected drop/inner-shadow effects imported"
    );

    // Embedded images: the Spectrum file ships avatar / cover / logo bitmaps, so
    // the ZIP must carry images, paints must reference them, and the extracted
    // asset map must back the `Fill::Image`s that reached the scene.
    assert!(!doc.images.is_empty(), "real .fig carries embedded images");
    assert!(
        report.images_imported > 0,
        "expected image paints recognized, got {}",
        report.images_imported
    );
    assert!(
        report.image_assets_extracted > 0,
        "expected embedded image bytes paired to assets"
    );
    assert!(scene_image_fills > 0, "image fills must reach scene nodes");
    // Every distinct asset a scene `Fill::Image` references must be one we handed
    // back for the resolver (modulo thumbnail-only refs with no ZIP entry, which
    // simply don't appear in `assets`). So the extracted count must cover the
    // scene's distinct assets that are present in the ZIP.
    let scene_assets_with_bytes = scene_image_assets
        .iter()
        .filter(|a| assets.contains_key(a))
        .count();
    assert!(
        scene_assets_with_bytes > 0,
        "at least one scene image asset must resolve to extracted bytes"
    );
    assert_eq!(
        assets.len(),
        report.image_assets_extracted,
        "report.image_assets_extracted must equal the returned asset map size"
    );
}

/// Opt-in: shared-style reference resolution (styleIdForFill inlining) against a
/// real Figma export. Skipped unless `FANTA_FIG_FIXTURE` is set.
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn imports_real_fig_fixture_style_resolution() {
    let Some(fx) = load_real_fixture() else {
        return;
    };
    let (doc, report) = (&fx.doc, &fx.report);

    // Independently re-measure the raw node_changes (before our pre-pass) so the
    // numbers are auditable: how many nodes reference a FILL style while carrying
    // an EMPTY fillPaints (the fills we silently dropped), how many style defs
    // exist, and — via the report — how many we now resolve.
    let raw = doc
        .root
        .get("nodeChanges")
        .and_then(KiwiValue::as_array)
        .expect("nodeChanges array");
    let style_defs_raw = raw
        .iter()
        .filter(|nc| nc.get("styleType").is_some())
        .count();
    let empty_fill_with_ref = raw
        .iter()
        .filter(|nc| {
            let empty = nc
                .get("fillPaints")
                .and_then(KiwiValue::as_array)
                .map(|a| a.is_empty())
                .unwrap_or(true);
            let has_ref = nc
                .get("styleIdForFill")
                .and_then(|r| r.get("guid"))
                .is_some();
            empty && has_ref
        })
        .count();
    eprintln!(
        "  STYLE RESOLUTION (this work): {style_defs_raw} style-def NodeChanges (styleType set); \
             {empty_fill_with_ref} nodes had styleIdForFill + EMPTY fillPaints (silently dropped before); \
             report: {} style defs, {} empty-fill refs seen, {} resolved to a real paint",
        report.style_def_count, report.style_ref_empty_fill, report.style_ref_resolved_fill,
    );
    print_style_sample(raw);

    // The Spectrum design system is built on shared color styles; the file must
    // carry style defs AND a meaningful number of empty-fill refs that our
    // pre-pass resolves.
    assert!(
        report.style_def_count > 0,
        "real design system must ship shared style defs (styleType present)"
    );
    if empty_fill_with_ref > 0 {
        assert!(
            report.style_ref_resolved_fill > 0,
            "empty-fill style refs existed ({empty_fill_with_ref}) but none resolved"
        );
    }
}

/// Print ONE sample: a dark-theme header-ish node → its `styleIdForFill` → the
/// resolved color hex (after inlining the FILL style's paints). Pure diagnostic.
fn print_style_sample(raw: &[KiwiValue]) {
    use fanta_doc::color::Color;
    let guid_str = |guid: &KiwiValue| -> Option<String> {
        let s = guid.get("sessionID").and_then(KiwiValue::as_f64)? as u64;
        let l = guid.get("localID").and_then(KiwiValue::as_f64)? as u64;
        Some(format!("{s}:{l}"))
    };
    // Build a guid→style-def map of FILL styles to chase a ref into.
    let mut fill_styles: std::collections::HashMap<String, &KiwiValue> =
        std::collections::HashMap::new();
    for nc in raw.iter() {
        if nc.get("styleType").and_then(KiwiValue::as_str) == Some("FILL") {
            if let Some(g) = nc.get("guid").and_then(guid_str) {
                fill_styles.insert(g, nc);
            }
        }
    }
    let solid_hex = |paints: &KiwiValue| -> Option<String> {
        let arr = paints.as_array()?;
        for p in arr {
            if p.get("type").and_then(KiwiValue::as_str) == Some("SOLID") {
                let c = p.get("color")?;
                let r = (c.get("r")?.as_f64()? * 255.0).round() as u8;
                let g = (c.get("g")?.as_f64()? * 255.0).round() as u8;
                let b = (c.get("b")?.as_f64()? * 255.0).round() as u8;
                return Some(Color { r, g, b, a: 255 }.to_hex());
            }
        }
        None
    };
    for nc in raw.iter() {
        let empty = nc
            .get("fillPaints")
            .and_then(KiwiValue::as_array)
            .map(|a| a.is_empty())
            .unwrap_or(true);
        if !empty {
            continue;
        }
        let Some(ref_guid) = nc
            .get("styleIdForFill")
            .and_then(|r| r.get("guid"))
            .and_then(guid_str)
        else {
            continue;
        };
        let Some(style) = fill_styles.get(&ref_guid) else {
            continue;
        };
        let Some(paints) = style.get("fillPaints") else {
            continue;
        };
        if let Some(hex) = solid_hex(paints) {
            let name = nc
                .get("name")
                .and_then(KiwiValue::as_str)
                .unwrap_or("<unnamed>");
            let kind = nc.get("type").and_then(KiwiValue::as_str).unwrap_or("?");
            let style_name = style
                .get("name")
                .and_then(KiwiValue::as_str)
                .unwrap_or("<style>");
            eprintln!(
                "  STYLE SAMPLE: {kind} '{name}' → styleIdForFill {ref_guid} \
                     (style '{style_name}') → resolved {hex}",
            );
            return;
        }
    }
    eprintln!("  STYLE SAMPLE: (no empty-fill node with a resolvable SOLID FILL style found)");
}

/// Opt-in malformed-input guard against a real Figma export: the file
/// truncated at sampled byte offsets must always come back as `Err` — never a
/// panic, never a silent partial `Ok`. Complements the synthetic
/// `every_strict_prefix_errors_cleanly` (container `tests` module), which
/// covers every offset of the small hand-built container; this one runs the
/// same property over the real 20 MB ZIP+zstd shape. Run with:
///   FANTA_FIG_FIXTURE=/path/to/file.fig cargo test -p fanta-fig-interop -- --ignored
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn real_fig_truncations_error_cleanly() {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping");
        return;
    };
    let bytes = std::fs::read(&path).expect("read fixture");
    assert!(bytes.len() > 4096, "fixture should be a real export");

    // Sample offsets: every byte of the first 64 (header / zip local header),
    // 256 evenly spaced interior cuts, and every length in the final 64 bytes
    // (the zip central-directory tail, where off-by-one truncation bugs live).
    let mut lens: Vec<usize> = (0..64.min(bytes.len())).collect();
    lens.extend((1..=256usize).map(|i| i * bytes.len() / 257));
    lens.extend((bytes.len() - 64)..bytes.len());

    for len in lens {
        assert!(
            read_fig(&bytes[..len]).is_err(),
            "real .fig truncated to {len}/{} bytes must error",
            bytes.len()
        );
    }
}
