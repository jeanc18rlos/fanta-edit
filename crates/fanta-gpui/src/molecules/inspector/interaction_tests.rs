use std::{cell::RefCell, rc::Rc};

use gpui::{
    Anchor, Context, EventEmitter, FocusHandle, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, Modifiers, MouseDownEvent, ParentElement as _, Render, SharedString, Styled as _,
    TestAppContext, VisualTestContext, Window, div, point, prelude::FluentBuilder as _, px, size,
};
use gpui_component::{Theme, ThemeMode, v_flex};

use crate::atoms::{CONTROL_KEY_CONTEXT, ControlExt as _};
use crate::test_support::{assert_pointer_and_keyboard_parity, mount_component};

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
enum SpecimenAction {
    Activated,
    SegmentSelected,
    CollectionSelected,
    ControlledPicker(InspectorControlledEdit<SharedString>),
}

struct InspectorSpecimen {
    root_focus: FocusHandle,
    action_focus: FocusHandle,
    overlay_trigger_focus: FocusHandle,
    grouped_read_only_focus: FocusHandle,
    grouped_disabled_focus: FocusHandle,
    overlay_focus: FocusHandle,
    overlay_open: bool,
    last_overlay_dismiss_cause: Option<InspectorOverlayDismissCause>,
}

impl EventEmitter<SpecimenAction> for InspectorSpecimen {}

impl InspectorSpecimen {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            root_focus: cx.focus_handle(),
            action_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            overlay_trigger_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            grouped_read_only_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            grouped_disabled_focus: cx.focus_handle(),
            overlay_focus: cx.focus_handle(),
            overlay_open: false,
            last_overlay_dismiss_cause: None,
        }
    }

    fn dismiss_overlay(
        &mut self,
        cause: InspectorOverlayDismissCause,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.overlay_open {
            return;
        }
        let dismissal = InspectorOverlayDismissIntent::new(
            "popover",
            cause,
            Some(InspectorOverlayFocusTarget::new(
                self.overlay_trigger_focus.clone(),
            )),
        );
        self.overlay_open = false;
        self.last_overlay_dismiss_cause = Some(dismissal.cause());
        cx.notify();
        assert!(dismissal.restore_focus(window, cx));
    }
}

impl Render for InspectorSpecimen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = InspectorMetrics::default();
        let action = inspector_action_button(
            "inspector-test-action",
            &InspectorFieldAccess::Editable,
            metrics,
            cx,
        )
        .debug_selector(|| "inspector-test-action".to_owned())
        .track_focus(&self.action_focus)
        .on_activate(cx.listener(|_, _, _, cx| cx.emit(SpecimenAction::Activated)))
        .child("Apply");

        let overlay_trigger = inspector_action_button(
            "inspector-test-overlay-trigger",
            &InspectorFieldAccess::Editable,
            metrics,
            cx,
        )
        .debug_selector(|| "inspector-test-overlay-trigger".to_owned())
        .track_focus(&self.overlay_trigger_focus)
        .on_activate(cx.listener(|this, _, window, cx| {
            this.overlay_open = true;
            this.overlay_focus.focus(window, cx);
            cx.notify();
        }))
        .child("Open");

        let disabled_menu_item = inspector_menu_item(
            "inspector-test-disabled",
            &InspectorFieldAccess::disabled(Some("Unavailable".into())),
            metrics,
            cx,
        )
        .debug_selector(|| "inspector-test-disabled".to_owned())
        .child("Unavailable");
        let invalid_number = InspectorNumberField::new(
            InspectorValue::Uniform(12.),
            InspectorFieldPresentation::new(InspectorFieldAccess::Editable).invalid(true),
        )
        .render("inspector-test-invalid-number", metrics, cx)
        .debug_selector(|| "inspector-test-invalid-number".to_owned())
        .child("Not a number");
        let mixed_field = InspectorTextField::new(
            InspectorValue::<SharedString>::Mixed,
            InspectorFieldPresentation::new(InspectorFieldAccess::Editable),
        );
        let mixed_value = mixed_field.frame().value().clone();
        let mixed_field = mixed_field
            .render("inspector-test-mixed-value", metrics, cx)
            .debug_selector(|| "inspector-test-mixed-value".to_owned())
            .child(match mixed_value {
                InspectorValue::Unset => SharedString::from("—"),
                InspectorValue::Mixed => SharedString::from("Mixed"),
                InspectorValue::Uniform(value) => value,
            });
        let controlled_picker = InspectorPickerField::new(
            InspectorValue::<SharedString>::Mixed,
            InspectorFieldPresentation::new(InspectorFieldAccess::Editable).bound(true),
        );
        let controlled_picker_edit = controlled_picker
            .select(SharedString::from("Chosen"))
            .expect("an editable picker accepts an immediate choice");
        let controlled_picker = controlled_picker
            .render("inspector-test-controlled-picker", metrics, cx)
            .debug_selector(|| "inspector-test-controlled-picker".to_owned())
            .on_activate(cx.listener(move |_, _, _, cx| {
                cx.emit(SpecimenAction::ControlledPicker(
                    controlled_picker_edit.clone(),
                ));
            }))
            .child("Choose");
        let segments = inspector_segmented_control(metrics, cx)
            .child(
                inspector_segment(
                    "inspector-test-segment-selected",
                    true,
                    &InspectorFieldAccess::Editable,
                    metrics,
                    cx,
                )
                .child("Left"),
            )
            .child(
                inspector_segment(
                    "inspector-test-segment-action",
                    false,
                    &InspectorFieldAccess::Editable,
                    metrics,
                    cx,
                )
                .debug_selector(|| "inspector-test-segment-action".to_owned())
                .on_activate(cx.listener(|_, _, _, cx| {
                    cx.emit(SpecimenAction::SegmentSelected);
                }))
                .child("Right"),
            );
        let collection_row = inspector_collection_row(
            "inspector-test-collection-action",
            false,
            &InspectorFieldAccess::Editable,
            metrics,
            cx,
        )
        .debug_selector(|| "inspector-test-collection-action".to_owned())
        .on_activate(cx.listener(|_, _, _, cx| {
            cx.emit(SpecimenAction::CollectionSelected);
        }))
        .child("Collection row");
        let grouped_fields = inspector_field_group(metrics, cx)
            .debug_selector(|| "inspector-test-field-group".to_owned())
            .child(
                inspector_grouped_field_frame(
                    "inspector-test-grouped-mixed",
                    &InspectorFieldPresentation::new(InspectorFieldAccess::Editable),
                    metrics,
                    cx,
                )
                .debug_selector(|| "inspector-test-grouped-mixed".to_owned())
                .flex_1()
                .child("Mixed"),
            )
            .child(
                inspector_grouped_field_frame(
                    "inspector-test-grouped-read-only",
                    &InspectorFieldPresentation::new(InspectorFieldAccess::read_only(Some(
                        "Inherited".into(),
                    )))
                    .bound(true),
                    metrics,
                    cx,
                )
                .debug_selector(|| "inspector-test-grouped-read-only".to_owned())
                .track_focus(&self.grouped_read_only_focus)
                .flex_1()
                .child("Bound"),
            )
            .child(
                inspector_grouped_field_frame(
                    "inspector-test-grouped-disabled",
                    &InspectorFieldPresentation::new(InspectorFieldAccess::disabled(Some(
                        "Unavailable".into(),
                    ))),
                    metrics,
                    cx,
                )
                .debug_selector(|| "inspector-test-grouped-disabled".to_owned())
                .track_focus(&self.grouped_disabled_focus)
                .flex_1()
                .child("Disabled"),
            );

        v_flex()
            .id("inspector-test-root")
            .debug_selector(|| "inspector-test-root".to_owned())
            .key_context(CONTROL_KEY_CONTEXT)
            .relative()
            .size_full()
            .track_focus(&self.root_focus)
            .gap_2()
            .p_4()
            .child(
                inspector_section_group().child(
                    inspector_section(cx)
                        .child(
                            inspector_section_header("inspector-test-header", metrics, cx)
                                .child("Section"),
                        )
                        .child(
                            inspector_field_grid(metrics)
                                .child(inspector_row(metrics).child(action).child(overlay_trigger))
                                .child(invalid_number)
                                .child(mixed_field)
                                .child(controlled_picker)
                                .child(segments)
                                .child(collection_row)
                                .child(grouped_fields)
                                .child(disabled_menu_item)
                                .child(
                                    div()
                                        .debug_selector(|| "inspector-test-validation".to_owned())
                                        .child(InspectorFieldMessage::validation("Invalid value")),
                                ),
                        ),
                ),
            )
            .when(self.overlay_open, |root| {
                root.child(inspector_anchored_popover(
                    Anchor::TopLeft,
                    point(px(24.), px(100.)),
                    1,
                    inspector_popover_surface("inspector-test-overlay", metrics, cx)
                        .debug_selector(|| "inspector-test-overlay".to_owned())
                        .key_context(CONTROL_KEY_CONTEXT)
                        .tab_index(0)
                        .track_focus(&self.overlay_focus)
                        .w(px(180.))
                        .h(px(80.))
                        .p_2()
                        .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.dismiss_overlay(
                                InspectorOverlayDismissCause::OutsideClick,
                                window,
                                cx,
                            );
                        }))
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                this.dismiss_overlay(
                                    InspectorOverlayDismissCause::Escape,
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }
                        }))
                        .child("Popover"),
                ))
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FieldSpecimenAction {
    Text(InspectorEditPhase),
    Number(InspectorEditPhase),
    Slider(InspectorEditPhase),
    Toggle {
        phase: InspectorEditPhase,
        value: bool,
    },
    Checkbox {
        phase: InspectorEditPhase,
        value: bool,
    },
    Color(InspectorEditPhase),
    AccessViolation,
}

struct InspectorFieldSpecimen {
    text: InspectorTextField,
    number: InspectorNumberField,
    slider: InspectorSliderField,
    toggle: InspectorToggleField,
    checkbox: InspectorCheckboxField,
    color: InspectorColorField,
    read_only_text: InspectorTextField,
    disabled_number: InspectorNumberField,
    text_focus: FocusHandle,
    read_only_focus: FocusHandle,
    disabled_focus: FocusHandle,
}

impl EventEmitter<FieldSpecimenAction> for InspectorFieldSpecimen {}

impl InspectorFieldSpecimen {
    fn editable() -> InspectorFieldPresentation {
        InspectorFieldPresentation::new(InspectorFieldAccess::Editable)
    }

    fn red() -> Hsla {
        Hsla {
            h: 0.,
            s: 1.,
            l: 0.5,
            a: 1.,
        }
    }

    fn blue() -> Hsla {
        Hsla {
            h: 2. / 3.,
            s: 1.,
            l: 0.5,
            a: 1.,
        }
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            text: InspectorTextField::new(
                InspectorValue::Uniform("before".into()),
                Self::editable(),
            ),
            number: InspectorNumberField::new(InspectorValue::Uniform(10.), Self::editable()),
            slider: InspectorSliderField::new(InspectorValue::Uniform(0.25), Self::editable()),
            toggle: InspectorToggleField::new(InspectorValue::Uniform(false), Self::editable()),
            checkbox: InspectorCheckboxField::new(InspectorValue::Mixed, Self::editable()),
            color: InspectorColorField::new(InspectorValue::Uniform(Self::red()), Self::editable()),
            read_only_text: InspectorTextField::new(
                InspectorValue::Uniform("bound".into()),
                InspectorFieldPresentation::new(InspectorFieldAccess::read_only(Some(
                    "Inherited".into(),
                )))
                .bound(true),
            ),
            disabled_number: InspectorNumberField::new(
                InspectorValue::Uniform(2.),
                InspectorFieldPresentation::new(InspectorFieldAccess::disabled(Some(
                    "Unavailable".into(),
                ))),
            ),
            text_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            read_only_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            disabled_focus: cx.focus_handle(),
        }
    }

    fn activate_text(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = self.text.begin_with("before") {
            cx.emit(FieldSpecimenAction::Text(edit.phase));
        }
        if let Some(edit) = self.text.preview("after") {
            cx.emit(FieldSpecimenAction::Text(edit.phase));
        }
        if let Some(edit) = self.text.commit() {
            cx.emit(FieldSpecimenAction::Text(edit.phase));
        }
    }

    fn activate_number(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = self.number.begin_with(10., "10") {
            cx.emit(FieldSpecimenAction::Number(edit.phase));
        }
        assert!(self.number.set_text("12"));
        if let Some(edit) = self.number.preview_with(|text| text.parse().ok()) {
            cx.emit(FieldSpecimenAction::Number(edit.phase));
        }
        if let Some(edit) = self.number.commit_with(|text| text.parse().ok()) {
            cx.emit(FieldSpecimenAction::Number(edit.phase));
        }
    }

    fn activate_slider(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = self.slider.begin(0.25) {
            cx.emit(FieldSpecimenAction::Slider(edit.phase));
        }
        if let Some(edit) = self.slider.preview(0.75) {
            cx.emit(FieldSpecimenAction::Slider(edit.phase));
        }
        if let Some(edit) = self.slider.commit() {
            cx.emit(FieldSpecimenAction::Slider(edit.phase));
        }
    }

    fn activate_toggle(&self, cx: &mut Context<Self>) {
        if let Some(edit) = self.toggle.toggle()
            && let InspectorValue::Uniform(value) = edit.value
        {
            cx.emit(FieldSpecimenAction::Toggle {
                phase: edit.phase,
                value,
            });
        }
    }

    fn activate_checkbox(&self, cx: &mut Context<Self>) {
        if let Some(edit) = self.checkbox.toggle()
            && let InspectorValue::Uniform(value) = edit.value
        {
            cx.emit(FieldSpecimenAction::Checkbox {
                phase: edit.phase,
                value,
            });
        }
    }

    fn activate_color(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = self.color.begin(Self::red()) {
            cx.emit(FieldSpecimenAction::Color(edit.phase));
        }
        if let Some(edit) = self.color.preview(Self::blue()) {
            cx.emit(FieldSpecimenAction::Color(edit.phase));
        }
        if let Some(edit) = self.color.commit() {
            cx.emit(FieldSpecimenAction::Color(edit.phase));
        }
    }
}

impl Render for InspectorFieldSpecimen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = InspectorMetrics::current();
        let mut swatch_metrics = metrics;
        swatch_metrics.row_height = px(16.);
        let swatch =
            InspectorColorSwatch::new(InspectorValue::Uniform(Self::red()), Self::editable())
                .render(Self::red(), swatch_metrics, cx);

        inspector_field_grid(metrics)
            .id("inspector-field-specimen")
            .child(
                self.text
                    .render("inspector-field-text", metrics, cx)
                    .debug_selector(|| "inspector-field-text".to_owned())
                    .track_focus(&self.text_focus)
                    .on_activate(cx.listener(|this, _, _, cx| this.activate_text(cx)))
                    .child("Text"),
            )
            .child(
                self.number
                    .render("inspector-field-number", metrics, cx)
                    .debug_selector(|| "inspector-field-number".to_owned())
                    .on_activate(cx.listener(|this, _, _, cx| this.activate_number(cx)))
                    .child("Number"),
            )
            .child(
                self.slider
                    .render("inspector-field-slider", metrics, cx)
                    .debug_selector(|| "inspector-field-slider".to_owned())
                    .on_activate(cx.listener(|this, _, _, cx| this.activate_slider(cx)))
                    .child("Slider"),
            )
            .child(
                self.toggle
                    .render("inspector-field-toggle", metrics, cx)
                    .debug_selector(|| "inspector-field-toggle".to_owned())
                    .on_activate(cx.listener(|this, _, _, cx| this.activate_toggle(cx)))
                    .child("Toggle"),
            )
            .child(
                self.checkbox
                    .render("inspector-field-checkbox", metrics, cx)
                    .debug_selector(|| "inspector-field-checkbox".to_owned())
                    .on_activate(cx.listener(|this, _, _, cx| this.activate_checkbox(cx)))
                    .child("Checkbox"),
            )
            .child(
                self.color
                    .render("inspector-field-color", metrics, cx)
                    .debug_selector(|| "inspector-field-color".to_owned())
                    .on_activate(cx.listener(|this, _, _, cx| this.activate_color(cx)))
                    .child(swatch)
                    .child("Color"),
            )
            .child(
                self.read_only_text
                    .render("inspector-field-read-only", metrics, cx)
                    .debug_selector(|| "inspector-field-read-only".to_owned())
                    .track_focus(&self.read_only_focus)
                    .on_activate(cx.listener(|this, _, _, cx| {
                        if this.read_only_text.begin().is_some() {
                            cx.emit(FieldSpecimenAction::AccessViolation);
                        }
                    }))
                    .child("Read only"),
            )
            .child(
                self.disabled_number
                    .render("inspector-field-disabled", metrics, cx)
                    .debug_selector(|| "inspector-field-disabled".to_owned())
                    .track_focus(&self.disabled_focus)
                    .on_activate(cx.listener(|this, _, _, cx| {
                        if this.disabled_number.begin("2").is_some() {
                            cx.emit(FieldSpecimenAction::AccessViolation);
                        }
                    }))
                    .child("Disabled"),
            )
    }
}

fn mount_fields(
    cx: &mut TestAppContext,
) -> crate::test_support::Mounted<'_, InspectorFieldSpecimen, FieldSpecimenAction> {
    mount_component(cx, |_, cx| InspectorFieldSpecimen::new(cx))
}

fn assert_field_activation_sequence(
    cx: &mut VisualTestContext,
    selector: &'static str,
    actions: &Rc<RefCell<Vec<FieldSpecimenAction>>>,
    expected: &[FieldSpecimenAction],
) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("field `{selector}` should render"));
    actions.borrow_mut().clear();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(actions.borrow().as_slice(), expected, "pointer: {selector}");

    for key in ["enter", "space"] {
        actions.borrow_mut().clear();
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        assert_eq!(actions.borrow().as_slice(), expected, "{key}: {selector}");
    }
    actions.borrow_mut().clear();
}

#[gpui::test]
fn rendered_fields_preserve_access_focus_and_edit_phase_contracts(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount_fields(cx);
    let component = cx.read(|app| host.read(app).component.clone());
    let lifecycle = |kind: fn(InspectorEditPhase) -> FieldSpecimenAction| {
        [
            kind(InspectorEditPhase::Begin),
            kind(InspectorEditPhase::Preview),
            kind(InspectorEditPhase::Commit),
        ]
    };

    assert_field_activation_sequence(
        cx,
        "inspector-field-text",
        &actions,
        &lifecycle(FieldSpecimenAction::Text),
    );
    assert_field_activation_sequence(
        cx,
        "inspector-field-number",
        &actions,
        &lifecycle(FieldSpecimenAction::Number),
    );
    assert_field_activation_sequence(
        cx,
        "inspector-field-slider",
        &actions,
        &lifecycle(FieldSpecimenAction::Slider),
    );
    assert_field_activation_sequence(
        cx,
        "inspector-field-toggle",
        &actions,
        &[FieldSpecimenAction::Toggle {
            phase: InspectorEditPhase::Commit,
            value: true,
        }],
    );
    assert_field_activation_sequence(
        cx,
        "inspector-field-checkbox",
        &actions,
        &[FieldSpecimenAction::Checkbox {
            phase: InspectorEditPhase::Commit,
            value: true,
        }],
    );
    assert_field_activation_sequence(
        cx,
        "inspector-field-color",
        &actions,
        &lifecycle(FieldSpecimenAction::Color),
    );

    for selector in ["inspector-field-read-only", "inspector-field-disabled"] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("field `{selector}` should render"));
        actions.borrow_mut().clear();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(
            actions.borrow().is_empty(),
            "{selector} must not begin an edit"
        );
    }

    let (text_focus, read_only_focus, disabled_focus) = cx.read(|app| {
        let specimen = component.read(app);
        (
            specimen.text_focus.clone(),
            specimen.read_only_focus.clone(),
            specimen.disabled_focus.clone(),
        )
    });
    cx.update(|window, app| {
        read_only_focus.focus(window, app);
        assert!(read_only_focus.is_focused(window));
        window.focus_next(app);
        assert!(!disabled_focus.is_focused(window));
        text_focus.focus(window, app);
        assert!(text_focus.is_focused(window));
    });

    for selector in [
        "inspector-field-text",
        "inspector-field-number",
        "inspector-field-slider",
        "inspector-field-toggle",
        "inspector-field-checkbox",
        "inspector-field-color",
    ] {
        assert_eq!(
            cx.debug_bounds(selector)
                .expect("rendered shared field")
                .size
                .height,
            px(InspectorMetrics::ROW_HEIGHT),
        );
    }
}

#[gpui::test]
fn grouped_fields_have_one_chrome_owner_and_uniform_geometry(cx: &mut TestAppContext) {
    let (host, _actions, cx) = mount(cx);
    let component = cx.read(|app| host.read(app).component.clone());

    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|_, app| Theme::change(mode, None, app));
        for width in [320., 400., 472.] {
            cx.simulate_resize(size(px(width), px(300.)));
            cx.run_until_parked();

            let group = cx
                .debug_bounds("inspector-test-field-group")
                .expect("the shared field group should render");
            let mixed = cx
                .debug_bounds("inspector-test-grouped-mixed")
                .expect("the mixed field should render");
            let read_only = cx
                .debug_bounds("inspector-test-grouped-read-only")
                .expect("the read-only field should render");
            let disabled = cx
                .debug_bounds("inspector-test-grouped-disabled")
                .expect("the disabled field should render");

            assert_eq!(group.size.height, px(24.));
            assert_eq!(mixed.size.height, group.size.height);
            assert_eq!(read_only.size.height, group.size.height);
            assert_eq!(disabled.size.height, group.size.height);
            assert_eq!(mixed.left(), group.left() + px(1.));
            assert_eq!(mixed.right(), read_only.left());
            assert_eq!(read_only.right(), disabled.left());
            assert_eq!(disabled.right(), group.right() - px(1.));
        }
    }

    let (read_only_focus, disabled_focus) = cx.read(|app| {
        let specimen = component.read(app);
        (
            specimen.grouped_read_only_focus.clone(),
            specimen.grouped_disabled_focus.clone(),
        )
    });
    cx.update(|window, app| {
        read_only_focus.focus(window, app);
        assert!(read_only_focus.is_focused(window));
        window.focus_next(app);
        assert!(!disabled_focus.is_focused(window));
    });
}

fn mount(
    cx: &mut TestAppContext,
) -> crate::test_support::Mounted<'_, InspectorSpecimen, SpecimenAction> {
    mount_component(cx, |_, cx| InspectorSpecimen::new(cx))
}

#[gpui::test]
fn action_recipe_has_pointer_enter_and_space_parity(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);
    assert_pointer_and_keyboard_parity(
        cx,
        "inspector-test-action",
        &actions,
        SpecimenAction::Activated,
    );
}

#[gpui::test]
fn segmented_and_collection_recipes_have_pointer_enter_and_space_parity(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);
    assert_pointer_and_keyboard_parity(
        cx,
        "inspector-test-segment-action",
        &actions,
        SpecimenAction::SegmentSelected,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "inspector-test-collection-action",
        &actions,
        SpecimenAction::CollectionSelected,
    );
}

#[gpui::test]
fn controlled_picker_has_pointer_enter_and_space_commit_parity(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);
    assert_pointer_and_keyboard_parity(
        cx,
        "inspector-test-controlled-picker",
        &actions,
        SpecimenAction::ControlledPicker(InspectorEdit::commit(InspectorValue::Uniform(
            SharedString::from("Chosen"),
        ))),
    );
}

#[gpui::test]
fn disabled_menu_items_are_skipped_by_keyboard_focus(cx: &mut TestAppContext) {
    let (host, _actions, cx) = mount(cx);
    let component = cx.read(|app| host.read(app).component.clone());
    let (action_focus, overlay_trigger_focus) = cx.read(|app| {
        let specimen = component.read(app);
        (
            specimen.action_focus.clone(),
            specimen.overlay_trigger_focus.clone(),
        )
    });
    cx.update(|window, app| {
        action_focus.focus(window, app);
        window.focus_next(app);
        assert!(overlay_trigger_focus.is_focused(window));
    });
}

#[gpui::test]
fn overlay_dismissal_restores_trigger_focus_for_pointer_and_escape(cx: &mut TestAppContext) {
    let (host, _actions, cx) = mount(cx);
    let component = cx.read(|app| host.read(app).component.clone());
    let trigger = cx
        .debug_bounds("inspector-test-overlay-trigger")
        .expect("overlay trigger should render");

    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("inspector-test-overlay").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("inspector-test-overlay").is_none());
    assert_eq!(
        cx.read(|app| component.read(app).last_overlay_dismiss_cause),
        Some(InspectorOverlayDismissCause::Escape)
    );
    assert!(
        cx.update(|window, app| { component.read(app).overlay_trigger_focus.is_focused(window) })
    );

    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    let root = cx
        .debug_bounds("inspector-test-root")
        .expect("specimen root should render");
    cx.simulate_click(
        gpui::point(root.right() - px(4.), root.bottom() - px(4.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("inspector-test-overlay").is_none());
    assert_eq!(
        cx.read(|app| component.read(app).last_overlay_dismiss_cause),
        Some(InspectorOverlayDismissCause::OutsideClick)
    );
    assert!(
        cx.update(|window, app| { component.read(app).overlay_trigger_focus.is_focused(window) })
    );
}

#[gpui::test]
fn validation_and_layout_remain_stable_across_supported_widths_and_themes(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|_, app| Theme::change(mode, None, app));
        for width in [320., 400., 472.] {
            cx.simulate_resize(size(px(width), px(300.)));
            cx.run_until_parked();
            let root = cx
                .debug_bounds("inspector-test-root")
                .expect("the themed inspector specimen should render");
            let validation = cx
                .debug_bounds("inspector-test-validation")
                .expect("validation feedback should render");
            let invalid_number = cx
                .debug_bounds("inspector-test-invalid-number")
                .expect("invalid field should retain its frame");
            let mixed_value = cx
                .debug_bounds("inspector-test-mixed-value")
                .expect("mixed controlled values should retain field geometry");
            assert!(validation.left() >= root.left() + px(16.));
            assert!(validation.right() <= root.right() - px(16.));
            assert_eq!(invalid_number.size.height, px(24.));
            assert_eq!(mixed_value.size.height, px(24.));
        }
    }
}
