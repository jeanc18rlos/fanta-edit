use super::super::super::{
    DesignComponentContext, DesignComponentOrigin, DesignComponentProperty, DesignComponentRole,
};
use super::super::*;
use gpui::ParentElement;

/// Stable identity for one Component/Instance render pass.
#[derive(Clone)]
pub(in super::super) struct ComponentIdentityProjection {
    panel_id: SharedString,
    node_id: SharedString,
    target: DesignPanelTarget,
}

impl ComponentIdentityProjection {
    pub(in super::super) fn new(
        panel_id: SharedString,
        node_id: SharedString,
        target: DesignPanelTarget,
    ) -> Self {
        Self {
            panel_id,
            node_id,
            target,
        }
    }
}

/// Component-system role and its resulting inspector section.
#[derive(Clone, Copy)]
pub(in super::super) struct ComponentRoleProjection {
    role: DesignComponentRole,
    section: DesignPanelSection,
}

impl ComponentRoleProjection {
    pub(in super::super) fn new(role: DesignComponentRole) -> Self {
        Self {
            role,
            section: if role.uses_instance_section() {
                DesignPanelSection::Instance
            } else {
                DesignPanelSection::Component
            },
        }
    }
}

/// Host-provided component metadata displayed before property controls.
#[derive(Clone)]
pub(in super::super) struct ComponentContextProjection {
    context: Option<DesignComponentContext>,
}

impl ComponentContextProjection {
    pub(in super::super) fn new(context: Option<DesignComponentContext>) -> Self {
        Self { context }
    }
}

/// Snapshot availability for actions anchored to a nested-instance heading.
#[derive(Clone, Copy)]
pub(in super::super) struct NestedInstanceActionAccess {
    select: bool,
    go_to_main: bool,
}

impl NestedInstanceActionAccess {
    pub(in super::super) const fn new(select: bool, go_to_main: bool) -> Self {
        Self { select, go_to_main }
    }
}

/// One stable component-property row plus its direct heading actions.
#[derive(Clone)]
pub(in super::super) struct ComponentPropertyProjection {
    index: usize,
    property: DesignComponentProperty,
    nested_actions: NestedInstanceActionAccess,
}

impl ComponentPropertyProjection {
    pub(in super::super) fn new(
        index: usize,
        property: DesignComponentProperty,
        nested_actions: NestedInstanceActionAccess,
    ) -> Self {
        Self {
            index,
            property,
            nested_actions,
        }
    }
}

/// Ordered host property snapshot. Indices are retained because they remain
/// part of the compatibility property contract during this migration.
#[derive(Clone)]
pub(in super::super) struct ComponentPropertiesProjection {
    rows: Vec<ComponentPropertyProjection>,
}

impl ComponentPropertiesProjection {
    pub(in super::super) fn new(rows: Vec<ComponentPropertyProjection>) -> Self {
        Self { rows }
    }
}

/// Which complex authoring surfaces have host data for this selection.
#[derive(Clone, Copy)]
pub(in super::super) struct ComponentAuthoringProjection {
    applied_controls: bool,
    definitions: bool,
    nested_exposures: bool,
}

impl ComponentAuthoringProjection {
    pub(in super::super) const fn new(
        applied_controls: bool,
        definitions: bool,
        nested_exposures: bool,
    ) -> Self {
        Self {
            applied_controls,
            definitions,
            nested_exposures,
        }
    }
}

/// Footer visibility and access remain separate so view-only snapshots retain
/// their disabled controls instead of silently removing them.
#[derive(Clone, Copy)]
pub(in super::super) struct ComponentFooterPresentation {
    reset_visible: bool,
    reset_enabled: bool,
    go_to_main_visible: bool,
    go_to_main_enabled: bool,
    detach_visible: bool,
    detach_enabled: bool,
}

impl ComponentFooterPresentation {
    pub(in super::super) const fn new(
        reset: (bool, bool),
        go_to_main: (bool, bool),
        detach: (bool, bool),
    ) -> Self {
        Self {
            reset_visible: reset.0,
            reset_enabled: reset.1,
            go_to_main_visible: go_to_main.0,
            go_to_main_enabled: go_to_main.1,
            detach_visible: detach.0,
            detach_enabled: detach.1,
        }
    }
}

/// Transient presentation state consumed by retained row editors and footer
/// composition. It never owns document values or property targets.
#[derive(Clone)]
pub(in super::super) struct ComponentPresentationProjection {
    multiline_property_id: Option<SharedString>,
    footer: ComponentFooterPresentation,
}

impl ComponentPresentationProjection {
    pub(in super::super) fn new(
        multiline_property_id: Option<SharedString>,
        footer: ComponentFooterPresentation,
    ) -> Self {
        Self {
            multiline_property_id,
            footer,
        }
    }
}

/// Complete immutable read model for the Component/Instance composition.
#[derive(Clone)]
pub(in super::super) struct ComponentProjection {
    identity: ComponentIdentityProjection,
    role: ComponentRoleProjection,
    context: ComponentContextProjection,
    properties: ComponentPropertiesProjection,
    authoring: ComponentAuthoringProjection,
    presentation: ComponentPresentationProjection,
}

impl ComponentProjection {
    pub(in super::super) fn new(
        identity: ComponentIdentityProjection,
        role: ComponentRoleProjection,
        context: ComponentContextProjection,
        properties: ComponentPropertiesProjection,
        authoring: ComponentAuthoringProjection,
        presentation: ComponentPresentationProjection,
    ) -> Self {
        Self {
            identity,
            role,
            context,
            properties,
            authoring,
            presentation,
        }
    }
}

/// Narrow bridge to retained component-property editors, dialogs, swap
/// browser, authoring controllers, and established section chrome.
pub(in super::super) trait ComponentInspectorChrome {
    fn component_applied_controls(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement>;

    fn component_definition_authoring(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement>;

    fn component_property_row(
        &self,
        role: DesignComponentRole,
        index: usize,
        property: DesignComponentProperty,
        multiline_editor_active: bool,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn component_nested_exposures(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement>;

    fn component_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl ComponentInspectorChrome for DesignPanel {
    fn component_applied_controls(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement> {
        self.render_applied_component_property_controls(
            DesignComponentPropertyApplicationSurface::NestedInstance,
            cx,
        )
    }

    fn component_definition_authoring(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement> {
        self.render_component_definition_authoring(cx)
    }

    fn component_property_row(
        &self,
        role: DesignComponentRole,
        index: usize,
        property: DesignComponentProperty,
        multiline_editor_active: bool,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_component_property_row(role, index, property, multiline_editor_active, cx)
    }

    fn component_nested_exposures(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement> {
        self.render_nested_component_property_exposures(cx)
    }

    fn component_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section(section, None, content, cx)
    }
}

fn direct_action_targets_current_snapshot(
    panel: &DesignPanel,
    identity: &ComponentIdentityProjection,
) -> bool {
    direct_action_identity_matches(
        &panel.host.inspected_node().id,
        &panel.command_target(),
        identity,
    )
}

fn direct_action_identity_matches(
    current_node_id: &SharedString,
    current_target: &DesignPanelTarget,
    identity: &ComponentIdentityProjection,
) -> bool {
    current_node_id == &identity.node_id && current_target == &identity.target
}

fn dispatch_direct_action(
    panel: &mut DesignPanel,
    identity: &ComponentIdentityProjection,
    action: DesignPanelAction,
    cx: &mut Context<DesignPanel>,
) {
    if direct_action_targets_current_snapshot(panel, identity)
        && panel.component_action_is_enabled(&action)
    {
        cx.emit_design_panel_action(panel, action);
    }
}

#[derive(Clone)]
pub(in super::super) struct ComponentEventSink {
    panel: Entity<DesignPanel>,
}

impl ComponentEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn dispatch_direct_action(
        &self,
        identity: ComponentIdentityProjection,
        action: DesignPanelAction,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            dispatch_direct_action(panel, &identity, action, cx);
        });
    }
}

fn render_direct_action_button(
    identity: &ComponentIdentityProjection,
    id_suffix: impl Into<SharedString>,
    label: impl Into<SharedString>,
    action: DesignPanelAction,
    enabled: bool,
    event_sink: ComponentEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let button_id = SharedString::from(format!("{}-{}", identity.panel_id, id_suffix.into()));
    let debug_button_id = button_id.clone();
    let mut button = div()
        .id(button_id)
        .debug_selector(move || debug_button_id.to_string())
        .h(px(ROW_HEIGHT))
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(cx.theme().transparent)
        .bg(cx.theme().secondary)
        .text_xs()
        .when(enabled, |button| {
            button
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().accent))
                .focus(|style| {
                    style
                        .bg(cx.theme().accent)
                        .border_color(cx.theme().selection)
                })
        })
        .when(!enabled, |button| {
            button.text_color(cx.theme().muted_foreground).opacity(0.62)
        });
    if enabled {
        let identity = identity.clone();
        button = button.on_activate(move |_, _, cx| {
            event_sink.dispatch_direct_action(identity.clone(), action.clone(), cx);
        });
    }
    button.child(label.into()).into_any_element()
}

fn render_context<T: ParentElement>(
    projection: &ComponentProjection,
    content: T,
    cx: &mut Context<DesignPanel>,
) -> T {
    let Some(context) = projection.context.context.as_ref() else {
        return content;
    };
    let role = projection.role.role;
    let mut content = content.child(
        h_flex()
            .h(px(24.))
            .justify_between()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(role.label()),
            )
            .when(context.overrides.reset_state.is_overridden(), |row| {
                row.child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(cx.theme().accent)
                        .text_xs()
                        .child(format!(
                            "{} overrides",
                            context.overrides.overridden_property_count
                                + context.overrides.nested_override_count
                        )),
                )
            }),
    );

    if let Some(main) = context.main_component.as_ref() {
        let origin: SharedString = match &main.origin {
            DesignComponentOrigin::Local => "Local".into(),
            DesignComponentOrigin::Remote { library_name } => {
                format!("Library · {library_name}").into()
            }
        };
        let available = main.availability.is_available();
        content = content.child(
            h_flex()
                .min_h(px(42.))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .rounded(px(5.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .child(render_lucide_icon(
                    if role.uses_instance_section() {
                        LucideIcon::Diamond
                    } else {
                        LucideIcon::Component
                    },
                    cx.theme().selection,
                    16.,
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .child(div().truncate().text_xs().child(main.name.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(origin),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if available {
                            cx.theme().green
                        } else {
                            cx.theme().red
                        })
                        .child(main.availability.label()),
                )
                .when(available && role.uses_instance_section(), |row| {
                    row.child(render_lucide_icon(
                        LucideIcon::ChevronRight,
                        cx.theme().foreground,
                        12.,
                    ))
                }),
        );
    }

    if let Some(description) = context.description.as_ref() {
        content = content.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description.clone()),
        );
    }
    for link in &context.documentation_links {
        content = content.child(
            h_flex()
                .gap_2()
                .child(render_lucide_icon(
                    LucideIcon::ExternalLink,
                    cx.theme().selection,
                    16.,
                ))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .child(link.label.clone()),
                )
                .child(
                    div()
                        .max_w(px(120.))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(link.url.clone()),
                ),
        );
    }
    content
}

pub(in super::super) fn render_component(
    projection: &ComponentProjection,
    chrome: &impl ComponentInspectorChrome,
    event_sink: ComponentEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
    content = render_context(projection, content, cx);

    if projection.authoring.applied_controls
        && let Some(applied) = chrome.component_applied_controls(cx)
    {
        content = content.child(applied);
    }
    if projection.authoring.definitions
        && let Some(definitions) = chrome.component_definition_authoring(cx)
    {
        content = content.child(definitions);
    }

    let mut previous_origin = None;
    for row in &projection.properties.rows {
        let property = &row.property;
        if previous_origin.as_ref() != Some(&property.origin) {
            if let DesignComponentPropertyOrigin::NestedInstance {
                instance_id,
                instance_name,
                main_component,
            } = &property.origin
            {
                let mut group_header = h_flex()
                    .w_full()
                    .min_h(px(32.))
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(render_lucide_icon(
                        LucideIcon::Diamond,
                        cx.theme().selection,
                        16.,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .font_semibold()
                                    .child(instance_name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Nested instance"),
                            ),
                    )
                    .child(render_direct_action_button(
                        &projection.identity,
                        format!("nested-select-{instance_id}"),
                        "Select",
                        DesignPanelAction::ComponentPropertyNestedInstanceSelectRequested {
                            node_id: projection.identity.node_id.clone(),
                            property_id: property.id.clone(),
                            instance_id: instance_id.clone(),
                        },
                        row.nested_actions.select,
                        event_sink.clone(),
                        cx,
                    ));
                if let Some(main_component) = main_component.as_ref() {
                    group_header = group_header.child(render_direct_action_button(
                        &projection.identity,
                        format!("nested-go-to-{}-{}", instance_id, main_component.id),
                        "Go to",
                        DesignPanelAction::ComponentPropertyNestedInstanceGoToMainRequested {
                            node_id: projection.identity.node_id.clone(),
                            property_id: property.id.clone(),
                            instance_id: instance_id.clone(),
                            main_component_id: main_component.id.clone(),
                        },
                        row.nested_actions.go_to_main,
                        event_sink.clone(),
                        cx,
                    ));
                }
                content = content.child(group_header);
            }
            previous_origin = Some(property.origin.clone());
        }

        let multiline_editor_active = projection
            .presentation
            .multiline_property_id
            .as_ref()
            .is_some_and(|property_id| *property_id == property.id);
        content = content.child(chrome.component_property_row(
            projection.role.role,
            row.index,
            property.clone(),
            multiline_editor_active,
            cx,
        ));
    }

    if projection.authoring.nested_exposures
        && let Some(exposures) = chrome.component_nested_exposures(cx)
    {
        content = content.child(exposures);
    }

    if projection.role.role.can_reset_instance_overrides() {
        let footer = projection.presentation.footer;
        if footer.reset_visible {
            content = content.child(render_direct_action_button(
                &projection.identity,
                "reset-overrides",
                "Reset all overrides",
                DesignPanelAction::ResetInstanceOverridesRequested {
                    node_id: projection.identity.node_id.clone(),
                },
                footer.reset_enabled,
                event_sink.clone(),
                cx,
            ));
        }
        let mut instance_actions = h_flex().gap_2();
        if footer.go_to_main_visible {
            instance_actions = instance_actions.child(render_direct_action_button(
                &projection.identity,
                "go-to-main",
                "Go to main",
                DesignPanelAction::GoToMainComponentRequested {
                    node_id: projection.identity.node_id.clone(),
                },
                footer.go_to_main_enabled,
                event_sink.clone(),
                cx,
            ));
        }
        if footer.detach_visible {
            instance_actions = instance_actions.child(render_direct_action_button(
                &projection.identity,
                "detach-instance",
                "Detach",
                DesignPanelAction::DetachInstanceRequested {
                    node_id: projection.identity.node_id.clone(),
                },
                footer.detach_enabled,
                event_sink,
                cx,
            ));
        }
        content = content.child(instance_actions);
    }

    chrome.component_section(projection.role.section, content.into_any_element(), cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_projection_selects_component_or_instance_section() {
        assert_eq!(
            ComponentRoleProjection::new(DesignComponentRole::StandaloneMain).section,
            DesignPanelSection::Component
        );
        assert_eq!(
            ComponentRoleProjection::new(DesignComponentRole::Instance).section,
            DesignPanelSection::Instance
        );
    }

    #[test]
    fn footer_visibility_and_access_are_independent() {
        let footer = ComponentFooterPresentation::new((true, false), (true, true), (false, false));
        assert!(footer.reset_visible);
        assert!(!footer.reset_enabled);
        assert!(footer.go_to_main_visible);
        assert!(footer.go_to_main_enabled);
        assert!(!footer.detach_visible);
    }

    #[test]
    fn component_section_has_no_inherent_design_panel_impl() {
        let source = include_str!("component.rs");
        let inherent_impl_marker = ["impl ", "DesignPanel", " {"].concat();
        assert!(!source.contains(&inherent_impl_marker));
        assert!(source.contains("struct ComponentEventSink"));

        let facade = include_str!("../component_props.rs");
        assert!(facade.contains("sections::component::render_component("));
    }

    #[test]
    fn direct_actions_reject_stale_nodes_and_selection_order() {
        let target = DesignPanelTarget::Nodes {
            node_ids: vec!["first".into(), "second".into()],
        };
        let identity =
            ComponentIdentityProjection::new("panel".into(), "first".into(), target.clone());
        assert!(direct_action_identity_matches(
            &"first".into(),
            &target,
            &identity
        ));
        assert!(!direct_action_identity_matches(
            &"stale".into(),
            &target,
            &identity
        ));
        assert!(!direct_action_identity_matches(
            &"first".into(),
            &DesignPanelTarget::Nodes {
                node_ids: vec!["second".into(), "first".into()],
            },
            &identity
        ));
    }
}
