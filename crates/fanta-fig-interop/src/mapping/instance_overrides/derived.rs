//! Decoding baked `derivedSymbolData` entries into typed `DerivedOverride`s.

use super::{
    Color, Fill, KiwiValue, MapReport, OverridePath, decode_geometry, parse_font_weight,
    read_fills, read_line_height, read_number_px, read_paint, read_transform,
};

/// Decode one `derivedSymbolData` entry (a resolved `NodeChange`) into a typed
/// [`fanta_doc::node::DerivedOverride`] at `path`. Returns `None` when the entry
/// carries no field we can apply (so a no-op entry isn't counted). `report` is
/// updated with the per-field coverage tallies.
pub(crate) fn read_derived_override(
    path: OverridePath,
    d: &KiwiValue,
    blobs: &[Vec<u8>],
    report: &mut MapReport,
) -> Option<fanta_doc::node::DerivedOverride> {
    use fanta_doc::node::{DerivedOverride, DerivedText};

    // Resolved local transform (Figma bakes the per-instance position/scale).
    let transform = d.get("transform").map(|_| read_transform(d));
    // Resolved box size.
    let size = d.get("size").map(|s| {
        let w = s.get("x").and_then(KiwiValue::as_f64).unwrap_or(0.0);
        let h = s.get("y").and_then(KiwiValue::as_f64).unwrap_or(0.0);
        [w, h]
    });
    // Resolved fill / stroke geometry, decoded from command blobs.
    let path_data = decode_geometry(d, "fillGeometry", blobs);
    let stroke_path = decode_geometry(d, "strokeGeometry", blobs);
    // Baked per-instance fills (rare in practice; present only when Figma stored
    // resolved paints on the entry rather than via variable/mode resolution).
    let fills = {
        let f = read_fills(d);
        (!f.is_empty()).then(|| f.into_iter().collect())
    };
    let stroke_weight = d
        .get("strokeWeight")
        .and_then(KiwiValue::as_f64)
        .filter(|w| *w >= 0.0);

    // Baked text layout. Figma's resolved per-instance text lives under
    // `derivedTextData`, but NOT as the scalar `fontSize`/`fontName`/`characters`
    // a NodeChange uses — instead the resolved size sits on each glyph
    // (`glyphs[].fontSize`) and the resolved font (family/style/weight) on the
    // run's `fontMetaData[].key` + `fontWeight`. We pull from those nested shapes
    // (verified on the Spectrum fixture), falling back to the scalar fields for
    // hand-built / synthetic entries. The full glyph run isn't modelled — we
    // re-shape from content + style.
    let dtd = d.get("derivedTextData");
    let font_size = d
        .get("fontSize")
        .and_then(KiwiValue::as_f64)
        .or_else(|| {
            dtd.and_then(|t| t.get("glyphs"))
                .and_then(KiwiValue::as_array)
                .and_then(|g| g.first())
                .and_then(|g0| g0.get("fontSize"))
                .and_then(KiwiValue::as_f64)
        })
        .filter(|f| *f > 0.0);
    let line_height = read_line_height(d.get("lineHeight"), font_size.unwrap_or(16.0));
    let letter_spacing = read_number_px(d.get("letterSpacing"), font_size.unwrap_or(16.0));
    let content = dtd
        .and_then(|t| t.get("characters"))
        .and_then(KiwiValue::as_str)
        .map(str::to_owned);
    // Per-instance derived text color: the entry's `fillPaints` first SOLID paint.
    // Figma bakes the resolved (theme-aware) glyph color here for files that store
    // it on the baked entry — op2 swaps the whole derived fill onto the text
    // descendant. (In the Spectrum fixture most theme text color instead flows
    // through symbolOverride fills / variable resolution, so this is often absent;
    // we still honor it where present.)
    let color = first_solid_color(d.get("fillPaints"));
    // Per-instance derived weight + family from the resolved font run. Prefer the
    // entry's `fontMetaData[].key`/`fontWeight` (Figma's resolved font), then any
    // top-level `fontName`, then `derivedTextData.fontName`. The `fontWeight` is a
    // direct numeric weight; the style string falls back through the full OpenType
    // table (op2 `parseFontWeight`).
    let font_meta = dtd
        .and_then(|t| t.get("fontMetaData"))
        .and_then(KiwiValue::as_array)
        .and_then(|m| m.first());
    let font_name = d
        .get("fontName")
        .or_else(|| dtd.and_then(|t| t.get("fontName")))
        .or_else(|| font_meta.and_then(|m| m.get("key")));
    let weight = font_meta
        .and_then(|m| m.get("fontWeight"))
        .and_then(KiwiValue::as_f64)
        .filter(|w| *w > 0.0)
        .map(|w| w.round().clamp(1.0, 1000.0) as u16)
        .or_else(|| {
            font_name
                .and_then(|f| f.get("style"))
                .and_then(KiwiValue::as_str)
                .and_then(|s| parse_font_weight(&s.to_ascii_lowercase()))
        });
    let family = font_name
        .and_then(|f| f.get("family"))
        .and_then(KiwiValue::as_str)
        .filter(|f| !f.is_empty())
        .map(str::to_owned);
    let text = if font_size.is_some()
        || line_height.is_some()
        || letter_spacing.is_some()
        || content.is_some()
        || color.is_some()
        || weight.is_some()
        || family.is_some()
    {
        Some(DerivedText {
            content,
            font_size,
            line_height,
            letter_spacing,
            color,
            weight,
            family,
        })
    } else {
        None
    };

    // An entry with nothing we can apply is a no-op — skip it (don't count it).
    if transform.is_none()
        && size.is_none()
        && path_data.is_none()
        && stroke_path.is_none()
        && fills.is_none()
        && stroke_weight.is_none()
        && text.is_none()
    {
        return None;
    }

    if path_data.is_some() {
        report.derived_geometry_decoded += 1;
    }
    if size.is_some() {
        report.derived_with_size += 1;
    }
    if transform.is_some() {
        report.derived_with_transform += 1;
    }
    if let Some(t) = &text {
        report.derived_with_text += 1;
        if t.color.is_some() {
            report.derived_text_color += 1;
        }
        if t.weight.is_some() {
            report.derived_text_weight += 1;
        }
    }

    Some(DerivedOverride {
        path,
        transform,
        size,
        fills,
        path_data,
        stroke_path,
        stroke_weight,
        text,
    })
}

/// The first SOLID color in a `fillPaints`-style paint array, or `None` when the
/// array is absent/empty or its first visible paint isn't solid. Used to pull a
/// derived text entry's resolved glyph color.
pub(crate) fn first_solid_color(paints: Option<&KiwiValue>) -> Option<Color> {
    let arr = paints.and_then(KiwiValue::as_array)?;
    arr.iter().find_map(|p| match read_paint(p) {
        Some(Fill::Solid { color }) => Some(color),
        _ => None,
    })
}
