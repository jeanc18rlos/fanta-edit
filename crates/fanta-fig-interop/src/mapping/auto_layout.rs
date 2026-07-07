//! Auto-layout ("stack") + per-child layout participation + mask flag reads.

use super::{
    AutoLayout, AxisSizing, CanvasNode, CounterAlign, KiwiValue, LayoutChild, LayoutMode, MaskType,
    PrimaryAlign,
};

/// Read the auto-layout ("stack") configuration from a `NodeChange`, or `None`
/// when the change carries none of OpenPencil's layout signals.
///
/// Figma stack spacing, padding, alignment, sizing, wrapping, and reverse-z
/// fields are mapped into Fanta's model. Axis hug sizing is only trusted for
/// explicit HORIZONTAL/VERTICAL stack modes; otherwise stale stack fields may be
/// present on non-auto-layout frames.
///
/// Figma fields (verified against the embedded `fig.kiwi`): `stackMode`,
/// `stackSpacing`, `stackCounterSpacing`, the `stack*Padding*` family,
/// `stackPrimaryAlignItems`/`stackJustify`,
/// `stackCounterAlignItems`/`stackCounterAlign`,
/// `stackPrimarySizing`/`stackCounterSizing`, `stackWrap`, `stackReverseZIndex`.
pub(crate) fn read_auto_layout(change: &KiwiValue) -> Option<AutoLayout> {
    let f = |name: &str| change.get(name).and_then(KiwiValue::as_f64);

    let primary_align = map_primary_align(
        change
            .get("stackPrimaryAlignItems")
            .or_else(|| change.get("stackJustify"))
            .and_then(KiwiValue::as_str),
    );
    let counter_align = map_counter_align(
        change
            .get("stackCounterAlignItems")
            .or_else(|| change.get("stackCounterAlign"))
            .and_then(KiwiValue::as_str),
    );

    // Padding fallback chain, mirroring OpenPencil `mapPadding`:
    //   top    = stackVerticalPadding   ?? stackPadding ?? 0
    //   bottom = stackPaddingBottom     ?? stackVerticalPadding   ?? stackPadding ?? 0
    //   left   = stackHorizontalPadding ?? stackPadding ?? 0
    //   right  = stackPaddingRight      ?? stackHorizontalPadding ?? stackPadding ?? 0
    let base = f("stackPadding");
    let vert = f("stackVerticalPadding");
    let horiz = f("stackHorizontalPadding");
    let pad_top = vert.or(base).unwrap_or(0.0);
    let pad_bottom = f("stackPaddingBottom").or(vert).or(base).unwrap_or(0.0);
    let pad_left = horiz.or(base).unwrap_or(0.0);
    let pad_right = f("stackPaddingRight").or(horiz).or(base).unwrap_or(0.0);
    let padding = [pad_top, pad_right, pad_bottom, pad_left];

    let spacing = f("stackSpacing").unwrap_or(0.0);
    let has_openpencil_infer_signal = spacing != 0.0
        || padding.iter().any(|v| *v != 0.0)
        || primary_align != PrimaryAlign::Start
        || counter_align != CounterAlign::Start;

    let stack_mode = change.get("stackMode").and_then(KiwiValue::as_str);
    let explicit_axis_mode = matches!(stack_mode, Some("HORIZONTAL" | "VERTICAL"));
    let mode = match stack_mode {
        Some("HORIZONTAL") => LayoutMode::Horizontal,
        Some("VERTICAL") => LayoutMode::Vertical,
        // OpenPencil maps any other non-NONE stack mode (notably GRID) to a
        // vertical flow. When no stack mode exists, its renderer still infers a
        // horizontal flow from gap/padding/align props.
        Some(mode) if mode != "NONE" => LayoutMode::Vertical,
        _ if has_openpencil_infer_signal => LayoutMode::Horizontal,
        _ => return None,
    };
    let primary_sizing = if explicit_axis_mode {
        map_axis_sizing(change.get("stackPrimarySizing").and_then(KiwiValue::as_str))
    } else {
        AxisSizing::Fixed
    };
    let counter_sizing = if explicit_axis_mode {
        map_axis_sizing(change.get("stackCounterSizing").and_then(KiwiValue::as_str))
    } else {
        AxisSizing::Fixed
    };

    Some(AutoLayout {
        mode,
        spacing,
        counter_spacing: f("stackCounterSpacing").unwrap_or(0.0),
        padding,
        primary_align,
        counter_align,
        primary_sizing,
        counter_sizing,
        wrap: change.get("stackWrap").and_then(KiwiValue::as_str) == Some("WRAP"),
        flow_reverse: false,
        child_layout: explicit_axis_mode,
        reverse_z: matches!(
            change.get("stackReverseZIndex"),
            Some(KiwiValue::Bool(true))
        ),
    })
}

/// Map Figma `StackSize` → [`AxisSizing`]. `RESIZE_TO_FIT*` ⇒ HUG, else FIXED
/// (op1 `mapStackSizing`; `FILL` only applies to *children*, handled separately).
pub(crate) fn map_axis_sizing(s: Option<&str>) -> AxisSizing {
    match s {
        Some("RESIZE_TO_FIT") | Some("RESIZE_TO_FIT_WITH_IMPLICIT_SIZE") => AxisSizing::Hug,
        _ => AxisSizing::Fixed,
    }
}

/// Map Figma `StackJustify` → [`PrimaryAlign`] (op1 `mapStackJustify`;
/// `SPACE_EVENLY` collapses to `SpaceBetween`).
pub(crate) fn map_primary_align(s: Option<&str>) -> PrimaryAlign {
    match s {
        Some("CENTER") => PrimaryAlign::Center,
        Some("MAX") => PrimaryAlign::End,
        Some("SPACE_BETWEEN") | Some("SPACE_EVENLY") => PrimaryAlign::SpaceBetween,
        _ => PrimaryAlign::Start,
    }
}

/// Map Figma `StackAlign`/`StackCounterAlign` → [`CounterAlign`] for the
/// *container's* counter alignment (op1 `mapStackCounterAlign`; `MIN`/absent ⇒
/// Start, `AUTO` ⇒ Start).
pub(crate) fn map_counter_align(s: Option<&str>) -> CounterAlign {
    match s {
        Some("CENTER") => CounterAlign::Center,
        Some("MAX") => CounterAlign::End,
        Some("STRETCH") => CounterAlign::Stretch,
        Some("BASELINE") => CounterAlign::Baseline,
        _ => CounterAlign::Start,
    }
}

/// Read per-child auto-layout participation (`stackChildPrimaryGrow`,
/// `stackPositioning`, `stackChildAlignSelf`) from a child `NodeChange`. Returns
/// `None` when the child carries no non-default layout data, so plain children
/// round-trip without an empty struct. Mirrors op1's per-child reads
/// (`layoutGrow`/`layoutPositioning`/`layoutAlignSelf`).
pub(crate) fn read_layout_child(change: &KiwiValue) -> Option<LayoutChild> {
    let grow = change
        .get("stackChildPrimaryGrow")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(0.0) as f32;
    let absolute = change.get("stackPositioning").and_then(KiwiValue::as_str) == Some("ABSOLUTE");
    // `stackChildAlignSelf` uses `StackCounterAlign`: MIN/CENTER/MAX/STRETCH/
    // BASELINE/AUTO. `AUTO` (and absent) ⇒ inherit the parent ⇒ `None`. `MIN`
    // ⇒ Start (op1 `mapAlignSelf`).
    let align_self = match change
        .get("stackChildAlignSelf")
        .and_then(KiwiValue::as_str)
    {
        Some("MIN") => Some(CounterAlign::Start),
        Some("CENTER") => Some(CounterAlign::Center),
        Some("MAX") => Some(CounterAlign::End),
        Some("STRETCH") => Some(CounterAlign::Stretch),
        Some("BASELINE") => Some(CounterAlign::Baseline),
        _ => None,
    };
    let lc = LayoutChild {
        grow,
        absolute,
        align_self,
    };
    (!lc.is_trivial()).then_some(lc)
}

/// Read a node's **mask** flag + type from its `NodeChange` onto `node`. A node
/// flagged `mask` (Kiwi field) — the same property Figma's editor calls "Use as
/// mask" / the API exposes as `isMask` — masks its FOLLOWING SIBLINGS within the
/// same parent until the next mask sibling (the renderer's children-painting
/// path applies this). The `maskType` selects how:
///
/// - `ALPHA` (default, and the fallback when absent) → [`MaskType::Alpha`].
/// - `LUMINANCE` → [`MaskType::Luminance`].
/// - `VECTOR` / `OUTLINE` → [`MaskType::Alpha`]: a vector/outline mask is the
///   alpha coverage of the mask shape, which alpha masking already produces.
///
/// A change with no/`false` mask flag leaves `node.is_mask == false`, so this is
/// a no-op on the overwhelming majority of nodes.
pub(crate) fn read_mask(change: &KiwiValue, node: &mut CanvasNode) {
    // Figma stores the flag as `mask` in the Kiwi schema; tolerate `isMask` too
    // (the public-API spelling some exporters carry).
    let is_mask = matches!(change.get("mask"), Some(KiwiValue::Bool(true)))
        || matches!(change.get("isMask"), Some(KiwiValue::Bool(true)));
    if !is_mask {
        return;
    }
    node.is_mask = true;
    node.mask_type = match change.get("maskType").and_then(KiwiValue::as_str) {
        Some("LUMINANCE") => MaskType::Luminance,
        // ALPHA / VECTOR / OUTLINE / absent → alpha coverage of the mask shape.
        _ => MaskType::Alpha,
    };
}
