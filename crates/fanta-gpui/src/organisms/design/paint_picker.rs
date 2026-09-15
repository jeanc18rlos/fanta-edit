//! Host-controlled paint-picker presentation used by the Design panel.
//!
//! The picker owns focus, draft input text, and the selected gradient stop. It
//! never mutates a document or treats its local paint snapshot as
//! authoritative: every edit emits [`PaintPickerEvent::Edit`], and the host
//! accepts that edit by calling [`PaintPicker::set_target`] with fresh data.

#[cfg(test)]
use std::path::PathBuf;

use gpui::{
    Anchor, AnyElement, App, AppContext as _, Bounds, Context, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    canvas, div, linear_color_stop, linear_gradient, pattern_slash, point,
    prelude::FluentBuilder as _, px, relative,
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

use crate::atoms::{
    ActivateControl, ButtonControlExt as _, CONTROL_KEY_CONTEXT, tokens, track_bounds,
};
use crate::molecules::{
    InspectorOverlayDismissCause, InspectorOverlayDismissIntent, InspectorOverlayFocusTarget,
    popup_height, popup_width,
};

mod contrast;
mod gradient;
mod media;
mod resource_browser;
mod shader;
mod solid_color;

use gradient::*;
#[cfg(test)]
use media::CROP_ROTATION_STEP_DEGREES;
pub(crate) use media::media_drop_from_paths;
#[cfg(test)]
use media::rotated_crop_preview;
use solid_color::*;

#[cfg(test)]
use super::DesignMediaPaintCapabilities;
use super::{
    CancelDesignInteraction, DesignBlendMode, DesignColor, DesignColorContrastCategory,
    DesignColorContrastLeafViewData, DesignColorContrastLevel, DesignColorContrastPaintViewData,
    DesignColorStyleSample, DesignColorStyleSampleSelection, DesignColorStyleSampleViewData,
    DesignGradientPaint, DesignGradientStop, DesignImageFilter, DesignImageFilters,
    DesignImagePaint, DesignMediaCropAction, DesignMediaCropToolState, DesignMediaDroppedFile,
    DesignMediaKind, DesignMediaPaintPlacement, DesignMediaPaintScaleMode, DesignMediaPaintView,
    DesignMediaPaintViewData, DesignMediaSourceAction, DesignMenuPreviewPhase, DesignPaint,
    DesignPaintBinding, DesignPaintColorTarget, DesignPaintEdit, DesignPaintKind,
    DesignPaintPayload, DesignPaintProperty, DesignPaintSource, DesignPaintTransform,
    DesignPaintType, DesignPaintValue, DesignPaintVariableViewData, DesignPanelCollection,
    DesignPanelEditPhase, DesignPatternHorizontalAlignment, DesignPatternPaint,
    DesignPatternSpacing, DesignPatternTileType, DesignShaderDefinition, DesignShaderPaint,
    DesignShaderPropertyDefinition, DesignShaderPropertyValue, DesignShaderSelection,
    DesignShaderViewData, DesignSolidPaint, DesignVariable, DesignVariableImportState,
    DesignVariableResolvedValue, DesignVariableSource, DesignVideoPaint, DesignVideoPreviewAction,
    DesignVideoPreviewState, DesignVideoPreviewStatus,
};

const PICKER_WIDTH: f32 = tokens::MenuWidth::PICKER;
const PICKER_MAX_HEIGHT: f32 = 520.;
const PICKER_HEADER_HEIGHT: f32 = tokens::RowHeight::SECTION_HEADER;
const PAINT_TYPE_ROW_HEIGHT: f32 = tokens::RowHeight::SECTION_HEADER;
const PAINT_HEADER_CONTROL_SIZE: f32 = tokens::ControlSize::CHROME;
const PAINT_HEADER_GAP: f32 = tokens::Space::XS;
const PAINT_HEADER_HORIZONTAL_PADDING: f32 = tokens::Space::LG;
const PAINT_HEADER_TRAILING_CONTROL_COUNT: usize = 2;
const COLOR_AREA_HEIGHT: f32 = 248.;
const CONTROL_HEIGHT: f32 = tokens::ControlSize::INLINE;
const KEYBOARD_FINE_STEP: f32 = 0.01;
const KEYBOARD_COARSE_STEP: f32 = 0.1;
const HUE_FINE_STEP: f32 = 1.;
const HUE_COARSE_STEP: f32 = 10.;

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

#[derive(Clone, Debug, PartialEq)]
struct ContinuousPaintEdit {
    edit: DesignPaintEdit,
}

/// One nested surface retained by the paint picker.
///
/// The outer Design inspector coordinates the paint-picker popover itself;
/// this identity is deliberately local to the retained child and covers only
/// menus rendered inside that popover.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaintPickerOverlay {
    BlendMode,
    ColorFormat,
    GradientKind,
    Creation,
    ResourceScope,
}

#[derive(Clone, Debug)]
struct OpenPaintPickerOverlay {
    overlay: PaintPickerOverlay,
    focus_return: Option<InspectorOverlayFocusTarget>,
}

/// Typed stack for nested paint-picker menus.
///
/// The most recently opened surface is deterministically topmost. Covered
/// ancestors therefore cannot also react to the same Escape or outside click,
/// and every close path shares the inspector focus-return contract.
#[derive(Default)]
struct PaintPickerOverlayCoordinator {
    open: Vec<OpenPaintPickerOverlay>,
}

impl PaintPickerOverlayCoordinator {
    fn is_open(&self, overlay: PaintPickerOverlay) -> bool {
        self.open.iter().any(|entry| entry.overlay == overlay)
    }

    fn topmost(&self) -> Option<PaintPickerOverlay> {
        self.open.last().map(|entry| entry.overlay)
    }

    fn open(
        &mut self,
        overlay: PaintPickerOverlay,
        focus_return: Option<InspectorOverlayFocusTarget>,
    ) {
        let retained_focus = self
            .open
            .iter()
            .position(|entry| entry.overlay == overlay)
            .and_then(|index| self.open.remove(index).focus_return);
        self.open.push(OpenPaintPickerOverlay {
            overlay,
            focus_return: focus_return.or(retained_focus),
        });
    }

    fn dismissal_intent(
        &self,
        overlay: PaintPickerOverlay,
        cause: InspectorOverlayDismissCause,
    ) -> Option<InspectorOverlayDismissIntent<PaintPickerOverlay>> {
        (self.topmost() == Some(overlay)).then(|| {
            let focus_return = self
                .open
                .last()
                .and_then(|entry| entry.focus_return.clone());
            InspectorOverlayDismissIntent::new(overlay, cause, focus_return)
        })
    }

    fn close(
        &mut self,
        overlay: PaintPickerOverlay,
    ) -> Option<Option<InspectorOverlayFocusTarget>> {
        let index = self
            .open
            .iter()
            .position(|entry| entry.overlay == overlay)?;
        Some(self.open.remove(index).focus_return)
    }

    fn finish_dismissal(&mut self, intent: &InspectorOverlayDismissIntent<PaintPickerOverlay>) {
        let _ = self.close(*intent.overlay());
    }

    fn clear(&mut self) {
        self.open.clear();
    }
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
    color_format_menu_index: usize,
    color_format_menu_focus_handle: FocusHandle,
    creation_menu_index: usize,
    creation_menu_focus_handle: FocusHandle,
    resource_scope: PaintResourceScope,
    resource_scope_menu_index: usize,
    resource_scope_menu_focus_handle: FocusHandle,
    gradient_kind_menu_index: usize,
    gradient_kind_menu_focus_handle: FocusHandle,
    nested_overlays: PaintPickerOverlayCoordinator,
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
            color_format_menu_index: ColorFormat::Hex.index(),
            color_format_menu_focus_handle: cx.focus_handle(),
            creation_menu_index: PaintCreationKind::Style as usize,
            creation_menu_focus_handle: cx.focus_handle(),
            resource_scope: PaintResourceScope::Page,
            resource_scope_menu_index: 0,
            resource_scope_menu_focus_handle: cx.focus_handle(),
            gradient_kind_menu_index: 0,
            gradient_kind_menu_focus_handle: cx.focus_handle(),
            nested_overlays: PaintPickerOverlayCoordinator::default(),
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

    fn nested_overlay_is_open(&self, overlay: PaintPickerOverlay) -> bool {
        self.nested_overlays.is_open(overlay)
    }

    fn open_nested_overlay(&mut self, overlay: PaintPickerOverlay, window: &Window, cx: &App) {
        self.nested_overlays.open(
            overlay,
            window.focused(cx).map(InspectorOverlayFocusTarget::new),
        );
    }

    fn cleanup_nested_overlay(&mut self, overlay: PaintPickerOverlay, cx: &mut Context<Self>) {
        if overlay == PaintPickerOverlay::BlendMode {
            self.cancel_blend_mode_preview(cx);
        }
    }

    fn close_nested_overlay(
        &mut self,
        overlay: PaintPickerOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(focus_return) = self.nested_overlays.close(overlay) else {
            return false;
        };
        self.cleanup_nested_overlay(overlay, cx);
        if let Some(focus_return) = focus_return {
            focus_return.restore(window, cx);
        }
        cx.notify();
        true
    }

    fn dismiss_nested_overlay(
        &mut self,
        overlay: PaintPickerOverlay,
        cause: InspectorOverlayDismissCause,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(intent) = self.nested_overlays.dismissal_intent(overlay, cause) else {
            return false;
        };
        self.cleanup_nested_overlay(overlay, cx);
        self.nested_overlays.finish_dismissal(&intent);
        intent.restore_focus(window, cx);
        cx.notify();
        true
    }

    fn forget_nested_overlay(&mut self, overlay: PaintPickerOverlay) -> bool {
        self.nested_overlays.close(overlay).is_some()
    }

    fn clear_nested_overlays(&mut self) {
        self.nested_overlays.clear();
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
            self.clear_nested_overlays();
            self.creation_menu_index = PaintCreationKind::Style as usize;
            self.gradient_kind_menu_index = 0;
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
        self.clear_nested_overlays();
        self.creation_menu_index = PaintCreationKind::Style as usize;
        self.resource_scope = PaintResourceScope::Page;
        self.resource_scope_menu_index = 0;
        self.gradient_kind_menu_index = 0;
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
                for overlay in [
                    PaintPickerOverlay::Creation,
                    PaintPickerOverlay::ResourceScope,
                    PaintPickerOverlay::GradientKind,
                    PaintPickerOverlay::BlendMode,
                ] {
                    self.forget_nested_overlay(overlay);
                }
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
        for overlay in [
            PaintPickerOverlay::Creation,
            PaintPickerOverlay::ResourceScope,
            PaintPickerOverlay::GradientKind,
            PaintPickerOverlay::BlendMode,
        ] {
            self.forget_nested_overlay(overlay);
        }
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
            self.clear_nested_overlays();
            self.scroll_handle.set_offset(Point::default());
            cx.notify();
        }
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

    fn select_blend_mode(
        &mut self,
        blend_mode: DesignBlendMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.close_nested_overlay(PaintPickerOverlay::BlendMode, window, cx) {
            // Programmatic activation can exercise the same preview lifecycle
            // without mounting the menu; the terminal End remains mandatory.
            self.cancel_blend_mode_preview(cx);
        }
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
                .selected(self.nested_overlay_is_open(PaintPickerOverlay::BlendMode))
                .disabled(self.editing_disabled())
                .child(self.render_blend_mode_mark(cx))
                .on_keyboard_activate(move |window, cx| {
                    picker_for_trigger.update(cx, |this, cx| {
                        if this.nested_overlay_is_open(PaintPickerOverlay::BlendMode) {
                            this.close_nested_overlay(PaintPickerOverlay::BlendMode, window, cx);
                        } else {
                            this.open_nested_overlay(PaintPickerOverlay::BlendMode, window, cx);
                            cx.notify();
                        }
                    });
                });

        let blend = Popover::new(SharedString::from(format!(
            "{}-paint-blend-mode-menu",
            self.id
        )))
        .anchor(Anchor::BottomRight)
        .open(self.nested_overlay_is_open(PaintPickerOverlay::BlendMode))
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            picker_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_nested_overlay(PaintPickerOverlay::BlendMode, window, cx);
                    cx.notify();
                } else {
                    this.dismiss_nested_overlay(
                        PaintPickerOverlay::BlendMode,
                        InspectorOverlayDismissCause::OutsideClick,
                        window,
                        cx,
                    );
                }
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
                    .on_activate(move |_, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.select_blend_mode(mode, window, cx);
                        });
                    })
                    // Enter/Space activate through the bound `ActivateControl`
                    // command above; only dismissal and focus-move previews
                    // stay key-matched.
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        picker_for_key.update(cx, |this, cx| {
                            this.set_blend_mode_preview(mode, true, cx);
                            match event.keystroke.key.as_str() {
                                "escape"
                                    if this.dismiss_nested_overlay(
                                        PaintPickerOverlay::BlendMode,
                                        InspectorOverlayDismissCause::Escape,
                                        window,
                                        cx,
                                    ) =>
                                {
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
    fn nested_overlay_coordinator_dismisses_only_the_topmost_surface() {
        let mut overlays = PaintPickerOverlayCoordinator::default();
        overlays.open(PaintPickerOverlay::ColorFormat, None);
        overlays.open(PaintPickerOverlay::Creation, None);

        assert_eq!(overlays.topmost(), Some(PaintPickerOverlay::Creation));
        assert!(
            overlays
                .dismissal_intent(
                    PaintPickerOverlay::ColorFormat,
                    InspectorOverlayDismissCause::OutsideClick,
                )
                .is_none(),
            "a covered surface must not react to the topmost surface's outside click"
        );

        let intent = overlays
            .dismissal_intent(
                PaintPickerOverlay::Creation,
                InspectorOverlayDismissCause::Escape,
            )
            .expect("the topmost surface should accept Escape");
        assert_eq!(intent.cause(), InspectorOverlayDismissCause::Escape);
        overlays.finish_dismissal(&intent);

        assert!(!overlays.is_open(PaintPickerOverlay::Creation));
        assert!(overlays.is_open(PaintPickerOverlay::ColorFormat));
        assert_eq!(overlays.topmost(), Some(PaintPickerOverlay::ColorFormat));
    }

    #[gpui::test]
    fn nested_overlay_dismissal_unwinds_focus_in_stack_order(cx: &mut TestAppContext) {
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

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
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

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                assert!(!picker.dismiss_nested_overlay(
                    PaintPickerOverlay::ColorFormat,
                    InspectorOverlayDismissCause::OutsideClick,
                    window,
                    cx,
                ));
                assert!(picker.dismiss_nested_overlay(
                    PaintPickerOverlay::Creation,
                    InspectorOverlayDismissCause::Escape,
                    window,
                    cx,
                ));
            });
        });
        visual_cx.run_until_parked();
        visual_cx.read(|app| {
            let picker = picker.read(app);
            assert!(picker.nested_overlay_is_open(PaintPickerOverlay::ColorFormat));
            assert!(!picker.nested_overlay_is_open(PaintPickerOverlay::Creation));
        });
        assert!(visual_cx.update(|window, app| {
            picker
                .read(app)
                .color_format_menu_focus_handle
                .is_focused(window)
        }));

        visual_cx.update(|window, app| {
            picker.update(app, |picker, cx| {
                assert!(picker.dismiss_nested_overlay(
                    PaintPickerOverlay::ColorFormat,
                    InspectorOverlayDismissCause::OutsideClick,
                    window,
                    cx,
                ));
            });
        });
        visual_cx.run_until_parked();
        assert!(
            visual_cx.update(|window, app| { picker.read(app).focus_handle.is_focused(window) })
        );
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
                picker.select_blend_mode(DesignBlendMode::Multiply, window, cx);
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
            assert!(!picker.nested_overlay_is_open(PaintPickerOverlay::ColorFormat));
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
            assert!(!picker.nested_overlay_is_open(PaintPickerOverlay::ColorFormat));
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
            assert!(!picker.nested_overlay_is_open(PaintPickerOverlay::ResourceScope));
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
                picker.select_blend_mode(DesignBlendMode::Multiply, window, cx);
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
                assert!(picker.nested_overlay_is_open(PaintPickerOverlay::GradientKind));
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
                assert!(!picker.nested_overlay_is_open(PaintPickerOverlay::GradientKind));
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
            if visual_cx.read(|app| {
                picker
                    .read(app)
                    .nested_overlay_is_open(PaintPickerOverlay::Creation)
            }) {
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
