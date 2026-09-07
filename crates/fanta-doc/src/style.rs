//! Visual style — fills, strokes, shadows, blend modes, opacity.
//!
//! Inspired by Figma's paint/effect model (one-of variants, list-of-paints for
//! stacking) and Skia's underlying capability set. Shadows are stored on the
//! node, not the fill, because they apply to the node's compositing not its
//! per-paint sampling.

use crate::color::{Color, Gradient};
use crate::id::AssetId;
use crate::serde_util::{default_opacity, is_default_opacity, is_false};
use serde::{Deserialize, Serialize};

/// A scalar constrained to `0.0..=1.0` — opacity, and any other "fraction of
/// full" value. The invariant is enforced at construction and on deserialize
/// (`from = "f32"` clamps a malformed doc), so nothing downstream has to
/// re-clamp: the value simply cannot be out of range. Serializes as a bare
/// number (`into = "f32"`), so the wire is unchanged from a plain `f32`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(from = "f32", into = "f32")]
pub struct UnitInterval(f32);

impl UnitInterval {
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    /// Clamp `v` into `0.0..=1.0`. `NaN` maps to `0.0` (fully transparent) —
    /// the safe end when a caller hands in garbage.
    pub fn new(v: f32) -> Self {
        Self(if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) })
    }

    /// The underlying `0.0..=1.0` value.
    pub fn get(self) -> f32 {
        self.0
    }
}

impl Default for UnitInterval {
    /// Fully opaque — the default for a node's opacity.
    fn default() -> Self {
        Self::ONE
    }
}

impl From<f32> for UnitInterval {
    fn from(v: f32) -> Self {
        Self::new(v)
    }
}

impl From<UnitInterval> for f32 {
    fn from(u: UnitInterval) -> Self {
        u.0
    }
}

/// Non-destructive image adjustments applied to an image paint at render time
/// (Figma's Fill-panel "Adjust" controls). Every field is `0.0` at rest — a
/// no-op — so a default `ImageAdjust` serializes to nothing and old docs stay
/// byte-identical. Ranges follow Figma: roughly `-1.0..=1.0` each, `0` = no
/// change. The renderer folds these into a tone-curve LUT (exposure / contrast /
/// highlights / shadows) composed with a colour matrix (saturation /
/// temperature / tint); the doc layer stays Skia-free and stores only the data.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ImageAdjust {
    /// Overall brightness (a multiplicative exposure in stops).
    #[serde(default, skip_serializing_if = "is_zero_adjust")]
    pub exposure: f32,
    /// Tonal contrast around mid-grey.
    #[serde(default, skip_serializing_if = "is_zero_adjust")]
    pub contrast: f32,
    /// Colour saturation (`-1` = greyscale, `+1` = doubled).
    #[serde(default, skip_serializing_if = "is_zero_adjust")]
    pub saturation: f32,
    /// White-balance warmth: `+` warms (more red), `-` cools (more blue).
    #[serde(default, skip_serializing_if = "is_zero_adjust")]
    pub temperature: f32,
    /// White-balance tint along the green–magenta axis.
    #[serde(default, skip_serializing_if = "is_zero_adjust")]
    pub tint: f32,
    /// Lift/lower the brightest tones.
    #[serde(default, skip_serializing_if = "is_zero_adjust")]
    pub highlights: f32,
    /// Lift/lower the darkest tones.
    #[serde(default, skip_serializing_if = "is_zero_adjust")]
    pub shadows: f32,
}

impl ImageAdjust {
    /// Whether every adjustment is at rest (a no-op) — the skip predicate and
    /// the render fast-path both consult this.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn is_zero_adjust(v: &f32) -> bool {
    *v == 0.0
}

/// A single paint layer applied to a shape. Multiple [`Fill`]s on a node stack
/// in declaration order (first listed = bottom-most), like Figma's fill stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fill {
    /// Solid color fill.
    Solid {
        color: Color,
        /// Blend mode of THIS paint against the paints below it in the fill
        /// stack (Figma per-paint `blendMode`). Default skipped, so old docs
        /// round-trip byte-identical.
        #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
        blend: BlendMode,
    },
    /// Linear or radial gradient.
    Gradient {
        gradient: Gradient,
        /// Blend mode of THIS paint against the paints below it in the fill
        /// stack (Figma per-paint `blendMode`). Default skipped, so old docs
        /// round-trip byte-identical.
        #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
        blend: BlendMode,
    },
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
        /// Tile scaling factor for [`ImageFitMode::Tile`] (Figma
        /// `ImagePaint.scalingFactor`): each tile is drawn at
        /// `natural_size × scale`. `None` ⇒ native size (1.0). Ignored for
        /// non-tile modes. Additive — absent on old docs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<f32>,
        /// Clockwise rotation of the image within the fill, in degrees (Figma
        /// exposes 90° increments). `None` ⇒ unrotated. Additive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<f32>,
        /// Blend mode of THIS paint against the paints below it in the fill
        /// stack (and the backdrop) — Figma's per-paint `blendMode`, distinct
        /// from the node-level blend. Default [`BlendMode::Normal`] is skipped
        /// so old docs round-trip byte-identical.
        #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
        blend: BlendMode,
        /// Non-destructive image adjustments (exposure / contrast / saturation /
        /// temperature / tint / highlights / shadows). Default (all zero) is a
        /// no-op and skipped, so old docs round-trip byte-identical.
        #[serde(default, skip_serializing_if = "ImageAdjust::is_default")]
        adjust: ImageAdjust,
    },
}

impl Fill {
    pub fn solid(color: Color) -> Self {
        Self::Solid {
            color,
            blend: BlendMode::Normal,
        }
    }

    /// This fill's single solid color, or `None` for a gradient/image fill
    /// (which has no one representative color). The match is exhaustive on
    /// purpose: a new `Fill` kind must decide here rather than silently read as
    /// `None` through a wildcard.
    pub fn solid_color(&self) -> Option<Color> {
        match self {
            Self::Solid { color, .. } => Some(*color),
            Self::Gradient { .. } | Self::Image { .. } => None,
        }
    }

    /// Overwrite this fill with a solid of `color`, preserving the paint's blend
    /// mode. A gradient/image fill is replaced wholesale (its blend resets to
    /// Normal) — binding a color to a non-solid paint is unusual, but clobbering
    /// beats silently no-oping, and it keeps color writes total. Exhaustive on
    /// purpose: a new `Fill` kind must decide how a color write lands rather
    /// than being clobbered to `Solid` by a wildcard.
    pub fn set_solid_color(&mut self, color: Color) {
        match self {
            Self::Solid { color: c, .. } => *c = color,
            Self::Gradient { .. } | Self::Image { .. } => {
                *self = Self::Solid {
                    color,
                    blend: BlendMode::Normal,
                }
            }
        }
    }
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
    /// Figma `showShadowBehindNode`: when `false` (Figma's default) a drop
    /// shadow is knocked out where the node itself covers it, so nothing shows
    /// through a translucent/transparent body; when `true` the shadow paints
    /// behind the whole node. Only meaningful for [`ShadowKind::Drop`].
    /// Default `false` is skipped, so old docs round-trip byte-identical.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_behind_node: bool,
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

impl BlendMode {
    /// Whether this is the default [`BlendMode::Normal`] — used by serde
    /// `skip_serializing_if` on per-paint blend fields.
    pub fn is_normal(&self) -> bool {
        matches!(self, BlendMode::Normal)
    }
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
            adjust: ImageAdjust::default(),
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
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
            adjust: ImageAdjust::default(),
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
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
    fn shadow_show_behind_node_round_trips_and_defaults_false() {
        // Old docs (no field) load as false — Figma's own default — and the
        // default is skipped on the wire so pre-existing docs stay
        // byte-identical.
        let base = Shadow {
            kind: ShadowKind::Drop,
            color: Color::BLACK,
            blur: 4.0,
            spread: 0.0,
            offset: [0.0, 2.0],
            show_behind_node: false,
        };
        let s = serde_json::to_string(&base).unwrap();
        assert!(!s.contains("show_behind_node"), "default skipped: {s}");
        let loaded: Shadow = serde_json::from_str(&s).unwrap();
        assert!(!loaded.show_behind_node);

        let behind = Shadow {
            show_behind_node: true,
            ..base
        };
        let j = serde_json::to_string(&behind).unwrap();
        let back: Shadow = serde_json::from_str(&j).unwrap();
        assert_eq!(back, behind);
    }

    #[test]
    fn image_adjust_defaults_off_and_round_trips() {
        use crate::id::AssetId;

        // A default adjust is a no-op: skipped from JSON so old docs are
        // byte-identical, and `is_default` reports it.
        assert!(ImageAdjust::default().is_default());
        let plain = Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
            adjust: ImageAdjust::default(),
        };
        let s = serde_json::to_string(&plain).unwrap();
        assert!(!s.contains("adjust"), "default adjust skipped: {s}");

        // A non-default adjust serializes (only the touched fields) and round-trips.
        let graded = Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
            adjust: ImageAdjust {
                exposure: 0.3,
                saturation: -0.5,
                ..Default::default()
            },
        };
        let j = serde_json::to_string(&graded).unwrap();
        assert!(
            j.contains("exposure") && j.contains("saturation"),
            "touched fields present: {j}"
        );
        assert!(
            !j.contains("contrast"),
            "untouched adjust fields skipped: {j}"
        );
        assert_eq!(serde_json::from_str::<Fill>(&j).unwrap(), graded);
    }

    #[test]
    fn image_fill_scale_rotation_blend_round_trip_and_default_off() {
        use crate::id::AssetId;

        let plain = Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Tile,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
            adjust: ImageAdjust::default(),
        };
        let s = serde_json::to_string(&plain).unwrap();
        for key in ["scale", "rotation", "blend"] {
            assert!(!s.contains(key), "{key} skipped at default: {s}");
        }
        let loaded: Fill = serde_json::from_str(&s).unwrap();
        assert_eq!(loaded, plain);

        let tiled = Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Tile,
            opacity: 1.0,
            crop: None,
            scale: Some(0.78),
            rotation: Some(90.0),
            blend: BlendMode::Multiply,
            adjust: ImageAdjust::default(),
        };
        let j = serde_json::to_string(&tiled).unwrap();
        let back: Fill = serde_json::from_str(&j).unwrap();
        assert_eq!(back, tiled);
    }

    #[test]
    fn gradient_fill_blend_round_trips_and_defaults_normal() {
        use crate::color::{Gradient, GradientStop};
        let g = Fill::Gradient {
            gradient: Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, 0.0],
                stops: vec![GradientStop {
                    position: 0.0,
                    color: Color::BLACK,
                }],
            },
            blend: BlendMode::Normal,
        };
        let s = serde_json::to_string(&g).unwrap();
        assert!(!s.contains("blend"), "normal blend skipped: {s}");
        let loaded: Fill = serde_json::from_str(&s).unwrap();
        assert_eq!(loaded, g);
    }

    #[test]
    fn solid_fill_blend_round_trips_and_defaults_normal() {
        // Default (Normal) is skipped, so pre-blend docs round-trip byte-identical.
        let plain = Fill::solid(Color::BLACK);
        let s = serde_json::to_string(&plain).unwrap();
        assert!(!s.contains("blend"), "normal blend skipped: {s}");
        assert_eq!(serde_json::from_str::<Fill>(&s).unwrap(), plain);

        // A non-default blend serializes and round-trips.
        let multiply = Fill::Solid {
            color: Color::BLACK,
            blend: BlendMode::Multiply,
        };
        let s = serde_json::to_string(&multiply).unwrap();
        assert!(s.contains("\"blend\":\"multiply\""), "blend present: {s}");
        assert_eq!(serde_json::from_str::<Fill>(&s).unwrap(), multiply);
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
