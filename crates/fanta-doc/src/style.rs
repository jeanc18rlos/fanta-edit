//! Visual style — fills, strokes, shadows, blend modes, opacity.
//!
//! Inspired by Figma's paint/effect model (one-of variants, list-of-paints for
//! stacking) and Skia's underlying capability set. Shadows are stored on the
//! node, not the fill, because they apply to the node's compositing not its
//! per-paint sampling.

use crate::color::{Color, Gradient};
use crate::id::AssetId;
use serde::{Deserialize, Serialize};

/// A single paint layer applied to a shape. Multiple [`Fill`]s on a node stack
/// in declaration order (first listed = bottom-most), like Figma's fill stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fill {
    /// Solid color fill.
    Solid { color: Color },
    /// Linear or radial gradient.
    Gradient { gradient: Gradient },
    /// Image fill. `asset` references a bitmap in the asset store; `mode`
    /// determines how the image is sized into the node bounds.
    ///
    /// `crop` is an optional normalized `[x, y, w, h]` sub-rectangle (0..=1 in
    /// asset space) — the same crop representation
    /// [`BitmapNode::crop`](crate::node::BitmapNode::crop) and the renderer's
    /// `crop_to_pixels` already use. It carries Figma's `imageScaleMode: CROP`
    /// `imageTransform` (a pan/zoom of the image within the fill).
    ///
    /// `Box`ed so the (rare) crop payload lives off-stack: this keeps
    /// `size_of::<Fill>()` unchanged, which matters because `Fill` is embedded in
    /// `OverrideValue` → `Operation::SetInstanceOverride`, and growing it would
    /// bloat that op enum. Serde-transparent (a `Box<[f32; 4]>` serializes
    /// identically to `[f32; 4]`), so cropped fills round-trip and — with
    /// `#[serde(default)]` — old `.fant` docs that have no `crop` key still load
    /// (the field is skipped from JSON when `None`).
    Image {
        asset: AssetId,
        mode: ImageFitMode,
        #[serde(
            default = "default_opacity",
            skip_serializing_if = "is_default_opacity"
        )]
        opacity: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        crop: Option<Box<[f32; 4]>>, // [x, y, w, h] in 0..=1 asset space
    },
}

impl Fill {
    pub fn solid(color: Color) -> Self {
        Self::Solid { color }
    }
}

fn default_opacity() -> f32 {
    1.0
}

fn is_default_opacity(opacity: &f32) -> bool {
    (*opacity - 1.0).abs() < f32::EPSILON
}

/// How an image fill is sized into the host node's bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFitMode {
    /// Fill the bounds, cropping as needed to preserve aspect.
    Fill,
    /// Fit inside the bounds, leaving empty space to preserve aspect.
    Fit,
    /// Stretch independently on each axis. Aspect not preserved.
    Stretch,
    /// Repeat the image at its native size.
    Tile,
}

/// A stroke applied to a shape's outline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub paint: Fill,
    pub width: f64,
    #[serde(default)]
    pub cap: StrokeCap,
    #[serde(default)]
    pub join: StrokeJoin,
    /// SVG-style miter limit. Ignored for non-miter joins.
    #[serde(default = "default_miter_limit")]
    pub miter_limit: f64,
    /// Dash pattern. Empty = solid stroke. Pairs of (on, off) lengths.
    #[serde(default)]
    pub dash: Vec<f64>,
    /// Where along the stroke width the geometric edge sits.
    #[serde(default)]
    pub align: StrokeAlign,
    /// Per-side stroke widths `[top, right, bottom, left]`, in logical px. When
    /// present (Figma's "individual" border weights —
    /// `borderStrokeWeightsIndependent` with `borderTop/Right/Bottom/LeftWeight`)
    /// each rectangle edge is stroked at its own width; the renderer falls back to
    /// the uniform [`width`](Stroke::width) for any non-rectangle path or when this
    /// is `None`. Additive — absent on every pre-existing doc, so they round-trip
    /// byte-identical (skipped from JSON when `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_side: Option<[f64; 4]>,
}

fn default_miter_limit() -> f64 {
    4.0
}

impl Stroke {
    pub fn solid(color: Color, width: f64) -> Self {
        Self {
            paint: Fill::solid(color),
            width,
            cap: StrokeCap::default(),
            join: StrokeJoin::default(),
            miter_limit: 4.0,
            dash: Vec::new(),
            align: StrokeAlign::default(),
            per_side: None,
        }
    }
}

/// End-cap style for an open stroke. SVG-spec names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// Corner-join style. SVG-spec names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Whether the stroke sits inside, outside, or centered on the edge.
/// Centered is the SVG default; Figma defaults inside for shapes and centered
/// for paths. We default centered for consistency with SVG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlign {
    #[default]
    Center,
    Inside,
    Outside,
}

/// A drop-shadow or inner-shadow effect applied to a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    #[serde(default)]
    pub kind: ShadowKind,
    pub color: Color,
    pub blur: f64,
    pub spread: f64,
    pub offset: [f64; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowKind {
    #[default]
    Drop,
    Inner,
}

/// A Gaussian-blur effect applied to a node (Figma `LAYER_BLUR` /
/// `BACKGROUND_BLUR`).
///
/// Modeled as its OWN node-level effect list (parallel to [`Shadow`]s, on the
/// [`CanvasNode`](crate::node::CanvasNode) wrapper) rather than overloading
/// [`Shadow`]: a blur has no color/offset/spread, only a radius and a target
/// (the node's own layer, or the backdrop behind it). Keeping it a separate
/// additive struct means old docs round-trip byte-identical (the field is
/// skipped when empty) and the [`Shadow`] shape is untouched.
///
/// The renderer maps `radius` to a Skia Gaussian sigma with the SAME `radius/2`
/// convention drop/inner shadows use, and caps the on-screen sigma so zoom stays
/// bounded.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Blur {
    #[serde(default)]
    pub kind: BlurKind,
    /// Blur radius in logical pixels (Figma's effect `radius`). `0` ⇒ no blur.
    pub radius: f64,
}

impl Blur {
    /// A layer (foreground) blur of the given radius.
    pub fn layer(radius: f64) -> Self {
        Self {
            kind: BlurKind::Layer,
            radius,
        }
    }

    /// A background (backdrop) blur of the given radius.
    pub fn background(radius: f64) -> Self {
        Self {
            kind: BlurKind::Background,
            radius,
        }
    }
}

/// Whether a [`Blur`] blurs the node's own rendered layer (Figma `LAYER_BLUR` /
/// `FOREGROUND_BLUR`) or the backdrop behind it (Figma `BACKGROUND_BLUR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlurKind {
    /// Gaussian-blur the node's own composited layer (and its subtree). The
    /// default. Figma `LAYER_BLUR` / `FOREGROUND_BLUR`.
    #[default]
    Layer,
    /// Gaussian-blur the BACKDROP visible through the node's silhouette (a
    /// "frosted glass" effect): Skia `image_filters::blur` used as a backdrop
    /// filter on a `save_layer`. Figma `BACKGROUND_BLUR`.
    Background,
}

/// Standard blend modes — matches the CSS Compositing Level 1 spec and Skia's
/// `SkBlendMode`. The doc only stores the enum; conversion to Skia happens in
/// `fanta-render`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_serializes_with_tagged_kind() {
        let f = Fill::solid(Color::rgb(255, 0, 0));
        let j = serde_json::to_value(&f).unwrap();
        assert_eq!(j["kind"], "solid");
    }

    #[test]
    fn stroke_defaults_round_trip() {
        let s = Stroke::solid(Color::BLACK, 2.0);
        let j = serde_json::to_string(&s).unwrap();
        let back: Stroke = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn image_fill_crop_round_trips_and_defaults_to_none() {
        use crate::id::AssetId;

        // A cropped image fill round-trips the crop rect.
        let cropped = Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 0.5,
            crop: Some(Box::new([0.1, 0.2, 0.5, 0.4])),
        };
        let j = serde_json::to_string(&cropped).unwrap();
        assert!(j.contains("crop"), "crop is present on the wire: {j}");
        assert!(
            j.contains("opacity"),
            "non-default opacity is present on the wire: {j}"
        );
        // A `Box<[f32; 4]>` serializes identically to `[f32; 4]`.
        assert!(j.contains("0.5"), "crop array elements round-trip: {j}");
        let back: Fill = serde_json::from_str(&j).unwrap();
        assert_eq!(cropped, back);

        // No crop is skipped from JSON (byte-identical to a pre-crop doc) and
        // an absent `crop` field defaults to `None` so old docs still load.
        let uncropped = Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
        };
        let ju = serde_json::to_string(&uncropped).unwrap();
        assert!(!ju.contains("crop"), "None crop is omitted: {ju}");
        assert!(!ju.contains("opacity"), "default opacity is omitted: {ju}");
        // A pre-crop doc (no `crop` key at all) still loads, defaulting to None.
        // Reuse the omitted-crop JSON above — it is exactly a legacy image fill.
        let from_legacy: Fill = serde_json::from_str(&ju).unwrap();
        assert!(
            matches!(
                from_legacy,
                Fill::Image {
                    crop: None,
                    opacity,
                    ..
                } if (opacity - 1.0).abs() < f32::EPSILON
            ),
            "absent crop and opacity default to None/1.0"
        );
    }

    #[test]
    fn blend_mode_snake_case_in_json() {
        let bm = BlendMode::ColorDodge;
        let j = serde_json::to_string(&bm).unwrap();
        assert_eq!(j, "\"color_dodge\"");
    }

    #[test]
    fn blur_round_trips_and_defaults_to_layer() {
        let layer = Blur::layer(8.0);
        assert_eq!(layer.kind, BlurKind::Layer);
        let j = serde_json::to_string(&layer).unwrap();
        let back: Blur = serde_json::from_str(&j).unwrap();
        assert_eq!(layer, back);

        let bg = Blur::background(12.0);
        assert_eq!(bg.kind, BlurKind::Background);
        let jb = serde_json::to_string(&bg).unwrap();
        assert!(
            jb.contains("background"),
            "kind serializes snake_case: {jb}"
        );
        let back_bg: Blur = serde_json::from_str(&jb).unwrap();
        assert_eq!(bg, back_bg);

        // `kind` is optional on the wire and defaults to Layer.
        let from_radius_only: Blur = serde_json::from_str("{\"radius\":4.0}").unwrap();
        assert_eq!(from_radius_only, Blur::layer(4.0));
    }
}
