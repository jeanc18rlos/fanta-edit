//! Host-controlled paint-picker presentation used by the Design panel.
//!
//! The picker owns focus, draft input text, and the selected gradient stop. It
//! never mutates a document or treats its local paint snapshot as
//! authoritative: every edit emits [`PaintPickerEvent::Edit`], and the host
//! accepts that edit by calling [`PaintPicker::set_target`] with fresh data.

use std::path::PathBuf;

use gpui::{
    Anchor, AnyElement, App, AppContext as _, Bounds, Context, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    canvas, div, linear_color_stop, linear_gradient, pattern_slash, point,
    prelude::FluentBuilder as _, px, relative, rgba,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    popover::Popover,
    scroll::{Scrollbar, ScrollbarAxis},
    v_flex,
};

use crate::atoms::{ActivateControl, ButtonControlExt as _, CONTROL_KEY_CONTEXT, track_bounds};
use crate::color::{parse_hex_rgba, rgba_channels};
use crate::molecules::{popup_height, popup_width};

use super::{
    CancelDesignInteraction, DesignBlendMode, DesignColor, DesignColorContrastCategory,
    DesignColorContrastLeafViewData, DesignColorContrastLevel, DesignColorContrastPaintViewData,
    DesignColorStyleSample, DesignColorStyleSampleSelection, DesignColorStyleSampleViewData,
    DesignGradientPaint, DesignGradientStop, DesignImageFilter, DesignImageFilters,
    DesignImagePaint, DesignMediaCropAction, DesignMediaCropToolState, DesignMediaDroppedFile,
    DesignMediaKind, DesignMediaPaintCapabilities, DesignMediaPaintPlacement,
    DesignMediaPaintScaleMode, DesignMediaPaintView, DesignMediaPaintViewData,
    DesignMediaSourceAction, DesignMenuPreviewPhase, DesignPaint, DesignPaintBinding,
    DesignPaintColorTarget, DesignPaintEdit, DesignPaintKind, DesignPaintPayload,
    DesignPaintProperty, DesignPaintSource, DesignPaintTransform, DesignPaintType,
    DesignPaintValue, DesignPaintVariableViewData, DesignPanelCollection, DesignPanelEditPhase,
    DesignPatternHorizontalAlignment, DesignPatternPaint, DesignPatternSpacing,
    DesignPatternTileType, DesignShaderDefinition, DesignShaderPropertyDefinition,
    DesignShaderPropertyValue, DesignShaderSelection, DesignShaderViewData, DesignSolidPaint,
    DesignVariable, DesignVariableImportState, DesignVariableResolvedValue, DesignVariableSource,
    DesignVideoPaint, DesignVideoPreviewAction, DesignVideoPreviewState, DesignVideoPreviewStatus,
};

const PICKER_WIDTH: f32 = 280.;
const PICKER_MAX_HEIGHT: f32 = 520.;
const PICKER_HEADER_HEIGHT: f32 = 40.;
const PAINT_TYPE_ROW_HEIGHT: f32 = 40.;
const PAINT_HEADER_CONTROL_SIZE: f32 = 28.;
const PAINT_HEADER_GAP: f32 = 4.;
const PAINT_HEADER_HORIZONTAL_PADDING: f32 = 16.;
const PAINT_HEADER_TRAILING_CONTROL_COUNT: usize = 2;
const COLOR_AREA_HEIGHT: f32 = 248.;
const CONTROL_HEIGHT: f32 = 16.;
const KEYBOARD_FINE_STEP: f32 = 0.01;
const KEYBOARD_COARSE_STEP: f32 = 0.1;
const HUE_FINE_STEP: f32 = 1.;
const HUE_COARSE_STEP: f32 = 10.;
const CROP_ROTATION_STEP_DEGREES: f32 = 15.;

fn rotated_crop_preview(crop_tool: DesignMediaCropToolState) -> DesignMediaCropAction {
    DesignMediaCropAction::Preview {
        transform: crop_tool.transform.rotated(CROP_ROTATION_STEP_DEGREES),
        zoom: crop_tool.zoom,
        aspect_ratio: crop_tool.aspect_ratio,
    }
}

pub(crate) fn media_drop_from_paths(
    paths: &[PathBuf],
    capabilities: DesignMediaPaintCapabilities,
) -> Option<DesignMediaDroppedFile> {
    capabilities
        .can_upload_source
        .then(|| DesignMediaDroppedFile::from_paths(paths, capabilities.accepted_drop_file_kinds))
        .flatten()
}

const fn paint_header_min_width() -> f32 {
    let controls = DesignPaintType::ALL.len() + PAINT_HEADER_TRAILING_CONTROL_COUNT;
    controls as f32 * PAINT_HEADER_CONTROL_SIZE
        // The zero-width flexible spacer between paint types and tools adds
        // one child, so the row has `controls` gaps rather than `controls - 1`.
        + controls as f32 * PAINT_HEADER_GAP
        + PAINT_HEADER_HORIZONTAL_PADDING
}

/// Opaque host identity for the paint currently shown by the picker.
#[derive(Clone, Debug, Eq)]
pub(crate) struct PaintPickerTarget {
    pub(crate) node_id: SharedString,
    pub(crate) collection: DesignPanelCollection,
    pub(crate) index: usize,
    pub(crate) paint_id: SharedString,
}

impl PartialEq for PaintPickerTarget {
    fn eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
            && self.collection == other.collection
            && if self.paint_id.is_empty() || other.paint_id.is_empty() {
                self.index == other.index
            } else {
                self.paint_id == other.paint_id
            }
    }
}

impl PaintPickerTarget {
    fn exactly_eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
            && self.collection == other.collection
            && self.index == other.index
            && self.paint_id == other.paint_id
    }
}

#[derive(Clone, Debug, Eq)]
struct BlendModeMenuPreview {
    target: PaintPickerTarget,
    original: DesignBlendMode,
    candidate: DesignBlendMode,
}

impl PartialEq for BlendModeMenuPreview {
    fn eq(&self, other: &Self) -> bool {
        self.target.exactly_eq(&other.target)
            && self.original == other.original
            && self.candidate == other.candidate
    }
}

/// Internal event emitted for a candidate host-controlled paint change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintPickerEvent {
    Edit {
        target: PaintPickerTarget,
        edit: Box<DesignPaintEdit>,
        phase: DesignPanelEditPhase,
    },
    BlendModePreview {
        target: PaintPickerTarget,
        original: DesignBlendMode,
        candidate: DesignBlendMode,
        phase: DesignMenuPreviewPhase,
    },
    SourceReplaceRequested {
        target: PaintPickerTarget,
    },
    MediaSourceActionRequested {
        target: PaintPickerTarget,
        source_id: SharedString,
        action: DesignMediaSourceAction,
    },
    MediaSourceDropRequested {
        target: PaintPickerTarget,
        expected_source_id: SharedString,
        expected_media_kind: DesignMediaKind,
        file: DesignMediaDroppedFile,
    },
    MediaCropActionRequested {
        target: PaintPickerTarget,
        action: DesignMediaCropAction,
    },
    VideoPreviewActionRequested {
        target: PaintPickerTarget,
        action: DesignVideoPreviewAction,
    },
    ShaderImportRequested {
        target: PaintPickerTarget,
        shader: DesignShaderSelection,
    },
    ShaderApplyRequested {
        target: PaintPickerTarget,
        shader: DesignShaderSelection,
    },
    ShaderPropertyBindRequested {
        target: PaintPickerTarget,
        definition_id: SharedString,
    },
    ShaderPropertyEditorRequested {
        target: PaintPickerTarget,
        definition_id: SharedString,
    },
    ShaderPropertyDetachRequested {
        target: PaintPickerTarget,
        definition_id: SharedString,
        variable_id: SharedString,
    },
    ColorVariableApplyRequested {
        target: PaintPickerTarget,
        color_target: DesignPaintColorTarget,
        variable_id: SharedString,
    },
    ColorVariableImportRequested {
        target: PaintPickerTarget,
        color_target: DesignPaintColorTarget,
        variable_id: SharedString,
    },
    ColorVariableDetachRequested {
        target: PaintPickerTarget,
        color_target: DesignPaintColorTarget,
        variable_id: SharedString,
    },
    ColorVariableCreateRequested {
        target: PaintPickerTarget,
        color_target: DesignPaintColorTarget,
        color: DesignColor,
    },
    /// Opens whole-Paint style creation from the exact picker leaf that
    /// exposed the creation menu. The surrounding panel revalidates the
    /// stable paint/gradient-stop identity before emitting its public action.
    PaintStyleCreateRequested {
        target: PaintPickerTarget,
        color_target: DesignPaintColorTarget,
    },
    ColorStyleSampleRequested {
        target: PaintPickerTarget,
        color_target: DesignPaintColorTarget,
        sample: DesignColorStyleSampleSelection,
    },
    EyedropperRequested {
        target: PaintPickerTarget,
        color_target: DesignPaintColorTarget,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Hsv {
    hue: f32,
    saturation: f32,
    value: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Hsl {
    hue: f32,
    saturation: f32,
    lightness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ParsedColorInput {
    color: DesignColor,
    explicit_alpha: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ColorFormat {
    #[default]
    Hex,
    Rgb,
    Css,
    Hsl,
    Hsb,
}

impl ColorFormat {
    const ALL: [Self; 5] = [Self::Hex, Self::Rgb, Self::Css, Self::Hsl, Self::Hsb];

    const fn index(self) -> usize {
        match self {
            Self::Hex => 0,
            Self::Rgb => 1,
            Self::Css => 2,
            Self::Hsl => 3,
            Self::Hsb => 4,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Hex => "Hex",
            Self::Rgb => "RGB",
            Self::Css => "CSS",
            Self::Hsl => "HSL",
            Self::Hsb => "HSB",
        }
    }

    const fn channel_labels(self) -> [&'static str; 3] {
        match self {
            Self::Hex | Self::Css => ["", "", ""],
            Self::Rgb => ["R", "G", "B"],
            Self::Hsl => ["H", "S", "L"],
            Self::Hsb => ["H", "S", "B"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MediaSettingsView {
    placement: DesignMediaPaintPlacement,
    filters: DesignImageFilters,
    crop_tool: DesignMediaCropToolState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContinuousControl {
    ColorArea,
    Hue,
    Alpha,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PaintPickerTab {
    #[default]
    Custom,
    Libraries,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PaintCreationKind {
    #[default]
    Style,
    Variable,
}

impl PaintCreationKind {
    const ALL: [Self; 2] = [Self::Style, Self::Variable];

    const fn label(self) -> &'static str {
        match self {
            Self::Style => "Create style",
            Self::Variable => "Create variable",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum PaintResourceScope {
    #[default]
    Page,
    Library {
        library_id: SharedString,
        library_name: SharedString,
    },
}

impl PaintResourceScope {
    fn label(&self) -> SharedString {
        match self {
            Self::Page => "Styles and variables on this page".into(),
            Self::Library { library_name, .. } => library_name.clone(),
        }
    }

    fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Page, Self::Page) => true,
            (
                Self::Library {
                    library_id: left, ..
                },
                Self::Library {
                    library_id: right, ..
                },
            ) => left == right,
            (Self::Page, Self::Library { .. }) | (Self::Library { .. }, Self::Page) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GradientTransformOperation {
    RotateClockwise90,
    FlipHorizontal,
}

const GRADIENT_KINDS: [DesignPaintKind; 4] = [
    DesignPaintKind::LinearGradient,
    DesignPaintKind::RadialGradient,
    DesignPaintKind::AngularGradient,
    DesignPaintKind::DiamondGradient,
];

#[derive(Clone, Debug, PartialEq)]
struct ContinuousPaintEdit {
    edit: DesignPaintEdit,
}

/// Stateful presentation for a single host-selected paint.
///
/// Keep one entity alive while the surrounding popover is mounted. Calling
/// [`set_target`](Self::set_target) replaces the controlled snapshot without
/// changing the target document.
pub(crate) struct PaintPicker {
    id: SharedString,
    focus_handle: FocusHandle,
    target: Option<PaintPickerTarget>,
    paint: Option<DesignPaint>,
    paint_variable_view_data: DesignPaintVariableViewData,
    color_style_sample_view_data: DesignColorStyleSampleViewData,
    media_view_data: DesignMediaPaintViewData,
    shader_view_data: DesignShaderViewData,
    disabled: bool,
    active_tab: PaintPickerTab,
    shader_browser_requested: bool,
    color_only_title: Option<SharedString>,
    color_only_color_editable: bool,
    color_only_opacity_editable: bool,
    color_format: ColorFormat,
    color_format_menu_open: bool,
    color_format_menu_index: usize,
    color_format_menu_focus_handle: FocusHandle,
    color_format_menu_return_focus: Option<FocusHandle>,
    creation_menu_open: bool,
    creation_menu_index: usize,
    creation_menu_focus_handle: FocusHandle,
    creation_menu_return_focus: Option<FocusHandle>,
    resource_scope: PaintResourceScope,
    resource_scope_menu_open: bool,
    resource_scope_menu_index: usize,
    resource_scope_menu_focus_handle: FocusHandle,
    resource_scope_menu_return_focus: Option<FocusHandle>,
    gradient_kind_menu_open: bool,
    gradient_kind_menu_index: usize,
    gradient_kind_menu_focus_handle: FocusHandle,
    gradient_kind_menu_return_focus: Option<FocusHandle>,
    blend_mode_menu_open: bool,
    blend_mode_menu_preview: Option<BlendModeMenuPreview>,
    contrast_checker_open: bool,
    contrast_view_data: Option<DesignColorContrastPaintViewData>,
    contrast_category: DesignColorContrastCategory,
    contrast_level: DesignColorContrastLevel,
    scroll_handle: ScrollHandle,
    selected_stop: usize,
    selected_stop_id: SharedString,
    remembered_hue: f32,
    color_area_bounds: Bounds<Pixels>,
    hue_bounds: Bounds<Pixels>,
    alpha_bounds: Bounds<Pixels>,
    gradient_bounds: Bounds<Pixels>,
    dragging_stop: Option<usize>,
    continuous_edit: Option<ContinuousPaintEdit>,
    resource_search_input: Entity<InputState>,
    hex_input: Entity<InputState>,
    color_channel_inputs: [Entity<InputState>; 3],
    opacity_input: Entity<InputState>,
    stop_position_input: Entity<InputState>,
    hex_invalid: bool,
    color_channel_invalid: [bool; 3],
    opacity_invalid: bool,
    stop_position_invalid: bool,
    suppress_input_events: bool,
    dismissal_event_guard: bool,
    /// A focused text field owns one Begin→(Preview)*→terminal session.
    ///
    /// Enter ends the session even though GPUI keeps the field focused; the
    /// following Blur must therefore be inert. A later Change while focus is
    /// retained opens a fresh session before emitting Preview.
    hex_edit_session: Option<DesignPaintEdit>,
    color_channel_edit_sessions: [Option<DesignPaintEdit>; 3],
    opacity_edit_session: Option<DesignPaintEdit>,
    stop_position_edit_session: Option<DesignPaintEdit>,
    ignore_next_hex_change: bool,
    ignore_next_color_channel_changes: [bool; 3],
    ignore_next_opacity_change: bool,
    ignore_next_stop_position_change: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaintPickerEvent> for PaintPicker {}

impl Focusable for PaintPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl PaintPicker {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let resource_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search styles and variables"));
        let hex_input = cx.new(|cx| InputState::new(window, cx).placeholder("RRGGBB"));
        let color_channel_inputs = [
            cx.new(|cx| InputState::new(window, cx).placeholder("0")),
            cx.new(|cx| InputState::new(window, cx).placeholder("0")),
            cx.new(|cx| InputState::new(window, cx).placeholder("0")),
        ];
        let opacity_input = cx.new(|cx| InputState::new(window, cx).placeholder("100"));
        let stop_position_input = cx.new(|cx| InputState::new(window, cx).placeholder("0"));

        let hex_subscription = cx.subscribe_in(
            &hex_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                this.handle_hex_input(event, window, cx);
            },
        );
        let opacity_subscription = cx.subscribe_in(
            &opacity_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                this.handle_opacity_input(event, window, cx);
            },
        );
        let stop_position_subscription = cx.subscribe_in(
            &stop_position_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                this.handle_stop_position_input(event, window, cx);
            },
        );
        let resource_search_subscription =
            cx.subscribe(&resource_search_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let mut subscriptions = vec![
            resource_search_subscription,
            hex_subscription,
            opacity_subscription,
            stop_position_subscription,
        ];
        for (channel, input) in color_channel_inputs.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    this.handle_color_channel_input(channel, event, window, cx);
                },
            ));
        }

        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            target: None,
            paint: None,
            paint_variable_view_data: DesignPaintVariableViewData::default(),
            color_style_sample_view_data: DesignColorStyleSampleViewData::default(),
            media_view_data: DesignMediaPaintViewData::default(),
            shader_view_data: DesignShaderViewData::default(),
            disabled: false,
            active_tab: PaintPickerTab::Custom,
            shader_browser_requested: false,
            color_only_title: None,
            color_only_color_editable: true,
            color_only_opacity_editable: true,
            color_format: ColorFormat::Hex,
            color_format_menu_open: false,
            color_format_menu_index: ColorFormat::Hex.index(),
            color_format_menu_focus_handle: cx.focus_handle(),
            color_format_menu_return_focus: None,
            creation_menu_open: false,
            creation_menu_index: PaintCreationKind::Style as usize,
            creation_menu_focus_handle: cx.focus_handle(),
            creation_menu_return_focus: None,
            resource_scope: PaintResourceScope::Page,
            resource_scope_menu_open: false,
            resource_scope_menu_index: 0,
            resource_scope_menu_focus_handle: cx.focus_handle(),
            resource_scope_menu_return_focus: None,
            gradient_kind_menu_open: false,
            gradient_kind_menu_index: 0,
            gradient_kind_menu_focus_handle: cx.focus_handle(),
            gradient_kind_menu_return_focus: None,
            blend_mode_menu_open: false,
            blend_mode_menu_preview: None,
            contrast_checker_open: false,
            contrast_view_data: None,
            contrast_category: DesignColorContrastCategory::Auto,
            contrast_level: DesignColorContrastLevel::Aa,
            scroll_handle: ScrollHandle::new(),
            selected_stop: 0,
            selected_stop_id: "".into(),
            remembered_hue: 0.,
            color_area_bounds: Bounds::default(),
            hue_bounds: Bounds::default(),
            alpha_bounds: Bounds::default(),
            gradient_bounds: Bounds::default(),
            dragging_stop: None,
            continuous_edit: None,
            resource_search_input,
            hex_input,
            color_channel_inputs,
            opacity_input,
            stop_position_input,
            hex_invalid: false,
            color_channel_invalid: [false; 3],
            opacity_invalid: false,
            stop_position_invalid: false,
            suppress_input_events: false,
            dismissal_event_guard: false,
            hex_edit_session: None,
            color_channel_edit_sessions: [None, None, None],
            opacity_edit_session: None,
            stop_position_edit_session: None,
            ignore_next_hex_change: false,
            ignore_next_color_channel_changes: [false; 3],
            ignore_next_opacity_change: false,
            ignore_next_stop_position_change: false,
            _subscriptions: subscriptions,
        }
    }

    /// Supplies the authoritative paint snapshot and opaque target identity.
    pub(crate) fn set_target(
        &mut self,
        node_id: impl Into<SharedString>,
        collection: DesignPanelCollection,
        index: usize,
        paint: DesignPaint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = PaintPickerTarget {
            node_id: node_id.into(),
            collection,
            index,
            paint_id: paint.id.clone(),
        };
        let target_changed = self.target.as_ref() != Some(&target);
        let paint = normalize_paint(paint);
        let preview_invalidated = self.blend_mode_menu_preview.as_ref().is_some_and(|active| {
            !active.target.exactly_eq(&target) || active.original != paint.blend_mode
        });
        if preview_invalidated {
            self.cancel_blend_mode_preview(cx);
        }

        let previous_stop_id = self.selected_stop_id.clone();
        if target_changed {
            self.reset_resource_search(window, cx);
            self.active_tab = PaintPickerTab::Custom;
            self.shader_browser_requested = paint.paint_type() == DesignPaintType::Shader;
            self.color_format_menu_open = false;
            self.creation_menu_open = false;
            self.creation_menu_index = PaintCreationKind::Style as usize;
            self.creation_menu_return_focus = None;
            self.resource_scope_menu_open = false;
            self.resource_scope_menu_return_focus = None;
            self.gradient_kind_menu_open = false;
            self.gradient_kind_menu_index = 0;
            self.gradient_kind_menu_return_focus = None;
            self.blend_mode_menu_open = false;
            self.contrast_checker_open = false;
            self.contrast_view_data = None;
            self.contrast_category = DesignColorContrastCategory::Auto;
            self.contrast_level = DesignColorContrastLevel::Aa;
            self.scroll_handle.set_offset(Point::default());
            self.selected_stop = 0;
            self.selected_stop_id = paint
                .gradient_stops
                .first()
                .map_or_else(|| "".into(), |stop| stop.id.clone());
            self.hex_invalid = false;
            self.color_channel_invalid = [false; 3];
            self.opacity_invalid = false;
            self.stop_position_invalid = false;
            self.dragging_stop = None;
            self.continuous_edit = None;
            self.reset_text_input_edit_sessions();
        } else {
            self.selected_stop = if !previous_stop_id.is_empty() {
                paint
                    .gradient_stops
                    .iter()
                    .position(|stop| stop.id == previous_stop_id)
                    .unwrap_or_else(|| clamp_stop_index(&paint, self.selected_stop))
            } else {
                clamp_stop_index(&paint, self.selected_stop)
            };
            self.selected_stop_id = paint
                .gradient_stops
                .get(self.selected_stop)
                .map_or_else(|| "".into(), |stop| stop.id.clone());
            if self.dragging_stop.is_some() {
                self.dragging_stop = Some(self.selected_stop);
            }
        }

        self.target = Some(target);
        self.paint = Some(paint);
        self.remember_current_hue();

        let preserve_hex_draft =
            !target_changed && self.hex_input.focus_handle(cx).is_focused(window);
        let preserve_opacity_draft =
            !target_changed && self.opacity_input.focus_handle(cx).is_focused(window);
        self.sync_inputs(
            window,
            cx,
            preserve_hex_draft,
            preserve_opacity_draft,
            !target_changed,
        );
        cx.notify();
    }

    /// Clears the target and returns the picker to its inert empty state.
    pub(crate) fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_blend_mode_preview(cx);
        self.reset_resource_search(window, cx);
        self.target = None;
        self.paint = None;
        self.active_tab = PaintPickerTab::Custom;
        self.shader_browser_requested = false;
        self.color_only_title = None;
        self.color_only_color_editable = true;
        self.color_only_opacity_editable = true;
        self.color_format = ColorFormat::Hex;
        self.color_format_menu_open = false;
        self.creation_menu_open = false;
        self.creation_menu_index = PaintCreationKind::Style as usize;
        self.creation_menu_return_focus = None;
        self.resource_scope = PaintResourceScope::Page;
        self.resource_scope_menu_open = false;
        self.resource_scope_menu_index = 0;
        self.resource_scope_menu_return_focus = None;
        self.gradient_kind_menu_open = false;
        self.gradient_kind_menu_index = 0;
        self.gradient_kind_menu_return_focus = None;
        self.blend_mode_menu_open = false;
        self.contrast_checker_open = false;
        self.contrast_view_data = None;
        self.contrast_category = DesignColorContrastCategory::Auto;
        self.contrast_level = DesignColorContrastLevel::Aa;
        self.scroll_handle.set_offset(Point::default());
        self.selected_stop = 0;
        self.selected_stop_id = "".into();
        self.remembered_hue = 0.;
        self.hex_invalid = false;
        self.color_channel_invalid = [false; 3];
        self.opacity_invalid = false;
        self.stop_position_invalid = false;
        self.dragging_stop = None;
        self.continuous_edit = None;
        self.reset_text_input_edit_sessions();
        self.sync_inputs(window, cx, false, false, false);
        cx.notify();
    }

    /// Enables or disables every editing surface without discarding the target.
    pub(crate) fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            if disabled {
                self.cancel_blend_mode_preview(cx);
                self.reset_text_input_edit_sessions();
                self.creation_menu_open = false;
                self.creation_menu_return_focus = None;
                self.resource_scope_menu_open = false;
                self.resource_scope_menu_return_focus = None;
                self.gradient_kind_menu_open = false;
                self.gradient_kind_menu_return_focus = None;
                self.blend_mode_menu_open = false;
            }
            cx.notify();
        }
    }

    /// Suppresses deferred input notifications while the containing Popover
    /// is being dismissed. The next controlled target sync re-enables input
    /// handling.
    pub(crate) fn prepare_for_dismissal(&mut self, cx: &mut Context<Self>) {
        self.cancel_blend_mode_preview(cx);
        self.suppress_input_events = true;
        self.dismissal_event_guard = true;
        self.continuous_edit = None;
        self.dragging_stop = None;
        self.creation_menu_open = false;
        self.creation_menu_return_focus = None;
        self.resource_scope_menu_open = false;
        self.resource_scope_menu_return_focus = None;
        self.gradient_kind_menu_open = false;
        self.gradient_kind_menu_return_focus = None;
        self.blend_mode_menu_open = false;
        self.reset_text_input_edit_sessions();
    }

    /// Restricts the retained picker to the shared solid-color editor.
    ///
    /// Design-panel surfaces such as Page background and text decoration use
    /// the same RGB/HSL/HSB, hue, alpha, and keyboard interaction as paints
    /// without exposing paint-type or library controls.
    pub(crate) fn set_color_only(&mut self, title: Option<SharedString>, cx: &mut Context<Self>) {
        if self.color_only_title != title {
            self.cancel_blend_mode_preview(cx);
            self.color_only_title = title;
            if self.color_only_title.is_none() {
                self.color_only_color_editable = true;
                self.color_only_opacity_editable = true;
            }
            self.active_tab = PaintPickerTab::Custom;
            self.color_format_menu_open = false;
            self.creation_menu_open = false;
            self.creation_menu_return_focus = None;
            self.resource_scope_menu_open = false;
            self.resource_scope_menu_return_focus = None;
            self.gradient_kind_menu_open = false;
            self.gradient_kind_menu_return_focus = None;
            self.blend_mode_menu_open = false;
            self.scroll_handle.set_offset(Point::default());
            cx.notify();
        }
    }

    pub(crate) fn color_only_title(&self) -> Option<&SharedString> {
        self.color_only_title.as_ref()
    }

    /// Independently gates RGB/eyedropper and alpha edits for retained
    /// color-only pickers.
    ///
    /// This is presentation state only. It lets a host keep one picker
    /// inspectable when, for example, a layout-guide Color leaf is bound but
    /// its Opacity leaf remains editable. Global [`Self::set_disabled`] and a
    /// read-only paint still take precedence.
    pub(crate) fn set_color_only_editability(
        &mut self,
        color_editable: bool,
        opacity_editable: bool,
        cx: &mut Context<Self>,
    ) {
        if self.color_only_color_editable == color_editable
            && self.color_only_opacity_editable == opacity_editable
        {
            return;
        }
        self.cancel_text_input_edit_sessions(cx);
        self.color_only_color_editable = color_editable;
        self.color_only_opacity_editable = opacity_editable;
        cx.notify();
    }

    pub(crate) const fn color_only_editability(&self) -> (bool, bool) {
        (
            self.color_only_color_editable,
            self.color_only_opacity_editable,
        )
    }

    fn resource_scopes(&self) -> Vec<PaintResourceScope> {
        let mut scopes = vec![PaintResourceScope::Page];
        for library in &self.color_style_sample_view_data.libraries {
            scopes.push(PaintResourceScope::Library {
                library_id: library.id.clone(),
                library_name: library.name.clone(),
            });
        }
        for variable in &self.paint_variable_view_data.variables {
            let DesignVariableSource::Library {
                library_id,
                library_name,
            } = &variable.source
            else {
                continue;
            };
            if scopes.iter().any(|scope| {
                matches!(
                    scope,
                    PaintResourceScope::Library {
                        library_id: existing,
                        ..
                    } if existing == library_id
                )
            }) {
                continue;
            }
            scopes.push(PaintResourceScope::Library {
                library_id: library_id.clone(),
                library_name: library_name.clone(),
            });
        }
        scopes
    }

    fn normalize_resource_scope(&mut self) {
        let scopes = self.resource_scopes();
        let Some(index) = scopes
            .iter()
            .position(|scope| scope.same_identity(&self.resource_scope))
        else {
            self.resource_scope = PaintResourceScope::Page;
            self.resource_scope_menu_index = 0;
            self.resource_scope_menu_open = false;
            self.resource_scope_menu_return_focus = None;
            return;
        };
        self.resource_scope = scopes[index].clone();
        self.resource_scope_menu_index = index;
    }

    fn reset_resource_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.resource_search_input.read(cx).value().is_empty() {
            self.resource_search_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
    }

    fn open_resource_scope_menu_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.normalize_resource_scope();
        self.resource_scope_menu_return_focus = window.focused(cx);
        self.resource_scope_menu_open = true;
        let focus_handle = self.resource_scope_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    fn close_resource_scope_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.resource_scope_menu_open = false;
        if let Some(return_focus) = self.resource_scope_menu_return_focus.take() {
            window.defer(cx, move |window, cx| {
                return_focus.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn select_resource_scope(
        &mut self,
        scope: PaintResourceScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .resource_scopes()
            .iter()
            .any(|candidate| candidate.same_identity(&scope))
        {
            self.resource_scope = scope;
            self.normalize_resource_scope();
        }
        self.close_resource_scope_menu(window, cx);
    }

    fn handle_resource_scope_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scopes = self.resource_scopes();
        let count = scopes.len();
        if count == 0 {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => {
                self.resource_scope_menu_index =
                    (self.resource_scope_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.resource_scope_menu_index = (self.resource_scope_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.resource_scope_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.resource_scope_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_resource_scope_menu(window, cx),
            "escape" => self.close_resource_scope_menu(window, cx),
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted resource scope; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    fn commit_resource_scope_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scopes = self.resource_scopes();
        if scopes.is_empty() {
            return;
        }
        let index = self.resource_scope_menu_index.min(scopes.len() - 1);
        self.select_resource_scope(scopes[index].clone(), window, cx);
    }

    /// Replaces the host-filtered Color-variable snapshot supplied by the
    /// surrounding Design-panel host.
    pub(crate) fn set_paint_variable_view_data(
        &mut self,
        view_data: DesignPaintVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.paint_variable_view_data != view_data {
            self.paint_variable_view_data = view_data;
            self.normalize_resource_scope();
            cx.notify();
        }
    }

    /// Supplies the exact host-owned contrast projection for the active paint.
    ///
    /// The selected audience and conformance level remain transient picker
    /// state. Ratio, effective background, Auto resolution, and correction
    /// colors never originate in the reusable component.
    pub(crate) fn set_contrast_view_data(
        &mut self,
        view_data: Option<DesignColorContrastPaintViewData>,
        cx: &mut Context<Self>,
    ) {
        let view_data = view_data.filter(DesignColorContrastPaintViewData::is_valid);
        if self.contrast_view_data == view_data {
            return;
        }
        self.contrast_view_data = view_data;
        if self.current_contrast_leaf().is_none() {
            self.contrast_checker_open = false;
        }
        cx.notify();
    }

    /// Replaces the host-filtered sample-only Color-style catalog.
    pub(crate) fn set_color_style_sample_view_data(
        &mut self,
        view_data: DesignColorStyleSampleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.color_style_sample_view_data != view_data {
            self.color_style_sample_view_data = view_data;
            self.normalize_resource_scope();
            cx.notify();
        }
    }

    /// Replaces host-controlled crop, video-preview, and media capability
    /// state without changing the paint document snapshot.
    pub(crate) fn set_media_view_data(
        &mut self,
        view_data: DesignMediaPaintViewData,
        cx: &mut Context<Self>,
    ) {
        if self.media_view_data != view_data {
            self.media_view_data = view_data;
            cx.notify();
        }
    }

    /// Replaces host-controlled fill-shader discovery/import data without
    /// mutating the current paint payload.
    pub(crate) fn set_shader_view_data(
        &mut self,
        view_data: DesignShaderViewData,
        cx: &mut Context<Self>,
    ) {
        if self.shader_view_data != view_data {
            self.shader_view_data = view_data;
            cx.notify();
        }
    }

    fn set_color_format(
        &mut self,
        format: ColorFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_text_input_edit_sessions(cx);
        self.color_format = format;
        self.color_format_menu_open = false;
        self.hex_invalid = false;
        self.color_channel_invalid = [false; 3];
        self.sync_inputs(window, cx, false, false, false);
        cx.notify();
    }

    fn open_color_format_menu_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.color_format_menu_index = self.color_format.index();
        self.color_format_menu_return_focus = window.focused(cx);
        self.color_format_menu_open = true;
        let focus_handle = self.color_format_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    fn close_color_format_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.color_format_menu_open = false;
        if let Some(return_focus) = self.color_format_menu_return_focus.take() {
            window.defer(cx, move |window, cx| {
                return_focus.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn handle_color_format_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = ColorFormat::ALL.len();
        match event.keystroke.key.as_str() {
            "up" => {
                self.color_format_menu_index = (self.color_format_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.color_format_menu_index = (self.color_format_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.color_format_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.color_format_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_color_format_menu(window, cx),
            "escape" => self.close_color_format_menu(window, cx),
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted color format; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    fn commit_color_format_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = ColorFormat::ALL.len();
        let format = ColorFormat::ALL[self.color_format_menu_index.min(count - 1)];
        self.set_color_format(format, window, cx);
        self.close_color_format_menu(window, cx);
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.disabled
    }

    #[cfg(test)]
    pub(crate) fn properties_editing_is_disabled(&self) -> bool {
        self.editing_disabled()
    }

    #[cfg(test)]
    pub(crate) fn source_replacement_is_disabled(&self) -> bool {
        self.source_replacement_disabled()
    }

    #[cfg(test)]
    pub(crate) fn media_source_action_is_disabled(&self, action: DesignMediaSourceAction) -> bool {
        self.media_source_action_disabled(action)
    }

    pub(crate) fn target(&self) -> Option<&PaintPickerTarget> {
        self.target.as_ref()
    }

    pub(crate) fn paint(&self) -> Option<&DesignPaint> {
        self.paint.as_ref()
    }

    fn current_media_view(&self) -> Option<&DesignMediaPaintView> {
        let target = self.target.as_ref()?;
        self.media_view_data
            .paint(target.collection, &target.paint_id, target.index)
    }

    fn current_media_capabilities(&self) -> DesignMediaPaintCapabilities {
        self.current_media_view()
            .map_or_else(DesignMediaPaintCapabilities::default, |view| {
                view.capabilities
            })
    }

    fn base_editing_disabled(&self) -> bool {
        self.disabled || self.paint.as_ref().is_some_and(paint_picker_locked)
    }

    fn editing_disabled(&self) -> bool {
        self.base_editing_disabled()
            || self.paint.as_ref().is_some_and(|paint| {
                matches!(
                    &paint.payload,
                    DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)
                ) && !self.current_media_capabilities().can_edit_properties
            })
    }

    fn source_replacement_disabled(&self) -> bool {
        if self.base_editing_disabled() {
            return true;
        }
        match self.paint.as_ref().map(|paint| &paint.payload) {
            Some(DesignPaintPayload::Pattern(_)) => false,
            Some(DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)) => {
                self.media_source_action_disabled(DesignMediaSourceAction::Upload)
            }
            _ => true,
        }
    }

    fn media_source_action_disabled(&self, action: DesignMediaSourceAction) -> bool {
        if self.base_editing_disabled() {
            return true;
        }
        let Some(paint_type) = self.paint.as_ref().map(DesignPaint::paint_type) else {
            return true;
        };
        !action.is_applicable_to(paint_type)
            || !self
                .current_media_capabilities()
                .allows_source_action(action)
    }

    fn color_editing_disabled(&self) -> bool {
        self.editing_disabled()
            || (self.color_only_title.is_some() && !self.color_only_color_editable)
            || self
                .paint
                .as_ref()
                .is_some_and(|paint| paint_color_locked(paint, self.selected_stop))
    }

    fn opacity_editing_disabled(&self) -> bool {
        self.editing_disabled()
            || (self.color_only_title.is_some() && !self.color_only_opacity_editable)
            || self.paint.as_ref().is_some_and(|paint| {
                paint.kind.is_gradient() && paint_color_locked(paint, self.selected_stop)
            })
    }

    fn remember_current_hue(&mut self) {
        let Some(color) = self.current_color() else {
            return;
        };
        let hsv = rgb_to_hsv(color);
        if hsv.saturation > f32::EPSILON {
            self.remembered_hue = hsv.hue;
        }
    }

    fn current_color(&self) -> Option<DesignColor> {
        self.paint
            .as_ref()
            .map(|paint| selected_color(paint, self.selected_stop))
    }

    fn current_picker_opacity(&self) -> Option<f32> {
        self.paint
            .as_ref()
            .map(|paint| picker_opacity(paint, self.selected_stop))
    }

    fn parse_color_text_input(&self, value: &str) -> Option<ParsedColorInput> {
        match self.color_format {
            ColorFormat::Hex => parse_hex_color_input(value),
            ColorFormat::Css => parse_css_color_input(value),
            ColorFormat::Rgb | ColorFormat::Hsl | ColorFormat::Hsb => None,
        }
    }

    fn color_text_terminal_edit(&self, parsed: ParsedColorInput) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        if parsed.explicit_alpha && !paint.kind.is_gradient() {
            Some(picker_opacity_edit(
                paint,
                self.selected_stop,
                f32::from(parsed.color.alpha) / 255. * 100.,
            ))
        } else {
            Some(color_input_edit(paint, self.selected_stop, parsed))
        }
    }

    fn preview_color_text_input(&self, parsed: ParsedColorInput, cx: &mut Context<Self>) {
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let _ = self.emit_edit(
            color_input_edit(paint, self.selected_stop, parsed),
            DesignPanelEditPhase::Preview,
            cx,
        );
        // Figma's solid-paint alpha is the paint opacity leaf, while a
        // gradient stop owns alpha on that exact color leaf. An 8/4-digit Hex
        // or CSS rgba edit therefore previews two typed solid-paint leaves
        // inside one whole-paint transaction, but only one gradient leaf.
        if parsed.explicit_alpha && !paint.kind.is_gradient() {
            let _ = self.emit_edit(
                picker_opacity_edit(
                    paint,
                    self.selected_stop,
                    f32::from(parsed.color.alpha) / 255. * 100.,
                ),
                DesignPanelEditPhase::Preview,
                cx,
            );
        }
    }

    fn current_hsv(&self) -> Option<Hsv> {
        let mut hsv = rgb_to_hsv(self.current_color()?);
        if hsv.saturation <= f32::EPSILON {
            hsv.hue = self.remembered_hue;
        }
        Some(hsv)
    }

    fn selected_color_target(&self) -> Option<DesignPaintColorTarget> {
        paint_color_target(self.paint.as_ref()?, self.selected_stop)
    }

    fn current_contrast_leaf(&self) -> Option<&DesignColorContrastLeafViewData> {
        let color_target = self.selected_color_target()?;
        self.contrast_view_data.as_ref()?.leaf(&color_target)
    }

    fn resolved_contrast_category(&self) -> Option<DesignColorContrastCategory> {
        let leaf = self.current_contrast_leaf()?;
        Some(match self.contrast_category {
            DesignColorContrastCategory::Auto => leaf.automatic_category,
            category => category,
        })
    }

    fn normalized_contrast_level(&self) -> DesignColorContrastLevel {
        if self.resolved_contrast_category() == Some(DesignColorContrastCategory::Graphics) {
            DesignColorContrastLevel::Aa
        } else {
            self.contrast_level
        }
    }

    pub(crate) fn request_color_variable_apply(
        &self,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        let Some(variable) = self.paint_variable_view_data.variable(variable_id.as_ref()) else {
            return;
        };
        match variable.import_state {
            DesignVariableImportState::Available => {
                cx.emit(PaintPickerEvent::ColorVariableImportRequested {
                    target,
                    color_target,
                    variable_id,
                });
            }
            DesignVariableImportState::Local | DesignVariableImportState::Imported => {
                cx.emit(PaintPickerEvent::ColorVariableApplyRequested {
                    target,
                    color_target,
                    variable_id,
                });
            }
        }
    }

    pub(crate) fn request_color_variable_detach(&self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target), Some(binding)) = (
            self.target.clone(),
            self.selected_color_target(),
            self.paint
                .as_ref()
                .and_then(|paint| selected_color_binding(paint, self.selected_stop)),
        ) else {
            return;
        };
        cx.emit(PaintPickerEvent::ColorVariableDetachRequested {
            target,
            color_target,
            variable_id: binding.variable_id.clone(),
        });
    }

    pub(crate) fn request_color_variable_create(&self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target), Some(color)) = (
            self.target.clone(),
            self.selected_color_target(),
            self.current_color(),
        ) else {
            return;
        };
        cx.emit(PaintPickerEvent::ColorVariableCreateRequested {
            target,
            color_target,
            color,
        });
    }

    pub(crate) fn request_paint_style_create(&self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        cx.emit(PaintPickerEvent::PaintStyleCreateRequested {
            target,
            color_target,
        });
    }

    fn open_creation_menu_from_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_disabled() || self.selected_color_target().is_none() {
            return;
        }
        self.creation_menu_index = PaintCreationKind::Style as usize;
        self.creation_menu_return_focus = window.focused(cx);
        self.creation_menu_open = true;
        let focus_handle = self.creation_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    fn close_creation_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.creation_menu_open = false;
        if let Some(return_focus) = self.creation_menu_return_focus.take() {
            window.defer(cx, move |window, cx| {
                return_focus.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn activate_creation_kind(
        &mut self,
        kind: PaintCreationKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match kind {
            PaintCreationKind::Style => self.request_paint_style_create(cx),
            PaintCreationKind::Variable => self.request_color_variable_create(cx),
        }
        self.close_creation_menu(window, cx);
    }

    fn handle_creation_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = PaintCreationKind::ALL.len();
        match event.keystroke.key.as_str() {
            "up" => {
                self.creation_menu_index = (self.creation_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.creation_menu_index = (self.creation_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.creation_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.creation_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_creation_menu(window, cx),
            "escape" => self.close_creation_menu(window, cx),
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted creation kind; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    fn commit_creation_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = PaintCreationKind::ALL.len();
        let kind = PaintCreationKind::ALL[self.creation_menu_index.min(count - 1)];
        self.activate_creation_kind(kind, window, cx);
    }

    pub(crate) fn request_color_style_sample(
        &self,
        sample: DesignColorStyleSampleSelection,
        cx: &mut Context<Self>,
    ) {
        if self.color_editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        if self
            .color_style_sample_view_data
            .sample(&sample)
            .is_none_or(|sample| sample.disabled_reason.is_some())
        {
            return;
        }
        cx.emit(PaintPickerEvent::ColorStyleSampleRequested {
            target,
            color_target,
            sample,
        });
    }

    pub(crate) fn request_eyedropper(&self, cx: &mut Context<Self>) {
        if self.color_editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        cx.emit(PaintPickerEvent::EyedropperRequested {
            target,
            color_target,
        });
    }

    fn select_blend_mode(&mut self, blend_mode: DesignBlendMode, cx: &mut Context<Self>) {
        self.cancel_blend_mode_preview(cx);
        self.blend_mode_menu_open = false;
        if blend_mode == DesignBlendMode::PassThrough {
            cx.notify();
            return;
        }
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::BlendMode,
                value: DesignPaintValue::BlendMode(blend_mode),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        cx.notify();
    }

    fn set_blend_mode_preview(
        &mut self,
        blend_mode: DesignBlendMode,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if !preview {
            if self
                .blend_mode_menu_preview
                .as_ref()
                .is_some_and(|active| active.candidate == blend_mode)
            {
                self.cancel_blend_mode_preview(cx);
            }
            return;
        }
        if self.editing_disabled() || blend_mode == DesignBlendMode::PassThrough {
            return;
        }
        let (Some(target), Some(paint)) = (self.target.clone(), self.paint.as_ref()) else {
            return;
        };
        if paint.blend_mode == blend_mode {
            return;
        }
        let next = BlendModeMenuPreview {
            target,
            original: paint.blend_mode,
            candidate: blend_mode,
        };
        if self.blend_mode_menu_preview.as_ref() == Some(&next) {
            return;
        }
        self.cancel_blend_mode_preview(cx);
        self.blend_mode_menu_preview = Some(next.clone());
        cx.emit(PaintPickerEvent::BlendModePreview {
            target: next.target,
            original: next.original,
            candidate: next.candidate,
            phase: DesignMenuPreviewPhase::Begin,
        });
    }

    fn cancel_blend_mode_preview(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.blend_mode_menu_preview.take() else {
            return;
        };
        cx.emit(PaintPickerEvent::BlendModePreview {
            target: active.target,
            original: active.original,
            candidate: active.candidate,
            phase: DesignMenuPreviewPhase::End,
        });
    }

    fn handle_picker_key(&self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if is_eyedropper_shortcut(event) {
            self.request_eyedropper(cx);
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    fn reset_text_input_edit_sessions(&mut self) {
        self.hex_edit_session = None;
        self.color_channel_edit_sessions = [None, None, None];
        self.opacity_edit_session = None;
        self.stop_position_edit_session = None;
        self.ignore_next_hex_change = false;
        self.ignore_next_color_channel_changes = [false; 3];
        self.ignore_next_opacity_change = false;
        self.ignore_next_stop_position_change = false;
    }

    fn cancel_text_input_edit_sessions(&mut self, cx: &mut Context<Self>) {
        let mut sessions = Vec::new();
        sessions.extend(self.hex_edit_session.take());
        for session in &mut self.color_channel_edit_sessions {
            sessions.extend(session.take());
        }
        sessions.extend(self.opacity_edit_session.take());
        sessions.extend(self.stop_position_edit_session.take());
        for session in sessions {
            self.finish_text_input_edit(session, None, cx);
        }
    }

    fn current_color_text_edit(&self) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        Some(color_edit(
            paint,
            self.selected_stop,
            selected_color(paint, self.selected_stop),
        ))
    }

    fn rgb_color_text_edit(&self, color: DesignColor) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        Some(color_edit(
            paint,
            self.selected_stop,
            rgb_edit_color(paint, self.selected_stop, color),
        ))
    }

    fn current_opacity_text_edit(&self) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        Some(picker_opacity_edit(
            paint,
            self.selected_stop,
            picker_opacity(paint, self.selected_stop),
        ))
    }

    fn begin_text_input_edit(
        &self,
        edit: DesignPaintEdit,
        cx: &mut Context<Self>,
    ) -> Option<DesignPaintEdit> {
        self.emit_edit(edit.clone(), DesignPanelEditPhase::Begin, cx)
            .map(|_| edit)
    }

    /// Emits the one unconditional terminal paired with a text-field Begin.
    ///
    /// Unlike ordinary one-shot edits, an unchanged Commit cannot be elided:
    /// the host still needs a terminal to release its transaction snapshot.
    fn finish_text_input_edit(
        &self,
        session: DesignPaintEdit,
        candidate: Option<DesignPaintEdit>,
        cx: &mut Context<Self>,
    ) {
        let candidate = candidate.filter(|edit| {
            self.paint
                .clone()
                .is_some_and(|mut paint| paint.apply_edit(edit))
        });
        let (edit, phase) = candidate.map_or((session, DesignPanelEditPhase::Cancel), |edit| {
            (edit, DesignPanelEditPhase::Commit)
        });
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::Edit {
            target,
            edit: Box::new(edit),
            phase,
        });
    }

    fn emit_edit(
        &self,
        edit: DesignPaintEdit,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> Option<DesignPaint> {
        if self.editing_disabled() {
            return None;
        }
        let mut paint = self.paint.clone()?;
        if phase != DesignPanelEditPhase::Begin && !paint.apply_edit(&edit) {
            return None;
        }
        if phase != DesignPanelEditPhase::Begin && self.paint.as_ref() == Some(&paint) {
            return None;
        }
        let target = self.target.clone()?;
        cx.emit(PaintPickerEvent::Edit {
            target,
            edit: Box::new(edit),
            phase,
        });
        Some(paint)
    }

    fn emit_payload(&self, paint: DesignPaint, cx: &mut Context<Self>) {
        if self.editing_disabled() || self.paint.as_ref() == Some(&paint) {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        let edit = DesignPaintEdit {
            property: DesignPaintProperty::Payload,
            value: DesignPaintValue::Payload(paint.payload.clone()),
        };
        cx.emit(PaintPickerEvent::Edit {
            target,
            edit: Box::new(edit),
            phase: DesignPanelEditPhase::Commit,
        });
    }

    fn begin_continuous_edit(&mut self, edit: DesignPaintEdit, cx: &mut Context<Self>) {
        self.cancel_text_input_edit_sessions(cx);
        self.continuous_edit = None;
        if self
            .emit_edit(edit.clone(), DesignPanelEditPhase::Begin, cx)
            .is_some()
        {
            self.continuous_edit = Some(ContinuousPaintEdit { edit });
        }
    }

    fn preview_continuous_edit(&mut self, edit: DesignPaintEdit, cx: &mut Context<Self>) {
        if self
            .emit_edit(edit.clone(), DesignPanelEditPhase::Preview, cx)
            .is_some()
        {
            self.continuous_edit = Some(ContinuousPaintEdit { edit });
        }
    }

    fn commit_continuous_edit(&mut self, cx: &mut Context<Self>) {
        let Some(continuous) = self.continuous_edit.take() else {
            return;
        };
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::Edit {
            target,
            edit: Box::new(continuous.edit),
            phase: DesignPanelEditPhase::Commit,
        });
    }

    fn select_paint_type(&mut self, paint_type: DesignPaintType, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        if paint_type == DesignPaintType::Shader && paint.paint_type() != DesignPaintType::Shader {
            self.active_tab = PaintPickerTab::Libraries;
            self.shader_browser_requested = true;
            self.scroll_handle.set_offset(Point::default());
            cx.notify();
            return;
        }
        self.shader_browser_requested = paint_type == DesignPaintType::Shader;
        let candidate = paint_for_type(paint, paint_type, self.selected_stop);
        self.selected_stop = clamp_stop_index(&candidate, self.selected_stop);
        self.selected_stop_id = candidate
            .gradient_stops
            .get(self.selected_stop)
            .map_or_else(|| "".into(), |stop| stop.id.clone());
        self.emit_payload(candidate, cx);
    }

    fn select_paint_kind(&mut self, kind: DesignPaintKind, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        if !kind.is_gradient() {
            self.select_paint_type(kind.paint_type(), cx);
            return;
        }
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        if !paint.kind.is_gradient() {
            let candidate = paint_for_kind(paint, kind, self.selected_stop);
            self.selected_stop = clamp_stop_index(&candidate, self.selected_stop);
            self.selected_stop_id = candidate
                .gradient_stops
                .get(self.selected_stop)
                .map_or_else(|| "".into(), |stop| stop.id.clone());
            self.emit_payload(candidate, cx);
            return;
        }
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientKind,
                value: DesignPaintValue::PaintKind(kind),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    fn select_gradient_stop(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        if !paint.kind.is_gradient() || index >= paint.gradient_stops.len() {
            return;
        }
        self.selected_stop = index;
        self.selected_stop_id = paint.gradient_stops[index].id.clone();
        self.remember_current_hue();
        self.hex_invalid = false;
        self.sync_inputs(window, cx, false, false, false);
        cx.notify();
    }

    fn add_gradient_stop(&mut self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let (candidate, selected_stop) = paint_with_added_stop(paint, self.selected_stop);
        self.selected_stop = selected_stop;
        self.selected_stop_id = candidate.gradient_stops[selected_stop].id.clone();
        let stop = candidate.gradient_stops[selected_stop].clone();
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientStopAdd,
                value: DesignPaintValue::GradientStop(stop),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    fn remove_gradient_stop(&mut self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        self.cancel_text_input_edit_sessions(cx);
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let removed_index = clamp_stop_index(paint, self.selected_stop);
        let stop = paint.gradient_stops[removed_index].clone();
        let Some((candidate, selected_stop)) = paint_with_removed_stop(paint, removed_index) else {
            return;
        };
        self.selected_stop = selected_stop;
        self.selected_stop_id = candidate
            .gradient_stops
            .get(selected_stop)
            .map_or_else(|| "".into(), |stop| stop.id.clone());
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientStopRemove {
                    stop_id: stop.id,
                    index: removed_index,
                },
                value: DesignPaintValue::None,
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    fn edit_color(
        &mut self,
        color: DesignColor,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        self.remembered_hue = rgb_to_hsv(color).hue;
        let edit = color_edit(paint, self.selected_stop, color);
        let _ = self.emit_edit(edit, phase, cx);
    }

    fn edit_rgb_color(
        &mut self,
        color: DesignColor,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let color = rgb_edit_color(paint, self.selected_stop, color);
        self.edit_color(color, phase, cx);
    }

    fn set_saturation_value(&mut self, saturation: f32, value: f32, cx: &mut Context<Self>) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let current = selected_color(paint, self.selected_stop);
        let color = rgb_edit_color(
            paint,
            self.selected_stop,
            hsv_to_color(
                Hsv {
                    hue: self.remembered_hue,
                    saturation: saturation.clamp(0., 1.),
                    value: value.clamp(0., 1.),
                },
                current.alpha,
            ),
        );
        let edit = color_edit(paint, self.selected_stop, color);
        if self.continuous_edit.is_some() {
            self.preview_continuous_edit(edit, cx);
        } else {
            let _ = self.emit_edit(edit, DesignPanelEditPhase::Commit, cx);
        }
    }

    fn set_hue(&mut self, hue: f32, cx: &mut Context<Self>) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        self.remembered_hue = hue.rem_euclid(360.);
        let current = selected_color(paint, self.selected_stop);
        let mut hsv = rgb_to_hsv(current);
        hsv.hue = self.remembered_hue;
        let color = rgb_edit_color(paint, self.selected_stop, hsv_to_color(hsv, current.alpha));
        let edit = color_edit(paint, self.selected_stop, color);
        if self.continuous_edit.is_some() {
            self.preview_continuous_edit(edit, cx);
        } else {
            let _ = self.emit_edit(edit, DesignPanelEditPhase::Commit, cx);
        }
        cx.notify();
    }

    fn set_alpha(&mut self, alpha: f32, cx: &mut Context<Self>) {
        if self.opacity_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let edit = picker_opacity_edit(paint, self.selected_stop, alpha * 100.);
        if self.continuous_edit.is_some() {
            self.preview_continuous_edit(edit, cx);
        } else {
            let _ = self.emit_edit(edit, DesignPanelEditPhase::Commit, cx);
        }
    }

    fn set_opacity(&self, opacity: f32, phase: DesignPanelEditPhase, cx: &mut Context<Self>) {
        if self.opacity_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let _ = self.emit_edit(
            picker_opacity_edit(paint, self.selected_stop, opacity),
            phase,
            cx,
        );
    }

    fn update_from_pointer(
        &mut self,
        control: ContinuousControl,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.paint.is_none()
            || match control {
                ContinuousControl::Alpha => self.opacity_editing_disabled(),
                ContinuousControl::ColorArea | ContinuousControl::Hue => {
                    self.color_editing_disabled()
                }
            }
        {
            return;
        }
        match control {
            ContinuousControl::ColorArea => {
                let x = fraction_x(self.color_area_bounds, position);
                let y = fraction_y(self.color_area_bounds, position);
                self.set_saturation_value(x, 1. - y, cx);
            }
            ContinuousControl::Hue => {
                self.set_hue(fraction_x(self.hue_bounds, position) * 360., cx);
            }
            ContinuousControl::Alpha => {
                self.set_alpha(fraction_x(self.alpha_bounds, position), cx);
            }
        }
    }

    fn handle_continuous_key(
        &mut self,
        control: ContinuousControl,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.paint.is_none()
            || match control {
                ContinuousControl::Alpha => self.opacity_editing_disabled(),
                ContinuousControl::ColorArea | ContinuousControl::Hue => {
                    self.color_editing_disabled()
                }
            }
        {
            return;
        }
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let handled = match control {
            ContinuousControl::ColorArea => {
                let Some(hsv) = self.current_hsv() else {
                    return;
                };
                let step = if shift {
                    KEYBOARD_COARSE_STEP
                } else {
                    KEYBOARD_FINE_STEP
                };
                match key {
                    "left" => {
                        self.set_saturation_value(hsv.saturation - step, hsv.value, cx);
                        true
                    }
                    "right" => {
                        self.set_saturation_value(hsv.saturation + step, hsv.value, cx);
                        true
                    }
                    "up" => {
                        self.set_saturation_value(hsv.saturation, hsv.value + step, cx);
                        true
                    }
                    "down" => {
                        self.set_saturation_value(hsv.saturation, hsv.value - step, cx);
                        true
                    }
                    _ => false,
                }
            }
            ContinuousControl::Hue => {
                let step = if shift {
                    HUE_COARSE_STEP
                } else {
                    HUE_FINE_STEP
                };
                match key {
                    "left" | "down" => {
                        self.set_hue(self.remembered_hue - step, cx);
                        true
                    }
                    "right" | "up" => {
                        self.set_hue(self.remembered_hue + step, cx);
                        true
                    }
                    _ => false,
                }
            }
            ContinuousControl::Alpha => {
                let Some(opacity) = self.current_picker_opacity() else {
                    return;
                };
                let step = if shift {
                    KEYBOARD_COARSE_STEP
                } else {
                    KEYBOARD_FINE_STEP
                };
                let alpha = opacity / 100.;
                match key {
                    "left" | "down" => {
                        self.set_alpha(alpha - step, cx);
                        true
                    }
                    "right" | "up" => {
                        self.set_alpha(alpha + step, cx);
                        true
                    }
                    _ => false,
                }
            }
        };
        if handled {
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    fn handle_hex_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if matches!(event, InputEvent::Change) && self.ignore_next_hex_change {
            self.ignore_next_hex_change = false;
            return;
        }
        if self.dismissal_event_guard
            || self.suppress_input_events
            || self.color_editing_disabled()
            || self.paint.is_none()
        {
            return;
        }
        match event {
            InputEvent::Change => {
                if !self.hex_input.focus_handle(cx).is_focused(window) {
                    return;
                }
                if self.hex_edit_session.is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.hex_edit_session = self.begin_text_input_edit(edit, cx);
                }
                let value = self.hex_input.read(cx).value();
                if let Some(parsed) = self.parse_color_text_input(value.as_ref()) {
                    self.hex_invalid = false;
                    self.preview_color_text_input(parsed, cx);
                } else {
                    self.hex_invalid = true;
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.hex_edit_session.take();
                let candidate = if self.hex_invalid {
                    self.sync_inputs(window, cx, false, true, true);
                    self.ignore_next_hex_change = true;
                    self.hex_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.hex_input.read(cx).value();
                    self.parse_color_text_input(value.as_ref())
                        .and_then(|parsed| self.color_text_terminal_edit(parsed))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.hex_edit_session.take();
                let candidate = if self.hex_invalid {
                    self.sync_inputs(window, cx, false, true, true);
                    self.ignore_next_hex_change = true;
                    self.hex_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.hex_input.read(cx).value();
                    self.parse_color_text_input(value.as_ref())
                        .and_then(|parsed| self.color_text_terminal_edit(parsed))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.hex_edit_session.is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.hex_edit_session = self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    fn color_from_channel_inputs(&self, cx: &App) -> Option<DesignColor> {
        let values = self
            .color_channel_inputs
            .each_ref()
            .map(|input| input.read(cx).value().to_string());
        parse_color_channels(self.color_format, [&values[0], &values[1], &values[2]])
    }

    fn handle_color_channel_input(
        &mut self,
        channel: usize,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if channel < self.ignore_next_color_channel_changes.len()
            && matches!(event, InputEvent::Change)
            && self.ignore_next_color_channel_changes[channel]
        {
            self.ignore_next_color_channel_changes[channel] = false;
            return;
        }
        if self.dismissal_event_guard
            || self.suppress_input_events
            || self.color_editing_disabled()
            || self.paint.is_none()
            || channel >= self.color_channel_inputs.len()
            || matches!(self.color_format, ColorFormat::Hex | ColorFormat::Css)
        {
            return;
        }

        let value = self.color_channel_inputs[channel].read(cx).value();
        self.color_channel_invalid[channel] =
            parse_color_channel_value(self.color_format, channel, value.as_ref()).is_none();

        match event {
            InputEvent::Change => {
                if !self.color_channel_inputs[channel]
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    return;
                }
                if self.color_channel_edit_sessions[channel].is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.color_channel_edit_sessions[channel] =
                        self.begin_text_input_edit(edit, cx);
                }
                if !self.color_channel_invalid.iter().any(|invalid| *invalid)
                    && let Some(color) = self.color_from_channel_inputs(cx)
                {
                    self.edit_rgb_color(color, DesignPanelEditPhase::Preview, cx);
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.color_channel_edit_sessions[channel].take();
                let candidate = if self.color_channel_invalid.iter().any(|invalid| *invalid) {
                    self.sync_inputs(window, cx, true, true, false);
                    self.ignore_next_color_channel_changes = [true; 3];
                    self.color_channel_invalid = [false; 3];
                    cx.notify();
                    None
                } else {
                    self.color_from_channel_inputs(cx)
                        .and_then(|color| self.rgb_color_text_edit(color))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.color_channel_edit_sessions[channel].take();
                let candidate = if self.color_channel_invalid.iter().any(|invalid| *invalid) {
                    self.sync_inputs(window, cx, true, true, false);
                    self.ignore_next_color_channel_changes = [true; 3];
                    self.color_channel_invalid = [false; 3];
                    cx.notify();
                    None
                } else {
                    self.color_from_channel_inputs(cx)
                        .and_then(|color| self.rgb_color_text_edit(color))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.color_channel_edit_sessions[channel].is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.color_channel_edit_sessions[channel] =
                        self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    fn handle_opacity_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if matches!(event, InputEvent::Change) && self.ignore_next_opacity_change {
            self.ignore_next_opacity_change = false;
            return;
        }
        if self.dismissal_event_guard
            || self.suppress_input_events
            || self.opacity_editing_disabled()
            || self.paint.is_none()
        {
            return;
        }
        match event {
            InputEvent::Change => {
                if !self.opacity_input.focus_handle(cx).is_focused(window) {
                    return;
                }
                if self.opacity_edit_session.is_none()
                    && let Some(edit) = self.current_opacity_text_edit()
                {
                    self.opacity_edit_session = self.begin_text_input_edit(edit, cx);
                }
                let value = self.opacity_input.read(cx).value();
                if let Some(opacity) = parse_opacity(value.as_ref()) {
                    self.opacity_invalid = false;
                    self.set_opacity(opacity, DesignPanelEditPhase::Preview, cx);
                } else {
                    self.opacity_invalid = true;
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.opacity_edit_session.take();
                let candidate = if self.opacity_invalid {
                    self.sync_inputs(window, cx, true, false, true);
                    self.ignore_next_opacity_change = true;
                    self.opacity_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.opacity_input.read(cx).value();
                    parse_opacity(value.as_ref()).and_then(|opacity| {
                        let paint = self.paint.as_ref()?;
                        Some(picker_opacity_edit(paint, self.selected_stop, opacity))
                    })
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.opacity_edit_session.take();
                let candidate = if self.opacity_invalid {
                    self.sync_inputs(window, cx, true, false, true);
                    self.ignore_next_opacity_change = true;
                    self.opacity_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.opacity_input.read(cx).value();
                    parse_opacity(value.as_ref()).and_then(|opacity| {
                        let paint = self.paint.as_ref()?;
                        Some(picker_opacity_edit(paint, self.selected_stop, opacity))
                    })
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.opacity_edit_session.is_none()
                    && let Some(edit) = self.current_opacity_text_edit()
                {
                    self.opacity_edit_session = self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    fn handle_stop_position_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if matches!(event, InputEvent::Change) && self.ignore_next_stop_position_change {
            self.ignore_next_stop_position_change = false;
            return;
        }
        if self.dismissal_event_guard || self.suppress_input_events || self.editing_disabled() {
            return;
        }
        match event {
            InputEvent::Change => {
                if !self.stop_position_input.focus_handle(cx).is_focused(window) {
                    return;
                }
                if self.stop_position_edit_session.is_none()
                    && let Some(edit) = self.gradient_stop_position_edit(
                        self.selected_gradient_stop()
                            .map_or(0., |stop| stop.position),
                    )
                {
                    self.stop_position_edit_session = self.begin_text_input_edit(edit, cx);
                }
                let value = self.stop_position_input.read(cx).value();
                if let Some(position) = parse_stop_position(value.as_ref()) {
                    self.stop_position_invalid = false;
                    if let Some(edit) = self.gradient_stop_position_edit(position) {
                        let _ = self.emit_edit(edit, DesignPanelEditPhase::Preview, cx);
                    }
                } else {
                    self.stop_position_invalid = true;
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.stop_position_edit_session.take();
                let candidate = if self.stop_position_invalid {
                    self.sync_inputs(window, cx, true, true, true);
                    self.ignore_next_stop_position_change = true;
                    self.stop_position_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.stop_position_input.read(cx).value();
                    parse_stop_position(value.as_ref())
                        .and_then(|position| self.gradient_stop_position_edit(position))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.stop_position_edit_session.take();
                let candidate = if self.stop_position_invalid {
                    self.sync_inputs(window, cx, true, true, true);
                    self.ignore_next_stop_position_change = true;
                    self.stop_position_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.stop_position_input.read(cx).value();
                    parse_stop_position(value.as_ref())
                        .and_then(|position| self.gradient_stop_position_edit(position))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.stop_position_edit_session.is_none()
                    && let Some(edit) = self.gradient_stop_position_edit(
                        self.selected_gradient_stop()
                            .map_or(0., |stop| stop.position),
                    )
                {
                    self.stop_position_edit_session = self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    fn selected_gradient_stop(&self) -> Option<&DesignGradientStop> {
        self.paint
            .as_ref()?
            .gradient_stops
            .get(clamp_stop_index(self.paint.as_ref()?, self.selected_stop))
    }

    fn gradient_stop_position_edit(&self, position: f32) -> Option<DesignPaintEdit> {
        let stop = self.selected_gradient_stop()?;
        Some(DesignPaintEdit {
            property: DesignPaintProperty::GradientStopPosition {
                stop_id: stop.id.clone(),
                index: self.selected_stop,
            },
            value: DesignPaintValue::Number(position.clamp(0., 1.)),
        })
    }

    fn sync_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        preserve_hex_draft: bool,
        preserve_opacity_draft: bool,
        preserve_color_channel_drafts: bool,
    ) {
        let color_text = self.paint.as_ref().map_or_else(String::new, |paint| {
            color_text_value(self.color_format, paint, self.selected_stop)
        });
        let color_channel_texts = self.current_color().map_or_else(
            || [String::new(), String::new(), String::new()],
            |color| color_channel_values(self.color_format, color),
        );
        let opacity_text = self
            .current_picker_opacity()
            .map_or_else(String::new, format_decimal);
        let stop_position_text = self
            .selected_gradient_stop()
            .map_or_else(String::new, |stop| {
                format_decimal(stop.position.clamp(0., 1.) * 100.)
            });
        let preserve_stop_position_draft =
            self.stop_position_input.focus_handle(cx).is_focused(window);
        let preserve_color_channels = self.color_channel_inputs.each_ref().map(|input| {
            preserve_color_channel_drafts && input.focus_handle(cx).is_focused(window)
        });

        self.suppress_input_events = true;
        if !preserve_hex_draft {
            self.hex_input.update(cx, |input, cx| {
                input.set_value(color_text, window, cx);
            });
        }
        if !preserve_opacity_draft {
            self.opacity_input.update(cx, |input, cx| {
                input.set_value(opacity_text, window, cx);
            });
        }
        for (channel, text) in color_channel_texts.into_iter().enumerate() {
            if !preserve_color_channels[channel] {
                self.color_channel_inputs[channel].update(cx, |input, cx| {
                    input.set_value(text, window, cx);
                });
                self.color_channel_invalid[channel] = false;
            }
        }
        if !preserve_stop_position_draft {
            self.stop_position_input.update(cx, |input, cx| {
                input.set_value(stop_position_text, window, cx);
            });
        }
        self.suppress_input_events = false;
    }

    fn render_creation_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let highlighted_index = self.creation_menu_index;
        let menu_focus_handle = self.creation_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let disabled = self.editing_disabled() || self.selected_color_target().is_none();
        let trigger = Button::new(SharedString::from(format!("{}-create", self.id)))
            .icon(IconName::Plus)
            .tooltip("Create style or variable")
            .xsmall()
            .compact()
            .ghost()
            .disabled(disabled)
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.creation_menu_open {
                        this.close_creation_menu(window, cx);
                    } else {
                        this.open_creation_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!("{}-creation-menu", self.id)))
            .anchor(Anchor::BottomRight)
            .open(self.creation_menu_open)
            .track_focus(&menu_focus_handle)
            .overlay_closable(true)
            .on_open_change(move |open, window, cx| {
                picker_for_open.update(cx, |this, cx| {
                    if *open && !this.editing_disabled() && this.selected_color_target().is_some() {
                        this.creation_menu_open = true;
                        this.creation_menu_index = PaintCreationKind::Style as usize;
                        cx.notify();
                    } else {
                        this.close_creation_menu(window, cx);
                    }
                });
            })
            .trigger(trigger)
            .content(move |_, window, _| {
                v_flex()
                    .id(SharedString::from(format!("{picker_id}-creation-options")))
                    .key_context(CONTROL_KEY_CONTEXT)
                    .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                    .on_action({
                        let picker = picker_for_content.clone();
                        move |_: &ActivateControl, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.commit_creation_menu(window, cx);
                            });
                        }
                    })
                    .on_key_down({
                        let picker = picker_for_content.clone();
                        move |event: &KeyDownEvent, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.handle_creation_menu_key(event, window, cx);
                            });
                        }
                    })
                    .w(popup_width(window, 144.))
                    .gap_1()
                    .children(PaintCreationKind::ALL.into_iter().enumerate().map(
                        |(index, kind)| {
                            let picker = picker_for_content.clone();
                            Button::new(SharedString::from(format!(
                                "{}-{}",
                                picker_id,
                                kind.label().to_lowercase().replace(' ', "-")
                            )))
                            .label(kind.label())
                            .tooltip(kind.label())
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .tab_stop(false)
                            .selected(highlighted_index == index)
                            .on_activate(move |_, window, cx| {
                                picker.update(cx, |this, cx| {
                                    this.activate_creation_kind(kind, window, cx);
                                });
                            })
                        },
                    ))
            })
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(PICKER_HEADER_HEIGHT))
            .flex_none()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new(SharedString::from(format!("{}-custom-tab", self.id)))
                    .label("Custom")
                    .xsmall()
                    .compact()
                    .ghost()
                    .selected(self.active_tab == PaintPickerTab::Custom)
                    .on_activate(cx.listener(|this, _, _, cx| {
                        this.active_tab = PaintPickerTab::Custom;
                        this.scroll_handle.set_offset(Point::default());
                        cx.notify();
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("{}-libraries-tab", self.id)))
                    .label("Libraries")
                    .xsmall()
                    .compact()
                    .ghost()
                    .selected(self.active_tab == PaintPickerTab::Libraries)
                    .on_activate(cx.listener(|this, _, _, cx| {
                        this.active_tab = PaintPickerTab::Libraries;
                        this.shader_browser_requested = this
                            .paint
                            .as_ref()
                            .is_some_and(|paint| paint.paint_type() == DesignPaintType::Shader);
                        this.scroll_handle.set_offset(Point::default());
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(self.render_creation_menu(cx))
            .child(
                Button::new(SharedString::from(format!("{}-close", self.id)))
                    .icon(IconName::Close)
                    .tooltip("Close")
                    .xsmall()
                    .compact()
                    .ghost()
                    .on_activate(|_, window, cx| {
                        window.dispatch_action(Box::new(CancelDesignInteraction), cx);
                    }),
            )
            .into_any_element()
    }

    fn render_color_only_header(&self, title: SharedString, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(PICKER_HEADER_HEIGHT))
            .flex_none()
            .gap_1()
            .px_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().flex_1().text_sm().font_semibold().child(title))
            .child(
                Button::new(SharedString::from(format!("{}-close", self.id)))
                    .icon(IconName::Close)
                    .tooltip("Close")
                    .xsmall()
                    .compact()
                    .ghost()
                    .on_activate(|_, window, cx| {
                        window.dispatch_action(Box::new(CancelDesignInteraction), cx);
                    }),
            )
            .into_any_element()
    }

    fn render_paint_kind_mark(&self, kind: DesignPaintKind, cx: &mut Context<Self>) -> AnyElement {
        let foreground = cx.theme().foreground;
        let faint = cx.theme().muted_foreground.opacity(0.18);
        let mark = div()
            .relative()
            .size(px(15.))
            .overflow_hidden()
            .rounded(px(2.))
            .border_1()
            .border_color(cx.theme().muted_foreground.opacity(0.72));

        match kind {
            DesignPaintKind::Solid => mark.bg(foreground).into_any_element(),
            DesignPaintKind::LinearGradient => mark
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(foreground, 0.),
                    linear_color_stop(faint, 1.),
                ))
                .into_any_element(),
            DesignPaintKind::RadialGradient => mark
                .bg(faint)
                .child(
                    div()
                        .absolute()
                        .left(px(3.))
                        .top(px(3.))
                        .size(px(7.))
                        .rounded(px(7.))
                        .bg(foreground),
                )
                .into_any_element(),
            DesignPaintKind::AngularGradient => mark
                .bg(linear_gradient(
                    45.,
                    linear_color_stop(foreground, 0.),
                    linear_color_stop(faint, 1.),
                ))
                .into_any_element(),
            DesignPaintKind::DiamondGradient => mark
                .bg(linear_gradient(
                    135.,
                    linear_color_stop(faint, 0.),
                    linear_color_stop(foreground, 1.),
                ))
                .into_any_element(),
            DesignPaintKind::Pattern => mark
                .bg(pattern_slash(
                    cx.theme().muted_foreground.opacity(0.75),
                    0.45,
                    0.45,
                ))
                .into_any_element(),
            DesignPaintKind::Image => Icon::new(IconName::GalleryVerticalEnd)
                .small()
                .into_any_element(),
            DesignPaintKind::Video => Icon::new(IconName::File).small().into_any_element(),
            DesignPaintKind::Shader => Icon::new(IconName::Asterisk).small().into_any_element(),
            DesignPaintKind::Unsupported => Icon::new(IconName::Info).small().into_any_element(),
        }
    }

    fn render_type_tabs(&self, paint: &DesignPaint, cx: &mut Context<Self>) -> AnyElement {
        debug_assert!(PICKER_WIDTH >= paint_header_min_width());
        let disabled = self.editing_disabled();
        let mut tabs = h_flex()
            .w_full()
            .h(px(PAINT_TYPE_ROW_HEIGHT))
            .flex_none()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(cx.theme().border);
        for paint_type in DesignPaintType::ALL {
            tabs = tabs.child(
                Button::new(SharedString::from(format!(
                    "{}-type-{}",
                    self.id,
                    paint_type.label().to_lowercase()
                )))
                .tooltip(paint_type.label())
                .xsmall()
                .compact()
                .ghost()
                .w(px(PAINT_HEADER_CONTROL_SIZE))
                .h(px(PAINT_HEADER_CONTROL_SIZE))
                .selected(paint.paint_type() == paint_type)
                .disabled(disabled)
                .child(self.render_paint_kind_mark(
                    if paint_type == DesignPaintType::Gradient && paint.kind.is_gradient() {
                        paint.kind
                    } else {
                        paint_type.default_kind()
                    },
                    cx,
                ))
                .on_activate(cx.listener(move |this, _, _, cx| {
                    this.select_paint_type(paint_type, cx);
                })),
            );
        }
        tabs.child(div().flex_1())
            .child(self.render_blend_mode_control(paint, cx))
            .child(
                Button::new(SharedString::from(format!("{}-contrast", self.id)))
                    .tooltip(
                        self.current_contrast_leaf()
                            .and_then(|leaf| leaf.disabled_reason.clone())
                            .unwrap_or_else(|| "Check WCAG color contrast".into()),
                    )
                    .xsmall()
                    .compact()
                    .ghost()
                    .w(px(PAINT_HEADER_CONTROL_SIZE))
                    .h(px(PAINT_HEADER_CONTROL_SIZE))
                    .selected(self.contrast_checker_open)
                    .disabled(self.current_contrast_leaf().is_none())
                    .child(self.render_contrast_mark(cx))
                    .on_activate(cx.listener(|this, _, _, cx| {
                        this.contrast_checker_open = !this.contrast_checker_open;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_blend_mode_mark(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .size(px(15.))
            .overflow_hidden()
            .rounded(px(15.))
            .border_1()
            .border_color(cx.theme().foreground)
            .child(
                div()
                    .w(px(7.))
                    .h_full()
                    .bg(cx.theme().foreground.opacity(0.72)),
            )
            .into_any_element()
    }

    fn render_contrast_mark(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .relative()
            .size(px(15.))
            .rounded(px(15.))
            .border_1()
            .border_color(cx.theme().foreground)
            .child(
                div()
                    .absolute()
                    .left(px(3.))
                    .top(px(4.))
                    .w(px(8.))
                    .h(px(1.))
                    .bg(cx.theme().foreground),
            )
            .child(
                div()
                    .absolute()
                    .left(px(3.))
                    .top(px(8.))
                    .w(px(8.))
                    .h(px(1.))
                    .bg(cx.theme().foreground),
            )
            .into_any_element()
    }

    fn render_blend_mode_control(&self, paint: &DesignPaint, cx: &mut Context<Self>) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let selected_mode = paint.blend_mode;
        let blend_trigger =
            Button::new(SharedString::from(format!("{}-paint-blend-mode", self.id)))
                .tooltip(format!("Paint blend mode · {}", paint.blend_mode.label()))
                .xsmall()
                .compact()
                .ghost()
                .w(px(PAINT_HEADER_CONTROL_SIZE))
                .h(px(PAINT_HEADER_CONTROL_SIZE))
                .selected(self.blend_mode_menu_open)
                .disabled(self.editing_disabled())
                .child(self.render_blend_mode_mark(cx))
                .on_keyboard_activate(move |_, cx| {
                    picker_for_trigger.update(cx, |this, cx| {
                        this.blend_mode_menu_open = !this.blend_mode_menu_open;
                        if !this.blend_mode_menu_open {
                            this.cancel_blend_mode_preview(cx);
                        }
                        cx.notify();
                    });
                });

        let blend = Popover::new(SharedString::from(format!(
            "{}-paint-blend-mode-menu",
            self.id
        )))
        .anchor(Anchor::BottomRight)
        .open(self.blend_mode_menu_open)
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            picker_for_open.update(cx, |this, cx| {
                this.blend_mode_menu_open = *open;
                if !*open {
                    this.cancel_blend_mode_preview(cx);
                }
                cx.notify();
            });
        })
        .trigger(blend_trigger)
        .content(move |_, window, _| {
            v_flex()
                .id(SharedString::from(format!("{picker_id}-blend-options")))
                .w(popup_width(window, 164.))
                .max_h(popup_height(window, 320.))
                .overflow_y_scroll()
                .gap_1()
                .children(DesignBlendMode::NON_PASS_THROUGH.into_iter().map(|mode| {
                    let picker = picker_for_content.clone();
                    let picker_for_key = picker.clone();
                    let picker_for_hover = picker.clone();
                    Button::new(SharedString::from(format!(
                        "{}-paint-blend-mode-{}",
                        picker_id,
                        mode.label().to_lowercase().replace(' ', "-")
                    )))
                    .label(mode.label())
                    .tooltip(mode.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .selected(selected_mode == mode)
                    .on_hover(move |hovered, _, cx| {
                        picker_for_hover.update(cx, |this, cx| {
                            this.set_blend_mode_preview(mode, *hovered, cx);
                        });
                    })
                    .on_activate(move |_, _, cx| {
                        picker.update(cx, |this, cx| {
                            this.select_blend_mode(mode, cx);
                        });
                    })
                    // Enter/Space activate through the bound `ActivateControl`
                    // command above; only dismissal and focus-move previews
                    // stay key-matched.
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        picker_for_key.update(cx, |this, cx| {
                            this.set_blend_mode_preview(mode, true, cx);
                            match event.keystroke.key.as_str() {
                                "escape" => {
                                    this.cancel_blend_mode_preview(cx);
                                    this.blend_mode_menu_open = false;
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                                "tab" => this.cancel_blend_mode_preview(cx),
                                _ => {}
                            }
                        });
                    })
                }))
        })
        .w(px(PAINT_HEADER_CONTROL_SIZE))
        .h(px(PAINT_HEADER_CONTROL_SIZE));

        blend.into_any_element()
    }

    fn render_paint_tools(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.contrast_checker_open
            .then(|| self.render_contrast_checker(cx))
            .flatten()
    }

    fn render_contrast_checker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let leaf = self.current_contrast_leaf()?.clone();
        let paint = self.paint.as_ref()?;
        let foreground = picker_color_with_opacity(paint, self.selected_stop);
        let category = self.resolved_contrast_category()?;
        let level = self.normalized_contrast_level();
        let threshold = contrast_threshold(category, level)?;
        let passes = leaf.ratio + f32::EPSILON >= threshold;
        let correction = leaf.correction(category, level);
        let correction_disabled = passes
            || correction.is_none()
            || leaf.disabled_reason.is_some()
            || self.color_editing_disabled();
        let pass_color = if passes {
            cx.theme().green
        } else {
            cx.theme().red
        };

        let mut category_controls = h_flex().w_full().gap_1().flex_wrap();
        for option in DesignColorContrastCategory::ALL {
            category_controls = category_controls.child(
                Button::new(SharedString::from(format!(
                    "{}-contrast-category-{}",
                    self.id,
                    option.label().to_ascii_lowercase().replace(' ', "-")
                )))
                .label(option.label())
                .tooltip(match option {
                    DesignColorContrastCategory::Auto => {
                        SharedString::from(format!("Auto · {}", leaf.automatic_category.label()))
                    }
                    _ => SharedString::from(option.label()),
                })
                .xsmall()
                .compact()
                .ghost()
                .selected(self.contrast_category == option)
                .on_activate(cx.listener(move |this, _, _, cx| {
                    this.contrast_category = option;
                    if option == DesignColorContrastCategory::Graphics {
                        this.contrast_level = DesignColorContrastLevel::Aa;
                    }
                    cx.notify();
                })),
            );
        }

        let mut level_controls = h_flex().gap_1();
        for option in DesignColorContrastLevel::ALL {
            let unavailable = category == DesignColorContrastCategory::Graphics
                && option == DesignColorContrastLevel::Aaa;
            level_controls = level_controls.child(
                Button::new(SharedString::from(format!(
                    "{}-contrast-level-{}",
                    self.id,
                    option.label().to_ascii_lowercase()
                )))
                .label(option.label())
                .tooltip(if unavailable {
                    "AAA applies to text only"
                } else {
                    option.label()
                })
                .xsmall()
                .compact()
                .ghost()
                .selected(level == option)
                .disabled(unavailable)
                .on_activate(cx.listener(move |this, _, _, cx| {
                    if this.resolved_contrast_category()
                        != Some(DesignColorContrastCategory::Graphics)
                        || option == DesignColorContrastLevel::Aa
                    {
                        this.contrast_level = option;
                        cx.notify();
                    }
                })),
            );
        }

        Some(
            v_flex()
                .w_full()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(div().text_xs().font_semibold().child("Contrast"))
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_semibold()
                                        .child(format!("{:.2}:1", leaf.ratio)),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "{}-contrast-correct",
                                        self.id
                                    )))
                                    .label(if passes { "✓" } else { "!" })
                                    .tooltip(leaf.disabled_reason.clone().unwrap_or_else(|| {
                                        if passes {
                                            "Contrast passes".into()
                                        } else if correction.is_some() {
                                            "Adjust to the nearest passing color".into()
                                        } else {
                                            "No host-supplied correction".into()
                                        }
                                    }))
                                    .xsmall()
                                    .compact()
                                    .ghost()
                                    .text_color(pass_color)
                                    .disabled(correction_disabled)
                                    .on_activate(
                                        cx.listener(|this, _, _, cx| {
                                            this.apply_contrast_correction(cx);
                                        }),
                                    ),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .size(px(24.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(color_to_hsla(foreground)),
                        )
                        .child(div().text_xs().child("Selected layer"))
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("on"),
                        )
                        .child(
                            div()
                                .size(px(24.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(color_to_hsla(leaf.effective_background)),
                        ),
                )
                .child(category_controls)
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(level_controls)
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(pass_color)
                                .child(if passes { "Pass" } else { "Fail" }),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} · {} requires {:.1}:1",
                            category.label(),
                            level.label(),
                            threshold
                        )),
                )
                .into_any_element(),
        )
    }

    fn apply_contrast_correction(&mut self, cx: &mut Context<Self>) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(category) = self.resolved_contrast_category() else {
            return;
        };
        let level = self.normalized_contrast_level();
        let Some(color) = self
            .current_contrast_leaf()
            .filter(|leaf| {
                leaf.disabled_reason.is_none()
                    && contrast_threshold(category, level)
                        .is_some_and(|threshold| leaf.ratio < threshold)
            })
            .and_then(|leaf| leaf.correction(category, level))
        else {
            return;
        };
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let color = rgb_edit_color(paint, self.selected_stop, color);
        let _ = self.emit_edit(
            color_edit(paint, self.selected_stop, color),
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    fn add_gradient_stop_at(&mut self, position: f32, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let (candidate, selected_stop) = paint_with_added_stop_at(paint, position);
        self.selected_stop = selected_stop;
        self.selected_stop_id = candidate.gradient_stops[selected_stop].id.clone();
        let stop = candidate.gradient_stops[selected_stop].clone();
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientStopAdd,
                value: DesignPaintValue::GradientStop(stop),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    fn begin_gradient_stop_drag(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() {
            return;
        }
        self.select_gradient_stop(index, window, cx);
        self.dragging_stop = Some(index);
        if let Some(edit) = self.gradient_stop_position_edit(
            self.selected_gradient_stop()
                .map_or(0., |stop| stop.position),
        ) {
            self.begin_continuous_edit(edit, cx);
        }
    }

    fn update_gradient_stop_from_pointer(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.dragging_stop else {
            return;
        };
        self.selected_stop = index;
        let position = fraction_x(self.gradient_bounds, position);
        if let Some(edit) = self.gradient_stop_position_edit(position) {
            self.preview_continuous_edit(edit, cx);
        }
    }

    fn finish_gradient_stop_drag(&mut self, cx: &mut Context<Self>) {
        self.dragging_stop = None;
        self.commit_continuous_edit(cx);
    }

    fn transform_gradient(&self, operation: GradientTransformOperation, cx: &mut Context<Self>) {
        let Some(DesignPaintPayload::Gradient(gradient)) =
            self.paint.as_ref().map(|paint| &paint.payload)
        else {
            return;
        };
        let transform = match operation {
            GradientTransformOperation::RotateClockwise90 => gradient.transform.rotated(90.),
            GradientTransformOperation::FlipHorizontal => gradient.transform.flipped_horizontal(),
        };
        let _ = self.emit_edit(
            DesignPaintEdit {
                property: DesignPaintProperty::GradientTransform,
                value: DesignPaintValue::Transform(transform),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    }

    fn current_gradient_kind_index(&self) -> usize {
        self.paint
            .as_ref()
            .and_then(|paint| GRADIENT_KINDS.iter().position(|kind| *kind == paint.kind))
            .unwrap_or(0)
    }

    fn open_gradient_kind_menu_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled()
            || !self
                .paint
                .as_ref()
                .is_some_and(|paint| paint.kind.is_gradient())
        {
            return;
        }
        self.gradient_kind_menu_index = self.current_gradient_kind_index();
        self.gradient_kind_menu_return_focus = window.focused(cx);
        self.gradient_kind_menu_open = true;
        let focus_handle = self.gradient_kind_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    fn close_gradient_kind_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.gradient_kind_menu_open = false;
        if let Some(return_focus) = self.gradient_kind_menu_return_focus.take() {
            window.defer(cx, move |window, cx| {
                return_focus.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn choose_gradient_kind(
        &mut self,
        kind: DesignPaintKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if GRADIENT_KINDS.contains(&kind) {
            self.select_paint_kind(kind, cx);
        }
        self.close_gradient_kind_menu(window, cx);
    }

    fn handle_gradient_kind_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = GRADIENT_KINDS.len();
        match event.keystroke.key.as_str() {
            "up" => {
                self.gradient_kind_menu_index = (self.gradient_kind_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.gradient_kind_menu_index = (self.gradient_kind_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.gradient_kind_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.gradient_kind_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_gradient_kind_menu(window, cx),
            "escape" => self.close_gradient_kind_menu(window, cx),
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted gradient kind; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    fn commit_gradient_kind_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = GRADIENT_KINDS.len();
        self.choose_gradient_kind(
            GRADIENT_KINDS[self.gradient_kind_menu_index.min(count - 1)],
            window,
            cx,
        );
    }

    fn render_gradient_preview(&self, paint: &DesignPaint, cx: &mut Context<Self>) -> AnyElement {
        let mut preview = div()
            .id(SharedString::from(format!("{}-gradient-preview", self.id)))
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(36.))
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(pattern_slash(
                cx.theme().muted_foreground.opacity(0.18),
                0.35,
                0.35,
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.editing_disabled() && this.dragging_stop.is_none() {
                        this.add_gradient_stop_at(
                            fraction_x(this.gradient_bounds, event.position),
                            cx,
                        );
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_gradient_stop_from_pointer(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.finish_gradient_stop_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.finish_gradient_stop_drag(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "delete" | "backspace") {
                    this.remove_gradient_stop(cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }));

        for (left, right, left_color, right_color) in gradient_segments(paint) {
            preview = preview.child(
                div()
                    .absolute()
                    .left(relative(left))
                    .w(relative((right - left).max(0.0001)))
                    .h_full()
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(color_to_hsla(left_color), 0.),
                        linear_color_stop(color_to_hsla(right_color), 1.),
                    )),
            );
        }

        for (index, stop) in paint.gradient_stops.iter().enumerate() {
            let selected = index == self.selected_stop;
            preview = preview.child(
                div()
                    .id(SharedString::from(format!(
                        "{}-gradient-handle-{index}",
                        self.id
                    )))
                    .absolute()
                    .left(relative(stop.position.clamp(0., 1.)))
                    .ml(px(-5.))
                    .bottom(px(2.))
                    .size(px(10.))
                    .rounded(px(2.))
                    .border_2()
                    .border_color(if selected {
                        cx.theme().selection
                    } else {
                        cx.theme().background
                    })
                    .bg(color_to_hsla(stop.color))
                    .cursor_col_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.begin_gradient_stop_drag(index, window, cx);
                        }),
                    ),
            );
        }

        preview
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.gradient_bounds = bounds;
            }))
            .into_any_element()
    }

    fn render_gradient_kind_selector(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let highlighted_index = self
            .gradient_kind_menu_index
            .min(GRADIENT_KINDS.len().saturating_sub(1));
        let menu_focus_handle = self.gradient_kind_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let disabled = self.editing_disabled();
        let trigger = Button::new(SharedString::from(format!("{}-gradient-kind", self.id)))
            .label(paint.kind.label())
            .dropdown_caret(true)
            .tooltip("Gradient type")
            .xsmall()
            .compact()
            .outline()
            .flex_1()
            .min_w(px(0.))
            .disabled(disabled)
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.gradient_kind_menu_open {
                        this.close_gradient_kind_menu(window, cx);
                    } else {
                        this.open_gradient_kind_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!(
            "{}-gradient-kind-menu",
            self.id
        )))
        .anchor(Anchor::BottomLeft)
        .open(self.gradient_kind_menu_open)
        .track_focus(&menu_focus_handle)
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            picker_for_open.update(cx, |this, cx| {
                if *open
                    && !this.editing_disabled()
                    && this
                        .paint
                        .as_ref()
                        .is_some_and(|paint| paint.kind.is_gradient())
                {
                    this.gradient_kind_menu_index = this.current_gradient_kind_index();
                    this.gradient_kind_menu_open = true;
                    cx.notify();
                } else {
                    this.close_gradient_kind_menu(window, cx);
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, _| {
            v_flex()
                .id(SharedString::from(format!(
                    "{picker_id}-gradient-kind-options"
                )))
                .key_context(CONTROL_KEY_CONTEXT)
                .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                .on_action({
                    let picker = picker_for_content.clone();
                    move |_: &ActivateControl, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.commit_gradient_kind_menu(window, cx);
                        });
                    }
                })
                .on_key_down({
                    let picker = picker_for_content.clone();
                    move |event: &KeyDownEvent, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.handle_gradient_kind_menu_key(event, window, cx);
                        });
                    }
                })
                .w(popup_width(window, 152.))
                .gap_1()
                .children(GRADIENT_KINDS.into_iter().enumerate().map(|(index, kind)| {
                    let picker = picker_for_content.clone();
                    Button::new(SharedString::from(format!(
                        "{picker_id}-gradient-kind-{}",
                        kind.label().to_lowercase()
                    )))
                    .label(kind.label())
                    .tooltip(kind.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .tab_stop(false)
                    .selected(highlighted_index == index)
                    .on_activate(move |_, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.choose_gradient_kind(kind, window, cx);
                        });
                    })
                }))
        })
        .into_any_element()
    }

    fn render_gradient_controls(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !paint.kind.is_gradient() {
            return None;
        }
        let disabled = self.editing_disabled();

        let mut stops = h_flex().w_full().gap_1().flex_wrap();
        for (index, stop) in paint.gradient_stops.iter().enumerate() {
            let selected = index == self.selected_stop;
            stops = stops.child(
                Button::new(SharedString::from(format!("{}-stop-{index}", self.id)))
                    .xsmall()
                    .compact()
                    .outline()
                    .disabled(disabled)
                    .on_activate(cx.listener(move |this, _, window, cx| {
                        this.select_gradient_stop(index, window, cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "delete" | "backspace") {
                            this.remove_gradient_stop(cx);
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .size(px(12.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(color_to_hsla(stop.color)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .child(format!("{}%", (stop.position * 100.).round() as i32)),
                            ),
                    )
                    .when(selected, |button| button.border_color(cx.theme().selection)),
            );
        }

        Some(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .child(self.render_gradient_kind_selector(paint, cx))
                        .child(
                            Button::new(SharedString::from(format!("{}-gradient-flip", self.id)))
                                .label("↔")
                                .tooltip("Flip gradient")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.transform_gradient(
                                        GradientTransformOperation::FlipHorizontal,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{}-gradient-rotate", self.id)))
                                .icon(IconName::Redo2)
                                .tooltip("Rotate gradient 90°")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.transform_gradient(
                                        GradientTransformOperation::RotateClockwise90,
                                        cx,
                                    );
                                })),
                        ),
                )
                .child(self.render_gradient_preview(paint, cx))
                .child(stops)
                .child(
                    h_flex().w_full().gap_2().child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Stop position"),
                            )
                            .child(
                                Input::new(&self.stop_position_input)
                                    .suffix(div().text_xs().child("%"))
                                    .xsmall()
                                    .h(px(26.))
                                    .disabled(disabled)
                                    .when(self.stop_position_invalid, |input| {
                                        input.border_color(cx.theme().red)
                                    }),
                            ),
                    ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("{}-add-stop", self.id)))
                                .label("Add stop")
                                .icon(IconName::Plus)
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.add_gradient_stop(cx);
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{}-remove-stop", self.id)))
                                .label("Remove")
                                .icon(IconName::Minus)
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(disabled || paint.gradient_stops.len() <= 2)
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.remove_gradient_stop(cx);
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_color_area(&self, color: DesignColor, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.color_editing_disabled();
        let hsv = self.current_hsv().unwrap_or_default();
        let hue_color = hsv_to_color(
            Hsv {
                hue: self.remembered_hue,
                saturation: 1.,
                value: 1.,
            },
            u8::MAX,
        );
        let id = SharedString::from(format!("{}-color-area", self.id));

        div()
            .id(id)
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(COLOR_AREA_HEIGHT))
            .overflow_hidden()
            .rounded(px(7.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(color_to_hsla(hue_color))
            .cursor_crosshair()
            .focus(|style| style.border_color(cx.theme().selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.color_editing_disabled()
                        && let (Some(paint), Some(color)) =
                            (this.paint.as_ref(), this.current_color())
                    {
                        this.begin_continuous_edit(
                            color_edit(paint, this.selected_stop, color),
                            cx,
                        );
                    }
                    this.update_from_pointer(ContinuousControl::ColorArea, event.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_from_pointer(ContinuousControl::ColorArea, event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_continuous_key(ContinuousControl::ColorArea, event, window, cx);
            }))
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(gpui::white(), 0.),
                linear_color_stop(gpui::transparent_white(), 1.),
            )))
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(gpui::transparent_black(), 0.),
                linear_color_stop(gpui::black(), 1.),
            )))
            .children(self.render_contrast_boundary(cx))
            .child(
                div()
                    .absolute()
                    .left(relative(hsv.saturation))
                    .top(relative(1. - hsv.value))
                    .ml(px(-6.))
                    .mt(px(-6.))
                    .size(px(12.))
                    .rounded(px(6.))
                    .border_2()
                    .border_color(cx.theme().background)
                    .shadow_sm()
                    .bg(color_to_hsla(color)),
            )
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.color_area_bounds = bounds;
            }))
            .when(disabled, |area| area.opacity(0.56))
            .into_any_element()
    }

    fn render_contrast_boundary(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.contrast_checker_open {
            return None;
        }
        let leaf = self.current_contrast_leaf()?;
        let category = self.resolved_contrast_category()?;
        let level = self.normalized_contrast_level();
        let threshold = contrast_threshold(category, level)?;
        let foreground_alpha =
            picker_color_with_opacity(self.paint.as_ref()?, self.selected_stop).alpha;
        let hue = self.remembered_hue;
        let background = leaf.effective_background;
        let stroke = if wcag_relative_luminance(opaque_color(background)) > 0.45 {
            gpui::black().opacity(0.78)
        } else {
            gpui::white().opacity(0.86)
        };

        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    const SATURATION_STEPS: usize = 48;
                    let width = f32::from(bounds.size.width);
                    let height = f32::from(bounds.size.height);
                    if width <= 0. || height <= 0. {
                        return;
                    }
                    let mut previous = Vec::<f32>::new();
                    let mut path = gpui::PathBuilder::stroke(px(1.15));
                    for step in 0..=SATURATION_STEPS {
                        let saturation = step as f32 / SATURATION_STEPS as f32;
                        let crossings = contrast_value_crossings(
                            hue,
                            saturation,
                            foreground_alpha,
                            background,
                            threshold,
                        );
                        let x = bounds.origin.x + px(width * saturation);
                        for (branch, value) in crossings.iter().copied().enumerate() {
                            let y = bounds.origin.y + px(height * (1. - value));
                            if let Some(previous_value) = previous.get(branch).copied() {
                                let previous_saturation =
                                    (step.saturating_sub(1)) as f32 / SATURATION_STEPS as f32;
                                let previous_x = bounds.origin.x + px(width * previous_saturation);
                                let previous_y =
                                    bounds.origin.y + px(height * (1. - previous_value));
                                path.move_to(point(previous_x, previous_y));
                                path.line_to(point(x, y));
                            }
                        }
                        previous = crossings;
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, stroke);
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }

    fn render_hue_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.color_editing_disabled();
        let colors = [
            DesignColor::rgb(0xff, 0x00, 0x00),
            DesignColor::rgb(0xff, 0xff, 0x00),
            DesignColor::rgb(0x00, 0xff, 0x00),
            DesignColor::rgb(0x00, 0xff, 0xff),
            DesignColor::rgb(0x00, 0x00, 0xff),
            DesignColor::rgb(0xff, 0x00, 0xff),
            DesignColor::rgb(0xff, 0x00, 0x00),
        ];
        let mut spectrum = h_flex().absolute().size_full();
        for pair in colors.windows(2) {
            spectrum = spectrum.child(div().h_full().flex_1().bg(linear_gradient(
                90.,
                linear_color_stop(color_to_hsla(pair[0]), 0.),
                linear_color_stop(color_to_hsla(pair[1]), 1.),
            )));
        }

        div()
            .id(SharedString::from(format!("{}-hue", self.id)))
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(CONTROL_HEIGHT))
            .overflow_hidden()
            .rounded(px(CONTROL_HEIGHT / 2.))
            .border_1()
            .border_color(cx.theme().border)
            .focus(|style| style.border_color(cx.theme().selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.color_editing_disabled()
                        && let (Some(paint), Some(color)) =
                            (this.paint.as_ref(), this.current_color())
                    {
                        this.begin_continuous_edit(
                            color_edit(paint, this.selected_stop, color),
                            cx,
                        );
                    }
                    this.update_from_pointer(ContinuousControl::Hue, event.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_from_pointer(ContinuousControl::Hue, event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_continuous_key(ContinuousControl::Hue, event, window, cx);
            }))
            .child(spectrum)
            .child(
                div()
                    .absolute()
                    .left(relative(self.remembered_hue / 360.))
                    .ml(px(-5.))
                    .top(px(3.))
                    .size(px(10.))
                    .rounded(px(5.))
                    .border_2()
                    .border_color(cx.theme().background)
                    .shadow_sm(),
            )
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.hue_bounds = bounds;
            }))
            .when(disabled, |control| control.opacity(0.56))
            .into_any_element()
    }

    fn render_alpha_control(&self, color: DesignColor, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.opacity_editing_disabled();
        let mut transparent = color;
        transparent.alpha = 0;
        let mut opaque = color;
        opaque.alpha = u8::MAX;
        let alpha = self.current_picker_opacity().unwrap_or(100.) / 100.;
        let mut thumb_color = color;
        thumb_color.alpha = channel_from_unit(alpha);

        div()
            .id(SharedString::from(format!("{}-alpha", self.id)))
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(CONTROL_HEIGHT))
            .overflow_hidden()
            .rounded(px(CONTROL_HEIGHT / 2.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(pattern_slash(
                cx.theme().muted_foreground.opacity(0.22),
                0.35,
                0.35,
            ))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.opacity_editing_disabled()
                        && let Some(edit) = this.paint.as_ref().map(|paint| {
                            picker_opacity_edit(
                                paint,
                                this.selected_stop,
                                picker_opacity(paint, this.selected_stop),
                            )
                        })
                    {
                        this.begin_continuous_edit(edit, cx);
                    }
                    this.update_from_pointer(ContinuousControl::Alpha, event.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_from_pointer(ContinuousControl::Alpha, event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_continuous_key(ContinuousControl::Alpha, event, window, cx);
            }))
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(color_to_hsla(transparent), 0.),
                linear_color_stop(color_to_hsla(opaque), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .left(relative(alpha))
                    .ml(px(-5.))
                    .top(px(3.))
                    .size(px(10.))
                    .rounded(px(5.))
                    .border_2()
                    .border_color(cx.theme().background)
                    .shadow_sm()
                    .bg(color_to_hsla(thumb_color)),
            )
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.alpha_bounds = bounds;
            }))
            .when(disabled, |control| control.opacity(0.56))
            .into_any_element()
    }

    fn render_color_format_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let selected_format = self.color_format;
        let highlighted_index = self.color_format_menu_index;
        let menu_focus_handle = self.color_format_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let trigger = Button::new(SharedString::from(format!("{}-color-format", self.id)))
            .label(selected_format.label())
            .dropdown_caret(true)
            .tooltip("Color format")
            .xsmall()
            .compact()
            .outline()
            .w(px(62.))
            .h(px(26.))
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.color_format_menu_open {
                        this.close_color_format_menu(window, cx);
                    } else {
                        this.open_color_format_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!("{}-color-format-menu", self.id)))
            .anchor(Anchor::BottomLeft)
            .open(self.color_format_menu_open)
            .track_focus(&menu_focus_handle)
            .overlay_closable(true)
            .on_open_change(move |open, window, cx| {
                picker_for_open.update(cx, |this, cx| {
                    if *open {
                        this.color_format_menu_open = true;
                        this.color_format_menu_index = this.color_format.index();
                        cx.notify();
                    } else {
                        this.close_color_format_menu(window, cx);
                    }
                });
            })
            .trigger(trigger)
            .content(move |_, window, _| {
                v_flex()
                    .id(SharedString::from(format!(
                        "{picker_id}-color-format-options"
                    )))
                    .key_context(CONTROL_KEY_CONTEXT)
                    .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                    .on_action({
                        let picker = picker_for_content.clone();
                        move |_: &ActivateControl, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.commit_color_format_menu(window, cx);
                            });
                        }
                    })
                    .on_key_down({
                        let picker = picker_for_content.clone();
                        move |event: &KeyDownEvent, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.handle_color_format_menu_key(event, window, cx);
                            });
                        }
                    })
                    .w(popup_width(window, 88.))
                    .gap_1()
                    .children(
                        ColorFormat::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(index, format)| {
                                let picker = picker_for_content.clone();
                                Button::new(SharedString::from(format!(
                                    "{}-color-format-{}",
                                    picker_id,
                                    format.label().to_lowercase()
                                )))
                                .label(format.label())
                                .tooltip(format.label())
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .tab_stop(false)
                                .selected(highlighted_index == index)
                                .when(selected_format == format, |button| {
                                    button.child(Icon::new(IconName::Check).xsmall())
                                })
                                .on_activate(
                                    move |_, window, cx| {
                                        picker.update(cx, |this, cx| {
                                            this.set_color_format(format, window, cx);
                                        });
                                    },
                                )
                            }),
                    )
            })
            .w(px(62.))
            .h(px(26.))
            .into_any_element()
    }

    fn render_color_channel_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        let labels = self.color_format.channel_labels();
        let disabled = self.color_editing_disabled();
        let mut fields = h_flex().flex_1().min_w(px(0.)).gap_1();
        for (channel, label) in labels.into_iter().enumerate() {
            let percentage =
                matches!(self.color_format, ColorFormat::Hsl | ColorFormat::Hsb) && channel > 0;
            fields = fields.child(
                Input::new(&self.color_channel_inputs[channel])
                    .prefix(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .when(percentage, |input| input.suffix(div().text_xs().child("%")))
                    .xsmall()
                    .h(px(26.))
                    .flex_1()
                    .min_w(px(0.))
                    .disabled(disabled)
                    .when(self.color_channel_invalid[channel], |input| {
                        input.border_color(cx.theme().red)
                    }),
            );
        }
        fields.into_any_element()
    }

    fn render_color_value_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let color_disabled = self.color_editing_disabled();
        let opacity_disabled = self.opacity_editing_disabled();
        let opacity = Input::new(&self.opacity_input)
            .suffix(div().text_xs().child("%"))
            .xsmall()
            .h(px(26.))
            .w(px(62.))
            .disabled(opacity_disabled)
            .when(self.opacity_invalid, |input| {
                input.border_color(cx.theme().red)
            });

        if matches!(self.color_format, ColorFormat::Hex | ColorFormat::Css) {
            let controls = h_flex()
                .w_full()
                .gap_1()
                .child(self.render_color_format_selector(cx))
                .child(
                    Input::new(&self.hex_input)
                        .xsmall()
                        .h(px(26.))
                        .flex_1()
                        .min_w(px(0.))
                        .disabled(color_disabled)
                        .when(self.hex_invalid, |input| input.border_color(cx.theme().red)),
                );
            return controls
                .when(self.color_format == ColorFormat::Hex, |controls| {
                    controls.child(opacity)
                })
                .into_any_element();
        }

        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(self.render_color_format_selector(cx))
                    .child(self.render_color_channel_fields(cx)),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Opacity"),
                    )
                    .child(opacity),
            )
            .into_any_element()
    }

    fn render_color_editor(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let color = self.current_color()?;
        Some(
            v_flex()
                .w_full()
                .gap_2()
                .child(self.render_color_area(color, cx))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("{}-eyedropper", self.id)))
                                .icon(IconName::Inspector)
                                .tooltip("Pick color from canvas")
                                .xsmall()
                                .compact()
                                .ghost()
                                .w(px(24.))
                                .disabled(self.color_editing_disabled())
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.request_eyedropper(cx);
                                })),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_2()
                                .child(self.render_hue_control(cx))
                                .child(self.render_alpha_control(color, cx)),
                        ),
                )
                .child(self.render_color_value_controls(cx))
                .into_any_element(),
        )
    }

    fn render_color_style_sample_swatch(
        &self,
        sample: &DesignColorStyleSample,
        selection: DesignColorStyleSampleSelection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(format!(
            "{}-page-color-style-sample-{}",
            self.id, sample.id
        )))
        .tooltip(format!("Color style: {}", sample.name))
        .xsmall()
        .compact()
        .ghost()
        .w(px(28.))
        .h(px(28.))
        .disabled(
            self.color_editing_disabled()
                || self.selected_color_target().is_none()
                || sample.disabled_reason.is_some(),
        )
        .child(
            div()
                .relative()
                .size(px(20.))
                .overflow_hidden()
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(pattern_slash(
                    cx.theme().muted_foreground.opacity(0.18),
                    0.45,
                    0.45,
                ))
                .child(div().absolute().size_full().bg(color_to_hsla(sample.color))),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_style_sample(selection.clone(), cx);
        }))
        .into_any_element()
    }

    fn render_color_style_sample_row(
        &self,
        sample: &DesignColorStyleSample,
        selection: DesignColorStyleSampleSelection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(format!(
            "{}-library-color-style-sample-{}",
            self.id, sample.id
        )))
        .w_full()
        .h(px(28.))
        .px_2()
        .compact()
        .ghost()
        .disabled(
            self.color_editing_disabled()
                || self.selected_color_target().is_none()
                || sample.disabled_reason.is_some(),
        )
        .tooltip(
            sample
                .disabled_reason
                .clone()
                .unwrap_or_else(|| "Sample this Color style without binding it".into()),
        )
        .child(
            h_flex()
                .w_full()
                .min_w(px(0.))
                .gap_2()
                .child(
                    div()
                        .size(px(18.))
                        .flex_none()
                        .rounded(px(9.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(color_to_hsla(sample.color)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_left()
                        .child(sample.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Style"),
                ),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_style_sample(selection.clone(), cx);
        }))
        .into_any_element()
    }

    fn render_color_variable_row(
        &self,
        variable: &DesignVariable,
        selected_binding: Option<&DesignPaintBinding>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = paint_variable_matches_binding(variable, selected_binding);
        let disabled = self.editing_disabled()
            || self.selected_color_target().is_none()
            || variable.disabled_reason.is_some();
        let variable_id = variable.id.clone();
        let source_id = match &variable.source {
            DesignVariableSource::Page { page_id, .. } => page_id,
            DesignVariableSource::Library { library_id, .. } => library_id,
        };
        let color = paint_variable_color(variable);
        let label = if variable.import_state == DesignVariableImportState::Available {
            format!("Import {}", variable.name)
        } else {
            variable.name.to_string()
        };
        Button::new(SharedString::from(format!(
            "{}-paint-variable-{}-{}",
            self.id, source_id, variable.id
        )))
        .w_full()
        .h(px(28.))
        .px_2()
        .compact()
        .ghost()
        .selected(selected)
        .disabled(disabled)
        .tooltip(
            variable
                .disabled_reason
                .clone()
                .unwrap_or_else(|| variable.collection_name.clone()),
        )
        .child(
            h_flex()
                .w_full()
                .min_w(px(0.))
                .gap_2()
                .child(
                    div()
                        .size(px(18.))
                        .flex_none()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(color_to_hsla(color)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_left()
                        .child(label),
                )
                .child(
                    div()
                        .max_w(px(74.))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(variable.collection_name.clone()),
                )
                .when(selected, |row| {
                    row.child(
                        Icon::new(IconName::Check)
                            .xsmall()
                            .text_color(cx.theme().foreground),
                    )
                }),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_variable_apply(variable_id.clone(), cx);
        }))
        .into_any_element()
    }

    fn render_color_variable_swatch(
        &self,
        variable: &DesignVariable,
        selected_binding: Option<&DesignPaintBinding>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = paint_variable_matches_binding(variable, selected_binding);
        let disabled = self.editing_disabled()
            || self.selected_color_target().is_none()
            || variable.disabled_reason.is_some();
        let color = paint_variable_color(variable);
        let variable_id = variable.id.clone();
        Button::new(SharedString::from(format!(
            "{}-page-paint-variable-swatch-{}",
            self.id, variable.id
        )))
        .tooltip(format!("{} — {}", variable.name, variable.collection_name))
        .xsmall()
        .compact()
        .ghost()
        .w(px(28.))
        .h(px(28.))
        .selected(selected)
        .disabled(disabled)
        .child(
            div()
                .relative()
                .size(px(20.))
                .overflow_hidden()
                .rounded(px(4.))
                .border_1()
                .border_color(if selected {
                    cx.theme().selection
                } else {
                    cx.theme().border
                })
                .bg(pattern_slash(
                    cx.theme().muted_foreground.opacity(0.18),
                    0.45,
                    0.45,
                ))
                .child(div().absolute().size_full().bg(color_to_hsla(color)))
                .when(selected, |swatch| {
                    swatch.child(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::Check)
                                    .xsmall()
                                    .text_color(cx.theme().foreground),
                            ),
                    )
                }),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_variable_apply(variable_id.clone(), cx);
        }))
        .into_any_element()
    }

    fn render_color_variable_detach_button(
        &self,
        scope: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(format!(
            "{}-detach-{}-paint-variable",
            self.id, scope
        )))
        .label("Detach current variable")
        .xsmall()
        .compact()
        .ghost()
        .w_full()
        .disabled(self.editing_disabled())
        .on_activate(cx.listener(|this, _, _, cx| {
            this.request_color_variable_detach(cx);
        }))
        .into_any_element()
    }

    fn render_resource_scope_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let scopes = self.resource_scopes();
        let highlighted_index = self
            .resource_scope_menu_index
            .min(scopes.len().saturating_sub(1));
        let menu_focus_handle = self.resource_scope_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let trigger = Button::new(SharedString::from(format!("{}-resource-scope", self.id)))
            .label(self.resource_scope.label())
            .dropdown_caret(true)
            .tooltip("Choose a page or library palette")
            .xsmall()
            .compact()
            .outline()
            .w_full()
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.resource_scope_menu_open {
                        this.close_resource_scope_menu(window, cx);
                    } else {
                        this.open_resource_scope_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!(
            "{}-resource-scope-menu",
            self.id
        )))
        .anchor(Anchor::BottomLeft)
        .open(self.resource_scope_menu_open)
        .track_focus(&menu_focus_handle)
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            picker_for_open.update(cx, |this, cx| {
                if *open {
                    this.normalize_resource_scope();
                    this.resource_scope_menu_open = true;
                    cx.notify();
                } else {
                    this.close_resource_scope_menu(window, cx);
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, _| {
            v_flex()
                .id(SharedString::from(format!(
                    "{picker_id}-resource-scope-options"
                )))
                .key_context(CONTROL_KEY_CONTEXT)
                .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                .on_action({
                    let picker = picker_for_content.clone();
                    move |_: &ActivateControl, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.commit_resource_scope_menu(window, cx);
                        });
                    }
                })
                .on_key_down({
                    let picker = picker_for_content.clone();
                    move |event: &KeyDownEvent, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.handle_resource_scope_menu_key(event, window, cx);
                        });
                    }
                })
                .w(popup_width(window, 216.))
                .max_h(popup_height(window, 280.))
                .overflow_y_scroll()
                .gap_1()
                .children(scopes.iter().cloned().enumerate().map(|(index, scope)| {
                    let picker = picker_for_content.clone();
                    let option_id = match &scope {
                        PaintResourceScope::Page => "page".to_owned(),
                        PaintResourceScope::Library { library_id, .. } => {
                            format!("library-{library_id}")
                        }
                    };
                    Button::new(SharedString::from(format!(
                        "{picker_id}-resource-scope-{option_id}"
                    )))
                    .label(scope.label())
                    .tooltip(scope.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .tab_stop(false)
                    .selected(highlighted_index == index)
                    .on_activate(move |_, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.select_resource_scope(scope.clone(), window, cx);
                        });
                    })
                }))
        })
        .into_any_element()
    }

    fn render_variable_scope(&self, paint: &DesignPaint, cx: &mut Context<Self>) -> AnyElement {
        let selected_binding = selected_color_binding(paint, self.selected_stop);
        let scoped_style_samples = match &self.resource_scope {
            PaintResourceScope::Page => self
                .color_style_sample_view_data
                .page_samples
                .iter()
                .collect::<Vec<_>>(),
            PaintResourceScope::Library { library_id, .. } => self
                .color_style_sample_view_data
                .libraries
                .iter()
                .find(|library| library.id == *library_id)
                .map_or_else(Vec::new, |library| library.samples.iter().collect()),
        };
        let scoped_variables = self
            .paint_variable_view_data
            .variables
            .iter()
            .filter(|variable| match (&self.resource_scope, &variable.source) {
                (PaintResourceScope::Page, DesignVariableSource::Page { .. }) => true,
                (
                    PaintResourceScope::Library { library_id, .. },
                    DesignVariableSource::Library {
                        library_id: source_id,
                        ..
                    },
                ) => library_id == source_id,
                (PaintResourceScope::Page, DesignVariableSource::Library { .. })
                | (PaintResourceScope::Library { .. }, DesignVariableSource::Page { .. }) => false,
            })
            .collect::<Vec<_>>();
        let mut style_samples = h_flex().w_full().gap_2().flex_wrap();
        if scoped_style_samples.is_empty() {
            style_samples = style_samples.child(
                div()
                    .w_full()
                    .min_h(px(24.))
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No Color styles in this source"),
            );
        } else {
            for sample in scoped_style_samples {
                let selection = match &self.resource_scope {
                    PaintResourceScope::Page => {
                        DesignColorStyleSampleSelection::page(sample.id.clone())
                    }
                    PaintResourceScope::Library { library_id, .. } => {
                        DesignColorStyleSampleSelection::library(
                            library_id.clone(),
                            sample.id.clone(),
                        )
                    }
                };
                style_samples = style_samples
                    .child(self.render_color_style_sample_swatch(sample, selection, cx));
            }
        }
        let mut variables = h_flex().w_full().gap_2().flex_wrap();
        if scoped_variables.is_empty() {
            variables = variables.child(
                div()
                    .w_full()
                    .min_h(px(24.))
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No Color variables in this source"),
            );
        } else {
            for variable in scoped_variables {
                variables = variables.child(self.render_color_variable_swatch(
                    variable,
                    selected_binding,
                    cx,
                ));
            }
        }

        v_flex()
            .w_full()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(self.render_resource_scope_selector(cx))
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color styles"),
            )
            .child(style_samples)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color variables"),
            )
            .child(variables)
            .when(selected_binding.is_some(), |scope| {
                scope.child(self.render_color_variable_detach_button("page", cx))
            })
            .into_any_element()
    }

    fn render_variables(&self, paint: &DesignPaint, cx: &mut Context<Self>) -> AnyElement {
        let selected_binding = selected_color_binding(paint, self.selected_stop);
        let query = self
            .resource_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let style_libraries = self
            .color_style_sample_view_data
            .libraries
            .iter()
            .filter_map(|library| {
                let samples = library
                    .samples
                    .iter()
                    .filter(|sample| {
                        paint_resource_matches(
                            &query,
                            [sample.name.as_ref(), library.name.as_ref()],
                        )
                    })
                    .collect::<Vec<_>>();
                (!samples.is_empty()).then_some((library, samples))
            })
            .collect::<Vec<_>>();
        let mut libraries: Vec<(SharedString, SharedString, Vec<&DesignVariable>)> = Vec::new();
        for variable in &self.paint_variable_view_data.variables {
            let DesignVariableSource::Library {
                library_id,
                library_name,
            } = &variable.source
            else {
                continue;
            };
            if !paint_resource_matches(
                &query,
                [
                    variable.name.as_ref(),
                    variable.collection_name.as_ref(),
                    library_name.as_ref(),
                ],
            ) {
                continue;
            }
            if let Some((_, _, variables)) =
                libraries.iter_mut().find(|(id, _, _)| id == library_id)
            {
                variables.push(variable);
            } else {
                libraries.push((library_id.clone(), library_name.clone(), vec![variable]));
            }
        }
        let mut content = v_flex()
            .w_full()
            .gap_3()
            .p_4()
            .child(Input::new(&self.resource_search_input).xsmall().h(px(28.)));
        if selected_binding.is_some() {
            content = content.child(self.render_color_variable_detach_button("library", cx));
        }
        if libraries.is_empty() && style_libraries.is_empty() {
            let message = if query.is_empty() {
                "No Color styles or variables supplied by the host."
            } else {
                "No styles or variables match this search."
            };
            return content
                .w_full()
                .min_h(px(180.))
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::BookOpen)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().text_sm().font_semibold().child("Color libraries"))
                .child(
                    div()
                        .text_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(message),
                )
                .into_any_element();
        }

        if !style_libraries.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color styles"),
            );
        }
        for (library, samples) in style_libraries {
            let mut group = v_flex().w_full().gap_1().child(
                h_flex()
                    .w_full()
                    .h(px(24.))
                    .gap_2()
                    .child(
                        Icon::new(IconName::BookOpen)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(library.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(samples.len().to_string()),
                    ),
            );
            for sample in samples {
                group = group.child(self.render_color_style_sample_row(
                    sample,
                    DesignColorStyleSampleSelection::library(library.id.clone(), sample.id.clone()),
                    cx,
                ));
            }
            content = content.child(group);
        }

        if !libraries.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color variables"),
            );
        }
        for (_, library_name, variables) in libraries {
            let count = variables.len();
            let mut group = v_flex().w_full().gap_1().child(
                h_flex()
                    .w_full()
                    .h(px(24.))
                    .gap_2()
                    .child(
                        Icon::new(IconName::BookOpen)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(library_name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(count.to_string()),
                    ),
            );
            for variable in variables {
                group = group.child(self.render_color_variable_row(variable, selected_binding, cx));
            }
            content = content.child(group);
        }
        content.into_any_element()
    }

    fn render_shader_row(
        &self,
        shader: &DesignShaderDefinition,
        selection: DesignShaderSelection,
        selected_shader_id: Option<&SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = selected_shader_id.is_some_and(|shader_id| *shader_id == shader.id);
        let selection_for_click = selection.clone();
        h_flex()
            .w_full()
            .min_h(px(34.))
            .gap_2()
            .px_2()
            .rounded(px(6.))
            .when(selected, |row| row.bg(cx.theme().accent.opacity(0.12)))
            .child(
                Icon::new(IconName::Asterisk)
                    .xsmall()
                    .text_color(if selected {
                        cx.theme().accent
                    } else {
                        cx.theme().muted_foreground
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(shader.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if shader.imported {
                                format!(
                                    "Imported · {} properties",
                                    shader.property_definitions.len()
                                )
                            } else {
                                "Available · import required".to_owned()
                            }),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "{}-shader-{}-{}",
                    self.id,
                    if shader.imported { "apply" } else { "import" },
                    shader.id
                )))
                .label(if shader.imported { "Apply" } else { "Import" })
                .xsmall()
                .compact()
                .outline()
                .disabled(self.editing_disabled() || (selected && shader.imported))
                .on_activate(cx.listener(move |this, _, _, cx| {
                    this.request_shader_selection(selection_for_click.clone(), cx);
                })),
            )
            .into_any_element()
    }

    fn render_shader_browser(&self, paint: &DesignPaint, cx: &mut Context<Self>) -> AnyElement {
        let selected_shader_id = match &paint.payload {
            DesignPaintPayload::Shader(shader) => Some(&shader.shader_id),
            _ => None,
        };
        if self.shader_view_data.page_shaders.is_empty()
            && self.shader_view_data.libraries.is_empty()
        {
            return v_flex()
                .w_full()
                .min_h(px(180.))
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .child(
                    Icon::new(IconName::Asterisk)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().text_sm().font_semibold().child("Shaders"))
                .child(
                    div()
                        .text_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No fill shaders supplied by the host."),
                )
                .into_any_element();
        }

        let mut content = v_flex().w_full().gap_3().p_4();
        if !self.shader_view_data.page_shaders.is_empty() {
            let mut page = v_flex()
                .w_full()
                .gap_1()
                .child(div().text_xs().font_semibold().child("In this file"));
            for shader in &self.shader_view_data.page_shaders {
                page = page.child(self.render_shader_row(
                    shader,
                    DesignShaderSelection::page(shader.id.clone()),
                    selected_shader_id,
                    cx,
                ));
            }
            content = content.child(page);
        }
        for library in &self.shader_view_data.libraries {
            let library_id = library.id.clone();
            let mut group = v_flex().w_full().gap_1().child(
                h_flex()
                    .w_full()
                    .h(px(24.))
                    .gap_2()
                    .child(
                        Icon::new(IconName::BookOpen)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(library.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(library.shaders.len().to_string()),
                    ),
            );
            for shader in &library.shaders {
                group = group.child(self.render_shader_row(
                    shader,
                    DesignShaderSelection::library(library_id.clone(), shader.id.clone()),
                    selected_shader_id,
                    cx,
                ));
            }
            content = content.child(group);
        }
        content.into_any_element()
    }

    fn request_source_replace(&self, cx: &mut Context<Self>) {
        if self.source_replacement_disabled() {
            return;
        }
        if !self
            .paint
            .as_ref()
            .is_some_and(|paint| matches!(&paint.payload, DesignPaintPayload::Pattern(_)))
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::SourceReplaceRequested { target });
    }

    fn request_media_source_action(&self, action: DesignMediaSourceAction, cx: &mut Context<Self>) {
        if self.media_source_action_disabled(action) {
            return;
        }
        let (Some(target), Some(paint)) = (self.target.clone(), self.paint.as_ref()) else {
            return;
        };
        let source_id = match &paint.payload {
            DesignPaintPayload::Image(image) => image.source.id.clone(),
            DesignPaintPayload::Video(video) => video.source.id.clone(),
            _ => return,
        };
        cx.emit(PaintPickerEvent::MediaSourceActionRequested {
            target,
            source_id,
            action,
        });
    }

    fn request_media_source_drop(&self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if self.base_editing_disabled() {
            return;
        }
        let (Some(target), Some(paint)) = (self.target.clone(), self.paint.as_ref()) else {
            return;
        };
        let capabilities = self.current_media_capabilities();
        let Some(file) = media_drop_from_paths(paths, capabilities) else {
            return;
        };
        let (expected_media_kind, expected_source_id) = match &paint.payload {
            DesignPaintPayload::Image(image) => (DesignMediaKind::Image, image.source.id.clone()),
            DesignPaintPayload::Video(video) => (DesignMediaKind::Video, video.source.id.clone()),
            _ => return,
        };
        cx.emit(PaintPickerEvent::MediaSourceDropRequested {
            target,
            expected_source_id,
            expected_media_kind,
            file,
        });
    }

    fn current_shader_definition(&self) -> Option<&DesignShaderDefinition> {
        let DesignPaintPayload::Shader(shader) = &self.paint.as_ref()?.payload else {
            return None;
        };
        self.shader_view_data.definition(&shader.shader_id)
    }

    fn request_shader_selection(&self, shader: DesignShaderSelection, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(definition)) =
            (self.target.clone(), self.shader_view_data.shader(&shader))
        else {
            return;
        };
        if definition.imported {
            cx.emit(PaintPickerEvent::ShaderApplyRequested { target, shader });
        } else {
            cx.emit(PaintPickerEvent::ShaderImportRequested { target, shader });
        }
    }

    fn request_shader_property_bind(&self, definition_id: SharedString, cx: &mut Context<Self>) {
        if self.editing_disabled()
            || self
                .current_shader_definition()
                .is_none_or(|shader| shader.property(&definition_id).is_none())
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::ShaderPropertyBindRequested {
            target,
            definition_id,
        });
    }

    fn request_shader_property_editor(&self, definition_id: SharedString, cx: &mut Context<Self>) {
        if self.editing_disabled()
            || self
                .current_shader_definition()
                .is_none_or(|shader| shader.property(&definition_id).is_none())
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::ShaderPropertyEditorRequested {
            target,
            definition_id,
        });
    }

    fn request_shader_property_detach(
        &self,
        definition_id: SharedString,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() {
            return;
        }
        let Some(DesignPaintPayload::Shader(shader)) =
            self.paint.as_ref().map(|paint| &paint.payload)
        else {
            return;
        };
        if shader
            .property(&definition_id)
            .and_then(DesignShaderPropertyValue::variable_alias_id)
            .is_none_or(|current| current != &variable_id)
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::ShaderPropertyDetachRequested {
            target,
            definition_id,
            variable_id,
        });
    }

    fn request_media_crop_action(&self, action: DesignMediaCropAction, cx: &mut Context<Self>) {
        if self.editing_disabled()
            || !self.paint.as_ref().is_some_and(|paint| {
                matches!(
                    &paint.payload,
                    DesignPaintPayload::Image(DesignImagePaint {
                        placement: DesignMediaPaintPlacement::Crop { .. },
                        ..
                    }) | DesignPaintPayload::Video(DesignVideoPaint {
                        placement: DesignMediaPaintPlacement::Crop { .. },
                        ..
                    })
                )
            })
        {
            return;
        }
        let valid = match &action {
            DesignMediaCropAction::Preview {
                transform, zoom, ..
            }
            | DesignMediaCropAction::Commit {
                transform, zoom, ..
            } => media_transform_is_finite(*transform) && zoom.is_finite() && *zoom > 0.,
            DesignMediaCropAction::Begin
            | DesignMediaCropAction::Cancel
            | DesignMediaCropAction::ResizeToFit => true,
        };
        if !valid {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::MediaCropActionRequested { target, action });
    }

    fn current_video_preview(&self) -> Option<&DesignVideoPreviewState> {
        self.current_media_view()?.video_preview.as_ref()
    }

    fn request_video_preview_action(
        &self,
        action: DesignVideoPreviewAction,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            self.paint.as_ref().map(|paint| &paint.payload),
            Some(DesignPaintPayload::Video(_))
        ) || !matches!(
            self.current_video_preview().map(|preview| &preview.status),
            Some(DesignVideoPreviewStatus::Ready)
        ) {
            return;
        }
        let valid = match action {
            DesignVideoPreviewAction::Play | DesignVideoPreviewAction::Pause => true,
            DesignVideoPreviewAction::Seek { seconds }
            | DesignVideoPreviewAction::Scrub { seconds, .. } => {
                seconds.is_finite() && seconds >= 0.
            }
        };
        if !valid {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::VideoPreviewActionRequested { target, action });
    }

    fn request_video_scrub(&self, seconds: f32, cx: &mut Context<Self>) {
        for phase in [
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Commit,
        ] {
            self.request_video_preview_action(
                DesignVideoPreviewAction::Scrub { seconds, phase },
                cx,
            );
        }
    }

    fn render_shader_property(
        &self,
        shader: &super::DesignShaderPaint,
        definition: &DesignShaderPropertyDefinition,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = shader
            .property(&definition.id)
            .or(definition.default_value.as_ref());
        let summary = value.map_or_else(|| "No value".into(), DesignShaderPropertyValue::summary);
        let disabled = self.editing_disabled();
        let definition_id = definition.id.clone();
        let variable_id = value
            .and_then(DesignShaderPropertyValue::variable_alias_id)
            .cloned();

        let value_control = match value {
            Some(DesignShaderPropertyValue::Boolean(current)) if variable_id.is_none() => {
                let next = !current;
                Button::new(SharedString::from(format!(
                    "{}-shader-property-{}-boolean",
                    self.id, definition.id
                )))
                .label(if *current { "On" } else { "Off" })
                .xsmall()
                .compact()
                .outline()
                .disabled(disabled)
                .on_activate({
                    let definition_id = definition_id.clone();
                    cx.listener(move |this, _, _, cx| {
                        let _ = this.emit_edit(
                            DesignPaintEdit {
                                property: DesignPaintProperty::ShaderProperty {
                                    definition_id: definition_id.clone(),
                                },
                                value: DesignPaintValue::ShaderProperty(
                                    DesignShaderPropertyValue::Boolean(next),
                                ),
                            },
                            DesignPanelEditPhase::Commit,
                            cx,
                        );
                    })
                })
                .into_any_element()
            }
            Some(DesignShaderPropertyValue::Number(current))
                if variable_id.is_none() && current.is_finite() =>
            {
                let next = *current + 0.25;
                Button::new(SharedString::from(format!(
                    "{}-shader-property-{}-number",
                    self.id, definition.id
                )))
                .label(format_decimal(*current))
                .xsmall()
                .compact()
                .outline()
                .disabled(disabled)
                .on_activate({
                    let definition_id = definition_id.clone();
                    cx.listener(move |this, _, _, cx| {
                        let _ = this.emit_edit(
                            DesignPaintEdit {
                                property: DesignPaintProperty::ShaderProperty {
                                    definition_id: definition_id.clone(),
                                },
                                value: DesignPaintValue::ShaderProperty(
                                    DesignShaderPropertyValue::Number(next),
                                ),
                            },
                            DesignPanelEditPhase::Commit,
                            cx,
                        );
                    })
                })
                .into_any_element()
            }
            _ => Button::new(SharedString::from(format!(
                "{}-shader-property-{}-edit",
                self.id, definition.id
            )))
            .label(summary)
            .xsmall()
            .compact()
            .outline()
            .disabled(disabled || variable_id.is_some())
            .on_activate({
                let definition_id = definition_id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.request_shader_property_editor(definition_id.clone(), cx);
                })
            })
            .into_any_element(),
        };

        let variable_control = if let Some(variable_id) = variable_id {
            Button::new(SharedString::from(format!(
                "{}-shader-property-{}-detach",
                self.id, definition.id
            )))
            .label("Detach")
            .tooltip(format!("Detach variable {variable_id}"))
            .xsmall()
            .compact()
            .ghost()
            .disabled(disabled)
            .on_activate({
                let definition_id = definition_id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.request_shader_property_detach(
                        definition_id.clone(),
                        variable_id.clone(),
                        cx,
                    );
                })
            })
            .into_any_element()
        } else {
            Button::new(SharedString::from(format!(
                "{}-shader-property-{}-bind",
                self.id, definition.id
            )))
            .label("Bind")
            .xsmall()
            .compact()
            .ghost()
            .disabled(disabled)
            .on_activate({
                let definition_id = definition_id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.request_shader_property_bind(definition_id.clone(), cx);
                })
            })
            .into_any_element()
        };

        v_flex()
            .w_full()
            .gap_1()
            .py_1()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .font_semibold()
                                    .child(definition.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {}",
                                        definition.kind.label(),
                                        definition.id
                                    )),
                            ),
                    )
                    .child(value_control)
                    .child(variable_control),
            )
            .when_some(definition.description.clone(), |row, description| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                )
            })
            .into_any_element()
    }

    fn render_shader_state(
        &self,
        shader: &super::DesignShaderPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut content = v_flex().w_full().gap_2().child(
            h_flex()
                .w_full()
                .p_2()
                .gap_2()
                .rounded(px(7.))
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    Icon::new(IconName::Asterisk)
                        .small()
                        .text_color(cx.theme().accent),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_semibold()
                                .child(shader.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(shader.shader_id.clone()),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("{}-choose-shader", self.id)))
                        .label("Choose")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(self.editing_disabled())
                        .on_activate(cx.listener(|this, _, _, cx| {
                            this.active_tab = PaintPickerTab::Libraries;
                            this.scroll_handle.set_offset(Point::default());
                            cx.notify();
                        })),
                ),
        );

        let Some(definition) = self.shader_view_data.definition(&shader.shader_id) else {
            return content
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Shader metadata is unavailable. The host still preserves every assignment.",
                        ),
                )
                .into_any_element();
        };
        if definition.property_definitions.is_empty() {
            return content
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Import metadata is pending; no property definitions are available.",
                        ),
                )
                .into_any_element();
        }
        content = content.child(div().text_xs().font_semibold().child("Properties"));
        for property in &definition.property_definitions {
            content = content.child(self.render_shader_property(shader, property, cx));
        }
        content.into_any_element()
    }

    fn render_source_state(&self, paint: &DesignPaint, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.editing_disabled();
        let (title, source_name, icon) = match &paint.payload {
            DesignPaintPayload::Pattern(pattern) => (
                "Pattern fill",
                pattern.source_node_id.clone(),
                IconName::LayoutDashboard,
            ),
            DesignPaintPayload::Image(image) => (
                "Image fill",
                image.source.name.clone(),
                IconName::GalleryVerticalEnd,
            ),
            DesignPaintPayload::Video(video) => {
                ("Video fill", video.source.name.clone(), IconName::File)
            }
            DesignPaintPayload::Shader(shader) => {
                return self.render_shader_state(shader, cx);
            }
            DesignPaintPayload::Unsupported(opaque) => {
                return v_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(opaque.type_name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("This payload is preserved losslessly and is read-only here."),
                    )
                    .into_any_element();
            }
            DesignPaintPayload::Solid(_) | DesignPaintPayload::Gradient(_) => {
                return div().into_any_element();
            }
        };

        let pattern_source = matches!(&paint.payload, DesignPaintPayload::Pattern(_));
        let media_source = matches!(
            &paint.payload,
            DesignPaintPayload::Image(_) | DesignPaintPayload::Video(_)
        );
        let media_capabilities = self.current_media_capabilities();
        let media_drop_enabled =
            media_source && !self.base_editing_disabled() && media_capabilities.can_upload_source;
        let media_drop_background = cx.theme().selection.opacity(0.18);
        let media_drop_border = cx.theme().selection;
        let source_card = h_flex()
            .w_full()
            .p_2()
            .gap_2()
            .rounded(px(7.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(if paint.paint_type() == DesignPaintType::Pattern {
                pattern_slash(cx.theme().muted_foreground.opacity(0.18), 0.5, 0.5)
            } else {
                cx.theme().secondary.into()
            })
            .child(
                Icon::new(icon)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                v_flex()
                    .min_w(px(0.))
                    .flex_1()
                    .child(div().text_sm().font_semibold().child(title))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(source_name),
                    ),
            )
            .when(pattern_source, |source| {
                source.child(
                    Button::new(SharedString::from(format!("{}-replace-source", self.id)))
                        .label("Replace")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(self.source_replacement_disabled())
                        .on_activate(cx.listener(|this, _, _, cx| {
                            this.request_source_replace(cx);
                        })),
                )
            })
            .when(media_drop_enabled, |source| {
                source
                    .can_drop(move |candidate, _, _| {
                        candidate
                            .downcast_ref::<ExternalPaths>()
                            .and_then(|paths| {
                                media_drop_from_paths(paths.paths(), media_capabilities)
                            })
                            .is_some()
                    })
                    .drag_over::<ExternalPaths>(move |style, paths, _, _| {
                        if media_drop_from_paths(paths.paths(), media_capabilities).is_some() {
                            style
                                .bg(media_drop_background)
                                .border_color(media_drop_border)
                        } else {
                            style
                        }
                    })
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                        this.request_media_source_drop(paths.paths(), cx);
                    }))
            });
        let mut content = v_flex().w_full().gap_2().child(source_card);
        if media_source {
            content = content.child(self.render_media_source_actions(paint, cx));
        }

        match &paint.payload {
            DesignPaintPayload::Pattern(pattern) => {
                let mut modes = h_flex().w_full().gap_1();
                for tile_type in DesignPatternTileType::ALL {
                    modes = modes.child(
                        Button::new(SharedString::from(format!(
                            "{}-pattern-mode-{}",
                            self.id,
                            tile_type.label().to_lowercase().replace(' ', "-")
                        )))
                        .label(tile_type.label())
                        .xsmall()
                        .compact()
                        .ghost()
                        .flex_1()
                        .selected(pattern.tile_type == tile_type)
                        .disabled(disabled)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::PatternTileType,
                                        value: DesignPaintValue::PatternTileType(tile_type),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    );
                }
                let mut alignment = h_flex().w_full().gap_1();
                for option in DesignPatternHorizontalAlignment::ALL {
                    alignment = alignment.child(
                        Button::new(SharedString::from(format!(
                            "{}-pattern-align-{}",
                            self.id,
                            option.label().to_lowercase()
                        )))
                        .label(option.label())
                        .xsmall()
                        .compact()
                        .ghost()
                        .flex_1()
                        .selected(pattern.horizontal_alignment == option)
                        .disabled(disabled)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::PatternHorizontalAlignment,
                                        value: DesignPaintValue::PatternHorizontalAlignment(option),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    );
                }
                content = content
                    .child(modes)
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                h_flex().w_full().gap_1().child(
                                    Button::new(SharedString::from(format!(
                                        "{}-pattern-scale",
                                        self.id
                                    )))
                                    .label(format!(
                                        "Scale {}%",
                                        format_decimal(pattern.scaling_factor * 100.)
                                    ))
                                    .xsmall()
                                    .compact()
                                    .outline()
                                    .flex_1()
                                    .disabled(disabled)
                                    .on_activate({
                                        let value = pattern.scaling_factor + 0.1;
                                        cx.listener(move |this, _, _, cx| {
                                            let _ = this.emit_edit(
                                                DesignPaintEdit {
                                                    property:
                                                        DesignPaintProperty::PatternScalingFactor,
                                                    value: DesignPaintValue::Number(value),
                                                },
                                                DesignPanelEditPhase::Commit,
                                                cx,
                                            );
                                        })
                                    }),
                                ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "{}-pattern-spacing-x",
                                            self.id
                                        )))
                                        .label(format!(
                                            "Space X {}",
                                            format_decimal(pattern.spacing.x)
                                        ))
                                        .xsmall()
                                        .compact()
                                        .outline()
                                        .flex_1()
                                        .disabled(disabled)
                                        .on_activate({
                                            let spacing = DesignPatternSpacing::new(
                                                pattern.spacing.x + 0.01,
                                                pattern.spacing.y,
                                            );
                                            cx.listener(move |this, _, _, cx| {
                                                let _ = this.emit_edit(
                                                    DesignPaintEdit {
                                                        property:
                                                            DesignPaintProperty::PatternSpacing,
                                                        value: DesignPaintValue::PatternSpacing(
                                                            spacing,
                                                        ),
                                                    },
                                                    DesignPanelEditPhase::Commit,
                                                    cx,
                                                );
                                            })
                                        }),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "{}-pattern-spacing-y",
                                            self.id
                                        )))
                                        .label(format!(
                                            "Space Y {}",
                                            format_decimal(pattern.spacing.y)
                                        ))
                                        .xsmall()
                                        .compact()
                                        .outline()
                                        .flex_1()
                                        .disabled(disabled)
                                        .on_activate({
                                            let spacing = DesignPatternSpacing::new(
                                                pattern.spacing.x,
                                                pattern.spacing.y + 0.01,
                                            );
                                            cx.listener(move |this, _, _, cx| {
                                                let _ = this.emit_edit(
                                                    DesignPaintEdit {
                                                        property:
                                                            DesignPaintProperty::PatternSpacing,
                                                        value: DesignPaintValue::PatternSpacing(
                                                            spacing,
                                                        ),
                                                    },
                                                    DesignPanelEditPhase::Commit,
                                                    cx,
                                                );
                                            })
                                        }),
                                    ),
                            ),
                    )
                    .child(alignment);
            }
            DesignPaintPayload::Image(image) => {
                content = content.child(self.render_media_settings(
                    MediaSettingsView {
                        placement: image.placement,
                        filters: image.filters,
                        crop_tool: self.current_media_view().map_or_else(
                            || DesignMediaCropToolState {
                                transform: image.placement.crop_transform().unwrap_or_default(),
                                ..DesignMediaCropToolState::default()
                            },
                            |view| view.crop_tool,
                        ),
                    },
                    cx,
                ));
            }
            DesignPaintPayload::Video(video) => {
                content = content
                    .child(self.render_media_settings(
                        MediaSettingsView {
                            placement: video.placement,
                            filters: video.filters,
                            crop_tool: self.current_media_view().map_or_else(
                                || DesignMediaCropToolState {
                                    transform: video.placement.crop_transform().unwrap_or_default(),
                                    ..DesignMediaCropToolState::default()
                                },
                                |view| view.crop_tool,
                            ),
                        },
                        cx,
                    ))
                    .child(self.render_video_preview(cx));
            }
            _ => {}
        }

        content.into_any_element()
    }

    fn render_media_source_actions(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let paint_type = paint.paint_type();
        let mut actions = v_flex().w_full().gap_1();
        for action in DesignMediaSourceAction::ALL {
            if !action.is_applicable_to(paint_type) {
                continue;
            }
            let mut button = Button::new(SharedString::from(format!(
                "{}-media-source-{}",
                self.id,
                action.slug()
            )))
            .label(action.label())
            .tooltip(action.label())
            .xsmall()
            .compact()
            .w_full()
            .disabled(self.media_source_action_disabled(action))
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.request_media_source_action(action, cx);
            }));
            button = if action == DesignMediaSourceAction::Upload {
                button.primary()
            } else {
                button.outline()
            };
            actions = actions.child(button);
        }
        actions.into_any_element()
    }

    fn render_media_settings(
        &self,
        settings: MediaSettingsView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.editing_disabled();
        let current_mode = settings.placement.mode();
        let rotation = settings.placement.rotation().unwrap_or_default();
        let tile_scale = settings.placement.tile_scaling_factor().unwrap_or(1.);
        let filters = settings.filters;
        let crop_tool = settings.crop_tool;
        let mut modes = h_flex().w_full().gap_1();
        for mode in DesignMediaPaintScaleMode::ALL {
            modes = modes.child(
                Button::new(SharedString::from(format!(
                    "{}-media-mode-{}",
                    self.id,
                    mode.label().to_lowercase()
                )))
                .label(mode.label())
                .xsmall()
                .compact()
                .ghost()
                .flex_1()
                .selected(current_mode == mode)
                .disabled(disabled)
                .on_activate(cx.listener(move |this, _, _, cx| {
                    let _ = this.emit_edit(
                        DesignPaintEdit {
                            property: DesignPaintProperty::MediaScaleMode,
                            value: DesignPaintValue::MediaScaleMode(mode),
                        },
                        DesignPanelEditPhase::Commit,
                        cx,
                    );
                })),
            );
        }
        v_flex()
            .w_full()
            .gap_2()
            .child(modes)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-rotation", self.id)))
                            .label(format!("Rotation {}°", rotation.degrees()))
                            .xsmall()
                            .compact()
                            .outline()
                            .disabled(disabled || current_mode == DesignMediaPaintScaleMode::Crop)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaQuarterTurn,
                                        value: DesignPaintValue::MediaQuarterTurn(
                                            rotation.rotated_clockwise(),
                                        ),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-tile-scale", self.id)))
                            .label(format!("Tile {}%", format_decimal(tile_scale * 100.)))
                            .xsmall()
                            .compact()
                            .outline()
                            .disabled(disabled || current_mode != DesignMediaPaintScaleMode::Tile)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaTileScalingFactor,
                                        value: DesignPaintValue::Number(tile_scale + 0.1),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            })),
                    ),
            )
            .when(
                current_mode == DesignMediaPaintScaleMode::Crop,
                |settings| settings.child(self.render_crop_tool(crop_tool, disabled, cx)),
            )
            .child(self.render_image_filter_rows(filters, disabled, cx))
            .into_any_element()
    }

    fn render_crop_tool(
        &self,
        crop_tool: DesignMediaCropToolState,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !crop_tool.active {
            return Button::new(SharedString::from(format!("{}-media-crop", self.id)))
                .label("Crop image")
                .xsmall()
                .compact()
                .outline()
                .w_full()
                .disabled(disabled)
                .on_activate(cx.listener(|this, _, _, cx| {
                    this.request_media_crop_action(DesignMediaCropAction::Begin, cx);
                }))
                .into_any_element();
        }

        let mut nudged_transform = crop_tool.transform;
        nudged_transform.tx += 0.05;
        let next_zoom = (crop_tool.zoom + 0.1).min(64.);
        let next_aspect_ratio = crop_tool.aspect_ratio.next();
        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-nudge", self.id)))
                            .label("Nudge crop")
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Preview {
                                        transform: nudged_transform,
                                        zoom: crop_tool.zoom,
                                        aspect_ratio: crop_tool.aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-zoom", self.id)))
                            .label(format!("Zoom {}×", format_decimal(crop_tool.zoom)))
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Preview {
                                        transform: crop_tool.transform,
                                        zoom: next_zoom,
                                        aspect_ratio: crop_tool.aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-aspect", self.id)))
                            .label(format!("Ratio {}", crop_tool.aspect_ratio.label()))
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Preview {
                                        transform: crop_tool.transform,
                                        zoom: crop_tool.zoom,
                                        aspect_ratio: next_aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-rotate", self.id)))
                            .label("Rotate 15°")
                            .xsmall()
                            .compact()
                            .outline()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(rotated_crop_preview(crop_tool), cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-crop-resize-to-fit",
                            self.id
                        )))
                        .label("Resize to fit")
                        .xsmall()
                        .compact()
                        .outline()
                        .flex_1()
                        .disabled(disabled)
                        .on_activate(cx.listener(|this, _, _, cx| {
                            this.request_media_crop_action(DesignMediaCropAction::ResizeToFit, cx);
                        })),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-cancel", self.id)))
                            .label("Cancel")
                            .xsmall()
                            .compact()
                            .ghost()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(|this, _, _, cx| {
                                this.request_media_crop_action(DesignMediaCropAction::Cancel, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-media-crop-apply", self.id)))
                            .label("Apply")
                            .xsmall()
                            .compact()
                            .primary()
                            .flex_1()
                            .disabled(disabled)
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.request_media_crop_action(
                                    DesignMediaCropAction::Commit {
                                        transform: crop_tool.transform,
                                        zoom: crop_tool.zoom,
                                        aspect_ratio: crop_tool.aspect_ratio,
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_image_filter_rows(
        &self,
        filters: DesignImageFilters,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows = v_flex().w_full().gap_1();
        for filter in DesignImageFilter::ALL {
            let value = filters.value(filter);
            let slug = filter.label().to_ascii_lowercase();
            rows = rows.child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(div().w(px(76.)).text_xs().child(filter.label()))
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-filter-{slug}-minus",
                            self.id
                        )))
                        .label("−")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(disabled || value <= -1.)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaFilter(filter),
                                        value: DesignPaintValue::Number(value - 0.1),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-filter-{slug}-reset",
                            self.id
                        )))
                        .label(format!("{:+.0}", value * 100.))
                        .tooltip("Reset")
                        .xsmall()
                        .compact()
                        .ghost()
                        .w(px(46.))
                        .disabled(disabled)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaFilter(filter),
                                        value: DesignPaintValue::Number(0.),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-media-filter-{slug}-plus",
                            self.id
                        )))
                        .label("+")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(disabled || value >= 1.)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                let _ = this.emit_edit(
                                    DesignPaintEdit {
                                        property: DesignPaintProperty::MediaFilter(filter),
                                        value: DesignPaintValue::Number(value + 0.1),
                                    },
                                    DesignPanelEditPhase::Commit,
                                    cx,
                                );
                            },
                        )),
                    ),
            );
        }
        rows.into_any_element()
    }

    fn render_video_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(preview) = self.current_video_preview() else {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Preview unavailable")
                .into_any_element();
        };
        match &preview.status {
            DesignVideoPreviewStatus::Idle => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Preview idle")
                .into_any_element(),
            DesignVideoPreviewStatus::Loading => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Loading preview…")
                .into_any_element(),
            DesignVideoPreviewStatus::Error { message } => div()
                .text_xs()
                .text_color(cx.theme().red)
                .child(format!("Preview error: {message}"))
                .into_any_element(),
            DesignVideoPreviewStatus::Ready => {
                let duration = preview.duration_seconds.max(0.);
                let current = preview.current_seconds.clamp(0., duration);
                let seek_back = (current - 1.).max(0.);
                let seek_forward = (current + 1.).min(duration);
                let scrub_forward = (current + duration * 0.1).min(duration);
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(div().text_xs().font_semibold().child("Video preview"))
                            .child(div().text_xs().child(format!(
                                "{} / {}s",
                                format_decimal(current),
                                format_decimal(duration)
                            ))),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-video-preview-play",
                                    self.id
                                )))
                                .label(if preview.playing { "Pause" } else { "Play" })
                                .xsmall()
                                .compact()
                                .primary()
                                .disabled(duration <= 0.)
                                .on_activate({
                                    let playing = preview.playing;
                                    cx.listener(move |this, _, _, cx| {
                                        this.request_video_preview_action(
                                            if playing {
                                                DesignVideoPreviewAction::Pause
                                            } else {
                                                DesignVideoPreviewAction::Play
                                            },
                                            cx,
                                        );
                                    })
                                }),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-video-preview-back",
                                    self.id
                                )))
                                .label("−1s")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(current <= 0.)
                                .on_activate(cx.listener(
                                    move |this, _, _, cx| {
                                        this.request_video_preview_action(
                                            DesignVideoPreviewAction::Seek { seconds: seek_back },
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-video-preview-forward",
                                    self.id
                                )))
                                .label("+1s")
                                .xsmall()
                                .compact()
                                .outline()
                                .disabled(current >= duration)
                                .on_activate(cx.listener(
                                    move |this, _, _, cx| {
                                        this.request_video_preview_action(
                                            DesignVideoPreviewAction::Seek {
                                                seconds: seek_forward,
                                            },
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "{}-video-preview-scrub",
                            self.id
                        )))
                        .label("Scrub +10%")
                        .tooltip("Simulate a begin/preview/commit scrub")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(duration <= 0. || current >= duration)
                        .on_activate(cx.listener(
                            move |this, _, _, cx| {
                                this.request_video_scrub(scrub_forward, cx);
                            },
                        )),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_opacity_only(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Paint opacity"),
            )
            .child(
                Input::new(&self.opacity_input)
                    .suffix(div().text_xs().child("%"))
                    .xsmall()
                    .h(px(26.))
                    .disabled(self.editing_disabled())
                    .when(self.opacity_invalid, |input| {
                        input.border_color(cx.theme().red)
                    }),
            )
            .into_any_element()
    }

    fn render_empty(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .w(popup_width(window, PICKER_WIDTH))
            .p_4()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .child(
                Icon::new(IconName::Palette)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(div().text_sm().font_semibold().child("No paint selected"))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Call set_target to enable the picker."),
            )
            .into_any_element()
    }

    fn render_picker(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(paint) = self.paint.as_ref() else {
            return self.render_empty(window, cx);
        };
        if let Some(title) = self.color_only_title.clone() {
            return v_flex()
                .id(self.id.clone())
                .track_focus(&self.focus_handle)
                .relative()
                .w(popup_width(window, PICKER_WIDTH))
                .max_h(popup_height(window, PICKER_MAX_HEIGHT))
                .overflow_hidden()
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .text_color(cx.theme().popover_foreground)
                .shadow_lg()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.handle_picker_key(event, window, cx);
                }))
                .child(self.render_color_only_header(title, cx))
                .child(
                    v_flex()
                        .w_full()
                        .gap_3()
                        .p_4()
                        .children(self.render_color_editor(cx)),
                )
                .into_any_element();
        }
        let paint_type = paint.paint_type();
        let color_editable = matches!(
            paint_type,
            DesignPaintType::Solid | DesignPaintType::Gradient
        );
        let scroll_handle = self.scroll_handle.clone();
        let body = if self.active_tab == PaintPickerTab::Libraries {
            v_flex()
                .id(SharedString::from(format!("{}-libraries-scroll", self.id)))
                .w_full()
                .max_h(popup_height(window, PICKER_MAX_HEIGHT) - px(PICKER_HEADER_HEIGHT))
                .overflow_y_scroll()
                .track_scroll(&self.scroll_handle)
                .child(if self.shader_browser_requested {
                    self.render_shader_browser(paint, cx)
                } else {
                    self.render_variables(paint, cx)
                })
                .into_any_element()
        } else {
            let content = v_flex()
                .w_full()
                .gap_3()
                .p_4()
                .children(self.render_paint_tools(cx))
                .children(self.render_gradient_controls(paint, cx))
                .when(color_editable, |content| {
                    content
                        .children(self.render_color_editor(cx))
                        .child(self.render_variable_scope(paint, cx))
                })
                .when(!color_editable, |content| {
                    content
                        .child(self.render_source_state(paint, cx))
                        .child(self.render_opacity_only(cx))
                });
            v_flex()
                .id(SharedString::from(format!("{}-custom-scroll", self.id)))
                .w_full()
                .max_h(popup_height(window, PICKER_MAX_HEIGHT) - px(PICKER_HEADER_HEIGHT))
                .overflow_y_scroll()
                .track_scroll(&self.scroll_handle)
                .child(self.render_type_tabs(paint, cx))
                .child(content)
                .into_any_element()
        };

        v_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .relative()
            .w(popup_width(window, PICKER_WIDTH))
            .max_h(popup_height(window, PICKER_MAX_HEIGHT))
            .overflow_hidden()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_picker_key(event, window, cx);
            }))
            .child(self.render_header(cx))
            .child(
                div().relative().min_h(px(0.)).child(body).child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .h_full()
                        .child(Scrollbar::new(&scroll_handle).axis(ScrollbarAxis::Vertical)),
                ),
            )
            .into_any_element()
    }
}

impl Render for PaintPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_picker(window, cx)
    }
}

fn is_eyedropper_shortcut(event: &KeyDownEvent) -> bool {
    let modifiers = event.keystroke.modifiers;
    let unmodified_i = event.keystroke.key.eq_ignore_ascii_case("i") && !modifiers.modified();
    let mac_control_c = cfg!(target_os = "macos")
        && event.keystroke.key.eq_ignore_ascii_case("c")
        && modifiers.control
        && !modifiers.alt
        && !modifiers.shift
        && !modifiers.platform
        && !modifiers.function;
    unmodified_i || mac_control_c
}

fn paint_picker_locked(paint: &DesignPaint) -> bool {
    paint.read_only || matches!(&paint.payload, DesignPaintPayload::Unsupported(_))
}

fn paint_color_locked(paint: &DesignPaint, selected_stop: usize) -> bool {
    paint_picker_locked(paint) || paint.selected_color_is_bound(selected_stop)
}

fn selected_color_binding(
    paint: &DesignPaint,
    selected_stop: usize,
) -> Option<&DesignPaintBinding> {
    match &paint.payload {
        DesignPaintPayload::Solid(solid) => solid.binding.as_ref(),
        DesignPaintPayload::Gradient(gradient) => gradient
            .stops
            .get(selected_stop.min(gradient.stops.len().saturating_sub(1)))
            .and_then(|stop| stop.binding.as_ref()),
        _ => None,
    }
}

fn paint_variable_matches_binding(
    variable: &DesignVariable,
    selected_binding: Option<&DesignPaintBinding>,
) -> bool {
    selected_binding.is_some_and(|binding| binding.variable_id == variable.id)
}

fn paint_variable_color(variable: &DesignVariable) -> DesignColor {
    match variable.resolved_value.as_ref() {
        Some(DesignVariableResolvedValue::Color(color)) => *color,
        _ => DesignColor::BLACK,
    }
}

fn paint_resource_matches<'a>(
    normalized_query: &str,
    values: impl IntoIterator<Item = &'a str>,
) -> bool {
    normalized_query.is_empty()
        || values
            .into_iter()
            .any(|value| value.to_lowercase().contains(normalized_query))
}

fn paint_color_target(paint: &DesignPaint, selected_stop: usize) -> Option<DesignPaintColorTarget> {
    match &paint.payload {
        DesignPaintPayload::Solid(_) => Some(DesignPaintColorTarget::Solid),
        DesignPaintPayload::Gradient(gradient) => {
            let index = selected_stop.min(gradient.stops.len().saturating_sub(1));
            let stop = gradient.stops.get(index)?;
            Some(DesignPaintColorTarget::GradientStop {
                stop_id: stop.id.clone(),
                index,
            })
        }
        DesignPaintPayload::Pattern(_)
        | DesignPaintPayload::Image(_)
        | DesignPaintPayload::Video(_)
        | DesignPaintPayload::Shader(_)
        | DesignPaintPayload::Unsupported(_) => None,
    }
}

fn selected_color(paint: &DesignPaint, selected_stop: usize) -> DesignColor {
    if paint.kind.is_gradient() {
        paint
            .gradient_stops
            .get(clamp_stop_index(paint, selected_stop))
            .map_or(paint.color, |stop| stop.color)
    } else {
        paint.color
    }
}

fn picker_opacity(paint: &DesignPaint, selected_stop: usize) -> f32 {
    if paint.kind.is_gradient() {
        f32::from(selected_color(paint, selected_stop).alpha) / 255. * 100.
    } else {
        paint.opacity.clamp(0., 100.)
    }
}

fn picker_opacity_edit(paint: &DesignPaint, selected_stop: usize, opacity: f32) -> DesignPaintEdit {
    let opacity = opacity.clamp(0., 100.);
    if paint.kind.is_gradient() {
        let mut color = selected_color(paint, selected_stop);
        color.alpha = channel_from_unit(opacity / 100.);
        color_edit(paint, selected_stop, color)
    } else {
        DesignPaintEdit {
            property: DesignPaintProperty::Opacity,
            value: DesignPaintValue::Number(opacity),
        }
    }
}

fn rgb_edit_color(
    paint: &DesignPaint,
    selected_stop: usize,
    mut color: DesignColor,
) -> DesignColor {
    color.alpha = if paint.kind.is_gradient() {
        selected_color(paint, selected_stop).alpha
    } else {
        u8::MAX
    };
    color
}

fn color_input_edit(
    paint: &DesignPaint,
    selected_stop: usize,
    parsed: ParsedColorInput,
) -> DesignPaintEdit {
    let mut color = rgb_edit_color(paint, selected_stop, parsed.color);
    if parsed.explicit_alpha && paint.kind.is_gradient() {
        color.alpha = parsed.color.alpha;
    }
    color_edit(paint, selected_stop, color)
}

fn rgb_hex(color: DesignColor) -> String {
    format!("{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}

fn rgba_hex(color: DesignColor) -> String {
    if color.alpha == u8::MAX {
        rgb_hex(color)
    } else {
        format!(
            "{:02X}{:02X}{:02X}{:02X}",
            color.red, color.green, color.blue, color.alpha
        )
    }
}

fn css_rgba(color: DesignColor) -> String {
    format!(
        "rgba({}, {}, {}, {})",
        color.red,
        color.green,
        color.blue,
        format_css_alpha(color.alpha)
    )
}

fn format_css_alpha(alpha: u8) -> String {
    let alpha = f32::from(alpha) / 255.;
    if alpha <= f32::EPSILON {
        "0".to_owned()
    } else if (alpha - 1.).abs() <= f32::EPSILON {
        "1".to_owned()
    } else {
        format!("{alpha:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn picker_color_with_opacity(paint: &DesignPaint, selected_stop: usize) -> DesignColor {
    let mut color = selected_color(paint, selected_stop);
    color.alpha = channel_from_unit(picker_opacity(paint, selected_stop) / 100.);
    color
}

fn color_text_value(format: ColorFormat, paint: &DesignPaint, selected_stop: usize) -> String {
    let color = picker_color_with_opacity(paint, selected_stop);
    match format {
        ColorFormat::Css => css_rgba(color),
        ColorFormat::Hex | ColorFormat::Rgb | ColorFormat::Hsl | ColorFormat::Hsb => {
            rgba_hex(color)
        }
    }
}

fn clamp_stop_index(paint: &DesignPaint, selected_stop: usize) -> usize {
    selected_stop.min(paint.gradient_stops.len().saturating_sub(1))
}

fn media_transform_is_finite(transform: DesignPaintTransform) -> bool {
    [
        transform.m11,
        transform.m12,
        transform.m21,
        transform.m22,
        transform.tx,
        transform.ty,
    ]
    .into_iter()
    .all(f32::is_finite)
}

fn normalize_paint(mut paint: DesignPaint) -> DesignPaint {
    paint.opacity = if paint.opacity.is_finite() {
        paint.opacity.clamp(0., 100.)
    } else {
        100.
    };
    if let DesignPaintPayload::Gradient(gradient) = &mut paint.payload {
        gradient.stops.retain(|stop| stop.position.is_finite());
        for stop in &mut gradient.stops {
            stop.position = stop.position.clamp(0., 1.);
        }
        gradient.stops.sort_by(|left, right| {
            left.position
                .partial_cmp(&right.position)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        match gradient.stops.as_slice() {
            [] => {
                let mut transparent = paint.color;
                transparent.alpha = 0;
                gradient.stops = vec![
                    DesignGradientStop::new(0., paint.color),
                    DesignGradientStop::new(1., transparent),
                ];
            }
            [only] => {
                let second = DesignGradientStop::new(1., only.color);
                gradient.stops.push(second);
            }
            _ => {}
        }
    }
    paint.sync_legacy_projection();
    paint
}

fn paint_for_kind(paint: &DesignPaint, kind: DesignPaintKind, selected_stop: usize) -> DesignPaint {
    let mut candidate = paint_for_type(paint, kind.paint_type(), selected_stop);
    if let DesignPaintPayload::Gradient(gradient) = &mut candidate.payload
        && kind.is_gradient()
    {
        gradient.kind = kind;
        candidate.sync_legacy_projection();
    }
    candidate
}

fn paint_for_type(
    paint: &DesignPaint,
    paint_type: DesignPaintType,
    selected_stop: usize,
) -> DesignPaint {
    if paint.paint_type() == paint_type {
        return paint.clone();
    }

    let color = selected_color(paint, selected_stop);
    let mut candidate = paint.clone();
    candidate.payload = match paint_type {
        DesignPaintType::Solid => DesignPaintPayload::Solid(DesignSolidPaint {
            color,
            binding: None,
        }),
        DesignPaintType::Gradient => {
            let mut transparent = color;
            transparent.alpha = 0;
            DesignPaintPayload::Gradient(DesignGradientPaint {
                kind: DesignPaintKind::LinearGradient,
                stops: vec![
                    DesignGradientStop::new(0., color),
                    DesignGradientStop::new(1., transparent),
                ],
                transform: DesignPaintTransform::IDENTITY,
            })
        }
        DesignPaintType::Pattern => DesignPaintPayload::Pattern(DesignPatternPaint {
            source_node_id: "".into(),
            tile_type: DesignPatternTileType::Rectangular,
            scaling_factor: 1.,
            spacing: DesignPatternSpacing::default(),
            horizontal_alignment: DesignPatternHorizontalAlignment::Center,
        }),
        DesignPaintType::Image => DesignPaintPayload::Image(DesignImagePaint {
            source: DesignPaintSource::default(),
            placement: DesignMediaPaintPlacement::default(),
            filters: DesignImageFilters::default(),
        }),
        DesignPaintType::Video => DesignPaintPayload::Video(DesignVideoPaint {
            source: DesignPaintSource::default(),
            placement: DesignMediaPaintPlacement::default(),
            filters: DesignImageFilters::default(),
        }),
        DesignPaintType::Shader | DesignPaintType::Unsupported => return candidate,
    };
    candidate.sync_legacy_projection();
    candidate
}

#[cfg(test)]
fn paint_with_color(paint: &DesignPaint, selected_stop: usize, color: DesignColor) -> DesignPaint {
    let mut candidate = paint.clone();
    let _ = candidate.apply_edit(&color_edit(paint, selected_stop, color));
    candidate
}

fn paint_with_added_stop(paint: &DesignPaint, selected_stop: usize) -> (DesignPaint, usize) {
    let candidate = normalize_paint(paint.clone());
    if !candidate.kind.is_gradient() {
        return (candidate, 0);
    }

    let index = clamp_stop_index(&candidate, selected_stop);
    let next_index = (index + 1).min(candidate.gradient_stops.len() - 1);
    let left = &candidate.gradient_stops[index];
    let right = &candidate.gradient_stops[next_index];
    let position = if index == next_index {
        (left.position + 0.1).min(1.)
    } else {
        (left.position + right.position) * 0.5
    };
    paint_with_added_stop_at(&candidate, position)
}

fn paint_with_added_stop_at(paint: &DesignPaint, position: f32) -> (DesignPaint, usize) {
    let mut candidate = normalize_paint(paint.clone());
    if !candidate.kind.is_gradient() {
        return (candidate, 0);
    }
    let position = position.clamp(0., 1.);
    let right_index = candidate
        .gradient_stops
        .iter()
        .position(|stop| stop.position >= position)
        .unwrap_or(candidate.gradient_stops.len() - 1);
    let left_index = right_index.saturating_sub(1);
    let left = &candidate.gradient_stops[left_index];
    let right = &candidate.gradient_stops[right_index];
    let span = (right.position - left.position).max(f32::EPSILON);
    let amount = ((position - left.position) / span).clamp(0., 1.);
    let stop = DesignGradientStop::new(position, mix_color(left.color, right.color, amount));
    let _ = candidate.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::GradientStopAdd,
        value: DesignPaintValue::GradientStop(stop.clone()),
    });
    let selected = candidate
        .gradient_stops
        .iter()
        .rposition(|candidate| candidate.position == stop.position && candidate.color == stop.color)
        .unwrap_or(left_index);
    (candidate, selected)
}

fn paint_with_removed_stop(
    paint: &DesignPaint,
    selected_stop: usize,
) -> Option<(DesignPaint, usize)> {
    let mut candidate = normalize_paint(paint.clone());
    if !candidate.kind.is_gradient() || candidate.gradient_stops.len() <= 2 {
        return None;
    }
    let index = clamp_stop_index(&candidate, selected_stop);
    let stop = candidate.gradient_stops[index].clone();
    let _ = candidate.apply_edit(&DesignPaintEdit {
        property: DesignPaintProperty::GradientStopRemove {
            stop_id: stop.id,
            index,
        },
        value: DesignPaintValue::None,
    });
    Some((
        candidate,
        index.min(paint.gradient_stops.len().saturating_sub(2)),
    ))
}

fn color_edit(paint: &DesignPaint, selected_stop: usize, color: DesignColor) -> DesignPaintEdit {
    let property = if paint.kind.is_gradient() {
        let index = clamp_stop_index(paint, selected_stop);
        DesignPaintProperty::GradientStopColor {
            stop_id: paint.gradient_stops[index].id.clone(),
            index,
        }
    } else {
        DesignPaintProperty::Color
    };
    DesignPaintEdit {
        property,
        value: DesignPaintValue::Color(color),
    }
}

fn gradient_segments(paint: &DesignPaint) -> Vec<(f32, f32, DesignColor, DesignColor)> {
    let mut segments = Vec::new();
    if let Some(first) = paint.gradient_stops.first() {
        let position = first.position.clamp(0., 1.);
        if position > 0. {
            segments.push((0., position, first.color, first.color));
        }
    }
    segments.extend(paint.gradient_stops.windows(2).map(|pair| {
        let left = pair[0].position.clamp(0., 1.);
        let right = pair[1].position.clamp(left, 1.);
        (left, right, pair[0].color, pair[1].color)
    }));
    if let Some(last) = paint.gradient_stops.last() {
        let position = last.position.clamp(0., 1.);
        if position < 1. {
            segments.push((position, 1., last.color, last.color));
        }
    }
    segments
}

fn color_channel_values(format: ColorFormat, color: DesignColor) -> [String; 3] {
    match format {
        ColorFormat::Hex | ColorFormat::Css => [String::new(), String::new(), String::new()],
        ColorFormat::Rgb => [
            color.red.to_string(),
            color.green.to_string(),
            color.blue.to_string(),
        ],
        ColorFormat::Hsl => {
            let hsl = rgb_to_hsl(color);
            [
                format_decimal(hsl.hue),
                format_decimal(hsl.saturation * 100.),
                format_decimal(hsl.lightness * 100.),
            ]
        }
        ColorFormat::Hsb => {
            let hsv = rgb_to_hsv(color);
            [
                format_decimal(hsv.hue),
                format_decimal(hsv.saturation * 100.),
                format_decimal(hsv.value * 100.),
            ]
        }
    }
}

fn parse_color_channel_value(format: ColorFormat, channel: usize, input: &str) -> Option<f32> {
    if matches!(format, ColorFormat::Hex | ColorFormat::Css) || channel >= 3 {
        return None;
    }
    let value = input.trim().parse::<f32>().ok()?;
    if !value.is_finite() {
        return None;
    }
    let maximum = match format {
        ColorFormat::Rgb => 255.,
        ColorFormat::Hsl | ColorFormat::Hsb if channel == 0 => 360.,
        ColorFormat::Hsl | ColorFormat::Hsb => 100.,
        ColorFormat::Hex | ColorFormat::Css => return None,
    };
    (0. ..=maximum).contains(&value).then_some(value)
}

fn parse_color_channels(format: ColorFormat, inputs: [&str; 3]) -> Option<DesignColor> {
    let values = [
        parse_color_channel_value(format, 0, inputs[0])?,
        parse_color_channel_value(format, 1, inputs[1])?,
        parse_color_channel_value(format, 2, inputs[2])?,
    ];
    match format {
        ColorFormat::Hex | ColorFormat::Css => None,
        ColorFormat::Rgb => Some(DesignColor::rgb(
            values[0].round() as u8,
            values[1].round() as u8,
            values[2].round() as u8,
        )),
        ColorFormat::Hsl => Some(hsl_to_color(
            Hsl {
                hue: values[0],
                saturation: values[1] / 100.,
                lightness: values[2] / 100.,
            },
            u8::MAX,
        )),
        ColorFormat::Hsb => Some(hsv_to_color(
            Hsv {
                hue: values[0],
                saturation: values[1] / 100.,
                value: values[2] / 100.,
            },
            u8::MAX,
        )),
    }
}

fn parse_hex_color_input(input: &str) -> Option<ParsedColorInput> {
    let trimmed = input.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let explicit_alpha = matches!(hex.len(), 4 | 8);
    let [red, green, blue, alpha] = rgba_channels(parse_hex_rgba(hex)?);
    Some(ParsedColorInput {
        color: DesignColor::rgba(red, green, blue, alpha),
        explicit_alpha,
    })
}

#[cfg(test)]
fn parse_hex_color(input: &str) -> Option<DesignColor> {
    parse_hex_color_input(input).map(|parsed| parsed.color)
}

fn parse_css_color_input(input: &str) -> Option<ParsedColorInput> {
    let input = input.trim();
    let open = input.find('(')?;
    let close = input.rfind(')')?;
    if close != input.len().saturating_sub(1) || !input[..open].trim().eq_ignore_ascii_case("rgba")
    {
        return None;
    }
    let mut components = input[open + 1..close].split(',').map(str::trim);
    let red = parse_css_rgb_channel(components.next()?)?;
    let green = parse_css_rgb_channel(components.next()?)?;
    let blue = parse_css_rgb_channel(components.next()?)?;
    let alpha = components.next()?.parse::<f32>().ok()?;
    if components.next().is_some() || !alpha.is_finite() || !(0. ..=1.).contains(&alpha) {
        return None;
    }
    Some(ParsedColorInput {
        color: DesignColor::rgba(red, green, blue, channel_from_unit(alpha)),
        explicit_alpha: true,
    })
}

fn parse_css_rgb_channel(input: &str) -> Option<u8> {
    let value = input.parse::<f32>().ok()?;
    (value.is_finite() && (0. ..=255.).contains(&value)).then(|| value.round() as u8)
}

fn parse_opacity(input: &str) -> Option<f32> {
    let input = input.trim().strip_suffix('%').unwrap_or(input.trim());
    let value = input.trim().parse::<f32>().ok()?;
    value.is_finite().then(|| value.clamp(0., 100.))
}

fn parse_stop_position(input: &str) -> Option<f32> {
    parse_opacity(input).map(|position| position / 100.)
}

fn format_decimal(value: f32) -> String {
    super::format::format_compact_number(value)
}

fn fraction_x(bounds: Bounds<Pixels>, position: Point<Pixels>) -> f32 {
    let width = f32::from(bounds.size.width);
    if width <= 0. {
        return 0.;
    }
    (f32::from(position.x - bounds.origin.x) / width).clamp(0., 1.)
}

fn fraction_y(bounds: Bounds<Pixels>, position: Point<Pixels>) -> f32 {
    let height = f32::from(bounds.size.height);
    if height <= 0. {
        return 0.;
    }
    (f32::from(position.y - bounds.origin.y) / height).clamp(0., 1.)
}

fn color_to_hsla(color: DesignColor) -> Hsla {
    let encoded = u32::from_be_bytes([color.red, color.green, color.blue, color.alpha]);
    rgba(encoded).into()
}

fn composite_color(foreground: DesignColor, background: DesignColor) -> DesignColor {
    let foreground_alpha = f32::from(foreground.alpha) / 255.;
    let background_alpha = f32::from(background.alpha) / 255.;
    let output_alpha = foreground_alpha + background_alpha * (1. - foreground_alpha);
    if output_alpha <= f32::EPSILON {
        return DesignColor::rgba(0, 0, 0, 0);
    }
    let channel = |foreground: u8, background: u8| {
        let value = (f32::from(foreground) * foreground_alpha
            + f32::from(background) * background_alpha * (1. - foreground_alpha))
            / output_alpha;
        value.clamp(0., 255.).round() as u8
    };
    DesignColor::rgba(
        channel(foreground.red, background.red),
        channel(foreground.green, background.green),
        channel(foreground.blue, background.blue),
        channel_from_unit(output_alpha),
    )
}

fn opaque_color(color: DesignColor) -> DesignColor {
    composite_color(color, DesignColor::WHITE)
}

fn wcag_relative_luminance(color: DesignColor) -> f32 {
    let linear = |channel: u8| {
        let encoded = f32::from(channel) / 255.;
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
}

fn wcag_contrast_ratio(foreground: DesignColor, background: DesignColor) -> f32 {
    let background = opaque_color(background);
    let foreground = composite_color(foreground, background);
    let foreground_luminance = wcag_relative_luminance(foreground);
    let background_luminance = wcag_relative_luminance(background);
    let lighter = foreground_luminance.max(background_luminance);
    let darker = foreground_luminance.min(background_luminance);
    (lighter + 0.05) / (darker + 0.05)
}

fn contrast_threshold(
    category: DesignColorContrastCategory,
    level: DesignColorContrastLevel,
) -> Option<f32> {
    match (category, level) {
        (DesignColorContrastCategory::NormalText, DesignColorContrastLevel::Aa) => Some(4.5),
        (DesignColorContrastCategory::NormalText, DesignColorContrastLevel::Aaa) => Some(7.),
        (DesignColorContrastCategory::LargeText, DesignColorContrastLevel::Aa) => Some(3.),
        (DesignColorContrastCategory::LargeText, DesignColorContrastLevel::Aaa) => Some(4.5),
        (DesignColorContrastCategory::Graphics, DesignColorContrastLevel::Aa) => Some(3.),
        (DesignColorContrastCategory::Auto, _)
        | (DesignColorContrastCategory::Graphics, DesignColorContrastLevel::Aaa) => None,
    }
}

fn contrast_value_crossings(
    hue: f32,
    saturation: f32,
    alpha: u8,
    background: DesignColor,
    threshold: f32,
) -> Vec<f32> {
    const VALUE_STEPS: usize = 96;
    let passes = |value: f32| {
        let color = hsv_to_color(
            Hsv {
                hue,
                saturation,
                value,
            },
            alpha,
        );
        wcag_contrast_ratio(color, background) + f32::EPSILON >= threshold
    };
    let mut crossings = Vec::with_capacity(2);
    let mut previous_value = 0.;
    let mut previous_passes = passes(previous_value);
    for step in 1..=VALUE_STEPS {
        let value = step as f32 / VALUE_STEPS as f32;
        let current_passes = passes(value);
        if current_passes != previous_passes {
            let mut low = previous_value;
            let mut high = value;
            for _ in 0..8 {
                let middle = (low + high) / 2.;
                if passes(middle) == previous_passes {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            crossings.push((low + high) / 2.);
        }
        previous_value = value;
        previous_passes = current_passes;
    }
    crossings
}

fn rgb_to_hsv(color: DesignColor) -> Hsv {
    let red = f32::from(color.red) / 255.;
    let green = f32::from(color.green) / 255.;
    let blue = f32::from(color.blue) / 255.;
    let maximum = red.max(green.max(blue));
    let minimum = red.min(green.min(blue));
    let delta = maximum - minimum;

    let hue = if delta <= f32::EPSILON {
        0.
    } else if maximum == red {
        (60. * ((green - blue) / delta)).rem_euclid(360.)
    } else if maximum == green {
        60. * (((blue - red) / delta) + 2.)
    } else {
        60. * (((red - green) / delta) + 4.)
    };
    let saturation = if maximum <= f32::EPSILON {
        0.
    } else {
        delta / maximum
    };

    Hsv {
        hue,
        saturation,
        value: maximum,
    }
}

fn rgb_to_hsl(color: DesignColor) -> Hsl {
    let red = f32::from(color.red) / 255.;
    let green = f32::from(color.green) / 255.;
    let blue = f32::from(color.blue) / 255.;
    let maximum = red.max(green.max(blue));
    let minimum = red.min(green.min(blue));
    let delta = maximum - minimum;
    let lightness = (maximum + minimum) / 2.;
    let saturation = if delta <= f32::EPSILON {
        0.
    } else {
        delta / (1. - (2. * lightness - 1.).abs())
    };

    Hsl {
        hue: rgb_to_hsv(color).hue,
        saturation,
        lightness,
    }
}

fn hsl_to_color(hsl: Hsl, alpha: u8) -> DesignColor {
    let hue = hsl.hue.rem_euclid(360.);
    let saturation = hsl.saturation.clamp(0., 1.);
    let lightness = hsl.lightness.clamp(0., 1.);
    let chroma = (1. - (2. * lightness - 1.).abs()) * saturation;
    let x = chroma * (1. - ((hue / 60.).rem_euclid(2.) - 1.).abs());
    let offset = lightness - chroma / 2.;
    let (red, green, blue) = match hue {
        hue if hue < 60. => (chroma, x, 0.),
        hue if hue < 120. => (x, chroma, 0.),
        hue if hue < 180. => (0., chroma, x),
        hue if hue < 240. => (0., x, chroma),
        hue if hue < 300. => (x, 0., chroma),
        _ => (chroma, 0., x),
    };

    DesignColor::rgba(
        channel_from_unit(red + offset),
        channel_from_unit(green + offset),
        channel_from_unit(blue + offset),
        alpha,
    )
}

fn hsv_to_color(hsv: Hsv, alpha: u8) -> DesignColor {
    let hue = hsv.hue.rem_euclid(360.);
    let saturation = hsv.saturation.clamp(0., 1.);
    let value = hsv.value.clamp(0., 1.);
    let chroma = value * saturation;
    let x = chroma * (1. - ((hue / 60.).rem_euclid(2.) - 1.).abs());
    let offset = value - chroma;
    let (red, green, blue) = match hue {
        hue if hue < 60. => (chroma, x, 0.),
        hue if hue < 120. => (x, chroma, 0.),
        hue if hue < 180. => (0., chroma, x),
        hue if hue < 240. => (0., x, chroma),
        hue if hue < 300. => (x, 0., chroma),
        _ => (chroma, 0., x),
    };

    DesignColor::rgba(
        channel_from_unit(red + offset),
        channel_from_unit(green + offset),
        channel_from_unit(blue + offset),
        alpha,
    )
}

fn channel_from_unit(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

fn mix_color(left: DesignColor, right: DesignColor, amount: f32) -> DesignColor {
    let amount = amount.clamp(0., 1.);
    let channel = |left: u8, right: u8| {
        (f32::from(left) + (f32::from(right) - f32::from(left)) * amount).round() as u8
    };
    DesignColor::rgba(
        channel(left.red, right.red),
        channel(left.green, right.green),
        channel(left.blue, right.blue),
        channel(left.alpha, right.alpha),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    use gpui::{
        Entity, IntoElement, Keystroke, Render, Subscription, TestAppContext, VisualTestContext,
        Window, div, px,
    };
    use gpui_component::Root;

    struct PickerTestHost {
        picker: Entity<PaintPicker>,
        events: Rc<RefCell<Vec<PaintPickerEvent>>>,
        _subscription: Subscription,
    }

    impl PickerTestHost {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let picker = cx.new(|cx| PaintPicker::new("paint-picker-test", window, cx));
            let events = Rc::new(RefCell::new(Vec::new()));
            let captured_events = events.clone();
            let subscription = cx.subscribe(&picker, move |_, _, event: &PaintPickerEvent, _| {
                captured_events.borrow_mut().push(event.clone());
            });
            Self {
                picker,
                events,
                _subscription: subscription,
            }
        }
    }

    impl Render for PickerTestHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(320.)).h(px(720.)).child(self.picker.clone())
        }
    }

    fn setup_picker(cx: &mut TestAppContext) -> (Entity<PickerTestHost>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let host_slot = Rc::new(RefCell::new(None));
        let captured_host = host_slot.clone();
        let (_, visual_cx) = cx.add_window_view(move |window, cx| {
            let host = cx.new(|cx| PickerTestHost::new(window, cx));
            *captured_host.borrow_mut() = Some(host.clone());
            Root::new(host, window, cx)
        });
        let host = host_slot
            .borrow_mut()
            .take()
            .expect("Paint picker test host should be installed");
        visual_cx.run_until_parked();
        (host, visual_cx)
    }

    fn picker(host: &Entity<PickerTestHost>, cx: &VisualTestContext) -> Entity<PaintPicker> {
        cx.read(|app| host.read(app).picker.clone())
    }

    fn picker_events(
        host: &Entity<PickerTestHost>,
        cx: &VisualTestContext,
    ) -> Rc<RefCell<Vec<PaintPickerEvent>>> {
        cx.read(|app| host.read(app).events.clone())
    }

    fn gradient() -> DesignPaint {
        DesignPaint::gradient(
            DesignPaintKind::RadialGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK),
                DesignGradientStop::new(1., DesignColor::WHITE),
            ],
        )
    }

    fn edit_phases(events: &[PaintPickerEvent]) -> Vec<DesignPanelEditPhase> {
        events
            .iter()
            .filter_map(|event| match event {
                PaintPickerEvent::Edit { phase, .. } => Some(*phase),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn active_crop_rotation_previews_an_arbitrary_affine_angle() {
        let crop_tool = DesignMediaCropToolState {
            active: true,
            transform: DesignPaintTransform::IDENTITY,
            zoom: 2.5,
            aspect_ratio: super::super::DesignMediaCropAspectRatio::Square,
        };

        let DesignMediaCropAction::Preview {
            transform,
            zoom,
            aspect_ratio,
        } = rotated_crop_preview(crop_tool)
        else {
            panic!("crop rotation must remain a host-controlled preview");
        };

        let radians = CROP_ROTATION_STEP_DEGREES.to_radians();
        assert!((transform.m11 - radians.cos()).abs() < 0.0001);
        assert!((transform.m12 - radians.sin()).abs() < 0.0001);
        assert!((transform.m21 + radians.sin()).abs() < 0.0001);
        assert!((transform.m22 - radians.cos()).abs() < 0.0001);
        assert_eq!((transform.tx, transform.ty), (0., 0.));
        assert_eq!(zoom, crop_tool.zoom);
        assert_eq!(aspect_ratio, crop_tool.aspect_ratio);
    }

    #[gpui::test]
    fn hex_enter_blur_and_retained_focus_each_have_one_complete_transaction(
        cx: &mut TestAppContext,
    ) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill"),
                    window,
                    cx,
                );
            });
        });
        let hex_input = visual_cx.read(|app| picker.read(app).hex_input.clone());
        visual_cx.update(|window, app| {
            hex_input.focus_handle(app).focus(window, app);
        });
        visual_cx.run_until_parked();
        visual_cx.update(|window, app| {
            hex_input.update(app, |input, cx| {
                input.set_value("112233", window, cx);
            });
        });
        visual_cx.run_until_parked();

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.handle_hex_input(&InputEvent::PressEnter { secondary: false }, window, cx);
                // GPUI keeps the field focused after Enter. This deferred Blur
                // is deliberately delivered and must not create a second
                // terminal.
                picker.handle_hex_input(&InputEvent::Blur, window, cx);
            });
        });
        visual_cx.run_until_parked();

        // Typing again while focus is retained starts a new Begin before the
        // next Preview; Blur then supplies that session's sole terminal.
        visual_cx.update(|window, app| {
            hex_input.update(app, |input, cx| {
                input.set_value("445566", window, cx);
            });
        });
        visual_cx.run_until_parked();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.handle_hex_input(&InputEvent::Blur, window, cx);
            });
        });
        visual_cx.run_until_parked();

        assert_eq!(
            edit_phases(events.borrow().as_slice()),
            [
                DesignPanelEditPhase::Begin,
                DesignPanelEditPhase::Preview,
                DesignPanelEditPhase::Commit,
                DesignPanelEditPhase::Begin,
                DesignPanelEditPhase::Preview,
                DesignPanelEditPhase::Commit,
            ]
        );
    }

    #[gpui::test]
    fn unchanged_and_invalid_text_edits_still_terminate_their_begin(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill"),
                    window,
                    cx,
                );
            });
        });
        let hex_input = visual_cx.read(|app| picker.read(app).hex_input.clone());
        visual_cx.update(|window, app| {
            hex_input.focus_handle(app).focus(window, app);
        });
        visual_cx.run_until_parked();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.handle_hex_input(&InputEvent::Focus, window, cx);
                picker.handle_hex_input(&InputEvent::Blur, window, cx);
            });
        });
        visual_cx.run_until_parked();
        assert_eq!(
            edit_phases(events.borrow().as_slice()),
            [DesignPanelEditPhase::Begin, DesignPanelEditPhase::Commit,]
        );

        events.borrow_mut().clear();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.handle_hex_input(&InputEvent::Focus, window, cx);
            });
        });
        visual_cx.update(|window, app| {
            hex_input.update(app, |input, cx| {
                input.set_value("invalid", window, cx);
            });
        });
        visual_cx.run_until_parked();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.handle_hex_input(&InputEvent::PressEnter { secondary: false }, window, cx);
                picker.handle_hex_input(&InputEvent::Blur, window, cx);
            });
        });
        visual_cx.run_until_parked();
        assert_eq!(
            edit_phases(events.borrow().as_slice()),
            [DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel,]
        );
    }

    #[gpui::test]
    fn rgba_text_edits_share_one_transaction_across_solid_color_and_opacity(
        cx: &mut TestAppContext,
    ) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill"),
                    window,
                    cx,
                );
            });
        });
        let input = visual_cx.read(|app| picker.read(app).hex_input.clone());
        visual_cx.update(|window, app| input.focus_handle(app).focus(window, app));
        visual_cx.run_until_parked();
        visual_cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_value("FF000080", window, cx);
            });
        });
        visual_cx.run_until_parked();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.handle_hex_input(&InputEvent::PressEnter { secondary: false }, window, cx);
                picker.handle_hex_input(&InputEvent::Blur, window, cx);
            });
        });
        visual_cx.run_until_parked();

        let events = events.borrow();
        assert_eq!(
            edit_phases(events.as_slice()),
            [
                DesignPanelEditPhase::Begin,
                DesignPanelEditPhase::Preview,
                DesignPanelEditPhase::Preview,
                DesignPanelEditPhase::Commit,
            ]
        );
        assert!(matches!(
            events.get(1),
            Some(PaintPickerEvent::Edit { edit, .. })
                if edit.property == DesignPaintProperty::Color
        ));
        assert!(matches!(
            events.get(2),
            Some(PaintPickerEvent::Edit { edit, .. })
                if edit.property == DesignPaintProperty::Opacity
        ));
        assert!(matches!(
            events.get(3),
            Some(PaintPickerEvent::Edit { edit, .. })
                if edit.property == DesignPaintProperty::Opacity
        ));
    }

    #[gpui::test]
    fn blend_mode_and_color_style_sample_emit_typed_atomic_intents(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill"),
                    window,
                    cx,
                );
                picker.set_color_style_sample_view_data(
                    DesignColorStyleSampleViewData::new(
                        [DesignColorStyleSample::new(
                            "page-brand",
                            "Brand",
                            DesignColor::PURPLE,
                        )],
                        [],
                    ),
                    cx,
                );
                picker.select_blend_mode(DesignBlendMode::Multiply, cx);
                picker.request_color_style_sample(
                    DesignColorStyleSampleSelection::page("page-brand"),
                    cx,
                );
            });
        });
        visual_cx.run_until_parked();

        let events = events.borrow();
        assert!(matches!(
            events.first(),
            Some(PaintPickerEvent::Edit {
                edit,
                phase: DesignPanelEditPhase::Commit,
                ..
            }) if edit.property == DesignPaintProperty::BlendMode
                && edit.value == DesignPaintValue::BlendMode(DesignBlendMode::Multiply)
        ));
        assert!(matches!(
            events.get(1),
            Some(PaintPickerEvent::ColorStyleSampleRequested {
                color_target: DesignPaintColorTarget::Solid,
                sample,
                ..
            }) if sample == &DesignColorStyleSampleSelection::page("page-brand")
        ));
    }

    #[gpui::test]
    fn color_only_editability_gates_color_and_opacity_independently(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::LayoutGrid,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("guide"),
                    window,
                    cx,
                );
                picker.set_color_only(Some("Guide color".into()), cx);
                picker.set_color_only_editability(false, true, cx);
                assert_eq!(picker.color_only_editability(), (false, true));
                assert!(picker.color_editing_disabled());
                assert!(!picker.opacity_editing_disabled());

                picker.set_color_only_editability(true, false, cx);
                assert_eq!(picker.color_only_editability(), (true, false));
                assert!(!picker.color_editing_disabled());
                assert!(picker.opacity_editing_disabled());
            });
        });
    }

    #[test]
    fn picker_recognizes_figma_eyedropper_shortcuts() {
        let i = KeyDownEvent {
            keystroke: Keystroke::parse("i").expect("I shortcut"),
            is_held: false,
            prefer_character_input: false,
        };
        let modified_i = KeyDownEvent {
            keystroke: Keystroke::parse("shift-i").expect("modified I shortcut"),
            is_held: false,
            prefer_character_input: false,
        };
        let control_c = KeyDownEvent {
            keystroke: Keystroke::parse("ctrl-c").expect("macOS Control-C shortcut"),
            is_held: false,
            prefer_character_input: false,
        };
        assert!(is_eyedropper_shortcut(&i));
        assert!(!is_eyedropper_shortcut(&modified_i));
        assert_eq!(
            is_eyedropper_shortcut(&control_c),
            cfg!(target_os = "macos")
        );
    }

    #[gpui::test]
    fn color_format_menu_roves_home_end_wraps_and_restores_focus(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill"),
                    window,
                    cx,
                );
                picker.focus_handle.focus(window, cx);
                picker.open_color_format_menu_from_keyboard(window, cx);
            });
        });
        visual_cx.run_until_parked();
        assert!(visual_cx.update(|window, app| {
            picker
                .read(app)
                .color_format_menu_focus_handle
                .is_focused(window)
        }));

        visual_cx.simulate_keystrokes("down");
        assert_eq!(
            visual_cx.read(|app| picker.read(app).color_format_menu_index),
            ColorFormat::Rgb.index()
        );
        visual_cx.simulate_keystrokes("end");
        assert_eq!(
            visual_cx.read(|app| picker.read(app).color_format_menu_index),
            ColorFormat::Hsb.index()
        );
        visual_cx.simulate_keystrokes("down");
        assert_eq!(
            visual_cx.read(|app| picker.read(app).color_format_menu_index),
            ColorFormat::Hex.index()
        );
        visual_cx.simulate_keystrokes("up");
        assert_eq!(
            visual_cx.read(|app| picker.read(app).color_format_menu_index),
            ColorFormat::Hsb.index()
        );
        visual_cx.simulate_keystrokes("home");
        assert_eq!(
            visual_cx.read(|app| picker.read(app).color_format_menu_index),
            ColorFormat::Hex.index()
        );
        visual_cx.simulate_keystrokes("end enter");
        visual_cx.run_until_parked();
        visual_cx.read(|app| {
            let picker = picker.read(app);
            assert_eq!(picker.color_format, ColorFormat::Hsb);
            assert!(!picker.color_format_menu_open);
        });
        assert!(
            visual_cx.update(|window, app| { picker.read(app).focus_handle.is_focused(window) })
        );

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.open_color_format_menu_from_keyboard(window, cx);
            });
        });
        visual_cx.run_until_parked();
        visual_cx.simulate_keystrokes("home escape");
        visual_cx.run_until_parked();
        visual_cx.read(|app| {
            let picker = picker.read(app);
            assert_eq!(picker.color_format, ColorFormat::Hsb);
            assert!(!picker.color_format_menu_open);
        });
        assert!(
            visual_cx.update(|window, app| { picker.read(app).focus_handle.is_focused(window) })
        );
    }

    #[gpui::test]
    fn creation_menu_roves_and_preserves_the_exact_gradient_stop_target(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        let paint = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::PURPLE).with_id("start"),
                DesignGradientStop::new(1., DesignColor::WHITE).with_id("end"),
            ],
        )
        .with_id("gradient");
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target("node", DesignPanelCollection::Fill, 0, paint, window, cx);
                picker.focus_handle.focus(window, cx);
                picker.open_creation_menu_from_keyboard(window, cx);
            });
        });
        visual_cx.run_until_parked();
        assert!(visual_cx.update(|window, app| {
            picker
                .read(app)
                .creation_menu_focus_handle
                .is_focused(window)
        }));

        visual_cx.simulate_keystrokes("enter");
        visual_cx.run_until_parked();
        assert!(matches!(
            events.borrow().as_slice(),
            [PaintPickerEvent::PaintStyleCreateRequested {
                target,
                color_target: DesignPaintColorTarget::GradientStop {
                    stop_id,
                    index: 0,
                },
            }] if target.node_id.as_ref() == "node"
                && target.paint_id.as_ref() == "gradient"
                && target.index == 0
                && stop_id.as_ref() == "start"
        ));
        assert!(
            visual_cx.update(|window, app| { picker.read(app).focus_handle.is_focused(window) })
        );

        events.borrow_mut().clear();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.open_creation_menu_from_keyboard(window, cx);
            });
        });
        visual_cx.run_until_parked();
        visual_cx.simulate_keystrokes("down enter");
        visual_cx.run_until_parked();
        assert!(matches!(
            events.borrow().as_slice(),
            [PaintPickerEvent::ColorVariableCreateRequested {
                target,
                color_target: DesignPaintColorTarget::GradientStop {
                    stop_id,
                    index: 0,
                },
                color,
            }] if target.node_id.as_ref() == "node"
                && target.paint_id.as_ref() == "gradient"
                && stop_id.as_ref() == "start"
                && *color == DesignColor::PURPLE
        ));
    }

    #[gpui::test]
    fn palette_scope_selector_uses_stable_library_ids_and_resets_stale_sources(
        cx: &mut TestAppContext,
    ) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill"),
                    window,
                    cx,
                );
                picker.set_color_style_sample_view_data(
                    DesignColorStyleSampleViewData::new(
                        [DesignColorStyleSample::new(
                            "page",
                            "Page",
                            DesignColor::BLACK,
                        )],
                        [super::super::DesignColorStyleSampleLibrary::new(
                            "shared",
                            "Shared library",
                            [DesignColorStyleSample::new(
                                "brand",
                                "Brand",
                                DesignColor::PURPLE,
                            )],
                        )],
                    ),
                    cx,
                );
                picker.set_paint_variable_view_data(
                    DesignPaintVariableViewData::new([
                        DesignVariable::page(
                            "shared-variable",
                            "Shared",
                            "colors",
                            "Colors",
                            super::super::DesignVariableResolvedType::Color,
                        )
                        .with_source(DesignVariableSource::library("shared", "Renamed duplicate")),
                        DesignVariable::page(
                            "other-variable",
                            "Other",
                            "colors",
                            "Colors",
                            super::super::DesignVariableResolvedType::Color,
                        )
                        .with_source(DesignVariableSource::library("other", "Other library")),
                    ]),
                    cx,
                );
                picker.focus_handle.focus(window, cx);
                picker.open_resource_scope_menu_from_keyboard(window, cx);
            });
        });
        visual_cx.run_until_parked();
        visual_cx.read(|app| {
            let picker = picker.read(app);
            let scopes = picker.resource_scopes();
            assert_eq!(scopes.len(), 3, "library IDs are de-duplicated");
            assert!(matches!(
                &scopes[1],
                PaintResourceScope::Library {
                    library_id,
                    library_name,
                } if library_id.as_ref() == "shared"
                    && library_name.as_ref() == "Shared library"
            ));
        });

        visual_cx.simulate_keystrokes("down enter");
        visual_cx.run_until_parked();
        assert!(visual_cx.read(|app| matches!(
            &picker.read(app).resource_scope,
            PaintResourceScope::Library { library_id, .. }
                if library_id.as_ref() == "shared"
        )));

        picker.update(visual_cx, |picker, cx| {
            picker.set_paint_variable_view_data(
                DesignPaintVariableViewData::new([DesignVariable::page(
                    "page-variable",
                    "Page",
                    "colors",
                    "Colors",
                    super::super::DesignVariableResolvedType::Color,
                )]),
                cx,
            );
            picker.set_color_style_sample_view_data(DesignColorStyleSampleViewData::default(), cx);
        });
        visual_cx.run_until_parked();
        visual_cx.read(|app| {
            let picker = picker.read(app);
            assert_eq!(picker.resource_scope, PaintResourceScope::Page);
            assert!(!picker.resource_scope_menu_open);
        });
    }

    #[test]
    fn contrast_checker_uses_wcag_luminance_and_alpha_compositing() {
        assert!(
            (wcag_contrast_ratio(DesignColor::rgb(0, 0, 0), DesignColor::WHITE) - 21.).abs()
                < 0.001
        );
        assert!((wcag_contrast_ratio(DesignColor::WHITE, DesignColor::WHITE) - 1.).abs() < 0.001);
        let translucent_black =
            wcag_contrast_ratio(DesignColor::rgba(0, 0, 0, 0x80), DesignColor::WHITE);
        assert!(translucent_black > 3.9 && translucent_black < 4.1);
        assert_eq!(
            composite_color(
                DesignColor::rgba(0xff, 0x00, 0x00, 0x80),
                DesignColor::WHITE
            ),
            DesignColor::rgb(0xff, 0x7f, 0x7f)
        );
    }

    #[gpui::test]
    fn contrast_checker_uses_host_ratio_mode_and_correction_without_predicting_paint(
        cx: &mut TestAppContext,
    ) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        let paint = DesignPaint::solid(DesignColor::rgb(0xa0, 0xae, 0xc0)).with_id("fill");
        let view_data = DesignColorContrastPaintViewData::new(
            crate::design::DesignColorContrastPaintTarget::paint(
                "node",
                DesignPanelCollection::Fill,
                "fill",
                0,
            ),
            [DesignColorContrastLeafViewData::new(
                DesignPaintColorTarget::Solid,
                DesignColor::WHITE,
                2.19,
                DesignColorContrastCategory::Graphics,
            )
            .with_corrections([crate::design::DesignColorContrastCorrection::new(
                DesignColorContrastCategory::Graphics,
                DesignColorContrastLevel::Aa,
                DesignColor::rgb(0, 0, 0),
            )])],
        );

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    paint.clone(),
                    window,
                    cx,
                );
                picker.set_contrast_view_data(Some(view_data), cx);
                picker.contrast_level = DesignColorContrastLevel::Aaa;
                assert_eq!(
                    picker.resolved_contrast_category(),
                    Some(DesignColorContrastCategory::Graphics)
                );
                assert_eq!(
                    picker.normalized_contrast_level(),
                    DesignColorContrastLevel::Aa
                );
                picker.apply_contrast_correction(cx);
                assert_eq!(picker.paint(), Some(&paint));
            });
        });

        let events = events.borrow();
        assert_eq!(events.len(), 1);
        let PaintPickerEvent::Edit {
            target,
            edit,
            phase,
        } = &events[0]
        else {
            panic!("contrast correction should use the ordinary paint edit event");
        };
        assert_eq!(target.node_id.as_ref(), "node");
        assert_eq!(target.paint_id.as_ref(), "fill");
        assert_eq!(*phase, DesignPanelEditPhase::Commit);
        assert_eq!(edit.property, DesignPaintProperty::Color);
        assert_eq!(
            edit.value,
            DesignPaintValue::Color(DesignColor::rgb(0, 0, 0))
        );
    }

    #[gpui::test]
    fn blend_mode_hover_preview_is_balanced_and_commit_cleans_it_up(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        let paint = DesignPaint::solid(DesignColor::BLUE).with_id("stable-fill");

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    paint.clone(),
                    window,
                    cx,
                );
                picker.set_blend_mode_preview(DesignBlendMode::Multiply, true, cx);
                picker.set_blend_mode_preview(DesignBlendMode::Multiply, true, cx);
                picker.select_blend_mode(DesignBlendMode::Multiply, cx);
                picker.cancel_blend_mode_preview(cx);
            });
        });
        visual_cx.run_until_parked();

        let events = events.borrow();
        assert!(matches!(
            events.as_slice(),
            [
                PaintPickerEvent::BlendModePreview {
                    phase: DesignMenuPreviewPhase::Begin,
                    original: DesignBlendMode::Normal,
                    candidate: DesignBlendMode::Multiply,
                    ..
                },
                PaintPickerEvent::BlendModePreview {
                    phase: DesignMenuPreviewPhase::End,
                    original: DesignBlendMode::Normal,
                    candidate: DesignBlendMode::Multiply,
                    ..
                },
                PaintPickerEvent::Edit {
                    edit,
                    phase: DesignPanelEditPhase::Commit,
                    ..
                }
            ] if edit.property == DesignPaintProperty::BlendMode
                && edit.value == DesignPaintValue::BlendMode(DesignBlendMode::Multiply)
        ));
        assert_eq!(
            visual_cx.read(|app| picker.read(app).paint().map(|paint| paint.blend_mode)),
            Some(DesignBlendMode::Normal),
            "menu previews and commits must not mutate the controlled picker snapshot"
        );
    }

    #[gpui::test]
    fn blend_mode_preview_ends_once_when_exact_paint_index_or_permission_changes(
        cx: &mut TestAppContext,
    ) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        let paint = DesignPaint::solid(DesignColor::PURPLE).with_id("stable-fill");

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    paint.clone(),
                    window,
                    cx,
                );
                picker.set_blend_mode_preview(DesignBlendMode::Screen, true, cx);
                picker.set_target("node", DesignPanelCollection::Fill, 1, paint, window, cx);
                picker.set_disabled(true, cx);
                picker.set_blend_mode_preview(DesignBlendMode::Multiply, true, cx);
                picker.prepare_for_dismissal(cx);
            });
        });
        visual_cx.run_until_parked();

        let previews = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                PaintPickerEvent::BlendModePreview {
                    target,
                    phase,
                    candidate,
                    ..
                } => Some((target.index, *phase, *candidate)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            previews,
            vec![
                (0, DesignMenuPreviewPhase::Begin, DesignBlendMode::Screen),
                (0, DesignMenuPreviewPhase::End, DesignBlendMode::Screen),
            ]
        );
    }

    #[test]
    fn contrast_palette_boundary_tracks_both_wcag_crossing_branches() {
        let crossings =
            contrast_value_crossings(210., 0.7, u8::MAX, DesignColor::rgb(0x77, 0x77, 0x77), 3.);
        assert!(crossings.len() <= 2);
        assert!(crossings.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(crossings.iter().all(|value| (0. ..=1.).contains(value)));
    }

    #[test]
    fn hex_parser_accepts_three_four_six_and_eight_digit_forms() {
        assert_eq!(
            parse_hex_color("#0af"),
            Some(DesignColor::rgb(0x00, 0xaa, 0xff))
        );
        assert_eq!(
            parse_hex_color("#0af8"),
            Some(DesignColor::rgba(0x00, 0xaa, 0xff, 0x88))
        );
        assert_eq!(
            parse_hex_color("4C86F7"),
            Some(DesignColor::rgb(0x4c, 0x86, 0xf7))
        );
        assert_eq!(
            parse_hex_color("4c86f780"),
            Some(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80))
        );
        assert!(parse_hex_color_input("4c86f7").is_some_and(|parsed| !parsed.explicit_alpha));
        assert!(parse_hex_color_input("4c86f780").is_some_and(|parsed| parsed.explicit_alpha));
        assert_eq!(parse_hex_color("12"), None);
        assert_eq!(parse_hex_color("12345"), None);
        assert_eq!(parse_hex_color("nope"), None);
    }

    #[test]
    fn hex_and_css_serializers_round_trip_alpha() {
        assert_eq!(rgb_hex(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80)), "4C86F7");
        assert_eq!(
            rgba_hex(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80)),
            "4C86F780"
        );
        assert_eq!(
            parse_css_color_input("rgba(76, 134, 247, 0.502)").map(|parsed| parsed.color),
            Some(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80))
        );
        let css = css_rgba(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80));
        assert_eq!(
            parse_css_color_input(&css).map(|parsed| parsed.color),
            Some(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80))
        );
        assert!(parse_css_color_input("rgb(76, 134, 247)").is_none());
        assert!(parse_css_color_input("rgba(76, 134, 247, 2)").is_none());
    }

    #[test]
    fn color_channel_conversions_cover_rgb_hsl_hsb_and_validation() {
        let source = DesignColor::rgb(0x4c, 0x86, 0xf7);
        for format in [ColorFormat::Rgb, ColorFormat::Hsl, ColorFormat::Hsb] {
            let values = color_channel_values(format, source);
            let round_trip = parse_color_channels(format, [&values[0], &values[1], &values[2]])
                .expect("formatted channels");
            for (actual, expected) in [
                (round_trip.red, source.red),
                (round_trip.green, source.green),
                (round_trip.blue, source.blue),
            ] {
                assert!(actual.abs_diff(expected) <= 1);
            }
        }

        assert_eq!(
            parse_color_channels(ColorFormat::Hsl, ["0", "100", "50"]),
            Some(DesignColor::rgb(0xff, 0x00, 0x00))
        );
        assert_eq!(
            parse_color_channels(ColorFormat::Hsb, ["120", "100", "100"]),
            Some(DesignColor::rgb(0x00, 0xff, 0x00))
        );
        assert!(parse_color_channels(ColorFormat::Rgb, ["256", "0", "0"]).is_none());
        assert!(parse_color_channels(ColorFormat::Hsl, ["361", "50", "50"]).is_none());
        assert!(parse_color_channels(ColorFormat::Hsb, ["0", "101", "50"]).is_none());
        assert!(parse_color_channels(ColorFormat::Rgb, ["NaN", "0", "0"]).is_none());
        assert!(parse_color_channels(ColorFormat::Css, ["0", "0", "0"]).is_none());
    }

    #[test]
    fn formatted_channel_edits_target_the_active_leaf_and_preserve_alpha() {
        let rgb = parse_color_channels(ColorFormat::Hsl, ["220", "91", "63"])
            .expect("valid HSL channels");
        let mut gradient = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK).with_id("start"),
                DesignGradientStop::new(1., DesignColor::rgba(0xff, 0xff, 0xff, 0x70))
                    .with_id("end"),
            ],
        );
        gradient.opacity = 58.;
        let color = rgb_edit_color(&gradient, 1, rgb);
        let edit = color_edit(&gradient, 1, color);
        assert_eq!(
            edit.property,
            DesignPaintProperty::GradientStopColor {
                stop_id: "end".into(),
                index: 1,
            }
        );
        assert_eq!(
            edit.value,
            DesignPaintValue::Color(DesignColor { alpha: 0x70, ..rgb })
        );
        assert!(gradient.apply_edit(&edit));
        assert_eq!(gradient.opacity, 58.);
        assert_eq!(gradient.gradient_stops[0].color, DesignColor::BLACK);
        assert_eq!(gradient.gradient_stops[1].color.alpha, 0x70);

        let mut solid = DesignPaint::solid(DesignColor::BLACK);
        solid.opacity = 42.;
        let color = rgb_edit_color(&solid, 0, rgb);
        let edit = color_edit(&solid, 0, color);
        assert_eq!(edit.property, DesignPaintProperty::Color);
        assert!(solid.apply_edit(&edit));
        assert_eq!(solid.opacity, 42.);
        assert_eq!(solid.color.alpha, u8::MAX);
    }

    #[test]
    fn solid_picker_uses_paint_opacity_for_both_rail_and_percentage() {
        let mut paint = DesignPaint::solid(DesignColor::rgba(0x12, 0x34, 0x56, 0x20));
        paint.opacity = 37.5;

        assert_eq!(picker_opacity(&paint, 0), 37.5);
        let edit = picker_opacity_edit(&paint, 0, 62.);
        assert_eq!(edit.property, DesignPaintProperty::Opacity);
        assert_eq!(edit.value, DesignPaintValue::Number(62.));
        assert!(paint.apply_edit(&edit));
        assert_eq!(paint.opacity, 62.);
        assert_eq!(paint.color.alpha, 0x20);
    }

    #[test]
    fn gradient_picker_uses_only_the_selected_stop_alpha() {
        let mut paint = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::rgba(0x12, 0x34, 0x56, 0x40))
                    .with_id("start"),
                DesignGradientStop::new(1., DesignColor::rgba(0xaa, 0xbb, 0xcc, 0xc0))
                    .with_id("end"),
            ],
        );
        paint.opacity = 33.;

        assert!((picker_opacity(&paint, 1) - f32::from(0xc0_u8) / 255. * 100.).abs() < 0.001);
        let edit = picker_opacity_edit(&paint, 1, 25.);
        assert_eq!(
            edit.property,
            DesignPaintProperty::GradientStopColor {
                stop_id: "end".into(),
                index: 1,
            }
        );
        assert_eq!(
            edit.value,
            DesignPaintValue::Color(DesignColor::rgba(0xaa, 0xbb, 0xcc, 0x40))
        );
        assert!(paint.apply_edit(&edit));
        assert_eq!(paint.opacity, 33.);
        assert_eq!(paint.gradient_stops[0].color.alpha, 0x40);
        assert_eq!(paint.gradient_stops[1].color.alpha, 0x40);
    }

    #[test]
    fn six_digit_rgb_edits_preserve_the_applicable_opacity() {
        let parsed = parse_hex_color("4C86F7").expect("six-digit RGB");

        let mut solid = DesignPaint::solid(DesignColor::rgba(0x12, 0x34, 0x56, 0x20));
        solid.opacity = 42.;
        let solid_color = rgb_edit_color(&solid, 0, parsed);
        assert_eq!(solid_color, DesignColor::rgb(0x4c, 0x86, 0xf7));
        let solid_edit = color_edit(&solid, 0, solid_color);
        assert!(solid.apply_edit(&solid_edit));
        assert_eq!(solid.opacity, 42.);

        let mut gradient = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK).with_id("start"),
                DesignGradientStop::new(1., DesignColor::rgba(0xff, 0xff, 0xff, 0x70))
                    .with_id("end"),
            ],
        );
        gradient.opacity = 58.;
        let stop_color = rgb_edit_color(&gradient, 1, parsed);
        assert_eq!(stop_color, DesignColor::rgba(0x4c, 0x86, 0xf7, 0x70));
        let gradient_edit = color_edit(&gradient, 1, stop_color);
        assert!(gradient.apply_edit(&gradient_edit));
        assert_eq!(gradient.opacity, 58.);
        assert_eq!(gradient.gradient_stops[1].color.alpha, 0x70);
    }

    #[test]
    fn explicit_alpha_maps_to_solid_paint_opacity_but_gradient_leaf_alpha() {
        let parsed = parse_hex_color_input("4C86F780").expect("eight-digit RGBA");

        let mut solid = DesignPaint::solid(DesignColor::BLACK);
        solid.opacity = 72.;
        let color_edit = color_input_edit(&solid, 0, parsed);
        assert_eq!(color_edit.property, DesignPaintProperty::Color);
        assert_eq!(
            color_edit.value,
            DesignPaintValue::Color(DesignColor::rgb(0x4c, 0x86, 0xf7))
        );
        assert!(solid.apply_edit(&color_edit));
        let opacity_edit =
            picker_opacity_edit(&solid, 0, f32::from(parsed.color.alpha) / 255. * 100.);
        assert_eq!(opacity_edit.property, DesignPaintProperty::Opacity);
        assert!(solid.apply_edit(&opacity_edit));
        assert_eq!(solid.color.alpha, u8::MAX);
        assert!((solid.opacity - f32::from(0x80_u8) / 255. * 100.).abs() < 0.001);

        let mut gradient = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK).with_id("start"),
                DesignGradientStop::new(1., DesignColor::WHITE).with_id("end"),
            ],
        );
        gradient.opacity = 37.;
        let edit = color_input_edit(&gradient, 1, parsed);
        assert_eq!(
            edit.property,
            DesignPaintProperty::GradientStopColor {
                stop_id: "end".into(),
                index: 1,
            }
        );
        assert_eq!(edit.value, DesignPaintValue::Color(parsed.color));
        assert!(gradient.apply_edit(&edit));
        assert_eq!(gradient.opacity, 37.);
        assert_eq!(gradient.gradient_stops[1].color.alpha, 0x80);
    }

    #[test]
    fn color_text_serialization_uses_the_applicable_alpha_leaf() {
        let mut solid = DesignPaint::solid(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x11));
        solid.opacity = f32::from(0x80_u8) / 255. * 100.;
        assert_eq!(color_text_value(ColorFormat::Hex, &solid, 0), "4C86F780");
        assert_eq!(
            color_text_value(ColorFormat::Css, &solid, 0),
            "rgba(76, 134, 247, 0.502)"
        );

        let gradient = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK),
                DesignGradientStop::new(1., DesignColor::rgba(0x4c, 0x86, 0xf7, 0x40)),
            ],
        );
        assert_eq!(color_text_value(ColorFormat::Hex, &gradient, 1), "4C86F740");
    }

    #[test]
    fn hsv_round_trip_preserves_display_channels_and_alpha() {
        for color in [
            DesignColor::rgba(0xff, 0x00, 0x00, 0x33),
            DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80),
            DesignColor::rgba(0x22, 0xcc, 0x88, 0xff),
            DesignColor::rgba(0x77, 0x77, 0x77, 0x01),
        ] {
            assert_eq!(hsv_to_color(rgb_to_hsv(color), color.alpha), color);
        }
    }

    #[test]
    fn top_level_types_preserve_gradient_subtypes_and_create_defaults() {
        assert_eq!(
            DesignPaintType::ALL,
            [
                DesignPaintType::Solid,
                DesignPaintType::Gradient,
                DesignPaintType::Pattern,
                DesignPaintType::Image,
                DesignPaintType::Video,
                DesignPaintType::Shader,
            ]
        );
        assert_eq!(PAINT_HEADER_TRAILING_CONTROL_COUNT, 2);
        assert_eq!(paint_header_min_width(), 272.);
        assert!(
            PICKER_WIDTH >= paint_header_min_width(),
            "six paint types plus compact Blend and Contrast controls must not overflow"
        );
        assert_eq!(
            COLOR_AREA_HEIGHT,
            PICKER_WIDTH - 32.,
            "the widened picker keeps the p-4 color plane square"
        );
        assert_eq!(
            GRADIENT_KINDS,
            [
                DesignPaintKind::LinearGradient,
                DesignPaintKind::RadialGradient,
                DesignPaintKind::AngularGradient,
                DesignPaintKind::DiamondGradient,
            ]
        );
        let radial = gradient();
        assert_eq!(
            paint_for_type(&radial, DesignPaintType::Gradient, 0).kind,
            DesignPaintKind::RadialGradient
        );

        let solid = DesignPaint::solid(DesignColor::BLUE);
        let created = paint_for_type(&solid, DesignPaintType::Gradient, 0);
        assert_eq!(created.kind, DesignPaintKind::LinearGradient);
        assert_eq!(created.gradient_stops.len(), 2);
        assert_eq!(created.gradient_stops[0].color, DesignColor::BLUE);
        assert_eq!(created.gradient_stops[1].color.alpha, 0);

        let pattern = paint_for_type(&solid, DesignPaintType::Pattern, 0);
        assert_eq!(pattern.kind, DesignPaintKind::Pattern);
        let DesignPaintPayload::Pattern(pattern) = pattern.payload else {
            panic!("pattern payload");
        };
        assert!(pattern.source_node_id.is_empty());
        assert_eq!(pattern.tile_type, DesignPatternTileType::Rectangular);
        assert_eq!(pattern.scaling_factor, 1.);
        assert_eq!(pattern.spacing, DesignPatternSpacing::default());
        assert_eq!(
            pattern.horizontal_alignment,
            DesignPatternHorizontalAlignment::Center
        );

        assert_eq!(
            paint_for_kind(&solid, DesignPaintKind::DiamondGradient, 0).kind,
            DesignPaintKind::DiamondGradient
        );
        assert_eq!(
            paint_for_kind(&radial, DesignPaintKind::AngularGradient, 0).kind,
            DesignPaintKind::AngularGradient
        );
    }

    #[gpui::test]
    fn top_level_type_and_gradient_subtype_emit_distinct_typed_edits(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLUE).with_id("fill"),
                    window,
                    cx,
                );
                picker.select_paint_type(DesignPaintType::Gradient, cx);
            });
        });
        assert!(matches!(
            events.borrow().as_slice(),
            [PaintPickerEvent::Edit {
                edit,
                phase: DesignPanelEditPhase::Commit,
                ..
            }] if matches!(
                &edit.value,
                DesignPaintValue::Payload(DesignPaintPayload::Gradient(gradient))
                    if gradient.kind == DesignPaintKind::LinearGradient
                        && gradient.stops.len() == 2
            )
        ));

        events.borrow_mut().clear();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    gradient().with_id("fill"),
                    window,
                    cx,
                );
                picker.select_paint_kind(DesignPaintKind::DiamondGradient, cx);
            });
        });
        assert!(matches!(
            events.borrow().as_slice(),
            [PaintPickerEvent::Edit {
                edit,
                phase: DesignPanelEditPhase::Commit,
                ..
            }] if edit.property == DesignPaintProperty::GradientKind
                && edit.value
                    == DesignPaintValue::PaintKind(DesignPaintKind::DiamondGradient)
        ));
    }

    #[gpui::test]
    fn gradient_subtype_menu_roves_wraps_and_commits_the_highlighted_kind(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    gradient().with_id("fill"),
                    window,
                    cx,
                );
                picker.focus_handle.focus(window, cx);
                picker.open_gradient_kind_menu_from_keyboard(window, cx);
            });
        });
        visual_cx.run_until_parked();
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                assert!(picker.gradient_kind_menu_open);
                assert_eq!(picker.gradient_kind_menu_index, 1);
                picker.handle_gradient_kind_menu_key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse("home").expect("Home"),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
                picker.handle_gradient_kind_menu_key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse("up").expect("Up"),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
                assert_eq!(picker.gradient_kind_menu_index, GRADIENT_KINDS.len() - 1);
                picker.handle_gradient_kind_menu_key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse("enter").expect("Enter"),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
                assert!(!picker.gradient_kind_menu_open);
            });
        });
        visual_cx.run_until_parked();

        assert!(matches!(
            events.borrow().as_slice(),
            [PaintPickerEvent::Edit {
                edit,
                phase: DesignPanelEditPhase::Commit,
                ..
            }] if edit.property == DesignPaintProperty::GradientKind
                && edit.value
                    == DesignPaintValue::PaintKind(DesignPaintKind::DiamondGradient)
        ));
        assert!(
            visual_cx.update(|window, app| { picker.read(app).focus_handle.is_focused(window) })
        );
    }

    #[test]
    fn editing_a_stop_returns_a_candidate_without_mutating_the_source() {
        let source = gradient();
        let color = DesignColor::rgba(0x12, 0x34, 0x56, 0x78);
        let candidate = paint_with_color(&source, 1, color);

        assert_eq!(source.gradient_stops[1].color, DesignColor::WHITE);
        assert_eq!(candidate.gradient_stops[1].color, color);
        assert_eq!(candidate.color, source.gradient_stops[0].color);
    }

    #[test]
    fn gradient_stop_add_and_remove_keep_a_valid_ordered_gradient() {
        let source = gradient();
        let (added, selected) = paint_with_added_stop(&source, 0);
        assert_eq!(added.gradient_stops.len(), 3);
        assert_eq!(added.gradient_stops[selected].position, 0.5);
        assert!(
            added
                .gradient_stops
                .windows(2)
                .all(|pair| pair[0].position <= pair[1].position)
        );

        let (removed, selected) =
            paint_with_removed_stop(&added, selected).expect("three-stop gradient");
        assert_eq!(removed.gradient_stops.len(), 2);
        assert!(selected < removed.gradient_stops.len());
        assert!(paint_with_removed_stop(&removed, selected).is_none());
    }

    #[test]
    fn click_add_interpolates_at_the_exact_pointer_position() {
        let source = gradient();
        let (added, selected) = paint_with_added_stop_at(&source, 0.25);
        assert_eq!(added.gradient_stops[selected].position, 0.25);
        assert_eq!(
            added.gradient_stops[selected].color,
            mix_color(DesignColor::BLACK, DesignColor::WHITE, 0.25)
        );
    }

    #[test]
    fn click_add_selects_the_new_legacy_stop_when_semantics_duplicate_an_existing_stop() {
        let source = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK).with_id("start"),
                DesignGradientStop::new(0.5, DesignColor::rgb(0x80, 0x80, 0x80))
                    .with_id("existing-midpoint"),
                DesignGradientStop::new(1., DesignColor::WHITE).with_id("end"),
            ],
        );
        let (added, selected) = paint_with_added_stop_at(&source, 0.5);

        assert_eq!(added.gradient_stops.len(), 4);
        assert_eq!(added.gradient_stops[selected].position, 0.5);
        assert_eq!(
            added.gradient_stops[selected].color,
            DesignColor::rgb(0x80, 0x80, 0x80)
        );
        assert!(added.gradient_stops[selected].id.is_empty());
        assert_ne!(
            added.gradient_stops[selected].id,
            added.gradient_stops[selected - 1].id
        );
    }

    #[gpui::test]
    fn disabled_gradient_preview_add_does_not_drift_transient_selection(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    gradient().with_id("fill"),
                    window,
                    cx,
                );
                picker.set_disabled(true, cx);
                picker.add_gradient_stop_at(0.5, cx);
                assert_eq!(picker.selected_stop, 0);
                assert!(picker.selected_stop_id.is_empty());
                assert_eq!(
                    picker
                        .paint
                        .as_ref()
                        .map(|paint| paint.gradient_stops.len()),
                    Some(2)
                );
            });
        });
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn preview_segments_use_host_stop_positions_instead_of_equal_flex_widths() {
        let paint = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK),
                DesignGradientStop::new(0.2, DesignColor::PURPLE),
                DesignGradientStop::new(1., DesignColor::WHITE),
            ],
        );
        let segments = gradient_segments(&paint);
        assert_eq!(segments.len(), 2);
        assert_eq!((segments[0].0, segments[0].1), (0., 0.2));
        assert_eq!((segments[1].0, segments[1].1), (0.2, 1.));

        let inset = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0.2, DesignColor::BLACK),
                DesignGradientStop::new(0.8, DesignColor::WHITE),
            ],
        );
        let inset_segments = gradient_segments(&inset);
        assert_eq!(inset_segments.len(), 3);
        assert_eq!((inset_segments[0].0, inset_segments[0].1), (0., 0.2));
        assert_eq!((inset_segments[2].0, inset_segments[2].1), (0.8, 1.));
    }

    #[test]
    fn stable_picker_target_equality_ignores_reordered_indices() {
        let first = PaintPickerTarget {
            node_id: "node".into(),
            collection: DesignPanelCollection::Fill,
            index: 0,
            paint_id: "stable-paint".into(),
        };
        let moved = PaintPickerTarget {
            index: 3,
            ..first.clone()
        };
        assert_eq!(first, moved);

        let legacy = PaintPickerTarget {
            paint_id: "".into(),
            ..first
        };
        let legacy_moved = PaintPickerTarget {
            index: 3,
            ..legacy.clone()
        };
        assert_ne!(legacy, legacy_moved);
    }

    #[test]
    fn media_drop_path_helper_requires_one_host_accepted_file() {
        let editor = DesignMediaPaintCapabilities::editor();
        assert_eq!(
            media_drop_from_paths(&[PathBuf::from("replacement.PNG")], editor)
                .map(|file| file.kind),
            Some(super::super::DesignMediaFileKind::Png)
        );
        assert_eq!(
            media_drop_from_paths(
                &[PathBuf::from("one.png"), PathBuf::from("two.png")],
                editor,
            ),
            None
        );
        assert_eq!(
            media_drop_from_paths(&[PathBuf::from("vector.svg")], editor),
            None
        );
        assert_eq!(
            media_drop_from_paths(&[PathBuf::from("scan.tiff")], editor),
            None
        );
        assert_eq!(
            media_drop_from_paths(
                &[PathBuf::from("scan.TIFF")],
                editor.with_accepted_drop_file_kinds(
                    editor.accepted_drop_file_kinds | super::super::DesignMediaFileKinds::TIFF,
                ),
            )
            .map(|file| file.kind),
            Some(super::super::DesignMediaFileKind::Tiff)
        );
    }

    #[gpui::test]
    fn media_drop_emits_current_source_identity_and_allows_cross_kind_file(
        cx: &mut TestAppContext,
    ) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let events = picker_events(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::image(DesignPaintSource::new("source-image", "Cover"))
                        .with_id("stable-media"),
                    window,
                    cx,
                );
                picker.set_media_view_data(
                    DesignMediaPaintViewData::new([DesignMediaPaintView::new(
                        DesignPanelCollection::Fill,
                        "stable-media",
                        0,
                    )]),
                    cx,
                );
                picker.request_media_source_drop(&[PathBuf::from("clip.WEBM")], cx);
            });
        });
        visual_cx.run_until_parked();
        assert!(matches!(
            events.borrow().as_slice(),
            [PaintPickerEvent::MediaSourceDropRequested {
                target,
                expected_source_id,
                expected_media_kind: DesignMediaKind::Image,
                file,
            }] if target.paint_id.as_ref() == "stable-media"
                && expected_source_id.as_ref() == "source-image"
                && file.kind == super::super::DesignMediaFileKind::Webm
                && file.media_kind() == DesignMediaKind::Video
        ));
    }

    #[test]
    fn picker_locks_only_global_read_only_and_opaque_paints() {
        assert!(paint_picker_locked(
            &DesignPaint::solid(DesignColor::BLACK).with_read_only(true)
        ));
        let bound = DesignPaint::from_payload(DesignPaintPayload::Solid(DesignSolidPaint {
            color: DesignColor::PURPLE,
            binding: Some(super::super::DesignPaintBinding::new("variable", "Brand")),
        }));
        assert!(!paint_picker_locked(&bound));
        assert!(paint_color_locked(&bound, 0));
        assert!(!paint_picker_locked(&DesignPaint::shader(
            "shader",
            "Fractal noise"
        )));
        assert!(!paint_picker_locked(&DesignPaint::solid(
            DesignColor::BLACK
        )));

        let partially_bound = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK),
                DesignGradientStop::new(1., DesignColor::WHITE)
                    .with_binding(super::super::DesignPaintBinding::new("variable", "Brand")),
            ],
        );
        assert!(!paint_color_locked(&partially_bound, 0));
        assert!(paint_color_locked(&partially_bound, 1));
    }

    #[test]
    fn color_style_selection_targets_only_the_active_color_leaf() {
        assert_eq!(
            paint_color_target(&DesignPaint::solid(DesignColor::PURPLE), 0),
            Some(DesignPaintColorTarget::Solid)
        );

        let gradient = DesignPaint::gradient(
            DesignPaintKind::LinearGradient,
            vec![
                DesignGradientStop::new(0., DesignColor::BLACK).with_id("start"),
                DesignGradientStop::new(1., DesignColor::WHITE).with_id("end"),
            ],
        );
        assert_eq!(
            paint_color_target(&gradient, 1),
            Some(DesignPaintColorTarget::GradientStop {
                stop_id: "end".into(),
                index: 1,
            })
        );
        assert_eq!(
            paint_color_target(&gradient, usize::MAX),
            Some(DesignPaintColorTarget::GradientStop {
                stop_id: "end".into(),
                index: 1,
            })
        );
        assert_eq!(
            paint_color_target(
                &DesignPaint::image(DesignPaintSource::new("image", "Hero")),
                0,
            ),
            None
        );
    }

    #[test]
    fn compact_swatch_and_library_row_share_variable_identity_selection() {
        let selected = DesignPaintBinding::new("brand-primary", "Brand");
        let exact = DesignVariable::page(
            "brand-primary",
            "Brand",
            "colors",
            "Colors",
            super::super::DesignVariableResolvedType::Color,
        )
        .with_resolved_value(DesignVariableResolvedValue::Color(DesignColor::PURPLE));
        let same_color = DesignVariable::page(
            "other",
            "Other",
            "colors",
            "Colors",
            super::super::DesignVariableResolvedType::Color,
        )
        .with_resolved_value(DesignVariableResolvedValue::Color(DesignColor::PURPLE));

        assert!(paint_variable_matches_binding(&exact, Some(&selected)));
        assert!(!paint_variable_matches_binding(
            &same_color,
            Some(&selected)
        ));
        assert!(!paint_variable_matches_binding(&exact, None));
    }

    #[test]
    fn library_resource_search_matches_names_groups_and_library_case_insensitively() {
        assert!(paint_resource_matches(
            "brand",
            ["Brand / Primary", "Semantic colors", "Foundations"]
        ));
        assert!(paint_resource_matches(
            "semantic",
            ["Brand / Primary", "Semantic colors", "Foundations"]
        ));
        assert!(paint_resource_matches(
            "foundations",
            ["Brand / Primary", "Semantic colors", "Product Foundations"]
        ));
        assert!(paint_resource_matches(
            "",
            ["Brand / Primary", "Semantic colors", "Foundations"]
        ));
        assert!(!paint_resource_matches(
            "motion",
            ["Brand / Primary", "Semantic colors", "Foundations"]
        ));
    }

    #[gpui::test]
    fn library_search_is_transient_and_resets_for_a_new_paint_target(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        let search = visual_cx.read(|app| picker.read(app).resource_search_input.clone());
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill-a"),
                    window,
                    cx,
                );
            });
            search.update(app, |input, cx| {
                input.set_value("brand", window, cx);
            });
        });
        visual_cx.run_until_parked();
        assert_eq!(visual_cx.read(|app| search.read(app).value()), "brand");

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    1,
                    DesignPaint::solid(DesignColor::WHITE).with_id("fill-b"),
                    window,
                    cx,
                );
            });
        });
        visual_cx.run_until_parked();
        assert!(visual_cx.read(|app| search.read(app).value().is_empty()));
    }

    #[test]
    fn normalization_clamps_host_values_and_repairs_missing_stops() {
        let mut paint = DesignPaint::gradient(
            DesignPaintKind::AngularGradient,
            vec![
                DesignGradientStop::new(2., DesignColor::WHITE),
                DesignGradientStop::new(-1., DesignColor::BLACK),
            ],
        );
        paint.opacity = f32::NAN;
        let normalized = normalize_paint(paint);

        assert_eq!(normalized.opacity, 100.);
        assert_eq!(normalized.gradient_stops[0].position, 0.);
        assert_eq!(normalized.gradient_stops[1].position, 1.);
        assert_eq!(normalized.color, DesignColor::BLACK);
    }

    #[test]
    fn opacity_parser_is_finite_and_clamped() {
        assert_eq!(parse_opacity("42.5%"), Some(42.5));
        assert_eq!(parse_opacity("-10"), Some(0.));
        assert_eq!(parse_opacity("120"), Some(100.));
        assert_eq!(parse_opacity("NaN"), None);
        assert_eq!(parse_opacity(""), None);
        assert_eq!(parse_stop_position("37.5%"), Some(0.375));
    }

    #[test]
    fn pointer_fractions_clamp_to_the_control_bounds() {
        let bounds = Bounds {
            origin: gpui::point(px(10.), px(20.)),
            size: gpui::size(px(100.), px(50.)),
        };
        assert_eq!(fraction_x(bounds, gpui::point(px(-20.), px(0.))), 0.);
        assert_eq!(fraction_x(bounds, gpui::point(px(60.), px(0.))), 0.5);
        assert_eq!(fraction_x(bounds, gpui::point(px(200.), px(0.))), 1.);
        assert_eq!(fraction_y(bounds, gpui::point(px(0.), px(45.))), 0.5);
    }

    #[gpui::test]
    fn picker_header_tab_and_creation_trigger_activate_from_the_keyboard(cx: &mut TestAppContext) {
        let (host, visual_cx) = setup_picker(cx);
        let picker = picker(&host, visual_cx);
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.set_target(
                    "node",
                    DesignPanelCollection::Fill,
                    0,
                    DesignPaint::solid(DesignColor::BLACK).with_id("fill"),
                    window,
                    cx,
                );
                picker.focus_handle.focus(window, cx);
            });
        });
        visual_cx.run_until_parked();

        let mut libraries_activated = false;
        for _ in 0..24 {
            visual_cx.update(|window, app| {
                window.focus_next(app);
            });
            visual_cx.simulate_keystrokes("enter");
            visual_cx.run_until_parked();
            if visual_cx.read(|app| picker.read(app).active_tab == PaintPickerTab::Libraries) {
                libraries_activated = true;
                break;
            }
        }
        assert!(
            libraries_activated,
            "the Libraries header tab must be reachable with Tab and activate from Enter"
        );

        // The creation trigger opens its roving menu from the same bound
        // command instead of raw key matching.
        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                picker.active_tab = PaintPickerTab::Custom;
                cx.notify();
                picker.focus_handle.focus(window, cx);
            });
        });
        visual_cx.run_until_parked();
        let mut creation_opened = false;
        for _ in 0..24 {
            visual_cx.update(|window, app| {
                window.focus_next(app);
            });
            visual_cx.simulate_keystrokes("enter");
            visual_cx.run_until_parked();
            if visual_cx.read(|app| picker.read(app).creation_menu_open) {
                creation_opened = true;
                break;
            }
        }
        assert!(
            creation_opened,
            "the creation-menu trigger must open its menu from Enter while focused"
        );
    }
}
