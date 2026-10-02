use super::*;
use gpui::{Animation, AnimationExt as _, BoxShadow, Hsla, PathBuilder, canvas, pulsating_between};
use std::time::Duration;

fn agent_color(agent_id: &str) -> Hsla {
    let identity = agent_id.bytes().fold(0u32, |identity, byte| {
        identity.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    gpui::rgb(match identity % 4 {
        0 => 0x387bff,
        1 => 0xd83bea,
        2 => 0x008b68,
        _ => 0xc65b06,
    })
    .into()
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

fn cursor_arrow(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let vertices = [
                (2.0, 2.0),
                (3.0, 25.0),
                (9.0, 19.0),
                (15.0, 30.0),
                (20.0, 27.0),
                (14.0, 17.0),
                (23.0, 16.0),
            ];
            for stroke in [false, true] {
                let mut builder = if stroke {
                    PathBuilder::stroke(px(1.8))
                } else {
                    PathBuilder::fill()
                };
                for (index, (x, y)) in vertices.into_iter().enumerate() {
                    let vertex = bounds.origin + gpui::point(px(x), px(y));
                    if index == 0 {
                        builder.move_to(vertex);
                    } else {
                        builder.line_to(vertex);
                    }
                }
                builder.close();
                match builder.build() {
                    Ok(path) => window.paint_path(path, if stroke { gpui::white() } else { color }),
                    Err(error) => log::error!("Could not draw the agent cursor: {error}"),
                }
            }
        },
    )
    .w(px(25.0))
    .h(px(32.0))
}

impl FigView {
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
            if is_target_view {
                let writer = single_source_writer(&activities, project_root.as_deref());
                let writer_focus = writer.and_then(|writer| {
                    writer
                        .source_path
                        .as_ref()
                        .map(|path| (writer.agent_id.clone(), path.clone()))
                });
                if writer_focus != view.source_writer_focus {
                    if let Some(writer) = writer {
                        if let Some(page) = writer.page {
                            view.select_page(page, cx);
                            if view.selected_page_index == Some(page) {
                                view.set_editor_workspace(EditorWorkspace::Code, cx);
                                if let Some(workspace) = workspace {
                                    let view = cx.entity();
                                    window.defer(cx, move |window, cx| {
                                        workspace.update(cx, |workspace, cx| {
                                            workspace.activate_item(&view, true, false, window, cx);
                                        });
                                    });
                                }
                            }
                        }
                    }
                    // Repeated chunks must not override a user's return to Canvas.
                    view.source_writer_focus = writer_focus;
                }
                view.code_workspace.update(cx, |workspace, cx| {
                    workspace.set_source_writer(writer.cloned(), window, cx);
                });
            } else {
                view.code_workspace.update(cx, |workspace, cx| {
                    workspace.set_source_writer(None, window, cx);
                });
            }
            let activity = activities.iter().find(|activity| {
                Some(activity.agent_id.as_str()) == followed.as_deref()
                    && activity.project_root == project_root
            });
            if let Some(activity) = activity {
                let zoom = view.viewport.map_or(1.0, |viewport| viewport.zoom);
                if let Some(page) = activity.page {
                    if view.selected_page_index != Some(page) {
                        view.select_page(page, cx);
                    }
                    if view.selected_page_index != Some(page) {
                        cx.notify();
                        return;
                    }
                }
                if let Some(world) = activity.world {
                    view.viewport = Some(Viewport {
                        center: world,
                        zoom,
                    });
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
        let active_page = cursor_page(self.item.read(cx).document());
        let blue: Hsla = gpui::rgb(0x387bff).into();
        let magenta: Hsla = gpui::rgb(0xe344ff).into();
        let background = cx.theme().colors().elevated_surface_background;
        let mut overlay = div().absolute().inset_0().overflow_hidden().child(
            div()
                .absolute()
                .inset_0()
                .border(px(5.0))
                .border_color(blue)
                .shadow(vec![
                    BoxShadow::new(px(0.0), px(0.0), magenta.opacity(0.8))
                        .blur_radius(px(30.0))
                        .spread_radius(px(10.0))
                        .inset(),
                    BoxShadow::new(px(0.0), px(0.0), blue.opacity(0.9))
                        .blur_radius(px(14.0))
                        .spread_radius(px(5.0))
                        .inset(),
                ])
                .with_animation(
                    "fanta-agent-canvas-presence",
                    Animation::new(Duration::from_millis(1600))
                        .repeat()
                        .with_easing(pulsating_between(0.65, 1.0)),
                    |element, opacity| element.opacity(opacity),
                ),
        );
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
            .child(Label::new("Agents on canvas").size(LabelSize::Small));
        for (index, activity) in activities.into_iter().enumerate() {
            let is_followed = followed.as_deref() == Some(activity.agent_id.as_str());
            let label = format!("{} · {}", activity.agent_name, activity.action);
            let agent_id = activity.agent_id.clone();
            let state = state.clone();
            controls = controls.child(
                Button::new(format!("follow-agent-{agent_id}"), label)
                    .toggle_state(is_followed)
                    .tooltip(Tooltip::text(if is_followed {
                        "Stop following this agent"
                    } else {
                        "Follow this agent on the canvas"
                    }))
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.follow((!is_followed).then(|| agent_id.clone()), cx);
                        });
                    }),
            );
            if activity.page.is_some() && activity.page != active_page {
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
                overlay = overlay.child(
                    h_flex()
                        .id(format!("agent-cursor-{}", activity.agent_id))
                        .absolute()
                        .left(px(screen.x as f32))
                        .top(px(screen.y as f32))
                        .gap_1()
                        .child(cursor_arrow(color))
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
                            format!("agent-cursor-pulse-{}", activity.agent_id),
                            Animation::new(Duration::from_millis(1200))
                                .repeat()
                                .with_easing(pulsating_between(0.0, 1.0)),
                            move |element, delta| element.top(px(screen.y as f32 + delta * 3.0)),
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
