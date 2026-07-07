use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, Result, anyhow};
use fanta_fig_interop::{KiwiValue, read_fig};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing .fig path"))?;
    let needle = args.next().unwrap_or_default().to_lowercase();

    let bytes = fs::read(&fig_path).with_context(|| format!("reading {}", fig_path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let node_changes = fig
        .root
        .get("nodeChanges")
        .and_then(KiwiValue::as_array)
        .context("fig root has no nodeChanges array")?;

    for change in node_changes {
        let name = change.get("name").and_then(KiwiValue::as_str).unwrap_or("");
        let ty = change.get("type").and_then(KiwiValue::as_str).unwrap_or("");
        let guid = format_value(change.get("guid"));
        let text = text_characters(change).unwrap_or("");
        if !needle.is_empty()
            && !name.to_lowercase().contains(&needle)
            && !ty.to_lowercase().contains(&needle)
            && !guid.to_lowercase().contains(&needle)
            && !text.to_lowercase().contains(&needle)
        {
            continue;
        }
        println!("NODE {name:?} type={ty} guid={guid}");
        print_selected_fields(change, 1);
        if let Some(overrides) = change
            .get("symbolData")
            .and_then(|symbol_data| symbol_data.get("symbolOverrides"))
            .and_then(KiwiValue::as_array)
        {
            for (index, override_change) in overrides.iter().enumerate() {
                println!("  OVERRIDE #{index}");
                print_selected_fields(override_change, 2);
            }
        }
        if let Some(derived_entries) = change
            .get("derivedSymbolData")
            .and_then(KiwiValue::as_array)
        {
            for (index, derived_change) in derived_entries.iter().enumerate() {
                println!("  DERIVED #{index}");
                print_selected_fields(derived_change, 2);
            }
        }
    }

    Ok(())
}

fn text_characters(value: &KiwiValue) -> Option<&str> {
    value
        .get("textData")
        .and_then(|text_data| text_data.get("characters"))
        .and_then(KiwiValue::as_str)
}

fn print_selected_fields(value: &KiwiValue, depth: usize) {
    let KiwiValue::Object { fields, .. } = value else {
        return;
    };
    let mut keys = fields.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        if should_print_key(key) {
            println!(
                "{}{} = {}",
                "  ".repeat(depth),
                key,
                format_value(fields.get(key))
            );
        }
    }
}

fn should_print_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower.contains("guid")
        || lower.contains("style")
        || lower.contains("fill")
        || lower.contains("text")
        || lower.contains("paint")
        || lower.contains("visible")
        || lower.contains("size")
        || lower.contains("transform")
        || lower.contains("layout")
        || lower.contains("stack")
}

fn format_value(value: Option<&KiwiValue>) -> String {
    let Some(value) = value else {
        return "<none>".to_owned();
    };
    match value {
        KiwiValue::Bool(value) => value.to_string(),
        KiwiValue::Byte(value) => value.to_string(),
        KiwiValue::Int(value) => value.to_string(),
        KiwiValue::Uint(value) => value.to_string(),
        KiwiValue::Float(value) => format!("{value:.3}"),
        KiwiValue::String(value) => format!("{value:?}"),
        KiwiValue::Int64(value) => value.to_string(),
        KiwiValue::Uint64(value) => value.to_string(),
        KiwiValue::Enum(value) => value.clone(),
        KiwiValue::Array(values) => {
            let items = values
                .iter()
                .take(4)
                .map(format_compact_value)
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if values.len() > 4 { ", ..." } else { "" };
            format!("[{}{}] len={}", items, suffix, values.len())
        }
        KiwiValue::Object { type_name, fields } => {
            if type_name == "GUID" {
                return format_guid_object(value).unwrap_or_else(|| "GUID{?}".to_owned());
            }
            if type_name == "StyleId" {
                return value
                    .get("guid")
                    .and_then(|guid| format_guid_object(guid))
                    .map(|guid| format!("StyleId({guid})"))
                    .unwrap_or_else(|| "StyleId{?}".to_owned());
            }
            if type_name == "GUIDPath" {
                let guids = value
                    .get("guids")
                    .and_then(KiwiValue::as_array)
                    .map(|guids| {
                        guids
                            .iter()
                            .filter_map(format_guid_object)
                            .collect::<Vec<_>>()
                            .join(">")
                    })
                    .unwrap_or_default();
                return format!("GUIDPath({guids})");
            }
            if type_name == "Paint" {
                return format_paint(value).unwrap_or_else(|| "Paint{?}".to_owned());
            }
            if type_name == "TextData" {
                let characters = value
                    .get("characters")
                    .and_then(KiwiValue::as_str)
                    .unwrap_or("");
                return format!("TextData(characters={characters:?})");
            }
            if type_name == "Vector" {
                let x = value
                    .get("x")
                    .and_then(KiwiValue::as_f64)
                    .unwrap_or_default();
                let y = value
                    .get("y")
                    .and_then(KiwiValue::as_f64)
                    .unwrap_or_default();
                return format!("Vector({x:.3},{y:.3})");
            }
            if type_name == "Matrix" {
                let get = |field: &str, default| {
                    value
                        .get(field)
                        .and_then(KiwiValue::as_f64)
                        .unwrap_or(default)
                };
                return format!(
                    "Matrix([{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}])",
                    get("m00", 1.0),
                    get("m01", 0.0),
                    get("m02", 0.0),
                    get("m10", 0.0),
                    get("m11", 1.0),
                    get("m12", 0.0),
                );
            }
            let mut keys = fields.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let keys = keys.into_iter().take(8).collect::<Vec<_>>().join(",");
            format!("{type_name}{{{keys}}}")
        }
    }
}

fn format_compact_value(value: &KiwiValue) -> String {
    format_value(Some(value))
}

fn format_guid_object(value: &KiwiValue) -> Option<String> {
    let session = value.get("sessionID").and_then(KiwiValue::as_f64)? as u64;
    let local = value.get("localID").and_then(KiwiValue::as_f64)? as u64;
    Some(format!("{session}:{local}"))
}

fn format_paint(value: &KiwiValue) -> Option<String> {
    let ty = value.get("type").and_then(KiwiValue::as_str).unwrap_or("?");
    let visible = value
        .get("visible")
        .map(|visible| matches!(visible, KiwiValue::Bool(true)))
        .unwrap_or(true);
    let opacity = value
        .get("opacity")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(1.0);
    if let Some(color) = value.get("color").and_then(format_color) {
        Some(format!(
            "Paint(type={ty}, visible={visible}, opacity={opacity:.3}, color={color})"
        ))
    } else {
        Some(format!(
            "Paint(type={ty}, visible={visible}, opacity={opacity:.3})"
        ))
    }
}

fn format_color(value: &KiwiValue) -> Option<String> {
    let opacity = value
        .get("opacity")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(1.0);
    let channel = |field: &str| -> Option<u8> {
        let value = value.get(field).and_then(KiwiValue::as_f64)?;
        Some((value.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    Some(format!(
        "#{:02X}{:02X}{:02X}@{opacity:.3}",
        channel("r")?,
        channel("g")?,
        channel("b")?,
    ))
}
