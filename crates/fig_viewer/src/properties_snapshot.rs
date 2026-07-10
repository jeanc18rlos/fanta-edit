//! Read-only "snapshot" model of the Fanta properties panel: the field
//! metadata, per-section value structs, and the free functions that build them
//! from the current document each frame. Rendering and mutation live in
//! `properties_panel` / `properties_ops`.

use std::collections::HashMap;

use fanta_canvas::transform_angle;
use fanta_doc::{
    Action, AxisSizing, BlendMode, Blur, BlurKind, BoundProp, CanvasNode, Color as FantaColor,
    ComponentDef, ComponentId, ComponentLibrary, ComponentPropId, ComponentPropKind, CounterAlign,
    Doc, Fill, Gradient, ImageFitMode, InstanceNode, LayoutChild, LayoutMode, NodeData, NodeFlags,
    NodeId, PrimaryAlign, Reaction, Shadow, ShadowKind, StrokeAlign, TextAlign, TextAutoResize,
    Transform2D, Trigger, UnitInterval, VAlign as TextVAlign, VarValue,
};
use smallvec::SmallVec;
use ui::prelude::*;

use crate::color_picker::GradientKind;
use crate::document::FigDocument;
use crate::inspector_widgets::AlignGlyph;
use crate::properties_ops::format_number;

pub(crate) const MIXED_VALUE: &str = "–";

/// One editable value in the inspector; identifies which node property the
/// shared inline editor (or an in-flight scrub / color-picker session) is
/// currently bound to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InspectorField {
    Name(NodeId),
    X(NodeId),
    Y(NodeId),
    Width(NodeId),
    Height(NodeId),
    Rotation(NodeId),
    CornerRadius(NodeId),
    /// One entry of the per-corner radii, indexed [TL, TR, BR, BL].
    CornerRadiusCorner {
        id: NodeId,
        corner: usize,
    },
    /// Squircle corner smoothing, edited as a whole percent (0–100 ⇒ 0.0–1.0).
    CornerSmoothing(NodeId),
    Opacity(NodeId),
    FillColor {
        id: NodeId,
        index: usize,
    },
    /// One paint's own opacity, as a whole percent: a solid's alpha channel, an
    /// image fill's `opacity`. Gradients carry their alpha per stop and have no
    /// paint-level opacity, so they never bind this field.
    PaintOpacity {
        id: NodeId,
        index: usize,
        is_stroke: bool,
    },
    /// A gradient fill/stroke bound to the gradient editor popover. Distinct
    /// from [`InspectorField::FillColor`] so the panel routes gradient edits to
    /// the gradient editor rather than the solid HSV picker. `is_stroke`
    /// selects the paint list.
    Gradient {
        id: NodeId,
        index: usize,
        is_stroke: bool,
    },
    StrokeColor {
        id: NodeId,
        index: usize,
    },
    StrokeWidth {
        id: NodeId,
        index: usize,
    },
    /// Text content of a virtual clone inside a component instance, edited
    /// from the panel as a [`SetInstanceOverride`] — Figma's "Content" field
    /// for instance text.
    ///
    /// [`SetInstanceOverride`]: fanta_doc::Operation::SetInstanceOverride
    InstanceText {
        id: NodeId,
        path: fanta_doc::OverridePath,
    },
    /// One canvas comment's text, addressed by the page node and comment id.
    CommentText {
        page: NodeId,
        id: String,
    },
    FontFamily(NodeId),
    FontSize(NodeId),
    LineHeight(NodeId),
    LetterSpacing(NodeId),
    LayoutGapH(NodeId),
    LayoutGapV(NodeId),
    LayoutPadH(NodeId),
    LayoutPadV(NodeId),
    EffectOffsetX {
        id: NodeId,
        index: usize,
    },
    EffectOffsetY {
        id: NodeId,
        index: usize,
    },
    EffectBlur {
        id: NodeId,
        index: usize,
    },
    EffectSpread {
        id: NodeId,
        index: usize,
    },
    EffectColor {
        id: NodeId,
        index: usize,
    },
    /// Radius of one entry of the node's blur stack (layer / background).
    BlurRadius {
        id: NodeId,
        index: usize,
    },
    /// A text node's glyph color: its Fill section is one color row, not a
    /// paint stack.
    TextColor(NodeId),
    InstanceTextProp {
        id: NodeId,
        prop: ComponentPropId,
    },
    InstanceNumberProp {
        id: NodeId,
        prop: ComponentPropId,
    },
    InstanceColorProp {
        id: NodeId,
        prop: ComponentPropId,
    },
    /// A distinct solid color across a multi-node selection. Editing it
    /// replaces that color everywhere in the selection, so the field binds a
    /// color rather than a node.
    SelectionColor {
        from: FantaColor,
    },
    PageBackground(NodeId),
}

/// The node a field belongs to, for snapshotting before a scrub / picker
/// session. `None` for fields that address the whole selection rather than one
/// node — those commit directly and never stage a per-node preview.
pub(crate) fn field_node(field: &InspectorField) -> Option<NodeId> {
    Some(match field {
        InspectorField::Name(id)
        | InspectorField::X(id)
        | InspectorField::Y(id)
        | InspectorField::Width(id)
        | InspectorField::Height(id)
        | InspectorField::Rotation(id)
        | InspectorField::CornerRadius(id)
        | InspectorField::CornerSmoothing(id)
        | InspectorField::Opacity(id)
        | InspectorField::TextColor(id)
        | InspectorField::FontFamily(id)
        | InspectorField::FontSize(id)
        | InspectorField::LineHeight(id)
        | InspectorField::LetterSpacing(id)
        | InspectorField::LayoutGapH(id)
        | InspectorField::LayoutGapV(id)
        | InspectorField::LayoutPadH(id)
        | InspectorField::LayoutPadV(id)
        | InspectorField::PageBackground(id)
        | InspectorField::CornerRadiusCorner { id, .. }
        | InspectorField::FillColor { id, .. }
        | InspectorField::PaintOpacity { id, .. }
        | InspectorField::Gradient { id, .. }
        | InspectorField::StrokeColor { id, .. }
        | InspectorField::StrokeWidth { id, .. }
        | InspectorField::EffectOffsetX { id, .. }
        | InspectorField::EffectOffsetY { id, .. }
        | InspectorField::EffectBlur { id, .. }
        | InspectorField::EffectSpread { id, .. }
        | InspectorField::EffectColor { id, .. }
        | InspectorField::BlurRadius { id, .. }
        | InspectorField::InstanceTextProp { id, .. }
        | InspectorField::InstanceNumberProp { id, .. }
        | InspectorField::InstanceColorProp { id, .. }
        | InspectorField::InstanceText { id, .. } => *id,
        InspectorField::CommentText { page, .. } => *page,
        InspectorField::SelectionColor { .. } => return None,
    })
}

/// The field Tab jumps to from `field` — its 2-up partner (X↔Y, W↔H, R↔
/// smoothing, LH↔LS, gaps, pads, effect pairs), or the next corner of the
/// per-corner radius grid.
pub(crate) fn paired_field(field: &InspectorField) -> Option<InspectorField> {
    Some(match field {
        InspectorField::X(id) => InspectorField::Y(*id),
        InspectorField::Y(id) => InspectorField::X(*id),
        InspectorField::Width(id) => InspectorField::Height(*id),
        InspectorField::Height(id) => InspectorField::Width(*id),
        InspectorField::CornerRadius(id) => InspectorField::CornerSmoothing(*id),
        InspectorField::CornerSmoothing(id) => InspectorField::CornerRadius(*id),
        InspectorField::CornerRadiusCorner { id, corner } => InspectorField::CornerRadiusCorner {
            id: *id,
            corner: (corner + 1) % 4,
        },
        InspectorField::LineHeight(id) => InspectorField::LetterSpacing(*id),
        InspectorField::LetterSpacing(id) => InspectorField::LineHeight(*id),
        InspectorField::LayoutGapH(id) => InspectorField::LayoutGapV(*id),
        InspectorField::LayoutGapV(id) => InspectorField::LayoutGapH(*id),
        InspectorField::LayoutPadV(id) => InspectorField::LayoutPadH(*id),
        InspectorField::LayoutPadH(id) => InspectorField::LayoutPadV(*id),
        InspectorField::EffectOffsetX { id, index } => InspectorField::EffectOffsetY {
            id: *id,
            index: *index,
        },
        InspectorField::EffectOffsetY { id, index } => InspectorField::EffectOffsetX {
            id: *id,
            index: *index,
        },
        InspectorField::EffectBlur { id, index } => InspectorField::EffectSpread {
            id: *id,
            index: *index,
        },
        InspectorField::EffectSpread { id, index } => InspectorField::EffectBlur {
            id: *id,
            index: *index,
        },
        _ => return None,
    })
}

/// Clamp a scrubbed value to the field's legal range so dragging can't produce
/// values the commit path would reject.
pub(crate) fn clamp_field_value(field: &InspectorField, value: f64) -> f64 {
    match field {
        InspectorField::Opacity(_)
        | InspectorField::CornerSmoothing(_)
        | InspectorField::PaintOpacity { .. } => value.clamp(0.0, 100.0),
        InspectorField::Width(_) | InspectorField::Height(_) | InspectorField::FontSize(_) => {
            value.max(1.0)
        }
        InspectorField::LineHeight(_) => value.max(0.1),
        InspectorField::CornerRadius(_)
        | InspectorField::CornerRadiusCorner { .. }
        | InspectorField::StrokeWidth { .. }
        | InspectorField::EffectBlur { .. }
        | InspectorField::BlurRadius { .. }
        | InspectorField::LayoutGapH(_)
        | InspectorField::LayoutGapV(_)
        | InspectorField::LayoutPadH(_)
        | InspectorField::LayoutPadV(_) => value.max(0.0),
        _ => value,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlignCommand {
    Left,
    CenterHorizontal,
    Right,
    Top,
    MiddleVertical,
    Bottom,
    DistributeHorizontal,
    DistributeVertical,
}

pub(crate) const ALIGN_BUTTONS: [(AlignGlyph, &str, AlignCommand); 6] = [
    (AlignGlyph::Left, "Align Left", AlignCommand::Left),
    (
        AlignGlyph::CenterH,
        "Align Horizontal Centers",
        AlignCommand::CenterHorizontal,
    ),
    (AlignGlyph::Right, "Align Right", AlignCommand::Right),
    (AlignGlyph::Top, "Align Top", AlignCommand::Top),
    (
        AlignGlyph::CenterV,
        "Align Vertical Centers",
        AlignCommand::MiddleVertical,
    ),
    (AlignGlyph::Bottom, "Align Bottom", AlignCommand::Bottom),
];

pub(crate) const DISTRIBUTE_BUTTONS: [(AlignGlyph, &str, AlignCommand); 2] = [
    (
        AlignGlyph::DistributeH,
        "Distribute Horizontally",
        AlignCommand::DistributeHorizontal,
    ),
    (
        AlignGlyph::DistributeV,
        "Distribute Vertically",
        AlignCommand::DistributeVertical,
    ),
];

pub(crate) const PRIMARY_ALIGNS: [(PrimaryAlign, &str); 4] = [
    (PrimaryAlign::Start, "Start"),
    (PrimaryAlign::Center, "Center"),
    (PrimaryAlign::End, "End"),
    (PrimaryAlign::SpaceBetween, "Space Between"),
];

pub(crate) const COUNTER_ALIGNS: [(CounterAlign, &str); 5] = [
    (CounterAlign::Start, "Start"),
    (CounterAlign::Center, "Center"),
    (CounterAlign::End, "End"),
    (CounterAlign::Stretch, "Stretch"),
    (CounterAlign::Baseline, "Baseline"),
];

pub(crate) const IMAGE_FIT_MODES: [(ImageFitMode, &str); 4] = [
    (ImageFitMode::Fill, "Fill"),
    (ImageFitMode::Fit, "Fit"),
    (ImageFitMode::Stretch, "Stretch"),
    (ImageFitMode::Tile, "Tile"),
];

pub(crate) const STROKE_ALIGNS: [(StrokeAlign, &str); 3] = [
    (StrokeAlign::Inside, "Inside"),
    (StrokeAlign::Center, "Center"),
    (StrokeAlign::Outside, "Outside"),
];

pub(crate) const LAYOUT_MODES: [(LayoutMode, &str); 2] = [
    (LayoutMode::Horizontal, "Horizontal"),
    (LayoutMode::Vertical, "Vertical"),
];

pub(crate) const AXIS_SIZINGS: [(AxisSizing, &str); 2] = [
    (AxisSizing::Fixed, "Fixed"),
    (AxisSizing::Hug, "Hug contents"),
];

pub(crate) const BLEND_MODES: [(BlendMode, &str); 16] = [
    (BlendMode::Normal, "Normal"),
    (BlendMode::Multiply, "Multiply"),
    (BlendMode::Screen, "Screen"),
    (BlendMode::Overlay, "Overlay"),
    (BlendMode::Darken, "Darken"),
    (BlendMode::Lighten, "Lighten"),
    (BlendMode::ColorDodge, "Color Dodge"),
    (BlendMode::ColorBurn, "Color Burn"),
    (BlendMode::HardLight, "Hard Light"),
    (BlendMode::SoftLight, "Soft Light"),
    (BlendMode::Difference, "Difference"),
    (BlendMode::Exclusion, "Exclusion"),
    (BlendMode::Hue, "Hue"),
    (BlendMode::Saturation, "Saturation"),
    (BlendMode::Color, "Color"),
    (BlendMode::Luminosity, "Luminosity"),
];

/// The OpenType weights the typography dropdown offers. The model stores the
/// numeric weight (100–900), so an off-list value round-trips untouched and is
/// labeled "Custom" instead of snapping onto the nearest stop.
pub(crate) const FONT_WEIGHTS: [(u16, &str); 6] = [
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "SemiBold"),
    (700, "Bold"),
    (800, "ExtraBold"),
];

pub(crate) fn text_resize_label(resize: TextAutoResize) -> &'static str {
    match resize {
        TextAutoResize::None => "Fixed",
        TextAutoResize::WidthAndHeight => "Auto width",
        TextAutoResize::Height => "Auto height",
    }
}

pub(crate) fn blur_kind_label(kind: BlurKind) -> &'static str {
    match kind {
        BlurKind::Layer => "Layer blur",
        BlurKind::Background => "Background blur",
    }
}

pub(crate) fn next_text_resize(resize: TextAutoResize) -> TextAutoResize {
    match resize {
        TextAutoResize::None => TextAutoResize::WidthAndHeight,
        TextAutoResize::WidthAndHeight => TextAutoResize::Height,
        TextAutoResize::Height => TextAutoResize::None,
    }
}

pub(crate) fn stacking_label(reverse_z: bool) -> &'static str {
    if reverse_z {
        "First on top"
    } else {
        "Last on top"
    }
}

/// The paint types the fill/stroke type selector offers, in cycle order.
pub(crate) const PAINT_KINDS: [PaintKind; 5] = [
    PaintKind::Solid,
    PaintKind::Gradient(GradientKind::Linear),
    PaintKind::Gradient(GradientKind::Radial),
    PaintKind::Gradient(GradientKind::Angular),
    PaintKind::Gradient(GradientKind::Diamond),
];

pub(crate) fn paint_kind_label(kind: PaintKind) -> &'static str {
    match kind {
        PaintKind::Solid => "Solid",
        PaintKind::Gradient(gradient_kind) => gradient_kind.label(),
    }
}

pub(crate) enum InspectorSnapshot {
    Message(SharedString),
    Ready {
        editable: bool,
        selection_len: usize,
        body: InspectorBody,
    },
}

pub(crate) enum InspectorBody {
    Page(PageSection),
    Node(Box<NodeSection>),
    Multi(MultiSection),
}

pub(crate) struct PageSection {
    pub(crate) id: Option<NodeId>,
    pub(crate) name: String,
    pub(crate) background: Option<PageBackgroundValue>,
    /// The page's comment pins, in pin-number order.
    pub(crate) comments: Vec<CommentSnapshot>,
}

pub(crate) struct CommentSnapshot {
    pub(crate) id: String,
    /// 1-based pin number shown on the canvas.
    pub(crate) number: usize,
    pub(crate) text: SharedString,
    pub(crate) resolved: bool,
}

pub(crate) enum PageBackgroundValue {
    None,
    Solid(FantaColor),
    Other(SharedString),
}

/// What the inspector treats the selected node as. Computed once per snapshot
/// build; every section's visibility keys off it (see the section matrix in
/// [`FantaPropertiesPanel::render`]). Deliberately coarser than [`NodeData`]:
/// the original Fanta inspector has no per-shape subtype — rect, ellipse, star,
/// line and boolean-op results all read as one "Shape".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    /// A `Group` that paints a surface (`is_frame_surface`).
    Frame,
    /// A plain `Group` with no surface of its own.
    Group,
    Shape,
    Text,
    Image,
    Instance,
    /// The root of a `ComponentDef` — overrides the underlying data variant.
    Component,
    /// `NodeGraph` / `AiArtifact` / `Embed` (and the out-of-scope media kinds):
    /// generic wrapper sections only.
    Other,
}

impl NodeKind {
    /// Corner radius, corner smoothing and the frame/group paint stack all key
    /// off "does this node carry a corner-capable surface".
    pub(crate) fn is_corner_capable(self) -> bool {
        matches!(
            self,
            Self::Frame | Self::Group | Self::Shape | Self::Component
        )
    }
}

pub(crate) struct NodeSection {
    pub(crate) id: NodeId,
    pub(crate) kind: NodeKind,
    pub(crate) type_name: SharedString,
    pub(crate) type_icon: IconName,
    pub(crate) name: String,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) rotation_degrees: f64,
    pub(crate) corner_radius: CornerRadiusValue,
    /// Squircle smoothing as a whole percent. `Some` only for corner-capable
    /// nodes (Vector / Group data).
    pub(crate) corner_smoothing: Option<f64>,
    pub(crate) opacity_percent: f64,
    pub(crate) blend_mode: BlendMode,
    pub(crate) fills: Option<Vec<PaintSnapshot>>,
    pub(crate) strokes: Option<Vec<PaintSnapshot>>,
    pub(crate) stroke_align: Option<StrokeAlign>,
    pub(crate) visible: bool,
    pub(crate) locked: bool,
    pub(crate) typography: Option<TypographySnapshot>,
    /// `Some` for every `Group`-backed node (frame, plain group, component
    /// master with a group root): clip content, plus auto-layout when enabled.
    pub(crate) layout: Option<LayoutSnapshot>,
    pub(crate) layout_child: Option<LayoutChildSnapshot>,
    pub(crate) image_fit: Option<ImageFitMode>,
    pub(crate) instance: Option<InstanceSection>,
    pub(crate) master: Option<MasterSection>,
    pub(crate) effects: Vec<EffectSnapshot>,
    pub(crate) blurs: Vec<BlurSnapshot>,
    pub(crate) reactions: Vec<SharedString>,
    pub(crate) bindings: Vec<BindingSnapshot>,
}

pub(crate) enum CornerRadiusValue {
    NotApplicable,
    Uniform(f64),
    PerCorner([f64; 4]),
}

pub(crate) struct LayoutSnapshot {
    pub(crate) clip: bool,
    pub(crate) auto_layout: Option<AutoLayoutSnapshot>,
}

pub(crate) struct AutoLayoutSnapshot {
    pub(crate) mode: LayoutMode,
    pub(crate) gap_h: f64,
    pub(crate) gap_v: f64,
    /// `None` when left and right padding differ (shown as mixed).
    pub(crate) pad_h: Option<f64>,
    /// `None` when top and bottom padding differ (shown as mixed).
    pub(crate) pad_v: Option<f64>,
    pub(crate) primary_align: PrimaryAlign,
    pub(crate) counter_align: CounterAlign,
    pub(crate) primary_sizing: AxisSizing,
    pub(crate) counter_sizing: AxisSizing,
    pub(crate) wrap: bool,
    pub(crate) reverse_z: bool,
}

pub(crate) struct LayoutChildSnapshot {
    pub(crate) fills_container: bool,
    pub(crate) absolute: bool,
}

/// The instance-side component info: which master it renders, its variant axes,
/// and its exposed props.
pub(crate) struct InstanceSection {
    pub(crate) component_name: SharedString,
    /// The resolved master's root node, for "go to main component". `None` when
    /// the master is dangling.
    pub(crate) main_root: Option<NodeId>,
    pub(crate) variants: Vec<VariantAxisSnapshot>,
    pub(crate) props: Vec<ComponentPropSnapshot>,
    /// The instance's painted text clones — one editable Content row each,
    /// committing a text override (Figma's per-instance text editing from the
    /// panel).
    pub(crate) texts: Vec<InstanceTextSnapshot>,
}

pub(crate) struct InstanceTextSnapshot {
    pub(crate) path: fanta_doc::OverridePath,
    /// The master text layer's name, the row label.
    pub(crate) label: SharedString,
    /// Resolved content (overrides applied) shown in the field.
    pub(crate) content: SharedString,
}

/// The master-side component info, shown when the selected node is the root of
/// a [`ComponentDef`]. Read-only: editing the schema or the variant set needs
/// `SetComponentProps` / `SetComponentSet` sub-editors (deferred).
pub(crate) struct MasterSection {
    pub(crate) name: SharedString,
    /// `Some` when the master belongs to a component set: the set's name, axes,
    /// and this member's value on each axis.
    pub(crate) variant_set: Option<VariantSetSnapshot>,
    pub(crate) props: Vec<PropSchemaSnapshot>,
}

pub(crate) struct VariantSetSnapshot {
    pub(crate) set_name: SharedString,
    /// One entry per axis: the axis name, its allowed values, and this member's
    /// selection.
    pub(crate) axes: Vec<VariantSetAxisSnapshot>,
    pub(crate) is_default_variant: bool,
}

pub(crate) struct VariantSetAxisSnapshot {
    pub(crate) name: SharedString,
    pub(crate) values: SharedString,
    pub(crate) selected: SharedString,
}

pub(crate) struct PropSchemaSnapshot {
    pub(crate) name: SharedString,
    pub(crate) kind: SharedString,
    pub(crate) default: SharedString,
}

pub(crate) struct VariantAxisSnapshot {
    pub(crate) axis: SharedString,
    pub(crate) value: SharedString,
}

pub(crate) struct ComponentPropSnapshot {
    pub(crate) id: ComponentPropId,
    pub(crate) name: SharedString,
    pub(crate) value: PropValueSnapshot,
}

pub(crate) enum PropValueSnapshot {
    Bool(bool),
    Text(String),
    Number(f64),
    Color(FantaColor),
    /// Read-only display for prop kinds without an editor (instance swap, alias
    /// defaults, text styles).
    Display(SharedString),
}

pub(crate) struct EffectSnapshot {
    pub(crate) kind: ShadowKind,
    pub(crate) color: FantaColor,
    pub(crate) offset: [f64; 2],
    pub(crate) blur: f64,
    pub(crate) spread: f64,
}

pub(crate) struct BlurSnapshot {
    pub(crate) kind: BlurKind,
    pub(crate) radius: f64,
}

/// One distinct solid color across a multi-node selection, with how many paints
/// use it.
pub(crate) struct SelectionColorSnapshot {
    pub(crate) color: FantaColor,
    pub(crate) uses: usize,
}

pub(crate) struct BindingSnapshot {
    pub(crate) property: SharedString,
    pub(crate) variable: SharedString,
}

pub(crate) struct PaintSnapshot {
    pub(crate) color: Option<FantaColor>,
    pub(crate) label: SharedString,
    pub(crate) stroke_width: Option<f64>,
    /// The gradient this paint carries, cloned so the swatch can preview it and
    /// the gradient editor can open on it. `None` for solid / image paints.
    pub(crate) gradient: Option<Gradient>,
    /// The paint's kind, driving the Solid/Linear/Radial/Angular/Diamond type
    /// selector. `None` for image paints (no type control offered).
    pub(crate) kind: Option<PaintKind>,
    /// The paint's own opacity as a whole percent — a solid's alpha, an image
    /// fill's `opacity`. `None` for gradients, whose alpha lives per stop.
    pub(crate) opacity_percent: Option<f64>,
    /// The per-paint blend mode (Figma's paint-level `blendMode`).
    pub(crate) blend: Option<BlendMode>,
    /// Whether the paint currently contributes any coverage. Drives the eye.
    pub(crate) visible: bool,
}

/// Which paint of which node a hidden-alpha memory belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PaintKey {
    pub(crate) id: NodeId,
    pub(crate) index: usize,
    pub(crate) is_stroke: bool,
}

/// The alpha a paint carried before the eye hid it. The data model has no
/// per-paint visible flag, so hiding zeroes the paint's alpha — remembering the
/// prior value here is what keeps a hide → show round-trip non-destructive
/// (the pre-fix code forced alpha back to 255 and only handled solids).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HiddenPaintAlpha {
    Solid(u8),
    /// One alpha per gradient stop, in stop order.
    Gradient(Vec<u8>),
    Image(f32),
}

/// The paint-type choices the fill/stroke type selector cycles through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaintKind {
    Solid,
    Gradient(GradientKind),
}

pub(crate) struct TypographySnapshot {
    pub(crate) font_family: String,
    pub(crate) size_px: f64,
    pub(crate) weight: u16,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
    pub(crate) line_height: f64,
    pub(crate) letter_spacing: f64,
    pub(crate) align: TextAlign,
    pub(crate) vertical_align: TextVAlign,
    pub(crate) auto_resize: TextAutoResize,
    /// The glyph color, which stands in for a Text node's fill stack.
    pub(crate) color: FantaColor,
    /// A live sub-selection spans runs whose colors disagree — the Fill row
    /// shows a mixed marker instead of a lying single swatch.
    pub(crate) color_mixed: bool,
}

pub(crate) struct MultiSection {
    pub(crate) count: usize,
    /// The first selected node, used only as a placeholder identity for the
    /// read-only multi-select field cells.
    pub(crate) first_id: NodeId,
    pub(crate) x: Option<f64>,
    pub(crate) y: Option<f64>,
    pub(crate) width: Option<f64>,
    pub(crate) height: Option<f64>,
    pub(crate) rotation_degrees: Option<f64>,
    /// Distinct solid colors used anywhere in the selection, first-seen order.
    pub(crate) colors: Vec<SelectionColorSnapshot>,
    /// How many of the selected nodes are component masters, gating "Combine as
    /// variants".
    pub(crate) master_count: usize,
}

// =============================================================================
// Scrub / preview session state
// =============================================================================

/// The pre-gesture state of the node a scrub (or color-picker session)
/// mutates. Restored right before commit so the committed operation records
/// `old` = gesture-start — one undo step per gesture, exactly like the canvas
/// tools' transient-then-commit staging.
#[derive(Clone)]
pub(crate) struct NodeSnapshot {
    pub(crate) id: NodeId,
    pub(crate) transform: Transform2D,
    pub(crate) opacity: UnitInterval,
    pub(crate) data: Box<NodeData>,
    pub(crate) effects: SmallVec<[Shadow; 0]>,
    pub(crate) blurs: SmallVec<[Blur; 0]>,
}

// =============================================================================
// Snapshot construction
// =============================================================================

pub(crate) fn page_section(document: &FigDocument, selected_page_index: Option<usize>) -> PageSection {
    let page = document.page(selected_page_index);
    let root = page.and_then(|page| page.root);
    let doc = &document.doc;
    let root_node = root.and_then(|root| doc.scene.get(root));
    let name = root_node
        .map(|node| node.name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| page.map(|page| page.name.to_string()))
        .unwrap_or_else(|| "Document".to_string());
    let background = root_node.and_then(|node| match &node.data {
        NodeData::Group(group) => Some(match &group.background {
            None => PageBackgroundValue::None,
            Some(Fill::Solid { color, .. }) => PageBackgroundValue::Solid(*color),
            Some(Fill::Gradient { .. }) => PageBackgroundValue::Other("Gradient".into()),
            Some(Fill::Image { .. }) => PageBackgroundValue::Other("Image".into()),
        }),
        _ => None,
    });
    let comments = root
        .map(|root| {
            crate::comments::read_comments(doc, root)
                .into_iter()
                .enumerate()
                .map(|(index, comment)| CommentSnapshot {
                    id: comment.id,
                    number: index + 1,
                    text: comment.text.into(),
                    resolved: comment.resolved,
                })
                .collect()
        })
        .unwrap_or_default();
    PageSection {
        id: root,
        name,
        background,
        comments,
    }
}

/// Every component master's root node → its component id. One pass over the
/// library per snapshot build; the alternative (asking "is this node a master"
/// per row) would be an O(library) scan per row.
pub(crate) fn master_roots(components: &ComponentLibrary) -> HashMap<NodeId, ComponentId> {
    components
        .defs
        .values()
        .map(|def| (def.root, def.id))
        .collect()
}

/// Classify the node for the section matrix. A component master overrides the
/// underlying data variant (old Fanta's `is_component_master`); everything else
/// reads off `NodeData`, with `Group` splitting on whether it paints a surface.
pub(crate) fn classify_node_kind(node: &CanvasNode, is_master: bool) -> NodeKind {
    if is_master {
        return NodeKind::Component;
    }
    match &node.data {
        NodeData::Group(group) if group.is_frame_surface() => NodeKind::Frame,
        NodeData::Group(_) => NodeKind::Group,
        NodeData::Vector(_) => NodeKind::Shape,
        NodeData::Text(_) => NodeKind::Text,
        NodeData::Bitmap(_) => NodeKind::Image,
        NodeData::Instance(_) => NodeKind::Instance,
        _ => NodeKind::Other,
    }
}

pub(crate) fn node_section(
    doc: &Doc,
    id: NodeId,
    masters: &HashMap<NodeId, ComponentId>,
) -> Option<NodeSection> {
    let node = doc.scene.get(id)?;
    let master_id = masters.get(&id).copied();
    let kind = classify_node_kind(node, master_id.is_some());
    let bounds = doc.scene.world_bounds(id);
    let (x, y) = bounds
        .map(|bounds| (bounds.min_x, bounds.min_y))
        .unwrap_or((0.0, 0.0));
    let (width, height) = doc
        .scene
        .world_obb_size(id)
        .or_else(|| bounds.map(|bounds| (bounds.width(), bounds.height())))
        .unwrap_or((0.0, 0.0));
    Some(NodeSection {
        id,
        kind,
        type_name: node_type_name(node, kind).into(),
        type_icon: node_type_icon(node, kind),
        name: node.name.clone(),
        x,
        y,
        width,
        height,
        rotation_degrees: transform_angle(&node.transform).to_degrees(),
        corner_radius: corner_radius_value(node),
        corner_smoothing: kind
            .is_corner_capable()
            .then(|| corner_smoothing_value(node))
            .flatten(),
        opacity_percent: f64::from(node.opacity.get()) * 100.0,
        blend_mode: node.blend_mode,
        fills: node_fills(node),
        strokes: node_strokes(node),
        stroke_align: node_stroke_align(node),
        visible: !node.flags.contains(NodeFlags::HIDDEN),
        locked: node.flags.contains(NodeFlags::LOCKED),
        typography: typography_snapshot(node),
        layout: layout_snapshot(node),
        layout_child: layout_child_snapshot(doc, node),
        image_fit: match &node.data {
            NodeData::Bitmap(bitmap) => Some(bitmap.fit),
            _ => None,
        },
        instance: instance_section(doc, node),
        master: master_id.and_then(|component| master_section(doc, component)),
        effects: node
            .effects
            .iter()
            .map(|shadow| EffectSnapshot {
                kind: shadow.kind,
                color: shadow.color,
                offset: shadow.offset,
                blur: shadow.blur,
                spread: shadow.spread,
            })
            .collect(),
        blurs: node
            .blurs
            .iter()
            .map(|blur| BlurSnapshot {
                kind: blur.kind,
                radius: blur.radius,
            })
            .collect(),
        reactions: node.reactions.iter().map(reaction_summary).collect(),
        bindings: binding_snapshots(doc, node),
    })
}

pub(crate) fn layout_snapshot(node: &CanvasNode) -> Option<LayoutSnapshot> {
    let NodeData::Group(group) = &node.data else {
        return None;
    };
    Some(LayoutSnapshot {
        clip: group.clip_size.is_some(),
        auto_layout: auto_layout_snapshot(node),
    })
}

pub(crate) fn auto_layout_snapshot(node: &CanvasNode) -> Option<AutoLayoutSnapshot> {
    let NodeData::Group(group) = &node.data else {
        return None;
    };
    let layout = group.auto_layout?;
    // "Gap H" / "Gap V" are screen-axis labels: the primary-axis spacing is
    // horizontal in a horizontal stack and vertical in a vertical one, while
    // the counter spacing (between wrapped rows/columns) is the other axis.
    let (gap_h, gap_v) = match layout.mode {
        LayoutMode::Horizontal => (layout.spacing, layout.counter_spacing),
        LayoutMode::Vertical => (layout.counter_spacing, layout.spacing),
    };
    let [top, right, bottom, left] = layout.padding;
    Some(AutoLayoutSnapshot {
        mode: layout.mode,
        gap_h,
        gap_v,
        pad_h: (left == right).then_some(right),
        pad_v: (top == bottom).then_some(top),
        primary_align: layout.primary_align,
        counter_align: layout.counter_align,
        primary_sizing: layout.primary_sizing,
        counter_sizing: layout.counter_sizing,
        wrap: layout.wrap,
        reverse_z: layout.reverse_z,
    })
}

/// The 3×3 grid cell matching the frame's alignment pair, or `None` when the
/// alignment isn't a pure cell (SpaceBetween / Stretch / Baseline). Columns
/// map to the visual horizontal axis, rows to the vertical one, resolved by
/// flow direction like the original.
pub(crate) fn align_grid_active_cell(layout: &AutoLayoutSnapshot) -> Option<(u8, u8)> {
    let primary_cell = match layout.primary_align {
        PrimaryAlign::Start => 0u8,
        PrimaryAlign::Center => 1,
        PrimaryAlign::End => 2,
        PrimaryAlign::SpaceBetween | PrimaryAlign::SpaceEvenly => return None,
    };
    let counter_cell = match layout.counter_align {
        CounterAlign::Start => 0u8,
        CounterAlign::Center => 1,
        CounterAlign::End => 2,
        CounterAlign::Stretch | CounterAlign::Baseline => return None,
    };
    Some(match layout.mode {
        LayoutMode::Horizontal => (primary_cell, counter_cell),
        LayoutMode::Vertical => (counter_cell, primary_cell),
    })
}

pub(crate) fn layout_child_snapshot(doc: &Doc, node: &CanvasNode) -> Option<LayoutChildSnapshot> {
    let parent = doc.scene.get(node.parent?)?;
    let NodeData::Group(group) = &parent.data else {
        return None;
    };
    let parent_layout = group.auto_layout?;
    if !parent_layout.child_layout {
        return None;
    }
    let child = node.layout_child.unwrap_or(LayoutChild {
        grow: 0.0,
        absolute: false,
        align_self: None,
    });
    Some(LayoutChildSnapshot {
        fills_container: child.grow > 0.0,
        absolute: child.absolute,
    })
}

/// The [`ComponentDef`] an instance renders: a direct def hit, or — when the
/// instance points at a component *set* — the member matching the instance's
/// variant prop selections, falling back to the set's default variant. Mirrors
/// `fanta_doc::resolve`'s private selection so the inspector shows the same
/// variant the renderer draws.
pub(crate) fn resolved_instance_def<'a>(
    components: &'a ComponentLibrary,
    instance: &InstanceNode,
) -> Option<&'a ComponentDef> {
    if let Some(def) = components.def(instance.component) {
        return Some(def);
    }
    let set = components.sets.get(&instance.component)?;
    let exact = set.members.iter().copied().find(|member| {
        components
            .def(*member)
            .and_then(|def| def.variant_of.as_ref())
            .is_some_and(|membership| {
                membership.axis_values.iter().all(|(axis, value)| {
                    variant_selection(components, instance, axis)
                        .is_none_or(|selected| selected == *value)
                })
            })
    });
    components.def(exact.unwrap_or(set.default_variant))
}

/// The instance's selected value for a variant axis, read from its
/// `prop_values` against any member def's `Variant { axis }` prop.
pub(crate) fn variant_selection(
    components: &ComponentLibrary,
    instance: &InstanceNode,
    axis: &str,
) -> Option<String> {
    components.defs.values().find_map(|def| {
        def.props.iter().find_map(|prop| match &prop.kind {
            ComponentPropKind::Variant { axis: prop_axis } if prop_axis == axis => instance
                .prop_values
                .get(&prop.id)
                .and_then(|value| match value {
                    VarValue::String { value } => Some(value.clone()),
                    _ => None,
                }),
            _ => None,
        })
    })
}

pub(crate) fn instance_section(doc: &Doc, node: &CanvasNode) -> Option<InstanceSection> {
    let NodeData::Instance(instance) = &node.data else {
        return None;
    };
    let components = &doc.components;
    let def = resolved_instance_def(components, instance)?;
    let mut variants = Vec::new();
    let mut component_name: SharedString = def.name.clone().into();
    if let Some(membership) = &def.variant_of
        && let Some(set) = components.sets.get(&membership.set)
    {
        component_name = set.name.clone().into();
        for axis in &set.axes {
            let value = membership
                .axis_values
                .get(&axis.name)
                .cloned()
                .or_else(|| axis.values.first().cloned())
                .unwrap_or_default();
            variants.push(VariantAxisSnapshot {
                axis: axis.name.clone().into(),
                value: value.into(),
            });
        }
    }
    let props = def
        .props
        .iter()
        .filter(|prop| !matches!(prop.kind, ComponentPropKind::Variant { .. }))
        .map(|prop| {
            let value = instance.prop_values.get(&prop.id).unwrap_or(&prop.default);
            ComponentPropSnapshot {
                id: prop.id,
                name: prop.name.clone().into(),
                value: prop_value_snapshot(&prop.kind, value),
            }
        })
        .collect();
    let texts = crate::instance_text::text_clones(doc, node.id)
        .into_iter()
        .map(|(path, label, content)| InstanceTextSnapshot {
            path,
            label: label.into(),
            content: content.into(),
        })
        .collect();
    Some(InstanceSection {
        component_name,
        // The master root must still exist in the scene to be navigable.
        main_root: doc.scene.contains(def.root).then_some(def.root),
        variants,
        props,
        texts,
    })
}

/// Pick the editor a prop's current value gets. Number and Color are editable
/// (`SetInstanceProp(Float)` / `SetInstanceProp(Color)`); an instance-swap, a
/// text style or a variable alias has no inspector editor and reads out.
pub(crate) fn prop_value_snapshot(kind: &ComponentPropKind, value: &VarValue) -> PropValueSnapshot {
    match (kind, value) {
        (ComponentPropKind::Bool, VarValue::Boolean { value }) => PropValueSnapshot::Bool(*value),
        (ComponentPropKind::Text, VarValue::String { value }) => {
            PropValueSnapshot::Text(value.clone())
        }
        (ComponentPropKind::Number, VarValue::Float { value }) => PropValueSnapshot::Number(*value),
        (ComponentPropKind::Color, VarValue::Color { value }) => PropValueSnapshot::Color(*value),
        (_, VarValue::Float { value }) => PropValueSnapshot::Display(format_number(*value).into()),
        (_, VarValue::Color { value }) => PropValueSnapshot::Display(value.to_hex().into()),
        (_, VarValue::String { value }) => PropValueSnapshot::Display(value.clone().into()),
        (_, VarValue::Boolean { value }) => {
            PropValueSnapshot::Display(if *value { "On" } else { "Off" }.into())
        }
        (_, VarValue::TextStyle { .. }) => PropValueSnapshot::Display("Text style".into()),
        (_, VarValue::Alias { .. }) => PropValueSnapshot::Display("Variable".into()),
    }
}

pub(crate) fn master_section(doc: &Doc, component: ComponentId) -> Option<MasterSection> {
    let def = doc.components.def(component)?;
    let variant_set = def.variant_of.as_ref().and_then(|membership| {
        let set = doc.components.sets.get(&membership.set)?;
        Some(VariantSetSnapshot {
            set_name: set.name.clone().into(),
            axes: set
                .axes
                .iter()
                .map(|axis| VariantSetAxisSnapshot {
                    name: axis.name.clone().into(),
                    values: axis.values.join(", ").into(),
                    selected: membership
                        .axis_values
                        .get(&axis.name)
                        .cloned()
                        .unwrap_or_else(|| MIXED_VALUE.to_string())
                        .into(),
                })
                .collect(),
            is_default_variant: set.default_variant == component,
        })
    });
    Some(MasterSection {
        name: def.name.clone().into(),
        variant_set,
        props: def
            .props
            .iter()
            .map(|prop| PropSchemaSnapshot {
                name: prop.name.clone().into(),
                kind: prop_kind_label(&prop.kind).into(),
                default: prop_default_label(&prop.default).into(),
            })
            .collect(),
    })
}

pub(crate) fn prop_kind_label(kind: &ComponentPropKind) -> String {
    match kind {
        ComponentPropKind::Bool => "Boolean".to_string(),
        ComponentPropKind::Text => "Text".to_string(),
        ComponentPropKind::Number => "Number".to_string(),
        ComponentPropKind::Color => "Color".to_string(),
        ComponentPropKind::InstanceSwap => "Instance swap".to_string(),
        ComponentPropKind::Variant { axis } => format!("Variant \u{b7} {axis}"),
    }
}

pub(crate) fn prop_default_label(value: &VarValue) -> String {
    match value {
        VarValue::Boolean { value } => if *value { "On" } else { "Off" }.to_string(),
        VarValue::Float { value } => format_number(*value),
        VarValue::String { value } => value.clone(),
        VarValue::Color { value } => value.to_hex(),
        VarValue::TextStyle { .. } => "Text style".to_string(),
        VarValue::Alias { .. } => "Variable".to_string(),
    }
}

pub(crate) fn reaction_summary(reaction: &Reaction) -> SharedString {
    let trigger = match &reaction.trigger {
        Trigger::Click => "On click".to_string(),
        Trigger::Drag => "On drag".to_string(),
        Trigger::Hover => "On hover".to_string(),
        Trigger::AfterDelay { delay_ms } => format!("After {delay_ms} ms"),
        Trigger::Key { keys } => format!("On key {}", keys.join(", ")),
        Trigger::WhilePressing => "While pressing".to_string(),
    };
    let action = match &reaction.action {
        Action::Navigate { .. } => "navigate",
        Action::Back => "go back",
        Action::Close => "close",
        Action::OpenOverlay { .. } => "open overlay",
        Action::ScrollTo { .. } => "scroll to",
        Action::SetVariable { .. } => "set variable",
        Action::UpdateVariant { .. } => "change variant",
        Action::OpenLink { .. } => "open link",
    };
    format!("{trigger} → {action}").into()
}

pub(crate) fn binding_snapshots(doc: &Doc, node: &CanvasNode) -> Vec<BindingSnapshot> {
    node.bindings
        .iter()
        .map(|(prop, variable_id)| BindingSnapshot {
            property: bound_prop_label(prop).into(),
            variable: doc
                .variables
                .variable(*variable_id)
                .map(|variable| variable.name.clone())
                .unwrap_or_else(|| "Missing variable".to_string())
                .into(),
        })
        .collect()
}

pub(crate) fn bound_prop_label(prop: &BoundProp) -> String {
    match prop {
        BoundProp::FillColor { index } => format!("Fill {} color", index + 1),
        BoundProp::StrokeColor { index } => format!("Stroke {} color", index + 1),
        BoundProp::StrokeWidth { index } => format!("Stroke {} width", index + 1),
        BoundProp::CornerRadius => "Corner radius".to_string(),
        BoundProp::Opacity => "Opacity".to_string(),
        BoundProp::Visible => "Visibility".to_string(),
        BoundProp::TextContent => "Text".to_string(),
        BoundProp::TextStyle => "Text style".to_string(),
        BoundProp::ClipWidth => "Width".to_string(),
        BoundProp::ClipHeight => "Height".to_string(),
    }
}

pub(crate) fn multi_section(
    doc: &Doc,
    ids: &[NodeId],
    masters: &HashMap<NodeId, ComponentId>,
) -> MultiSection {
    let mut xs = Vec::with_capacity(ids.len());
    let mut ys = Vec::with_capacity(ids.len());
    let mut widths = Vec::with_capacity(ids.len());
    let mut heights = Vec::with_capacity(ids.len());
    let mut rotations = Vec::with_capacity(ids.len());
    let mut colors = Vec::new();
    for &id in ids {
        if let Some(bounds) = doc.scene.world_bounds(id) {
            xs.push(bounds.min_x);
            ys.push(bounds.min_y);
        }
        if let Some((width, height)) = doc.scene.world_obb_size(id) {
            widths.push(width);
            heights.push(height);
        }
        if let Some(node) = doc.scene.get(id) {
            rotations.push(transform_angle(&node.transform).to_degrees());
            node_solid_colors(node, &mut colors);
        }
    }
    let common = |values: &[f64]| -> Option<f64> {
        let first = *values.first()?;
        (values.len() == ids.len() && values.iter().all(|value| (value - first).abs() < 0.01))
            .then_some(first)
    };
    MultiSection {
        count: ids.len(),
        first_id: ids.first().copied().unwrap_or_else(NodeId::new),
        x: common(&xs),
        y: common(&ys),
        width: common(&widths),
        height: common(&heights),
        rotation_degrees: common(&rotations),
        colors: group_selection_colors(colors),
        // Only masters NOT already in a variant set can be combined; the op
        // (`combine_as_variants_operations`) filters the same way, so gating the
        // "Combine N as variants" button on the raw master count would offer it
        // for a selection that then does nothing.
        master_count: ids
            .iter()
            .filter_map(|id| masters.get(id))
            .filter(|component| {
                doc.components
                    .def(**component)
                    .is_some_and(|def| def.variant_of.is_none())
            })
            .count(),
    }
}

/// Every solid color the node paints with, in the order the inspector's paint
/// sections list them: fills (a frame's background first), then strokes, then a
/// text node's glyph color.
pub(crate) fn node_solid_colors(node: &CanvasNode, out: &mut Vec<FantaColor>) {
    let mut push_fill = |fill: &Fill| {
        if let Fill::Solid { color, .. } = fill {
            out.push(*color);
        }
    };
    match &node.data {
        NodeData::Vector(vector) => {
            vector.fills.iter().for_each(&mut push_fill);
            vector
                .strokes
                .iter()
                .for_each(|stroke| push_fill(&stroke.paint));
        }
        NodeData::Group(group) => {
            group
                .background
                .iter()
                .chain(group.background_fills.iter())
                .for_each(&mut push_fill);
            group
                .strokes
                .iter()
                .for_each(|stroke| push_fill(&stroke.paint));
        }
        NodeData::Text(text) => out.push(text.style.color),
        _ => {}
    }
}

/// Collapse the selection's solid colors into distinct rows with usage counts,
/// preserving first-seen order so the list is stable across rebuilds.
pub(crate) fn group_selection_colors(colors: Vec<FantaColor>) -> Vec<SelectionColorSnapshot> {
    let mut grouped: Vec<SelectionColorSnapshot> = Vec::new();
    for color in colors {
        match grouped.iter_mut().find(|entry| entry.color == color) {
            Some(entry) => entry.uses += 1,
            None => grouped.push(SelectionColorSnapshot { color, uses: 1 }),
        }
    }
    grouped
}

pub(crate) fn node_type_name(node: &CanvasNode, kind: NodeKind) -> &'static str {
    match kind {
        // Only these two override the data variant's own name. Everything else
        // already reads correctly from it — including `Vector` → "Shape", the
        // label the original inspector uses for every vector subtype.
        NodeKind::Component => "Component",
        NodeKind::Frame => "Frame",
        _ => node.data.default_name(),
    }
}

pub(crate) fn node_type_icon(node: &CanvasNode, kind: NodeKind) -> IconName {
    match kind {
        NodeKind::Component | NodeKind::Instance => IconName::Box,
        NodeKind::Text => IconName::ToolText,
        NodeKind::Image => IconName::Image,
        NodeKind::Shape => IconName::ToolRect,
        NodeKind::Frame | NodeKind::Group => IconName::ToolFrame,
        NodeKind::Other => match &node.data {
            NodeData::Video(_) => IconName::Image,
            _ => IconName::Box,
        },
    }
}

pub(crate) fn corner_radius_value(node: &CanvasNode) -> CornerRadiusValue {
    let (radius, radii) = match &node.data {
        NodeData::Vector(vector) => (vector.corner_radius, vector.corner_radii),
        NodeData::Group(group) => (group.corner_radius, group.corner_radii),
        _ => return CornerRadiusValue::NotApplicable,
    };
    match radii {
        Some([a, b, c, d]) if a == b && b == c && c == d => CornerRadiusValue::Uniform(a),
        Some(radii) => CornerRadiusValue::PerCorner(radii),
        None => CornerRadiusValue::Uniform(radius.unwrap_or(0.0)),
    }
}

pub(crate) fn corner_smoothing_value(node: &CanvasNode) -> Option<f64> {
    let smoothing = match &node.data {
        NodeData::Vector(vector) => vector.corner_smoothing,
        NodeData::Group(group) => group.corner_smoothing,
        _ => return None,
    };
    Some(f64::from(smoothing) * 100.0)
}

pub(crate) fn paint_snapshot(fill: &Fill, stroke_width: Option<f64>) -> PaintSnapshot {
    let (color, label, gradient, kind, opacity_percent, blend) = match fill {
        Fill::Solid { color, blend } => (
            Some(*color),
            SharedString::from(color.to_hex().trim_start_matches('#').to_string()),
            None,
            Some(PaintKind::Solid),
            Some(alpha_to_percent(color.a)),
            Some(*blend),
        ),
        Fill::Gradient { gradient, blend } => (
            None,
            GradientKind::of(gradient).label().into(),
            Some(gradient.clone()),
            Some(PaintKind::Gradient(GradientKind::of(gradient))),
            // A gradient's transparency lives on its stops, not the paint.
            None,
            Some(*blend),
        ),
        Fill::Image { opacity, blend, .. } => (
            None,
            "Image".into(),
            None,
            None,
            Some(f64::from(*opacity) * 100.0),
            Some(*blend),
        ),
    };
    PaintSnapshot {
        color,
        label,
        stroke_width,
        gradient,
        kind,
        opacity_percent,
        blend,
        visible: paint_is_visible(fill),
    }
}

pub(crate) fn alpha_to_percent(alpha: u8) -> f64 {
    f64::from(alpha) / 255.0 * 100.0
}

/// Whether a paint contributes any coverage. This is what the per-paint eye
/// reflects, since the model carries no per-paint visible flag.
pub(crate) fn paint_is_visible(fill: &Fill) -> bool {
    match fill {
        Fill::Solid { color, .. } => color.a != 0,
        Fill::Gradient { gradient, .. } => crate::color_picker::gradient_stops(gradient)
            .iter()
            .any(|stop| stop.color.a != 0),
        Fill::Image { opacity, .. } => *opacity > 0.0,
    }
}

/// Snapshot a paint's alpha so the eye can restore it on show.
pub(crate) fn paint_alpha(fill: &Fill) -> HiddenPaintAlpha {
    match fill {
        Fill::Solid { color, .. } => HiddenPaintAlpha::Solid(color.a),
        Fill::Gradient { gradient, .. } => HiddenPaintAlpha::Gradient(
            crate::color_picker::gradient_stops(gradient)
                .iter()
                .map(|stop| stop.color.a)
                .collect(),
        ),
        Fill::Image { opacity, .. } => HiddenPaintAlpha::Image(*opacity),
    }
}

pub(crate) fn paint_alpha_is_visible(alpha: &HiddenPaintAlpha) -> bool {
    match alpha {
        HiddenPaintAlpha::Solid(a) => *a != 0,
        HiddenPaintAlpha::Gradient(stops) => stops.iter().any(|a| *a != 0),
        HiddenPaintAlpha::Image(opacity) => *opacity > 0.0,
    }
}

/// The fully-transparent counterpart of `alpha`, shaped for the same paint kind.
pub(crate) fn zeroed_paint_alpha(fill: &Fill) -> HiddenPaintAlpha {
    match paint_alpha(fill) {
        HiddenPaintAlpha::Solid(_) => HiddenPaintAlpha::Solid(0),
        HiddenPaintAlpha::Gradient(stops) => HiddenPaintAlpha::Gradient(vec![0; stops.len()]),
        HiddenPaintAlpha::Image(_) => HiddenPaintAlpha::Image(0.0),
    }
}

/// The fully-opaque counterpart, used when a paint is shown with no remembered
/// alpha (a doc that loaded already-hidden, or a panel rebuilt since the hide).
pub(crate) fn opaque_paint_alpha(alpha: &HiddenPaintAlpha) -> HiddenPaintAlpha {
    match alpha {
        HiddenPaintAlpha::Solid(_) => HiddenPaintAlpha::Solid(255),
        HiddenPaintAlpha::Gradient(stops) => HiddenPaintAlpha::Gradient(vec![255; stops.len()]),
        HiddenPaintAlpha::Image(_) => HiddenPaintAlpha::Image(1.0),
    }
}

/// Write a snapshotted alpha back onto a paint. A stop-count mismatch (the
/// gradient gained or lost stops while hidden) leaves the extra stops alone.
pub(crate) fn set_paint_alpha(fill: &mut Fill, alpha: &HiddenPaintAlpha) {
    match (fill, alpha) {
        (Fill::Solid { color, .. }, HiddenPaintAlpha::Solid(a)) => color.a = *a,
        (Fill::Gradient { gradient, .. }, HiddenPaintAlpha::Gradient(alphas)) => {
            for (stop, a) in crate::color_picker::gradient_stops_mut(gradient)
                .iter_mut()
                .zip(alphas)
            {
                stop.color.a = *a;
            }
        }
        (Fill::Image { opacity, .. }, HiddenPaintAlpha::Image(value)) => *opacity = *value,
        _ => {}
    }
}

pub(crate) fn node_fills(node: &CanvasNode) -> Option<Vec<PaintSnapshot>> {
    match &node.data {
        NodeData::Vector(vector) => Some(
            vector
                .fills
                .iter()
                .map(|fill| paint_snapshot(fill, None))
                .collect(),
        ),
        NodeData::Group(group) => Some(
            group
                .background
                .iter()
                .chain(group.background_fills.iter())
                .map(|fill| paint_snapshot(fill, None))
                .collect(),
        ),
        _ => None,
    }
}

pub(crate) fn node_strokes(node: &CanvasNode) -> Option<Vec<PaintSnapshot>> {
    let strokes = match &node.data {
        NodeData::Vector(vector) => &vector.strokes,
        NodeData::Group(group) => &group.strokes,
        _ => return None,
    };
    Some(
        strokes
            .iter()
            .map(|stroke| paint_snapshot(&stroke.paint, Some(stroke.width)))
            .collect(),
    )
}

pub(crate) fn node_stroke_align(node: &CanvasNode) -> Option<StrokeAlign> {
    let strokes = match &node.data {
        NodeData::Vector(vector) => &vector.strokes,
        NodeData::Group(group) => &group.strokes,
        _ => return None,
    };
    strokes.first().map(|stroke| stroke.align)
}

pub(crate) fn typography_snapshot(node: &CanvasNode) -> Option<TypographySnapshot> {
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    Some(TypographySnapshot {
        font_family: text.style.font_family.clone(),
        size_px: text.style.size_px,
        weight: text.style.weight,
        italic: text.style.italic,
        underline: text.style.underline,
        strikethrough: text.style.strikethrough,
        line_height: text.style.line_height,
        letter_spacing: text.style.letter_spacing,
        align: text.align,
        vertical_align: text.vertical_align,
        auto_resize: text.auto_resize,
        color: text.style.color,
        color_mixed: false,
    })
}

/// Overlay the live text session's sub-selection typography onto the node's
/// snapshot, so the panel reflects (and edits) the selected characters rather
/// than the node's base style — Figma's behavior while editing text. Node-box
/// facts (align, resize) stay node-level.
pub(crate) fn overlay_selection_typography(
    typography: &mut TypographySnapshot,
    selection: &crate::text_edit::SelectionTypography,
) {
    typography.font_family = selection.style.font_family.clone();
    typography.size_px = selection.style.size_px;
    typography.weight = selection.style.weight;
    typography.italic = selection.style.italic;
    typography.underline = selection.style.underline;
    typography.strikethrough = selection.style.strikethrough;
    typography.line_height = selection.style.line_height;
    typography.letter_spacing = selection.style.letter_spacing;
    typography.color = selection.style.color;
    typography.color_mixed = selection.color_mixed;
}

#[cfg(test)]
pub(crate) mod tests {
    use fanta_doc::{GroupNode, Stroke};

    use super::*;

    #[test]
    fn tab_pairs_hop_between_2up_partners() {
        let id = NodeId::new();
        assert_eq!(
            paired_field(&InspectorField::X(id)),
            Some(InspectorField::Y(id))
        );
        assert_eq!(
            paired_field(&InspectorField::Y(id)),
            Some(InspectorField::X(id))
        );
        assert_eq!(
            paired_field(&InspectorField::Width(id)),
            Some(InspectorField::Height(id))
        );
        assert_eq!(
            paired_field(&InspectorField::CornerRadius(id)),
            Some(InspectorField::CornerSmoothing(id))
        );
        assert_eq!(
            paired_field(&InspectorField::LayoutPadV(id)),
            Some(InspectorField::LayoutPadH(id))
        );
        assert_eq!(paired_field(&InspectorField::Name(id)), None);
        assert_eq!(paired_field(&InspectorField::Opacity(id)), None);
        // Rotation lost its 2-up partner when corner radius moved to Appearance.
        assert_eq!(paired_field(&InspectorField::Rotation(id)), None);
    }

    #[test]
    fn selection_colors_are_addressed_by_color_not_node() {
        let id = NodeId::new();
        assert_eq!(field_node(&InspectorField::Opacity(id)), Some(id));
        assert_eq!(
            field_node(&InspectorField::SelectionColor {
                from: FantaColor::BLACK
            }),
            None
        );
    }

    #[test]
    fn tab_cycles_the_four_corner_radii() {
        let id = NodeId::new();
        let mut field = InspectorField::CornerRadiusCorner { id, corner: 0 };
        let mut visited = Vec::new();
        for _ in 0..4 {
            field = paired_field(&field).expect("corner fields always pair");
            if let InspectorField::CornerRadiusCorner { corner, .. } = field {
                visited.push(corner);
            }
        }
        assert_eq!(visited, vec![1, 2, 3, 0]);
    }

    #[test]
    fn scrub_values_clamp_to_field_ranges() {
        let id = NodeId::new();
        assert_eq!(
            clamp_field_value(&InspectorField::Opacity(id), 140.0),
            100.0
        );
        assert_eq!(clamp_field_value(&InspectorField::Opacity(id), -5.0), 0.0);
        assert_eq!(clamp_field_value(&InspectorField::Width(id), -10.0), 1.0);
        assert_eq!(
            clamp_field_value(&InspectorField::CornerRadius(id), -3.0),
            0.0
        );
        assert_eq!(clamp_field_value(&InspectorField::X(id), -250.0), -250.0);
        assert_eq!(
            clamp_field_value(&InspectorField::EffectSpread { id, index: 0 }, -4.0),
            -4.0
        );
    }

    #[test]
    fn scrub_values_clamp_the_new_percent_fields() {
        let id = NodeId::new();
        assert_eq!(
            clamp_field_value(&InspectorField::CornerSmoothing(id), 130.0),
            100.0
        );
        assert_eq!(
            clamp_field_value(&InspectorField::CornerSmoothing(id), -1.0),
            0.0
        );
        assert_eq!(
            clamp_field_value(
                &InspectorField::PaintOpacity {
                    id,
                    index: 0,
                    is_stroke: false
                },
                150.0
            ),
            100.0
        );
        assert_eq!(
            clamp_field_value(&InspectorField::BlurRadius { id, index: 0 }, -8.0),
            0.0
        );
    }

    #[test]
    fn text_resize_cycle_is_total() {
        assert_eq!(
            next_text_resize(TextAutoResize::None),
            TextAutoResize::WidthAndHeight
        );
        assert_eq!(
            next_text_resize(TextAutoResize::WidthAndHeight),
            TextAutoResize::Height
        );
        assert_eq!(
            next_text_resize(TextAutoResize::Height),
            TextAutoResize::None
        );
    }

    pub(crate) fn vector_with_fill(fill: Fill) -> NodeData {
        let mut vector = fanta_doc::VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, FantaColor::BLACK);
        vector.fills = smallvec::smallvec![fill];
        NodeData::Vector(vector)
    }

    pub(crate) fn node_with_data(data: NodeData) -> CanvasNode {
        CanvasNode::new(data)
    }

    pub(crate) fn frame_group() -> GroupNode {
        GroupNode {
            background: Some(Fill::solid(FantaColor::WHITE)),
            ..GroupNode::default()
        }
    }

    pub(crate) fn text_node() -> fanta_doc::TextNode {
        fanta_doc::TextNode::new("Hello", 100.0, 20.0)
    }

    #[test]
    fn node_kinds_split_frames_from_groups_and_collapse_every_shape() {
        let plain_group = node_with_data(NodeData::Group(GroupNode::default()));
        assert_eq!(classify_node_kind(&plain_group, false), NodeKind::Group);

        let frame = node_with_data(NodeData::Group(frame_group()));
        assert_eq!(classify_node_kind(&frame, false), NodeKind::Frame);

        // Rect, ellipse, star, line, boolean-op results are all one `Vector` in
        // the model and one "Shape" in the inspector.
        let rect = node_with_data(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        assert_eq!(classify_node_kind(&rect, false), NodeKind::Shape);

        let text = node_with_data(NodeData::Text(text_node()));
        assert_eq!(classify_node_kind(&text, false), NodeKind::Text);

        // A master overrides whatever data variant backs it.
        assert_eq!(classify_node_kind(&frame, true), NodeKind::Component);
        assert_eq!(classify_node_kind(&rect, true), NodeKind::Component);
    }

    #[test]
    fn only_corner_capable_kinds_get_radius_and_smoothing() {
        assert!(NodeKind::Frame.is_corner_capable());
        assert!(NodeKind::Group.is_corner_capable());
        assert!(NodeKind::Shape.is_corner_capable());
        assert!(NodeKind::Component.is_corner_capable());
        assert!(!NodeKind::Text.is_corner_capable());
        assert!(!NodeKind::Image.is_corner_capable());
        assert!(!NodeKind::Instance.is_corner_capable());
        assert!(!NodeKind::Other.is_corner_capable());
    }

    #[test]
    fn master_roots_indexes_every_def_by_its_root_node() {
        let mut library = ComponentLibrary::new();
        let root = NodeId::new();
        let component = ComponentId::new();
        library
            .defs
            .insert(component, ComponentDef::new(component, root, "Button"));
        let index = master_roots(&library);
        assert_eq!(index.get(&root), Some(&component));
        assert_eq!(index.get(&NodeId::new()), None);
    }

    #[test]
    fn corner_smoothing_value_reads_out_as_a_percent() {
        let mut vector = fanta_doc::VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, FantaColor::BLACK);
        vector.corner_smoothing = 0.45;
        let node = node_with_data(NodeData::Vector(vector));
        // The model stores an f32, so widening to a percent carries f32 dust.
        assert!((corner_smoothing_value(&node).unwrap() - 45.0).abs() < 1e-4);

        let text = node_with_data(NodeData::Text(text_node()));
        assert_eq!(corner_smoothing_value(&text), None);
    }

    #[test]
    fn selection_colors_group_by_value_and_count_uses() {
        let red = FantaColor::rgb(255, 0, 0);
        let blue = FantaColor::rgb(0, 0, 255);
        let grouped = group_selection_colors(vec![red, blue, red, red]);
        assert_eq!(grouped.len(), 2);
        // First-seen order, so the list is stable across rebuilds.
        assert_eq!(grouped[0].color, red);
        assert_eq!(grouped[0].uses, 3);
        assert_eq!(grouped[1].color, blue);
        assert_eq!(grouped[1].uses, 1);
        assert!(group_selection_colors(Vec::new()).is_empty());
    }

    #[test]
    fn selection_colors_read_fills_strokes_and_glyph_color() {
        let fill = FantaColor::rgb(1, 2, 3);
        let stroke = FantaColor::rgb(4, 5, 6);
        let mut vector = fanta_doc::VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, fill);
        vector.strokes = smallvec::smallvec![Stroke::solid(stroke, 1.0)];
        let mut colors = Vec::new();
        node_solid_colors(&node_with_data(NodeData::Vector(vector)), &mut colors);
        assert_eq!(colors, vec![fill, stroke]);

        let mut text = text_node();
        text.set_glyph_color(FantaColor::rgb(9, 9, 9));
        let mut colors = Vec::new();
        node_solid_colors(&node_with_data(NodeData::Text(text)), &mut colors);
        assert_eq!(colors, vec![FantaColor::rgb(9, 9, 9)]);

        // Gradient paints carry no single solid color, so they contribute none.
        let mut colors = Vec::new();
        node_solid_colors(
            &node_with_data(vector_with_fill(Fill::Gradient {
                gradient: Gradient::Linear {
                    start: [0.0, 0.0],
                    end: [1.0, 0.0],
                    stops: Vec::new(),
                },
                blend: BlendMode::Normal,
            })),
            &mut colors,
        );
        assert!(colors.is_empty());
    }
}
