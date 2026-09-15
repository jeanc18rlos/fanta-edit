//! Projection, controller, and top-level renderer for the Effects section.

use super::super::*;

/// Stable identity used to validate deferred Effects interactions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) struct EffectTargetProjection {
    node_id: SharedString,
    effect_id: SharedString,
    index: usize,
}

impl EffectTargetProjection {
    pub(in super::super) fn new(
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
    ) -> Self {
        Self {
            node_id,
            effect_id,
            index,
        }
    }

    fn matches(&self, effect: &DesignEffect, index: usize) -> bool {
        if self.effect_id.is_empty() || effect.id.is_empty() {
            self.index == index
        } else {
            self.effect_id == effect.id
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct EffectsIdentityProjection {
    panel_id: SharedString,
    node_id: SharedString,
}

impl EffectsIdentityProjection {
    pub(in super::super) fn new(panel_id: SharedString, node_id: SharedString) -> Self {
        Self { panel_id, node_id }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct EffectsAccessProjection {
    can_edit: bool,
    collection_supported: bool,
}

impl EffectsAccessProjection {
    pub(in super::super) const fn new(can_edit: bool, collection_supported: bool) -> Self {
        Self {
            can_edit,
            collection_supported,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct EffectsContentProjection {
    style_binding_name: Option<SharedString>,
    effects: Vec<DesignEffect>,
}

impl EffectsContentProjection {
    pub(in super::super) fn new(
        style_binding_name: Option<SharedString>,
        effects: Vec<DesignEffect>,
    ) -> Self {
        Self {
            style_binding_name,
            effects,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct EffectsOverlayProjection {
    active_settings: Option<EffectTargetProjection>,
}

impl EffectsOverlayProjection {
    pub(in super::super) fn new(active_settings: Option<EffectTargetProjection>) -> Self {
        Self { active_settings }
    }
}

/// Immutable read model consumed by the extracted Effects renderer.
#[derive(Clone)]
pub(in super::super) struct EffectsProjection {
    identity: EffectsIdentityProjection,
    access: EffectsAccessProjection,
    content: EffectsContentProjection,
    overlays: EffectsOverlayProjection,
}

impl EffectsProjection {
    pub(in super::super) fn new(
        identity: EffectsIdentityProjection,
        access: EffectsAccessProjection,
        content: EffectsContentProjection,
        overlays: EffectsOverlayProjection,
    ) -> Self {
        Self {
            identity,
            access,
            content,
            overlays,
        }
    }

    fn target(&self, effect: &DesignEffect, index: usize) -> EffectTargetProjection {
        EffectTargetProjection::new(self.identity.node_id.clone(), effect.id.clone(), index)
    }

    fn settings_are_active(&self, effect: &DesignEffect, index: usize) -> bool {
        self.overlays
            .active_settings
            .as_ref()
            .is_some_and(|target| target.matches(effect, index))
    }
}

/// Narrow adapter around chrome and retained, complex effect-settings editors.
pub(in super::super) trait EffectsInspectorChrome {
    fn effects_bound_style_summary(
        &self,
        name: SharedString,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn effects_section(&self, content: AnyElement, cx: &mut Context<DesignPanel>) -> AnyElement;

    fn effects_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn effects_visibility_button(
        &self,
        id_suffix: impl Into<SharedString>,
        visible: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn effects_settings_for_target(
        &self,
        target: &EffectTargetProjection,
        cx: &mut Context<DesignPanel>,
    ) -> Option<AnyElement>;
}

impl EffectsInspectorChrome for DesignPanel {
    fn effects_bound_style_summary(
        &self,
        name: SharedString,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_bound_style_summary("effects", name, cx)
    }

    fn effects_section(&self, content: AnyElement, cx: &mut Context<DesignPanel>) -> AnyElement {
        self.render_section(
            DesignPanelSection::Effects,
            Some(DesignPanelCollection::Effect),
            content,
            cx,
        )
    }

    fn effects_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_value_cell(id_suffix, prefix, value, property, next, cx)
    }

    fn effects_visibility_button(
        &self,
        id_suffix: impl Into<SharedString>,
        visible: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_visibility_button(id_suffix, visible, property, cx)
    }

    fn effects_settings_for_target(
        &self,
        target: &EffectTargetProjection,
        cx: &mut Context<DesignPanel>,
    ) -> Option<AnyElement> {
        let index = resolve_effect_index(self, target)?;
        let effect = self.host.inspected_node().effects.get(index)?;
        Some(self.render_effect_settings(index, effect, cx))
    }
}

#[derive(Clone)]
enum EffectsEvent {
    ToggleSettings(EffectTargetProjection),
    SetSettings(EffectTargetProjection, bool),
    Reorder {
        dragged: EffectTargetProjection,
        destination: EffectTargetProjection,
    },
    Remove(EffectTargetProjection),
}

/// Narrow callback boundary for the Effects section.
///
/// The renderer can emit Effects-domain events and request the currently
/// projected settings content without retaining the full inspector facade.
#[derive(Clone)]
pub(in super::super) struct EffectsEventSink {
    panel: Entity<DesignPanel>,
}

impl EffectsEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send(&self, event: EffectsEvent, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, event, cx));
    }

    /// Dispatches from a panel listener without recursively updating the
    /// retained entity that the listener is already borrowing mutably.
    fn send_in_context(
        panel: &mut DesignPanel,
        event: EffectsEvent,
        cx: &mut Context<DesignPanel>,
    ) {
        dispatch(panel, event, cx);
    }

    fn set_popover_open(
        &self,
        target: EffectTargetProjection,
        open: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            if open {
                panel.remember_overlay_focus_return(DesignOpenOverlay::EffectSettings, window, cx);
                dispatch(panel, EffectsEvent::SetSettings(target, true), cx);
            } else if settings_target_is_active(panel, &target) {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::EffectSettings,
                    window,
                    cx,
                );
            }
        });
    }

    fn settings_content(
        &self,
        target: &EffectTargetProjection,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.panel.update(cx, |panel, cx| {
            let Some(settings) = panel.effects_settings_for_target(target, cx) else {
                return div().into_any_element();
            };
            v_flex()
                .w(popup_width(window, 292.))
                .p_3()
                .gap_2()
                .child(
                    div().text_xs().font_semibold().child(
                        panel.host.inspected_node().effects
                            [resolve_effect_index(panel, target).expect("validated effect target")]
                        .settings
                        .kind()
                        .label(),
                    ),
                )
                .child(settings)
                .into_any_element()
        })
    }
}

fn resolve_effect_index(panel: &DesignPanel, target: &EffectTargetProjection) -> Option<usize> {
    if panel.host.inspected_node().id != target.node_id
        || !panel.collection_is_supported(DesignPanelCollection::Effect)
        || panel.host.inspected_node().effect_style_binding.is_some()
    {
        return None;
    }
    if target.effect_id.is_empty() {
        panel
            .host
            .inspected_node()
            .effects
            .get(target.index)
            .map(|_| target.index)
    } else {
        panel
            .host
            .inspected_node()
            .effect_index_by_id(target.effect_id.as_ref())
    }
}

fn settings_target_is_active(panel: &DesignPanel, target: &EffectTargetProjection) -> bool {
    let Some(index) = resolve_effect_index(panel, target) else {
        return false;
    };
    panel
        .overlays
        .active_effect_settings()
        .as_ref()
        .is_some_and(|active| active.matches(&panel.host.inspected_node().effects[index], index))
}

fn set_settings_open(
    panel: &mut DesignPanel,
    target: EffectTargetProjection,
    open: bool,
    cx: &mut Context<DesignPanel>,
) {
    let Some(index) = resolve_effect_index(panel, &target) else {
        return;
    };
    let effect_id = panel.host.inspected_node().effects[index].id.clone();
    let live_target = EffectSettingsTarget { index, effect_id };
    if open {
        panel.prepare_paint_picker_for_dismissal(cx);
        panel.cancel_menu_preview(cx);
        panel
            .overlays
            .open(DesignOverlayState::EffectSettings(live_target));
    } else if panel.overlays.active_effect_settings().as_ref() == Some(&live_target) {
        panel.cancel_menu_preview(cx);
        panel.overlays.discard(DesignOpenOverlay::PreviewOptionMenu);
        panel.overlays.discard(DesignOpenOverlay::EffectSettings);
    }
    cx.notify();
}

fn dispatch(panel: &mut DesignPanel, event: EffectsEvent, cx: &mut Context<DesignPanel>) {
    match event {
        EffectsEvent::ToggleSettings(target) => {
            let Some(index) = resolve_effect_index(panel, &target) else {
                return;
            };
            let effect = &panel.host.inspected_node().effects[index];
            let active = panel
                .overlays
                .active_effect_settings()
                .as_ref()
                .is_some_and(|active| active.matches(effect, index));
            set_settings_open(panel, target, !active, cx);
        }
        EffectsEvent::SetSettings(target, open) => set_settings_open(panel, target, open, cx),
        EffectsEvent::Reorder {
            dragged,
            destination,
        } => {
            let Some(from_index) = resolve_effect_index(panel, &dragged) else {
                return;
            };
            let Some(to_index) = resolve_effect_index(panel, &destination) else {
                return;
            };
            let effect_id = panel.host.inspected_node().effects[from_index].id.clone();
            panel.emit_effect_reorder(effect_id, from_index, to_index, cx);
        }
        EffectsEvent::Remove(target) => {
            let Some(index) = resolve_effect_index(panel, &target) else {
                return;
            };
            if !panel.can_edit() {
                return;
            }
            let action = DesignPanelAction::EffectRemoveRequested {
                node_id: panel.host.inspected_node().id.clone(),
                effect_id: panel.host.inspected_node().effects[index].id.clone(),
                index,
            };
            if panel.node_capability_allows_action(&action) {
                cx.emit_design_panel_action(panel, action);
            }
        }
    }
}

fn render_remove_button(
    projection: &EffectsProjection,
    target: EffectTargetProjection,
    index: usize,
    events: &EffectsEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    if !projection.access.can_edit || !projection.access.collection_supported {
        return div().size(px(24.)).flex_none().into_any_element();
    }
    let events = events.clone();
    div()
        .id(SharedString::from(format!(
            "{}-remove-effect-{index}",
            projection.identity.panel_id
        )))
        .key_context(CONTROL_KEY_CONTEXT)
        .tab_index(0)
        .size(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().accent))
        .focus(|style| {
            style
                .bg(cx.theme().accent)
                .border_1()
                .border_color(cx.theme().selection)
        })
        .on_activate(move |_, _, cx| {
            events.send(EffectsEvent::Remove(target.clone()), cx);
        })
        .child(Icon::new(IconName::Minus).xsmall())
        .into_any_element()
}

pub(in super::super) fn render(
    projection: &EffectsProjection,
    chrome: &impl EffectsInspectorChrome,
    events: &EffectsEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let metrics = InspectorMetrics::current();
    if let Some(name) = projection.content.style_binding_name.clone() {
        let content = v_flex()
            .px(px(PANEL_PADDING))
            .pb_4()
            .child(chrome.effects_bound_style_summary(name, cx));
        return chrome.effects_section(content.into_any_element(), cx);
    }
    if projection.content.effects.is_empty() {
        return chrome.effects_section(div().into_any_element(), cx);
    }

    let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_1();
    for (index, effect) in projection.content.effects.iter().enumerate() {
        let can_reorder = projection.access.can_edit
            && projection.access.collection_supported
            && projection.content.effects.len() > 1;
        let active = projection.settings_are_active(effect, index);
        let target = projection.target(effect, index);
        let target_for_keyboard = target.clone();
        let events_for_keyboard = events.clone();
        let settings_trigger = Button::new(SharedString::from(format!(
            "{}-effect-settings-{index}",
            projection.identity.panel_id
        )))
        .tooltip("Effect settings")
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .selected(active)
        .on_keyboard_activate(move |_, cx| {
            events_for_keyboard.send(
                EffectsEvent::ToggleSettings(target_for_keyboard.clone()),
                cx,
            );
        })
        .icon(IconName::Settings2);

        let events_for_open = events.clone();
        let target_for_open = target.clone();
        let events_for_content = events.clone();
        let target_for_content = target.clone();
        let settings_popover = Popover::new(SharedString::from(format!(
            "{}-effect-settings-popover-{index}",
            projection.identity.panel_id
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            events_for_open.set_popover_open(target_for_open.clone(), *open, window, cx);
        })
        .trigger(settings_trigger)
        .content(move |_, window, cx| {
            events_for_content.settings_content(&target_for_content, window, cx)
        });

        let destination = target.clone();
        let drag = EffectDrag {
            effect_id: effect.id.clone(),
            from_index: index,
            label: effect.settings.kind().label().into(),
        };
        let node_id_for_drop = projection.identity.node_id.clone();
        let destination_for_drop = destination.clone();
        let drop_listener = cx.listener(move |panel, drag: &EffectDrag, _, cx| {
            let dragged = EffectTargetProjection::new(
                node_id_for_drop.clone(),
                drag.effect_id.clone(),
                drag.from_index,
            );
            EffectsEventSink::send_in_context(
                panel,
                EffectsEvent::Reorder {
                    dragged,
                    destination: destination_for_drop.clone(),
                },
                cx,
            );
        });
        let can_drop_effect_id = effect.id.clone();
        let drop_effect_id = effect.id.clone();
        let drop_background = cx.theme().selection.opacity(0.18);
        let drop_border = cx.theme().selection;
        let row_access = if can_reorder {
            InspectorFieldAccess::Editable
        } else {
            InspectorFieldAccess::read_only(None)
        };
        let mut swatch_metrics = metrics;
        swatch_metrics.row_height = px(20.);
        let swatch = crate::molecules::InspectorColorSwatch::new(
            InspectorValue::Uniform(color_hsla(effect.color)),
            InspectorFieldPresentation::new(InspectorFieldAccess::read_only(None)),
        )
        .render(color_hsla(effect.color), swatch_metrics, cx)
        .opacity(1.);
        let actions = crate::molecules::inspector_action_group(metrics)
            .gap_1()
            .child(settings_popover)
            .child(chrome.effects_visibility_button(
                format!("effect-visible-{index}"),
                effect.visible,
                DesignPanelProperty::EffectVisible(index),
                cx,
            ))
            .child(render_remove_button(
                projection,
                target.clone(),
                index,
                events,
                cx,
            ));
        let row = crate::molecules::inspector_collection_row(
            SharedString::from(format!(
                "{}-effect-row-{index}",
                projection.identity.panel_id
            )),
            false,
            &row_access,
            metrics,
            cx,
        )
        .px(px(0.))
        .gap_1()
        .when(can_reorder, |row| row.cursor_move())
        .child(
            div()
                .w(px(14.))
                .flex_none()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .when(can_reorder, |handle| handle.child("⠇")),
        )
        .child(swatch)
        .child(chrome.effects_value_cell(
            format!("effect-kind-{index}"),
            "",
            effect.settings.kind().label(),
            DesignPanelProperty::EffectKind(index),
            DesignPanelValue::EffectKind(DesignEffectKind::DropShadow),
            cx,
        ))
        .child(actions)
        .when(can_reorder, move |row| {
            row.on_drag(drag, |drag, _, _, cx| {
                cx.new(|_| EffectDragPreview { drag: drag.clone() })
            })
            .can_drop(move |drag, _, _| {
                drag.downcast_ref::<EffectDrag>().is_some_and(|drag| {
                    if drag.effect_id.is_empty() || can_drop_effect_id.is_empty() {
                        drag.from_index != index
                    } else {
                        drag.effect_id != can_drop_effect_id
                    }
                })
            })
            .drag_over::<EffectDrag>(move |style, drag, _, _| {
                let same = if drag.effect_id.is_empty() || drop_effect_id.is_empty() {
                    drag.from_index == index
                } else {
                    drag.effect_id == drop_effect_id
                };
                if same {
                    style
                } else {
                    style.bg(drop_background).border_color(drop_border)
                }
            })
            .on_drop(drop_listener)
        });
        content = content.child(row);
    }
    chrome.effects_section(content.into_any_element(), cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_target_prefers_effect_identity_over_stale_index() {
        let target = EffectTargetProjection::new("node".into(), "second".into(), 0);
        let effect = DesignEffect::new(DesignEffectKind::DropShadow).with_id("second");
        assert!(target.matches(&effect, 1));
        assert!(!target.matches(
            &DesignEffect::new(DesignEffectKind::DropShadow).with_id("other"),
            0
        ));
    }
}
