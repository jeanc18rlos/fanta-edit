//! The Fanta properties panel: the inspector for the current canvas
//! selection, falling back to page properties when nothing is selected,
//! mirroring the original Fanta right inspector — its section stack, field
//! density, drag-to-scrub numeric fields, opacity slider, and anchored color
//! picker.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use editor::{Editor, EditorEvent, actions::SelectAll};
use fanta_canvas::{
    Axis, HAlign, ResizeHandle, VAlign, align_horizontal, align_to_bounds_h, align_to_bounds_v,
    align_vertical, distribute, resize_transform_keep_rotation, rotate_about, transform_angle,
};
use fanta_doc::{
    Action, AutoLayout, AxisSizing, BlendMode, Blur, BlurKind, BoundProp, Bounds as FantaBounds,
    CanvasNode, Color as FantaColor, ComponentDef, ComponentId, ComponentLibrary, ComponentPropId,
    ComponentPropKind, ComponentSet, ComponentSetMembership, CounterAlign, Doc, Fill, Gradient,
    GroupNode, ImageFitMode, InstanceNode, LayoutChild, LayoutMode, NodeData, NodeFlags, NodeId,
    Operation, PrimaryAlign, Reaction, Shadow, ShadowKind, Stroke, StrokeAlign, TextAlign,
    TextAutoResize, Transform2D, Trigger, VAlign as TextVAlign, VarValue, VariantAxis, Viewport,
    expand_instance,
};
use fanta_render::{AssetResolver, RasterRenderer, RenderInputs};
use fs::Fs;
use glam::DVec2;
use gpui::{
    Anchor, App, AsyncWindowContext, Bounds, Context, Div, DragMoveEvent, Entity, EventEmitter,
    FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Rgba,
    ScrollHandle, Subscription, WeakEntity, Window, actions, anchored, canvas, deferred, point, px,
    relative,
};
use settings::{Settings as _, update_settings_file};
use smallvec::SmallVec;
use ui::prelude::*;
use ui::{
    ContextMenu, ContextMenuEntry, Divider, DropdownMenu, DropdownStyle, PopoverMenu, Switch,
    ToggleState, Tooltip,
};
use util::ResultExt as _;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::color_picker::{
    ColorPicker, ColorPickerEvent, GradientEditor, GradientEditorEvent, GradientKind,
    gradient_preview_strip, representative_gradient_color, seed_gradient_from_color,
};
use crate::document::{DocChange, FigDocument, FigItem};
use crate::inspector_widgets::{
    AlignGlyph, PanelDrag, TextAlignGlyph, TextDecorationGlyph, align_glyph, scrub_value,
    text_align_glyph, text_decoration_glyph, track_value,
};
use crate::panel_settings::FantaPropertiesPanelSettings;
use crate::view::FigView;

actions!(
    fanta_properties_panel,
    [
        /// Toggle focus on the Fanta properties panel.
        ToggleFocus
    ]
);

const NO_DOCUMENT_MESSAGE: &str = "Open a Figma document to inspect properties";
const MIXED_VALUE: &str = "–";
const DEFAULT_FILL_COLOR: FantaColor = FantaColor::rgb(217, 217, 217);

/// Height of a boxed field / pill / control, the panel's vertical rhythm unit
/// (the original's 30px `FIELD_BOX_H` translated to Zed density).
const FIELD_BOX_H: f32 = 28.0;
/// Height of one fill / stroke list row (the original's 36px `LIST_ROW_H`).
const LIST_ROW_H: f32 = 32.0;
/// Height of a section-title header band (the original's `SECTION_HEADER_H`).
const SECTION_HEADER_H: f32 = 28.0;
/// The fixed label column width to the left of pills and sliders.
const PILL_LABEL_W: f32 = 68.0;
/// Side length of the auto-layout 3×3 alignment grid box.
const ALIGN_GRID_SIZE: f32 = 64.0;
/// Height of the export preview band (the original's `EXPORT_PREVIEW_H`).
const EXPORT_PREVIEW_H: f32 = 84.0;

/// One editable value in the inspector; identifies which node property the
/// shared inline editor (or an in-flight scrub / color-picker session) is
/// currently bound to.
#[derive(Debug, Clone, PartialEq)]
enum InspectorField {
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
fn field_node(field: &InspectorField) -> Option<NodeId> {
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
        | InspectorField::InstanceColorProp { id, .. } => *id,
        InspectorField::SelectionColor { .. } => return None,
    })
}

/// The field Tab jumps to from `field` — its 2-up partner (X↔Y, W↔H, R↔
/// smoothing, LH↔LS, gaps, pads, effect pairs), or the next corner of the
/// per-corner radius grid.
fn paired_field(field: &InspectorField) -> Option<InspectorField> {
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
fn clamp_field_value(field: &InspectorField, value: f64) -> f64 {
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
enum AlignCommand {
    Left,
    CenterHorizontal,
    Right,
    Top,
    MiddleVertical,
    Bottom,
    DistributeHorizontal,
    DistributeVertical,
}

const ALIGN_BUTTONS: [(AlignGlyph, &str, AlignCommand); 6] = [
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

const DISTRIBUTE_BUTTONS: [(AlignGlyph, &str, AlignCommand); 2] = [
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

const PRIMARY_ALIGNS: [(PrimaryAlign, &str); 4] = [
    (PrimaryAlign::Start, "Start"),
    (PrimaryAlign::Center, "Center"),
    (PrimaryAlign::End, "End"),
    (PrimaryAlign::SpaceBetween, "Space Between"),
];

const COUNTER_ALIGNS: [(CounterAlign, &str); 5] = [
    (CounterAlign::Start, "Start"),
    (CounterAlign::Center, "Center"),
    (CounterAlign::End, "End"),
    (CounterAlign::Stretch, "Stretch"),
    (CounterAlign::Baseline, "Baseline"),
];

const IMAGE_FIT_MODES: [(ImageFitMode, &str); 4] = [
    (ImageFitMode::Fill, "Fill"),
    (ImageFitMode::Fit, "Fit"),
    (ImageFitMode::Stretch, "Stretch"),
    (ImageFitMode::Tile, "Tile"),
];

const STROKE_ALIGNS: [(StrokeAlign, &str); 3] = [
    (StrokeAlign::Inside, "Inside"),
    (StrokeAlign::Center, "Center"),
    (StrokeAlign::Outside, "Outside"),
];

const LAYOUT_MODES: [(LayoutMode, &str); 2] = [
    (LayoutMode::Horizontal, "Horizontal"),
    (LayoutMode::Vertical, "Vertical"),
];

const AXIS_SIZINGS: [(AxisSizing, &str); 2] = [
    (AxisSizing::Fixed, "Fixed"),
    (AxisSizing::Hug, "Hug contents"),
];

/// Cap on either export dimension: bounds this large produce surfaces Skia (and
/// memory) cannot reasonably back, so the export zoom is reduced to fit instead.
const MAX_EXPORT_PIXELS: u32 = 8192;

const BLEND_MODES: [(BlendMode, &str); 16] = [
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
const FONT_WEIGHTS: [(u16, &str); 6] = [
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "SemiBold"),
    (700, "Bold"),
    (800, "ExtraBold"),
];

fn text_resize_label(resize: TextAutoResize) -> &'static str {
    match resize {
        TextAutoResize::None => "Fixed",
        TextAutoResize::WidthAndHeight => "Auto width",
        TextAutoResize::Height => "Auto height",
    }
}

fn blur_kind_label(kind: BlurKind) -> &'static str {
    match kind {
        BlurKind::Layer => "Layer blur",
        BlurKind::Background => "Background blur",
    }
}

fn next_text_resize(resize: TextAutoResize) -> TextAutoResize {
    match resize {
        TextAutoResize::None => TextAutoResize::WidthAndHeight,
        TextAutoResize::WidthAndHeight => TextAutoResize::Height,
        TextAutoResize::Height => TextAutoResize::None,
    }
}

fn stacking_label(reverse_z: bool) -> &'static str {
    if reverse_z {
        "First on top"
    } else {
        "Last on top"
    }
}

/// The paint types the fill/stroke type selector offers, in cycle order.
const PAINT_KINDS: [PaintKind; 5] = [
    PaintKind::Solid,
    PaintKind::Gradient(GradientKind::Linear),
    PaintKind::Gradient(GradientKind::Radial),
    PaintKind::Gradient(GradientKind::Angular),
    PaintKind::Gradient(GradientKind::Diamond),
];

fn paint_kind_label(kind: PaintKind) -> &'static str {
    match kind {
        PaintKind::Solid => "Solid",
        PaintKind::Gradient(gradient_kind) => gradient_kind.label(),
    }
}

enum InspectorSnapshot {
    Message(SharedString),
    Ready {
        editable: bool,
        selection_len: usize,
        body: InspectorBody,
    },
}

enum InspectorBody {
    Page(PageSection),
    Node(Box<NodeSection>),
    Multi(MultiSection),
}

struct PageSection {
    id: Option<NodeId>,
    name: String,
    background: Option<PageBackgroundValue>,
}

enum PageBackgroundValue {
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
enum NodeKind {
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
    fn is_corner_capable(self) -> bool {
        matches!(
            self,
            Self::Frame | Self::Group | Self::Shape | Self::Component
        )
    }
}

struct NodeSection {
    id: NodeId,
    kind: NodeKind,
    type_name: SharedString,
    type_icon: IconName,
    name: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    rotation_degrees: f64,
    corner_radius: CornerRadiusValue,
    /// Squircle smoothing as a whole percent. `Some` only for corner-capable
    /// nodes (Vector / Group data).
    corner_smoothing: Option<f64>,
    opacity_percent: f64,
    blend_mode: BlendMode,
    fills: Option<Vec<PaintSnapshot>>,
    strokes: Option<Vec<PaintSnapshot>>,
    stroke_align: Option<StrokeAlign>,
    visible: bool,
    locked: bool,
    typography: Option<TypographySnapshot>,
    /// `Some` for every `Group`-backed node (frame, plain group, component
    /// master with a group root): clip content, plus auto-layout when enabled.
    layout: Option<LayoutSnapshot>,
    layout_child: Option<LayoutChildSnapshot>,
    image_fit: Option<ImageFitMode>,
    instance: Option<InstanceSection>,
    master: Option<MasterSection>,
    effects: Vec<EffectSnapshot>,
    blurs: Vec<BlurSnapshot>,
    reactions: Vec<SharedString>,
    bindings: Vec<BindingSnapshot>,
}

enum CornerRadiusValue {
    NotApplicable,
    Uniform(f64),
    PerCorner([f64; 4]),
}

struct LayoutSnapshot {
    clip: bool,
    auto_layout: Option<AutoLayoutSnapshot>,
}

struct AutoLayoutSnapshot {
    mode: LayoutMode,
    gap_h: f64,
    gap_v: f64,
    /// `None` when left and right padding differ (shown as mixed).
    pad_h: Option<f64>,
    /// `None` when top and bottom padding differ (shown as mixed).
    pad_v: Option<f64>,
    primary_align: PrimaryAlign,
    counter_align: CounterAlign,
    primary_sizing: AxisSizing,
    counter_sizing: AxisSizing,
    wrap: bool,
    reverse_z: bool,
}

struct LayoutChildSnapshot {
    fills_container: bool,
    absolute: bool,
}

/// The instance-side component info: which master it renders, its variant axes,
/// and its exposed props.
struct InstanceSection {
    component_name: SharedString,
    /// The resolved master's root node, for "go to main component". `None` when
    /// the master is dangling.
    main_root: Option<NodeId>,
    variants: Vec<VariantAxisSnapshot>,
    props: Vec<ComponentPropSnapshot>,
}

/// The master-side component info, shown when the selected node is the root of
/// a [`ComponentDef`]. Read-only: editing the schema or the variant set needs
/// `SetComponentProps` / `SetComponentSet` sub-editors (deferred).
struct MasterSection {
    name: SharedString,
    /// `Some` when the master belongs to a component set: the set's name, axes,
    /// and this member's value on each axis.
    variant_set: Option<VariantSetSnapshot>,
    props: Vec<PropSchemaSnapshot>,
}

struct VariantSetSnapshot {
    set_name: SharedString,
    /// One entry per axis: the axis name, its allowed values, and this member's
    /// selection.
    axes: Vec<VariantSetAxisSnapshot>,
    is_default_variant: bool,
}

struct VariantSetAxisSnapshot {
    name: SharedString,
    values: SharedString,
    selected: SharedString,
}

struct PropSchemaSnapshot {
    name: SharedString,
    kind: SharedString,
    default: SharedString,
}

struct VariantAxisSnapshot {
    axis: SharedString,
    value: SharedString,
}

struct ComponentPropSnapshot {
    id: ComponentPropId,
    name: SharedString,
    value: PropValueSnapshot,
}

enum PropValueSnapshot {
    Bool(bool),
    Text(String),
    Number(f64),
    Color(FantaColor),
    /// Read-only display for prop kinds without an editor (instance swap, alias
    /// defaults, text styles).
    Display(SharedString),
}

struct EffectSnapshot {
    kind: ShadowKind,
    color: FantaColor,
    offset: [f64; 2],
    blur: f64,
    spread: f64,
}

struct BlurSnapshot {
    kind: BlurKind,
    radius: f64,
}

/// One distinct solid color across a multi-node selection, with how many paints
/// use it.
struct SelectionColorSnapshot {
    color: FantaColor,
    uses: usize,
}

struct BindingSnapshot {
    property: SharedString,
    variable: SharedString,
}

struct PaintSnapshot {
    color: Option<FantaColor>,
    label: SharedString,
    stroke_width: Option<f64>,
    /// The gradient this paint carries, cloned so the swatch can preview it and
    /// the gradient editor can open on it. `None` for solid / image paints.
    gradient: Option<Gradient>,
    /// The paint's kind, driving the Solid/Linear/Radial/Angular/Diamond type
    /// selector. `None` for image paints (no type control offered).
    kind: Option<PaintKind>,
    /// The paint's own opacity as a whole percent — a solid's alpha, an image
    /// fill's `opacity`. `None` for gradients, whose alpha lives per stop.
    opacity_percent: Option<f64>,
    /// The per-paint blend mode (Figma's paint-level `blendMode`). `None` for
    /// solids, which carry no blend in this model.
    blend: Option<BlendMode>,
    /// Whether the paint currently contributes any coverage. Drives the eye.
    visible: bool,
}

/// Which paint of which node a hidden-alpha memory belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PaintKey {
    id: NodeId,
    index: usize,
    is_stroke: bool,
}

/// The alpha a paint carried before the eye hid it. The data model has no
/// per-paint visible flag, so hiding zeroes the paint's alpha — remembering the
/// prior value here is what keeps a hide → show round-trip non-destructive
/// (the pre-fix code forced alpha back to 255 and only handled solids).
#[derive(Debug, Clone, PartialEq)]
enum HiddenPaintAlpha {
    Solid(u8),
    /// One alpha per gradient stop, in stop order.
    Gradient(Vec<u8>),
    Image(f32),
}

/// The paint-type choices the fill/stroke type selector cycles through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaintKind {
    Solid,
    Gradient(GradientKind),
}

struct TypographySnapshot {
    font_family: String,
    size_px: f64,
    weight: u16,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    line_height: f64,
    letter_spacing: f64,
    align: TextAlign,
    vertical_align: TextVAlign,
    auto_resize: TextAutoResize,
    /// The glyph color, which stands in for a Text node's fill stack.
    color: FantaColor,
}

struct MultiSection {
    count: usize,
    /// The first selected node, used only as a placeholder identity for the
    /// read-only multi-select field cells.
    first_id: NodeId,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    rotation_degrees: Option<f64>,
    /// Distinct solid colors used anywhere in the selection, first-seen order.
    colors: Vec<SelectionColorSnapshot>,
    /// How many of the selected nodes are component masters, gating "Combine as
    /// variants".
    master_count: usize,
}

// =============================================================================
// Scrub / preview session state
// =============================================================================

/// The pre-gesture state of the node a scrub (or color-picker session)
/// mutates. Restored right before commit so the committed operation records
/// `old` = gesture-start — one undo step per gesture, exactly like the canvas
/// tools' transient-then-commit staging.
#[derive(Clone)]
struct NodeSnapshot {
    id: NodeId,
    transform: Transform2D,
    opacity: f32,
    data: Box<NodeData>,
    effects: SmallVec<[Shadow; 0]>,
    blurs: SmallVec<[Blur; 0]>,
}

/// The panel's draggable slider tracks. Their painted bounds are captured every
/// frame so a press maps straight to a fraction of the track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliderTrack {
    Opacity,
    CornerSmoothing,
}

const SLIDER_TRACK_COUNT: usize = 2;

enum ScrubKind {
    /// Value follows the horizontal mouse delta (numeric field labels).
    Relative,
    /// Value maps the cursor to a fraction of a track (opacity slider).
    Track {
        bounds: Bounds<Pixels>,
        min: f64,
        max: f64,
    },
}

struct ScrubState {
    field: InspectorField,
    snapshot: NodeSnapshot,
    kind: ScrubKind,
    start_position: Point<Pixels>,
    start_value: f64,
    current_value: f64,
    moved: bool,
}

/// An open color-picker popover bound to one color field.
struct PickerSession {
    field: InspectorField,
    original: FantaColor,
    snapshot: NodeSnapshot,
    changed: bool,
    picker: Entity<ColorPicker>,
    _subscription: Subscription,
}

/// An open gradient-editor popover bound to one gradient paint. Mirrors
/// [`PickerSession`]: transient previews while open, one undoable op on close.
struct GradientSession {
    field: InspectorField,
    original: Gradient,
    snapshot: NodeSnapshot,
    changed: bool,
    editor: Entity<GradientEditor>,
    _subscription: Subscription,
}

pub struct FantaPropertiesPanel {
    focus_handle: FocusHandle,
    fs: Arc<dyn Fs>,
    active_view: Option<WeakEntity<FigView>>,
    width: Option<Pixels>,
    field_editor: Entity<Editor>,
    editing_field: Option<InspectorField>,
    /// Scroll state of the section stack. Reset whenever the inspected
    /// subject changes: a shorter inspector would otherwise keep the previous
    /// subject's offset and show blank space past its content.
    content_scroll: ScrollHandle,
    /// User override for the per-corner radius expander. `None` follows the
    /// node's data (expanded only when it already carries distinct corners);
    /// reset on subject change so one node's expansion doesn't leak to the next.
    corner_radii_expanded: Option<bool>,
    /// An in-flight drag-to-scrub gesture (field label or a slider track).
    scrub: Option<ScrubState>,
    /// Each slider track's window bounds, captured during paint so a click/drag
    /// on the track maps to a 0–100% fraction. Indexed by [`SliderTrack`].
    slider_tracks: [Option<Bounds<Pixels>>; SLIDER_TRACK_COUNT],
    /// The alpha each hidden paint carried before its eye was toggled off, so
    /// showing it again restores the value instead of forcing full opacity.
    /// View-only state: cleared whenever the inspected subject changes.
    hidden_paint_alpha: HashMap<PaintKey, HiddenPaintAlpha>,
    /// The open color-picker popover, if any.
    picker: Option<PickerSession>,
    /// The open gradient-editor popover, if any.
    gradient_editor: Option<GradientSession>,
    /// Set on a swatch mouse-down when that swatch's popover is already open, so
    /// the click's release (which the popover's mouse-down-out handler has by
    /// then closed) does not immediately reopen it — letting a second click on a
    /// swatch toggle its popover shut. Only one popover is open at a time, so a
    /// single flag covers both the color and gradient swatches.
    swatch_press_dismissed: bool,
    _subscriptions: Vec<Subscription>,
    _active_view_subscription: Option<Subscription>,
}

impl FantaPropertiesPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            Self::new(workspace, window, cx)
        })
    }

    pub(crate) fn new_embedded(
        active_view: Entity<FigView>,
        fs: Arc<dyn Fs>,
        window: &mut Window,
        cx: &mut Context<FigView>,
    ) -> Entity<Self> {
        cx.new(|cx| Self::build(fs, Some(active_view), window, cx, Vec::new()))
    }

    fn new(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let fs = workspace.app_state().fs.clone();
        // The workspace entity is mid-update here, so the initial active view
        // must come from the `&mut Workspace` we were handed — reading the
        // entity would double-lease and panic.
        let initial_view = workspace
            .active_item(cx)
            .and_then(|item| item.downcast::<FigView>());
        let workspace_entity = cx.entity();
        cx.new(|cx| {
            let workspace_subscription = cx.subscribe_in(
                &workspace_entity,
                window,
                |this: &mut Self, workspace, event, window, cx| {
                    if matches!(event, workspace::Event::ActiveItemChanged) {
                        this.update_active_view(workspace, window, cx);
                    }
                },
            );
            Self::build(fs, initial_view, window, cx, vec![workspace_subscription])
        })
    }

    fn build(
        fs: Arc<dyn Fs>,
        initial_view: Option<Entity<FigView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
        mut subscriptions: Vec<Subscription>,
    ) -> Self {
        let field_editor = cx.new(|cx| Editor::single_line(window, cx));
        subscriptions.push(cx.subscribe_in(
            &field_editor,
            window,
            |this: &mut Self, _, event: &EditorEvent, _window, cx| {
                if matches!(event, EditorEvent::Blurred) && this.editing_field.is_some() {
                    this.stop_editing(cx);
                }
            },
        ));
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            fs,
            active_view: None,
            width: None,
            field_editor,
            editing_field: None,
            content_scroll: ScrollHandle::new(),
            corner_radii_expanded: None,
            scrub: None,
            slider_tracks: [None; SLIDER_TRACK_COUNT],
            hidden_paint_alpha: HashMap::new(),
            picker: None,
            gradient_editor: None,
            swatch_press_dismissed: false,
            _subscriptions: subscriptions,
            _active_view_subscription: None,
        };
        this.set_active_view(initial_view, cx);
        this
    }

    fn update_active_view(
        &mut self,
        workspace: &Entity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_view = workspace
            .read(cx)
            .active_item(cx)
            .and_then(|item| item.downcast::<FigView>());
        self.set_active_view(active_view, cx);
    }

    fn set_active_view(&mut self, active_view: Option<Entity<FigView>>, cx: &mut Context<Self>) {
        match active_view {
            Some(view) => {
                let is_same = self
                    .active_view
                    .as_ref()
                    .is_some_and(|previous| previous.entity_id() == view.entity_id());
                if !is_same {
                    // Subscribe to the item's event stream rather than
                    // observing the view: the view notifies on every pan and
                    // pointer-move frame, which re-rendered the inspector at
                    // input rate. Transient preview frames still refresh it —
                    // live X/Y during drags — but that render is O(selection),
                    // not O(document).
                    let item = view.read(cx).item().clone();
                    self._active_view_subscription = Some(cx.subscribe(
                        &item,
                        |this, _, event: &crate::document::FigItemEvent, cx| {
                            match event {
                                crate::document::FigItemEvent::SelectionChanged => {
                                    this.reset_for_new_subject(true, cx);
                                }
                                crate::document::FigItemEvent::StateChanged => {
                                    // The document may have been replaced from
                                    // disk; a stale snapshot restore would
                                    // resurrect old content, so abandon any
                                    // preview without restoring.
                                    this.reset_for_new_subject(false, cx);
                                }
                                _ => {}
                            }
                            cx.notify();
                        },
                    ));
                    self.active_view = Some(view.downgrade());
                    self.editing_field = None;
                    self.scrub = None;
                    self.picker = None;
                    self.gradient_editor = None;
                    self.content_scroll.set_offset(gpui::Point::default());
                    self.corner_radii_expanded = None;
                    self.hidden_paint_alpha.clear();
                }
            }
            None => {
                // Keep the last canvas bound while other items are active so
                // the inspector doesn't flicker away on focus changes.
            }
        }
        cx.notify();
    }

    /// Reset per-subject view state when the inspected subject changes.
    /// `restore_previews` restores any in-flight scrub / picker preview to its
    /// gesture-start state first (safe while the document is unchanged; not
    /// safe across a reload, where the snapshot would resurrect stale data).
    fn reset_for_new_subject(&mut self, restore_previews: bool, cx: &mut Context<Self>) {
        self.cancel_scrub(restore_previews, cx);
        self.abandon_color_picker(restore_previews, cx);
        self.abandon_gradient_editor(restore_previews, cx);
        self.content_scroll.set_offset(gpui::Point::default());
        self.corner_radii_expanded = None;
        self.hidden_paint_alpha.clear();
    }

    fn active_view(&self, _cx: &App) -> Option<Entity<FigView>> {
        self.active_view.as_ref().and_then(|view| view.upgrade())
    }

    fn active_item(&self, cx: &App) -> Option<Entity<FigItem>> {
        Some(self.active_view(cx)?.read(cx).item().clone())
    }

    // === Mutations ========================================================

    /// Build operations against the current document and apply them through
    /// the item so undo history and dirty tracking stay correct.
    ///
    /// A gesture that authors more than one operation (aligning a selection,
    /// replacing a color across it, detaching an instance) is wrapped in a
    /// single history transaction, so the whole gesture collapses to one undo
    /// step instead of one per node.
    fn apply_document_ops(
        &mut self,
        cx: &mut Context<Self>,
        build: impl FnOnce(&Doc) -> Vec<Operation>,
    ) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let operations = {
            let item_state = item.read(cx);
            if !item_state.is_editable() {
                return;
            }
            let Some(document) = item_state.document() else {
                return;
            };
            finite_transform_operations(build(&document.doc))
        };
        match operations.len() {
            0 => {}
            1 => {
                item.update(cx, |item, cx| {
                    for operation in operations {
                        if let Err(error) = item.apply(operation, cx) {
                            log::error!(
                                "Fanta properties panel failed to apply operation: {error:#}"
                            );
                        }
                    }
                });
            }
            _ => self.apply_document_transaction(&item, operations, cx),
        }
    }

    /// Apply several operations as one undoable transaction. `Doc::apply`
    /// appends to an open transaction instead of committing per-op, so the
    /// whole batch pushes a single undo entry.
    fn apply_document_transaction(
        &mut self,
        item: &Entity<FigItem>,
        operations: Vec<Operation>,
        cx: &mut Context<Self>,
    ) {
        let label = operations
            .first()
            .map(|operation| operation.label().to_string())
            .unwrap_or_else(|| "Edit".to_string());
        item.update(cx, |item, cx| {
            let applied = item.with_document(cx, |document| {
                let doc = &mut document.doc;
                doc.history.begin(label, &mut doc.scene);
                for operation in operations {
                    if let Err(error) = doc.apply(operation) {
                        log::error!("Fanta properties panel failed to apply operation: {error:#}");
                        break;
                    }
                }
                doc.history.commit(&mut doc.scene);
                ((), DocChange::Content)
            });
            if applied.is_none() {
                log::debug!("dropping inspector edit: the document is not ready");
            }
        });
    }

    fn apply_align(&mut self, command: AlignCommand, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, |doc| {
            let ids: Vec<NodeId> = doc.selection.iter().copied().collect();
            let scene = &doc.scene;
            match command {
                AlignCommand::Left => align_horizontal(scene, &ids, HAlign::Left),
                AlignCommand::CenterHorizontal => align_horizontal(scene, &ids, HAlign::Center),
                AlignCommand::Right => align_horizontal(scene, &ids, HAlign::Right),
                AlignCommand::Top => align_vertical(scene, &ids, VAlign::Top),
                AlignCommand::MiddleVertical => align_vertical(scene, &ids, VAlign::Middle),
                AlignCommand::Bottom => align_vertical(scene, &ids, VAlign::Bottom),
                AlignCommand::DistributeHorizontal => distribute(scene, &ids, Axis::X),
                AlignCommand::DistributeVertical => distribute(scene, &ids, Axis::Y),
            }
        });
    }

    fn update_node_data(
        &mut self,
        id: NodeId,
        mutate: impl FnOnce(&mut NodeData),
        cx: &mut Context<Self>,
    ) {
        self.apply_document_ops(cx, move |doc| replace_data_operation(doc, id, mutate));
    }

    fn toggle_flag(&mut self, id: NodeId, flag: NodeFlags, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            vec![Operation::SetFlags {
                id,
                old: node.flags,
                new: node.flags ^ flag,
            }]
        });
    }

    fn set_blend_mode(&mut self, id: NodeId, blend_mode: BlendMode, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            if node.blend_mode == blend_mode {
                return Vec::new();
            }
            vec![Operation::SetBlendMode {
                id,
                old: node.blend_mode,
                new: blend_mode,
            }]
        });
    }

    fn add_paint(&mut self, id: NodeId, is_stroke: bool, cx: &mut Context<Self>) {
        self.forget_hidden_paint_alpha(id, is_stroke);
        self.update_node_data(
            id,
            move |data| {
                if is_stroke {
                    if let Some(strokes) = stroke_list_mut(data) {
                        strokes.push(Stroke::solid(FantaColor::BLACK, 1.0));
                    }
                } else {
                    add_fill(data);
                }
            },
            cx,
        );
    }

    /// Drop the visibility-alpha memory for a paint list whose indices are about
    /// to shift. `hidden_paint_alpha` keys paints by list position, so after an
    /// add/remove a surviving key would resolve to a *different* paint; the
    /// memory is best-effort (show falls back to opaque when absent), so forget
    /// it rather than restore the wrong paint's alpha.
    fn forget_hidden_paint_alpha(&mut self, id: NodeId, is_stroke: bool) {
        self.hidden_paint_alpha
            .retain(|key, _| !(key.id == id && key.is_stroke == is_stroke));
    }

    fn remove_paint(&mut self, id: NodeId, index: usize, is_stroke: bool, cx: &mut Context<Self>) {
        self.forget_hidden_paint_alpha(id, is_stroke);
        self.update_node_data(
            id,
            move |data| {
                if is_stroke {
                    if let Some(strokes) = stroke_list_mut(data)
                        && index < strokes.len()
                    {
                        strokes.remove(index);
                    }
                } else {
                    remove_fill(data, index);
                }
            },
            cx,
        );
    }

    /// Toggle a paint's visibility by zeroing / restoring its alpha — the
    /// closest analog of the original's per-paint eye in a data model that
    /// carries no per-paint visible flag. Works for every paint kind (a solid's
    /// alpha, every gradient stop's alpha, an image fill's opacity) and, on
    /// show, restores the alpha the paint had when it was hidden rather than
    /// forcing it fully opaque.
    fn toggle_paint_visibility(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        cx: &mut Context<Self>,
    ) {
        let key = PaintKey {
            id,
            index,
            is_stroke,
        };
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let current = {
            let item_state = item.read(cx);
            let Some(document) = item_state.document() else {
                return;
            };
            let Some(node) = document.doc.scene.get(id) else {
                return;
            };
            let mut data = node.data.clone();
            paint_slot_mut(&mut data, index, is_stroke).map(|paint| paint_alpha(paint))
        };
        let Some(current) = current else {
            return;
        };
        let restore = if paint_alpha_is_visible(&current) {
            self.hidden_paint_alpha.insert(key, current);
            None
        } else {
            // Unknown prior alpha (a doc that loaded with a zeroed paint, or a
            // panel rebuilt since): fall back to fully opaque. A remembered
            // alpha whose kind no longer matches the live paint (the paint was
            // converted solid↔gradient↔image while hidden) is discarded too —
            // `set_paint_alpha` would silently no-op on the mismatch, leaving
            // the "show" click dead.
            Some(
                self.hidden_paint_alpha
                    .remove(&key)
                    .filter(|remembered| {
                        std::mem::discriminant(remembered) == std::mem::discriminant(&current)
                    })
                    .unwrap_or_else(|| opaque_paint_alpha(&current)),
            )
        };
        self.update_node_data(
            id,
            move |data| {
                if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
                    match &restore {
                        Some(alpha) => set_paint_alpha(paint, alpha),
                        None => set_paint_alpha(paint, &zeroed_paint_alpha(paint)),
                    }
                }
            },
            cx,
        );
    }

    fn set_font_weight(&mut self, id: NodeId, weight: u16, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    text.style.weight = weight;
                }
            },
            cx,
        );
    }

    fn toggle_text_decoration(
        &mut self,
        id: NodeId,
        decoration: TextDecorationGlyph,
        cx: &mut Context<Self>,
    ) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    let flag = match decoration {
                        TextDecorationGlyph::Italic => &mut text.style.italic,
                        TextDecorationGlyph::Underline => &mut text.style.underline,
                        TextDecorationGlyph::Strikethrough => &mut text.style.strikethrough,
                    };
                    *flag = !*flag;
                }
            },
            cx,
        );
    }

    fn set_text_align(&mut self, id: NodeId, align: TextAlign, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    text.align = align;
                }
            },
            cx,
        );
    }

    fn set_text_vertical_align(&mut self, id: NodeId, align: TextVAlign, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    text.vertical_align = align;
                }
            },
            cx,
        );
    }

    fn cycle_text_resize(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            |data| {
                if let NodeData::Text(text) = data {
                    text.auto_resize = next_text_resize(text.auto_resize);
                }
            },
            cx,
        );
    }

    fn set_stroke_align(&mut self, id: NodeId, align: StrokeAlign, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.align = align;
                    }
                }
            },
            cx,
        );
    }

    fn add_effect(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            effects_operations(doc, id, |effects| effects.push(default_shadow()))
        });
    }

    fn remove_effect(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            effects_operations(doc, id, |effects| {
                if index < effects.len() {
                    effects.remove(index);
                }
            })
        });
    }

    fn toggle_effect_kind(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            effects_operations(doc, id, |effects| {
                if let Some(shadow) = effects.get_mut(index) {
                    shadow.kind = match shadow.kind {
                        ShadowKind::Drop => ShadowKind::Inner,
                        ShadowKind::Inner => ShadowKind::Drop,
                    };
                }
            })
        });
    }

    fn add_blur(&mut self, id: NodeId, kind: BlurKind, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            blurs_operations(doc, id, |blurs| blurs.push(default_blur(kind)))
        });
    }

    fn remove_blur(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            blurs_operations(doc, id, |blurs| {
                if index < blurs.len() {
                    blurs.remove(index);
                }
            })
        });
    }

    fn set_blur_kind(&mut self, id: NodeId, index: usize, kind: BlurKind, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            blurs_operations(doc, id, |blurs| {
                if let Some(blur) = blurs.get_mut(index) {
                    blur.kind = kind;
                }
            })
        });
    }

    /// Add or remove a frame/group's auto layout. `FigItem::apply` flips the
    /// document's cached `uses_auto_layout` gate on when an op introduces the
    /// first auto layout, so the frame is re-solved immediately.
    fn toggle_auto_layout(&mut self, id: NodeId, enable: bool, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            // The solver treats an auto-layout group as a frame that carries a
            // `clip_size` — imported frames always do, but a plain group has
            // none. Seed one from the current content bounds when enabling on a
            // clip-less group, or a later "Hug contents" sizing collapses the
            // frame to a zero-extent box and clips away every child. Mirrors
            // `toggle_clip_content`.
            let local_size = doc
                .scene
                .local_bounds(id)
                .map(|bounds| [bounds.width(), bounds.height()]);
            replace_data_operation(doc, id, move |data| {
                if let NodeData::Group(group) = data {
                    group.auto_layout = enable.then(AutoLayout::default);
                    if enable && group.clip_size.is_none() {
                        group.clip_size = local_size;
                    }
                }
            })
        });
    }

    fn update_auto_layout(
        &mut self,
        id: NodeId,
        mutate: impl FnOnce(&mut AutoLayout),
        cx: &mut Context<Self>,
    ) {
        self.update_node_data(
            id,
            |data| {
                if let NodeData::Group(group) = data
                    && let Some(layout) = group.auto_layout.as_mut()
                {
                    mutate(layout);
                }
            },
            cx,
        );
    }

    fn set_layout_direction(&mut self, id: NodeId, mode: LayoutMode, cx: &mut Context<Self>) {
        self.update_auto_layout(id, move |layout| layout.mode = mode, cx);
    }

    fn set_primary_align(&mut self, id: NodeId, align: PrimaryAlign, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.primary_align = align, cx);
    }

    fn set_counter_align(&mut self, id: NodeId, align: CounterAlign, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.counter_align = align, cx);
    }

    /// Set both auto-layout alignments from a clicked 3×3 grid cell. Which
    /// visual axis is "primary" depends on the flow direction, like the
    /// original's Figma-style grid.
    fn set_align_cell(&mut self, id: NodeId, col: u8, row: u8, cx: &mut Context<Self>) {
        self.update_auto_layout(
            id,
            move |layout| {
                let (primary_cell, counter_cell) = match layout.mode {
                    LayoutMode::Horizontal => (col, row),
                    LayoutMode::Vertical => (row, col),
                };
                layout.primary_align = match primary_cell {
                    0 => PrimaryAlign::Start,
                    1 => PrimaryAlign::Center,
                    _ => PrimaryAlign::End,
                };
                layout.counter_align = match counter_cell {
                    0 => CounterAlign::Start,
                    1 => CounterAlign::Center,
                    _ => CounterAlign::End,
                };
            },
            cx,
        );
    }

    fn set_primary_axis_sizing(&mut self, id: NodeId, sizing: AxisSizing, cx: &mut Context<Self>) {
        self.update_auto_layout(id, move |layout| layout.primary_sizing = sizing, cx);
    }

    fn set_counter_axis_sizing(&mut self, id: NodeId, sizing: AxisSizing, cx: &mut Context<Self>) {
        self.update_auto_layout(id, move |layout| layout.counter_sizing = sizing, cx);
    }

    fn toggle_layout_wrap(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.wrap = !layout.wrap, cx);
    }

    fn toggle_layout_stacking(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.reverse_z = !layout.reverse_z, cx);
    }

    fn toggle_clip_content(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            let local_size = doc
                .scene
                .local_bounds(id)
                .map(|bounds| [bounds.width(), bounds.height()])
                .unwrap_or([0.0, 0.0]);
            replace_data_operation(doc, id, |data| {
                if let NodeData::Group(group) = data {
                    group.clip_size = match group.clip_size {
                        Some(_) => None,
                        None => Some(local_size),
                    };
                }
            })
        });
    }

    fn update_layout_child(
        &mut self,
        id: NodeId,
        mutate: impl FnOnce(&mut LayoutChild),
        cx: &mut Context<Self>,
    ) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            let old = node.layout_child;
            let mut child = old.unwrap_or(LayoutChild {
                grow: 0.0,
                absolute: false,
                align_self: None,
            });
            mutate(&mut child);
            let new = (!child.is_trivial()).then_some(child);
            if new == old {
                return Vec::new();
            }
            vec![Operation::SetLayoutChild { id, old, new }]
        });
    }

    fn set_image_fit(&mut self, id: NodeId, fit: ImageFitMode, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Bitmap(bitmap) = data {
                    bitmap.fit = fit;
                }
            },
            cx,
        );
    }

    fn set_instance_prop(
        &mut self,
        id: NodeId,
        prop: ComponentPropId,
        new: Option<VarValue>,
        cx: &mut Context<Self>,
    ) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            let NodeData::Instance(instance) = &node.data else {
                return Vec::new();
            };
            let old = instance.prop_values.get(&prop).cloned();
            if old == new {
                return Vec::new();
            }
            vec![Operation::SetInstanceProp { id, prop, old, new }]
        });
    }

    fn cycle_variant_axis(&mut self, id: NodeId, axis: SharedString, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            variant_cycle_operations(doc, id, axis.as_ref())
        });
    }

    /// Replace the instance with editable copies of its master's subtree. One
    /// undoable transaction: the detach itself, plus the ops that fold the
    /// master root's own surface props onto the (now plain) frame, which the
    /// data swap alone would drop.
    fn detach_instance(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| detach_instance_operations(doc, id));
    }

    /// Merge the selected component masters into one variant set, so their
    /// instances can switch between them along a single axis.
    fn combine_as_variants(&mut self, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, combine_as_variants_operations);
    }

    /// Select and scroll to the master a component instance renders, switching
    /// pages when the master lives on the hidden Components page.
    fn focus_main_component(&mut self, target: NodeId, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().clone();
        let selected_page = view.read(cx).selected_page_index();
        let (page_index, current_index) = {
            let fig_item = item.read(cx);
            let Some(document) = fig_item.document() else {
                return;
            };
            (
                document.page_index_of_node(target),
                document.page_index(selected_page),
            )
        };
        view.update(cx, |view, cx| {
            if let Some(index) = page_index
                && Some(index) != current_index
            {
                view.select_page(index, cx);
            }
            view.focus_node(target, cx);
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(target);
                ((), DocChange::Selection)
            });
        });
    }

    /// Set one paint's per-paint blend mode (gradient / image paints only —
    /// solids carry no blend in this model).
    fn set_paint_blend(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        blend: BlendMode,
        cx: &mut Context<Self>,
    ) {
        self.update_node_data(
            id,
            move |data| {
                if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
                    match paint {
                        Fill::Gradient { blend: slot, .. } | Fill::Image { blend: slot, .. } => {
                            *slot = blend;
                        }
                        Fill::Solid { .. } => {}
                    }
                }
            },
            cx,
        );
    }

    // === Export ===========================================================

    /// Render the selected node (or the whole page when nothing is selected)
    /// at 2x into `<project_root>/exports/<name>.png` on a background thread.
    fn export_png(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let (item, selected_page_index) = {
            let view = view.read(cx);
            (view.item().clone(), view.selected_page_index())
        };
        let job = {
            let item = item.read(cx);
            let Some(project_root) = item.project_root().map(PathBuf::from) else {
                log::error!("Fanta PNG export requires a Fanta project on disk");
                return;
            };
            let Some(document) = item.document() else {
                return;
            };
            let doc = &document.doc;
            let selected = doc
                .selection
                .iter()
                .copied()
                .find(|id| doc.scene.contains(*id));
            let (root, name, bounds) = match selected {
                Some(id) => {
                    let Some(bounds) = doc.scene.world_bounds(id).filter(|bounds| {
                        bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0
                    }) else {
                        log::error!("Fanta PNG export skipped: the selected node has no bounds");
                        return;
                    };
                    let name = doc
                        .scene
                        .get(id)
                        .map(|node| node.name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "Untitled".to_string());
                    (Some(id), name, bounds)
                }
                None => {
                    let page = document.page(selected_page_index);
                    let root = page.and_then(|page| page.root);
                    let name = page
                        .map(|page| page.name.to_string())
                        .unwrap_or_else(|| "Page".to_string());
                    (root, name, crate::document::page_bounds(doc, root))
                }
            };
            ExportJob {
                doc: doc.clone(),
                asset_resolver: document.asset_resolver.clone(),
                root,
                name,
                bounds,
                project_root,
            }
        };
        cx.background_spawn(async move {
            match run_png_export(&job) {
                Ok(path) => log::info!("Fanta PNG export written to {}", path.display()),
                Err(error) => log::error!("Fanta PNG export failed: {error:#}"),
            }
        })
        .detach();
    }

    // === Inline editing ===================================================

    fn start_editing(
        &mut self,
        field: InspectorField,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_field = Some(field);
        self.field_editor.update(cx, |editor, cx| {
            editor.set_text(initial, window, cx);
            editor.select_all(&SelectAll, window, cx);
        });
        self.field_editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.editing_field = None;
        cx.notify();
    }

    fn cancel_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_editing(cx);
        self.focus_handle.focus(window, cx);
    }

    fn commit_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.editing_field.take() else {
            return;
        };
        let text = self.field_editor.read(cx).text(cx);
        self.focus_handle.focus(window, cx);
        cx.notify();
        self.apply_document_ops(cx, |doc| field_operations(doc, &field, text.trim()));
    }

    /// The committed display text a field would show right now, used to seed
    /// the editor when Tab hops to the paired field.
    fn field_display_text(&self, field: &InspectorField, cx: &App) -> Option<String> {
        let item = self.active_item(cx)?;
        let item = item.read(cx);
        let document = item.document()?;
        read_field_text(&document.doc, field)
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_field.is_some() {
            match event.keystroke.key.as_str() {
                "enter" => {
                    cx.stop_propagation();
                    self.commit_editing(window, cx);
                }
                "escape" => {
                    cx.stop_propagation();
                    self.cancel_editing(window, cx);
                }
                "tab" => {
                    cx.stop_propagation();
                    let field = self.editing_field.clone();
                    self.commit_editing(window, cx);
                    if let Some(field) = field
                        && let Some(next) = paired_field(&field)
                        && let Some(initial) = self.field_display_text(&next, cx)
                    {
                        self.start_editing(next, initial, window, cx);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.picker.is_some() {
            match event.keystroke.key.as_str() {
                "enter" => {
                    cx.stop_propagation();
                    self.close_color_picker(true, cx);
                }
                "escape" => {
                    cx.stop_propagation();
                    self.close_color_picker(false, cx);
                }
                _ => {}
            }
            return;
        }
        if self.gradient_editor.is_some() {
            match event.keystroke.key.as_str() {
                "enter" => {
                    cx.stop_propagation();
                    self.close_gradient_editor(true, cx);
                }
                "escape" => {
                    cx.stop_propagation();
                    self.close_gradient_editor(false, cx);
                }
                _ => {}
            }
        }
    }

    // === Drag-to-scrub ====================================================

    /// Snapshot the state a gesture on `field` will mutate, so the preview can
    /// be recomputed from gesture-start each frame and the commit records
    /// `old` = gesture-start. `None` when the document isn't editable.
    fn snapshot_for_field(&self, field: &InspectorField, cx: &App) -> Option<NodeSnapshot> {
        let item = self.active_item(cx)?;
        let item = item.read(cx);
        if !item.is_editable() {
            return None;
        }
        let document = item.document()?;
        let id = field_node(field)?;
        let node = document.doc.scene.get(id)?;
        Some(NodeSnapshot {
            id,
            transform: node.transform,
            opacity: node.opacity,
            data: Box::new(node.data.clone()),
            effects: node.effects.clone(),
            blurs: node.blurs.clone(),
        })
    }

    fn begin_field_scrub(
        &mut self,
        field: InspectorField,
        start_value: f64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.finish_scrub(cx);
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        self.scrub = Some(ScrubState {
            field,
            snapshot,
            kind: ScrubKind::Relative,
            start_position: position,
            start_value,
            current_value: start_value,
            moved: false,
        });
    }

    /// Begin a slider gesture: the press position maps straight to a 0–100%
    /// value (a click alone sets and commits it on release).
    fn begin_track_scrub(
        &mut self,
        track: SliderTrack,
        field: InspectorField,
        start_percent: f64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.finish_scrub(cx);
        let Some(bounds) = self.slider_tracks[track as usize] else {
            return;
        };
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        let value = clamp_field_value(
            &field,
            track_value(
                f64::from(position.x),
                f64::from(bounds.left()),
                f64::from(bounds.size.width),
                0.0,
                100.0,
            ),
        );
        self.scrub = Some(ScrubState {
            field: field.clone(),
            snapshot: snapshot.clone(),
            kind: ScrubKind::Track {
                bounds,
                min: 0.0,
                max: 100.0,
            },
            start_position: position,
            start_value: start_percent,
            current_value: value,
            moved: true,
        });
        self.preview_field_text(&field, &snapshot, &format_number(value), cx);
    }

    fn handle_scrub_move(
        &mut self,
        event: &DragMoveEvent<PanelDrag>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scrub) = self.scrub.as_mut() else {
            return;
        };
        let position = event.event.position;
        let modifiers = event.event.modifiers;
        let value = match &scrub.kind {
            ScrubKind::Relative => {
                let dx = f64::from(position.x - scrub.start_position.x);
                scrub_value(scrub.start_value, dx, modifiers.shift, modifiers.alt)
            }
            ScrubKind::Track { bounds, min, max } => track_value(
                f64::from(position.x),
                f64::from(bounds.left()),
                f64::from(bounds.size.width),
                *min,
                *max,
            ),
        };
        let value = clamp_field_value(&scrub.field, value);
        if scrub.moved && value == scrub.current_value {
            return;
        }
        scrub.moved = true;
        scrub.current_value = value;
        let field = scrub.field.clone();
        let snapshot = scrub.snapshot.clone();
        self.preview_field_text(&field, &snapshot, &format_number(value), cx);
    }

    /// Commit an in-flight scrub as ONE undoable operation: restore the
    /// gesture-start state, then author the op from there to the final value —
    /// the same transient-then-commit staging the canvas move tool uses.
    fn finish_scrub(&mut self, cx: &mut Context<Self>) {
        let Some(scrub) = self.scrub.take() else {
            return;
        };
        if !scrub.moved {
            return;
        }
        self.restore_snapshot_preview(&scrub.snapshot, cx);
        if scrub.current_value != scrub.start_value {
            let field = scrub.field.clone();
            let text = format_number(scrub.current_value);
            self.apply_document_ops(cx, |doc| field_operations(doc, &field, &text));
        }
        cx.notify();
    }

    /// Drop an in-flight scrub. `restore` puts the document back to the
    /// gesture-start state (skip after a reload, where the snapshot is stale).
    fn cancel_scrub(&mut self, restore: bool, cx: &mut Context<Self>) {
        if let Some(scrub) = self.scrub.take()
            && scrub.moved
            && restore
        {
            self.restore_snapshot_preview(&scrub.snapshot, cx);
        }
    }

    /// Apply `text` to `field` as a TRANSIENT preview: restore the snapshot,
    /// author the same operations the commit path would, and write their `new`
    /// values straight into the scene (no history), reporting a
    /// `ContentPreview` so the canvas repaints without a layout re-solve.
    fn preview_field_text(
        &self,
        field: &InspectorField,
        snapshot: &NodeSnapshot,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let field = field.clone();
        let snapshot = snapshot.clone();
        let text = text.to_string();
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                return;
            }
            let applied = item.with_document(cx, |document| {
                restore_snapshot(&mut document.doc, &snapshot);
                let operations =
                    finite_transform_operations(field_operations(&document.doc, &field, &text));
                for operation in &operations {
                    apply_preview_operation(&mut document.doc, operation);
                }
                ((), DocChange::ContentPreview)
            });
            if applied.is_none() {
                log::debug!("dropping inspector preview: the document is not ready");
            }
        });
    }

    fn restore_snapshot_preview(&self, snapshot: &NodeSnapshot, cx: &mut Context<Self>) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let snapshot = snapshot.clone();
        item.update(cx, |item, cx| {
            let applied = item.with_document(cx, |document| {
                restore_snapshot(&mut document.doc, &snapshot);
                ((), DocChange::ContentPreview)
            });
            if applied.is_none() {
                log::debug!("dropping inspector preview restore: the document is not ready");
            }
        });
    }

    // === Color picker =====================================================

    /// Open (or toggle closed) the color-picker popover for `field`, seeded
    /// with the paint's current color. While open, every picker change
    /// previews transiently; closing commits one undoable operation.
    fn toggle_color_picker(
        &mut self,
        field: InspectorField,
        current: FantaColor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .picker
            .as_ref()
            .is_some_and(|session| session.field == field)
        {
            self.close_color_picker(true, cx);
            return;
        }
        self.close_color_picker(true, cx);
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        let picker = cx.new(|cx| ColorPicker::new(current, window, cx));
        let subscription = cx.subscribe(
            &picker,
            |this, _, event: &ColorPickerEvent, cx| match event {
                ColorPickerEvent::Changed(color) => this.preview_picker_color(*color, cx),
                ColorPickerEvent::Commit => this.close_color_picker(true, cx),
                ColorPickerEvent::Cancel => this.close_color_picker(false, cx),
            },
        );
        picker.read(cx).focus_handle(cx).focus(window, cx);
        self.picker = Some(PickerSession {
            field,
            original: current,
            snapshot,
            changed: false,
            picker,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn preview_picker_color(&mut self, color: FantaColor, cx: &mut Context<Self>) {
        let Some(session) = self.picker.as_mut() else {
            return;
        };
        session.changed = true;
        let field = session.field.clone();
        let snapshot = session.snapshot.clone();
        self.preview_field_text(&field, &snapshot, &color.to_hex(), cx);
    }

    /// Close the picker popover. `commit` keeps the picked color by restoring
    /// the pre-open state and authoring one undoable operation; otherwise the
    /// preview is rolled back.
    fn close_color_picker(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(session) = self.picker.take() else {
            return;
        };
        if session.changed {
            self.restore_snapshot_preview(&session.snapshot, cx);
            if commit {
                let final_color = session.picker.read(cx).color();
                if final_color != session.original {
                    let field = session.field.clone();
                    let text = final_color.to_hex();
                    self.apply_document_ops(cx, |doc| field_operations(doc, &field, &text));
                }
            }
        }
        cx.notify();
    }

    /// Drop the picker popover without committing. `restore` rolls back any
    /// live preview (skip after a reload, where the snapshot is stale).
    fn abandon_color_picker(&mut self, restore: bool, cx: &mut Context<Self>) {
        if let Some(session) = self.picker.take()
            && session.changed
            && restore
        {
            self.restore_snapshot_preview(&session.snapshot, cx);
        }
    }

    // === Gradient editor ==================================================

    /// Open (or toggle closed) the gradient-editor popover for a gradient
    /// paint. `is_stroke` selects the paint list; `gradient` seeds the editor.
    /// While open, every change previews transiently; closing commits one op.
    fn toggle_gradient_editor(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        gradient: Gradient,
        cx: &mut Context<Self>,
    ) {
        let field = InspectorField::Gradient {
            id,
            index,
            is_stroke,
        };
        if self
            .gradient_editor
            .as_ref()
            .is_some_and(|session| session.field == field)
        {
            self.close_gradient_editor(true, cx);
            return;
        }
        self.close_color_picker(true, cx);
        self.close_gradient_editor(true, cx);
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        let editor = cx.new(|cx| GradientEditor::new(gradient.clone(), cx));
        let subscription =
            cx.subscribe(
                &editor,
                |this, _, event: &GradientEditorEvent, cx| match event {
                    GradientEditorEvent::Changed(gradient) => {
                        this.preview_gradient(gradient.clone(), cx)
                    }
                    GradientEditorEvent::Commit => this.close_gradient_editor(true, cx),
                    GradientEditorEvent::Cancel => this.close_gradient_editor(false, cx),
                },
            );
        self.gradient_editor = Some(GradientSession {
            field,
            original: gradient,
            snapshot,
            changed: false,
            editor,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn preview_gradient(&mut self, gradient: Gradient, cx: &mut Context<Self>) {
        let Some(session) = self.gradient_editor.as_mut() else {
            return;
        };
        session.changed = true;
        let InspectorField::Gradient {
            id,
            index,
            is_stroke,
        } = session.field
        else {
            return;
        };
        let snapshot = session.snapshot.clone();
        self.preview_gradient_paint(id, index, is_stroke, &snapshot, gradient, cx);
    }

    /// Preview a gradient as a transient (no-history) edit: restore the
    /// snapshot, then write the gradient straight into the fill/stroke slot.
    fn preview_gradient_paint(
        &self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        snapshot: &NodeSnapshot,
        gradient: Gradient,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let snapshot = snapshot.clone();
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                return;
            }
            let applied = item.with_document(cx, |document| {
                restore_snapshot(&mut document.doc, &snapshot);
                let operations = finite_transform_operations(replace_data_operation(
                    &document.doc,
                    id,
                    |data| set_paint_gradient(data, index, is_stroke, gradient.clone()),
                ));
                for operation in &operations {
                    apply_preview_operation(&mut document.doc, operation);
                }
                ((), DocChange::ContentPreview)
            });
            if applied.is_none() {
                log::debug!("dropping inspector gradient preview: the document is not ready");
            }
        });
    }

    /// Close the gradient editor. `commit` keeps the edited gradient by
    /// restoring the pre-open state and authoring one undoable op; otherwise
    /// the preview is rolled back.
    fn close_gradient_editor(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(session) = self.gradient_editor.take() else {
            return;
        };
        if session.changed {
            self.restore_snapshot_preview(&session.snapshot, cx);
            if commit {
                let final_gradient = session.editor.read(cx).gradient();
                if final_gradient != session.original
                    && let InspectorField::Gradient {
                        id,
                        index,
                        is_stroke,
                    } = session.field
                {
                    self.update_node_data(
                        id,
                        move |data| set_paint_gradient(data, index, is_stroke, final_gradient),
                        cx,
                    );
                }
            }
        }
        cx.notify();
    }

    /// Drop the gradient editor without committing. `restore` rolls back any
    /// live preview (skip after a reload, where the snapshot is stale).
    fn abandon_gradient_editor(&mut self, restore: bool, cx: &mut Context<Self>) {
        if let Some(session) = self.gradient_editor.take()
            && session.changed
            && restore
        {
            self.restore_snapshot_preview(&session.snapshot, cx);
        }
    }

    /// Change a paint's kind: Solid seeds a two-stop gradient (or flattens a
    /// gradient back to its representative solid color); the gradient kinds
    /// convert an existing gradient or seed one from the current solid. One
    /// undoable op.
    fn set_paint_kind(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        kind: PaintKind,
        cx: &mut Context<Self>,
    ) {
        self.close_gradient_editor(true, cx);
        self.close_color_picker(true, cx);
        self.update_node_data(
            id,
            move |data| convert_paint_kind(data, index, is_stroke, kind),
            cx,
        );
    }

    // === Snapshot =========================================================

    fn build_snapshot(&self, cx: &App) -> InspectorSnapshot {
        let Some(view) = self.active_view(cx) else {
            return InspectorSnapshot::Message(NO_DOCUMENT_MESSAGE.into());
        };
        let (item, selected_page_index) = {
            let view = view.read(cx);
            (view.item().clone(), view.selected_page_index())
        };
        let item = item.read(cx);
        if let Some(message) = item.document.loading_message() {
            return InspectorSnapshot::Message(message);
        }
        if let Some(error) = item.document.error() {
            return InspectorSnapshot::Message(
                format!("Could not open Figma file: {error:#}").into(),
            );
        }
        let Some(document) = item.document() else {
            return InspectorSnapshot::Message(NO_DOCUMENT_MESSAGE.into());
        };
        let editable = item.is_editable();
        let doc = &document.doc;
        let selection: Vec<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|id| doc.scene.contains(*id))
            .collect();
        // One O(library) scan per snapshot build — never per row. Both the
        // single-node "is this a component master" test and the multi-select
        // "how many masters are selected" count read it.
        let masters = master_roots(&doc.components);
        let body = match selection.as_slice() {
            [] => InspectorBody::Page(page_section(document, selected_page_index)),
            [id] => match node_section(doc, *id, &masters) {
                Some(node) => InspectorBody::Node(Box::new(node)),
                None => InspectorBody::Page(page_section(document, selected_page_index)),
            },
            ids => InspectorBody::Multi(multi_section(doc, ids, &masters)),
        };
        InspectorSnapshot::Ready {
            editable,
            selection_len: selection.len(),
            body,
        }
    }

    // === Rendering primitives =============================================

    /// A titled section header band: a muted caption at the left and an
    /// optional action (the Fill/Stroke/Effects "+" box) hugging the right
    /// inset, matching the original's header anatomy.
    fn render_section_header(title: &'static str, action: Option<AnyElement>) -> AnyElement {
        h_flex()
            .px_4()
            .h(px(SECTION_HEADER_H))
            .items_center()
            .justify_between()
            .child(
                Label::new(title)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .children(action)
            .into_any_element()
    }

    fn section_add_button(
        &self,
        id: &'static str,
        tooltip: &'static str,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        IconButton::new(id, IconName::Plus)
            .icon_size(IconSize::XSmall)
            .tooltip(Tooltip::text(tooltip))
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
            .into_any_element()
    }

    /// The muted fixed-width caption to the left of pills, sliders, and
    /// dropdowns.
    fn pill_label(text: &'static str) -> Div {
        div()
            .w(px(PILL_LABEL_W))
            .flex_none()
            .child(Label::new(text).size(LabelSize::XSmall).color(Color::Muted))
    }

    /// A 2-up numeric field box: the mini-label INSIDE the box at the left
    /// (the drag-to-scrub handle), the value (click-to-edit via the shared
    /// inline editor), and an optional dim unit suffix at the right.
    #[allow(clippy::too_many_arguments)]
    fn render_numeric_cell(
        &self,
        key: &'static str,
        ix: usize,
        label: Option<SharedString>,
        field: InspectorField,
        value: Option<f64>,
        suffix: Option<&'static str>,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let editing = self.editing_field.as_ref() == Some(&field);
        // The whole field box is the scrub handle (Figma/Blender behaviour):
        // press-drag anywhere on it to scrub, a plain click focuses the editor.
        let scrubbable = editable && value.is_some() && !editing;
        let mut cell = h_flex()
            .id((key, ix))
            .flex_1()
            .min_w_0()
            .h(px(FIELD_BOX_H))
            .px_1p5()
            .gap_1()
            .rounded_sm()
            .border_1()
            .bg(colors.editor_background)
            .border_color(if editing {
                colors.border_focused
            } else {
                colors.border_variant
            });
        if let Some(label_text) = label {
            cell = cell.child(
                div().flex_none().child(
                    Label::new(label_text)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            );
        }
        if editing {
            cell = cell.child(div().flex_1().min_w_0().child(self.field_editor.clone()));
        } else {
            let display: SharedString = match value {
                Some(value) => format_number(value).into(),
                None => MIXED_VALUE.into(),
            };
            cell = cell.child(
                div().flex_1().min_w_0().overflow_hidden().child(
                    Label::new(display)
                        .size(LabelSize::Small)
                        .color(if editable {
                            Color::Default
                        } else {
                            Color::Muted
                        })
                        .single_line(),
                ),
            );
            if let Some(suffix) = suffix {
                cell = cell.child(
                    Label::new(suffix)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                );
            }
        }
        if scrubbable {
            let scrub_field = field.clone();
            let edit_field = field;
            let start_value = value.unwrap_or(0.0);
            let initial = value.map(format_number).unwrap_or_default();
            cell = cell
                .debug_selector(|| format!("scrub-{key}-{ix}"))
                .cursor_ew_resize()
                .hover(|style| style.border_color(colors.border))
                .on_drag(PanelDrag, |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.begin_field_scrub(
                            scrub_field.clone(),
                            start_value,
                            event.position,
                            cx,
                        );
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.finish_scrub(cx)),
                )
                // A press that never turned into a drag is a plain click: focus
                // the editor for keyboard entry. gpui suppresses `on_click` when
                // a drag occurred, so a scrub won't also open the editor.
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_editing(edit_field.clone(), initial.clone(), window, cx);
                }));
        } else if editable && !editing {
            // Mixed / valueless but still editable: click to type a value.
            let edit_field = field;
            cell = cell
                .cursor_text()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_editing(edit_field.clone(), String::new(), window, cx);
                }));
        }
        cell.into_any_element()
    }

    /// A text field box (hex colors, font family, instance text props):
    /// click-to-edit, no scrub.
    #[allow(clippy::too_many_arguments)]
    fn render_text_cell(
        &self,
        key: &'static str,
        ix: usize,
        label: Option<SharedString>,
        field: InspectorField,
        display: SharedString,
        initial: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let editing = self.editing_field.as_ref() == Some(&field);
        let mut cell = h_flex()
            .flex_1()
            .min_w_0()
            .h(px(FIELD_BOX_H))
            .px_1p5()
            .gap_1()
            .rounded_md()
            .border_1()
            .bg(colors.editor_background)
            .border_color(if editing {
                colors.border_focused
            } else {
                colors.border_variant
            });
        if let Some(label_text) = label {
            cell = cell.child(
                Label::new(label_text)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        }
        if editing {
            cell = cell.child(div().flex_1().min_w_0().child(self.field_editor.clone()));
        } else {
            let editable = initial.is_some();
            let mut value_element = div()
                .id((key, ix))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(
                    Label::new(display)
                        .size(LabelSize::Small)
                        .color(if editable {
                            Color::Default
                        } else {
                            Color::Muted
                        })
                        .single_line(),
                );
            if let Some(initial) = initial {
                let edit_field = field;
                value_element = value_element.cursor_text().on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.start_editing(edit_field.clone(), initial.clone(), window, cx);
                    },
                ));
            }
            cell = cell.child(value_element);
        }
        cell.into_any_element()
    }

    /// A full-width click-to-cycle pill: value at the left, a chevron hinting
    /// the cycle at the right — the original's dropdown-look cycle control.
    fn render_pill(
        &self,
        id: impl Into<ElementId>,
        value: SharedString,
        tooltip: &'static str,
        editable: bool,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let mut pill = h_flex()
            .id(id)
            .flex_1()
            .min_w_0()
            .h(px(FIELD_BOX_H))
            .px_2()
            .gap_1()
            .justify_between()
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.editor_background)
            .child(
                Label::new(value)
                    .size(LabelSize::Small)
                    .color(if editable {
                        Color::Default
                    } else {
                        Color::Muted
                    })
                    .single_line(),
            )
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            );
        if editable {
            pill = pill
                .cursor_pointer()
                .hover(|style| style.bg(colors.element_hover))
                .tooltip(Tooltip::text(tooltip))
                .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)));
        }
        pill.into_any_element()
    }

    /// A labeled cycle-pill row: muted caption column + pill.
    #[allow(clippy::too_many_arguments)]
    fn render_pill_row(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        value: SharedString,
        tooltip: &'static str,
        editable: bool,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .px_4()
            .gap_2()
            .items_center()
            .child(Self::pill_label(label))
            .child(self.render_pill(id, value, tooltip, editable, on_click, cx))
            .into_any_element()
    }

    /// A labeled enum-choice row: a muted caption column beside a compact Zed
    /// [`DropdownMenu`]. The native equivalent of the old click-to-cycle pill.
    #[allow(clippy::too_many_arguments)]
    fn render_choice_row<T: Copy + PartialEq + 'static>(
        &self,
        element_id: &'static str,
        label: &'static str,
        aria_label: &'static str,
        id: NodeId,
        current: T,
        options: &'static [(T, &'static str)],
        apply: fn(&mut Self, NodeId, T, &mut Context<Self>),
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .px_4()
            .gap_2()
            .items_center()
            .child(Self::pill_label(label))
            .child(self.render_choice_dropdown(
                element_id, aria_label, id, current, options, apply, editable, window, cx,
            ))
            .into_any_element()
    }

    /// A labeled switch row (the original's toggle rows: Visible, Wrap, Clip
    /// content, Ignore auto layout, …).
    fn render_switch_row(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        editable: bool,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .px_4()
            .h(px(FIELD_BOX_H))
            .items_center()
            .justify_between()
            .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
            .child(
                Switch::new(id, ToggleState::from(on))
                    .disabled(!editable)
                    .on_click(cx.listener(move |this, _: &ToggleState, _, cx| on_click(this, cx))),
            )
            .into_any_element()
    }

    /// A full-width dropdown cell for a discrete node property. Falls back to
    /// a read-only pill when the document is not editable.
    #[allow(clippy::too_many_arguments)]
    fn render_choice_dropdown<T: Copy + PartialEq + 'static>(
        &self,
        element_id: &'static str,
        aria_label: &'static str,
        id: NodeId,
        current: T,
        options: &'static [(T, &'static str)],
        apply: fn(&mut Self, NodeId, T, &mut Context<Self>),
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label: SharedString = options
            .iter()
            .find(|(value, _)| *value == current)
            .map(|(_, label)| *label)
            .unwrap_or(MIXED_VALUE)
            .into();
        self.render_labeled_dropdown(
            element_id, aria_label, label, id, current, options, apply, editable, window, cx,
        )
    }

    /// [`Self::render_choice_dropdown`] with an explicit trigger label, for
    /// properties whose current value may sit off the option list (a font weight
    /// of 350 reads "Custom", not "–", and is never snapped onto a stop).
    #[allow(clippy::too_many_arguments)]
    fn render_labeled_dropdown<T: Copy + PartialEq + 'static>(
        &self,
        element_id: &'static str,
        aria_label: &'static str,
        label: SharedString,
        id: NodeId,
        current: T,
        options: &'static [(T, &'static str)],
        apply: fn(&mut Self, NodeId, T, &mut Context<Self>),
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !editable {
            let colors = cx.theme().colors();
            return h_flex()
                .flex_1()
                .min_w_0()
                .px_2()
                .h(px(FIELD_BOX_H))
                .rounded_md()
                .border_1()
                .bg(colors.editor_background)
                .border_color(colors.border_variant)
                .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
                .into_any_element();
        }
        let panel = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
            for (value, name) in options {
                let panel = panel.clone();
                let value = *value;
                menu.push_item(
                    ContextMenuEntry::new(*name)
                        .toggleable(IconPosition::End, value == current)
                        .handler(move |_window, cx| {
                            if let Err(error) =
                                panel.update(cx, |this, cx| apply(this, id, value, cx))
                            {
                                log::debug!(
                                    "dropping {aria_label} change for closed properties panel: {error:#}"
                                );
                            }
                        }),
                );
            }
            menu
        });
        div()
            .flex_1()
            .min_w_0()
            .child(
                DropdownMenu::new(element_id, label, menu)
                    .style(DropdownStyle::Outlined)
                    .trigger_size(ButtonSize::Compact)
                    .full_width(true)
                    .aria_label(aria_label),
            )
            .into_any_element()
    }

    /// A color swatch that opens the anchored color-picker popover. The open
    /// picker is anchored just below the swatch via `deferred(anchored())`.
    #[allow(clippy::too_many_arguments)]
    fn render_color_swatch(
        &self,
        key: &'static str,
        ix: usize,
        color: Option<FantaColor>,
        field: Option<InspectorField>,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let mut swatch = div()
            .id((key, ix))
            .relative()
            .size(px(18.))
            .flex_none()
            .rounded_sm()
            .border_1()
            .border_color(colors.border);
        if let Some(color) = color {
            swatch = swatch.bg(fanta_color_rgba(color));
        } else {
            swatch = swatch.bg(colors.element_background);
        }
        let Some(field) = field else {
            return swatch.into_any_element();
        };
        if editable && let Some(color) = color {
            let picker_field = field.clone();
            let press_field = field.clone();
            swatch = swatch
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, _| {
                        // The popover's mouse-down-out handler will close this
                        // swatch's picker during this same press; remember that
                        // so the release does not reopen it (toggle-closed).
                        this.swatch_press_dismissed = this
                            .picker
                            .as_ref()
                            .is_some_and(|session| session.field == press_field);
                    }),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if std::mem::take(&mut this.swatch_press_dismissed) {
                        return;
                    }
                    this.toggle_color_picker(picker_field.clone(), color, window, cx);
                }));
        }
        if let Some(session) = &self.picker
            && session.field == field
        {
            swatch = swatch.child(
                div().absolute().left_0().bottom_0().size_0().child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .snap_to_window_with_margin(px(8.))
                            .offset(point(px(0.), px(4.)))
                            .child(session.picker.clone()),
                    )
                    .with_priority(1),
                ),
            );
        }
        swatch.into_any_element()
    }

    /// A swatch previewing a gradient paint. Clicking it opens the gradient
    /// editor popover, anchored just below the swatch like the color picker.
    fn render_gradient_swatch(
        &self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        gradient: &Gradient,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let field = InspectorField::Gradient {
            id,
            index,
            is_stroke,
        };
        let key: ElementId = if is_stroke {
            ("fanta-stroke-gradient-swatch", index).into()
        } else {
            ("fanta-fill-gradient-swatch", index).into()
        };
        let mut swatch = div()
            .id(key)
            .debug_selector(|| {
                if is_stroke {
                    format!("fanta-stroke-gradient-swatch-{index}")
                } else {
                    format!("fanta-fill-gradient-swatch-{index}")
                }
            })
            .relative()
            .size(px(18.))
            .flex_none()
            .rounded_sm()
            .border_1()
            .border_color(colors.border)
            .overflow_hidden()
            .child(gradient_preview_strip(gradient).absolute().inset_0());
        if editable {
            let gradient = gradient.clone();
            let press_field = field.clone();
            swatch = swatch
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, _| {
                        this.swatch_press_dismissed = this
                            .gradient_editor
                            .as_ref()
                            .is_some_and(|session| session.field == press_field);
                    }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if std::mem::take(&mut this.swatch_press_dismissed) {
                        return;
                    }
                    this.toggle_gradient_editor(id, index, is_stroke, gradient.clone(), cx);
                }));
        }
        if let Some(session) = &self.gradient_editor
            && session.field == field
        {
            swatch = swatch.child(
                div().absolute().left_0().bottom_0().size_0().child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .snap_to_window_with_margin(px(8.))
                            .offset(point(px(0.), px(4.)))
                            .child(session.editor.clone()),
                    )
                    .with_priority(1),
                ),
            );
        }
        swatch.into_any_element()
    }

    /// The per-paint type selector: a compact dropdown cycling the paint
    /// between Solid and the four gradient kinds. Seeds / flattens gradients as
    /// needed via [`Self::set_paint_kind`].
    fn render_paint_type_selector(
        &self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        current: PaintKind,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let label: SharedString = paint_kind_label(current).into();
        if !editable {
            return h_flex()
                .w(px(84.))
                .flex_none()
                .px_2()
                .h(px(FIELD_BOX_H))
                .rounded_md()
                .border_1()
                .bg(colors.editor_background)
                .border_color(colors.border_variant)
                .child(
                    Label::new(label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element();
        }
        let element_id: ElementId = if is_stroke {
            ("fanta-stroke-type", index).into()
        } else {
            ("fanta-fill-type", index).into()
        };
        let panel = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
            for kind in PAINT_KINDS {
                let panel = panel.clone();
                menu.push_item(
                    ContextMenuEntry::new(paint_kind_label(kind))
                        .toggleable(IconPosition::End, kind == current)
                        .handler(move |_window, cx| {
                            if let Err(error) = panel.update(cx, |this, cx| {
                                this.set_paint_kind(id, index, is_stroke, kind, cx)
                            }) {
                                log::debug!(
                                    "dropping paint-type change for closed properties panel: {error:#}"
                                );
                            }
                        }),
                );
            }
            menu
        });
        div()
            .w(px(84.))
            .flex_none()
            .child(
                DropdownMenu::new(element_id, label, menu)
                    .style(DropdownStyle::Outlined)
                    .trigger_size(ButtonSize::Compact)
                    .full_width(true)
                    .aria_label("Paint type"),
            )
            .into_any_element()
    }

    /// The per-paint blend dropdown (Figma's paint-level `blendMode`). Offered
    /// only for gradient and image paints — a solid carries no blend of its own
    /// in this model.
    #[allow(clippy::too_many_arguments)]
    fn render_paint_blend_selector(
        &self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        current: BlendMode,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let label: SharedString = BLEND_MODES
            .iter()
            .find(|(mode, _)| *mode == current)
            .map(|(_, label)| *label)
            .unwrap_or(MIXED_VALUE)
            .into();
        if !editable {
            return h_flex()
                .flex_1()
                .min_w_0()
                .px_2()
                .h(px(FIELD_BOX_H))
                .rounded_md()
                .border_1()
                .bg(colors.editor_background)
                .border_color(colors.border_variant)
                .child(
                    Label::new(label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element();
        }
        let element_id: ElementId = if is_stroke {
            ("fanta-stroke-blend", index).into()
        } else {
            ("fanta-fill-blend", index).into()
        };
        let panel = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
            for (mode, name) in BLEND_MODES {
                let panel = panel.clone();
                menu.push_item(
                    ContextMenuEntry::new(name)
                        .toggleable(IconPosition::End, mode == current)
                        .handler(move |_window, cx| {
                            if let Err(error) = panel.update(cx, |this, cx| {
                                this.set_paint_blend(id, index, is_stroke, mode, cx)
                            }) {
                                log::debug!(
                                    "dropping paint-blend change for closed properties panel: {error:#}"
                                );
                            }
                        }),
                );
            }
            menu
        });
        div()
            .flex_1()
            .min_w_0()
            .child(
                DropdownMenu::new(element_id, label, menu)
                    .style(DropdownStyle::Outlined)
                    .trigger_size(ButtonSize::Compact)
                    .full_width(true)
                    .aria_label("Paint blend mode"),
            )
            .into_any_element()
    }

    /// A ghost "+ Add …" row, the original's add affordance under a paint /
    /// effect list.
    fn render_add_row(
        &self,
        id: &'static str,
        label: &'static str,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        h_flex()
            .px_4()
            .child(
                h_flex()
                    .id(id)
                    .flex_1()
                    .h(px(FIELD_BOX_H))
                    .gap_1()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border_variant)
                    .cursor_pointer()
                    .hover(|style| style.bg(colors.element_hover))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new(label)
                            .size(LabelSize::Small)
                            .color(Color::Accent),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx))),
            )
            .into_any_element()
    }

    /// The transparent overlay that records a slider track's bounds during
    /// paint, so track clicks map to fractions.
    fn track_bounds_probe(&self, track: SliderTrack, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        canvas(
            move |bounds, _, cx| {
                this.update(cx, |this, _| {
                    this.slider_tracks[track as usize] = Some(bounds)
                })
                .log_err();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    // === Sections =========================================================

    /// The type/name header band above the section stack.
    fn render_header(
        &self,
        type_name: SharedString,
        name: String,
        rename: Option<InspectorField>,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rename = if editable { rename } else { None };
        let editing = rename.is_some() && self.editing_field == rename;
        let mut header = v_flex().px_4().py_2().gap_0p5().child(
            Label::new(type_name)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        );
        if editing {
            header = header.child(div().w_full().child(self.field_editor.clone()));
        } else {
            let display_name: SharedString = if name.is_empty() {
                "Untitled".into()
            } else {
                name.clone().into()
            };
            let mut name_element = div()
                .id("fanta-node-name")
                .w_full()
                .child(Label::new(display_name).single_line());
            if let Some(field) = rename {
                name_element = name_element.cursor_pointer().on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.start_editing(field.clone(), name.clone(), window, cx);
                    },
                ));
            }
            header = header.child(name_element);
        }
        header.into_any_element()
    }

    /// The Align section: 6 edge-align buttons (enabled at 2+ selected) plus,
    /// at 3+ selected, the 2 distribute buttons — evenly spread across the
    /// panel like the original's alignment row.
    fn render_align_section(
        &self,
        selection_len: usize,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let align_enabled = editable && selection_len >= 2;
        let distribute_enabled = editable && selection_len >= 3;
        let mut row = h_flex().px_4().h(px(34.)).items_center().justify_between();
        for (index, (glyph, tooltip, command)) in ALIGN_BUTTONS.iter().enumerate() {
            row = row.child(self.render_align_button(
                ("fanta-align", index),
                *glyph,
                tooltip,
                *command,
                align_enabled,
                cx,
            ));
        }
        if selection_len >= 3 {
            row = row.child(Divider::vertical());
            for (index, (glyph, tooltip, command)) in DISTRIBUTE_BUTTONS.iter().enumerate() {
                row = row.child(self.render_align_button(
                    ("fanta-distribute", index),
                    *glyph,
                    tooltip,
                    *command,
                    distribute_enabled,
                    cx,
                ));
            }
        }
        v_flex()
            .py_1()
            .child(Self::render_section_header("Align", None))
            .child(row)
            .into_any_element()
    }

    fn render_align_button(
        &self,
        id: impl Into<ElementId>,
        glyph: AlignGlyph,
        tooltip: &'static str,
        command: AlignCommand,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let icon_color = if enabled {
            colors.text
        } else {
            colors.text_disabled
        };
        let mut button = div()
            .id(id)
            .w(px(26.))
            .h(px(26.))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .child(align_glyph(glyph, icon_color));
        if enabled {
            button = button
                .cursor_pointer()
                .hover(|style| style.bg(colors.element_hover))
                .tooltip(Tooltip::text(tooltip))
                .on_click(cx.listener(move |this, _, _, cx| this.apply_align(command, cx)));
        }
        button.into_any_element()
    }

    /// The Position section: X/Y (hidden while the node flows in a parent's
    /// auto layout), W/H, and the rotation cell.
    ///
    /// Diverges from the original Fanta, which showed a frame's W/H inside its
    /// Layout section instead: keeping W/H here for every node kind means one
    /// dimension editor, already exercised by the resize / rotation math.
    /// Corner radius + smoothing live in Appearance (old Fanta's `insp_appearance`).
    fn render_position_section(
        &self,
        node: &NodeSection,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = node.id;
        let in_flow = node
            .layout_child
            .as_ref()
            .is_some_and(|child| !child.absolute);
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Position", None));
        if !in_flow {
            section = section.child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(self.render_numeric_cell(
                        "fanta-x",
                        0,
                        Some("X".into()),
                        InspectorField::X(id),
                        Some(node.x),
                        None,
                        editable,
                        cx,
                    ))
                    .child(self.render_numeric_cell(
                        "fanta-y",
                        0,
                        Some("Y".into()),
                        InspectorField::Y(id),
                        Some(node.y),
                        None,
                        editable,
                        cx,
                    )),
            );
        }
        section = section.child(
            h_flex()
                .px_4()
                .gap_2()
                .child(self.render_numeric_cell(
                    "fanta-w",
                    0,
                    Some("W".into()),
                    InspectorField::Width(id),
                    Some(node.width),
                    None,
                    editable,
                    cx,
                ))
                .child(self.render_numeric_cell(
                    "fanta-h",
                    0,
                    Some("H".into()),
                    InspectorField::Height(id),
                    Some(node.height),
                    None,
                    editable,
                    cx,
                )),
        );

        section = section.child(
            h_flex()
                .px_4()
                .gap_2()
                .child(self.render_numeric_cell(
                    "fanta-rotation",
                    0,
                    Some("∠".into()),
                    InspectorField::Rotation(id),
                    Some(node.rotation_degrees),
                    Some("°"),
                    editable,
                    cx,
                ))
                .child(div().flex_1()),
        );
        section.into_any_element()
    }

    /// The "Auto layout child" section: how this node participates in its
    /// parent's auto layout. Only rendered when the parent is an auto-layout
    /// frame.
    fn render_layout_child_section(
        &self,
        id: NodeId,
        layout_child: &LayoutChildSnapshot,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fills_container = layout_child.fills_container;
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Auto layout child", None));
        if !layout_child.absolute {
            section = section.child(self.render_pill_row(
                "fanta-child-resize",
                "Resize",
                if fills_container {
                    "Fill container".into()
                } else {
                    "Fixed width".into()
                },
                "Toggle Fixed Width / Fill Container",
                editable,
                move |this, cx| {
                    this.update_layout_child(
                        id,
                        |child| child.grow = if fills_container { 0.0 } else { 1.0 },
                        cx,
                    );
                },
                cx,
            ));
        }
        section
            .child(self.render_switch_row(
                "fanta-child-absolute",
                "Ignore auto layout",
                layout_child.absolute,
                editable,
                move |this, cx| {
                    this.update_layout_child(id, |child| child.absolute = !child.absolute, cx);
                },
                cx,
            ))
            .into_any_element()
    }

    /// The per-corner radius pad: a uniform R cell with an expander that opens
    /// the four TL/TR/BR/BL cells. Lives inside Appearance, next to smoothing.
    fn render_corner_rows(
        &self,
        id: NodeId,
        corner_radius: &CornerRadiusValue,
        corner_smoothing: Option<f64>,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        let corners_expanded = self
            .corner_radii_expanded
            .unwrap_or(matches!(corner_radius, CornerRadiusValue::PerCorner(_)));
        let radius_value = match corner_radius {
            CornerRadiusValue::NotApplicable => None,
            CornerRadiusValue::Uniform(radius) => Some(Some(*radius)),
            // Mixed corners read blank, like every other mixed-value cell.
            CornerRadiusValue::PerCorner(_) => Some(None),
        };
        if let Some(radius_value) = radius_value {
            rows.push(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(self.render_numeric_cell(
                        "fanta-radius",
                        0,
                        Some("R".into()),
                        InspectorField::CornerRadius(id),
                        radius_value,
                        None,
                        editable,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(
                        IconButton::new("fanta-corner-expander", IconName::SquareDot)
                            .icon_size(IconSize::Small)
                            .toggle_state(corners_expanded)
                            .tooltip(Tooltip::text("Individual Corners"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.corner_radii_expanded = Some(!corners_expanded);
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        if radius_value.is_some() && corners_expanded {
            let radii = match corner_radius {
                CornerRadiusValue::PerCorner(radii) => *radii,
                CornerRadiusValue::Uniform(radius) => [*radius; 4],
                CornerRadiusValue::NotApplicable => [0.0; 4],
            };
            let corner_keys: [(&'static str, &'static str); 4] = [
                ("fanta-corner-tl", "TL"),
                ("fanta-corner-tr", "TR"),
                ("fanta-corner-br", "BR"),
                ("fanta-corner-bl", "BL"),
            ];
            let corner_cell = |this: &Self, corner: usize, cx: &mut Context<Self>| {
                let (key, label) = corner_keys[corner];
                this.render_numeric_cell(
                    key,
                    0,
                    Some(label.into()),
                    InspectorField::CornerRadiusCorner { id, corner },
                    radii.get(corner).copied(),
                    None,
                    editable,
                    cx,
                )
            };
            rows.push(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(corner_cell(self, 0, cx))
                    .child(corner_cell(self, 1, cx))
                    .into_any_element(),
            );
            rows.push(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(corner_cell(self, 3, cx))
                    .child(corner_cell(self, 2, cx))
                    .into_any_element(),
            );
        }
        if let Some(percent) = corner_smoothing {
            rows.push(self.render_slider_row(
                SliderTrack::CornerSmoothing,
                "fanta-smoothing-track",
                "Smoothing",
                InspectorField::CornerSmoothing(id),
                percent,
                editable,
                cx,
            ));
        }
        rows
    }

    /// The Layout section: clip content — which a plain frame carries with or
    /// without auto layout — followed by the auto-layout controls when the frame
    /// has an auto layout.
    fn render_layout_section(
        &self,
        id: NodeId,
        layout: &LayoutSnapshot,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let auto_layout_on = layout.auto_layout.is_some();
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Layout", None))
            .child(self.render_switch_row(
                "fanta-layout-clip",
                "Clip content",
                layout.clip,
                editable,
                move |this, cx| this.toggle_clip_content(id, cx),
                cx,
            ))
            .child(self.render_switch_row(
                "fanta-layout-auto",
                "Auto layout",
                auto_layout_on,
                editable,
                move |this, cx| this.toggle_auto_layout(id, !auto_layout_on, cx),
                cx,
            ));
        if let Some(auto_layout) = &layout.auto_layout {
            section = section.child(self.render_auto_layout_section(
                id,
                auto_layout,
                editable,
                window,
                cx,
            ));
        }
        section.into_any_element()
    }

    /// The Auto layout controls: direction pill, the interactive 3×3 alignment
    /// grid beside the align dropdowns, gap and padding pairs, per-axis
    /// sizing, wrap, and stacking — the original's full control set.
    fn render_auto_layout_section(
        &self,
        id: NodeId,
        layout: &AutoLayoutSnapshot,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Which underlying axis reads as "W" depends on the flow direction,
        // exactly like the original's Resize W / Resize H pills.
        let horizontal = matches!(layout.mode, LayoutMode::Horizontal);
        let (w_primary, w_sizing) = if horizontal {
            (true, layout.primary_sizing)
        } else {
            (false, layout.counter_sizing)
        };
        let (h_primary, h_sizing) = if horizontal {
            (false, layout.counter_sizing)
        } else {
            (true, layout.primary_sizing)
        };
        v_flex()
            .gap_2()
            .child(self.render_choice_row(
                "fanta-layout-direction",
                "Direction",
                "Layout direction",
                id,
                layout.mode,
                &LAYOUT_MODES,
                Self::set_layout_direction,
                editable,
                window,
                cx,
            ))
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_start()
                    .child(self.render_align_grid(id, layout, editable, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div().w(px(36.)).flex_none().child(
                                            Label::new("Align")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                    )
                                    .child(self.render_choice_dropdown(
                                        "fanta-layout-primary-align",
                                        "Primary axis alignment",
                                        id,
                                        layout.primary_align,
                                        &PRIMARY_ALIGNS,
                                        Self::set_primary_align,
                                        editable,
                                        window,
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div().w(px(36.)).flex_none().child(
                                            Label::new("Cross")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                    )
                                    .child(self.render_choice_dropdown(
                                        "fanta-layout-counter-align",
                                        "Counter axis alignment",
                                        id,
                                        layout.counter_align,
                                        &COUNTER_ALIGNS,
                                        Self::set_counter_align,
                                        editable,
                                        window,
                                        cx,
                                    )),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(self.render_numeric_cell(
                        "fanta-layout-gap-h",
                        0,
                        Some("Gap H".into()),
                        InspectorField::LayoutGapH(id),
                        Some(layout.gap_h),
                        None,
                        editable,
                        cx,
                    ))
                    .child(self.render_numeric_cell(
                        "fanta-layout-gap-v",
                        0,
                        Some("Gap V".into()),
                        InspectorField::LayoutGapV(id),
                        Some(layout.gap_v),
                        None,
                        editable,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(self.render_numeric_cell(
                        "fanta-layout-pad-v",
                        0,
                        Some("Pad V".into()),
                        InspectorField::LayoutPadV(id),
                        layout.pad_v,
                        None,
                        editable,
                        cx,
                    ))
                    .child(self.render_numeric_cell(
                        "fanta-layout-pad-h",
                        0,
                        Some("Pad H".into()),
                        InspectorField::LayoutPadH(id),
                        layout.pad_h,
                        None,
                        editable,
                        cx,
                    )),
            )
            .child(self.render_choice_row(
                "fanta-layout-resize-w",
                "Resize W",
                "Resize width",
                id,
                w_sizing,
                &AXIS_SIZINGS,
                if w_primary {
                    Self::set_primary_axis_sizing
                } else {
                    Self::set_counter_axis_sizing
                },
                editable,
                window,
                cx,
            ))
            .child(self.render_choice_row(
                "fanta-layout-resize-h",
                "Resize H",
                "Resize height",
                id,
                h_sizing,
                &AXIS_SIZINGS,
                if h_primary {
                    Self::set_primary_axis_sizing
                } else {
                    Self::set_counter_axis_sizing
                },
                editable,
                window,
                cx,
            ))
            .child(self.render_switch_row(
                "fanta-layout-wrap",
                "Wrap",
                layout.wrap,
                editable,
                move |this, cx| this.toggle_layout_wrap(id, cx),
                cx,
            ))
            .child(self.render_pill_row(
                "fanta-layout-stacking",
                "Stacking",
                stacking_label(layout.reverse_z).into(),
                "Toggle Stacking Order",
                editable,
                move |this, cx| this.toggle_layout_stacking(id, cx),
                cx,
            ))
            .into_any_element()
    }

    /// The Figma-style 3×3 alignment grid: nine dotted cells; the active cell
    /// carries an accent wash and a white dot; clicking a cell sets primary +
    /// counter alignment together.
    fn render_align_grid(
        &self,
        id: NodeId,
        layout: &AutoLayoutSnapshot,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let active = align_grid_active_cell(layout);
        let mut grid = v_flex()
            .w(px(ALIGN_GRID_SIZE))
            .h(px(ALIGN_GRID_SIZE))
            .flex_none()
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.editor_background)
            .p_0p5();
        for row in 0..3u8 {
            let mut row_element = h_flex().flex_1();
            for col in 0..3u8 {
                let is_active = active == Some((col, row));
                let dot_color = if is_active {
                    gpui::white()
                } else {
                    colors.text_muted
                };
                let mut cell = div()
                    .id(("fanta-align-grid", (row * 3 + col) as usize))
                    .flex_1()
                    .m(px(1.))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(4.)).rounded_full().bg(dot_color));
                if is_active {
                    cell = cell.bg(colors.text_accent);
                }
                if editable {
                    let hover_bg = colors.element_hover;
                    cell = cell
                        .cursor_pointer()
                        .when(!is_active, |cell| {
                            cell.hover(move |style| style.bg(hover_bg))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_align_cell(id, col, row, cx);
                        }));
                }
                row_element = row_element.child(cell);
            }
            grid = grid.child(row_element);
        }
        grid.into_any_element()
    }

    /// The Typography section: family, weight + size, LH/LS, the independent
    /// italic / underline / strikethrough toggles, the 7-cell align strip
    /// (4 horizontal incl. justify + 3 vertical), and the resize-mode pill.
    fn render_typography_section(
        &self,
        id: NodeId,
        typography: &TypographySnapshot,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Typography", None))
            .child(h_flex().px_4().child(self.render_text_cell(
                "fanta-font-family",
                0,
                Some("Aa".into()),
                InspectorField::FontFamily(id),
                typography.font_family.clone().into(),
                editable.then(|| typography.font_family.clone()),
                cx,
            )))
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(self.render_labeled_dropdown(
                        "fanta-font-weight",
                        "Font weight",
                        font_weight_label(typography.weight).into(),
                        id,
                        typography.weight,
                        &FONT_WEIGHTS,
                        Self::set_font_weight,
                        editable,
                        window,
                        cx,
                    ))
                    .child(div().w(px(84.)).flex_none().child(self.render_numeric_cell(
                        "fanta-font-size",
                        0,
                        Some("S".into()),
                        InspectorField::FontSize(id),
                        Some(typography.size_px),
                        None,
                        editable,
                        cx,
                    ))),
            )
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(self.render_numeric_cell(
                        "fanta-line-height",
                        0,
                        Some("LH".into()),
                        InspectorField::LineHeight(id),
                        Some(typography.line_height),
                        None,
                        editable,
                        cx,
                    ))
                    .child(self.render_numeric_cell(
                        "fanta-letter-spacing",
                        0,
                        Some("LS".into()),
                        InspectorField::LetterSpacing(id),
                        Some(typography.letter_spacing),
                        None,
                        editable,
                        cx,
                    )),
            )
            .child(self.render_text_decoration_row(id, typography, editable, cx))
            .child(self.render_text_align_row(id, typography, editable, cx))
            .child(self.render_pill_row(
                "fanta-text-resize",
                "Resize",
                text_resize_label(typography.auto_resize).into(),
                "Cycle Text Resize Mode",
                editable,
                move |this, cx| this.cycle_text_resize(id, cx),
                cx,
            ))
            .into_any_element()
    }

    /// The three independent decoration toggles. Unlike the align strip these
    /// are not mutually exclusive: a run can be italic AND underlined.
    fn render_text_decoration_row(
        &self,
        id: NodeId,
        typography: &TypographySnapshot,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let decorations = [
            (TextDecorationGlyph::Italic, typography.italic, "Italic"),
            (
                TextDecorationGlyph::Underline,
                typography.underline,
                "Underline",
            ),
            (
                TextDecorationGlyph::Strikethrough,
                typography.strikethrough,
                "Strikethrough",
            ),
        ];
        let mut strip = h_flex()
            .flex_1()
            .h(px(FIELD_BOX_H))
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.editor_background)
            .overflow_hidden();
        for (index, (glyph, active, tooltip)) in decorations.into_iter().enumerate() {
            let glyph_color = if active {
                gpui::white()
            } else {
                colors.text_muted
            };
            let mut cell = div()
                .id(("fanta-text-decoration", index))
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(text_decoration_glyph(glyph, glyph_color));
            if active {
                cell = cell.bg(colors.text_accent);
            }
            if editable {
                let hover_bg = colors.element_hover;
                cell = cell
                    .cursor_pointer()
                    .when(!active, |cell| cell.hover(move |style| style.bg(hover_bg)))
                    .tooltip(Tooltip::text(tooltip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_text_decoration(id, glyph, cx);
                    }));
            }
            strip = strip.child(cell);
        }
        h_flex()
            .px_4()
            .gap_2()
            .items_center()
            .child(Self::pill_label("Style"))
            .child(strip)
            .child(div().flex_1())
            .into_any_element()
    }

    /// The 7-cell text align strip: 4 horizontal cells — left / center / right /
    /// justify — (the active one gets a solid accent pill + white glyph) then 3
    /// vertical cells (the active one reads via an accent glyph).
    fn render_text_align_row(
        &self,
        id: NodeId,
        typography: &TypographySnapshot,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let h_active = match typography.align {
            TextAlign::Left => 0usize,
            TextAlign::Center => 1,
            TextAlign::Right => 2,
            TextAlign::Justify => 3,
        };
        let v_active = match typography.vertical_align {
            TextVAlign::Top => 4usize,
            TextVAlign::Center => 5,
            TextVAlign::Bottom => 6,
        };
        let glyphs = [
            TextAlignGlyph::Left,
            TextAlignGlyph::CenterH,
            TextAlignGlyph::Right,
            TextAlignGlyph::Justify,
            TextAlignGlyph::Top,
            TextAlignGlyph::CenterV,
            TextAlignGlyph::Bottom,
        ];
        let mut strip = h_flex()
            .flex_1()
            .h(px(FIELD_BOX_H))
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.editor_background)
            .overflow_hidden();
        for (index, glyph) in glyphs.into_iter().enumerate() {
            let h_on = h_active == index;
            let v_on = v_active == index;
            let glyph_color = if h_on {
                gpui::white()
            } else if v_on {
                colors.text_accent
            } else {
                colors.text_muted
            };
            let mut cell = div()
                .id(("fanta-text-align", index))
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(text_align_glyph(glyph, glyph_color));
            if h_on {
                cell = cell.bg(colors.text_accent);
            }
            if editable {
                let hover_bg = colors.element_hover;
                cell = cell
                    .cursor_pointer()
                    .when(!h_on, |cell| cell.hover(move |style| style.bg(hover_bg)))
                    .on_click(cx.listener(move |this, _, _, cx| match index {
                        0 => this.set_text_align(id, TextAlign::Left, cx),
                        1 => this.set_text_align(id, TextAlign::Center, cx),
                        2 => this.set_text_align(id, TextAlign::Right, cx),
                        3 => this.set_text_align(id, TextAlign::Justify, cx),
                        4 => this.set_text_vertical_align(id, TextVAlign::Top, cx),
                        5 => this.set_text_vertical_align(id, TextVAlign::Center, cx),
                        _ => this.set_text_vertical_align(id, TextVAlign::Bottom, cx),
                    }));
            }
            strip = strip.child(cell);
        }
        h_flex()
            .px_4()
            .gap_2()
            .items_center()
            .child(Self::pill_label("Align"))
            .child(strip)
            .into_any_element()
    }

    fn render_image_section(
        &self,
        id: NodeId,
        fit: ImageFitMode,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Image", None))
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(Self::pill_label("Fit"))
                    .child(self.render_choice_dropdown(
                        "fanta-image-fit",
                        "Image fit mode",
                        id,
                        fit,
                        &IMAGE_FIT_MODES,
                        Self::set_image_fit,
                        editable,
                        window,
                        cx,
                    )),
            )
            .into_any_element()
    }

    /// The component-master section: identity, the variant-set summary, and the
    /// exposed-properties schema.
    ///
    /// Read-only. Editing the schema (`SetComponentProps` + a per-kind default
    /// editor + a descendant binding picker) or the variant set
    /// (`SetComponentSet` + axis/value chips) is a large sub-editor apiece;
    /// both are deferred.
    fn render_master_section(&self, master: &MasterSection) -> AnyElement {
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Component", None))
            .child(
                h_flex()
                    .px_4()
                    .gap_1p5()
                    .items_center()
                    .child(
                        Icon::new(IconName::Box)
                            .size(IconSize::Small)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new(master.name.clone())
                            .size(LabelSize::Small)
                            .single_line(),
                    ),
            );
        if let Some(variant_set) = &master.variant_set {
            section = section.child(
                h_flex().px_4().child(
                    Label::new(format!(
                        "Variant of \u{201c}{}\u{201d}{}",
                        variant_set.set_name,
                        if variant_set.is_default_variant {
                            " \u{b7} default"
                        } else {
                            ""
                        }
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .single_line(),
                ),
            );
            for axis in &variant_set.axes {
                section = section.child(
                    h_flex()
                        .px_4()
                        .gap_2()
                        .items_center()
                        .child(
                            div().w(px(PILL_LABEL_W)).flex_none().child(
                                Label::new(axis.name.clone())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .single_line(),
                            ),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                Label::new(axis.selected.clone())
                                    .size(LabelSize::Small)
                                    .single_line(),
                            ),
                        )
                        .child(
                            Label::new(axis.values.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .single_line(),
                        ),
                );
            }
        }
        section = section.child(
            h_flex().px_4().pt_1().child(
                Label::new("Properties")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            ),
        );
        if master.props.is_empty() {
            section = section.child(
                h_flex().px_4().child(
                    Label::new("No properties")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        }
        for prop in &master.props {
            section = section.child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(
                        div().w(px(PILL_LABEL_W)).flex_none().child(
                            Label::new(prop.name.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .single_line(),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(prop.kind.clone())
                                .size(LabelSize::Small)
                                .single_line(),
                        ),
                    )
                    .child(
                        Label::new(prop.default.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .single_line(),
                    ),
            );
        }
        section.into_any_element()
    }

    /// The component instance Properties section: an identity row that doubles
    /// as "go to main component", one cycle pill per variant axis, editors for
    /// bool / text / number / color props, and the detach action.
    fn render_instance_section(
        &self,
        id: NodeId,
        component: &InstanceSection,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut identity = h_flex()
            .id("fanta-main-component")
            .px_4()
            .gap_1p5()
            .items_center()
            .child(
                Icon::new(IconName::Box)
                    .size(IconSize::Small)
                    .color(Color::Accent),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(component.component_name.clone())
                        .size(LabelSize::Small)
                        .single_line(),
                ),
            );
        if let Some(main_root) = component.main_root {
            identity = identity
                .cursor_pointer()
                .tooltip(Tooltip::text("Go to Main Component"))
                .child(
                    Icon::new(IconName::ArrowUpRight)
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.focus_main_component(main_root, cx);
                }));
        }
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Properties", None))
            .child(identity);
        for (index, variant) in component.variants.iter().enumerate() {
            let axis = variant.axis.clone();
            section = section.child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(
                        div().w(px(PILL_LABEL_W)).flex_none().child(
                            Label::new(variant.axis.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .single_line(),
                        ),
                    )
                    .child(self.render_pill(
                        ("fanta-variant-axis", index),
                        variant.value.clone(),
                        "Cycle Variant",
                        editable,
                        move |this, cx| this.cycle_variant_axis(id, axis.clone(), cx),
                        cx,
                    )),
            );
        }
        for (index, prop) in component.props.iter().enumerate() {
            match &prop.value {
                PropValueSnapshot::Bool(value) => {
                    let value = *value;
                    let prop_id = prop.id;
                    section = section.child(
                        h_flex()
                            .px_4()
                            .h(px(FIELD_BOX_H))
                            .items_center()
                            .justify_between()
                            .child(
                                Label::new(prop.name.clone())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .single_line(),
                            )
                            .child(
                                Switch::new(("fanta-prop-bool", index), ToggleState::from(value))
                                    .disabled(!editable)
                                    .on_click(cx.listener(move |this, _: &ToggleState, _, cx| {
                                        this.set_instance_prop(
                                            id,
                                            prop_id,
                                            Some(VarValue::Boolean { value: !value }),
                                            cx,
                                        );
                                    })),
                            ),
                    );
                }
                PropValueSnapshot::Text(value) => {
                    let prop_id = prop.id;
                    section = section.child(
                        h_flex()
                            .px_4()
                            .gap_2()
                            .items_center()
                            .child(
                                div().w(px(PILL_LABEL_W)).flex_none().child(
                                    Label::new(prop.name.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .single_line(),
                                ),
                            )
                            .child(self.render_text_cell(
                                "fanta-prop-text",
                                index,
                                None,
                                InspectorField::InstanceTextProp { id, prop: prop_id },
                                value.clone().into(),
                                editable.then(|| value.clone()),
                                cx,
                            )),
                    );
                }
                PropValueSnapshot::Number(value) => {
                    let prop_id = prop.id;
                    section = section.child(
                        h_flex()
                            .px_4()
                            .gap_2()
                            .items_center()
                            .child(
                                div().w(px(PILL_LABEL_W)).flex_none().child(
                                    Label::new(prop.name.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .single_line(),
                                ),
                            )
                            .child(self.render_numeric_cell(
                                "fanta-prop-number",
                                index,
                                None,
                                InspectorField::InstanceNumberProp { id, prop: prop_id },
                                Some(*value),
                                None,
                                editable,
                                cx,
                            )),
                    );
                }
                PropValueSnapshot::Color(color) => {
                    let prop_id = prop.id;
                    let field = InspectorField::InstanceColorProp { id, prop: prop_id };
                    section = section.child(
                        h_flex()
                            .px_4()
                            .gap_1p5()
                            .items_center()
                            .child(
                                div().w(px(PILL_LABEL_W)).flex_none().child(
                                    Label::new(prop.name.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .single_line(),
                                ),
                            )
                            .child(self.render_color_swatch(
                                "fanta-prop-color-swatch",
                                index,
                                Some(*color),
                                Some(field.clone()),
                                editable,
                                cx,
                            ))
                            .child(div().flex_1().min_w_0().child(self.render_text_cell(
                                "fanta-prop-color",
                                index,
                                None,
                                field,
                                color.to_hex().trim_start_matches('#').to_string().into(),
                                editable.then(|| color.to_hex()),
                                cx,
                            ))),
                    );
                }
                PropValueSnapshot::Display(value) => {
                    section = section.child(
                        h_flex()
                            .px_4()
                            .gap_2()
                            .items_center()
                            .child(
                                div().w(px(PILL_LABEL_W)).flex_none().child(
                                    Label::new(prop.name.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .single_line(),
                                ),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    Label::new(value.clone())
                                        .size(LabelSize::Small)
                                        .color(Color::Muted)
                                        .single_line(),
                                ),
                            ),
                    );
                }
            }
        }
        if editable {
            section = section.child(
                h_flex().px_4().pt_1().child(
                    Button::new("fanta-detach-instance", "Detach instance")
                        .start_icon(Icon::new(IconName::Scissors).size(IconSize::XSmall))
                        .size(ButtonSize::Compact)
                        .label_size(LabelSize::Small)
                        .full_width()
                        .tooltip(Tooltip::text("Replace the instance with editable copies"))
                        .on_click(cx.listener(move |this, _, _, cx| this.detach_instance(id, cx))),
                ),
            );
        }
        section.into_any_element()
    }

    /// The Appearance section: the opacity slider (draggable track + editable
    /// % readout), corner radius with its per-corner pad and the corner
    /// smoothing slider (corner-capable nodes only), the blend dropdown, and
    /// the visible / locked switches.
    fn render_appearance_section(
        &self,
        node: &NodeSection,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = node.id;
        v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Appearance", None))
            .child(self.render_slider_row(
                SliderTrack::Opacity,
                "fanta-opacity-track",
                "Opacity",
                InspectorField::Opacity(id),
                node.opacity_percent,
                editable,
                cx,
            ))
            .children(self.render_corner_rows(
                id,
                &node.corner_radius,
                node.corner_smoothing,
                editable,
                cx,
            ))
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(Self::pill_label("Blend"))
                    .child(self.render_choice_dropdown(
                        "fanta-blend-mode",
                        "Blend mode",
                        id,
                        node.blend_mode,
                        &BLEND_MODES,
                        Self::set_blend_mode,
                        editable,
                        window,
                        cx,
                    )),
            )
            .child(self.render_switch_row(
                "fanta-toggle-visible",
                "Visible",
                node.visible,
                editable,
                move |this, cx| this.toggle_flag(id, NodeFlags::HIDDEN, cx),
                cx,
            ))
            .child(self.render_switch_row(
                "fanta-toggle-locked",
                "Locked",
                node.locked,
                editable,
                move |this, cx| this.toggle_flag(id, NodeFlags::LOCKED, cx),
                cx,
            ))
            .into_any_element()
    }

    /// A 0–100% slider row: label, a real draggable track (filled bar + knob),
    /// and the focusable % readout. Backs both Opacity and corner Smoothing.
    #[allow(clippy::too_many_arguments)]
    fn render_slider_row(
        &self,
        track_kind: SliderTrack,
        element_id: &'static str,
        label: &'static str,
        field: InspectorField,
        percent: f64,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let fraction = (percent / 100.0).clamp(0.0, 1.0) as f32;
        let thumb_color = if editable {
            colors.text
        } else {
            colors.text_disabled
        };
        let mut track = div()
            .id(element_id)
            .relative()
            .flex_1()
            .h(px(FIELD_BOX_H))
            .child(self.track_bounds_probe(track_kind, cx))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(13.))
                    .h(px(3.))
                    .rounded_full()
                    .bg(colors.element_background),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(13.))
                    .h(px(3.))
                    .w(relative(fraction))
                    .rounded_full()
                    .bg(if editable {
                        colors.text_accent
                    } else {
                        colors.element_active
                    }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(9.))
                    .left(relative(fraction))
                    .ml(px(-5.5))
                    .size(px(11.))
                    .rounded_full()
                    .bg(colors.elevated_surface_background)
                    .border_2()
                    .border_color(thumb_color),
            );
        if editable {
            let scrub_field = field.clone();
            track = track
                .cursor_pointer()
                .on_drag(PanelDrag, |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.begin_track_scrub(
                            track_kind,
                            scrub_field.clone(),
                            percent,
                            event.position,
                            cx,
                        );
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.finish_scrub(cx)),
                );
        }
        h_flex()
            .px_4()
            .gap_2()
            .items_center()
            .child(Self::pill_label(label))
            .child(track)
            .child(div().w(px(60.)).flex_none().child(self.render_numeric_cell(
                element_id,
                0,
                None,
                field,
                Some(percent),
                Some("%"),
                editable,
                cx,
            )))
            .into_any_element()
    }

    /// A Fill / Stroke section: header with a right-aligned "+" add box, then
    /// one row per paint — swatch (opens the color picker) → hex → opacity %
    /// or width token → eye → × — plus the stroke Position pill and the ghost
    /// add row, matching the original's list anatomy.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn render_paint_section(
        &self,
        id: NodeId,
        title: &'static str,
        entries: &[PaintSnapshot],
        stroke_align: Option<StrokeAlign>,
        is_stroke: bool,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let add_button = editable.then(|| {
            self.section_add_button(
                if is_stroke {
                    "fanta-add-stroke"
                } else {
                    "fanta-add-fill"
                },
                if is_stroke { "Add Stroke" } else { "Add Fill" },
                move |this, cx| this.add_paint(id, is_stroke, cx),
                cx,
            )
        });
        let mut section = v_flex()
            .py_1()
            .gap_1()
            .child(Self::render_section_header(title, add_button));
        for (index, entry) in entries.iter().enumerate() {
            let color_field = if is_stroke {
                InspectorField::StrokeColor { id, index }
            } else {
                InspectorField::FillColor { id, index }
            };
            let visible = entry.visible;
            let mut row = h_flex().px_4().h(px(LIST_ROW_H)).gap_1p5().items_center();
            if let Some(gradient) = &entry.gradient {
                row = row
                    .child(
                        self.render_gradient_swatch(id, index, is_stroke, gradient, editable, cx),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(entry.label.clone())
                                .size(LabelSize::Small)
                                .single_line(),
                        ),
                    );
            } else {
                row = row
                    .child(self.render_color_swatch(
                        if is_stroke {
                            "fanta-stroke-swatch"
                        } else {
                            "fanta-fill-swatch"
                        },
                        index,
                        entry.color,
                        Some(color_field.clone()),
                        editable,
                        cx,
                    ))
                    .child(div().flex_1().min_w_0().child(self.render_text_cell(
                        if is_stroke {
                            "fanta-stroke-hex"
                        } else {
                            "fanta-fill-hex"
                        },
                        index,
                        None,
                        color_field,
                        entry.label.clone(),
                        (editable && entry.color.is_some()).then(|| entry.label.to_string()),
                        cx,
                    )));
            }
            if let Some(kind) = entry.kind {
                row =
                    row.child(self.render_paint_type_selector(
                        id, index, is_stroke, kind, editable, window, cx,
                    ));
            }
            if let Some(stroke_width) = entry.stroke_width {
                row = row.child(div().w(px(56.)).flex_none().child(self.render_numeric_cell(
                    "fanta-stroke-width",
                    index,
                    Some("W".into()),
                    InspectorField::StrokeWidth { id, index },
                    Some(stroke_width),
                    None,
                    editable,
                    cx,
                )));
            } else if let Some(opacity_percent) = entry.opacity_percent {
                row = row.child(div().w(px(58.)).flex_none().child(self.render_numeric_cell(
                    "fanta-paint-opacity",
                    index,
                    None,
                    InspectorField::PaintOpacity {
                        id,
                        index,
                        is_stroke,
                    },
                    Some(opacity_percent),
                    Some("%"),
                    editable,
                    cx,
                )));
            }
            if editable {
                let eye_id: ElementId = if is_stroke {
                    ("fanta-stroke-eye", index).into()
                } else {
                    ("fanta-fill-eye", index).into()
                };
                row = row.child(
                    IconButton::new(
                        eye_id,
                        if visible {
                            IconName::Eye
                        } else {
                            IconName::EyeOff
                        },
                    )
                    .icon_size(IconSize::XSmall)
                    .tooltip(Tooltip::text("Toggle Visibility"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_paint_visibility(id, index, is_stroke, cx);
                    })),
                );
                let remove_id: ElementId = if is_stroke {
                    ("fanta-stroke-remove", index).into()
                } else {
                    ("fanta-fill-remove", index).into()
                };
                row = row.child(
                    IconButton::new(remove_id, IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text(if is_stroke {
                            "Remove Stroke"
                        } else {
                            "Remove Fill"
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_paint(id, index, is_stroke, cx);
                        })),
                );
            }
            section = section.child(row);
            // The per-paint blend gets its own line: the swatch row is already
            // at its width budget at the panel's 260px minimum.
            if let Some(blend) = entry.blend {
                section = section.child(
                    h_flex()
                        .px_4()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(18.)).flex_none())
                        .child(self.render_paint_blend_selector(
                            id, index, is_stroke, blend, editable, window, cx,
                        )),
                );
            }
        }
        if is_stroke
            && !entries.is_empty()
            && let Some(align) = stroke_align
        {
            section = section.child(self.render_choice_row(
                "fanta-stroke-align",
                "Position",
                "Stroke position",
                id,
                align,
                &STROKE_ALIGNS,
                Self::set_stroke_align,
                editable,
                window,
                cx,
            ));
        }
        if editable {
            section = section.child(self.render_add_row(
                if is_stroke {
                    "fanta-add-stroke-row"
                } else {
                    "fanta-add-fill-row"
                },
                if is_stroke { "Add stroke" } else { "Add fill" },
                move |this, cx| this.add_paint(id, is_stroke, cx),
                cx,
            ));
        } else if entries.is_empty() {
            section = section.child(
                h_flex().px_4().child(
                    Label::new("None")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        }
        section.into_any_element()
    }

    /// A Text node's Fill section: one glyph-color row (swatch + hex), not a
    /// paint stack — the model stores the color on `TextStyle`, and the hex
    /// carries alpha (`#RRGGBBAA`) so transparency is editable here too.
    fn render_text_fill_section(
        &self,
        id: NodeId,
        color: FantaColor,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = InspectorField::TextColor(id);
        v_flex()
            .py_1()
            .gap_1()
            .child(Self::render_section_header("Fill", None))
            .child(
                h_flex()
                    .px_4()
                    .h(px(LIST_ROW_H))
                    .gap_1p5()
                    .items_center()
                    .child(self.render_color_swatch(
                        "fanta-text-fill-swatch",
                        0,
                        Some(color),
                        Some(field.clone()),
                        editable,
                        cx,
                    ))
                    .child(div().flex_1().min_w_0().child(self.render_text_cell(
                        "fanta-text-fill",
                        0,
                        None,
                        field,
                        color.to_hex().trim_start_matches('#').to_string().into(),
                        editable.then(|| color.to_hex()),
                        cx,
                    ))),
            )
            .into_any_element()
    }

    /// The Effects "+" menu: Drop shadow / Layer blur / Background blur, the
    /// same three the original offers.
    fn render_effects_add_menu(&self, id: NodeId, cx: &mut Context<Self>) -> AnyElement {
        let panel = cx.weak_entity();
        PopoverMenu::new("fanta-add-effect-menu")
            .anchor(Anchor::TopRight)
            .trigger(
                IconButton::new("fanta-add-effect", IconName::Plus)
                    .icon_size(IconSize::XSmall)
                    .tooltip(Tooltip::text("Add Effect")),
            )
            .menu(move |window, cx| {
                let panel = panel.clone();
                Some(ContextMenu::build(
                    window,
                    cx,
                    move |mut menu, _window, _cx| {
                        let entries: [(&'static str, Option<BlurKind>); 3] = [
                            ("Drop shadow", None),
                            ("Layer blur", Some(BlurKind::Layer)),
                            ("Background blur", Some(BlurKind::Background)),
                        ];
                        for (label, blur_kind) in entries {
                            let panel = panel.clone();
                            menu
                                .push_item(ContextMenuEntry::new(label).handler(move |_window, cx| {
                                if let Err(error) = panel.update(cx, |this, cx| match blur_kind {
                                    Some(kind) => this.add_blur(id, kind, cx),
                                    None => this.add_effect(id, cx),
                                }) {
                                    log::debug!(
                                        "dropping add-effect for closed properties panel: {error:#}"
                                    );
                                }
                            }));
                        }
                        menu
                    },
                ))
            })
            .into_any_element()
    }

    /// The Effects section: header "+" adds a drop shadow, a layer blur, or a
    /// background blur. Each shadow is an editable block — kind pill + remove ×,
    /// X/Y and Blur/Spread 2-ups, and a color row whose swatch opens the picker.
    /// Each blur is a kind pill + radius cell + remove ×.
    fn render_effects_section(
        &self,
        id: NodeId,
        effects: &[EffectSnapshot],
        blurs: &[BlurSnapshot],
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let add_button = editable.then(|| self.render_effects_add_menu(id, cx));
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Effects", add_button));
        for (index, effect) in effects.iter().enumerate() {
            let kind_label: SharedString = match effect.kind {
                ShadowKind::Drop => "Drop shadow".into(),
                ShadowKind::Inner => "Inner shadow".into(),
            };
            let mut kind_row = h_flex()
                .px_4()
                .gap_2()
                .items_center()
                .child(self.render_pill(
                    ("fanta-effect-kind", index),
                    kind_label,
                    "Toggle Drop / Inner Shadow",
                    editable,
                    move |this, cx| this.toggle_effect_kind(id, index, cx),
                    cx,
                ));
            if editable {
                kind_row = kind_row.child(
                    IconButton::new(("fanta-effect-remove", index), IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text("Remove Effect"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_effect(id, index, cx);
                        })),
                );
            }
            section = section
                .child(kind_row)
                .child(
                    h_flex()
                        .px_4()
                        .gap_2()
                        .child(self.render_numeric_cell(
                            "fanta-effect-x",
                            index,
                            Some("X".into()),
                            InspectorField::EffectOffsetX { id, index },
                            Some(effect.offset[0]),
                            None,
                            editable,
                            cx,
                        ))
                        .child(self.render_numeric_cell(
                            "fanta-effect-y",
                            index,
                            Some("Y".into()),
                            InspectorField::EffectOffsetY { id, index },
                            Some(effect.offset[1]),
                            None,
                            editable,
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .px_4()
                        .gap_2()
                        .child(self.render_numeric_cell(
                            "fanta-effect-blur",
                            index,
                            Some("B".into()),
                            InspectorField::EffectBlur { id, index },
                            Some(effect.blur),
                            None,
                            editable,
                            cx,
                        ))
                        .child(self.render_numeric_cell(
                            "fanta-effect-spread",
                            index,
                            Some("S".into()),
                            InspectorField::EffectSpread { id, index },
                            Some(effect.spread),
                            None,
                            editable,
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .px_4()
                        .gap_1p5()
                        .items_center()
                        .child(self.render_color_swatch(
                            "fanta-effect-swatch",
                            index,
                            Some(effect.color),
                            Some(InspectorField::EffectColor { id, index }),
                            editable,
                            cx,
                        ))
                        .child(
                            div().flex_1().min_w_0().child(
                                self.render_text_cell(
                                    "fanta-effect-color",
                                    index,
                                    None,
                                    InspectorField::EffectColor { id, index },
                                    effect
                                        .color
                                        .to_hex()
                                        .trim_start_matches('#')
                                        .to_string()
                                        .into(),
                                    editable.then(|| effect.color.to_hex()),
                                    cx,
                                ),
                            ),
                        ),
                );
        }
        for (index, blur) in blurs.iter().enumerate() {
            let kind = blur.kind;
            let mut row = h_flex()
                .px_4()
                .gap_2()
                .items_center()
                .child(self.render_pill(
                    ("fanta-blur-kind", index),
                    blur_kind_label(kind).into(),
                    "Toggle Layer / Background Blur",
                    editable,
                    move |this, cx| {
                        let next = match kind {
                            BlurKind::Layer => BlurKind::Background,
                            BlurKind::Background => BlurKind::Layer,
                        };
                        this.set_blur_kind(id, index, next, cx);
                    },
                    cx,
                ))
                .child(div().w(px(72.)).flex_none().child(self.render_numeric_cell(
                    "fanta-blur-radius",
                    index,
                    Some("R".into()),
                    InspectorField::BlurRadius { id, index },
                    Some(blur.radius),
                    None,
                    editable,
                    cx,
                )));
            if editable {
                row = row.child(
                    IconButton::new(("fanta-blur-remove", index), IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text("Remove Blur"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_blur(id, index, cx);
                        })),
                );
            }
            section = section.child(row);
        }
        if effects.is_empty() && blurs.is_empty() {
            if editable {
                section = section.child(self.render_add_row(
                    "fanta-add-effect-row",
                    "Add effect",
                    move |this, cx| this.add_effect(id, cx),
                    cx,
                ));
            } else {
                section = section.child(
                    h_flex().px_4().child(
                        Label::new("None")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                );
            }
        }
        section.into_any_element()
    }

    fn render_interactions_section(&self, reactions: &[SharedString]) -> AnyElement {
        let mut section = v_flex()
            .py_1()
            .gap_1()
            .child(Self::render_section_header("Interactions", None));
        for summary in reactions {
            section = section.child(
                h_flex().px_4().child(
                    Label::new(summary.clone())
                        .size(LabelSize::Small)
                        .single_line(),
                ),
            );
        }
        section.into_any_element()
    }

    fn render_bindings_section(&self, bindings: &[BindingSnapshot]) -> AnyElement {
        let mut section = v_flex()
            .py_1()
            .gap_1()
            .child(Self::render_section_header("Bindings", None));
        for binding in bindings {
            section = section.child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .justify_between()
                    .child(
                        Label::new(binding.property.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .single_line(),
                    )
                    .child(
                        Label::new(binding.variable.clone())
                            .size(LabelSize::Small)
                            .single_line(),
                    ),
            );
        }
        section.into_any_element()
    }

    /// The Export section: format row + Export button, and the preview band —
    /// a labeled panel with a thumbnail surface carrying the node's type
    /// glyph, matching the original's export block.
    fn render_export_section(
        &self,
        can_export: bool,
        preview_icon: Option<IconName>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Export", None))
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .h(px(FIELD_BOX_H))
                            .px_2()
                            .gap_1()
                            .rounded_md()
                            .border_1()
                            .border_color(colors.border_variant)
                            .bg(colors.editor_background)
                            .child(Label::new("PNG").size(LabelSize::Small))
                            .child(Label::new("2x").size(LabelSize::XSmall).color(Color::Muted)),
                    )
                    .child(
                        Button::new("fanta-export-png", "Export")
                            .size(ButtonSize::Compact)
                            .label_size(LabelSize::Small)
                            .disabled(!can_export)
                            .tooltip(Tooltip::text(if can_export {
                                "Export to <project>/exports"
                            } else {
                                "Create a Fanta project to export"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.export_png(cx))),
                    ),
            )
            .child(
                h_flex().px_4().child(
                    div()
                        .relative()
                        .flex_1()
                        .h(px(EXPORT_PREVIEW_H))
                        .rounded_lg()
                        .border_1()
                        .border_color(colors.border_variant)
                        .bg(colors.editor_background)
                        .child(
                            div().absolute().top_2().left_3().child(
                                Label::new("Preview")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                        )
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .pt_3()
                                .child(
                                    div()
                                        .w(px(96.))
                                        .h(px(46.))
                                        .rounded_md()
                                        .border_1()
                                        .border_color(colors.border_variant)
                                        .bg(colors.surface_background)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::new(preview_icon.unwrap_or(IconName::Image))
                                                .size(IconSize::Small)
                                                .color(Color::Muted),
                                        ),
                                ),
                        ),
                ),
            )
            .into_any_element()
    }

    fn render_page_properties(
        &self,
        page: &PageSection,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Page", None));
        if let (Some(id), Some(background)) = (page.id, &page.background) {
            let (color, label, initial) = match background {
                PageBackgroundValue::None => {
                    (None, SharedString::from("None"), editable.then(String::new))
                }
                PageBackgroundValue::Solid(color) => (
                    Some(*color),
                    SharedString::from(color.to_hex().trim_start_matches('#').to_string()),
                    editable.then(|| color.to_hex()),
                ),
                PageBackgroundValue::Other(label) => (None, label.clone(), None),
            };
            section = section.child(
                h_flex()
                    .px_4()
                    .gap_1p5()
                    .items_center()
                    .child(self.render_color_swatch(
                        "fanta-page-background-swatch",
                        0,
                        color,
                        Some(InspectorField::PageBackground(id)),
                        editable,
                        cx,
                    ))
                    .child(div().flex_1().min_w_0().child(self.render_text_cell(
                        "fanta-page-background",
                        0,
                        Some("BG".into()),
                        InspectorField::PageBackground(id),
                        label,
                        initial,
                        cx,
                    ))),
            );
        } else {
            section = section.child(
                h_flex().px_4().child(
                    Label::new("No page properties available")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        }
        section.into_any_element()
    }

    fn render_multi_position_section(
        &self,
        multi: &MultiSection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cell = |this: &Self,
                    key: &'static str,
                    label: &'static str,
                    field: InspectorField,
                    value: Option<f64>,
                    cx: &mut Context<Self>| {
            this.render_numeric_cell(key, 1, Some(label.into()), field, value, None, false, cx)
        };
        // The field id is a placeholder: multi-select cells are read-only.
        let id = multi.first_id;
        v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Position", None))
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(cell(
                        self,
                        "fanta-x",
                        "X",
                        InspectorField::X(id),
                        multi.x,
                        cx,
                    ))
                    .child(cell(
                        self,
                        "fanta-y",
                        "Y",
                        InspectorField::Y(id),
                        multi.y,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(cell(
                        self,
                        "fanta-w",
                        "W",
                        InspectorField::Width(id),
                        multi.width,
                        cx,
                    ))
                    .child(cell(
                        self,
                        "fanta-h",
                        "H",
                        InspectorField::Height(id),
                        multi.height,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(cell(
                        self,
                        "fanta-rotation",
                        "∠",
                        InspectorField::Rotation(id),
                        multi.rotation_degrees,
                        cx,
                    ))
                    .child(div().flex_1()),
            )
            .into_any_element()
    }

    /// The Selection colors section: one row per distinct solid color used
    /// anywhere in the selection — swatch, editable hex, usage count. Committing
    /// a hex replaces that color across every selected node in one undo step.
    fn render_selection_colors_section(
        &self,
        colors: &[SelectionColorSnapshot],
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut section = v_flex()
            .py_1()
            .gap_1()
            .child(Self::render_section_header("Selection colors", None));
        for (index, entry) in colors.iter().enumerate() {
            let field = InspectorField::SelectionColor { from: entry.color };
            let hex: SharedString = entry
                .color
                .to_hex()
                .trim_start_matches('#')
                .to_string()
                .into();
            section = section.child(
                h_flex()
                    .px_4()
                    .h(px(LIST_ROW_H))
                    .gap_1p5()
                    .items_center()
                    // The swatch is a preview only: a live color-picker preview
                    // would need a snapshot of every selected node, and the hex
                    // field already commits the replacement in one step.
                    .child(self.render_color_swatch(
                        "fanta-selection-color-swatch",
                        index,
                        Some(entry.color),
                        None,
                        false,
                        cx,
                    ))
                    .child(div().flex_1().min_w_0().child(self.render_text_cell(
                        "fanta-selection-color",
                        index,
                        None,
                        field,
                        hex,
                        editable.then(|| entry.color.to_hex()),
                        cx,
                    )))
                    .child(
                        Label::new(format!("{}", entry.uses))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }
        if colors.is_empty() {
            section = section.child(
                h_flex().px_4().child(
                    Label::new("No solid colors")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        }
        section.into_any_element()
    }

    /// "Combine N as variants": merge the selected component masters into one
    /// variant set so their instances can switch between them.
    fn render_combine_variants_section(
        &self,
        master_count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Variants", None))
            .child(
                h_flex().px_4().child(
                    Button::new(
                        "fanta-combine-variants",
                        format!("Combine {master_count} as variants"),
                    )
                    .size(ButtonSize::Compact)
                    .label_size(LabelSize::Small)
                    .full_width()
                    .tooltip(Tooltip::text("Merge the selected components into one set"))
                    .on_click(cx.listener(|this, _, _, cx| this.combine_as_variants(cx))),
                ),
            )
            .into_any_element()
    }
}

impl Render for FantaPropertiesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_started = std::time::Instant::now();
        let snapshot = self.build_snapshot(cx);
        crate::report_slow("properties panel snapshot", render_started);

        // A scrub whose drag ended outside the panel gets no drop event; the
        // drag's end still forces a redraw, so commit it from here.
        if self.scrub.as_ref().is_some_and(|scrub| scrub.moved) && !cx.has_active_drag() {
            cx.defer_in(window, |this, _, cx| this.finish_scrub(cx));
        }

        let root = v_flex()
            .key_context("FantaPropertiesPanel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_drag_move(cx.listener(Self::handle_scrub_move))
            .on_drop(cx.listener(|this, _: &PanelDrag, _, cx| this.finish_scrub(cx)))
            .size_full()
            .bg(cx.theme().colors().panel_background);
        match snapshot {
            InspectorSnapshot::Message(message) => root.child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .child(Label::new(message).color(Color::Muted)),
            ),
            InspectorSnapshot::Ready {
                editable,
                selection_len,
                body,
            } => {
                let mut content = v_flex()
                    .id("fanta-properties-content")
                    .flex_1()
                    .min_h_0()
                    .track_scroll(&self.content_scroll)
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .pb_4();
                // Sections after the header are divider-separated; collect them
                // so the per-kind matrix below reads as a list of section rows
                // rather than an interleaved `.child(Divider)` chain.
                let mut sections: Vec<AnyElement> = Vec::new();
                match body {
                    InspectorBody::Page(page) => {
                        content = content.child(self.render_header(
                            "Page".into(),
                            page.name.clone(),
                            page.id.map(InspectorField::Name),
                            editable,
                            cx,
                        ));
                        sections.push(self.render_align_section(selection_len, editable, cx));
                        sections.push(self.render_page_properties(&page, editable, cx));
                        sections.push(self.render_export_section(editable, None, cx));
                    }
                    InspectorBody::Node(node) => {
                        use NodeKind::*;
                        content = content.child(self.render_header(
                            node.type_name.clone(),
                            node.name.clone(),
                            Some(InspectorField::Name(node.id)),
                            editable,
                            cx,
                        ));
                        let id = node.id;
                        let kind = node.kind;

                        // ---- The per-node-kind section matrix ----------------
                        // Section order is fixed. `Y` = shown, `-` = hidden.
                        //
                        //                    Frame Group Shape Text Image Inst Comp Other
                        //  Align               Y     Y     Y     Y    Y     Y    Y    Y
                        //  Position (X/Y/W/H)  Y     Y     Y     Y    Y     Y    Y    Y
                        //  Component master    -     -     -     -    -     -    Y    -
                        //  Instance info       -     -     -     -    -     Y    -    -
                        //  Typography          -     -     -     Y    -     -    -    -
                        //  Image (fit only)    -     -     -     -    Y     -    -    -
                        //  Layout              Y     Y     -     -    -     -    Y¹   -
                        //  Auto layout child  parent is an auto-layout frame    -    "
                        //  Appearance          Y     Y     Y     Y    Y     Y    Y    Y
                        //   + radius/smoothing Y     Y     Y     -    -     -    Y    -
                        //  Fill                Y(bg) Y(bg) Y     Y²   -     -³   Y(bg) -
                        //  Stroke              Y⁴    Y⁴    Y     -    -     -    Y⁴   -
                        //  Effects (+ blur)    Y     Y     Y     Y    Y     Y    Y    Y
                        //  Interactions        Y     Y     Y     Y    Y     Y    Y    Y
                        //  Export              Y     Y     Y     Y    Y     Y    Y    Y
                        //
                        // 1. Only when the master's root is a `Group`.
                        // 2. A single glyph-color row, not a paint stack.
                        // 3. Instance fills are reached through exposed Color
                        //    props, never a raw paint stack.
                        // 4. Broader than the original, which serves strokes to
                        //    vectors only — group/frame strokes are real here.
                        sections.push(self.render_align_section(selection_len, editable, cx));
                        sections.push(self.render_position_section(&node, editable, cx));

                        // Component master identity + variant set + schema.
                        if let Some(master) = &node.master {
                            sections.push(self.render_master_section(master));
                        }
                        // Instance info: master, variants, props, detach.
                        if let Some(instance) = &node.instance {
                            sections.push(self.render_instance_section(id, instance, editable, cx));
                        }
                        if kind == Text
                            && let Some(typography) = &node.typography
                        {
                            sections.push(
                                self.render_typography_section(
                                    id, typography, editable, window, cx,
                                ),
                            );
                        }
                        if kind == Image
                            && let Some(image_fit) = node.image_fit
                        {
                            sections.push(
                                self.render_image_section(id, image_fit, editable, window, cx),
                            );
                        }
                        // Layout serves the group-backed kinds — a component
                        // master rooted at a frame gets it too.
                        if matches!(kind, Frame | Group | Component)
                            && let Some(layout) = &node.layout
                        {
                            sections
                                .push(self.render_layout_section(id, layout, editable, window, cx));
                        }
                        // A master is never a child of an auto-layout frame in
                        // the scene sense the inspector cares about.
                        if kind != Component
                            && let Some(layout_child) = &node.layout_child
                        {
                            sections.push(self.render_layout_child_section(
                                id,
                                layout_child,
                                editable,
                                cx,
                            ));
                        }
                        sections.push(self.render_appearance_section(&node, editable, window, cx));
                        if let Some(fills) = &node.fills {
                            sections.push(self.render_paint_section(
                                id, "Fill", fills, None, false, editable, window, cx,
                            ));
                        } else if kind == Text
                            && let Some(typography) = &node.typography
                        {
                            // Text has no paint stack: its Fill is the glyph color.
                            sections.push(self.render_text_fill_section(
                                id,
                                typography.color,
                                editable,
                                cx,
                            ));
                        }
                        if let Some(strokes) = &node.strokes {
                            sections.push(self.render_paint_section(
                                id,
                                "Stroke",
                                strokes,
                                node.stroke_align,
                                true,
                                editable,
                                window,
                                cx,
                            ));
                        }
                        sections.push(self.render_effects_section(
                            id,
                            &node.effects,
                            &node.blurs,
                            editable,
                            cx,
                        ));
                        if !node.reactions.is_empty() {
                            sections.push(self.render_interactions_section(&node.reactions));
                        }
                        if !node.bindings.is_empty() {
                            sections.push(self.render_bindings_section(&node.bindings));
                        }
                        sections.push(self.render_export_section(
                            editable,
                            Some(node.type_icon),
                            cx,
                        ));
                    }
                    InspectorBody::Multi(multi) => {
                        content = content.child(self.render_header(
                            "Selection".into(),
                            format!("{} selected", multi.count),
                            None,
                            editable,
                            cx,
                        ));
                        sections.push(self.render_align_section(selection_len, editable, cx));
                        sections.push(self.render_multi_position_section(&multi, cx));
                        sections.push(self.render_selection_colors_section(
                            &multi.colors,
                            editable,
                            cx,
                        ));
                        if editable && multi.master_count >= 2 {
                            sections
                                .push(self.render_combine_variants_section(multi.master_count, cx));
                        }
                    }
                }
                for section in sections {
                    content = content.child(Divider::horizontal()).child(section);
                }
                root.child(content)
            }
        }
    }
}

// =============================================================================
// Snapshot construction
// =============================================================================

fn page_section(document: &FigDocument, selected_page_index: Option<usize>) -> PageSection {
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
            Some(Fill::Solid { color }) => PageBackgroundValue::Solid(*color),
            Some(Fill::Gradient { .. }) => PageBackgroundValue::Other("Gradient".into()),
            Some(Fill::Image { .. }) => PageBackgroundValue::Other("Image".into()),
        }),
        _ => None,
    });
    PageSection {
        id: root,
        name,
        background,
    }
}

/// Every component master's root node → its component id. One pass over the
/// library per snapshot build; the alternative (asking "is this node a master"
/// per row) would be an O(library) scan per row.
fn master_roots(components: &ComponentLibrary) -> HashMap<NodeId, ComponentId> {
    components
        .defs
        .values()
        .map(|def| (def.root, def.id))
        .collect()
}

/// Classify the node for the section matrix. A component master overrides the
/// underlying data variant (old Fanta's `is_component_master`); everything else
/// reads off `NodeData`, with `Group` splitting on whether it paints a surface.
fn classify_node_kind(node: &CanvasNode, is_master: bool) -> NodeKind {
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

fn node_section(
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
        opacity_percent: f64::from(node.opacity) * 100.0,
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

fn layout_snapshot(node: &CanvasNode) -> Option<LayoutSnapshot> {
    let NodeData::Group(group) = &node.data else {
        return None;
    };
    Some(LayoutSnapshot {
        clip: group.clip_size.is_some(),
        auto_layout: auto_layout_snapshot(node),
    })
}

fn auto_layout_snapshot(node: &CanvasNode) -> Option<AutoLayoutSnapshot> {
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
fn align_grid_active_cell(layout: &AutoLayoutSnapshot) -> Option<(u8, u8)> {
    let primary_cell = match layout.primary_align {
        PrimaryAlign::Start => 0u8,
        PrimaryAlign::Center => 1,
        PrimaryAlign::End => 2,
        PrimaryAlign::SpaceBetween => return None,
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

fn layout_child_snapshot(doc: &Doc, node: &CanvasNode) -> Option<LayoutChildSnapshot> {
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
fn resolved_instance_def<'a>(
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
fn variant_selection(
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

fn instance_section(doc: &Doc, node: &CanvasNode) -> Option<InstanceSection> {
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
    Some(InstanceSection {
        component_name,
        // The master root must still exist in the scene to be navigable.
        main_root: doc.scene.contains(def.root).then_some(def.root),
        variants,
        props,
    })
}

/// Pick the editor a prop's current value gets. Number and Color are editable
/// (`SetInstanceProp(Float)` / `SetInstanceProp(Color)`); an instance-swap, a
/// text style or a variable alias has no inspector editor and reads out.
fn prop_value_snapshot(kind: &ComponentPropKind, value: &VarValue) -> PropValueSnapshot {
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

fn master_section(doc: &Doc, component: ComponentId) -> Option<MasterSection> {
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

fn prop_kind_label(kind: &ComponentPropKind) -> String {
    match kind {
        ComponentPropKind::Bool => "Boolean".to_string(),
        ComponentPropKind::Text => "Text".to_string(),
        ComponentPropKind::Number => "Number".to_string(),
        ComponentPropKind::Color => "Color".to_string(),
        ComponentPropKind::InstanceSwap => "Instance swap".to_string(),
        ComponentPropKind::Variant { axis } => format!("Variant \u{b7} {axis}"),
    }
}

fn prop_default_label(value: &VarValue) -> String {
    match value {
        VarValue::Boolean { value } => if *value { "On" } else { "Off" }.to_string(),
        VarValue::Float { value } => format_number(*value),
        VarValue::String { value } => value.clone(),
        VarValue::Color { value } => value.to_hex(),
        VarValue::TextStyle { .. } => "Text style".to_string(),
        VarValue::Alias { .. } => "Variable".to_string(),
    }
}

fn reaction_summary(reaction: &Reaction) -> SharedString {
    let trigger = match &reaction.trigger {
        Trigger::Click => "On click".to_string(),
        Trigger::Drag => "On drag".to_string(),
        Trigger::Hover => "On hover".to_string(),
        Trigger::AfterDelay { delay_ms } => format!("After {delay_ms} ms"),
        Trigger::Key { keys } => format!("On key {}", keys.join(", ")),
    };
    let action = match &reaction.action {
        Action::Navigate { .. } => "navigate",
        Action::Back => "go back",
        Action::Close => "close",
        Action::OpenOverlay { .. } => "open overlay",
        Action::ScrollTo { .. } => "scroll to",
        Action::SetVariable { .. } => "set variable",
    };
    format!("{trigger} → {action}").into()
}

fn binding_snapshots(doc: &Doc, node: &CanvasNode) -> Vec<BindingSnapshot> {
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

fn bound_prop_label(prop: &BoundProp) -> String {
    match prop {
        BoundProp::FillColor { index } => format!("Fill {} color", index + 1),
        BoundProp::StrokeColor { index } => format!("Stroke {} color", index + 1),
        BoundProp::StrokeWidth { index } => format!("Stroke {} width", index + 1),
        BoundProp::CornerRadius => "Corner radius".to_string(),
        BoundProp::Opacity => "Opacity".to_string(),
        BoundProp::Visible => "Visibility".to_string(),
        BoundProp::TextContent => "Text".to_string(),
        BoundProp::ClipWidth => "Width".to_string(),
        BoundProp::ClipHeight => "Height".to_string(),
    }
}

fn multi_section(
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
fn node_solid_colors(node: &CanvasNode, out: &mut Vec<FantaColor>) {
    let mut push_fill = |fill: &Fill| {
        if let Fill::Solid { color } = fill {
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
fn group_selection_colors(colors: Vec<FantaColor>) -> Vec<SelectionColorSnapshot> {
    let mut grouped: Vec<SelectionColorSnapshot> = Vec::new();
    for color in colors {
        match grouped.iter_mut().find(|entry| entry.color == color) {
            Some(entry) => entry.uses += 1,
            None => grouped.push(SelectionColorSnapshot { color, uses: 1 }),
        }
    }
    grouped
}

fn node_type_name(node: &CanvasNode, kind: NodeKind) -> &'static str {
    match kind {
        // Only these two override the data variant's own name. Everything else
        // already reads correctly from it — including `Vector` → "Shape", the
        // label the original inspector uses for every vector subtype.
        NodeKind::Component => "Component",
        NodeKind::Frame => "Frame",
        _ => node.data.default_name(),
    }
}

fn node_type_icon(node: &CanvasNode, kind: NodeKind) -> IconName {
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

fn corner_radius_value(node: &CanvasNode) -> CornerRadiusValue {
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

fn corner_smoothing_value(node: &CanvasNode) -> Option<f64> {
    let smoothing = match &node.data {
        NodeData::Vector(vector) => vector.corner_smoothing,
        NodeData::Group(group) => group.corner_smoothing,
        _ => return None,
    };
    Some(f64::from(smoothing) * 100.0)
}

fn paint_snapshot(fill: &Fill, stroke_width: Option<f64>) -> PaintSnapshot {
    let (color, label, gradient, kind, opacity_percent, blend) = match fill {
        Fill::Solid { color } => (
            Some(*color),
            SharedString::from(color.to_hex().trim_start_matches('#').to_string()),
            None,
            Some(PaintKind::Solid),
            Some(alpha_to_percent(color.a)),
            None,
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

fn alpha_to_percent(alpha: u8) -> f64 {
    f64::from(alpha) / 255.0 * 100.0
}

/// Whether a paint contributes any coverage. This is what the per-paint eye
/// reflects, since the model carries no per-paint visible flag.
fn paint_is_visible(fill: &Fill) -> bool {
    match fill {
        Fill::Solid { color } => color.a != 0,
        Fill::Gradient { gradient, .. } => crate::color_picker::gradient_stops(gradient)
            .iter()
            .any(|stop| stop.color.a != 0),
        Fill::Image { opacity, .. } => *opacity > 0.0,
    }
}

/// Snapshot a paint's alpha so the eye can restore it on show.
fn paint_alpha(fill: &Fill) -> HiddenPaintAlpha {
    match fill {
        Fill::Solid { color } => HiddenPaintAlpha::Solid(color.a),
        Fill::Gradient { gradient, .. } => HiddenPaintAlpha::Gradient(
            crate::color_picker::gradient_stops(gradient)
                .iter()
                .map(|stop| stop.color.a)
                .collect(),
        ),
        Fill::Image { opacity, .. } => HiddenPaintAlpha::Image(*opacity),
    }
}

fn paint_alpha_is_visible(alpha: &HiddenPaintAlpha) -> bool {
    match alpha {
        HiddenPaintAlpha::Solid(a) => *a != 0,
        HiddenPaintAlpha::Gradient(stops) => stops.iter().any(|a| *a != 0),
        HiddenPaintAlpha::Image(opacity) => *opacity > 0.0,
    }
}

/// The fully-transparent counterpart of `alpha`, shaped for the same paint kind.
fn zeroed_paint_alpha(fill: &Fill) -> HiddenPaintAlpha {
    match paint_alpha(fill) {
        HiddenPaintAlpha::Solid(_) => HiddenPaintAlpha::Solid(0),
        HiddenPaintAlpha::Gradient(stops) => HiddenPaintAlpha::Gradient(vec![0; stops.len()]),
        HiddenPaintAlpha::Image(_) => HiddenPaintAlpha::Image(0.0),
    }
}

/// The fully-opaque counterpart, used when a paint is shown with no remembered
/// alpha (a doc that loaded already-hidden, or a panel rebuilt since the hide).
fn opaque_paint_alpha(alpha: &HiddenPaintAlpha) -> HiddenPaintAlpha {
    match alpha {
        HiddenPaintAlpha::Solid(_) => HiddenPaintAlpha::Solid(255),
        HiddenPaintAlpha::Gradient(stops) => HiddenPaintAlpha::Gradient(vec![255; stops.len()]),
        HiddenPaintAlpha::Image(_) => HiddenPaintAlpha::Image(1.0),
    }
}

/// Write a snapshotted alpha back onto a paint. A stop-count mismatch (the
/// gradient gained or lost stops while hidden) leaves the extra stops alone.
fn set_paint_alpha(fill: &mut Fill, alpha: &HiddenPaintAlpha) {
    match (fill, alpha) {
        (Fill::Solid { color }, HiddenPaintAlpha::Solid(a)) => color.a = *a,
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

fn node_fills(node: &CanvasNode) -> Option<Vec<PaintSnapshot>> {
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

fn node_strokes(node: &CanvasNode) -> Option<Vec<PaintSnapshot>> {
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

fn node_stroke_align(node: &CanvasNode) -> Option<StrokeAlign> {
    let strokes = match &node.data {
        NodeData::Vector(vector) => &vector.strokes,
        NodeData::Group(group) => &group.strokes,
        _ => return None,
    };
    strokes.first().map(|stroke| stroke.align)
}

fn typography_snapshot(node: &CanvasNode) -> Option<TypographySnapshot> {
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
    })
}

// =============================================================================
// Operations
// =============================================================================

/// Advance one variant axis of an instance to its next value by swapping the
/// instance to the sibling member that carries it. Values are tried in cycle
/// order until one names an existing member, so sparse variant grids still
/// advance instead of dead-ending.
fn variant_cycle_operations(doc: &Doc, id: NodeId, axis: &str) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return Vec::new();
    };
    let components = &doc.components;
    let Some(def) = resolved_instance_def(components, instance) else {
        return Vec::new();
    };
    let Some(membership) = &def.variant_of else {
        return Vec::new();
    };
    let Some(set) = components.sets.get(&membership.set) else {
        return Vec::new();
    };
    let Some(axis_def) = set.axes.iter().find(|candidate| candidate.name == axis) else {
        return Vec::new();
    };
    if axis_def.values.is_empty() {
        return Vec::new();
    }
    let current = membership
        .axis_values
        .get(axis)
        .cloned()
        .unwrap_or_default();
    let current_index = axis_def
        .values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    for step in 1..=axis_def.values.len() {
        let index = (current_index + step) % axis_def.values.len();
        let Some(candidate_value) = axis_def.values.get(index) else {
            continue;
        };
        let mut target_values = membership.axis_values.clone();
        target_values.insert(axis.to_string(), candidate_value.clone());
        let target = set.members.iter().copied().find(|member| {
            components
                .def(*member)
                .and_then(|member_def| member_def.variant_of.as_ref())
                .is_some_and(|member_membership| member_membership.axis_values == target_values)
        });
        if let Some(target) = target {
            if target == instance.component {
                return Vec::new();
            }
            return vec![Operation::SwapInstance {
                id,
                old: instance.component,
                new: target,
            }];
        }
    }
    Vec::new()
}

fn field_operations(doc: &Doc, field: &InspectorField, text: &str) -> Vec<Operation> {
    let scene = &doc.scene;
    match field {
        InspectorField::Name(id) => {
            let Some(node) = scene.get(*id) else {
                return Vec::new();
            };
            if text.is_empty() || node.name == text {
                return Vec::new();
            }
            vec![Operation::SetName {
                id: *id,
                old: node.name.clone(),
                new: text.to_string(),
            }]
        }
        InspectorField::X(id) => parse_number(text)
            .map(|x| {
                align_to_bounds_h(
                    scene,
                    &[*id],
                    FantaBounds::from_xywh(x, 0.0, 0.0, 0.0),
                    HAlign::Left,
                )
            })
            .unwrap_or_default(),
        InspectorField::Y(id) => parse_number(text)
            .map(|y| {
                align_to_bounds_v(
                    scene,
                    &[*id],
                    FantaBounds::from_xywh(0.0, y, 0.0, 0.0),
                    VAlign::Top,
                )
            })
            .unwrap_or_default(),
        InspectorField::Width(id) => parse_number(text)
            .map(|width| resize_operations(doc, *id, width, true))
            .unwrap_or_default(),
        InspectorField::Height(id) => parse_number(text)
            .map(|height| resize_operations(doc, *id, height, false))
            .unwrap_or_default(),
        InspectorField::Rotation(id) => parse_number(text.trim_end_matches('°'))
            .map(|degrees| rotation_operations(doc, *id, degrees))
            .unwrap_or_default(),
        InspectorField::CornerRadius(id) => parse_number(text)
            .map(|radius| {
                replace_data_operation(doc, *id, |data| set_corner_radius(data, radius.max(0.0)))
            })
            .unwrap_or_default(),
        InspectorField::CornerSmoothing(id) => parse_number(text.trim_end_matches('%'))
            .map(|percent| {
                let smoothing = (percent / 100.0).clamp(0.0, 1.0) as f32;
                replace_data_operation(doc, *id, |data| set_corner_smoothing(data, smoothing))
            })
            .unwrap_or_default(),
        InspectorField::Opacity(id) => {
            let Some(percent) = parse_number(text.trim_end_matches('%')) else {
                return Vec::new();
            };
            let Some(node) = scene.get(*id) else {
                return Vec::new();
            };
            let new = (percent / 100.0).clamp(0.0, 1.0) as f32;
            if (new - node.opacity).abs() < f32::EPSILON {
                return Vec::new();
            }
            vec![Operation::SetOpacity {
                id: *id,
                old: node.opacity,
                new,
            }]
        }
        InspectorField::FillColor { id, index } => {
            let index = *index;
            parse_color(text)
                .map(|color| {
                    replace_data_operation(doc, *id, |data| set_fill_color(data, index, color))
                })
                .unwrap_or_default()
        }
        InspectorField::StrokeColor { id, index } => {
            let index = *index;
            parse_color(text)
                .map(|color| {
                    replace_data_operation(doc, *id, |data| {
                        if let Some(strokes) = stroke_list_mut(data)
                            && let Some(stroke) = strokes.get_mut(index)
                        {
                            stroke.paint = Fill::solid(color);
                        }
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::PaintOpacity {
            id,
            index,
            is_stroke,
        } => {
            let (index, is_stroke) = (*index, *is_stroke);
            parse_number(text.trim_end_matches('%'))
                .map(|percent| {
                    let fraction = (percent / 100.0).clamp(0.0, 1.0);
                    replace_data_operation(doc, *id, |data| {
                        if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
                            match paint {
                                Fill::Solid { color } => {
                                    color.a = (fraction * 255.0).round() as u8;
                                }
                                Fill::Image { opacity, .. } => *opacity = fraction as f32,
                                Fill::Gradient { .. } => {}
                            }
                        }
                    })
                })
                .unwrap_or_default()
        }
        // Gradients commit through the dedicated gradient-editor path, not the
        // text field, so there is no text operation to author here.
        InspectorField::Gradient { .. } => Vec::new(),
        InspectorField::StrokeWidth { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|width| {
                    replace_data_operation(doc, *id, |data| {
                        if let Some(strokes) = stroke_list_mut(data)
                            && let Some(stroke) = strokes.get_mut(index)
                        {
                            stroke.width = width.max(0.0);
                        }
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::FontFamily(id) => {
            if text.is_empty() {
                return Vec::new();
            }
            replace_data_operation(doc, *id, |data| {
                if let NodeData::Text(node_text) = data {
                    node_text.style.font_family = text.to_string();
                }
            })
        }
        InspectorField::FontSize(id) => parse_number(text)
            .filter(|size| *size > 0.0)
            .map(|size| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.size_px = size;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::LineHeight(id) => parse_number(text)
            .filter(|line_height| *line_height > 0.0)
            .map(|line_height| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.line_height = line_height;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::LetterSpacing(id) => parse_number(text)
            .map(|letter_spacing| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.letter_spacing = letter_spacing;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::CornerRadiusCorner { id, corner } => {
            let corner = *corner;
            parse_number(text)
                .map(|radius| {
                    replace_data_operation(doc, *id, |data| {
                        set_corner_radius_corner(data, corner, radius.max(0.0))
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::LayoutGapH(id) => layout_gap_operations(doc, *id, text, true),
        InspectorField::LayoutGapV(id) => layout_gap_operations(doc, *id, text, false),
        InspectorField::LayoutPadH(id) => layout_padding_operations(doc, *id, text, true),
        InspectorField::LayoutPadV(id) => layout_padding_operations(doc, *id, text, false),
        InspectorField::EffectOffsetX { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|x| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.offset[0] = x)
                })
                .unwrap_or_default()
        }
        InspectorField::EffectOffsetY { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|y| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.offset[1] = y)
                })
                .unwrap_or_default()
        }
        InspectorField::EffectBlur { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|blur| {
                    shadow_field_operations(doc, *id, index, move |shadow| {
                        shadow.blur = blur.max(0.0)
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::EffectSpread { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|spread| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.spread = spread)
                })
                .unwrap_or_default()
        }
        InspectorField::EffectColor { id, index } => {
            let index = *index;
            parse_color(text)
                .map(|color| {
                    shadow_field_operations(doc, *id, index, move |shadow| shadow.color = color)
                })
                .unwrap_or_default()
        }
        InspectorField::BlurRadius { id, index } => {
            let index = *index;
            parse_number(text)
                .map(|radius| {
                    blurs_operations(doc, *id, |blurs| {
                        if let Some(blur) = blurs.get_mut(index) {
                            blur.radius = radius.max(0.0);
                        }
                    })
                })
                .unwrap_or_default()
        }
        InspectorField::TextColor(id) => parse_color(text)
            .map(|color| {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Text(node_text) = data {
                        node_text.style.color = color;
                    }
                })
            })
            .unwrap_or_default(),
        InspectorField::InstanceTextProp { id, prop } => instance_prop_operations(
            doc,
            *id,
            *prop,
            Some(VarValue::String {
                value: text.to_string(),
            }),
        ),
        InspectorField::InstanceNumberProp { id, prop } => parse_number(text)
            .map(|value| instance_prop_operations(doc, *id, *prop, Some(VarValue::Float { value })))
            .unwrap_or_default(),
        InspectorField::InstanceColorProp { id, prop } => parse_color(text)
            .map(|value| instance_prop_operations(doc, *id, *prop, Some(VarValue::Color { value })))
            .unwrap_or_default(),
        InspectorField::SelectionColor { from } => parse_color(text)
            .filter(|to| to != from)
            .map(|to| selection_color_operations(doc, *from, to))
            .unwrap_or_default(),
        InspectorField::PageBackground(id) => {
            if text.is_empty() {
                replace_data_operation(doc, *id, |data| {
                    if let NodeData::Group(group) = data {
                        group.background = None;
                    }
                })
            } else {
                parse_color(text)
                    .map(|color| {
                        replace_data_operation(doc, *id, |data| {
                            if let NodeData::Group(group) = data {
                                group.background = Some(Fill::solid(color));
                            }
                        })
                    })
                    .unwrap_or_default()
            }
        }
    }
}

/// The committed display text a field currently shows, for seeding the shared
/// editor when Tab hops between paired fields. Only the fields reachable via
/// [`paired_field`] need coverage.
fn read_field_text(doc: &Doc, field: &InspectorField) -> Option<String> {
    let scene = &doc.scene;
    match field {
        InspectorField::X(id) => scene.world_bounds(*id).map(|b| format_number(b.min_x)),
        InspectorField::Y(id) => scene.world_bounds(*id).map(|b| format_number(b.min_y)),
        InspectorField::Width(id) => scene
            .world_obb_size(*id)
            .map(|(w, _)| w)
            .or_else(|| scene.world_bounds(*id).map(|b| b.width()))
            .map(format_number),
        InspectorField::Height(id) => scene
            .world_obb_size(*id)
            .map(|(_, h)| h)
            .or_else(|| scene.world_bounds(*id).map(|b| b.height()))
            .map(format_number),
        InspectorField::Rotation(id) => scene
            .get(*id)
            .map(|node| format_number(transform_angle(&node.transform).to_degrees())),
        InspectorField::CornerRadius(id) => {
            let node = scene.get(*id)?;
            match corner_radius_value(node) {
                CornerRadiusValue::Uniform(radius) => Some(format_number(radius)),
                CornerRadiusValue::PerCorner(_) => Some(String::new()),
                CornerRadiusValue::NotApplicable => None,
            }
        }
        InspectorField::CornerRadiusCorner { id, corner } => {
            let node = scene.get(*id)?;
            match corner_radius_value(node) {
                CornerRadiusValue::Uniform(radius) => Some(format_number(radius)),
                CornerRadiusValue::PerCorner(radii) => {
                    radii.get(*corner).copied().map(format_number)
                }
                CornerRadiusValue::NotApplicable => None,
            }
        }
        InspectorField::CornerSmoothing(id) => {
            corner_smoothing_value(scene.get(*id)?).map(format_number)
        }
        InspectorField::LineHeight(id) => text_style_value(scene, *id, |style| style.line_height),
        InspectorField::LetterSpacing(id) => {
            text_style_value(scene, *id, |style| style.letter_spacing)
        }
        InspectorField::FontSize(id) => text_style_value(scene, *id, |style| style.size_px),
        InspectorField::LayoutGapH(id) | InspectorField::LayoutGapV(id) => {
            let node = scene.get(*id)?;
            let layout = auto_layout_snapshot(node)?;
            Some(format_number(
                if matches!(field, InspectorField::LayoutGapH(_)) {
                    layout.gap_h
                } else {
                    layout.gap_v
                },
            ))
        }
        InspectorField::LayoutPadH(id) | InspectorField::LayoutPadV(id) => {
            let node = scene.get(*id)?;
            let layout = auto_layout_snapshot(node)?;
            let pad = if matches!(field, InspectorField::LayoutPadH(_)) {
                layout.pad_h
            } else {
                layout.pad_v
            };
            Some(pad.map(format_number).unwrap_or_default())
        }
        InspectorField::EffectOffsetX { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.offset[0])
        }
        InspectorField::EffectOffsetY { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.offset[1])
        }
        InspectorField::EffectBlur { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.blur)
        }
        InspectorField::EffectSpread { id, index } => {
            effect_value(scene, *id, *index, |shadow| shadow.spread)
        }
        _ => None,
    }
}

fn text_style_value(
    scene: &fanta_doc::Scene,
    id: NodeId,
    read: impl FnOnce(&fanta_doc::TextStyle) -> f64,
) -> Option<String> {
    let node = scene.get(id)?;
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    Some(format_number(read(&text.style)))
}

fn effect_value(
    scene: &fanta_doc::Scene,
    id: NodeId,
    index: usize,
    read: impl FnOnce(&Shadow) -> f64,
) -> Option<String> {
    let node = scene.get(id)?;
    node.effects
        .get(index)
        .map(|shadow| format_number(read(shadow)))
}

fn replace_data_operation(
    doc: &Doc,
    id: NodeId,
    mutate: impl FnOnce(&mut NodeData),
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let old = node.data.clone();
    let mut new = old.clone();
    mutate(&mut new);
    if new == old {
        return Vec::new();
    }
    vec![Operation::ReplaceData {
        id,
        old: Box::new(old),
        new: Box::new(new),
    }]
}

fn effects_operations(
    doc: &Doc,
    id: NodeId,
    mutate: impl FnOnce(&mut SmallVec<[Shadow; 0]>),
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let old = node.effects.clone();
    let mut new = old.clone();
    mutate(&mut new);
    if new == old {
        return Vec::new();
    }
    vec![Operation::SetEffects { id, old, new }]
}

fn shadow_field_operations(
    doc: &Doc,
    id: NodeId,
    index: usize,
    mutate: impl FnOnce(&mut Shadow),
) -> Vec<Operation> {
    effects_operations(doc, id, |effects| {
        if let Some(shadow) = effects.get_mut(index) {
            mutate(shadow);
        }
    })
}

/// The blur twin of [`effects_operations`]: snapshot the node's blur stack,
/// mutate a copy, and emit one `SetBlurs` when it actually changed.
fn blurs_operations(
    doc: &Doc,
    id: NodeId,
    mutate: impl FnOnce(&mut SmallVec<[Blur; 0]>),
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let old = node.blurs.clone();
    let mut new = old.clone();
    mutate(&mut new);
    if new == old {
        return Vec::new();
    }
    vec![Operation::SetBlurs { id, old, new }]
}

/// Set (or clear) one exposed prop on an instance. `None` clears the override
/// back to the def's default.
fn instance_prop_operations(
    doc: &Doc,
    id: NodeId,
    prop: ComponentPropId,
    new: Option<VarValue>,
) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return Vec::new();
    };
    let old = instance.prop_values.get(&prop).cloned();
    if old == new {
        return Vec::new();
    }
    vec![Operation::SetInstanceProp { id, prop, old, new }]
}

/// Replace one solid color across every selected node — the multi-select
/// "Selection colors" edit. One `ReplaceData` per node that actually uses the
/// color; the caller batches them into a single undo step.
fn selection_color_operations(doc: &Doc, from: FantaColor, to: FantaColor) -> Vec<Operation> {
    doc.selection
        .iter()
        .copied()
        .filter(|id| doc.scene.contains(*id))
        .flat_map(|id| {
            replace_data_operation(doc, id, |data| {
                let replace = |fill: &mut Fill| {
                    if let Fill::Solid { color } = fill
                        && *color == from
                    {
                        *color = to;
                    }
                };
                match data {
                    NodeData::Vector(vector) => {
                        vector.fills.iter_mut().for_each(replace);
                        vector
                            .strokes
                            .iter_mut()
                            .for_each(|stroke| replace(&mut stroke.paint));
                    }
                    NodeData::Group(group) => {
                        group
                            .background
                            .iter_mut()
                            .chain(group.background_fills.iter_mut())
                            .for_each(replace);
                        group
                            .strokes
                            .iter_mut()
                            .for_each(|stroke| replace(&mut stroke.paint));
                    }
                    NodeData::Text(text) => {
                        if text.style.color == from {
                            text.style.color = to;
                        }
                    }
                    _ => {}
                }
            })
        })
        .collect()
}

/// Detach an instance into a concrete subtree: the instance node becomes the
/// resolved master-root frame and the expansion's descendants are materialized
/// under it. The master root's own wrapper-level surface props (opacity,
/// effects, blurs, blend) live outside `NodeData`, so the data swap alone would
/// drop them — they are composed onto the detached node by the follow-up ops.
fn detach_instance_operations(doc: &Doc, id: NodeId) -> Vec<Operation> {
    let Some(node) = doc.scene.get(id) else {
        return Vec::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return Vec::new();
    };
    let expanded = expand_instance(&doc.scene, &doc.components, instance);
    // `expand_instance` yields the root first (empty def path), then its subtree
    // parents-before-children, so this insertion order stays valid.
    let Some(root) = expanded.iter().find(|entry| entry.def_path.is_empty()) else {
        // A dangling master: nothing to materialize.
        return Vec::new();
    };
    let root_id = root.node.id;
    let children: Vec<CanvasNode> = expanded
        .iter()
        .filter(|entry| !entry.def_path.is_empty())
        .map(|entry| {
            let mut child = entry.node.clone();
            if child.parent == Some(root_id) {
                child.parent = Some(id);
            }
            child
        })
        .collect();

    let mut operations = vec![Operation::DetachInstance {
        id,
        old: Box::new(node.data.clone()),
        new: Box::new(root.node.data.clone()),
        expanded: children,
    }];
    let composed_opacity = node.opacity * root.node.opacity;
    if (composed_opacity - node.opacity).abs() > f32::EPSILON {
        operations.push(Operation::SetOpacity {
            id,
            old: node.opacity,
            new: composed_opacity,
        });
    }
    if node.effects.is_empty() && !root.node.effects.is_empty() {
        operations.push(Operation::SetEffects {
            id,
            old: node.effects.clone(),
            new: root.node.effects.clone(),
        });
    }
    if node.blurs.is_empty() && !root.node.blurs.is_empty() {
        operations.push(Operation::SetBlurs {
            id,
            old: node.blurs.clone(),
            new: root.node.blurs.clone(),
        });
    }
    if node.blend_mode.is_normal() && !root.node.blend_mode.is_normal() {
        operations.push(Operation::SetBlendMode {
            id,
            old: node.blend_mode,
            new: root.node.blend_mode,
        });
    }
    operations
}

/// Merge the selected component masters into one variant set along a single
/// "Variant" axis whose values are the masters' names. The first selected master
/// becomes the set's default variant.
fn combine_as_variants_operations(doc: &Doc) -> Vec<Operation> {
    let masters = master_roots(&doc.components);
    let members: Vec<ComponentId> = doc
        .selection
        .iter()
        .filter_map(|id| masters.get(id).copied())
        // A master already in a set would need its old set repaired first.
        .filter(|component| {
            doc.components
                .def(*component)
                .is_some_and(|def| def.variant_of.is_none())
        })
        .collect();
    let (Some(default_variant), true) = (members.first().copied(), members.len() >= 2) else {
        return Vec::new();
    };
    const AXIS: &str = "Variant";
    let values: Vec<String> = members
        .iter()
        .filter_map(|component| doc.components.def(*component))
        .map(|def| def.name.clone())
        .collect();
    let set = ComponentSet {
        id: ComponentId::new(),
        name: doc
            .components
            .def(default_variant)
            .map(|def| def.name.clone())
            .unwrap_or_else(|| "Components".to_string()),
        axes: vec![VariantAxis {
            name: AXIS.to_string(),
            values,
        }],
        members: members.clone(),
        default_variant,
    };
    let set_id = set.id;
    let mut operations = vec![Operation::DefineComponentSet { set: Box::new(set) }];
    for component in members {
        let Some(def) = doc.components.def(component) else {
            continue;
        };
        let mut axis_values = std::collections::BTreeMap::new();
        axis_values.insert(AXIS.to_string(), def.name.clone());
        operations.push(Operation::SetVariantMembership {
            id: component,
            old: def.variant_of.clone(),
            new: Some(ComponentSetMembership {
                set: set_id,
                axis_values,
            }),
        });
    }
    operations
}

/// Write one screen-axis gap of an auto-layout frame. The horizontal gap is
/// the primary spacing of a horizontal stack but the counter (wrap) spacing of
/// a vertical one; see [`auto_layout_snapshot`].
fn layout_gap_operations(doc: &Doc, id: NodeId, text: &str, horizontal: bool) -> Vec<Operation> {
    let Some(gap) = parse_number(text).map(|gap| gap.max(0.0)) else {
        return Vec::new();
    };
    replace_data_operation(doc, id, |data| {
        if let NodeData::Group(group) = data
            && let Some(layout) = group.auto_layout.as_mut()
        {
            let primary = horizontal == (layout.mode == LayoutMode::Horizontal);
            if primary {
                layout.spacing = gap;
            } else {
                layout.counter_spacing = gap;
            }
        }
    })
}

fn layout_padding_operations(
    doc: &Doc,
    id: NodeId,
    text: &str,
    horizontal: bool,
) -> Vec<Operation> {
    let Some(padding) = parse_number(text).map(|padding| padding.max(0.0)) else {
        return Vec::new();
    };
    replace_data_operation(doc, id, |data| {
        if let NodeData::Group(group) = data
            && let Some(layout) = group.auto_layout.as_mut()
        {
            // padding is [top, right, bottom, left].
            if horizontal {
                layout.padding[1] = padding;
                layout.padding[3] = padding;
            } else {
                layout.padding[0] = padding;
                layout.padding[2] = padding;
            }
        }
    })
}

/// Change one oriented dimension of a node by pinning the opposite edge, the
/// same math the canvas resize handles use so rotation is preserved. The world
/// resize is rebased into the parent's frame to produce the new local
/// transform.
fn resize_operations(doc: &Doc, id: NodeId, new_size: f64, horizontal: bool) -> Vec<Operation> {
    if new_size <= 0.0 {
        return Vec::new();
    }
    let scene = &doc.scene;
    let Some(node) = scene.get(id) else {
        return Vec::new();
    };
    let Some(local) = scene.local_bounds(id) else {
        return Vec::new();
    };
    let Some(world_transform) = scene.world_transform(id) else {
        return Vec::new();
    };
    let theta = transform_angle(&world_transform);
    let rotation = Transform2D::rotation(theta);
    let rotation_inverse = rotation.inverse();
    let frame_bounds = local.transformed(&world_transform.then(&rotation_inverse));
    let (cursor_frame, handle) = if horizontal {
        (
            DVec2::new(frame_bounds.min_x + new_size, frame_bounds.center().y),
            ResizeHandle::East,
        )
    } else {
        (
            DVec2::new(frame_bounds.center().x, frame_bounds.min_y + new_size),
            ResizeHandle::South,
        )
    };
    let cursor_world = rotation.transform_point(cursor_frame);
    let new_world =
        resize_transform_keep_rotation(world_transform, local, handle, cursor_world, false, false);
    let parent_world = node
        .parent
        .and_then(|parent| scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let new_local = new_world.then(&parent_world.inverse());
    vec![Operation::SetTransform {
        id,
        old: node.transform,
        new: new_local,
    }]
}

/// Drop transform operations whose result is non-finite. Imported documents
/// can carry non-finite parent transforms, which the world→local rebase in
/// the field editors would propagate into NaN matrices; writing one poisons
/// layout and rendering for the whole subtree.
fn finite_transform_operations(operations: Vec<Operation>) -> Vec<Operation> {
    operations
        .into_iter()
        .filter(|operation| match operation {
            Operation::SetTransform { new, .. } => {
                let finite = new
                    .to_components()
                    .iter()
                    .all(|component| component.is_finite());
                if !finite {
                    log::warn!("discarding inspector edit that produced a non-finite transform");
                }
                finite
            }
            _ => true,
        })
        .collect()
}

fn rotation_operations(doc: &Doc, id: NodeId, degrees: f64) -> Vec<Operation> {
    let scene = &doc.scene;
    let Some(node) = scene.get(id) else {
        return Vec::new();
    };
    let Some(bounds) = scene.world_bounds(id) else {
        return Vec::new();
    };
    let parent_world = node
        .parent
        .and_then(|parent| scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let pivot = parent_world.inverse().transform_point(bounds.center());
    let current = transform_angle(&node.transform);
    let delta = degrees.to_radians() - current;
    if delta.abs() < 1e-9 {
        return Vec::new();
    }
    let new = node.transform.then(&rotate_about(pivot, delta));
    vec![Operation::SetTransform {
        id,
        old: node.transform,
        new,
    }]
}

// =============================================================================
// Transient previews
// =============================================================================

/// Restore the snapshotted node state directly into the scene — the
/// gesture-start baseline every preview frame recomputes from, mirroring the
/// canvas move tool's transient staging.
fn restore_snapshot(doc: &mut Doc, snapshot: &NodeSnapshot) {
    if let Some(node) = doc.scene.get_mut(snapshot.id) {
        node.transform = snapshot.transform;
        node.opacity = snapshot.opacity;
        node.data = (*snapshot.data).clone();
        node.effects = snapshot.effects.clone();
        node.blurs = snapshot.blurs.clone();
    }
}

/// Write an operation's `new` value straight into the scene WITHOUT history —
/// the preview twin of `Doc::apply`, so a scrub frame shows exactly what the
/// commit will produce.
fn apply_preview_operation(doc: &mut Doc, operation: &Operation) {
    match operation {
        Operation::SetTransform { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.transform = *new;
            }
        }
        Operation::SetOpacity { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.opacity = *new;
            }
        }
        Operation::ReplaceData { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.data = (**new).clone();
            }
        }
        Operation::SetEffects { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.effects = new.clone();
            }
        }
        Operation::SetBlurs { id, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id) {
                node.blurs = new.clone();
            }
        }
        Operation::SetInstanceProp { id, prop, new, .. } => {
            if let Some(node) = doc.scene.get_mut(*id)
                && let NodeData::Instance(instance) = &mut node.data
            {
                match new {
                    Some(value) => instance.prop_values.insert(*prop, value.clone()),
                    None => instance.prop_values.remove(prop),
                };
            }
        }
        _ => {}
    }
}

// =============================================================================
// Node data mutators
// =============================================================================

fn set_corner_radius(data: &mut NodeData, radius: f64) {
    let (corner_radius, corner_radii) = match data {
        NodeData::Vector(vector) => (&mut vector.corner_radius, &mut vector.corner_radii),
        NodeData::Group(group) => (&mut group.corner_radius, &mut group.corner_radii),
        _ => return,
    };
    *corner_radius = (radius > 0.0).then_some(radius);
    *corner_radii = None;
}

/// Set one corner of the per-corner radii (indexed [TL, TR, BR, BL]), seeding
/// the other three from the uniform radius. Collapses back to the uniform
/// field when all four end up equal so the compact representation survives.
fn set_corner_radius_corner(data: &mut NodeData, corner: usize, radius: f64) {
    let (corner_radius, corner_radii) = match data {
        NodeData::Vector(vector) => (&mut vector.corner_radius, &mut vector.corner_radii),
        NodeData::Group(group) => (&mut group.corner_radius, &mut group.corner_radii),
        _ => return,
    };
    let mut radii = corner_radii.unwrap_or([corner_radius.unwrap_or(0.0); 4]);
    let Some(slot) = radii.get_mut(corner) else {
        return;
    };
    *slot = radius;
    if radii.iter().all(|value| *value == radii[0]) {
        *corner_radius = (radii[0] > 0.0).then_some(radii[0]);
        *corner_radii = None;
    } else {
        *corner_radii = Some(radii);
    }
}

/// Set the squircle corner smoothing (0.0 = circular corners, 1.0 = full
/// squircle) on a corner-capable node.
fn set_corner_smoothing(data: &mut NodeData, smoothing: f32) {
    let slot = match data {
        NodeData::Vector(vector) => &mut vector.corner_smoothing,
        NodeData::Group(group) => &mut group.corner_smoothing,
        _ => return,
    };
    *slot = smoothing.clamp(0.0, 1.0);
}

fn stroke_list_mut(data: &mut NodeData) -> Option<&mut SmallVec<[Stroke; 1]>> {
    match data {
        NodeData::Vector(vector) => Some(&mut vector.strokes),
        NodeData::Group(group) => Some(&mut group.strokes),
        _ => None,
    }
}

fn group_fill_slot(group: &mut GroupNode, index: usize) -> Option<&mut Fill> {
    if group.background.is_some() {
        if index == 0 {
            group.background.as_mut()
        } else {
            group.background_fills.get_mut(index - 1)
        }
    } else {
        group.background_fills.get_mut(index)
    }
}

fn fill_slot_mut(data: &mut NodeData, index: usize) -> Option<&mut Fill> {
    match data {
        NodeData::Vector(vector) => vector.fills.get_mut(index),
        NodeData::Group(group) => group_fill_slot(group, index),
        _ => None,
    }
}

fn set_fill_color(data: &mut NodeData, index: usize, color: FantaColor) {
    if let Some(fill) = fill_slot_mut(data, index) {
        *fill = Fill::solid(color);
    }
}

/// A mutable handle to the paint at `index` in either the fill or stroke list.
fn paint_slot_mut(data: &mut NodeData, index: usize, is_stroke: bool) -> Option<&mut Fill> {
    if is_stroke {
        stroke_list_mut(data)
            .and_then(|strokes| strokes.get_mut(index))
            .map(|stroke| &mut stroke.paint)
    } else {
        fill_slot_mut(data, index)
    }
}

/// Replace the paint at `index` with a gradient fill, preserving the paint's
/// per-paint blend mode when it already carried one.
fn set_paint_gradient(data: &mut NodeData, index: usize, is_stroke: bool, gradient: Gradient) {
    if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
        let blend = match paint {
            Fill::Gradient { blend, .. } => *blend,
            _ => BlendMode::Normal,
        };
        *paint = Fill::Gradient { gradient, blend };
    }
}

/// Convert the paint at `index` to `kind`. Solid ⇄ gradient seeding matches
/// Figma: a solid becomes a two-stop gradient seeded from its color, and a
/// gradient flattens back to its representative (first-stop) color.
fn convert_paint_kind(data: &mut NodeData, index: usize, is_stroke: bool, kind: PaintKind) {
    let Some(paint) = paint_slot_mut(data, index, is_stroke) else {
        return;
    };
    match kind {
        PaintKind::Solid => {
            let color = match paint {
                Fill::Solid { color } => *color,
                Fill::Gradient { gradient, .. } => representative_gradient_color(gradient),
                Fill::Image { .. } => return,
            };
            *paint = Fill::solid(color);
        }
        PaintKind::Gradient(gradient_kind) => {
            let blend = match paint {
                Fill::Gradient { blend, .. } => *blend,
                _ => BlendMode::Normal,
            };
            let gradient = match paint {
                Fill::Gradient { gradient, .. } => {
                    crate::color_picker::convert_gradient_kind(gradient, gradient_kind)
                }
                Fill::Solid { color } => crate::color_picker::convert_gradient_kind(
                    &seed_gradient_from_color(*color),
                    gradient_kind,
                ),
                Fill::Image { .. } => return,
            };
            *paint = Fill::Gradient { gradient, blend };
        }
    }
}

fn remove_fill(data: &mut NodeData, index: usize) {
    match data {
        NodeData::Vector(vector) => {
            if index < vector.fills.len() {
                vector.fills.remove(index);
            }
        }
        NodeData::Group(group) => {
            if group.background.is_some() {
                if index == 0 {
                    group.background = None;
                } else if index - 1 < group.background_fills.len() {
                    group.background_fills.remove(index - 1);
                }
            } else if index < group.background_fills.len() {
                group.background_fills.remove(index);
            }
        }
        _ => {}
    }
}

fn add_fill(data: &mut NodeData) {
    match data {
        NodeData::Vector(vector) => vector.fills.push(Fill::solid(DEFAULT_FILL_COLOR)),
        NodeData::Group(group) => {
            if group.background.is_none() {
                group.background = Some(Fill::solid(DEFAULT_FILL_COLOR));
            } else {
                group.background_fills.push(Fill::solid(DEFAULT_FILL_COLOR));
            }
        }
        _ => {}
    }
}

fn default_shadow() -> Shadow {
    Shadow {
        kind: ShadowKind::Drop,
        color: FantaColor::rgba(0, 0, 0, 64),
        blur: 4.0,
        spread: 0.0,
        offset: [0.0, 2.0],
        // Figma's default: the shadow is knocked out under the node's body.
        show_behind_node: false,
    }
}

/// Figma's default blur radius for a freshly added layer / background blur.
fn default_blur(kind: BlurKind) -> Blur {
    Blur { kind, radius: 4.0 }
}

// =============================================================================
// PNG export
// =============================================================================

struct ExportJob {
    doc: Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    /// Subtree to render: the selected node, the page root, or `None` for
    /// every root (a document without explicit pages).
    root: Option<NodeId>,
    name: String,
    bounds: FantaBounds,
    project_root: PathBuf,
}

fn run_png_export(job: &ExportJob) -> Result<PathBuf> {
    const EXPORT_SCALE: f64 = 2.0;
    let width = ((job.bounds.width() * EXPORT_SCALE).ceil() as u32).clamp(1, MAX_EXPORT_PIXELS);
    let height = ((job.bounds.height() * EXPORT_SCALE).ceil() as u32).clamp(1, MAX_EXPORT_PIXELS);
    // When a dimension got clamped, shrink the zoom so the whole subject
    // still fits in frame instead of cropping it.
    let zoom = (f64::from(width) / job.bounds.width())
        .min(f64::from(height) / job.bounds.height())
        .min(EXPORT_SCALE);

    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating {width}x{height} export surface: {error}"))?;
    if let Some(asset_resolver) = job.asset_resolver.clone() {
        renderer.set_asset_resolver(asset_resolver);
    }
    let center = job.bounds.center();
    let viewport = Viewport {
        center: [center.x, center.y],
        zoom,
    };
    let inputs = RenderInputs {
        components: &job.doc.components,
        variables: &job.doc.variables,
        active_modes: &job.doc.active_modes,
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };
    renderer.render_page_with(&job.doc.scene, &viewport, job.root, &inputs);
    let png = renderer
        .encode_png()
        .map_err(|error| anyhow::anyhow!("encoding export PNG: {error}"))?;

    let exports_dir = job.project_root.join("exports");
    std::fs::create_dir_all(&exports_dir)
        .with_context(|| format!("creating {}", exports_dir.display()))?;
    let path = exports_dir.join(format!("{}.png", sanitize_file_name(&job.name)));
    std::fs::write(&path, png).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                '-'
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "export".to_string()
    } else {
        trimmed.to_string()
    }
}

// =============================================================================
// Formatting and parsing
// =============================================================================

fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}

fn parse_number(text: &str) -> Option<f64> {
    let cleaned = text.trim().trim_end_matches("px").trim();
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_color(text: &str) -> Option<FantaColor> {
    let trimmed = text.trim();
    if let Some(color) = FantaColor::from_hex(trimmed) {
        return Some(color);
    }
    FantaColor::from_hex(&format!("#{trimmed}"))
}

/// The dropdown label for a numeric OpenType weight. Weights off the standard
/// ladder (an imported 350 or 900) read "Custom" and are never rewritten —
/// only an explicit pick from the menu changes the stored value.
fn font_weight_label(weight: u16) -> &'static str {
    FONT_WEIGHTS
        .iter()
        .find(|(value, _)| *value == weight)
        .map(|(_, label)| *label)
        .unwrap_or("Custom")
}

fn fanta_color_rgba(color: FantaColor) -> Rgba {
    Rgba {
        r: f32::from(color.r) / 255.0,
        g: f32::from(color.g) / 255.0,
        b: f32::from(color.b) / 255.0,
        a: f32::from(color.a) / 255.0,
    }
}

impl EventEmitter<PanelEvent> for FantaPropertiesPanel {}

impl Focusable for FantaPropertiesPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for FantaPropertiesPanel {
    fn persistent_name() -> &'static str {
        "Fanta Properties Panel"
    }

    fn panel_key() -> &'static str {
        "FantaPropertiesPanel"
    }

    fn position(&self, _window: &Window, cx: &App) -> DockPosition {
        FantaPropertiesPanelSettings::get_global(cx).dock
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        position: DockPosition,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings.fanta_properties_panel.get_or_insert_default().dock = Some(position.into());
        });
    }

    fn default_size(&self, _window: &Window, cx: &App) -> Pixels {
        self.width
            .unwrap_or_else(|| FantaPropertiesPanelSettings::get_global(cx).default_width)
    }

    fn min_size(&self, _window: &Window, _cx: &App) -> Option<Pixels> {
        // Below this the 2-up field grid degenerates into unusable slivers.
        Some(px(260.))
    }

    fn icon(&self, _window: &Window, cx: &App) -> Option<IconName> {
        (FantaPropertiesPanelSettings::get_global(cx).button && self.active_view.is_some())
            .then_some(IconName::Sliders)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Fanta Properties Panel")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        9
    }

    fn enabled(&self, _cx: &App) -> bool {
        self.active_view.is_some()
    }
}

#[cfg(test)]
mod panel_integration_tests {
    //! End-to-end coverage for the gradient path through the *real*
    //! [`FantaPropertiesPanel`]: a Fanta project with a gradient-filled node is
    //! written to a real temp dir, opened through a `Project`/`FigView`, mounted
    //! in the panel, and drawn — then the gradient editor is opened, previewed,
    //! and committed. This reproduces the "crash on opening gradients" a plain
    //! render test of the gradient widgets alone cannot, because the panic lives
    //! in the panel ↔ item preview/commit wiring, not in the widgets.
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    use fanta_doc::{CanvasNode, GradientStop, GroupNode, VectorNode};
    use gpui::TestAppContext;
    use project::{Project, ProjectItem as _, ProjectPath};
    use workspace::ProjectItem as _;

    fn init_test(cx: &mut TestAppContext) {
        // The panel loads its document off a real temp dir through a real
        // `Project`, whose worktree scan and file watcher block on real IO.
        cx.executor().allow_parking();
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn linear_gradient_fill() -> Gradient {
        Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: FantaColor::BLACK,
                },
                GradientStop {
                    position: 1.0,
                    color: FantaColor::WHITE,
                },
            ],
        }
    }

    /// Write a Fanta project containing a page with a gradient-filled vector and
    /// a text node, returning their stable node ids.
    fn write_gradient_project(root: &Path, gradient: Gradient) -> (NodeId, NodeId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page One".into();
        let page_id = page.id;
        doc.scene.insert(page).unwrap();
        doc.add_page(page_id);

        let mut vector =
            VectorNode::rect_solid(0.0, 0.0, 120.0, 60.0, FantaColor::rgb(200, 200, 200));
        vector.fills = smallvec::smallvec![Fill::Gradient {
            gradient,
            blend: BlendMode::Normal,
        }];
        let mut node = CanvasNode::new(NodeData::Vector(vector));
        node.parent = Some(page_id);
        node.name = "Gradient Rect".into();
        let vector_id = node.id;
        doc.scene.insert(node).unwrap();

        // A text node so the typography / decorations / glyph-fill sections get
        // laid out by the draw tests too.
        let mut text = fanta_doc::TextNode::new("Hello", 100.0, 24.0);
        text.style.weight = 350; // an off-ladder weight must survive a render
        text.align = TextAlign::Justify;
        let mut text_node = CanvasNode::new(NodeData::Text(text));
        text_node.parent = Some(page_id);
        text_node.name = "Label".into();
        let text_id = text_node.id;
        doc.scene.insert(text_node).unwrap();

        doc.set_active_page(Some(page_id));

        fanta_format::write_project_tree(root, &doc, &BTreeMap::new())
            .expect("writing the fanta project tree");
        (vector_id, text_id)
    }

    struct Harness {
        panel: gpui::WindowHandle<FantaPropertiesPanel>,
        vector_id: NodeId,
        text_id: NodeId,
        _view: Entity<FigView>,
        _temp: tempfile::TempDir,
    }

    impl Harness {
        /// Replace the document selection, the way the canvas would.
        fn select(&self, ids: &[NodeId], cx: &mut TestAppContext) {
            let item = self._view.read_with(cx, |view, _| view.item().clone());
            let ids = ids.to_vec();
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.selection.replace_with(ids);
                    ((), DocChange::Selection)
                });
            });
            cx.run_until_parked();
        }
    }

    async fn open_panel_with_gradient(gradient: Gradient, cx: &mut TestAppContext) -> Harness {
        let temp = tempfile::tempdir().unwrap();
        let (vector_id, text_id) = write_gradient_project(temp.path(), gradient);

        let fs = std::sync::Arc::new(fs::RealFs::new(None, cx.executor()));
        let project = Project::test(fs.clone(), [temp.path()], cx).await;
        let worktree_id = project.update(cx, |project, cx| {
            project.worktrees(cx).next().unwrap().read(cx).id()
        });
        let path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path("fanta.json").into(),
        };

        let item = cx
            .update(|cx| FigItem::try_open(&project, &path, cx))
            .expect("fanta.json is openable as a FigItem")
            .await
            .expect("loading the FigItem");
        cx.run_until_parked();

        // The document loads on a background task; wait for it, then select the
        // gradient node so the inspector shows its fill section.
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(vector_id);
                ((), DocChange::Selection)
            });
        });
        cx.run_until_parked();

        // A throwaway window supplies the `&mut Window` FigView construction
        // needs; the FigView entity itself is not window-bound.
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| {
                    FigView::for_project_item(project.clone(), None, item.clone(), window, cx)
                })
            })
            .unwrap();

        let fs_dyn: std::sync::Arc<dyn fs::Fs> = fs;
        let panel = cx.add_window(|window, cx| FantaPropertiesPanel {
            focus_handle: cx.focus_handle(),
            fs: fs_dyn,
            active_view: None,
            width: None,
            field_editor: cx.new(|cx| Editor::single_line(window, cx)),
            editing_field: None,
            content_scroll: ScrollHandle::new(),
            corner_radii_expanded: None,
            scrub: None,
            slider_tracks: [None; SLIDER_TRACK_COUNT],
            hidden_paint_alpha: HashMap::new(),
            picker: None,
            gradient_editor: None,
            swatch_press_dismissed: false,
            _subscriptions: Vec::new(),
            _active_view_subscription: None,
        });
        panel
            .update(cx, |panel, _, cx| {
                panel.set_active_view(Some(view.clone()), cx);
            })
            .unwrap();
        cx.run_until_parked();

        Harness {
            panel,
            vector_id,
            text_id,
            _view: view,
            _temp: temp,
        }
    }

    fn draw(panel: gpui::WindowHandle<FantaPropertiesPanel>, cx: &mut TestAppContext) {
        cx.update_window(panel.into(), |_, window, cx| {
            window.draw(cx).clear();
        })
        .expect("drawing the properties panel");
    }

    #[gpui::test]
    async fn gradient_fill_row_draws_without_panicking(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn opening_the_gradient_editor_and_previewing_does_not_panic(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;
        draw(harness.panel, cx);

        // Open the gradient editor from the swatch, then draw the popover.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, linear_gradient_fill(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        // Drive a change through the editor exactly like a user edit: the editor
        // emits `Changed`, the panel's subscription previews it onto the item.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                editor.update(cx, |editor, cx| {
                    editor.set_kind(crate::color_picker::GradientKind::Radial, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        // Open the nested stop color picker, draw both popovers, then drive a
        // color change all the way through: stop picker → gradient editor →
        // panel preview → item. Finally commit the stop picker (which drops the
        // subscription that is mid-callback) and the whole editor.
        harness
            .panel
            .update(cx, |panel, window, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                editor.update(cx, |editor, cx| editor.open_stop_picker(0, window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        harness
            .panel
            .update(cx, |panel, window, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                let picker = editor.read(cx).stop_picker_for_test().unwrap();
                picker.update(cx, |picker, cx| {
                    picker.set_test_color(FantaColor::rgb(12, 240, 33), window, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        // Commit the edit (one undoable op) and redraw.
        harness
            .panel
            .update(cx, |panel, _, cx| panel.close_gradient_editor(true, cx))
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn converting_solid_to_gradient_via_paint_type_does_not_panic(cx: &mut TestAppContext) {
        init_test(cx);
        // Start from a gradient node, flatten to solid, then back to a gradient
        // through the same `set_paint_kind` path the paint-type dropdown drives.
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_paint_kind(vector_id, 0, false, PaintKind::Solid, cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_paint_kind(
                    vector_id,
                    0,
                    false,
                    PaintKind::Gradient(crate::color_picker::GradientKind::Angular),
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn real_clicks_open_and_dismiss_the_gradient_editor(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        // A tall panel so the fill section (near the bottom of the stack) is
        // laid out and hit-testable.
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let swatch = vcx
            .debug_bounds("fanta-fill-gradient-swatch-0")
            .expect("the gradient fill swatch is laid out");
        // Real click on the swatch dispatches through the hitbox / click
        // machinery — the exact path the app takes to open the editor.
        vcx.simulate_click(swatch.center(), gpui::Modifiers::default());
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        // Click far outside the popover: the editor's `on_mouse_down_out`
        // commits and dismisses it.
        vcx.simulate_click(gpui::Point::new(px(5.), px(5.)), gpui::Modifiers::default());
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();
    }

    fn node_x(harness: &Harness, cx: &mut TestAppContext) -> f64 {
        harness._view.read_with(cx, |view, cx| {
            let item = view.item().read(cx);
            let doc = &item.document().unwrap().doc;
            doc.scene
                .get(harness.vector_id)
                .unwrap()
                .transform
                .0
                .translation
                .x
        })
    }

    #[gpui::test]
    async fn real_drag_on_numeric_label_scrubs_the_value(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let start_x = node_x(&harness, cx);

        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let handle = vcx
            .debug_bounds("scrub-fanta-x-0")
            .expect("the X field scrub handle is laid out");
        let origin = handle.center();
        // Press, drag right by 40px past the gpui drag threshold, release —
        // exactly the gesture the app's scrub relies on.
        vcx.simulate_mouse_down(origin, MouseButton::Left, gpui::Modifiers::default());
        // Incremental moves, like a real drag: the first move past the threshold
        // only initiates gpui's drag; scrub deltas arrive on later moves.
        for step in 1..=8 {
            vcx.simulate_mouse_move(
                origin + gpui::point(px(step as f32 * 5.), px(0.)),
                MouseButton::Left,
                gpui::Modifiers::default(),
            );
        }
        vcx.simulate_mouse_up(
            origin + gpui::point(px(40.), px(0.)),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let end_x = node_x(&harness, cx);
        assert!(
            (end_x - start_x - 40.0).abs() < 1.5,
            "dragging the X label 40px right should scrub X by ~40 (from {start_x} to {end_x})"
        );
    }

    #[gpui::test]
    async fn plain_click_on_a_numeric_field_focuses_the_editor(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let field_x = InspectorField::X(harness.vector_id);

        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let handle = vcx
            .debug_bounds("scrub-fanta-x-0")
            .expect("X field is laid out");
        // A click with no drag must open the inline editor, not scrub.
        vcx.simulate_click(handle.center(), gpui::Modifiers::default());
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let editing = harness
            .panel
            .read_with(cx, |panel, _| panel.editing_field.clone())
            .unwrap();
        assert_eq!(
            editing,
            Some(field_x),
            "a plain click should focus the field editor"
        );
    }

    #[gpui::test]
    async fn real_drag_of_a_gradient_stop_does_not_panic(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;
        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        // Open the editor so its preview bar and stop markers are laid out.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, linear_gradient_fill(), cx);
            })
            .unwrap();
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let marker = vcx
            .debug_bounds("fanta-gradient-stop-marker-0")
            .expect("the first gradient stop marker is laid out");
        let origin = marker.center();
        vcx.simulate_mouse_down(origin, MouseButton::Left, gpui::Modifiers::default());
        for step in 1..=8 {
            vcx.simulate_mouse_move(
                origin + gpui::point(px(step as f32 * 4.), px(0.)),
                MouseButton::Left,
                gpui::Modifiers::default(),
            );
            vcx.update(|window, cx| {
                window.draw(cx).clear();
            });
        }
        vcx.simulate_mouse_up(
            origin + gpui::point(px(32.), px(0.)),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();
    }

    #[gpui::test]
    async fn selection_change_during_a_live_gradient_preview_does_not_panic(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;

        // Open the editor and drive a change so the session is marked `changed`
        // and a preview snapshot is staged onto the item.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, linear_gradient_fill(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        harness
            .panel
            .update(cx, |panel, _, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                editor.update(cx, |editor, cx| {
                    editor.set_kind(crate::color_picker::GradientKind::Radial, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();

        // Now change the selection on the item. This emits `SelectionChanged`,
        // which the panel handles by resetting per-subject state — restoring the
        // in-flight preview snapshot via a fresh `item.update` from inside the
        // item's own event dispatch. A re-entrant update would panic here.
        let view = harness._view.clone();
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.clear();
                ((), DocChange::Selection)
            });
        });
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn text_node_draws_typography_decorations_and_glyph_fill(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        // The text node carries an off-ladder weight (350) and a Justify align:
        // both were previously unrepresentable in the inspector.
        harness.select(&[harness.text_id], cx);
        draw(harness.panel, cx);

        let text_id = harness.text_id;
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_text_decoration(text_id, TextDecorationGlyph::Underline, cx);
                panel.set_font_weight(text_id, 800, cx);
                panel.set_text_align(text_id, TextAlign::Justify, cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        let (weight, underline, align) = harness._view.read_with(cx, |view, cx| {
            let item = view.item().read(cx);
            let doc = &item.document().unwrap().doc;
            let NodeData::Text(text) = &doc.scene.get(text_id).unwrap().data else {
                panic!("expected a text node");
            };
            (text.style.weight, text.style.underline, text.align)
        });
        assert_eq!(weight, 800);
        assert!(underline);
        assert_eq!(align, TextAlign::Justify);
    }

    #[gpui::test]
    async fn blur_rows_add_edit_and_remove_through_set_blurs(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;

        let blurs = |cx: &mut TestAppContext| {
            harness._view.read_with(cx, |view, cx| {
                let item = view.item().read(cx);
                let doc = &item.document().unwrap().doc;
                doc.scene.get(vector_id).unwrap().blurs.to_vec()
            })
        };

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.add_blur(vector_id, BlurKind::Layer, cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        assert_eq!(blurs(cx).len(), 1);
        assert_eq!(blurs(cx)[0].kind, BlurKind::Layer);

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_blur_kind(vector_id, 0, BlurKind::Background, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(blurs(cx)[0].kind, BlurKind::Background);

        harness
            .panel
            .update(cx, |panel, _, cx| panel.remove_blur(vector_id, 0, cx))
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        assert!(blurs(cx).is_empty());

        // The whole add → edit → remove sequence stays undoable.
        let item = harness._view.read_with(cx, |view, _| view.item().clone());
        item.update(cx, |item, cx| item.undo(cx).unwrap());
        cx.run_until_parked();
        assert_eq!(blurs(cx).len(), 1);
    }

    #[gpui::test]
    async fn hiding_then_showing_a_paint_preserves_partial_alpha(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;

        // Flatten the gradient to a partially transparent solid.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_paint_kind(vector_id, 0, false, PaintKind::Solid, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let item = harness._view.read_with(cx, |view, _| view.item().clone());
        item.update(cx, |item, cx| {
            item.apply(
                {
                    let doc = &item.document().unwrap().doc;
                    replace_data_operation(doc, vector_id, |data| {
                        if let Some(Fill::Solid { color }) = fill_slot_mut(data, 0) {
                            color.a = 128;
                        }
                    })
                    .pop()
                    .expect("an alpha edit")
                },
                cx,
            )
            .expect("applying the alpha edit");
        });
        cx.run_until_parked();

        let alpha = |cx: &mut TestAppContext| {
            harness._view.read_with(cx, |view, cx| {
                let item = view.item().read(cx);
                let doc = &item.document().unwrap().doc;
                let NodeData::Vector(vector) = &doc.scene.get(vector_id).unwrap().data else {
                    panic!("expected a vector");
                };
                match vector.fills.first() {
                    Some(Fill::Solid { color }) => color.a,
                    _ => panic!("expected a solid fill"),
                }
            })
        };
        assert_eq!(alpha(cx), 128);

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_paint_visibility(vector_id, 0, false, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(alpha(cx), 0, "hiding zeroes the paint's alpha");

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_paint_visibility(vector_id, 0, false, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            alpha(cx),
            128,
            "showing restores the alpha, not full opacity"
        );
    }

    #[gpui::test]
    async fn multi_selection_draws_selection_colors_and_replaces_across_the_selection(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        // The text node's black glyph color is the only solid in the selection.
        harness.select(&[harness.vector_id, harness.text_id], cx);
        draw(harness.panel, cx);

        let text_id = harness.text_id;
        let glyph_color = |cx: &mut TestAppContext| {
            harness._view.read_with(cx, |view, cx| {
                let item = view.item().read(cx);
                let doc = &item.document().unwrap().doc;
                let NodeData::Text(text) = &doc.scene.get(text_id).unwrap().data else {
                    panic!("expected a text node");
                };
                text.style.color
            })
        };
        let original = glyph_color(cx);
        let replacement = FantaColor::rgb(0x22, 0x88, 0x44);

        harness
            .panel
            .update(cx, |panel, _, cx| {
                let field = InspectorField::SelectionColor { from: original };
                let text = replacement.to_hex();
                panel.apply_document_ops(cx, |doc| field_operations(doc, &field, &text));
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        assert_eq!(glyph_color(cx), replacement);
    }

    #[gpui::test]
    async fn degenerate_gradient_fill_draws_without_panicking(cx: &mut TestAppContext) {
        init_test(cx);
        // A single-stop gradient — the kind a real imported `.fig` can carry.
        let gradient = Gradient::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
            handles: None,
            stops: vec![GradientStop {
                position: 0.0,
                color: FantaColor::rgb(30, 60, 90),
            }],
        };
        let harness = open_panel_with_gradient(gradient.clone(), cx).await;
        let vector_id = harness.vector_id;
        draw(harness.panel, cx);
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, gradient.clone(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn enabling_auto_layout_on_a_clip_less_group_seeds_its_clip_box(cx: &mut TestAppContext) {
        init_test(cx);
        // The page root is a plain Group (no clip_size, no background) with two
        // children — exactly the clip-less group the review flagged.
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let page_id = harness._view.read_with(cx, |view, cx| {
            view.item()
                .read(cx)
                .document()
                .expect("ready")
                .doc
                .pages()
                .first()
                .copied()
                .expect("one page")
        });
        harness.select(&[page_id], cx);

        // Precondition: a clip-less group.
        let before = harness._view.read_with(cx, |view, cx| {
            match &view
                .item()
                .read(cx)
                .document()
                .unwrap()
                .doc
                .scene
                .get(page_id)
                .unwrap()
                .data
            {
                NodeData::Group(group) => (group.auto_layout.is_some(), group.clip_size),
                _ => panic!("the page root is a group"),
            }
        });
        assert_eq!(
            before,
            (false, None),
            "starts as a clip-less group with no auto layout"
        );

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_auto_layout(page_id, true, cx)
            })
            .unwrap();
        cx.run_until_parked();

        // Enabling auto layout must seed a clip box from the content bounds, or
        // a later Hug sizing would collapse it to zero and hide every child.
        let (has_auto_layout, clip_size) = harness._view.read_with(cx, |view, cx| {
            match &view
                .item()
                .read(cx)
                .document()
                .unwrap()
                .doc
                .scene
                .get(page_id)
                .unwrap()
                .data
            {
                NodeData::Group(group) => (group.auto_layout.is_some(), group.clip_size),
                _ => unreachable!(),
            }
        });
        assert!(has_auto_layout, "auto layout is enabled");
        let clip = clip_size.expect("clip_size is seeded when auto layout is enabled");
        assert!(
            clip[0] > 0.0 && clip[1] > 0.0,
            "the seeded clip box wraps the content instead of collapsing to zero: {clip:?}"
        );
    }
}

#[cfg(test)]
mod tests {
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
    fn font_weight_dropdown_labels_the_ladder_and_keeps_custom_weights() {
        assert_eq!(font_weight_label(300), "Light");
        assert_eq!(font_weight_label(400), "Regular");
        assert_eq!(font_weight_label(500), "Medium");
        assert_eq!(font_weight_label(600), "SemiBold");
        assert_eq!(font_weight_label(700), "Bold");
        assert_eq!(font_weight_label(800), "ExtraBold");
        // Off-ladder weights read "Custom"; nothing snaps them onto a stop.
        assert_eq!(font_weight_label(350), "Custom");
        assert_eq!(font_weight_label(900), "Custom");
        // Every option the dropdown offers must round-trip through the label.
        for (weight, label) in FONT_WEIGHTS {
            assert_eq!(font_weight_label(weight), label);
        }
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

    fn vector_with_fill(fill: Fill) -> NodeData {
        let mut vector = fanta_doc::VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, FantaColor::BLACK);
        vector.fills = smallvec::smallvec![fill];
        NodeData::Vector(vector)
    }

    fn fill_at(data: &NodeData, index: usize) -> Option<&Fill> {
        match data {
            NodeData::Vector(vector) => vector.fills.get(index),
            _ => None,
        }
    }

    #[test]
    fn convert_solid_to_gradient_seeds_two_stops_from_color() {
        let color = FantaColor::rgb(0x11, 0x22, 0x33);
        let mut data = vector_with_fill(Fill::solid(color));
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Linear),
        );
        let Some(Fill::Gradient { gradient, .. }) = fill_at(&data, 0) else {
            panic!("fill should be a gradient after conversion");
        };
        let stops = crate::color_picker::gradient_stops(gradient);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].color, color);
        assert_eq!(stops[1].color, FantaColor::rgba(0x11, 0x22, 0x33, 0));
        assert_eq!(GradientKind::of(gradient), GradientKind::Linear);
    }

    #[test]
    fn convert_gradient_to_solid_flattens_to_first_stop() {
        let mut data = vector_with_fill(Fill::solid(FantaColor::rgb(0x40, 0x80, 0xC0)));
        // Solid → linear gradient, then back to solid keeps the base color.
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Linear),
        );
        convert_paint_kind(&mut data, 0, false, PaintKind::Solid);
        assert_eq!(
            fill_at(&data, 0),
            Some(&Fill::solid(FantaColor::rgb(0x40, 0x80, 0xC0)))
        );
    }

    #[test]
    fn convert_between_gradient_kinds_preserves_stops() {
        let mut data = vector_with_fill(Fill::solid(FantaColor::rgb(1, 2, 3)));
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Linear),
        );
        let stops_before = match fill_at(&data, 0) {
            Some(Fill::Gradient { gradient, .. }) => {
                crate::color_picker::gradient_stops(gradient).to_vec()
            }
            _ => panic!("expected gradient"),
        };
        convert_paint_kind(
            &mut data,
            0,
            false,
            PaintKind::Gradient(GradientKind::Radial),
        );
        let Some(Fill::Gradient { gradient, .. }) = fill_at(&data, 0) else {
            panic!("expected gradient");
        };
        assert_eq!(GradientKind::of(gradient), GradientKind::Radial);
        assert_eq!(
            crate::color_picker::gradient_stops(gradient),
            stops_before.as_slice()
        );
    }

    #[test]
    fn set_paint_gradient_preserves_per_paint_blend() {
        let base = Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                fanta_doc::GradientStop {
                    position: 0.0,
                    color: FantaColor::BLACK,
                },
                fanta_doc::GradientStop {
                    position: 1.0,
                    color: FantaColor::WHITE,
                },
            ],
        };
        let mut data = vector_with_fill(Fill::Gradient {
            gradient: base.clone(),
            blend: BlendMode::Multiply,
        });
        let radial = crate::color_picker::convert_gradient_kind(&base, GradientKind::Radial);
        set_paint_gradient(&mut data, 0, false, radial.clone());
        assert_eq!(
            fill_at(&data, 0),
            Some(&Fill::Gradient {
                gradient: radial,
                blend: BlendMode::Multiply,
            })
        );
    }

    // === Node-kind classification ========================================

    fn node_with_data(data: NodeData) -> CanvasNode {
        CanvasNode::new(data)
    }

    fn frame_group() -> GroupNode {
        GroupNode {
            background: Some(Fill::solid(FantaColor::WHITE)),
            ..GroupNode::default()
        }
    }

    fn text_node() -> fanta_doc::TextNode {
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

    // === Corner smoothing ================================================

    #[test]
    fn corner_smoothing_writes_the_fraction_and_clamps() {
        let mut data = vector_with_fill(Fill::solid(FantaColor::BLACK));
        set_corner_smoothing(&mut data, 0.6);
        let NodeData::Vector(vector) = &data else {
            panic!("expected a vector");
        };
        assert!((vector.corner_smoothing - 0.6).abs() < 1e-6);

        set_corner_smoothing(&mut data, 4.0);
        let NodeData::Vector(vector) = &data else {
            panic!("expected a vector");
        };
        assert_eq!(vector.corner_smoothing, 1.0);

        let mut group = NodeData::Group(GroupNode::default());
        set_corner_smoothing(&mut group, -1.0);
        let NodeData::Group(group) = &group else {
            panic!("expected a group");
        };
        assert_eq!(group.corner_smoothing, 0.0);

        // Kinds without corners are untouched.
        let mut text = NodeData::Text(text_node());
        let before = text.clone();
        set_corner_smoothing(&mut text, 1.0);
        assert_eq!(text, before);
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

    // === Blur op builder =================================================

    /// A one-node doc, so the op builders can be exercised against a real `Doc`.
    fn doc_with_node(data: NodeData) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let node = CanvasNode::new(data);
        let id = node.id;
        doc.scene.insert(node).expect("inserting the test node");
        (doc, id)
    }

    fn blurs_of(doc: &Doc, id: NodeId) -> Vec<Blur> {
        doc.scene
            .get(id)
            .map(|node| node.blurs.to_vec())
            .unwrap_or_default()
    }

    #[test]
    fn blurs_operations_emits_one_set_blurs_carrying_the_prior_stack() {
        let (doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        let operations =
            blurs_operations(&doc, id, |blurs| blurs.push(default_blur(BlurKind::Layer)));
        assert_eq!(operations.len(), 1);
        let Some(Operation::SetBlurs { old, new, .. }) = operations.first() else {
            panic!("expected a SetBlurs operation");
        };
        assert!(old.is_empty(), "undo must restore the empty stack");
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].kind, BlurKind::Layer);
        assert_eq!(new[0].radius, 4.0);
    }

    #[test]
    fn blurs_operations_is_a_no_op_when_nothing_changed() {
        let (doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        assert!(blurs_operations(&doc, id, |_| {}).is_empty());
        // Removing past the end changes nothing either.
        assert!(
            blurs_operations(&doc, id, |blurs| {
                if 3 < blurs.len() {
                    blurs.remove(3);
                }
            })
            .is_empty()
        );
        assert!(
            blurs_operations(&doc, NodeId::new(), |blurs| blurs
                .push(default_blur(BlurKind::Layer)))
            .is_empty()
        );
    }

    #[test]
    fn blur_radius_commits_and_previews_through_set_blurs() {
        let (mut doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        for operation in blurs_operations(&doc, id, |blurs| {
            blurs.push(default_blur(BlurKind::Background))
        }) {
            doc.apply(operation).expect("adding a blur");
        }
        assert_eq!(blurs_of(&doc, id).len(), 1);

        let operations = field_operations(&doc, &InspectorField::BlurRadius { id, index: 0 }, "12");
        assert_eq!(operations.len(), 1);
        // The scrub preview path must understand the op it will produce.
        for operation in &operations {
            apply_preview_operation(&mut doc, operation);
        }
        assert_eq!(blurs_of(&doc, id)[0].radius, 12.0);
        assert_eq!(blurs_of(&doc, id)[0].kind, BlurKind::Background);

        // Negative radii clamp rather than poisoning the renderer.
        let operations = field_operations(&doc, &InspectorField::BlurRadius { id, index: 0 }, "-5");
        for operation in &operations {
            apply_preview_operation(&mut doc, operation);
        }
        assert_eq!(blurs_of(&doc, id)[0].radius, 0.0);
    }

    #[test]
    fn a_blur_snapshot_restore_rolls_back_the_stack() {
        let (mut doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        let snapshot = {
            let node = doc.scene.get(id).expect("the test node");
            NodeSnapshot {
                id,
                transform: node.transform,
                opacity: node.opacity,
                data: Box::new(node.data.clone()),
                effects: node.effects.clone(),
                blurs: node.blurs.clone(),
            }
        };
        for operation in
            blurs_operations(&doc, id, |blurs| blurs.push(default_blur(BlurKind::Layer)))
        {
            doc.apply(operation).expect("adding a blur");
        }
        assert_eq!(blurs_of(&doc, id).len(), 1);
        restore_snapshot(&mut doc, &snapshot);
        assert!(blurs_of(&doc, id).is_empty());
    }

    // === Selection colors ================================================

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

    #[test]
    fn selection_color_edit_replaces_the_color_on_every_selected_node() {
        let from = FantaColor::rgb(10, 20, 30);
        let to = FantaColor::rgb(40, 50, 60);
        let mut doc = Doc::new();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let node = CanvasNode::new(vector_with_fill(Fill::solid(from)));
            ids.push(node.id);
            doc.scene.insert(node).expect("inserting a test node");
        }
        // A third node that does not use the color must not be touched.
        let untouched = CanvasNode::new(vector_with_fill(Fill::solid(FantaColor::WHITE)));
        let untouched_id = untouched.id;
        doc.scene.insert(untouched).expect("inserting a test node");
        doc.selection
            .replace_with(ids.iter().copied().chain([untouched_id]));

        let operations = selection_color_operations(&doc, from, to);
        assert_eq!(
            operations.len(),
            2,
            "only the nodes carrying the color author an op"
        );
        for operation in operations {
            doc.apply(operation).expect("replacing a selection color");
        }
        for id in ids {
            let node = doc.scene.get(id).expect("the test node");
            assert_eq!(fill_at(&node.data, 0), Some(&Fill::solid(to)));
        }
        let node = doc.scene.get(untouched_id).expect("the untouched node");
        assert_eq!(
            fill_at(&node.data, 0),
            Some(&Fill::solid(FantaColor::WHITE))
        );
    }

    // === Per-paint visibility (non-destructive) ==========================

    #[test]
    fn hiding_and_showing_a_paint_round_trips_partial_alpha() {
        let translucent = FantaColor::rgba(10, 20, 30, 128);
        let mut data = vector_with_fill(Fill::solid(translucent));
        let fill = fill_at(&data, 0).expect("a fill").clone();
        assert!(paint_is_visible(&fill));

        let remembered = paint_alpha(&fill);
        assert_eq!(remembered, HiddenPaintAlpha::Solid(128));

        // Hide.
        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            let zeroed = zeroed_paint_alpha(paint);
            set_paint_alpha(paint, &zeroed);
        }
        assert!(!paint_is_visible(fill_at(&data, 0).expect("a fill")));

        // Show, restoring the remembered alpha rather than forcing 255.
        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            set_paint_alpha(paint, &remembered);
        }
        assert_eq!(fill_at(&data, 0), Some(&Fill::solid(translucent)));
    }

    #[test]
    fn gradient_and_image_paints_can_be_hidden_too() {
        let gradient = Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                fanta_doc::GradientStop {
                    position: 0.0,
                    color: FantaColor::rgba(0, 0, 0, 200),
                },
                fanta_doc::GradientStop {
                    position: 1.0,
                    color: FantaColor::rgba(255, 255, 255, 90),
                },
            ],
        };
        let mut data = vector_with_fill(Fill::Gradient {
            gradient,
            blend: BlendMode::Normal,
        });
        let remembered = paint_alpha(fill_at(&data, 0).expect("a fill"));
        assert_eq!(remembered, HiddenPaintAlpha::Gradient(vec![200, 90]));

        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            let zeroed = zeroed_paint_alpha(paint);
            set_paint_alpha(paint, &zeroed);
        }
        assert!(!paint_is_visible(fill_at(&data, 0).expect("a fill")));
        if let Some(paint) = paint_slot_mut(&mut data, 0, false) {
            set_paint_alpha(paint, &remembered);
        }
        assert_eq!(
            paint_alpha(fill_at(&data, 0).expect("a fill")),
            HiddenPaintAlpha::Gradient(vec![200, 90])
        );

        let mut image = vector_with_fill(Fill::Image {
            asset: fanta_doc::AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 0.4,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
        });
        let remembered = paint_alpha(fill_at(&image, 0).expect("a fill"));
        assert_eq!(remembered, HiddenPaintAlpha::Image(0.4));
        if let Some(paint) = paint_slot_mut(&mut image, 0, false) {
            let zeroed = zeroed_paint_alpha(paint);
            set_paint_alpha(paint, &zeroed);
        }
        assert!(!paint_is_visible(fill_at(&image, 0).expect("a fill")));
        if let Some(paint) = paint_slot_mut(&mut image, 0, false) {
            set_paint_alpha(paint, &remembered);
        }
        assert_eq!(
            paint_alpha(fill_at(&image, 0).expect("a fill")),
            HiddenPaintAlpha::Image(0.4)
        );
    }

    #[test]
    fn showing_a_paint_with_no_remembered_alpha_falls_back_to_opaque() {
        assert_eq!(
            opaque_paint_alpha(&HiddenPaintAlpha::Solid(0)),
            HiddenPaintAlpha::Solid(255)
        );
        assert_eq!(
            opaque_paint_alpha(&HiddenPaintAlpha::Gradient(vec![0, 0, 0])),
            HiddenPaintAlpha::Gradient(vec![255, 255, 255])
        );
        assert_eq!(
            opaque_paint_alpha(&HiddenPaintAlpha::Image(0.0)),
            HiddenPaintAlpha::Image(1.0)
        );
    }

    // === Instances: props, detach, combine ===============================

    /// A doc holding a component master (a frame with one vector child) and one
    /// live instance of it. Returns the doc, the master's component id, the
    /// master root's node id and the instance's node id.
    fn doc_with_instance() -> (Doc, ComponentId, NodeId, NodeId) {
        let mut doc = Doc::new();
        let master_root = CanvasNode::new(NodeData::Group(frame_group()));
        let master_root_id = master_root.id;
        doc.scene.insert(master_root).expect("inserting the master");

        let mut child = CanvasNode::new(vector_with_fill(Fill::solid(FantaColor::BLACK)));
        child.parent = Some(master_root_id);
        let child_id = child.id;
        doc.scene.insert(child).expect("inserting the master child");

        let component = ComponentId::new();
        let mut def = ComponentDef::new(component, master_root_id, "Button");
        def.props.push(fanta_doc::ComponentPropDef {
            id: ComponentPropId::new(),
            name: "Radius".into(),
            kind: ComponentPropKind::Number,
            formatter: Default::default(),
            default: VarValue::Float { value: 4.0 },
            bindings: Vec::new(),
        });
        doc.components.defs.insert(component, def);

        let instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: std::collections::BTreeMap::new(),
            derived: Vec::new(),
            local_size: [120.0, 60.0],
        }));
        let instance_id = instance.id;
        doc.scene.insert(instance).expect("inserting the instance");
        let _ = child_id;
        (doc, component, master_root_id, instance_id)
    }

    #[test]
    fn instance_number_and_color_props_commit_through_set_instance_prop() {
        let (mut doc, component, _, instance_id) = doc_with_instance();
        let prop = doc.components.def(component).unwrap().props[0].id;

        let field = InspectorField::InstanceNumberProp {
            id: instance_id,
            prop,
        };
        let operations = field_operations(&doc, &field, "12.5");
        assert_eq!(operations.len(), 1);
        let Some(Operation::SetInstanceProp { old, new, .. }) = operations.first() else {
            panic!("expected a SetInstanceProp operation");
        };
        assert_eq!(*old, None, "undo must clear back to the def default");
        assert_eq!(*new, Some(VarValue::Float { value: 12.5 }));

        // The scrub preview path must understand the op it will produce.
        for operation in &operations {
            apply_preview_operation(&mut doc, operation);
        }
        let NodeData::Instance(instance) = &doc.scene.get(instance_id).unwrap().data else {
            panic!("expected an instance");
        };
        assert_eq!(
            instance.prop_values.get(&prop),
            Some(&VarValue::Float { value: 12.5 })
        );

        // Re-committing the same value authors nothing.
        assert!(field_operations(&doc, &field, "12.5").is_empty());

        let field = InspectorField::InstanceColorProp {
            id: instance_id,
            prop,
        };
        let operations = field_operations(&doc, &field, "#204080");
        let Some(Operation::SetInstanceProp { new, .. }) = operations.first() else {
            panic!("expected a SetInstanceProp operation");
        };
        assert_eq!(
            *new,
            Some(VarValue::Color {
                value: FantaColor::rgb(0x20, 0x40, 0x80)
            })
        );
    }

    #[test]
    fn detach_instance_materializes_the_master_subtree_in_one_step() {
        let (mut doc, _, master_root_id, instance_id) = doc_with_instance();
        let operations = detach_instance_operations(&doc, instance_id);
        let Some(Operation::DetachInstance {
            id, new, expanded, ..
        }) = operations.first()
        else {
            panic!("expected a DetachInstance operation");
        };
        assert_eq!(*id, instance_id);
        // The instance node becomes the resolved master-root frame.
        assert!(matches!(**new, NodeData::Group(_)));
        // The master's one child is materialized, re-parented onto the node.
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].parent, Some(instance_id));
        assert_ne!(
            expanded[0].id, master_root_id,
            "the expansion must use fresh ids, not the master's"
        );

        for operation in operations {
            doc.apply(operation).expect("detaching the instance");
        }
        let node = doc.scene.get(instance_id).expect("the detached node");
        assert!(matches!(node.data, NodeData::Group(_)));
        assert_eq!(doc.scene.children_of(Some(instance_id)).len(), 1);

        // Detaching something that is not an instance is a no-op.
        assert!(detach_instance_operations(&doc, master_root_id).is_empty());
        assert!(detach_instance_operations(&doc, NodeId::new()).is_empty());
    }

    #[test]
    fn combine_as_variants_defines_a_set_and_joins_every_selected_master() {
        let mut doc = Doc::new();
        let mut components = Vec::new();
        for name in ["Default", "Hover"] {
            let root = CanvasNode::new(NodeData::Group(frame_group()));
            let root_id = root.id;
            doc.scene.insert(root).expect("inserting a master");
            let component = ComponentId::new();
            doc.components
                .defs
                .insert(component, ComponentDef::new(component, root_id, name));
            components.push((component, root_id));
        }
        doc.selection
            .replace_with(components.iter().map(|(_, root)| *root));

        let operations = combine_as_variants_operations(&doc);
        assert_eq!(operations.len(), 3, "one DefineComponentSet + two members");
        let Some(Operation::DefineComponentSet { set }) = operations.first() else {
            panic!("expected a DefineComponentSet operation");
        };
        assert_eq!(set.members.len(), 2);
        assert_eq!(set.default_variant, components[0].0);
        assert_eq!(set.axes.len(), 1);
        assert_eq!(set.axes[0].values, vec!["Default", "Hover"]);

        for operation in operations {
            doc.apply(operation).expect("combining as variants");
        }
        for (component, _) in &components {
            let membership = doc
                .components
                .def(*component)
                .expect("the master")
                .variant_of
                .as_ref()
                .expect("the master joined the set");
            assert_eq!(membership.axis_values.len(), 1);
        }

        // Masters already inside a set are skipped, so the button no-ops now.
        assert!(combine_as_variants_operations(&doc).is_empty());
    }

    #[test]
    fn combine_as_variants_needs_at_least_two_masters() {
        let mut doc = Doc::new();
        let root = CanvasNode::new(NodeData::Group(frame_group()));
        let root_id = root.id;
        doc.scene.insert(root).expect("inserting a master");
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Only"));
        doc.selection.select_only(root_id);
        assert!(combine_as_variants_operations(&doc).is_empty());
    }

    #[test]
    fn paint_opacity_commits_to_alpha_for_solids_and_opacity_for_images() {
        let (doc, id) = doc_with_node(vector_with_fill(Fill::solid(FantaColor::rgb(1, 2, 3))));
        let field = InspectorField::PaintOpacity {
            id,
            index: 0,
            is_stroke: false,
        };
        let operations = field_operations(&doc, &field, "50");
        let Some(Operation::ReplaceData { new, .. }) = operations.first() else {
            panic!("expected a ReplaceData operation");
        };
        let Some(Fill::Solid { color }) = fill_at(new, 0) else {
            panic!("expected a solid fill");
        };
        assert_eq!(color.a, 128);

        let (doc, id) = doc_with_node(vector_with_fill(Fill::Image {
            asset: fanta_doc::AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
        }));
        let field = InspectorField::PaintOpacity {
            id,
            index: 0,
            is_stroke: false,
        };
        let operations = field_operations(&doc, &field, "25%");
        let Some(Operation::ReplaceData { new, .. }) = operations.first() else {
            panic!("expected a ReplaceData operation");
        };
        let Some(Fill::Image { opacity, .. }) = fill_at(new, 0) else {
            panic!("expected an image fill");
        };
        assert!((opacity - 0.25).abs() < 1e-6);
    }
}
