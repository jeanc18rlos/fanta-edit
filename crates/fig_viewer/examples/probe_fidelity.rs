//! Probes .fig files for the raw Kiwi fields behind the fidelity-catalog
//! findings, so each claim can be verified against the corpus before fixing.
//!
//! usage: cargo run -p fig_viewer --example probe_fidelity -- <file.fig> [...]

use std::{collections::BTreeMap, env, fs};

use anyhow::{Context as _, Result};
use fanta_fig_interop::{KiwiValue, read_fig};

fn main() -> Result<()> {
    for path in env::args().skip(1) {
        println!("==== {path} ====");
        let bytes = fs::read(&path).with_context(|| format!("reading {path}"))?;
        let fig = read_fig(&bytes).context("parsing .fig")?;
        let node_changes = fig
            .root
            .get("nodeChanges")
            .and_then(KiwiValue::as_array)
            .context("no nodeChanges")?;
        probe(node_changes);
    }
    Ok(())
}

fn bump(map: &mut BTreeMap<String, usize>, key: impl Into<String>) {
    *map.entry(key.into()).or_default() += 1;
}

fn probe(changes: &[KiwiValue]) {
    let mut counters: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut sample = |key: &str, value: String| {
        let v = samples.entry(key.to_owned()).or_default();
        if v.len() < 6 {
            v.push(value);
        }
    };

    for change in changes {
        let ty = change
            .get("type")
            .and_then(KiwiValue::as_str)
            .unwrap_or("NONE");
        let name = change.get("name").and_then(KiwiValue::as_str).unwrap_or("");

        // 1. page canvas background
        if ty == "CANVAS" {
            bump(&mut counters, "canvas");
            for f in [
                "backgroundColor",
                "backgroundOpacity",
                "backgroundEnabled",
                "backgroundPaints",
            ] {
                if let Some(v) = change.get(f) {
                    bump(&mut counters, format!("canvas.{f}"));
                    sample(&format!("canvas.{f}"), format!("{name:?}: {v:?}"));
                }
            }
        }

        // 2. line height units
        if let Some(lh) = change.get("lineHeight") {
            let units = lh.get("units").and_then(KiwiValue::as_str).unwrap_or("?");
            let value = lh.get("value").and_then(KiwiValue::as_f64).unwrap_or(-1.0);
            bump(&mut counters, format!("lineHeight.units.{units}"));
            if units == "PERCENT" {
                sample("lineHeight.PERCENT", format!("{name:?}: {value}"));
                if (value - 100.0).abs() < 1e-6 {
                    bump(&mut counters, "lineHeight.PERCENT.100");
                }
            }
            if units == "RAW" {
                sample("lineHeight.RAW", format!("{name:?}: {value}"));
            }
        }

        // 3. fontVariations
        if let Some(fv) = change.get("fontVariations") {
            bump(&mut counters, "fontVariations");
            sample("fontVariations", format!("{name:?}: {fv:?}"));
        }

        // 4. cornerSmoothing
        if let Some(cs) = change.get("cornerSmoothing").and_then(KiwiValue::as_f64) {
            if cs > 0.0 {
                bump(&mut counters, "cornerSmoothing>0");
                sample("cornerSmoothing", format!("{name:?} ({ty}): {cs}"));
            }
        }

        // 5. showShadowBehindNode
        if let Some(effects) = change.get("effects").and_then(KiwiValue::as_array) {
            for e in effects {
                if let Some(v) = e.get("showShadowBehindNode") {
                    bump(&mut counters, format!("effect.showShadowBehindNode={v:?}"));
                }
            }
        }

        // 6/7. image paint scale/rotation + per-paint blendMode
        for field in ["fillPaints", "strokePaints", "backgroundPaints"] {
            let Some(paints) = change.get(field).and_then(KiwiValue::as_array) else {
                continue;
            };
            for p in paints {
                let pty = p.get("type").and_then(KiwiValue::as_str).unwrap_or("?");
                if let Some(bm) = p.get("blendMode").and_then(KiwiValue::as_str) {
                    if bm != "NORMAL" {
                        bump(&mut counters, format!("paint.blendMode.{bm}"));
                        sample("paint.blendMode", format!("{name:?} {pty}: {bm}"));
                    }
                }
                if pty == "IMAGE" {
                    let mode = p
                        .get("imageScaleMode")
                        .and_then(KiwiValue::as_str)
                        .unwrap_or("FILL");
                    bump(&mut counters, format!("image.mode.{mode}"));
                    if let Some(s) = p.get("scale").and_then(KiwiValue::as_f64) {
                        if (s - 1.0).abs() > 1e-6 {
                            bump(&mut counters, "image.scale!=1");
                            sample("image.scale", format!("{name:?} mode={mode}: {s}"));
                        }
                    }
                    if let Some(r) = p.get("rotation").and_then(KiwiValue::as_f64) {
                        if r.abs() > 1e-6 {
                            bump(&mut counters, "image.rotation!=0");
                            sample("image.rotation", format!("{name:?} mode={mode}: {r}"));
                        }
                    }
                    if p.get("imageTransform").is_some() || p.get("transform").is_some() {
                        bump(&mut counters, format!("image.transform-present.{mode}"));
                    }
                    if let Some(filters) = p.get("filters") {
                        bump(&mut counters, "image.filters");
                        sample("image.filters", format!("{name:?}: {filters:?}"));
                    }
                }
                // 8. radial/diamond gradient transforms with rotation/anisotropy
                if pty == "GRADIENT_RADIAL" || pty == "GRADIENT_DIAMOND" {
                    if let Some(t) = p.get("transform") {
                        let g = |k: &str, d: f64| t.get(k).and_then(KiwiValue::as_f64).unwrap_or(d);
                        let (m00, m01, m10, m11) =
                            (g("m00", 1.0), g("m01", 0.0), g("m10", 0.0), g("m11", 1.0));
                        if m01.abs() > 1e-4 || m10.abs() > 1e-4 {
                            bump(&mut counters, format!("{pty}.rotated"));
                            sample(
                                "radial.rotated",
                                format!("{name:?}: [{m00:.3},{m01:.3};{m10:.3},{m11:.3}]"),
                            );
                        }
                        let sx = (m00 * m00 + m10 * m10).sqrt();
                        let sy = (m01 * m01 + m11 * m11).sqrt();
                        if sx > 1e-9 && sy > 1e-9 && (sx / sy - 1.0).abs() > 0.05 {
                            bump(&mut counters, format!("{pty}.anisotropic"));
                        }
                    }
                }
            }
        }

        // 9. text truncation / maxLines / paragraph spacing / indent
        for f in [
            "textTruncation",
            "maxLines",
            "paragraphSpacing",
            "paragraphIndent",
        ] {
            if let Some(v) = change.get(f) {
                let nontrivial = match v.as_f64() {
                    Some(x) => x != 0.0,
                    None => true,
                };
                if nontrivial {
                    bump(&mut counters, format!("text.{f}"));
                    sample(&format!("text.{f}"), format!("{name:?}: {v:?}"));
                }
            }
        }

        // 10. text fill: gradient/image or stacked fills on TEXT nodes
        if ty == "TEXT" {
            if let Some(paints) = change.get("fillPaints").and_then(KiwiValue::as_array) {
                let visible: Vec<&KiwiValue> = paints
                    .iter()
                    .filter(|p| !matches!(p.get("visible"), Some(KiwiValue::Bool(false))))
                    .collect();
                if visible.len() > 1 {
                    bump(&mut counters, "text.stacked-fills");
                }
                if let Some(first) = visible.first() {
                    let pty = first.get("type").and_then(KiwiValue::as_str).unwrap_or("?");
                    if pty != "SOLID" {
                        bump(&mut counters, format!("text.first-fill.{pty}"));
                        sample("text.nonsolid-fill", format!("{name:?}: {pty}"));
                    }
                }
            }
        }

        // 11. LINE nodes / arrow caps / boolean stroke-only
        if ty == "LINE" {
            bump(&mut counters, "line-node");
            let has_fill_geom = change
                .get("fillGeometry")
                .and_then(KiwiValue::as_array)
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            let has_stroke_geom = change
                .get("strokeGeometry")
                .and_then(KiwiValue::as_array)
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            bump(
                &mut counters,
                format!("line.fillGeom={has_fill_geom},strokeGeom={has_stroke_geom}"),
            );
        }
        for f in ["strokeCap", "strokeCapStart", "strokeCapEnd"] {
            if let Some(cap) = change.get(f).and_then(KiwiValue::as_str) {
                if cap.contains("ARROW")
                    || cap.contains("TRIANGLE")
                    || cap.contains("CIRCLE")
                    || cap.contains("DIAMOND")
                {
                    bump(&mut counters, format!("{f}.{cap}"));
                    sample("arrow-caps", format!("{name:?} ({ty}): {f}={cap}"));
                }
            }
        }
        if ty == "BOOLEAN_OPERATION" {
            let has_fill_geom = change
                .get("fillGeometry")
                .and_then(KiwiValue::as_array)
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            let has_stroke_geom = change
                .get("strokeGeometry")
                .and_then(KiwiValue::as_array)
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            bump(
                &mut counters,
                format!("bool-op.fillGeom={has_fill_geom},strokeGeom={has_stroke_geom}"),
            );
        }

        // 12. per-path winding + per-path styleID
        if let Some(paths) = change.get("fillGeometry").and_then(KiwiValue::as_array) {
            let rules: Vec<&str> = paths
                .iter()
                .map(|p| {
                    p.get("windingRule")
                        .and_then(KiwiValue::as_str)
                        .unwrap_or("NONZERO")
                })
                .collect();
            if rules.len() > 1 && rules.iter().any(|r| *r != rules[0]) {
                bump(&mut counters, "fillGeometry.mixed-winding");
                sample("mixed-winding", format!("{name:?} ({ty}): {rules:?}"));
            }
            let style_ids: Vec<u64> = paths
                .iter()
                .filter_map(|p| p.get("styleID").and_then(KiwiValue::as_f64))
                .map(|v| v as u64)
                .collect();
            if style_ids.iter().any(|s| *s != 0) {
                bump(&mut counters, "fillGeometry.nonzero-styleID");
                sample("path-styleID", format!("{name:?} ({ty}): {style_ids:?}"));
            }
        }

        // 13. container blendMode NORMAL vs PASS_THROUGH
        if matches!(
            ty,
            "FRAME" | "GROUP" | "SYMBOL" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE" | "SECTION"
        ) {
            if let Some(bm) = change.get("blendMode").and_then(KiwiValue::as_str) {
                bump(&mut counters, format!("container.blendMode.{bm}"));
            }
        }

        // 15. per-run textCase overrides + SMALL_CAPS + LINEAR_BURN/DODGE blends
        if let Some(case) = change.get("textCase").and_then(KiwiValue::as_str) {
            if case.starts_with("SMALL_CAPS") {
                bump(&mut counters, format!("textCase.{case}"));
            }
        }
        if let Some(bm) = change.get("blendMode").and_then(KiwiValue::as_str) {
            if bm.starts_with("LINEAR_") {
                bump(&mut counters, format!("node.blendMode.{bm}"));
            }
        }
        for field in ["textData", "derivedTextData"] {
            if let Some(table) = change
                .get(field)
                .and_then(|td| td.get("styleOverrideTable"))
                .and_then(KiwiValue::as_array)
            {
                for entry in table {
                    if let Some(case) = entry.get("textCase").and_then(KiwiValue::as_str) {
                        bump(&mut counters, format!("run.textCase.{case}"));
                        sample("run.textCase", format!("{name:?}: {case}"));
                    }
                }
            }
        }

        // 14. miterLimit vs strokeMiterAngle
        for f in ["miterLimit", "strokeMiterAngle"] {
            if let Some(v) = change.get(f).and_then(KiwiValue::as_f64) {
                bump(&mut counters, format!("stroke.{f}"));
                sample(&format!("stroke.{f}"), format!("{name:?} ({ty}): {v}"));
            }
        }
    }

    for (key, count) in &counters {
        println!("{count:>7}  {key}");
    }
    println!("---- samples ----");
    for (key, values) in &samples {
        for v in values {
            println!("  {key}: {v}");
        }
    }
}
