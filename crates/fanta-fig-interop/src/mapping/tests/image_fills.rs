//! Embedded bitmap image fills → Fill::Image + stable AssetId.

use super::*;

// =============================================================================
// Image fills (embedded bitmap → Fill::Image + AssetId)
// =============================================================================

/// Build an IMAGE `Paint` referencing `hash_bytes` with the given scale mode.
fn image_paint(hash_bytes: &[u8], scale_mode: &str) -> KiwiValue {
    let hash = KiwiValue::Array(hash_bytes.iter().map(|b| KiwiValue::Byte(*b)).collect());
    o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("IMAGE".to_owned())),
            ("opacity", KiwiValue::Float(1.0)),
            ("visible", KiwiValue::Bool(true)),
            ("imageScaleMode", KiwiValue::Enum(scale_mode.to_owned())),
            (
                "image",
                o(
                    "Image",
                    vec![
                        ("hash", hash),
                        ("name", KiwiValue::String("pic".to_owned())),
                    ],
                ),
            ),
        ],
    )
}

/// A `CROP` IMAGE `Paint` carrying an `imageTransform` (2x3 affine `m`).
fn crop_image_paint(hash_bytes: &[u8], m: [f32; 6]) -> KiwiValue {
    let hash = KiwiValue::Array(hash_bytes.iter().map(|b| KiwiValue::Byte(*b)).collect());
    o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("IMAGE".to_owned())),
            ("opacity", KiwiValue::Float(1.0)),
            ("visible", KiwiValue::Bool(true)),
            ("imageScaleMode", KiwiValue::Enum("CROP".to_owned())),
            ("imageTransform", matrix(m)),
            (
                "image",
                o(
                    "Image",
                    vec![
                        ("hash", hash),
                        ("name", KiwiValue::String("pic".to_owned())),
                    ],
                ),
            ),
        ],
    )
}

/// A rectangle node carrying the given fill paints.
fn rect_with_paints(paints: Vec<KiwiValue>) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(40.0, 40.0)),
            ("fillPaints", KiwiValue::Array(paints)),
        ],
    )
}

#[test]
fn image_paint_maps_to_fill_image_with_stable_asset_id() {
    // The 20-byte sha1 the paint references; hex is the ZIP key.
    let hash_bytes: Vec<u8> = (1u8..=20).collect();
    let hash_hex = hash_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let png = b"\x89PNG\r\n\x1a\nFAKE".to_vec();
    let images: std::collections::HashMap<String, Vec<u8>> =
        [(hash_hex, png.clone())].into_iter().collect();

    let rect = rect_with_paints(vec![image_paint(&hash_bytes, "FILL")]);
    let fig = doc_from_with_images(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 0)),
                    ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("parentIndex", parent_index(0, 0)),
                    ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ],
            ),
            rect,
        ],
        images,
    );

    let (doc, report, assets) = fig_to_doc(&fig).unwrap();
    let v = first_vector(&doc);
    assert_eq!(v.fills.len(), 1, "the image paint maps to exactly one fill");
    let Fill::Image {
        asset,
        mode,
        opacity,
        crop,
    } = &v.fills[0]
    else {
        panic!("expected Fill::Image, got {:?}", v.fills[0]);
    };
    let asset = *asset;
    assert_eq!(*mode, ImageFitMode::Fill, "FILL scale mode maps to Fill");
    assert_eq!(*opacity, 1.0, "default image opacity is preserved");
    assert_eq!(*crop, None, "a FILL paint carries no crop");

    // The AssetId is a deterministic function of the hash, so re-mapping the
    // same file yields the same id (re-import stable).
    let (doc2, _, _) = fig_to_doc(&fig).unwrap();
    let Fill::Image { asset: asset2, .. } = &first_vector(&doc2).fills[0] else {
        panic!("expected Fill::Image second time");
    };
    assert_eq!(asset, *asset2, "AssetId is stable across imports");

    // The asset map backs that exact id with the bytes from the ZIP.
    assert_eq!(report.images_imported, 1);
    assert_eq!(report.image_assets_extracted, 1);
    assert_eq!(assets.len(), 1);
    assert_eq!(
        assets.get(&asset),
        Some(&png),
        "asset map carries the image bytes"
    );
}

#[test]
fn background_image_paint_extracts_asset_bytes() {
    // Frames often store image surfaces under `backgroundPaints` rather than
    // `fillPaints`. The fill importer already reads that fallback; the asset
    // collector must mirror it or those frames render as debug placeholders.
    let hash_bytes: Vec<u8> = (21u8..=40).collect();
    let hash_hex = hash_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let png = b"\x89PNG\r\n\x1a\nBG".to_vec();
    let images: std::collections::HashMap<String, Vec<u8>> =
        [(hash_hex, png.clone())].into_iter().collect();

    let fig = doc_from_with_images(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 0)),
                    ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("parentIndex", parent_index(0, 0)),
                    ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 2)),
                    ("parentIndex", parent_index(0, 1)),
                    ("type", KiwiValue::Enum("FRAME".to_owned())),
                    ("size", vector(52.0, 52.0)),
                    (
                        "backgroundPaints",
                        KiwiValue::Array(vec![image_paint(&hash_bytes, "FILL")]),
                    ),
                ],
            ),
        ],
        images,
    );

    let (doc, report, assets) = fig_to_doc(&fig).unwrap();
    let frame_id = doc.scene.children_of(Some(doc.scene.roots()[0]))[0];
    let NodeData::Group(group) = &doc.scene.get(frame_id).unwrap().data else {
        panic!("expected frame group");
    };
    let Fill::Image { asset, .. } = group.background.as_ref().expect("image background") else {
        panic!("expected image background fill");
    };
    assert_eq!(report.image_assets_extracted, 1);
    assert_eq!(assets.get(asset), Some(&png));
}

#[test]
fn derived_background_image_paint_extracts_asset_bytes() {
    // Product-card imagery in the Spectrum fixture is baked into an instance's
    // derivedSymbolData. The renderer still needs those bytes even though the
    // image paint is nested inside override material rather than top-level
    // nodeChanges.
    let hash_bytes: Vec<u8> = (41u8..=60).collect();
    let hash_hex = hash_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let png = b"\x89PNG\r\n\x1a\nDERIVED".to_vec();
    let images: std::collections::HashMap<String, Vec<u8>> =
        [(hash_hex, png.clone())].into_iter().collect();

    let fig = doc_from_with_images(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("INSTANCE".to_owned())),
                (
                    "derivedSymbolData",
                    KiwiValue::Array(vec![o(
                        "NodeChange",
                        vec![
                            ("guid", guid(0, 2)),
                            (
                                "backgroundPaints",
                                KiwiValue::Array(vec![image_paint(&hash_bytes, "FILL")]),
                            ),
                        ],
                    )]),
                ),
            ],
        )],
        images,
    );

    let (_doc, report, assets) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.image_assets_extracted, 1);
    assert_eq!(assets.values().next(), Some(&png));
}

#[test]
fn image_scale_modes_map_to_fit_modes() {
    let hash_bytes: Vec<u8> = (1u8..=20).collect();
    for (figma, expected) in [
        ("FILL", ImageFitMode::Fill),
        ("FIT", ImageFitMode::Fit),
        ("STRETCH", ImageFitMode::Stretch),
        ("TILE", ImageFitMode::Tile),
        ("CROP", ImageFitMode::Fill), // CROP fills the cropped sub-rect (see image_crop)
    ] {
        let rect = rect_with_paints(vec![image_paint(&hash_bytes, figma)]);
        let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
        let Fill::Image { mode, .. } = &first_vector(&doc).fills[0] else {
            panic!("expected Fill::Image for scale mode {figma}");
        };
        assert_eq!(*mode, expected, "scale mode {figma}");
    }
}

#[test]
fn image_paint_opacity_is_preserved() {
    let hash_bytes: Vec<u8> = (51u8..=70).collect();
    let mut paint = image_paint(&hash_bytes, "CROP");
    paint.set_field("opacity", KiwiValue::Float(0.42));

    let fig = doc_from_with_images(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("type", KiwiValue::Enum("CANVAS".to_owned())),
                    ("name", KiwiValue::String("Page".to_owned())),
                    ("size", vector(100.0, 100.0)),
                ],
            ),
            rect_with_paints(vec![paint]),
        ],
        std::collections::HashMap::new(),
    );

    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let Fill::Image { opacity, .. } = &first_vector(&doc).fills[0] else {
        panic!("expected image fill");
    };
    assert!(
        (*opacity - 0.42).abs() < 1e-6,
        "image paint opacity should survive import, got {opacity}"
    );
}

#[test]
fn crop_paint_imports_imagetransform_as_normalized_crop_rect() {
    // A CROP paint whose imageTransform shows the centre half of the image:
    // m = [sx, 0, tx, 0, sy, ty] = [0.5, 0, 0.25, 0, 0.5, 0.25] → crop the
    // centred 0.5×0.5 window at (0.25, 0.25). FILL is the mode (the cropped
    // sub-rect fills the node); the crop window rides on `crop`.
    let hash_bytes: Vec<u8> = (1u8..=20).collect();
    let m = [0.5, 0.0, 0.25, 0.0, 0.5, 0.25];
    let rect = rect_with_paints(vec![crop_image_paint(&hash_bytes, m)]);
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let Fill::Image { mode, crop, .. } = &first_vector(&doc).fills[0] else {
        panic!("expected Fill::Image for a CROP paint");
    };
    assert_eq!(*mode, ImageFitMode::Fill, "CROP maps to the Fill fit mode");
    let crop = crop
        .as_deref()
        .copied()
        .expect("a CROP paint with an imageTransform carries a crop");
    let eps = 1e-5;
    assert!(
        (crop[0] - 0.25).abs() < eps
            && (crop[1] - 0.25).abs() < eps
            && (crop[2] - 0.5).abs() < eps
            && (crop[3] - 0.5).abs() < eps,
        "crop rect maps [m02, m12, m00, m11], got {crop:?}"
    );
}

#[test]
fn image_paint_imports_paint_transform_as_crop_rect() {
    let hash_bytes: Vec<u8> = (71u8..=90).collect();
    let m = [0.25, 0.0, 0.5, 0.0, 0.75, 0.125];
    let mut paint = image_paint(&hash_bytes, "CROP");
    paint.set_field("transform", matrix(m));

    let rect = rect_with_paints(vec![paint]);
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ("name", KiwiValue::String("Page".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
        rect,
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let Fill::Image { crop, .. } = &first_vector(&doc).fills[0] else {
        panic!("expected image fill");
    };
    let crop = crop
        .as_deref()
        .copied()
        .expect("axis-aligned paint transform carries a crop");
    let eps = 1e-6;
    assert!(
        (crop[0] - 0.5).abs() < eps
            && (crop[1] - 0.125).abs() < eps
            && (crop[2] - 0.25).abs() < eps
            && (crop[3] - 0.75).abs() < eps,
        "paint transform maps [m02, m12, m00, m11], got {crop:?}"
    );
}

#[test]
fn fill_paint_with_image_transform_carries_crop_rect() {
    let hash_bytes: Vec<u8> = (91u8..=110).collect();
    let m = [0.5, 0.0, 0.25, 0.0, 0.5, 0.125];
    let mut paint = image_paint(&hash_bytes, "FILL");
    paint.set_field("imageTransform", matrix(m));

    let rect = rect_with_paints(vec![paint]);
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let Fill::Image { crop, .. } = &first_vector(&doc).fills[0] else {
        panic!("expected image fill");
    };
    let crop = crop
        .as_deref()
        .copied()
        .expect("a FILL paint with imageTransform carries a crop");
    assert_eq!(crop, [0.25, 0.125, 0.5, 0.5]);
}

#[test]
fn crop_paint_with_identity_transform_carries_no_crop() {
    // An identity imageTransform (whole image visible) is not a meaningful crop.
    let hash_bytes: Vec<u8> = (1u8..=20).collect();
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let rect = rect_with_paints(vec![crop_image_paint(&hash_bytes, identity)]);
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let Fill::Image { crop, .. } = &first_vector(&doc).fills[0] else {
        panic!("expected Fill::Image");
    };
    assert_eq!(*crop, None, "an identity crop window carries no crop");
}

#[test]
fn distinct_image_paints_dedupe_to_one_asset() {
    // Two paints on two nodes referencing the SAME hash must collapse to one
    // asset (one decode) but two `Fill::Image`s that share the AssetId.
    let hash_bytes: Vec<u8> = (5u8..=24).collect();
    let hash_hex = hash_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let images: std::collections::HashMap<String, Vec<u8>> =
        [(hash_hex, b"\x89PNGdup".to_vec())].into_iter().collect();

    let r1 = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(10.0, 10.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![image_paint(&hash_bytes, "FILL")]),
            ),
        ],
    );
    let r2 = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 3)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(10.0, 10.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![image_paint(&hash_bytes, "FILL")]),
            ),
        ],
    );
    let fig = doc_from_with_images(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 0)),
                    ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("parentIndex", parent_index(0, 0)),
                    ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ],
            ),
            r1,
            r2,
        ],
        images,
    );
    let (_doc, report, assets) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.images_imported, 2, "both image paints recognized");
    assert_eq!(assets.len(), 1, "shared hash collapses to one asset");
    assert_eq!(report.image_assets_extracted, 1);
}

#[test]
fn image_paint_without_bytes_still_maps_but_has_no_asset() {
    // A paint whose hash has no `images/<hash>` entry (thumbnail-only / stripped
    // export) still maps to a Fill::Image, but contributes no asset bytes — the
    // renderer falls back to its placeholder. Must not panic or drop the fill.
    let hash_bytes: Vec<u8> = (100u8..=119).collect();
    let rect = rect_with_paints(vec![image_paint(&hash_bytes, "FILL")]);
    let (doc, report, assets) = fig_to_doc(&doc_with_shape(rect)).unwrap(); // doc_with_shape => empty images
    let v = first_vector(&doc);
    assert!(
        matches!(v.fills[0], Fill::Image { .. }),
        "paint still maps to image fill"
    );
    assert_eq!(report.images_imported, 1);
    assert_eq!(report.image_assets_extracted, 0, "no bytes => no asset");
    assert!(assets.is_empty());
}
