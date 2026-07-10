//! Comment interaction UI on the canvas: the draft composer that opens when
//! the comment tool places a pin, the hover preview card, and the open-thread
//! popover with replies — the original fanta's polished flow, adapted to GPUI.
//!
//! Pins themselves paint inside [`CanvasElement`](crate::canvas); everything
//! here is GPUI element trees pushed as siblings ABOVE the canvas surface (the
//! same z-order trick as the text-edit overlay).

use std::collections::HashMap;

use editor::Editor;
use fanta_doc::NodeId;
use glam::DVec2;
use gpui::{AnyElement, App, Entity, Focusable as _, SharedString};
use ui::prelude::*;
use ui::{IconButton, IconButtonShape, Tooltip};

use crate::comments::{self, Comment};
use crate::tools::ToolKind;
use crate::canvas::{bounds_size, screen_position_in_bounds};
use crate::view::FigView;

/// Screen-fixed pin size; must match `canvas::COMMENT_PIN_SIZE`.
pub(crate) const PIN_SIZE: f64 = 30.0;

/// A comment being composed: the pin is placed but nothing is in the doc yet.
pub(crate) struct CommentDraft {
    pub(crate) world: DVec2,
    pub(crate) editor: Entity<Editor>,
}

/// Session-local comment view state on [`FigView`].
#[derive(Default)]
pub(crate) struct CommentState {
    pub(crate) draft: Option<CommentDraft>,
    pub(crate) open_thread: Option<String>,
    pub(crate) hovered_pin: Option<String>,
    pub(crate) reply_editor: Option<Entity<Editor>>,
    /// Newest `created` seen per thread — session-only read state, never
    /// persisted (viewing must not dirty the doc or pollute undo).
    pub(crate) read_until: HashMap<String, u64>,
}

impl FigView {
    /// The active page's comments, when a document is loaded.
    fn page_comments(&self, cx: &App) -> Vec<Comment> {
        self.item
            .read(cx)
            .document()
            .and_then(|document| {
                let page = document.doc.active_page()?;
                Some(comments::read_comments(&document.doc, page))
            })
            .unwrap_or_default()
    }

    fn active_page(&self, cx: &App) -> Option<NodeId> {
        self.item
            .read(cx)
            .document()
            .and_then(|document| document.doc.active_page())
    }

    /// The pin's screen rect for `world`, or `None` without a viewport. The
    /// anchor is the teardrop's squared bottom-left tail.
    fn pin_screen_rect(&self, world: DVec2) -> Option<(DVec2, DVec2)> {
        let bounds = self.container_bounds?;
        let viewport = self.viewport?;
        let (width, height) = bounds_size(bounds);
        let anchor =
            fanta_canvas::world_to_screen(world, &viewport, DVec2::new(width, height));
        let min = DVec2::new(anchor.x, anchor.y - PIN_SIZE);
        Some((min, min + DVec2::new(PIN_SIZE, PIN_SIZE)))
    }

    /// The comment whose pin contains `screen`, topmost (last painted) first.
    pub(crate) fn comment_pin_at(&self, screen: DVec2, cx: &App) -> Option<String> {
        let comments = self.page_comments(cx);
        comments
            .iter()
            .rev()
            .find(|comment| {
                self.pin_screen_rect(DVec2::new(comment.world[0], comment.world[1]))
                    .is_some_and(|(min, max)| {
                        screen.x >= min.x
                            && screen.x <= max.x
                            && screen.y >= min.y
                            && screen.y <= max.y
                    })
            })
            .map(|comment| comment.id.clone())
    }

    /// A left press routed to comments. Returns true when consumed: a pin
    /// click toggles its thread; in comment mode an empty-canvas click opens
    /// the draft composer (nothing hits the doc until Send).
    pub(crate) fn handle_comment_mouse_down(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(bounds) = self.container_bounds else {
            return false;
        };
        let screen = screen_position_in_bounds(position, bounds);
        // Pin clicks work with ANY tool, like the original.
        if let Some(id) = self.comment_pin_at(screen, cx) {
            self.toggle_comment_thread(id, window, cx);
            return true;
        }
        if self.tools.kind() != ToolKind::Comment || !self.is_editable(cx) {
            return false;
        }
        if self.comment_state.draft.is_some() {
            // A draft is open: clicks on the canvas are inert (Send or Escape
            // decide its fate); they must not re-place the pin.
            return true;
        }
        let Some(viewport) = self.viewport else {
            return false;
        };
        let (width, height) = bounds_size(bounds);
        let world =
            fanta_canvas::screen_to_world(screen, &viewport, DVec2::new(width, height));
        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(1, 6, window, cx);
            editor.set_placeholder_text("Add a comment…", window, cx);
            editor
        });
        editor.read(cx).focus_handle(cx).focus(window, cx);
        self.comment_state.draft = Some(CommentDraft { world, editor });
        cx.notify();
        true
    }

    /// Track which pin the pointer is over, for hover grow + preview card.
    pub(crate) fn handle_comment_mouse_move(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let screen = screen_position_in_bounds(position, bounds);
        let hovered = self.comment_pin_at(screen, cx);
        if hovered != self.comment_state.hovered_pin {
            self.comment_state.hovered_pin = hovered;
            cx.notify();
        }
    }

    pub(crate) fn toggle_comment_thread(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comment_state.open_thread.as_deref() == Some(id.as_str()) {
            self.comment_state.open_thread = None;
            self.comment_state.reply_editor = None;
        } else {
            // Opening marks the thread read.
            if let Some(comment) = self
                .page_comments(cx)
                .into_iter()
                .find(|comment| comment.id == id)
            {
                self.comment_state
                    .read_until
                    .insert(id.clone(), comment.newest_created());
            }
            self.comment_state.open_thread = Some(id);
            let editor = cx.new(|cx| {
                let mut editor = Editor::auto_height(1, 4, window, cx);
                editor.set_placeholder_text("Reply…", window, cx);
                editor
            });
            self.comment_state.reply_editor = Some(editor);
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    /// Post the draft: build the add op, open the new thread, return to Select
    /// (one pin per arming, like the original).
    pub(crate) fn post_comment_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.comment_state.draft.take() else {
            return;
        };
        let text = draft.editor.read(cx).text(cx);
        let Some(page) = self.active_page(cx) else {
            cx.notify();
            return;
        };
        let world = [draft.world.x, draft.world.y];
        let posted = self.item.update(cx, |item, cx| {
            let operation = item
                .document()
                .and_then(|document| comments::add_comment_op(&document.doc, page, world, &text));
            operation.map(|(id, operation)| {
                if let Err(error) = item.apply(operation, cx) {
                    log::error!("posting a canvas comment failed: {error:#}");
                }
                id
            })
        });
        if let Some(id) = posted {
            self.comment_state.read_until.insert(
                id.clone(),
                comments::now_secs(),
            );
            self.toggle_comment_thread(id, window, cx);
        }
        self.activate_tool(ToolKind::Select, cx);
        self.invalidate_canvas_cache();
        cx.notify();
    }

    pub(crate) fn cancel_comment_draft(&mut self, cx: &mut Context<Self>) -> bool {
        if self.comment_state.draft.take().is_some() {
            // Fully exit comment mode so the ghost pin doesn't re-arm.
            self.activate_tool(ToolKind::Select, cx);
            cx.notify();
            return true;
        }
        false
    }

    fn post_comment_reply(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.comment_state.reply_editor.clone() else {
            return;
        };
        let body = editor.read(cx).text(cx);
        let Some(page) = self.active_page(cx) else {
            return;
        };
        let applied = self.item.update(cx, |item, cx| {
            let operation = item
                .document()
                .and_then(|document| comments::reply_comment_op(&document.doc, page, &id, &body));
            match operation {
                Some(operation) => {
                    if let Err(error) = item.apply(operation, cx) {
                        log::error!("replying to a canvas comment failed: {error:#}");
                        false
                    } else {
                        true
                    }
                }
                None => false,
            }
        });
        if applied {
            editor.update(cx, |editor, cx| editor.clear(window, cx));
        }
        if let Some(comment) = self
            .page_comments(cx)
            .into_iter()
            .find(|comment| comment.id == id)
        {
            self.comment_state
                .read_until
                .insert(id, comment.newest_created());
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    fn resolve_comment_thread(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(page) = self.active_page(cx) else {
            return;
        };
        self.item.update(cx, |item, cx| {
            let operation = item
                .document()
                .and_then(|document| comments::toggle_resolved_op(&document.doc, page, &id));
            if let Some(operation) = operation {
                if let Err(error) = item.apply(operation, cx) {
                    log::error!("resolving a canvas comment failed: {error:#}");
                }
            }
        });
        self.invalidate_canvas_cache();
        cx.notify();
    }

    fn delete_comment_thread(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(page) = self.active_page(cx) else {
            return;
        };
        self.item.update(cx, |item, cx| {
            let operation = item
                .document()
                .and_then(|document| comments::remove_comment_op(&document.doc, page, &id));
            if let Some(operation) = operation {
                if let Err(error) = item.apply(operation, cx) {
                    log::error!("deleting a canvas comment failed: {error:#}");
                }
            }
        });
        self.comment_state.open_thread = None;
        self.comment_state.reply_editor = None;
        self.invalidate_canvas_cache();
        cx.notify();
    }

    /// The floating comment UI: the draft composer, or the open thread
    /// popover, or the hover preview card. `None` when nothing is showing.
    pub(crate) fn render_comment_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(draft) = &self.comment_state.draft {
            return self.render_comment_composer(draft, cx);
        }
        if let Some(open) = self.comment_state.open_thread.clone() {
            let comment = self
                .page_comments(cx)
                .into_iter()
                .find(|comment| comment.id == open)?;
            return self.render_comment_thread(&comment, cx);
        }
        if let Some(hovered) = self.comment_state.hovered_pin.clone() {
            let comment = self
                .page_comments(cx)
                .into_iter()
                .find(|comment| comment.id == hovered)?;
            return self.render_comment_preview(&comment);
        }
        None
    }

    fn render_comment_composer(
        &self,
        draft: &CommentDraft,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (pin_min, _) = self.pin_screen_rect(draft.world)?;
        let editor = draft.editor.clone();
        Some(
            v_flex()
                .absolute()
                .left(px((pin_min.x + PIN_SIZE + 12.0) as f32))
                .top(px((pin_min.y - 4.0) as f32))
                .w(px(300.))
                .p_2p5()
                .gap_2()
                .rounded_xl()
                .bg(cx.theme().colors().elevated_surface_background)
                .border_1()
                .border_color(cx.theme().colors().border)
                .shadow_lg()
                .child(div().child(editor))
                .child(
                    h_flex().justify_end().child(
                        IconButton::new("fanta-comment-send", IconName::ArrowUp)
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Post Comment"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.post_comment_draft(window, cx);
                            })),
                    ),
                )
                .into_any_element(),
        )
    }

    fn render_comment_preview(&self, comment: &Comment) -> Option<AnyElement> {
        let world = DVec2::new(comment.world[0], comment.world[1]);
        let (pin_min, _) = self.pin_screen_rect(world)?;
        let author: SharedString = display_author(comment).into();
        let when: SharedString = relative_time(comment.created).into();
        let body: SharedString = comment.text.clone().into();
        Some(
            v_flex()
                .absolute()
                .left(px((pin_min.x + PIN_SIZE * 0.6 + 18.0) as f32))
                .top(px((pin_min.y - 30.0) as f32))
                .max_w(px(300.))
                .p_3()
                .gap_1()
                .rounded_xl()
                .bg(gpui::opaque_grey(0.12, 1.0))
                .border_1()
                .border_color(gpui::opaque_grey(0.3, 1.0))
                .shadow_lg()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Label::new(author).size(LabelSize::Small).weight(gpui::FontWeight::BOLD))
                        .child(Label::new(when).size(LabelSize::XSmall).color(Color::Muted)),
                )
                .child(Label::new(body).size(LabelSize::Small))
                .into_any_element(),
        )
    }

    fn render_comment_thread(
        &self,
        comment: &Comment,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let world = DVec2::new(comment.world[0], comment.world[1]);
        let (pin_min, _) = self.pin_screen_rect(world)?;
        let resolve_id = comment.id.clone();
        let delete_id = comment.id.clone();
        let reply_id = comment.id.clone();
        let close_id = comment.id.clone();

        let mut messages = v_flex()
            .id("fanta-comment-messages")
            .px_3p5()
            .py_3()
            .gap_3()
            .max_h(px(400.))
            .overflow_y_scroll();
        messages = messages.child(Self::comment_message_row(
            display_author(comment),
            relative_time(comment.created),
            comment.text.clone(),
        ));
        for reply in &comment.replies {
            messages = messages.child(Self::comment_message_row(
                reply.author.clone(),
                relative_time(reply.created),
                reply.body.clone(),
            ));
        }

        let reply_editor = self.comment_state.reply_editor.clone();
        Some(
            v_flex()
                .absolute()
                .left(px((pin_min.x + PIN_SIZE + 12.0) as f32))
                .top(px((pin_min.y - 8.0) as f32))
                .w(px(300.))
                .rounded_xl()
                .bg(cx.theme().colors().elevated_surface_background)
                .border_1()
                .border_color(cx.theme().colors().border)
                .shadow_lg()
                .child(
                    h_flex()
                        .px_3p5()
                        .py_2()
                        .justify_between()
                        .items_center()
                        .border_b_1()
                        .border_color(cx.theme().colors().border)
                        .child(
                            Label::new("Comment")
                                .size(LabelSize::Small)
                                .weight(gpui::FontWeight::BOLD),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    IconButton::new(
                                        "fanta-comment-resolve-thread",
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
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.resolve_comment_thread(resolve_id.clone(), cx);
                                    })),
                                )
                                .child(
                                    IconButton::new("fanta-comment-delete-thread", IconName::Trash)
                                        .icon_size(IconSize::XSmall)
                                        .tooltip(Tooltip::text("Delete Thread"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.delete_comment_thread(delete_id.clone(), cx);
                                        })),
                                )
                                .child(
                                    IconButton::new("fanta-comment-close-thread", IconName::Close)
                                        .icon_size(IconSize::XSmall)
                                        .tooltip(Tooltip::text("Close"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.toggle_comment_thread(
                                                close_id.clone(),
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                        ),
                )
                .child(messages)
                .when_some(reply_editor, |this, editor| {
                    this.child(
                        h_flex()
                            .px_3p5()
                            .py_2p5()
                            .gap_2()
                            .items_end()
                            .border_t_1()
                            .border_color(cx.theme().colors().border)
                            .child(div().flex_1().child(editor))
                            .child(
                                IconButton::new("fanta-comment-reply-send", IconName::ArrowUp)
                                    .shape(IconButtonShape::Square)
                                    .icon_size(IconSize::Small)
                                    .tooltip(Tooltip::text("Reply"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.post_comment_reply(reply_id.clone(), window, cx);
                                    })),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    fn comment_message_row(author: String, when: String, body: String) -> gpui::Div {
        v_flex()
            .gap_0p5()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Label::new(SharedString::from(author))
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::BOLD),
                    )
                    .child(
                        Label::new(SharedString::from(when))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(Label::new(SharedString::from(body)).size(LabelSize::Small))
    }
}

fn display_author(comment: &Comment) -> String {
    if comment.author.trim().is_empty() {
        "you".to_string()
    } else {
        comment.author.clone()
    }
}

/// "just now" → minutes → hours → days → weeks, like the original.
pub(crate) fn relative_time(created: u64) -> String {
    let now = comments::now_secs();
    let delta = now.saturating_sub(created);
    if delta < 45 {
        return "just now".to_string();
    }
    let minutes = (delta as f64 / 60.0).round() as u64;
    if minutes < 90 {
        return format!("{minutes} minute{} ago", if minutes == 1 { "" } else { "s" });
    }
    let hours = (delta as f64 / 3600.0).round() as u64;
    if hours < 36 {
        return format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" });
    }
    let days = (delta as f64 / 86_400.0).round() as u64;
    if days < 11 {
        return format!("{days} day{} ago", if days == 1 { "" } else { "s" });
    }
    let weeks = (delta as f64 / 604_800.0).round() as u64;
    format!("{weeks} week{} ago", if weeks == 1 { "" } else { "s" })
}
