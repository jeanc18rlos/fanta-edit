use std::rc::Rc;

use fanta_doc::{BoundProp, CanvasNode, Doc, NodeData, NodeId, Operation, VariableId};
use gpui::{Anchor, App, ElementId, IntoElement, RenderOnce, SharedString, Window, div};
use ui::{ContextMenu, ContextMenuEntry, IconPosition, PopoverMenu, Tooltip, prelude::*};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VariableBindingOption {
    pub(crate) id: VariableId,
    pub(crate) label: SharedString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VariableBindingModel {
    pub(crate) current: Option<VariableId>,
    pub(crate) current_label: Option<SharedString>,
    pub(crate) options: Vec<VariableBindingOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindableProperty {
    pub(crate) prop: BoundProp,
    pub(crate) label: SharedString,
}

type BindingHandler = Rc<dyn Fn(Option<VariableId>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct VariableBindingControl {
    id: ElementId,
    model: VariableBindingModel,
    disabled: bool,
    on_change: BindingHandler,
}

impl VariableBindingControl {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        model: VariableBindingModel,
        on_change: impl Fn(Option<VariableId>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            model,
            disabled: false,
            on_change: Rc::new(on_change),
        }
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for VariableBindingControl {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let is_bound = self.model.current.is_some();
        let tooltip = self
            .model
            .current_label
            .clone()
            .map(|label| format!("Bound to {label}"))
            .unwrap_or_else(|| "Bind variable".to_owned());
        let model = self.model;
        let on_change = self.on_change;
        let trigger = IconButton::new(self.id.clone(), IconName::Link)
            .icon_size(IconSize::XSmall)
            .toggle_state(is_bound)
            .icon_color(if is_bound {
                Color::Accent
            } else {
                Color::Muted
            })
            .disabled(self.disabled)
            .tooltip(Tooltip::text(tooltip));

        div()
            .flex_none()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                PopoverMenu::new((self.id, "popover"))
                    .anchor(Anchor::TopRight)
                    .trigger(trigger)
                    .menu(move |window, cx| {
                        let on_change = on_change.clone();
                        let model = model.clone();
                        Some(ContextMenu::build(window, cx, move |mut menu, _, _| {
                            if model.current.is_some() {
                                let on_change = on_change.clone();
                                menu.push_item(
                                    ContextMenuEntry::new("Unbind variable")
                                        .icon(IconName::Link)
                                        .handler(move |window, cx| on_change(None, window, cx)),
                                );
                                if !model.options.is_empty() {
                                    menu = menu.separator();
                                }
                            }

                            if model.options.is_empty() {
                                menu.push_item(
                                    ContextMenuEntry::new("No compatible variables").disabled(true),
                                );
                            } else {
                                for option in &model.options {
                                    let on_change = on_change.clone();
                                    let variable = option.id;
                                    menu.push_item(
                                        ContextMenuEntry::new(option.label.clone())
                                            .toggleable(
                                                IconPosition::End,
                                                model.current == Some(variable),
                                            )
                                            .handler(move |window, cx| {
                                                on_change(Some(variable), window, cx)
                                            }),
                                    );
                                }
                            }
                            menu
                        }))
                    }),
            )
    }
}

pub(crate) fn variable_binding_model(
    doc: &Doc,
    node_id: NodeId,
    prop: BoundProp,
) -> Option<VariableBindingModel> {
    let node = doc.scene.get(node_id)?;
    if !prop.applies_to(node) {
        return None;
    }

    let mut options: Vec<_> = doc
        .variables
        .variables
        .values()
        .filter(|variable| {
            prop.accepts_variable_type(variable.ty)
                && doc.variables.collections.contains_key(&variable.collection)
        })
        .filter_map(|variable| {
            Some(VariableBindingOption {
                id: variable.id,
                label: variable_label(doc, variable.id)?,
            })
        })
        .collect();
    options.sort_by(|left, right| left.label.cmp(&right.label));

    let current = node.bindings.get(&prop).copied();
    let current_label = current
        .map(|variable| variable_label(doc, variable).unwrap_or_else(|| "Missing variable".into()));
    Some(VariableBindingModel {
        current,
        current_label,
        options,
    })
}

pub(crate) fn bindable_properties(node: &CanvasNode) -> Vec<BindableProperty> {
    let mut properties = vec![
        bindable(BoundProp::Visible, "Visibility"),
        bindable(BoundProp::Opacity, "Opacity"),
    ];

    let fill_count = match &node.data {
        NodeData::Vector(vector) => vector.fills.len(),
        NodeData::Group(group) => {
            usize::from(group.background.is_some()) + group.background_fills.len()
        }
        NodeData::Text(_) => 1,
        _ => 0,
    };
    for index in 0..fill_count.min(usize::from(u16::MAX) + 1) {
        properties.push(bindable(
            BoundProp::FillColor {
                index: index as u16,
            },
            format!("Fill {} color", index + 1),
        ));
    }

    let stroke_count = node.data.strokes().map_or(0, |strokes| strokes.len());
    for index in 0..stroke_count.min(usize::from(u16::MAX) + 1) {
        properties.push(bindable(
            BoundProp::StrokeColor {
                index: index as u16,
            },
            format!("Stroke {} color", index + 1),
        ));
        properties.push(bindable(
            BoundProp::StrokeWidth {
                index: index as u16,
            },
            format!("Stroke {} width", index + 1),
        ));
    }

    for (prop, label) in [
        (BoundProp::CornerRadius, "Corner radius"),
        (BoundProp::TextContent, "Text content"),
        (BoundProp::TextStyle, "Typography"),
        (BoundProp::ClipWidth, "Width"),
        (BoundProp::ClipHeight, "Height"),
    ] {
        if prop.applies_to(node) {
            properties.push(bindable(prop, label));
        }
    }
    properties
}

pub(crate) fn variable_binding_operation(
    doc: &Doc,
    node_id: NodeId,
    prop: BoundProp,
    variable: Option<VariableId>,
) -> Result<Option<Operation>, &'static str> {
    match variable {
        Some(variable) => bind_property_operation(doc, node_id, prop, variable),
        None => unbind_property_operation(doc, node_id, prop),
    }
}

fn bindable(prop: BoundProp, label: impl Into<SharedString>) -> BindableProperty {
    BindableProperty {
        prop,
        label: label.into(),
    }
}

fn variable_label(doc: &Doc, variable_id: VariableId) -> Option<SharedString> {
    let variable = doc.variables.variables.get(&variable_id)?;
    let collection = doc.variables.collections.get(&variable.collection)?;
    Some(format!("{} / {}", collection.name, variable.name).into())
}

fn bind_property_operation(
    doc: &Doc,
    node_id: NodeId,
    prop: BoundProp,
    variable_id: VariableId,
) -> Result<Option<Operation>, &'static str> {
    let node = doc
        .scene
        .get(node_id)
        .ok_or("The selected layer no longer exists")?;
    if !prop.applies_to(node) {
        return Err("The property does not apply to the selected layer");
    }
    let variable = doc
        .variables
        .variables
        .get(&variable_id)
        .ok_or("The variable no longer exists")?;
    if !doc.variables.collections.contains_key(&variable.collection) {
        return Err("The variable's collection no longer exists");
    }
    if !prop.accepts_variable_type(variable.ty) {
        return Err("The variable type is not compatible with this property");
    }
    let old = node.bindings.get(&prop).copied();
    if old == Some(variable_id) {
        return Ok(None);
    }
    Ok(Some(Operation::BindProperty {
        node: node_id,
        prop,
        old,
        new: variable_id,
    }))
}

fn unbind_property_operation(
    doc: &Doc,
    node_id: NodeId,
    prop: BoundProp,
) -> Result<Option<Operation>, &'static str> {
    let node = doc
        .scene
        .get(node_id)
        .ok_or("The selected layer no longer exists")?;
    let Some(variable) = node.bindings.get(&prop).copied() else {
        return Ok(None);
    };
    let old_data = node.data.clone();
    let old_opacity = node.opacity;
    let old_flags = node.flags;
    let mut baked = node.clone();
    if let Some(value) = fanta_doc::resolve_bound_value(
        &doc.variables,
        &doc.scene,
        node_id,
        &doc.active_modes,
        variable,
    ) {
        prop.apply_resolved(&mut baked, value);
    }
    Ok(Some(Operation::UnbindProperty {
        node: node_id,
        prop,
        variable,
        old_data: Box::new(old_data),
        new_data: Box::new(baked.data),
        old_opacity,
        new_opacity: baked.opacity,
        old_flags,
        new_flags: baked.flags,
    }))
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

    use fanta_doc::{
        Color, Fill, GroupNode, Mode, ModeId, Stroke, TextNode, Variable, VariableCollection,
        VariableCollectionId, VariableType, VectorNode,
    };
    use gpui::{Context, Render, TestAppContext};

    use super::*;

    fn collection_with_variables(doc: &mut Doc) -> (VariableId, VariableId, VariableId) {
        let collection_id = VariableCollectionId::new();
        let mode_id = ModeId::new();
        let color_id = VariableId::new();
        let float_id = VariableId::new();
        let typography_id = VariableId::new();
        doc.variables.collections.insert(
            collection_id,
            VariableCollection {
                id: collection_id,
                name: "Tokens".into(),
                modes: vec![Mode {
                    id: mode_id,
                    name: "Default".into(),
                }],
                default_mode: mode_id,
                variable_order: vec![color_id, float_id, typography_id],
            },
        );
        for (id, name, ty) in [
            (color_id, "Accent", VariableType::Color),
            (float_id, "Spacing", VariableType::Float),
            (typography_id, "Body", VariableType::Typography),
        ] {
            doc.variables.variables.insert(
                id,
                Variable {
                    id,
                    collection: collection_id,
                    name: name.into(),
                    ty,
                    values_by_mode: BTreeMap::new(),
                    scopes: Vec::new(),
                },
            );
        }
        (color_id, float_id, typography_id)
    }

    #[test]
    fn candidates_cover_every_paint_slot_and_specialized_properties() {
        let mut vector = VectorNode::default();
        vector.fills.push(Fill::solid(Color::BLACK));
        vector.fills.push(Fill::solid(Color::WHITE));
        vector.strokes.push(Stroke::solid(Color::BLACK, 1.0));
        vector.strokes.push(Stroke::solid(Color::WHITE, 2.0));
        let vector = CanvasNode::new(NodeData::Vector(vector));
        let properties = bindable_properties(&vector);
        for prop in [
            BoundProp::FillColor { index: 0 },
            BoundProp::FillColor { index: 1 },
            BoundProp::StrokeColor { index: 0 },
            BoundProp::StrokeColor { index: 1 },
            BoundProp::StrokeWidth { index: 0 },
            BoundProp::StrokeWidth { index: 1 },
        ] {
            assert!(properties.iter().any(|candidate| candidate.prop == prop));
        }

        let frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 60.0]),
            background: Some(Fill::solid(Color::BLACK)),
            background_fills: smallvec::smallvec![Fill::solid(Color::WHITE)],
            ..GroupNode::default()
        }));
        let frame_properties = bindable_properties(&frame);
        for prop in [
            BoundProp::FillColor { index: 0 },
            BoundProp::FillColor { index: 1 },
            BoundProp::ClipWidth,
            BoundProp::ClipHeight,
        ] {
            assert!(
                frame_properties
                    .iter()
                    .any(|candidate| candidate.prop == prop)
            );
        }

        let text = CanvasNode::new(NodeData::Text(TextNode::new("Hello", 100.0, 20.0)));
        let text_properties = bindable_properties(&text);
        assert!(
            text_properties
                .iter()
                .any(|candidate| candidate.prop == BoundProp::TextStyle)
        );
    }

    #[test]
    fn model_filters_variables_by_property_type_and_builds_collection_labels() {
        let mut doc = Doc::new();
        let (color, _float, _typography) = collection_with_variables(&mut doc);
        let vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::BLACK,
        )));
        let node_id = vector.id;
        doc.scene.insert(vector).expect("insert vector");

        let model = variable_binding_model(&doc, node_id, BoundProp::FillColor { index: 0 })
            .expect("fill is bindable");
        assert_eq!(model.options.len(), 1);
        assert_eq!(model.options[0].id, color);
        assert_eq!(model.options[0].label.as_ref(), "Tokens / Accent");
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    struct BindingHarness {
        model: VariableBindingModel,
        changes: Rc<RefCell<Vec<Option<VariableId>>>>,
    }

    impl Render for BindingHarness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let changes = self.changes.clone();
            VariableBindingControl::new(
                "test-variable-binding",
                self.model.clone(),
                move |selected, _, _| changes.borrow_mut().push(selected),
            )
        }
    }

    #[gpui::test]
    fn clicking_a_choice_routes_through_the_callback(cx: &mut TestAppContext) {
        init_test(cx);
        let variable = VariableId::new();
        let changes = Rc::new(RefCell::new(Vec::new()));
        let changes_for_view = changes.clone();
        let window = cx.add_window(move |_, _| BindingHarness {
            model: VariableBindingModel {
                current: None,
                current_label: None,
                options: vec![VariableBindingOption {
                    id: variable,
                    label: "Tokens / Accent".into(),
                }],
            },
            changes: changes_for_view,
        });
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.update(|window, cx| window.draw(cx).clear());
        vcx.run_until_parked();

        let trigger = vcx.debug_bounds("ICON-Link").expect("binding trigger");
        vcx.simulate_click(trigger.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let choice = vcx
            .debug_bounds("MENU_ITEM-Tokens / Accent")
            .expect("variable choice");
        vcx.simulate_click(choice.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert_eq!(changes.borrow().as_slice(), &[Some(variable)]);
    }

    #[gpui::test]
    fn clicking_unbind_routes_none_through_the_callback(cx: &mut TestAppContext) {
        init_test(cx);
        let variable = VariableId::new();
        let changes = Rc::new(RefCell::new(Vec::new()));
        let changes_for_view = changes.clone();
        let window = cx.add_window(move |_, _| BindingHarness {
            model: VariableBindingModel {
                current: Some(variable),
                current_label: Some("Tokens / Accent".into()),
                options: vec![VariableBindingOption {
                    id: variable,
                    label: "Tokens / Accent".into(),
                }],
            },
            changes: changes_for_view,
        });
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.update(|window, cx| window.draw(cx).clear());
        vcx.run_until_parked();

        let trigger = vcx.debug_bounds("ICON-Link").expect("binding trigger");
        vcx.simulate_click(trigger.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        let unbind = vcx
            .debug_bounds("MENU_ITEM-Unbind variable")
            .expect("unbind choice");
        vcx.simulate_click(unbind.center(), gpui::Modifiers::default());
        vcx.run_until_parked();
        assert_eq!(changes.borrow().as_slice(), &[None]);
    }
}
