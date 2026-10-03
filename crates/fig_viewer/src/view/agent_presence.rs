use super::*;
use gpui::{
    Animation, AnimationExt as _, Hsla, PathBuilder, canvas, ease_out_quint, pulsating_between,
};
use std::time::Duration;

fn agent_color(agent_id: &str) -> Hsla {
    design_surface::agent_color(agent_id)
}

fn cursor_position(
    world: Option<[f64; 2]>,
    viewport: Option<Viewport>,
    size: DVec2,
    index: usize,
) -> DVec2 {
    let position = match (world, viewport) {
        (Some(world), Some(viewport)) => {
            fanta_canvas::world_to_screen(DVec2::from_array(world), &viewport, size)
        }
        _ => size * 0.5 + DVec2::new(index as f64 * 36.0, index as f64 * 42.0),
    };
    DVec2::new(
        position.x.clamp(12.0, (size.x - 180.0).max(12.0)),
        position.y.clamp(12.0, (size.y - 44.0).max(12.0)),
    )
}

fn cursor_page(document: Option<&FigDocument>) -> Option<usize> {
    document.and_then(|document| {
        document
            .doc
            .active_page()
            .and_then(|root| document.page_index_of_node(root))
    })
}

fn activity_is_on_canvas(
    activity: &design_surface::AgentActivity,
    document: Option<&FigDocument>,
) -> bool {
    if activity.page.is_none() || activity.page == cursor_page(document) {
        return true;
    }
    let Some(document) = document else {
        return false;
    };
    let Some(root) = document.doc.active_page() else {
        return false;
    };
    let Some(node) = activity
        .node
        .as_deref()
        .and_then(|id| id.parse::<NodeId>().ok())
    else {
        return false;
    };
    node == root
        || document
            .doc
            .scene
            .ancestors_of(node)
            .any(|ancestor| ancestor.id == root)
}

fn single_source_writer<'a>(
    activities: &'a [design_surface::AgentActivity],
    project_root: Option<&str>,
) -> Option<&'a design_surface::AgentActivity> {
    let project_root = project_root?;
    let mut activities = activities
        .iter()
        .filter(|activity| activity.project_root.as_deref() == Some(project_root));
    let activity = activities.next()?;
    (activities.next().is_none()
        && activity.action == "Editing source"
        && activity.source_path.is_some()
        && activity.page.is_some())
    .then_some(activity)
}

fn editor_workspace(workspace: design_surface::AgentWorkspace) -> EditorWorkspace {
    match workspace {
        design_surface::AgentWorkspace::Canvas => EditorWorkspace::Canvas,
        design_surface::AgentWorkspace::Variables => EditorWorkspace::Variables,
        design_surface::AgentWorkspace::Code => EditorWorkspace::Code,
    }
}

fn activity_component(
    activity: &design_surface::AgentActivity,
    document: &FigDocument,
) -> Option<(fanta_doc::ComponentId, NodeId)> {
    let root = activity
        .page
        .and_then(|page| document.pages.get(page))
        .and_then(|page| page.root)?;
    document
        .doc
        .components
        .defs
        .values()
        .find(|definition| definition.root == root)
        .map(|definition| (definition.id, root))
}

fn open_agent_component_tab(
    workspace: &mut workspace::Workspace,
    item: Entity<FigItem>,
    component: fanta_doc::ComponentId,
    activity: design_surface::AgentActivity,
    window: &mut Window,
    cx: &mut Context<workspace::Workspace>,
) {
    let scope = Some(FigScope::Component(component));
    let existing = workspace.items_of_type::<FigView>(cx).find(|view| {
        let view = view.read(cx);
        view.item.entity_id() == item.entity_id() && view.scope == scope
    });
    let view = match existing {
        Some(view) => {
            workspace.activate_item(&view, true, false, window, cx);
            view
        }
        None => {
            let project = workspace.project().clone();
            let view = cx.new(|cx| {
                let mut view = FigView::new(item, project, window, cx);
                view.scope = scope;
                view.last_seen_root = None;
                view
            });
            workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
            view
        }
    };
    view.update(cx, |view, cx| {
        if let Some(page) = activity.page {
            view.follow_agent_page(page, cx);
        }
        view.set_editor_workspace(EditorWorkspace::Canvas, cx);
        cx.emit(FigViewEvent::TitleChanged);
    });
}

fn open_agent_source_without_activation(
    workspace: &mut workspace::Workspace,
    source: std::path::PathBuf,
    window: &mut Window,
    cx: &mut Context<workspace::Workspace>,
) -> Task<Result<Box<dyn workspace::item::ItemHandle>>> {
    let path = workspace::Workspace::project_path_for_path(
        workspace.project().clone(),
        &source,
        false,
        cx,
    );
    let pane = workspace.active_pane().downgrade();
    cx.spawn_in(window, async move |workspace, cx| {
        let (_, path) = path.await?;
        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_path_preview(path, Some(pane), false, false, false, window, cx)
            })?
            .await
    })
}

fn cursor_arrow(color: Hsla, background: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let point = |x, y| bounds.origin + gpui::point(px(x), px(y));
            for stroke in [false, true] {
                let mut builder = if stroke {
                    PathBuilder::stroke(px(2.2))
                } else {
                    PathBuilder::fill()
                };
                builder.move_to(point(3.0, 5.0));
                builder.curve_to(point(5.0, 4.0), point(3.0, 2.0));
                builder.line_to(point(28.0, 24.0));
                builder.curve_to(point(27.0, 26.0), point(30.0, 26.0));
                builder.line_to(point(14.0, 26.0));
                builder.curve_to(point(12.0, 27.0), point(13.0, 26.0));
                builder.line_to(point(5.0, 36.0));
                builder.curve_to(point(3.0, 35.0), point(3.0, 39.0));
                builder.close();
                match builder.build() {
                    Ok(path) => window.paint_path(path, if stroke { color } else { background }),
                    Err(error) => log::error!("Could not draw the agent cursor: {error}"),
                }
            }
        },
    )
    .w(px(32.0))
    .h(px(40.0))
}

impl FigView {
    pub(super) fn agent_tab_color(&self, cx: &App) -> Option<Hsla> {
        let state = design_surface::existing_activity_state(cx)?;
        let item = self.item.read(cx);
        let document = item.document()?;
        let project_root = item.project_root().map(|root| root.display().to_string());
        let root = match self.scope {
            Some(FigScope::Page(root)) => root,
            Some(FigScope::Component(component)) => {
                document.doc.components.defs.get(&component)?.root
            }
            _ => return None,
        };
        state
            .read(cx)
            .activities()
            .into_iter()
            .rev()
            .find(|activity| {
                activity.project_root == project_root
                    && activity
                        .page
                        .and_then(|page| document.pages.get(page))
                        .and_then(|page| page.root)
                        == Some(root)
            })
            .map(|activity| agent_color(&activity.agent_id))
    }
    fn follow_agent_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let item = self.item.read(cx);
        let Some(document) = item.document() else {
            return;
        };
        let Some(root) = document.pages.get(page).and_then(|page| page.root) else {
            return;
        };
        let scope = if document.doc.is_component_root(root) {
            document
                .doc
                .components
                .defs
                .values()
                .find(|definition| definition.root == root)
                .map(|definition| FigScope::Component(definition.id))
        } else {
            Some(FigScope::Page(root))
        };
        let Some(scope) = scope else {
            return;
        };
        if !item.content_preview_active() {
            self.select_page(page, cx);
            if self.selected_page_root == Some(root) {
                self.scope = Some(scope);
            }
            return;
        }
        // Navigation is presence state, so following can change pages while
        // the agent keeps ownership of the in-progress content transaction.
        let requester = ScopeRequester::View(cx.entity_id());
        self.item.update(cx, |item, cx| {
            item.request_scope(scope, requester, cx);
        });
        if self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.active_page())
            != Some(root)
        {
            return;
        }
        self.scope = Some(scope);
        self.selected_page_index = Some(page);
        self.selected_page_root = Some(root);
        if self.last_seen_root != Some(root) {
            self.last_seen_root = Some(root);
            self.viewport = None;
            self.hovered_node = None;
            self.invalidate_local_media_origin();
            self.invalidate_canvas_cache();
            cx.emit(FigViewEvent::TitleChanged);
        }
        cx.notify();
    }

    pub(super) fn observe_agent_activity(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        let state = design_surface::activity_state(cx);
        cx.observe_in(&state, window, |view, state, window, cx| {
            let (activities, followed) = {
                let state = state.read(cx);
                (
                    state.activities(),
                    state.followed_agent().map(str::to_string),
                )
            };
            let project_root = view
                .item
                .read(cx)
                .project_root()
                .map(|root| root.display().to_string());
            let workspace = window
                .root::<MultiWorkspace>()
                .flatten()
                .map(|root| root.read(cx).workspace().clone());
            let is_target_view = workspace
                .as_ref()
                .and_then(|workspace| {
                    let workspace = workspace.read(cx);
                    workspace
                        .active_item_as::<FigView>(cx)
                        .or_else(|| workspace.item_of_type::<FigView>(cx))
                })
                .is_some_and(|target| target.entity_id() == cx.entity_id());
            let can_follow = workspace.is_none() || is_target_view;
            let followed_activity = activities.iter().find(|activity| {
                Some(activity.agent_id.as_str()) == followed.as_deref()
                    && activity.project_root == project_root
            });
            let writer = followed_activity
                .filter(|activity| activity.action == "Editing source")
                .or_else(|| single_source_writer(&activities, project_root.as_deref()));
            view.code_workspace.update(cx, |workspace, cx| {
                workspace.set_source_writer(
                    is_target_view.then(|| writer.cloned()).flatten(),
                    window,
                    cx,
                );
            });
            let component_activity = followed_activity
                .or_else(|| {
                    is_target_view
                        .then(|| {
                            activities.iter().rev().find(|activity| {
                                activity.project_root == project_root
                                    && activity.workspace
                                        != Some(design_surface::AgentWorkspace::Code)
                                    && view.item.read(cx).document().is_some_and(|document| {
                                        activity_component(activity, document).is_some()
                                    })
                            })
                        })
                        .flatten()
                })
                .filter(|activity| {
                    workspace.is_some()
                        && can_follow
                        && activity
                            .workspace
                            .is_none_or(|area| area == design_surface::AgentWorkspace::Canvas)
                })
                .and_then(|activity| {
                    let component = activity_component(activity, view.item.read(cx).document()?)?;
                    Some((activity, component))
                });
            if let Some((activity, (component, _))) = component_activity {
                let focus = (
                    activity.agent_id.clone(),
                    activity.source_path.clone(),
                    activity.page,
                    design_surface::AgentWorkspace::Canvas,
                );
                if view.agent_navigation_focus.as_ref() != Some(&focus)
                    || (followed_activity.is_some()
                        && view.scope != Some(FigScope::Component(component)))
                {
                    view.agent_navigation_focus = Some(focus.clone());
                    if let Some(workspace) = workspace.clone() {
                        let item = view.item.clone();
                        let activity = activity.clone();
                        let following = followed_activity.is_some();
                        cx.spawn_in(window, async move |origin, cx| {
                            if !origin
                                .read_with(cx, |view, _| {
                                    view.agent_navigation_focus.as_ref() == Some(&focus)
                                })
                                .is_ok_and(|current| current)
                            {
                                return anyhow::Ok(());
                            }
                            workspace.update_in(cx, |workspace, window, cx| {
                                let target = workspace
                                    .active_item_as::<FigView>(cx)
                                    .or_else(|| workspace.item_of_type::<FigView>(cx));
                                if target
                                    .is_none_or(|target| target.entity_id() != origin.entity_id())
                                {
                                    return;
                                }
                                let state = design_surface::activity_state(cx);
                                if following
                                    && state.read(cx).followed_agent()
                                        != Some(activity.agent_id.as_str())
                                {
                                    return;
                                }
                                open_agent_component_tab(
                                    workspace, item, component, activity, window, cx,
                                );
                            })?;
                            anyhow::Ok(())
                        })
                        .detach_and_log_err(cx);
                    }
                }
            }
            let navigation = followed_activity
                .filter(|_| can_follow)
                .or_else(|| is_target_view.then_some(writer).flatten())
                .filter(|_| component_activity.is_none());
            let previous_zoom = view.viewport.map_or(1.0, |viewport| viewport.zoom);
            if let Some(activity) = navigation {
                let area = activity
                    .workspace
                    .unwrap_or(design_surface::AgentWorkspace::Canvas);
                let source = activity.source_path.clone().or_else(|| {
                    let item = view.item.read(cx);
                    let root = item.project_root()?;
                    let document = item.document()?;
                    let page_root = document.pages.get(activity.page?)?.root?;
                    let component = document
                        .doc
                        .components
                        .defs
                        .values()
                        .find(|definition| definition.root == page_root);
                    component
                        .and_then(|definition| {
                            fanta_format::locate_master_source(root, definition.id)
                        })
                        .or_else(|| fanta_format::locate_page_source(root, page_root))
                        .map(|path| path.display().to_string())
                });
                let focus = (
                    activity.agent_id.clone(),
                    source.clone(),
                    activity.page,
                    area,
                );
                if view.agent_navigation_focus.as_ref() != Some(&focus) {
                    if let Some(page) = activity.page {
                        view.follow_agent_page(page, cx);
                    }
                    view.set_editor_workspace(editor_workspace(area), cx);
                    if area == design_surface::AgentWorkspace::Code {
                        if let Some(source) = &source {
                            view.code_workspace.update(cx, |workspace, cx| {
                                workspace.reveal_source_path(
                                    std::path::Path::new(source),
                                    window,
                                    cx,
                                );
                            });
                        }
                    }
                    let source_ready = source
                        .as_ref()
                        .is_none_or(|path| std::path::Path::new(path).exists());
                    view.agent_navigation_focus = source_ready.then_some(focus.clone());
                    if is_target_view && source_ready {
                        if let (Some(workspace), Some(source)) = (workspace, source) {
                            let activity = activity.clone();
                            let following = followed_activity.is_some();
                            cx.spawn_in(window, async move |origin, cx| {
                                if !origin
                                    .read_with(cx, |view, _| {
                                        view.agent_navigation_focus.as_ref() == Some(&focus)
                                    })
                                    .is_ok_and(|current| current)
                                {
                                    return anyhow::Ok(());
                                }
                                let open = workspace.update_in(cx, |workspace, window, cx| {
                                    let target = workspace
                                        .active_item_as::<FigView>(cx)
                                        .or_else(|| workspace.item_of_type::<FigView>(cx));
                                    if target.is_none_or(|target| {
                                        target.entity_id() != origin.entity_id()
                                    }) {
                                        return None;
                                    }
                                    let state = design_surface::activity_state(cx);
                                    if following
                                        && state.read(cx).followed_agent()
                                            != Some(activity.agent_id.as_str())
                                    {
                                        return None;
                                    }
                                    Some(open_agent_source_without_activation(
                                        workspace,
                                        std::path::PathBuf::from(&source),
                                        window,
                                        cx,
                                    ))
                                })?;
                                let Some(open) = open else {
                                    return anyhow::Ok(());
                                };
                                let opened = open.await?;
                                if !origin
                                    .read_with(cx, |view, _| {
                                        view.agent_navigation_focus.as_ref() == Some(&focus)
                                    })
                                    .is_ok_and(|current| current)
                                {
                                    return anyhow::Ok(());
                                }
                                workspace.update_in(cx, |workspace, window, cx| {
                                    let target = workspace
                                        .active_item_as::<FigView>(cx)
                                        .or_else(|| workspace.item_of_type::<FigView>(cx));
                                    if target.is_none_or(|target| {
                                        target.entity_id() != origin.entity_id()
                                            && target.entity_id() != opened.item_id()
                                    }) {
                                        return;
                                    }
                                    let state = design_surface::activity_state(cx);
                                    if following
                                        && state.read(cx).followed_agent()
                                            != Some(activity.agent_id.as_str())
                                    {
                                        return;
                                    }
                                    if let Some(view) = opened.downcast::<FigView>() {
                                        view.update(cx, |view, cx| {
                                            if let Some(page) = activity.page {
                                                view.follow_agent_page(page, cx);
                                            }
                                            view.set_editor_workspace(editor_workspace(area), cx);
                                            if area == design_surface::AgentWorkspace::Code {
                                                view.code_workspace.update(cx, |workspace, cx| {
                                                    workspace.reveal_source_path(
                                                        std::path::Path::new(&source),
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }
                                        });
                                    }
                                    workspace.activate_item(&*opened, true, false, window, cx);
                                })?;
                                anyhow::Ok(())
                            })
                            .detach_and_log_err(cx);
                        }
                    }
                }
            } else if component_activity.is_none() {
                view.agent_navigation_focus = None;
            }
            if let Some(activity) = followed_activity.filter(|_| {
                can_follow
                    && component_activity.is_none_or(|(_, (component, _))| {
                        view.scope == Some(FigScope::Component(component))
                    })
            }) {
                if let Some(world) = activity.world {
                    let visible = view.viewport.zip(view.container_bounds).is_some_and(
                        |(viewport, bounds)| {
                            let (width, height) = bounds_size(bounds);
                            let size = DVec2::new(width, height);
                            let position = fanta_canvas::world_to_screen(
                                DVec2::from_array(world),
                                &viewport,
                                size,
                            );
                            let margin = 60.0f64.min(width.min(height) * 0.2);
                            position.x >= margin
                                && position.y >= margin
                                && position.x <= width - margin
                                && position.y <= height - margin
                        },
                    );
                    if !visible {
                        view.viewport = Some(Viewport {
                            center: world,
                            zoom: previous_zoom,
                        });
                    }
                }
            }
            cx.notify();
        })
    }

    pub(super) fn render_agent_presence(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = design_surface::activity_state(cx);
        let project_root = self
            .item
            .read(cx)
            .project_root()
            .map(|root| root.display().to_string());
        let activities: Vec<_> = state
            .read(cx)
            .activities()
            .into_iter()
            .filter(|activity| activity.project_root == project_root)
            .collect();
        if activities.is_empty() {
            return None;
        }
        let followed = state.read(cx).followed_agent().map(str::to_string);
        let background = cx.theme().colors().elevated_surface_background;
        let mut overlay = div().absolute().inset_0().overflow_hidden();
        let mut controls = v_flex()
            .absolute()
            .top(px(54.0))
            .right_2()
            .gap_1()
            .p_2()
            .rounded_md()
            .bg(background)
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(Label::new("Agents").size(LabelSize::Small));
        for (index, activity) in activities.into_iter().enumerate() {
            let is_followed = followed.as_deref() == Some(activity.agent_id.as_str());
            let label = format!("{} · {}", activity.agent_name, activity.action);
            let agent_id = activity.agent_id.clone();
            let follow_state = state.clone();
            controls = controls.child(
                Button::new(format!("follow-agent-{agent_id}"), label)
                    .toggle_state(is_followed)
                    .tooltip(Tooltip::text(if is_followed {
                        "Stop following this agent"
                    } else {
                        "Follow this agent across the design"
                    }))
                    .on_click(move |_, _, cx| {
                        follow_state.update(cx, |state, cx| {
                            state.follow((!is_followed).then(|| agent_id.clone()), cx);
                        });
                    }),
            );
            if self.editor_workspace(cx) != EditorWorkspace::Canvas
                || activity
                    .workspace
                    .is_some_and(|workspace| workspace != design_surface::AgentWorkspace::Canvas)
            {
                continue;
            }
            if !activity_is_on_canvas(&activity, self.item.read(cx).document()) {
                continue;
            }
            if let Some(bounds) = self.container_bounds {
                let screen = cursor_position(
                    activity.world,
                    self.viewport,
                    DVec2::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
                    index,
                );
                let color = agent_color(&activity.agent_id);
                let (previous_world, revision) = state
                    .read(cx)
                    .cursor_motion(&activity.agent_id)
                    .unwrap_or((None, 0));
                let previous_screen = cursor_position(
                    previous_world,
                    self.viewport,
                    DVec2::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
                    index,
                );
                if let Some(viewport) = self.viewport
                    && let Some(document) = self.item.read(cx).document()
                    && let Some(node) = activity
                        .node
                        .as_deref()
                        .and_then(|id| id.parse::<NodeId>().ok())
                    && let Some(region) = document
                        .doc
                        .scene
                        .world_bounds(node)
                        .filter(fanta_doc::Bounds::is_finite)
                {
                    let size =
                        DVec2::new(f64::from(bounds.size.width), f64::from(bounds.size.height));
                    let origin = fanta_canvas::world_to_screen(
                        DVec2::new(region.min_x, region.min_y),
                        &viewport,
                        size,
                    );
                    overlay = overlay.child(
                        div()
                            .absolute()
                            .left(px(origin.x as f32 - 3.0))
                            .top(px(origin.y as f32 - 3.0))
                            .w(px((region.width() * viewport.zoom) as f32 + 6.0))
                            .h(px((region.height() * viewport.zoom) as f32 + 6.0))
                            .border_2()
                            .border_color(color)
                            .bg(color.opacity(0.08))
                            .rounded_sm()
                            .with_animation(
                                format!("agent-region-{}-{revision}", activity.agent_id),
                                Animation::new(Duration::from_millis(900))
                                    .repeat()
                                    .with_easing(pulsating_between(0.55, 1.0)),
                                |element, opacity| element.opacity(opacity),
                            ),
                    );
                }
                overlay = overlay.child(
                    h_flex()
                        .id(format!("agent-cursor-{}", activity.agent_id))
                        .absolute()
                        .left(px(screen.x as f32))
                        .top(px(screen.y as f32))
                        .gap_1()
                        .child(
                            div().child(cursor_arrow(color, background)).with_animation(
                                format!("agent-cursor-pulse-{}", activity.agent_id),
                                Animation::new(Duration::from_millis(1600))
                                    .repeat()
                                    .with_easing(pulsating_between(0.8, 1.0)),
                                |element, opacity| element.opacity(opacity),
                            ),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(color)
                                .shadow_sm()
                                .child(
                                    Label::new(activity.agent_name)
                                        .size(LabelSize::Small)
                                        .color(Color::Custom(gpui::white())),
                                ),
                        )
                        .with_animation(
                            format!("agent-cursor-move-{}-{revision}", activity.agent_id),
                            Animation::new(Duration::from_millis(180))
                                .with_easing(ease_out_quint()),
                            move |element, delta| {
                                let position = previous_screen.lerp(screen, f64::from(delta));
                                element
                                    .left(px(position.x as f32))
                                    .top(px(position.y as f32))
                            },
                        ),
                );
            }
        }
        Some(overlay.child(controls).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use settings::Settings as _;

    #[test]
    fn component_activity_stays_visible_on_its_containing_canvas() -> Result<()> {
        let mut doc = Doc::new();
        let home = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        let home_root = home.id;
        doc.apply(Operation::create_node(home))?;
        doc.add_page(home_root);
        let other = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        let other_root = other.id;
        doc.apply(Operation::create_node(other))?;
        doc.add_page(other_root);
        let mut master = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        master.parent = Some(home_root);
        let master_root = master.id;
        doc.apply(Operation::create_node(master))?;
        doc.apply(Operation::DefineComponent {
            def: Box::new(fanta_doc::ComponentDef::new(
                fanta_doc::ComponentId::new(),
                master_root,
                "Card",
            )),
        })?;
        let mut document = FigDocument::from_doc(doc, BTreeMap::new());
        let activity = design_surface::AgentActivity {
            agent_id: "designer".into(),
            agent_name: "Morgana".into(),
            action: "Building Card".into(),
            page: Some(2),
            node: Some(master_root.to_string()),
            world: None,
            active: true,
            project_root: None,
            source_path: None,
            workspace: Some(design_surface::AgentWorkspace::Canvas),
        };
        assert!(activity_is_on_canvas(&activity, Some(&document)));
        document.doc.set_active_page(Some(other_root));
        assert!(!activity_is_on_canvas(&activity, Some(&document)));
        document.doc.set_active_page(Some(master_root));
        assert!(activity_is_on_canvas(&activity, Some(&document)));
        Ok(())
    }

    #[gpui::test]
    async fn component_activity_opens_a_reusable_tab_before_its_source_exists(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            cx.update(|cx| {
                let settings_store = settings::SettingsStore::test(cx);
                cx.set_global(settings_store);
                theme_settings::init(theme::LoadThemes::JustBase, cx);
                project::DisableAiSettings::register(cx);
            });
            let project = Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
            let mut doc = Doc::new();
            let mut page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
            page.name = "Home".into();
            let page_root = page.id;
            doc.apply(Operation::create_node(page))?;
            doc.add_page(page_root);
            let mut master = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
            master.parent = Some(page_root);
            let master_root = master.id;
            doc.apply(Operation::create_node(master))?;
            let component = fanta_doc::ComponentId::new();
            doc.apply(Operation::DefineComponent {
                def: Box::new(fanta_doc::ComponentDef::new(component, master_root, "Card")),
            })?;
            let item = crate::document::ready_item_for_test(
                &project,
                std::path::PathBuf::from("/tmp/Unsaved.fig"),
                doc,
                cx,
            );
            let window =
                cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
            let saved_viewport = Viewport {
                center: [110.0, -20.0],
                zoom: 3.0,
            };
            let (workspace, original) = window.update(cx, |root, window, cx| {
                let workspace = root.workspace().clone();
                let original = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
                workspace.update(cx, |workspace, cx| {
                    workspace.add_item_to_active_pane(
                        Box::new(original.clone()),
                        None,
                        true,
                        window,
                        cx,
                    );
                });
                original.update(cx, |view, cx| {
                    view.follow_agent_page(0, cx);
                    view.viewport = Some(saved_viewport);
                });
                (workspace, original)
            })?;
            let component_page = item
                .read_with(cx, |item, _| {
                    item.document()
                        .and_then(|document| document.page_index_of_node(master_root))
                })
                .context("component scope")?;
            let activity = design_surface::AgentActivity {
                agent_id: "designer".into(),
                agent_name: "Morgana".into(),
                action: "Building Card".into(),
                page: Some(component_page),
                node: Some(master_root.to_string()),
                world: Some([900.0, 800.0]),
                active: true,
                project_root: None,
                source_path: None,
                workspace: Some(design_surface::AgentWorkspace::Canvas),
            };
            let state = cx.update(design_surface::activity_state);
            state.update(cx, |state, cx| state.record(activity.clone(), cx));
            cx.run_until_parked();
            let component_view = workspace
                .read_with(cx, |workspace, cx| {
                    assert_eq!(workspace.items_of_type::<FigView>(cx).count(), 2);
                    workspace.active_item_as::<FigView>(cx)
                })
                .context("active component tab")?;
            assert_ne!(original.entity_id(), component_view.entity_id());
            component_view.read_with(cx, |view, cx| {
                assert_eq!(view.scope, Some(FigScope::Component(component)));
                assert_eq!(view.selected_page_root, Some(master_root));
                assert_eq!(view.agent_tab_color(cx), Some(agent_color("designer")));
                assert_eq!(view.tab_content_text(0, cx).as_ref(), "Card");
            });
            original.read_with(cx, |view, _| {
                assert_eq!(view.scope, Some(FigScope::Page(page_root)));
                assert_eq!(view.viewport, Some(saved_viewport));
            });
            state.update(cx, |state, cx| state.record(activity, cx));
            cx.run_until_parked();
            workspace.read_with(cx, |workspace, cx| {
                assert_eq!(workspace.items_of_type::<FigView>(cx).count(), 2);
                assert_eq!(
                    workspace.active_item_as::<FigView>(cx),
                    Some(component_view.clone())
                );
            });
            window.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    assert!(workspace.activate_item(&original, true, false, window, cx));
                });
            })?;
            cx.run_until_parked();
            state.update(cx, |state, cx| state.follow(Some("designer".into()), cx));
            cx.run_until_parked();
            workspace.read_with(cx, |workspace, cx| {
                assert_eq!(workspace.items_of_type::<FigView>(cx).count(), 2);
                assert_eq!(
                    workspace.active_item_as::<FigView>(cx),
                    Some(component_view)
                );
            });
            original.read_with(cx, |view, _| {
                assert_eq!(view.scope, Some(FigScope::Page(page_root)));
                assert_eq!(view.viewport, Some(saved_viewport));
            });
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("component activity should keep the page tab and reuse its component tab");
    }

    #[gpui::test]
    async fn opening_an_agent_source_waits_for_activation_before_changing_scope(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            cx.update(|cx| {
                let settings_store = settings::SettingsStore::test(cx);
                cx.set_global(settings_store);
                theme_settings::init(theme::LoadThemes::JustBase, cx);
                project::DisableAiSettings::register(cx);
                workspace::register_project_item::<FigView>(cx);
            });
            cx.executor().allow_parking();
            let directory = tempfile::tempdir()?;
            let mut doc = Doc::new();
            let mut roots = Vec::new();
            for name in ["Home", "Library"] {
                let mut page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
                page.name = name.into();
                let root = page.id;
                doc.apply(Operation::create_node(page))?;
                doc.add_page(root);
                roots.push(root);
            }
            let first = *roots.first().context("first page")?;
            let second = *roots.get(1).context("second page")?;
            doc.set_active_page(Some(first));
            crate::document::write_project(directory.path(), &doc, &BTreeMap::new())?;
            let source = fanta_format::locate_page_source(directory.path(), second)
                .context("second page source")?;
            let file_system = Arc::new(fs::RealFs::new(None, cx.executor()));
            let project = Project::test(file_system, [directory.path()], cx).await;
            let window = cx.add_window(|_, _| gpui::Empty);
            let workspace = window.update(cx, |_, window, cx| {
                cx.new(|cx| workspace::Workspace::test_new(project, window, cx))
            })?;
            let open = window.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_abs_path(
                        directory.path().join("fanta.json"),
                        workspace::OpenOptions {
                            focus: Some(false),
                            ..Default::default()
                        },
                        window,
                        cx,
                    )
                })
            })?;
            let initial = open.await?;
            cx.run_until_parked();
            let initial_view = initial
                .downcast::<FigView>()
                .context("initial design tab")?;
            initial_view.update(cx, |view, cx| view.follow_agent_page(0, cx));
            let item = initial_view.read_with(cx, |view, _| view.item.clone());
            let open = window.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    open_agent_source_without_activation(workspace, source, window, cx)
                })
            })?;
            let background = open.await?;
            cx.run_until_parked();
            let background_view = background.downcast::<FigView>().context("background tab")?;
            assert_ne!(background.item_id(), initial.item_id());
            assert_eq!(
                workspace.read_with(cx, |workspace, cx| workspace
                    .active_item(cx)
                    .map(|item| item.item_id())),
                Some(initial.item_id()),
                "a delayed agent source load must not activate its tab"
            );
            assert_eq!(
                item.read_with(cx, |item, _| item.doc().and_then(Doc::active_page)),
                Some(first)
            );
            background_view.read_with(cx, |view, _| {
                assert_eq!(view.item.entity_id(), item.entity_id());
                assert_eq!(view.scope, Some(FigScope::Page(second)));
            });
            window.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    assert!(workspace.activate_item(&*background, true, false, window, cx));
                });
            })?;
            cx.run_until_parked();
            assert_eq!(
                item.read_with(cx, |item, _| item.doc().and_then(Doc::active_page)),
                Some(second)
            );
            initial_view.read_with(cx, |view, _| {
                assert_eq!(view.scope, Some(FigScope::Page(first)));
            });
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("agent source tabs must apply their scope only when activated");
    }

    #[gpui::test]
    async fn following_an_agent_preserves_inactive_scoped_tabs(cx: &mut gpui::TestAppContext) {
        async {
            cx.update(|cx| {
                let settings_store = settings::SettingsStore::test(cx);
                cx.set_global(settings_store);
                theme_settings::init(theme::LoadThemes::JustBase, cx);
                project::DisableAiSettings::register(cx);
            });
            let project = project::Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
            let mut doc = Doc::new();
            let mut roots = Vec::new();
            for name in ["Home", "Library"] {
                let mut page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
                page.name = name.into();
                let root = page.id;
                doc.apply(Operation::create_node(page))?;
                doc.add_page(root);
                roots.push(root);
            }
            let first = *roots.first().context("first page")?;
            let second = *roots.get(1).context("second page")?;
            let item = crate::document::ready_item_for_test(
                &project,
                std::path::PathBuf::from("/tmp/Design.fig"),
                doc,
                cx,
            );
            let window =
                cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
            let saved_viewport = Viewport {
                center: [110.0, -20.0],
                zoom: 3.0,
            };
            let (inactive, active) = window.update(cx, |root, window, cx| {
                let workspace = root.workspace().clone();
                let inactive = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
                let active = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
                workspace.update(cx, |workspace, cx| {
                    workspace.add_item_to_active_pane(
                        Box::new(inactive.clone()),
                        None,
                        false,
                        window,
                        cx,
                    );
                    workspace.add_item_to_active_pane(
                        Box::new(active.clone()),
                        None,
                        true,
                        window,
                        cx,
                    );
                });
                inactive.update(cx, |view, cx| {
                    view.follow_agent_page(0, cx);
                    view.viewport = Some(saved_viewport);
                });
                active.update(cx, |view, cx| {
                    view.follow_agent_page(0, cx);
                    view.viewport = Some(Viewport {
                        center: [0.0, 0.0],
                        zoom: 2.0,
                    });
                });
                (inactive, active)
            })?;
            let state = cx.update(design_surface::activity_state);
            state.update(cx, |state, cx| {
                state.record(
                    design_surface::AgentActivity {
                        agent_id: "reviewer".into(),
                        agent_name: "Reviewer".into(),
                        action: "Building Library".into(),
                        page: Some(1),
                        node: None,
                        world: Some([300.0, 400.0]),
                        active: true,
                        project_root: None,
                        source_path: None,
                        workspace: Some(design_surface::AgentWorkspace::Canvas),
                    },
                    cx,
                );
                state.follow(Some("reviewer".into()), cx);
            });
            cx.run_until_parked();
            inactive.read_with(cx, |view, _| {
                assert_eq!(view.scope, Some(FigScope::Page(first)));
                assert_eq!(view.selected_page_root, Some(first));
                assert_eq!(view.viewport, Some(saved_viewport));
            });
            active.read_with(cx, |view, _| {
                assert_eq!(view.scope, Some(FigScope::Page(second)));
                assert_eq!(view.selected_page_root, Some(second));
                assert_eq!(
                    view.viewport,
                    Some(Viewport {
                        center: [300.0, 400.0],
                        zoom: 2.0
                    })
                );
            });
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("following must preserve inactive tabs");
    }

    #[gpui::test]
    async fn following_pages_and_components_keeps_the_agent_preview_owned(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            cx.update(|cx| {
                let settings_store = settings::SettingsStore::test(cx);
                cx.set_global(settings_store);
            });
            let project = project::Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
            let mut doc = Doc::new();
            let mut roots = Vec::new();
            for name in ["Home", "Components"] {
                let mut page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
                page.name = name.into();
                let root = page.id;
                doc.apply(Operation::create_node(page))?;
                doc.add_page(root);
                roots.push(root);
            }
            let first = *roots.first().context("first page")?;
            let second = *roots.get(1).context("second page")?;
            let component = fanta_doc::ComponentId::new();
            doc.components.defs.insert(
                component,
                fanta_doc::ComponentDef::new(component, second, "Card"),
            );
            let item = crate::document::ready_item_for_test(
                &project,
                std::path::PathBuf::from("/tmp/Design.fig"),
                doc,
                cx,
            );
            let window = cx.add_window(|_, _| gpui::Empty);
            let view = window.update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project, window, cx))
            })?;
            view.update(cx, |view, cx| view.follow_agent_page(0, cx));
            let owner = cx.new(|_| ());
            item.update(cx, |item, cx| {
                item.with_document_for_preview_owner(owner.entity_id(), cx, |document| {
                    document
                        .doc
                        .history
                        .begin("Agent edit", &mut document.doc.scene);
                    ((), DocChange::ContentPreview)
                });
            });
            view.update(cx, |view, cx| view.follow_agent_page(1, cx));
            view.read_with(cx, |view, _| {
                assert_eq!(view.selected_page_root, Some(second));
                assert_eq!(view.scope, Some(FigScope::Component(component)));
            });
            item.read_with(cx, |item, _| {
                assert!(item.content_preview_active());
                assert!(item.can_preview_for_owner(owner.entity_id()));
                assert_eq!(item.doc().and_then(Doc::active_page), Some(second));
            });
            view.update(cx, |view, cx| view.follow_agent_page(0, cx));
            view.read_with(cx, |view, _| {
                assert_eq!(view.selected_page_root, Some(first));
                assert_eq!(view.scope, Some(FigScope::Page(first)));
            });
            item.update(cx, |item, cx| {
                item.with_document_for_owner(owner.entity_id(), cx, |document| {
                    let result = document.doc.abort_transaction();
                    (result, DocChange::ContentPreview)
                })
                .context("preview document")??;
                assert!(item.finish_content_preview(owner.entity_id(), false, cx));
                Ok::<_, anyhow::Error>(())
            })?;
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("following must retain the agent preview owner");
    }

    #[test]
    fn cursor_page_uses_the_active_document_page_after_reopening() -> Result<()> {
        let mut doc = Doc::new();
        let first_page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        let first_root = first_page.id;
        doc.apply(Operation::create_node(first_page))?;
        doc.add_page(first_root);
        let second_page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        let second_root = second_page.id;
        doc.apply(Operation::create_node(second_page))?;
        doc.add_page(second_root);
        doc.set_active_page(None);

        let mut document = FigDocument::from_doc(doc, BTreeMap::new());
        let selected_page_index: Option<usize> = None;
        let active_page_index = document
            .page_index(selected_page_index)
            .ok_or_else(|| anyhow::anyhow!("reopened document has no default page"))?;
        let (other_page_index, other_page_root) = document
            .pages
            .iter()
            .enumerate()
            .find_map(|(index, page)| {
                (index != active_page_index)
                    .then_some(page.root)
                    .flatten()
                    .map(|root| (index, root))
            })
            .ok_or_else(|| anyhow::anyhow!("reopened document has no other page"))?;
        assert_eq!(
            document.doc.active_page(),
            document
                .page(selected_page_index)
                .and_then(|page| page.root)
        );
        assert_eq!(cursor_page(Some(&document)), Some(active_page_index));
        assert_ne!(cursor_page(Some(&document)), selected_page_index);

        let visible_cursor_pages = |document: &FigDocument| {
            let active_page = cursor_page(Some(document));
            [Some(active_page_index), Some(other_page_index), None]
                .into_iter()
                .filter(|page| page.is_none() || *page == active_page)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            visible_cursor_pages(&document),
            [Some(active_page_index), None]
        );

        document.doc.set_active_page(Some(other_page_root));
        assert_eq!(cursor_page(Some(&document)), Some(other_page_index));
        assert_eq!(
            visible_cursor_pages(&document),
            [Some(other_page_index), None]
        );
        assert_eq!(cursor_page(None), None);
        Ok(())
    }

    #[test]
    fn source_follow_requires_one_writer_in_the_same_project() {
        let writer = design_surface::AgentActivity {
            agent_id: "lead".into(),
            agent_name: "Fanta".into(),
            action: "Editing source".into(),
            page: Some(1),
            node: None,
            world: None,
            active: true,
            project_root: Some("/design".into()),
            source_path: Some("/design/pages/Home/page.fnx".into()),
            workspace: None,
        };
        assert!(single_source_writer(std::slice::from_ref(&writer), Some("/design")).is_some());
        assert!(single_source_writer(std::slice::from_ref(&writer), Some("/other")).is_none());
        assert!(single_source_writer(std::slice::from_ref(&writer), None).is_none());
        let mut reviewer = writer.clone();
        reviewer.agent_id = "reviewer".into();
        reviewer.action = "Reviewing layout".into();
        reviewer.source_path = None;
        assert!(
            single_source_writer(&[writer.clone(), reviewer.clone()], Some("/design")).is_none()
        );
        reviewer.project_root = Some("/other".into());
        assert!(single_source_writer(&[writer, reviewer], Some("/design")).is_some());
    }

    #[test]
    fn cursor_is_visible_without_reported_coordinates_and_for_offscreen_work() {
        let size = DVec2::new(1000.0, 800.0);
        assert_eq!(cursor_position(None, None, size, 0), size * 0.5);
        assert_ne!(
            cursor_position(None, None, size, 0),
            cursor_position(None, None, size, 1)
        );
        assert_eq!(
            cursor_position(
                Some([10000.0, -10000.0]),
                Some(Viewport {
                    center: [0.0, 0.0],
                    zoom: 1.0
                }),
                size,
                0,
            ),
            DVec2::new(820.0, 12.0)
        );
        assert_eq!(
            cursor_position(None, None, DVec2::ZERO, 0),
            DVec2::splat(12.0)
        );
    }
}
