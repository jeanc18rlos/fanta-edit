//! The rendering half of [`FantaPropertiesPanel`]: the shared row / pill /
//! swatch / dropdown primitives and the per-section `render_*` methods that
//! turn an [`InspectorSnapshot`] into the panel's element tree. State,
//! mutation handlers, and the `Render` impl live in `properties_panel`.

use fanta_doc::{
    BlendMode, BlurKind, BoundProp, Color as FantaColor, Gradient, ImageFitMode, LayoutMode,
    NodeFlags, NodeId, StrokeAlign, TextAlign, VAlign as TextVAlign, VarValue,
};
use gpui::{
    Anchor, Context, Div, MouseButton, MouseDownEvent, Window, anchored, canvas, deferred, point,
    px, relative,
};
use util::ResultExt as _;

use ui::prelude::*;
use ui::{
    ContextMenu, ContextMenuEntry, DropdownMenu, DropdownStyle, PopoverMenu, Switch, ToggleState,
    Tooltip,
};

use crate::color_picker::gradient_preview_strip;
use crate::component_properties::CreateComponentPropertyKind;
use crate::export::{EXPORT_FORMATS, EXPORT_SCALES};
use crate::inspector_components::{InspectorPropertyRow, InspectorSectionHeader};
use crate::inspector_widgets::{
    AlignGlyph, PanelDrag, TextAlignGlyph, TextDecorationGlyph, align_glyph, text_align_glyph,
    text_decoration_glyph,
};
use crate::mode_overrides::{ModeOverridesControl, mode_overrides_model};
use crate::properties_ops::{fanta_color_rgba, font_weight_label, format_number};
use crate::properties_panel::{FantaPropertiesPanel, SliderTrack};
use crate::properties_snapshot::{
    AXIS_SIZINGS, AlignCommand, AutoLayoutSnapshot, BLEND_MODES, BLUR_KINDS, BindingSnapshot,
    BlurSnapshot, COUNTER_ALIGNS, ComponentBindingSection, CornerRadiusValue, DISTRIBUTE_BUTTONS,
    EffectSnapshot, FONT_WEIGHTS, HORIZONTAL_ALIGN_BUTTONS, IMAGE_FIT_MODES, InspectorField,
    InstanceSection, LAYOUT_CHILD_RESIZES, LAYOUT_MODES, LayoutChildSnapshot, LayoutSnapshot,
    MIXED_VALUE, MasterSection, MultiSection, NodeSection, PAINT_KINDS, PRIMARY_ALIGNS,
    PageBackgroundValue, PageSection, PaintKind, PaintSnapshot, PropValueSnapshot, SHADOW_KINDS,
    STACKING_ORDERS, STROKE_ALIGNS, SelectionColorSnapshot, TEXT_RESIZE_MODES, TypographySnapshot,
    VERTICAL_ALIGN_BUTTONS, align_grid_active_cell, paint_kind_label,
};

fn align_section_visible(selection_len: usize) -> bool {
    selection_len >= 2
}
use crate::variable_binding::{VariableBindingControl, variable_binding_model};

/// Height of a boxed field / pill / control, the panel's vertical rhythm unit
/// (the original's 30px `FIELD_BOX_H` translated to Zed density).
const FIELD_BOX_H: f32 = 28.0;
/// Height of one fill / stroke list row (the original's 36px `LIST_ROW_H`).
const LIST_ROW_H: f32 = 32.0;
/// The fixed label column width to the left of pills and sliders.
const PILL_LABEL_W: f32 = 68.0;
/// Side length of the auto-layout 3×3 alignment grid box.
const ALIGN_GRID_SIZE: f32 = 64.0;
/// Height of the export preview band (the original's `EXPORT_PREVIEW_H`).
const EXPORT_PREVIEW_H: f32 = 84.0;

fn inspector_field_binding(field: &InspectorField) -> Option<(NodeId, BoundProp)> {
    match field {
        InspectorField::Width(id) => Some((*id, BoundProp::ClipWidth)),
        InspectorField::Height(id) => Some((*id, BoundProp::ClipHeight)),
        InspectorField::CornerRadius(id) => Some((*id, BoundProp::CornerRadius)),
        InspectorField::Opacity(id) => Some((*id, BoundProp::Opacity)),
        InspectorField::StrokeWidth { id, index } => Some((
            *id,
            BoundProp::StrokeWidth {
                index: u16::try_from(*index).ok()?,
            },
        )),
        _ => None,
    }
}

impl FantaPropertiesPanel {
    // === Rendering primitives =============================================

    /// A titled section header band: a muted caption at the left and an
    /// optional action (the Fill/Stroke/Effects "+" box) hugging the right
    /// inset, matching the original's header anatomy.
    fn render_section_header(title: &'static str, action: Option<AnyElement>) -> AnyElement {
        match action {
            Some(action) => InspectorSectionHeader::new(title)
                .action(action)
                .without_top_border()
                .into_any_element(),
            None => InspectorSectionHeader::new(title)
                .without_top_border()
                .into_any_element(),
        }
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

    fn render_variable_binding_control(
        &self,
        element_id: impl Into<ElementId>,
        node: NodeId,
        prop: BoundProp,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let item = self.active_item.as_ref()?.upgrade()?;
        let model = item
            .read(cx)
            .doc()
            .and_then(|doc| variable_binding_model(doc, node, prop))?;
        let panel = cx.weak_entity();
        Some(
            VariableBindingControl::new(element_id, model, move |variable, _, cx| {
                panel
                    .update(cx, |panel, cx| {
                        panel.set_variable_binding(node, prop, variable, cx)
                    })
                    .log_err();
            })
            .disabled(!editable)
            .into_any_element(),
        )
    }

    pub(crate) fn render_mode_overrides_section(
        &self,
        node: Option<NodeId>,
        editable: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let item = self.active_item.as_ref()?.upgrade()?;
        let model = item
            .read(cx)
            .doc()
            .and_then(|doc| mode_overrides_model(doc, node))?;
        if model.rows.is_empty() {
            return None;
        }
        let scope = model.scope.clone();
        let scope_name = model.scope_name.clone();
        let panel = cx.weak_entity();
        Some(
            v_flex()
                .child(Self::render_section_header("Variable modes", None))
                .child(
                    v_flex()
                        .px_4()
                        .pb_2()
                        .gap_2()
                        .child(
                            Label::new(format!("Overrides for {scope_name}"))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            ModeOverridesControl::new(
                                "fanta-properties-mode-overrides",
                                model,
                                move |collection, mode, _, cx| {
                                    panel
                                        .update(cx, |panel, cx| {
                                            panel.set_mode_override(
                                                scope.clone(),
                                                collection,
                                                mode,
                                                cx,
                                            )
                                        })
                                        .log_err();
                                },
                            )
                            .disabled(!editable),
                        ),
                )
                .into_any_element(),
        )
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
        let binding = inspector_field_binding(&field).and_then(|(node, prop)| {
            self.render_variable_binding_control(
                SharedString::from(format!("{key}-{ix}-variable-binding")),
                node,
                prop,
                editable,
                cx,
            )
        });
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
        if let Some(binding) = binding {
            cell = cell.child(binding);
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
        InspectorPropertyRow::new(
            label,
            self.render_choice_dropdown(
                element_id, aria_label, id, current, options, apply, editable, window, cx,
            ),
        )
        .label_width(PILL_LABEL_W)
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
        binding: Option<(NodeId, BoundProp)>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let binding = binding.and_then(|(node, prop)| {
            self.render_variable_binding_control(
                SharedString::from(format!("{id}-variable-binding")),
                node,
                prop,
                editable,
                cx,
            )
        });
        h_flex()
            .px_4()
            .h(px(FIELD_BOX_H))
            .items_center()
            .justify_between()
            .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
            .child(
                h_flex().gap_1().children(binding).child(
                    Switch::new(id, ToggleState::from(on))
                        .disabled(!editable)
                        .on_click(
                            cx.listener(move |this, _: &ToggleState, _, cx| on_click(this, cx)),
                        ),
                ),
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

    #[allow(clippy::too_many_arguments)]
    fn render_indexed_choice_dropdown<T: Copy + PartialEq + 'static>(
        &self,
        key: &'static str,
        index: usize,
        aria_label: &'static str,
        id: NodeId,
        current: T,
        options: &'static [(T, &'static str)],
        apply: fn(&mut Self, NodeId, usize, T, &mut Context<Self>),
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
        let panel = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (value, name) in options {
                let panel = panel.clone();
                let value = *value;
                menu.push_item(
                    ContextMenuEntry::new(*name)
                        .toggleable(IconPosition::End, value == current)
                        .handler(move |_, cx| {
                            if let Err(error) = panel
                                .update(cx, |panel, cx| apply(panel, id, index, value, cx))
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
        DropdownMenu::new((key, index), label, menu)
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!editable)
            .aria_label(aria_label)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_variant_dropdown(
        &self,
        index: usize,
        id: NodeId,
        axis: SharedString,
        current: SharedString,
        options: Vec<SharedString>,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.weak_entity();
        let current_for_menu = current.clone();
        let axis_for_menu = axis.clone();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for option in &options {
                let panel = panel.clone();
                let axis = axis_for_menu.clone();
                let value = option.clone();
                menu.push_item(
                    ContextMenuEntry::new(option.clone())
                        .toggleable(IconPosition::End, *option == current_for_menu)
                        .handler(move |_, cx| {
                            if let Err(error) = panel.update(cx, |panel, cx| {
                                panel.select_variant_axis(id, axis.clone(), value.clone(), cx)
                            }) {
                                log::debug!(
                                    "dropping variant selection for closed properties panel: {error:#}"
                                );
                            }
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(("fanta-variant-axis", index), current, menu)
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!editable)
            .aria_label(axis)
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
    pub(crate) fn render_header(
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

    /// The Align section: one three-button group per visual axis, matching
    /// Figma's horizontal/vertical grouping. Distribution stays separate and
    /// appears when three or more nodes are selected.
    pub(crate) fn render_align_section(
        &self,
        selection_len: usize,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !align_section_visible(selection_len) {
            return None;
        }
        let align_enabled = editable && selection_len >= 2;
        let distribute_enabled = editable && selection_len >= 3;
        let colors = cx.theme().colors().clone();
        let horizontal = self.render_align_button_group(
            "fanta-align-horizontal",
            &HORIZONTAL_ALIGN_BUTTONS,
            align_enabled,
            cx,
        );
        let vertical = self.render_align_button_group(
            "fanta-align-vertical",
            &VERTICAL_ALIGN_BUTTONS,
            align_enabled,
            cx,
        );
        let mut section = v_flex()
            .py_1()
            .child(Self::render_section_header("Align", None))
            .child(
                h_flex()
                    .px_4()
                    .py_1()
                    .gap_2()
                    .child(horizontal)
                    .child(vertical),
            );
        if selection_len >= 3 {
            let mut distribute = h_flex()
                .flex_1()
                .h(px(30.))
                .rounded_md()
                .border_1()
                .border_color(colors.border_variant)
                .bg(colors.editor_background)
                .justify_around();
            for (index, (glyph, tooltip, command)) in DISTRIBUTE_BUTTONS.iter().enumerate() {
                distribute = distribute.child(self.render_align_button(
                    ("fanta-distribute", index),
                    *glyph,
                    tooltip,
                    *command,
                    distribute_enabled,
                    cx,
                ));
            }
            section = section.child(h_flex().px_4().pb_1().child(distribute));
        }
        Some(section.into_any_element())
    }

    fn render_align_button_group(
        &self,
        id_prefix: &'static str,
        buttons: &[(AlignGlyph, &'static str, AlignCommand)],
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let mut group = h_flex()
            .flex_1()
            .h(px(30.))
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.editor_background)
            .justify_around();
        for (index, (glyph, tooltip, command)) in buttons.iter().enumerate() {
            group = group.child(self.render_align_button(
                (id_prefix, index),
                *glyph,
                tooltip,
                *command,
                enabled,
                cx,
            ));
        }
        group.into_any_element()
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
    pub(crate) fn render_position_section(
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
    pub(crate) fn render_layout_child_section(
        &self,
        id: NodeId,
        layout_child: &LayoutChildSnapshot,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Auto layout child", None));
        if !layout_child.absolute {
            section = section.child(self.render_choice_row(
                "fanta-child-resize",
                "Resize",
                "Auto-layout child resize",
                id,
                layout_child.fills_container,
                &LAYOUT_CHILD_RESIZES,
                Self::set_layout_child_fill,
                editable,
                window,
                cx,
            ));
        }
        section
            .child(self.render_switch_row(
                "fanta-child-absolute",
                "Ignore auto layout",
                layout_child.absolute,
                editable,
                None,
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
    pub(crate) fn render_layout_section(
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
                None,
                move |this, cx| this.toggle_clip_content(id, cx),
                cx,
            ))
            .child(self.render_switch_row(
                "fanta-layout-auto",
                "Auto layout",
                auto_layout_on,
                editable,
                None,
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

    /// The Auto layout controls: direction menu, the interactive 3×3 alignment
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
            .child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .child(self.render_numeric_cell(
                        "fanta-layout-width-limit",
                        0,
                        Some("Min W".into()),
                        InspectorField::LayoutMinWidth(id),
                        layout.min_width,
                        None,
                        editable,
                        cx,
                    ))
                    .child(self.render_numeric_cell(
                        "fanta-layout-width-limit",
                        1,
                        Some("Max W".into()),
                        InspectorField::LayoutMaxWidth(id),
                        layout.max_width,
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
                        "fanta-layout-height-limit",
                        0,
                        Some("Min H".into()),
                        InspectorField::LayoutMinHeight(id),
                        layout.min_height,
                        None,
                        editable,
                        cx,
                    ))
                    .child(self.render_numeric_cell(
                        "fanta-layout-height-limit",
                        1,
                        Some("Max H".into()),
                        InspectorField::LayoutMaxHeight(id),
                        layout.max_height,
                        None,
                        editable,
                        cx,
                    )),
            )
            .child(self.render_switch_row(
                "fanta-layout-wrap",
                "Wrap",
                layout.wrap,
                editable,
                None,
                move |this, cx| this.toggle_layout_wrap(id, cx),
                cx,
            ))
            .child(self.render_choice_row(
                "fanta-layout-stacking",
                "Stacking",
                "Auto-layout stacking order",
                id,
                layout.reverse_z,
                &STACKING_ORDERS,
                Self::set_layout_stacking,
                editable,
                window,
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
    /// (4 horizontal incl. justify + 3 vertical), and the resize-mode menu.
    pub(crate) fn render_typography_section(
        &self,
        id: NodeId,
        typography: &TypographySnapshot,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text_style_binding = self.render_variable_binding_control(
            "fanta-text-style-variable-binding",
            id,
            BoundProp::TextStyle,
            editable,
            cx,
        );
        v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header(
                "Typography",
                text_style_binding,
            ))
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
            .child(self.render_choice_row(
                "fanta-text-resize",
                "Resize",
                "Text resize mode",
                id,
                typography.auto_resize,
                &TEXT_RESIZE_MODES,
                Self::set_text_resize,
                editable,
                window,
                cx,
            ))
            .when(editable, |section| {
                section.child(
                    h_flex().px_4().justify_end().child(
                        Button::new("fanta-text-to-outlines", "Convert to outlines")
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .start_icon(Icon::new(IconName::ToolPathSelect).size(IconSize::XSmall))
                            .tooltip(Tooltip::text(
                                "Replace editable text with stretchable filled vector paths",
                            ))
                            .on_click(cx.listener(move |panel, _, _, cx| {
                                panel.convert_text_to_outlines(id, cx)
                            })),
                    ),
                )
            })
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

    pub(crate) fn render_image_section(
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

    fn render_component_property_add_menu(
        &self,
        master: &MasterSection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.weak_entity();
        let component = master.id;
        let can_create_variant = master.can_create_variant;
        PopoverMenu::new("fanta-create-component-property-menu")
            .anchor(Anchor::TopRight)
            .trigger(
                IconButton::new("fanta-create-component-property", IconName::Plus)
                    .icon_size(IconSize::XSmall)
                    .tooltip(Tooltip::text("Create property")),
            )
            .menu(move |window, cx| {
                let panel = panel.clone();
                Some(ContextMenu::build(window, cx, move |mut menu, _, _| {
                    for kind in CreateComponentPropertyKind::ALL {
                        let panel = panel.clone();
                        menu.push_item(
                            ContextMenuEntry::new(kind.label())
                                .disabled(
                                    kind == CreateComponentPropertyKind::Variant
                                        && !can_create_variant,
                                )
                                .handler(move |_, cx| {
                                    panel
                                        .update(cx, |panel, cx| {
                                            panel.create_component_property(component, kind, cx)
                                        })
                                        .log_err();
                                }),
                        );
                    }
                    menu
                }))
            })
            .into_any_element()
    }

    /// The component-master section: identity, the variant-set summary, and the
    /// exposed-properties schema.
    pub(crate) fn render_master_section(
        &self,
        master: &MasterSection,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let create_property = editable.then(|| self.render_component_property_add_menu(master, cx));
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Component", create_property))
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
            let trailing: SharedString = if prop.bindings == 0 {
                prop.default.clone()
            } else {
                format!(
                    "{} · {} {}",
                    prop.default,
                    prop.bindings,
                    if prop.bindings == 1 {
                        "binding"
                    } else {
                        "bindings"
                    }
                )
                .into()
            };
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
                        Label::new(trailing)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .single_line(),
                    ),
            );
        }
        section.into_any_element()
    }

    pub(crate) fn render_component_binding_section(
        &self,
        id: NodeId,
        binding: &ComponentBindingSection,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Component properties", None));
        for (index, field) in binding.fields.iter().enumerate() {
            let current_id = field.current.as_ref().map(|property| property.id);
            let current_label = field
                .current
                .as_ref()
                .map(|property| property.name.clone())
                .unwrap_or_else(|| "Not bound".into());
            let panel = cx.weak_entity();
            let choices = field.choices.clone();
            let component = binding.component;
            let target_prop = field.prop;
            let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
                let panel_for_none = panel.clone();
                menu.push_item(
                    ContextMenuEntry::new("Not bound")
                        .toggleable(IconPosition::End, current_id.is_none())
                        .handler(move |_, cx| {
                            panel_for_none
                                .update(cx, |panel, cx| {
                                    panel.set_component_property_binding(
                                        component,
                                        id,
                                        target_prop,
                                        None,
                                        cx,
                                    )
                                })
                                .log_err();
                        }),
                );
                for choice in &choices {
                    let panel = panel.clone();
                    let property = choice.id;
                    menu.push_item(
                        ContextMenuEntry::new(choice.name.clone())
                            .toggleable(IconPosition::End, current_id == Some(property))
                            .handler(move |_, cx| {
                                panel
                                    .update(cx, |panel, cx| {
                                        panel.set_component_property_binding(
                                            component,
                                            id,
                                            target_prop,
                                            Some(property),
                                            cx,
                                        )
                                    })
                                    .log_err();
                            }),
                    );
                }
                menu
            });
            section = section.child(
                InspectorPropertyRow::new(
                    field.label.clone(),
                    DropdownMenu::new(("fanta-component-binding", index), current_label, menu)
                        .style(DropdownStyle::Outlined)
                        .trigger_size(ButtonSize::Compact)
                        .full_width(true)
                        .disabled(!editable)
                        .aria_label(field.label.clone()),
                )
                .label_width(PILL_LABEL_W),
            );
        }
        section.into_any_element()
    }

    /// The component instance Properties section: an identity row that doubles
    /// as "go to main component", one choice menu per variant axis, editors for
    /// bool / text / number / color props, and the detach action.
    pub(crate) fn render_instance_section(
        &self,
        id: NodeId,
        component: &InstanceSection,
        editable: bool,
        window: &mut Window,
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
            section = section.child(
                InspectorPropertyRow::new(
                    variant.axis.clone(),
                    self.render_variant_dropdown(
                        index,
                        id,
                        variant.axis.clone(),
                        variant.value.clone(),
                        variant.options.clone(),
                        editable,
                        window,
                        cx,
                    ),
                )
                .label_width(PILL_LABEL_W),
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
        // One Content row per painted text clone — editing writes a text
        // override on the instance, Figma's panel-side instance text editing.
        for (index, text) in component.texts.iter().enumerate() {
            section = section.child(
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(
                        div().w(px(PILL_LABEL_W)).flex_none().child(
                            Label::new(text.label.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .single_line(),
                        ),
                    )
                    .child(self.render_text_cell(
                        "fanta-instance-text",
                        index,
                        None,
                        InspectorField::InstanceText {
                            id,
                            path: text.path.clone(),
                        },
                        text.content.clone(),
                        editable.then(|| text.content.to_string()),
                        cx,
                    )),
            );
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
    pub(crate) fn render_appearance_section(
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
                Some((id, BoundProp::Visible)),
                move |this, cx| this.toggle_flag(id, NodeFlags::HIDDEN, cx),
                cx,
            ))
            .child(self.render_switch_row(
                "fanta-toggle-locked",
                "Locked",
                node.locked,
                editable,
                None,
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
    pub(crate) fn render_paint_section(
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
            if let Ok(index_u16) = u16::try_from(index) {
                let prop = if is_stroke {
                    BoundProp::StrokeColor { index: index_u16 }
                } else {
                    BoundProp::FillColor { index: index_u16 }
                };
                let binding_id = if is_stroke {
                    ("fanta-stroke-color-variable-binding", index)
                } else {
                    ("fanta-fill-color-variable-binding", index)
                };
                if let Some(binding) =
                    self.render_variable_binding_control(binding_id, id, prop, editable, cx)
                {
                    row = row.child(binding);
                }
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
    pub(crate) fn render_text_fill_section(
        &self,
        id: NodeId,
        color: FantaColor,
        color_mixed: bool,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = InspectorField::TextColor(id);
        // A mixed sub-selection shows the mixed marker instead of one run's
        // color; typing a hex still applies to the whole selection.
        let hex_label: SharedString = if color_mixed {
            MIXED_VALUE.to_string().into()
        } else {
            color.to_hex().trim_start_matches('#').to_string().into()
        };
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
                        (!color_mixed).then_some(color),
                        Some(field.clone()),
                        editable,
                        cx,
                    ))
                    .child(div().flex_1().min_w_0().child(self.render_text_cell(
                        "fanta-text-fill",
                        0,
                        None,
                        field,
                        hex_label,
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
    /// Each blur is a kind menu + radius cell + remove ×.
    pub(crate) fn render_effects_section(
        &self,
        id: NodeId,
        effects: &[EffectSnapshot],
        blurs: &[BlurSnapshot],
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let add_button = editable.then(|| self.render_effects_add_menu(id, cx));
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Effects", add_button));
        for (index, effect) in effects.iter().enumerate() {
            let mut kind_row =
                h_flex()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_indexed_choice_dropdown(
                                "fanta-effect-kind",
                                index,
                                "Shadow kind",
                                id,
                                effect.kind,
                                &SHADOW_KINDS,
                                Self::set_effect_kind,
                                editable,
                                window,
                                cx,
                            )),
                    );
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
            let mut row = h_flex()
                .px_4()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(self.render_indexed_choice_dropdown(
                            "fanta-blur-kind",
                            index,
                            "Blur kind",
                            id,
                            blur.kind,
                            &BLUR_KINDS,
                            Self::set_blur_kind,
                            editable,
                            window,
                            cx,
                        )),
                )
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

    pub(crate) fn render_interactions_section(&self, reactions: &[SharedString]) -> AnyElement {
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

    pub(crate) fn render_bindings_section(&self, bindings: &[BindingSnapshot]) -> AnyElement {
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

    pub(crate) fn render_export_section(
        &self,
        can_export: bool,
        preview_icon: Option<IconName>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let controls_enabled = self.export_task.is_none();
        let add = IconButton::new("fanta-export-add-preset", IconName::Plus)
            .icon_size(IconSize::XSmall)
            .aria_label("Add export setting")
            .tooltip(Tooltip::text("Add export setting"))
            .disabled(!controls_enabled)
            .on_click(cx.listener(|this, _, _, cx| this.add_export_preset(cx)));
        let mut section = v_flex().py_1().gap_2().child(Self::render_section_header(
            "Export",
            Some(add.into_any_element()),
        ));

        for (index, preset) in self.export_presets.iter().copied().enumerate() {
            let panel = cx.weak_entity();
            let format_menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
                for (format, label) in EXPORT_FORMATS {
                    let panel = panel.clone();
                    let format = *format;
                    menu.push_item(
                        ContextMenuEntry::new(*label)
                            .toggleable(IconPosition::End, preset.format == format)
                            .handler(move |_, cx| {
                                if let Err(error) = panel.update(cx, |panel, cx| {
                                    panel.set_export_format(index, format, cx)
                                }) {
                                    log::debug!(
                                        "dropping export format change for closed inspector: {error:#}"
                                    );
                                }
                            }),
                    );
                }
                menu
            });
            let format = DropdownMenu::new(
                ("fanta-export-format", index),
                preset.format.label(),
                format_menu,
            )
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .disabled(!controls_enabled)
            .aria_label("Export format");

            let scale = if preset.format.is_raster() {
                let panel = cx.weak_entity();
                let scale_menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
                    for (scale, label) in EXPORT_SCALES {
                        let panel = panel.clone();
                        let scale = *scale;
                        menu.push_item(
                            ContextMenuEntry::new(*label)
                                .toggleable(IconPosition::End, preset.scale == scale)
                                .handler(move |_, cx| {
                                    if let Err(error) = panel.update(cx, |panel, cx| {
                                        panel.set_export_scale(index, scale, cx)
                                    }) {
                                        log::debug!(
                                            "dropping export scale change for closed inspector: {error:#}"
                                        );
                                    }
                                }),
                        );
                    }
                    menu
                });
                DropdownMenu::new(
                    ("fanta-export-scale", index),
                    preset.scale.label(),
                    scale_menu,
                )
                .style(DropdownStyle::Outlined)
                .trigger_size(ButtonSize::Compact)
                .full_width(true)
                .disabled(!controls_enabled)
                .aria_label("Export scale")
                .into_any_element()
            } else {
                h_flex()
                    .h(px(FIELD_BOX_H))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border_variant)
                    .bg(colors.editor_background)
                    .child(
                        Label::new("Vector")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .into_any_element()
            };
            section = section.child(
                h_flex()
                    .px_4()
                    .gap_1()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(format))
                    .child(div().w(px(68.)).flex_none().child(scale))
                    .child(
                        IconButton::new(("fanta-export-remove", index), IconName::Close)
                            .icon_size(IconSize::XSmall)
                            .aria_label("Remove export setting")
                            .tooltip(Tooltip::text("Remove export setting"))
                            .disabled(!controls_enabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_export_preset(index, cx);
                            })),
                    ),
            );
        }

        let preset_count = self.export_presets.len();
        let preset_summary = match preset_count {
            0 => "Add an export setting".to_string(),
            1 => "1 format per selected layer".to_string(),
            count => format!("{count} formats per selected layer"),
        };
        let preview_summary = if self.export_presets.is_empty() {
            "No export settings".to_string()
        } else {
            self.export_presets
                .iter()
                .map(|preset| {
                    if preset.format.is_raster() {
                        format!("{} {}", preset.format.label(), preset.scale.label())
                    } else {
                        preset.format.label().to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" · ")
        };
        section
            .child(
                h_flex()
                    .px_4()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        Label::new(preset_summary)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Button::new("fanta-export-selection", "Export")
                            .size(ButtonSize::Compact)
                            .label_size(LabelSize::Small)
                            .disabled(!controls_enabled || !can_export || preset_count == 0)
                            .tooltip(Tooltip::text(if !controls_enabled {
                                "Export in progress"
                            } else if can_export {
                                "Export to <project>/exports"
                            } else {
                                "Create a Fanta project to export"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.export_selection(cx))),
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
                                    v_flex()
                                        .max_w(px(180.))
                                        .min_h(px(46.))
                                        .px_3()
                                        .py_2()
                                        .gap_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(colors.border_variant)
                                        .bg(colors.surface_background)
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::new(
                                                if self
                                                    .export_presets
                                                    .iter()
                                                    .any(|preset| !preset.format.is_raster())
                                                {
                                                    IconName::FileCode
                                                } else {
                                                    preview_icon.unwrap_or(IconName::Image)
                                                },
                                            )
                                            .size(IconSize::Small)
                                            .color(Color::Muted),
                                        )
                                        .child(
                                            Label::new(preview_summary)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted)
                                                .line_clamp(1),
                                        ),
                                ),
                        ),
                ),
            )
            .into_any_element()
    }

    pub(crate) fn render_page_properties(
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

    pub(crate) fn render_page_comments_section(
        &self,
        page: &PageSection,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let page_id = page.id?;
        if page.comments.is_empty() {
            return None;
        }

        let colors = cx.theme().colors().clone();
        let mut section = v_flex()
            .py_1()
            .gap_2()
            .child(Self::render_section_header("Comments", None));
        for (row, comment) in page.comments.iter().enumerate() {
            let edit_id = comment.id.clone();
            let open_id = comment.id.clone();
            let resolve_id = comment.id.clone();
            let delete_id = comment.id.clone();
            let mut details = Vec::new();
            if comment.replies > 0 {
                details.push(format!(
                    "{} repl{}",
                    comment.replies,
                    if comment.replies == 1 { "y" } else { "ies" }
                ));
            }
            if comment.attachments > 0 {
                details.push(format!(
                    "{} attachment{}",
                    comment.attachments,
                    if comment.attachments == 1 { "" } else { "s" }
                ));
            }
            if comment.mentions > 0 {
                details.push(format!(
                    "{} mention{}",
                    comment.mentions,
                    if comment.mentions == 1 { "" } else { "s" }
                ));
            }
            if let Some(skill) = &comment.skill {
                details.push(format!("AI: {skill}"));
            }
            section = section.child(
                v_flex()
                    .mx_3()
                    .p_2()
                    .gap_1p5()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border_variant)
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap_1p5()
                                    .child(
                                        Label::new(format!("{}", comment.number))
                                            .size(LabelSize::XSmall)
                                            .color(if comment.resolved {
                                                Color::Muted
                                            } else {
                                                Color::Accent
                                            }),
                                    )
                                    .child(
                                        Label::new(comment.author.clone())
                                            .size(LabelSize::XSmall)
                                            .single_line()
                                            .truncate(),
                                    ),
                            )
                            .child(
                                Label::new(if comment.resolved { "Resolved" } else { "Open" })
                                    .size(LabelSize::XSmall)
                                    .color(if comment.resolved {
                                        Color::Muted
                                    } else {
                                        Color::Accent
                                    }),
                            ),
                    )
                    .child(self.render_text_cell(
                        "fanta-comment-text",
                        row,
                        None,
                        InspectorField::CommentText {
                            page: page_id,
                            id: edit_id,
                        },
                        comment.text.clone(),
                        editable.then(|| comment.text.to_string()),
                        cx,
                    ))
                    .when(!details.is_empty(), |this| {
                        this.child(
                            Label::new(details.join(" · "))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new(("fanta-comment-open", row), "Open thread")
                                    .size(ButtonSize::Compact)
                                    .label_size(LabelSize::XSmall)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_comment_thread(open_id.clone(), window, cx);
                                    })),
                            )
                            .child(
                                IconButton::new(
                                    ("fanta-comment-resolve", row),
                                    if comment.resolved {
                                        IconName::Undo
                                    } else {
                                        IconName::Check
                                    },
                                )
                                .icon_size(IconSize::XSmall)
                                .tooltip(Tooltip::text(if comment.resolved {
                                    "Reopen"
                                } else {
                                    "Resolve"
                                }))
                                .disabled(!editable)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.toggle_comment_resolved(
                                            page_id,
                                            resolve_id.clone(),
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(("fanta-comment-delete", row), IconName::Trash)
                                    .icon_size(IconSize::XSmall)
                                    .tooltip(Tooltip::text("Delete thread"))
                                    .disabled(!editable)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.delete_comment(page_id, delete_id.clone(), cx);
                                    })),
                            ),
                    ),
            );
        }
        Some(section.into_any_element())
    }

    pub(crate) fn render_multi_position_section(
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
    pub(crate) fn render_selection_colors_section(
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
    pub(crate) fn render_combine_variants_section(
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

#[cfg(test)]
mod section_visibility_tests {
    use super::align_section_visible;

    #[test]
    fn align_is_not_a_phantom_page_or_single_selection_section() {
        assert!(!align_section_visible(0));
        assert!(!align_section_visible(1));
        assert!(align_section_visible(2));
        assert!(align_section_visible(3));
    }
}
