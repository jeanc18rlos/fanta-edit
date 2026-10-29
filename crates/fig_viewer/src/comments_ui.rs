//! Comment interaction UI on the canvas: the draft composer that opens when
//! the comment tool places a pin, the hover preview card, and the open-thread
//! popover with replies — the original fanta's polished flow, adapted to GPUI.
//!
//! Pins themselves paint inside [`CanvasElement`](crate::canvas); everything
//! here is GPUI element trees pushed as siblings ABOVE the canvas surface (the
//! same z-order trick as the text-edit overlay).

use std::collections::HashMap;

use editor::Editor;
use fanta_doc::{DocId, NodeId};
use glam::DVec2;
use gpui::{
    AnyElement, App, Bounds, Entity, Focusable as _, Hsla, ObjectFit, Pixels, Point, ScrollHandle,
    SharedString, Size, Stateful, canvas, deferred, img, point,
};
use ui::prelude::*;
use ui::{Callout, IconButton, ListItem, ListItemSpacing, Severity, Tooltip, rems_from_px};
use workspace::MultiWorkspace;

use crate::canvas::{bounds_size, screen_position_in_bounds};
use crate::comments::{self, Attachment, Comment, CommentSkill, Mention, MotionCommentAnchor};
use crate::editor_session::EditorMode;
use crate::tools::ToolKind;
use crate::view::{FigView, SubmitComment};

/// Screen-fixed pin size; must match `canvas::COMMENT_PIN_SIZE`.
pub(crate) const PIN_SIZE: f64 = 30.0;

fn comment_overlay_origin(
    limits: Bounds<Pixels>,
    pin: Bounds<Pixels>,
    card_size: Size<Pixels>,
    offset: Point<Pixels>,
) -> Point<Pixels> {
    let preferred_left = pin.right() + offset.x;
    let flipped_left = pin.left() - offset.x - card_size.width;
    let left = if preferred_left + card_size.width > limits.right() && flipped_left >= limits.left()
    {
        flipped_left
    } else {
        preferred_left
    };
    point(
        left.clamp(
            limits.left(),
            (limits.right() - card_size.width).max(limits.left()),
        ),
        (pin.top() + offset.y).clamp(
            limits.top(),
            (limits.bottom() - card_size.height).max(limits.top()),
        ),
    )
}

fn comment_overlay(
    world: DVec2,
    viewport: fanta_doc::Viewport,
    offset: Point<Pixels>,
    card: Stateful<gpui::Div>,
) -> AnyElement {
    // Workspace tabs paint after the canvas's parent, so the card must be deferred
    // to remain visible and interactive above both the tabs and canvas toolbar.
    deferred(
        canvas(
            move |bounds, window, cx| {
                let inset = px(8.)
                    .min(bounds.size.width / 2.)
                    .min(bounds.size.height / 2.);
                let limits = bounds.inset(inset);
                let mut card = card
                    .max_w(limits.size.width)
                    .max_h(limits.size.height)
                    .overflow_y_scroll()
                    .occlude()
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .into_any_element();
                // Measure after the canvas lays out: sidebar and window resizing can change
                // these bounds without another render, and editor/picker heights vary.
                let card_size = card.layout_as_root(limits.size.into(), window, cx);
                let anchor = fanta_canvas::world_to_screen(
                    world,
                    &viewport,
                    DVec2::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
                );
                let pin = Bounds::new(
                    bounds.origin + point(px(anchor.x as f32), px((anchor.y - PIN_SIZE) as f32)),
                    gpui::size(px(PIN_SIZE as f32), px(PIN_SIZE as f32)),
                );
                let origin = comment_overlay_origin(limits, pin, card_size, offset);
                card.prepaint_at(origin, window, cx);
                card
            },
            |_, mut card, window, cx| card.paint(window, cx),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
    .into_any_element()
}

/// A comment being composed: the pin is placed but nothing is in the doc yet.
pub(crate) struct CommentDraft {
    pub(crate) world: DVec2,
    pub(crate) editor: Entity<Editor>,
    pub(crate) motion_anchor: Option<MotionCommentAnchor>,
    composer_id: String,
    origin: CommentOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CommentOrigin {
    document: DocId,
    page: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingMotionComment {
    origin: CommentOrigin,
    anchor: MotionCommentAnchor,
}

fn document_has_comment_origin(document: &fanta_doc::Doc, origin: CommentOrigin) -> bool {
    document.id == origin.document
        && document.pages.contains(&origin.page)
        && document.scene.get(origin.page).is_some()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CommentComposerTarget {
    Draft,
    Reply,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SkillRouteNotice {
    ReadyForReview,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AttachmentPickerOrigin {
    Draft {
        origin: CommentOrigin,
        composer_id: String,
    },
    Reply {
        origin: CommentOrigin,
        thread_id: String,
        composer_id: String,
    },
}

impl AttachmentPickerOrigin {
    fn target(&self) -> CommentComposerTarget {
        match self {
            Self::Draft { .. } => CommentComposerTarget::Draft,
            Self::Reply { .. } => CommentComposerTarget::Reply,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CommentSubmitOutcome {
    Empty,
    Applied(String),
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
enum CommentMutationOutcome {
    Applied,
    Failed(String),
}

impl CommentSubmitOutcome {
    fn applied_id(&self) -> Option<&str> {
        match self {
            Self::Applied(id) => Some(id),
            Self::Empty | Self::Failed(_) => None,
        }
    }
}

/// Session-local comment view state on [`FigView`].
#[derive(Default)]
pub(crate) struct CommentState {
    pub(crate) draft: Option<CommentDraft>,
    pending_motion_anchor: Option<PendingMotionComment>,
    pub(crate) open_thread: Option<String>,
    open_thread_origin: Option<CommentOrigin>,
    /// The open thread was opened from the Comments panel, which then shows
    /// it as a page instead of a popover on the canvas.
    thread_in_panel: bool,
    thread_scroll: ScrollHandle,
    pub(crate) hovered_pin: Option<String>,
    pub(crate) reply_editor: Option<Entity<Editor>>,
    reply_composer_id: Option<String>,
    draft_attachments: Vec<Attachment>,
    reply_attachments: Vec<Attachment>,
    draft_error: Option<String>,
    reply_error: Option<String>,
    mention_picker: Option<CommentComposerTarget>,
    skill_picker: Option<CommentComposerTarget>,
    draft_skill: Option<CommentSkill>,
    reply_skill: Option<CommentSkill>,
    skill_route_notice: Option<SkillRouteNotice>,
    /// Newest `created` seen per thread — session-only read state, never
    /// persisted (viewing must not dirty the doc or pollute undo).
    pub(crate) read_until: HashMap<String, u64>,
}

impl CommentState {
    fn arm_motion_anchor(&mut self, origin: CommentOrigin, anchor: MotionCommentAnchor) {
        self.pending_motion_anchor = Some(PendingMotionComment { origin, anchor });
    }

    pub(crate) fn clear_pending_motion_anchor(&mut self) {
        self.pending_motion_anchor = None;
    }

    pub(crate) fn motion_time_comment_active(&self) -> bool {
        self.pending_motion_anchor.is_some()
            || self
                .draft
                .as_ref()
                .is_some_and(|draft| draft.motion_anchor.is_some())
    }

    pub(crate) fn pending_motion_anchor(&self) -> Option<MotionCommentAnchor> {
        self.pending_motion_anchor.map(|pending| pending.anchor)
    }

    pub(crate) fn close_thread(&mut self) {
        self.open_thread = None;
        self.open_thread_origin = None;
        self.thread_in_panel = false;
        self.reply_editor = None;
        self.reply_composer_id = None;
        self.reply_attachments.clear();
        self.reply_error = None;
        self.mention_picker = None;
        self.skill_picker = None;
        self.reply_skill = None;
        self.skill_route_notice = None;
    }
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

    fn visible_page_comments(&self, cx: &App) -> Vec<Comment> {
        let active_motion_clip = self.active_motion_comment_clip(cx);
        self.page_comments(cx)
            .into_iter()
            .filter(|comment| comments::comment_is_visible(comment, active_motion_clip))
            .collect()
    }

    fn motion_comment_label(&self, anchor: MotionCommentAnchor, cx: &App) -> String {
        self.item
            .read(cx)
            .document()
            .map(|document| comments::motion_comment_label(&document.doc, anchor))
            .unwrap_or_else(|| {
                format!(
                    "Animation unavailable · {}",
                    comments::motion_comment_time_label(anchor.time_ms)
                )
            })
    }

    fn active_comment_origin(&self, cx: &App) -> Option<CommentOrigin> {
        self.item.read(cx).document().and_then(|document| {
            let origin = CommentOrigin {
                document: document.doc.id,
                page: document.doc.active_page()?,
            };
            document_has_comment_origin(&document.doc, origin).then_some(origin)
        })
    }

    pub(crate) fn arm_motion_comment_anchor(
        &mut self,
        anchor: MotionCommentAnchor,
        cx: &App,
    ) -> bool {
        let Some(origin) = self.active_comment_origin(cx) else {
            return false;
        };
        self.comment_state.arm_motion_anchor(origin, anchor);
        true
    }

    /// The open thread's page and id, which the Comments panel highlights.
    pub(crate) fn open_comment_thread(&self) -> Option<(NodeId, String)> {
        Some((
            self.comment_state.open_thread_origin?.page,
            self.comment_state.open_thread.clone()?,
        ))
    }

    pub(crate) fn has_unsent_comment_reply(&self, cx: &App) -> bool {
        self.comment_state
            .reply_editor
            .as_ref()
            .is_some_and(|editor| !editor.read(cx).text(cx).trim().is_empty())
            || !self.comment_state.reply_attachments.is_empty()
            || self.comment_state.reply_skill.is_some()
    }

    pub(crate) fn close_comment_thread_if_matches(
        &mut self,
        page: NodeId,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.comment_state.open_thread.as_deref() == Some(id)
            && self
                .comment_state
                .open_thread_origin
                .is_some_and(|origin| origin.page == page)
        {
            self.comment_state.close_thread();
            self.invalidate_canvas_cache();
            cx.notify();
        }
    }

    fn comments_for_origin(&self, origin: CommentOrigin, cx: &App) -> Vec<Comment> {
        self.item
            .read(cx)
            .document()
            .filter(|document| document_has_comment_origin(&document.doc, origin))
            .map(|document| comments::read_comments(&document.doc, origin.page))
            .unwrap_or_default()
    }

    fn origin_is_active(&self, origin: CommentOrigin, cx: &App) -> bool {
        self.active_comment_origin(cx) == Some(origin)
    }

    fn attachment_picker_origin(
        &self,
        target: CommentComposerTarget,
    ) -> Option<AttachmentPickerOrigin> {
        match target {
            CommentComposerTarget::Draft => {
                let draft = self.comment_state.draft.as_ref()?;
                Some(AttachmentPickerOrigin::Draft {
                    origin: draft.origin,
                    composer_id: draft.composer_id.clone(),
                })
            }
            CommentComposerTarget::Reply => Some(AttachmentPickerOrigin::Reply {
                origin: self.comment_state.open_thread_origin?,
                thread_id: self.comment_state.open_thread.clone()?,
                composer_id: self.comment_state.reply_composer_id.clone()?,
            }),
        }
    }

    /// The pin's screen rect for `world`, or `None` without a viewport. The
    /// anchor is the teardrop's squared bottom-left tail.
    fn pin_screen_rect(&self, world: DVec2) -> Option<(DVec2, DVec2)> {
        let bounds = self.container_bounds?;
        let viewport = self.viewport?;
        let (width, height) = bounds_size(bounds);
        let anchor = fanta_canvas::world_to_screen(world, &viewport, DVec2::new(width, height));
        let min = DVec2::new(anchor.x, anchor.y - PIN_SIZE);
        Some((min, min + DVec2::new(PIN_SIZE, PIN_SIZE)))
    }

    /// Where a comment's pin is now: its layer's current placement when it is
    /// anchored, otherwise the authored position.
    fn comment_world(&self, comment: &Comment, cx: &App) -> DVec2 {
        let [x, y] = self
            .item
            .read(cx)
            .document()
            .map(|document| comments::resolve_world(&document.doc, comment))
            .unwrap_or(comment.world);
        DVec2::new(x, y)
    }

    /// The comment whose pin contains `screen`, topmost (last painted) first.
    pub(crate) fn comment_pin_at(&self, screen: DVec2, cx: &App) -> Option<String> {
        let comments = self.visible_page_comments(cx);
        comments
            .iter()
            .rev()
            .find(|comment| {
                self.pin_screen_rect(self.comment_world(comment, cx))
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
        if self.comment_state.draft.is_some() {
            // A draft is open: clicks on the canvas are inert (Send or Escape
            // decide its fate); they must not re-place the pin or open a thread.
            return true;
        }
        // Pin clicks work with ANY tool, like the original.
        if let Some(id) = self.comment_pin_at(screen, cx) {
            self.toggle_comment_thread(id, window, cx);
            return true;
        }
        if self.tools.kind() != ToolKind::Comment || !self.is_editable(cx) {
            return false;
        }
        if self.has_unsent_comment_reply(cx) {
            crate::view::show_canvas_notice(
                "Send or clear the current reply before placing another comment.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return true;
        }
        let Some(viewport) = self.viewport else {
            return false;
        };
        let Some(origin) = self.active_comment_origin(cx) else {
            return false;
        };
        let motion_anchor = match self.comment_state.pending_motion_anchor.take() {
            Some(pending) if pending.origin == origin => Some(pending.anchor),
            Some(_) => {
                self.activate_tool(ToolKind::Select, cx);
                crate::view::show_canvas_notice(
                    "The page changed before the time comment was placed. Choose the animation and try again."
                        .to_string(),
                    window,
                    cx,
                );
                cx.notify();
                return true;
            }
            None => None,
        };
        let (width, height) = bounds_size(bounds);
        let world = fanta_canvas::screen_to_world(screen, &viewport, DVec2::new(width, height));
        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(1, 6, window, cx);
            editor.set_placeholder_text("Add a comment…", window, cx);
            editor
        });
        editor.read(cx).focus_handle(cx).focus(window, cx);
        // The press keeps bubbling to FigView's tracked-focus element, which
        // would take focus back to the canvas: typing would then run canvas
        // shortcuts, and Backspace would delete the selected layers.
        window.prevent_default();
        self.comment_state.draft = Some(CommentDraft {
            world,
            editor,
            motion_anchor,
            composer_id: NodeId::new().to_string(),
            origin,
        });
        self.comment_state.draft_attachments.clear();
        self.comment_state.draft_error = None;
        self.comment_state.mention_picker = None;
        self.comment_state.skill_picker = None;
        self.comment_state.draft_skill = None;
        self.comment_state.skill_route_notice = None;
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
        if self.comment_state.draft.is_some() {
            crate::view::show_canvas_notice(
                "Send or cancel the current comment draft before opening a thread.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return;
        }
        if self.has_unsent_comment_reply(cx) {
            crate::view::show_canvas_notice(
                "Send or clear the current reply before switching comment threads.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return;
        }
        let Some(origin) = self.active_comment_origin(cx) else {
            return;
        };
        self.comment_state.clear_pending_motion_anchor();
        if self.comment_state.open_thread.as_deref() == Some(id.as_str())
            && self.comment_state.open_thread_origin == Some(origin)
        {
            self.comment_state.close_thread();
        } else {
            self.open_comment_thread_for_origin(id, origin, window, cx);
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    /// Open a thread from a panel row. Unlike a pin click this is idempotent:
    /// re-opening an already selected timed thread navigates back to its clip
    /// without discarding an unsent reply or its attachments.
    pub(crate) fn show_comment_thread(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comment_state.draft.is_some() {
            crate::view::show_canvas_notice(
                "Send or cancel the current comment draft before opening a thread.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return;
        }
        let Some(origin) = self.active_comment_origin(cx) else {
            return;
        };
        let already_open = self.comment_state.open_thread.as_deref() == Some(id.as_str())
            && self.comment_state.open_thread_origin == Some(origin);
        if !already_open && self.has_unsent_comment_reply(cx) {
            crate::view::show_canvas_notice(
                "Send or clear the current reply before switching comment threads.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return;
        }
        self.comment_state.clear_pending_motion_anchor();
        if already_open {
            if let Some(comment) = self
                .comments_for_origin(origin, cx)
                .into_iter()
                .find(|comment| comment.id == id)
            {
                self.comment_state
                    .read_until
                    .insert(id, comment.newest_created());
                if let Some(anchor) = comment.motion_anchor {
                    self.navigate_to_motion_comment(anchor, window, cx);
                }
            }
        } else {
            self.open_comment_thread_for_origin(id, origin, window, cx);
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    pub(crate) fn show_comment_thread_on_page(
        &mut self,
        page: NodeId,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page_index = self.item.read(cx).document().and_then(|document| {
            document
                .pages
                .iter()
                .position(|candidate| candidate.root == Some(page))
        });
        let Some(page_index) = page_index else {
            crate::view::show_canvas_notice(
                "This comment's page is no longer available.".to_string(),
                window,
                cx,
            );
            return;
        };
        if self.comment_state.draft.is_some() {
            crate::view::show_canvas_notice(
                "Send or cancel the current comment draft before opening a thread.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return;
        }
        let already_open = self.comment_state.open_thread.as_deref() == Some(id.as_str())
            && self
                .comment_state
                .open_thread_origin
                .is_some_and(|origin| origin.page == page);
        if !already_open && self.has_unsent_comment_reply(cx) {
            crate::view::show_canvas_notice(
                "Send or clear the current reply before switching comment threads.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return;
        }
        self.select_page(page_index, cx);
        self.show_comment_thread(id, window, cx);
    }

    /// Open a thread from a Comments panel row as the panel's thread page.
    pub(crate) fn show_comment_thread_in_panel(
        &mut self,
        page: NodeId,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_comment_thread_on_page(page, id.clone(), window, cx);
        if self.open_comment_thread() == Some((page, id)) {
            self.comment_state.thread_in_panel = true;
            cx.notify();
        }
    }

    /// Back from the panel's thread page to the list of threads.
    fn close_comment_thread_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_unsent_comment_reply(cx) {
            crate::view::show_canvas_notice(
                "Send or clear the current reply before going back to all threads.".to_string(),
                window,
                cx,
            );
            cx.notify();
            return;
        }
        self.comment_state.close_thread();
        self.invalidate_canvas_cache();
        cx.notify();
    }

    /// Whether the open thread shows as a page in the Comments panel. It
    /// falls back to the canvas popover whenever that page is not on screen.
    pub(crate) fn comment_thread_in_panel(&self, cx: &App) -> bool {
        self.comment_state.thread_in_panel
            && self.editor_mode(cx) == EditorMode::Comments
            && self.inspector_sidebar_expanded(cx)
            && !self.annotation_panel_active(cx)
    }

    fn open_comment_thread_for_origin(
        &mut self,
        id: String,
        origin: CommentOrigin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(comment) = self
            .comments_for_origin(origin, cx)
            .into_iter()
            .find(|comment| comment.id == id)
        {
            self.comment_state
                .read_until
                .insert(id.clone(), comment.newest_created());
            if let Some(anchor) = comment.motion_anchor
                && !self.is_dev_mode(cx)
            {
                self.navigate_to_motion_comment(anchor, window, cx);
            }
        }
        self.comment_state.open_thread = Some(id);
        self.comment_state.open_thread_origin = Some(origin);
        self.comment_state.thread_in_panel = false;
        self.comment_state.thread_scroll = ScrollHandle::new();
        self.comment_state.reply_editor = (!self.is_dev_mode(cx)).then(|| {
            cx.new(|cx| {
                let mut editor = Editor::auto_height(1, 4, window, cx);
                editor.set_placeholder_text("Reply…", window, cx);
                editor
            })
        });
        self.comment_state.reply_composer_id = Some(NodeId::new().to_string());
        self.comment_state.reply_attachments.clear();
        self.comment_state.reply_error = None;
        self.comment_state.mention_picker = None;
        self.comment_state.skill_picker = None;
        self.comment_state.reply_skill = None;
        self.comment_state.skill_route_notice = None;
    }

    /// Post the draft: build the add op, open the new thread, return to Select
    /// (one pin per arming, like the original).
    pub(crate) fn post_comment_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) {
            self.comment_state.draft_error =
                Some("Switch to Design to post comments. Your draft was kept.".to_owned());
            cx.notify();
            return;
        }
        self.finish_document_edits_for_external_change(cx);
        let Some(draft) = self.comment_state.draft.as_ref() else {
            return;
        };
        let text = draft.editor.read(cx).text(cx);
        let origin = draft.origin;
        let page = origin.page;
        let world = [draft.world.x, draft.world.y];
        let attachments = self.comment_state.draft_attachments.clone();
        let skill = self.comment_state.draft_skill;
        let motion_anchor = draft.motion_anchor;
        let has_payload = !text.trim().is_empty() || !attachments.is_empty() || skill.is_some();
        self.comment_state.draft_error = None;
        let preview_owner = cx.entity_id();
        let outcome = self.item.update(cx, |item, cx| {
            if !item.can_preview_for_owner(preview_owner) || item.content_preview_active() {
                return CommentSubmitOutcome::Failed(
                    "Could not post comment while Save As, a source edit, or another canvas preview is active. Finish or cancel that edit and retry; your draft was kept."
                        .to_string(),
                );
            }
            if motion_anchor.is_some_and(|anchor| {
                item.document()
                    .is_none_or(|document| document.doc.motion.clip(anchor.clip).is_none())
            }) {
                return CommentSubmitOutcome::Failed(
                    "Could not post time comment because its animation no longer exists. Choose a current animation and place the comment again; your draft was kept."
                        .to_string(),
                );
            }
            let operation = item.document().and_then(|document| {
                if !document_has_comment_origin(&document.doc, origin) {
                    return None;
                }
                let mentions = comments::mentions_for_body(&document.doc, page, &text);
                comments::add_comment_full_op(
                    &document.doc,
                    page,
                    world,
                    &text,
                    mentions,
                    attachments,
                    skill,
                    motion_anchor,
                )
            });
            let Some((id, operation)) = operation else {
                return if has_payload {
                    CommentSubmitOutcome::Failed(
                        "Could not post comment: its originating document or page is no longer available. Return to that page or copy the draft before retrying."
                            .to_string(),
                    )
                } else {
                    CommentSubmitOutcome::Empty
                };
            };
            match item.apply_for_preview_owner(preview_owner, operation, cx) {
                Ok(()) => CommentSubmitOutcome::Applied(id),
                Err(error) => {
                    CommentSubmitOutcome::Failed(format!("Could not post comment: {error:#}"))
                }
            }
        });
        if let Some(id) = outcome.applied_id().map(str::to_string) {
            let skill_prompt = skill.and_then(|skill| {
                self.item
                    .read(cx)
                    .document()
                    .filter(|document| document.doc.id == origin.document)
                    .and_then(|document| {
                        comments::skill_prompt_for_comment(&document.doc, page, &id, skill)
                    })
            });
            self.comment_state.draft = None;
            self.comment_state.draft_attachments.clear();
            self.comment_state.draft_error = None;
            self.comment_state.mention_picker = None;
            self.comment_state.skill_picker = None;
            self.comment_state.draft_skill = None;
            self.comment_state.clear_pending_motion_anchor();
            self.comment_state
                .read_until
                .insert(id.clone(), comments::now_secs());
            self.open_comment_thread_for_origin(id, origin, window, cx);
            // The draft editor is gone: hand the keyboard back to the canvas.
            self.focus_handle(cx).focus(window, cx);
            if skill.is_some() {
                self.route_comment_skill_prompt(skill_prompt, window, cx);
            }
            self.activate_tool(ToolKind::Select, cx);
        } else if let CommentSubmitOutcome::Failed(error) = outcome {
            log::error!("posting a canvas comment failed: {error}");
            self.comment_state.draft_error = Some(error);
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    pub(crate) fn cancel_comment_draft(&mut self, cx: &mut Context<Self>) -> bool {
        let pending_motion_anchor = self.comment_state.pending_motion_anchor.take().is_some();
        if self.comment_state.draft.take().is_some() {
            self.comment_state.draft_attachments.clear();
            self.comment_state.draft_error = None;
            self.comment_state.mention_picker = None;
            self.comment_state.skill_picker = None;
            self.comment_state.draft_skill = None;
            self.comment_state.skill_route_notice = None;
            // Fully exit comment mode so the ghost pin doesn't re-arm.
            self.activate_tool(ToolKind::Select, cx);
            cx.notify();
            return true;
        }
        if pending_motion_anchor {
            self.activate_tool(ToolKind::Select, cx);
            cx.notify();
            return true;
        }
        false
    }

    fn post_comment_reply(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) {
            self.comment_state.reply_error =
                Some("Switch to Design to change comments. Your draft was kept.".to_owned());
            cx.notify();
            return;
        }
        self.finish_document_edits_for_external_change(cx);
        let Some(editor) = self.comment_state.reply_editor.clone() else {
            return;
        };
        let body = editor.read(cx).text(cx);
        let Some(origin) = self
            .comment_state
            .open_thread_origin
            .filter(|_| self.comment_state.open_thread.as_deref() == Some(id.as_str()))
        else {
            self.comment_state.reply_error = Some(
                "Could not post reply: the thread origin is no longer available. Reopen the thread and retry."
                    .to_string(),
            );
            cx.notify();
            return;
        };
        let page = origin.page;
        let attachments = self.comment_state.reply_attachments.clone();
        let skill = self.comment_state.reply_skill;
        let has_payload = !body.trim().is_empty() || !attachments.is_empty() || skill.is_some();
        self.comment_state.reply_error = None;
        let preview_owner = cx.entity_id();
        let outcome = self.item.update(cx, |item, cx| {
            if !item.can_preview_for_owner(preview_owner) || item.content_preview_active() {
                return CommentSubmitOutcome::Failed(
                    "Could not post reply while Save As, a source edit, or another canvas preview is active. Finish or cancel that edit and retry; your reply was kept."
                        .to_string(),
                );
            }
            let operation = item.document().and_then(|document| {
                if !document_has_comment_origin(&document.doc, origin) {
                    return None;
                }
                let mentions = comments::mentions_for_body(&document.doc, page, &body);
                comments::reply_comment_full_op(
                    &document.doc,
                    page,
                    &id,
                    &body,
                    mentions,
                    attachments,
                    skill,
                )
            });
            let Some(operation) = operation else {
                return if has_payload {
                    CommentSubmitOutcome::Failed(
                        "Could not post reply: the originating thread or page no longer exists. Reopen the thread before retrying."
                            .to_string(),
                    )
                } else {
                    CommentSubmitOutcome::Empty
                };
            };
            match item.apply_for_preview_owner(preview_owner, operation, cx) {
                Ok(()) => CommentSubmitOutcome::Applied(id.clone()),
                Err(error) => {
                    CommentSubmitOutcome::Failed(format!("Could not post reply: {error:#}"))
                }
            }
        });
        if let Some(id) = outcome.applied_id().map(str::to_string) {
            let skill_prompt = skill.and_then(|skill| {
                self.item
                    .read(cx)
                    .document()
                    .filter(|document| document.doc.id == origin.document)
                    .and_then(|document| {
                        comments::skill_prompt_for_comment(&document.doc, page, &id, skill)
                    })
            });
            editor.update(cx, |editor, cx| editor.clear(window, cx));
            self.comment_state.thread_scroll.scroll_to_bottom();
            self.comment_state.reply_attachments.clear();
            self.comment_state.reply_composer_id = Some(NodeId::new().to_string());
            self.comment_state.reply_error = None;
            self.comment_state.mention_picker = None;
            self.comment_state.skill_picker = None;
            self.comment_state.reply_skill = None;
            if let Some(comment) = self
                .comments_for_origin(origin, cx)
                .into_iter()
                .find(|comment| comment.id == id)
            {
                self.comment_state
                    .read_until
                    .insert(id, comment.newest_created());
            }
            if skill.is_some() {
                self.route_comment_skill_prompt(skill_prompt, window, cx);
            }
        } else if let CommentSubmitOutcome::Failed(error) = outcome {
            log::error!("replying to a canvas comment failed: {error}");
            self.comment_state.reply_error = Some(error);
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    fn resolve_comment_thread(&mut self, id: String, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) {
            self.comment_state.reply_error =
                Some("Switch to Design to change comments. Your draft was kept.".to_owned());
            cx.notify();
            return;
        }
        self.finish_document_edits_for_external_change(cx);
        let Some(origin) = self
            .comment_state
            .open_thread_origin
            .filter(|_| self.comment_state.open_thread.as_deref() == Some(id.as_str()))
        else {
            self.comment_state.reply_error = Some(
                "Could not change the thread status because its page is no longer available. Reopen the thread and retry."
                    .to_string(),
            );
            cx.notify();
            return;
        };
        let preview_owner = cx.entity_id();
        let outcome = self.item.update(cx, |item, cx| {
            if !item.can_preview_for_owner(preview_owner) || item.content_preview_active() {
                return CommentMutationOutcome::Failed(
                    "Could not change the thread status while Save As, a source edit, or another canvas preview is active. Finish or cancel that edit and retry."
                        .to_string(),
                );
            }
            let operation = item.document().and_then(|document| {
                document_has_comment_origin(&document.doc, origin)
                .then(|| comments::toggle_resolved_op(&document.doc, origin.page, &id))
                .flatten()
            });
            let Some(operation) = operation else {
                return CommentMutationOutcome::Failed(
                    "Could not change the thread status because the originating thread or page no longer exists. Reopen the thread and retry."
                        .to_string(),
                );
            };
            match item.apply_for_preview_owner(preview_owner, operation, cx) {
                Ok(()) => CommentMutationOutcome::Applied,
                Err(error) => CommentMutationOutcome::Failed(format!(
                    "Could not change the thread status: {error:#}. Your reply draft and attachments were kept."
                )),
            }
        });
        match outcome {
            CommentMutationOutcome::Applied => self.comment_state.reply_error = None,
            CommentMutationOutcome::Failed(error) => {
                log::error!("resolving a canvas comment failed: {error}");
                self.comment_state.reply_error = Some(error);
            }
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    fn delete_comment_thread(&mut self, id: String, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) {
            self.comment_state.reply_error =
                Some("Switch to Design to change comments. Your draft was kept.".to_owned());
            cx.notify();
            return;
        }
        self.finish_document_edits_for_external_change(cx);
        let Some(origin) = self
            .comment_state
            .open_thread_origin
            .filter(|_| self.comment_state.open_thread.as_deref() == Some(id.as_str()))
        else {
            self.comment_state.reply_error = Some(
                "Could not delete the thread because its page is no longer available. Reopen the thread and retry."
                    .to_string(),
            );
            cx.notify();
            return;
        };
        let preview_owner = cx.entity_id();
        let outcome = self.item.update(cx, |item, cx| {
            if !item.can_preview_for_owner(preview_owner) || item.content_preview_active() {
                return CommentMutationOutcome::Failed(
                    "Could not delete the thread while Save As, a source edit, or another canvas preview is active. Finish or cancel that edit and retry."
                        .to_string(),
                );
            }
            let operation = item.document().and_then(|document| {
                document_has_comment_origin(&document.doc, origin)
                .then(|| comments::remove_comment_op(&document.doc, origin.page, &id))
                .flatten()
            });
            let Some(operation) = operation else {
                return CommentMutationOutcome::Failed(
                    "Could not delete the thread because the originating thread or page no longer exists. Reopen the thread and retry."
                        .to_string(),
                );
            };
            match item.apply_for_preview_owner(preview_owner, operation, cx) {
                Ok(()) => CommentMutationOutcome::Applied,
                Err(error) => CommentMutationOutcome::Failed(format!(
                    "Could not delete the thread: {error:#}. The thread, reply draft, and attachments were kept."
                )),
            }
        });
        match outcome {
            CommentMutationOutcome::Applied => self.comment_state.close_thread(),
            CommentMutationOutcome::Failed(error) => {
                log::error!("deleting a canvas comment failed: {error}");
                self.comment_state.reply_error = Some(error);
            }
        }
        self.invalidate_canvas_cache();
        cx.notify();
    }

    fn insert_comment_mention(
        &mut self,
        target: CommentComposerTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.comment_state.skill_picker = None;
        if self.comment_state.mention_picker == Some(target) {
            self.comment_state.mention_picker = None;
            cx.notify();
            return;
        }
        let editor = match target {
            CommentComposerTarget::Draft => self
                .comment_state
                .draft
                .as_ref()
                .map(|draft| draft.editor.clone()),
            CommentComposerTarget::Reply => self.comment_state.reply_editor.clone(),
        };
        if let Some(editor) = editor {
            editor.update(cx, |editor, cx| editor.insert("@", window, cx));
            editor.read(cx).focus_handle(cx).focus(window, cx);
            self.comment_state.mention_picker = Some(target);
            cx.notify();
        }
    }

    fn choose_comment_mention(
        &mut self,
        target: CommentComposerTarget,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = match target {
            CommentComposerTarget::Draft => self
                .comment_state
                .draft
                .as_ref()
                .map(|draft| draft.editor.clone()),
            CommentComposerTarget::Reply => self.comment_state.reply_editor.clone(),
        };
        if let Some(editor) = editor {
            let handle = label
                .split_whitespace()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("_");
            editor.update(cx, |editor, cx| {
                editor.insert(&format!("{handle} "), window, cx)
            });
            editor.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.comment_state.mention_picker = None;
        cx.notify();
    }

    fn comment_mention_candidates(&self, cx: &App) -> Vec<String> {
        let mut candidates = vec![
            "Claude".to_string(),
            "Codex".to_string(),
            "Fanta".to_string(),
        ];
        if let Some(document) = self.item.read(cx).document()
            && let Some(page) = document.doc.active_page()
        {
            for id in document.doc.scene.descendants_of(page).skip(1) {
                let Some(node) = document.doc.scene.get(id) else {
                    continue;
                };
                let name = node.name.trim();
                if name.is_empty()
                    || candidates
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                candidates.push(name.to_string());
                if candidates.len() >= 8 {
                    break;
                }
            }
        }
        candidates
    }

    fn toggle_comment_skill_picker(
        &mut self,
        target: CommentComposerTarget,
        cx: &mut Context<Self>,
    ) {
        self.comment_state.mention_picker = None;
        self.comment_state.skill_picker = if self.comment_state.skill_picker == Some(target) {
            None
        } else {
            Some(target)
        };
        cx.notify();
    }

    fn choose_comment_skill(
        &mut self,
        target: CommentComposerTarget,
        skill: Option<CommentSkill>,
        cx: &mut Context<Self>,
    ) {
        match target {
            CommentComposerTarget::Draft => self.comment_state.draft_skill = skill,
            CommentComposerTarget::Reply => self.comment_state.reply_skill = skill,
        }
        self.comment_state.skill_picker = None;
        cx.notify();
    }

    fn selected_comment_skill(&self, target: CommentComposerTarget) -> Option<CommentSkill> {
        match target {
            CommentComposerTarget::Draft => self.comment_state.draft_skill,
            CommentComposerTarget::Reply => self.comment_state.reply_skill,
        }
    }

    pub(crate) fn apply_open_comment_suggested_fix(&mut self, cx: &mut Context<Self>) {
        let result = (|| {
            let origin = self
                .comment_state
                .open_thread_origin
                .ok_or_else(|| anyhow::anyhow!("open a comment thread first"))?;
            let id = self
                .comment_state
                .open_thread
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("open a comment thread first"))?;
            anyhow::ensure!(
                self.item
                    .read(cx)
                    .document()
                    .is_some_and(|document| document_has_comment_origin(&document.doc, origin)),
                "the comment document is no longer open"
            );
            crate::agent_surface::apply_comment_suggested_fix(&self.item, origin.page, id, cx)
        })();
        if let Err(error) = result {
            crate::view::show_canvas_notice_deferred(
                format!("Could not apply suggested fix: {error:#}"),
                cx,
            );
        }
    }

    fn route_comment_skill_prompt(
        &mut self,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let notice = match prompt {
            None => SkillRouteNotice::Failed(
                "The comment was posted, but its Agent prompt could not be built.".to_string(),
            ),
            Some(prompt) => {
                let workspace = window
                    .root::<MultiWorkspace>()
                    .flatten()
                    .map(|multi_workspace| multi_workspace.read(cx).workspace().clone());
                match workspace {
                    Some(workspace) => {
                        match agent_ui::open_external_prompt_for_review(
                            workspace, &prompt, window, cx,
                        ) {
                            Ok(()) => SkillRouteNotice::ReadyForReview,
                            Err(error) => SkillRouteNotice::Failed(format!(
                                "The comment was posted, but the Agent prompt could not be opened: {error:#}"
                            )),
                        }
                    }
                    None => SkillRouteNotice::Failed(
                        "The comment was posted, but this window has no active workspace for the Agent Panel."
                            .to_string(),
                    ),
                }
            }
        };
        if let SkillRouteNotice::Failed(error) = &notice {
            log::error!("routing a canvas comment skill failed: {error}");
        }
        self.comment_state.skill_route_notice = Some(notice);
        cx.notify();
    }

    fn choose_comment_attachments(
        &mut self,
        target: CommentComposerTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(origin) = self.attachment_picker_origin(target) else {
            return;
        };
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach images or files".into()),
        });
        cx.spawn(async move |this, cx| {
            let selected = match paths.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return Ok::<(), anyhow::Error>(()),
            };
            #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
            if let Err(error) = workspace::remember_user_selected_paths(&selected) {
                log::warn!("Could not retain access to comment attachments: {error:#}");
            }
            this.update(cx, |this, cx| {
                if this.attachment_picker_origin(origin.target()).as_ref() != Some(&origin) {
                    return;
                }
                let attachments = match origin.target() {
                    CommentComposerTarget::Draft => &mut this.comment_state.draft_attachments,
                    CommentComposerTarget::Reply => &mut this.comment_state.reply_attachments,
                };
                for path in selected {
                    if attachments
                        .iter()
                        .any(|attachment| attachment.path.as_ref() == Some(&path))
                    {
                        continue;
                    }
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Attachment")
                        .to_string();
                    attachments.push(Attachment {
                        name,
                        path: Some(path),
                    });
                }
                cx.notify();
            })?;
            Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn remove_comment_attachment(
        &mut self,
        target: CommentComposerTarget,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let attachments = match target {
            CommentComposerTarget::Draft => &mut self.comment_state.draft_attachments,
            CommentComposerTarget::Reply => &mut self.comment_state.reply_attachments,
        };
        if index < attachments.len() {
            attachments.remove(index);
            cx.notify();
        }
    }

    /// The floating comment UI: the draft composer, or the open thread
    /// popover, or the hover preview card. `None` when nothing is showing.
    pub(crate) fn render_comment_overlay(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if let Some(draft) = &self.comment_state.draft
            && self.origin_is_active(draft.origin, cx)
        {
            return self.render_comment_composer(draft, window, cx);
        }
        if let (Some(open), Some(origin)) = (
            self.comment_state.open_thread.clone(),
            self.comment_state.open_thread_origin,
        ) && self.origin_is_active(origin, cx)
            && !self.comment_thread_in_panel(cx)
        {
            let comment = self
                .comments_for_origin(origin, cx)
                .into_iter()
                .find(|comment| comment.id == open)?;
            let orphaned_motion_anchor = comment.motion_anchor.is_some_and(|anchor| {
                self.item
                    .read(cx)
                    .document()
                    .is_none_or(|document| document.doc.motion.clip(anchor.clip).is_none())
            });
            if !comments::comment_is_visible(&comment, self.active_motion_comment_clip(cx))
                && !orphaned_motion_anchor
            {
                return None;
            }
            return self.render_comment_thread(&comment, window, cx);
        }
        if let Some(hovered) = self.comment_state.hovered_pin.clone() {
            let comment = self
                .visible_page_comments(cx)
                .into_iter()
                .find(|comment| comment.id == hovered)?;
            return self.render_comment_preview(&comment, cx);
        }
        None
    }

    fn render_comment_composer(
        &self,
        draft: &CommentDraft,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let viewport = self.viewport?;
        let motion_label = draft
            .motion_anchor
            .map(|anchor| self.motion_comment_label(anchor, cx));
        let input = comment_input(&draft.editor, window, cx);
        let context = self.render_comment_composer_context(CommentComposerTarget::Draft, cx);
        let toolbar = self.render_comment_composer_toolbar(CommentComposerTarget::Draft, cx);
        Some(comment_overlay(
            draft.world,
            viewport,
            point(px(12.), px(-4.)),
            v_flex()
                .id("fanta-comment-composer")
                .debug_selector(|| "fanta-comment-composer".to_owned())
                .key_context(COMMENT_COMPOSER_CONTEXT)
                .on_action(cx.listener(|this, _: &SubmitComment, window, cx| {
                    this.post_comment_draft(window, cx);
                }))
                .on_action(
                    cx.listener(|this, _: &editor::actions::Cancel, window, cx| {
                        this.cancel_comment_draft(cx);
                        this.focus_handle(cx).focus(window, cx);
                    }),
                )
                .w(rems(20.))
                .p_2()
                .gap_2()
                .elevation_2(cx)
                .children(motion_label.map(comment_motion_label))
                .child(input)
                .children(context)
                .child(toolbar),
        ))
    }

    fn render_comment_preview(
        &self,
        comment: &Comment,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let world = self.comment_world(comment, cx);
        let viewport = self.viewport?;
        let mut summary = Vec::new();
        if comment.resolved {
            summary.push("Resolved".to_string());
        }
        match comment.replies.len() {
            0 => {}
            1 => summary.push("1 reply".to_string()),
            replies => summary.push(format!("{replies} replies")),
        }
        if let Some(anchor) = comment.motion_anchor {
            summary.push(self.motion_comment_label(anchor, cx));
        }
        Some(comment_overlay(
            world,
            viewport,
            point(px(6.), px(-30.)),
            v_flex()
                .id("fanta-comment-preview")
                .debug_selector(|| "fanta-comment-preview".to_owned())
                .w(rems(18.))
                .p_2()
                .gap_1p5()
                .elevation_2(cx)
                .child(comment_message(
                    CommentMessageView::of_comment(comment),
                    true,
                    cx,
                ))
                .when(!summary.is_empty(), |card| {
                    card.child(
                        Label::new(summary.join(" · "))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                }),
        ))
    }

    fn comment_location(&self, comment: &Comment, cx: &App) -> Option<SharedString> {
        let doc = &self.item.read(cx).document()?.doc;
        let page_name = self
            .comment_state
            .open_thread_origin
            .and_then(|origin| doc.scene.get(origin.page))
            .map(|page| page.name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Canvas".to_string());
        Some(comment.location_label(doc, &page_name).into())
    }

    fn render_comment_thread(
        &self,
        comment: &Comment,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let world = self.comment_world(comment, cx);
        let viewport = self.viewport?;
        let location = self.comment_location(comment, cx)?;
        let motion_label = comment
            .motion_anchor
            .map(|anchor| self.motion_comment_label(anchor, cx));
        let read_only = self.is_dev_mode(cx);
        let border_variant = cx.theme().colors().border_variant;
        let close_id = comment.id.clone();

        let messages = v_flex()
            .id("fanta-comment-messages")
            .p_2()
            .gap_3()
            .max_h(rems(24.))
            .overflow_y_scroll()
            .children(comment_thread_messages(comment, cx));

        let header = v_flex()
            .pl_2()
            .pr_1()
            .py_1()
            .gap_0p5()
            .border_b_1()
            .border_color(border_variant)
            .child(
                h_flex()
                    .gap_1()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1p5()
                            .child(
                                Icon::new(if comment.resolved {
                                    IconName::Check
                                } else {
                                    IconName::Chat
                                })
                                .size(IconSize::Small)
                                .color(if comment.resolved {
                                    Color::Success
                                } else {
                                    Color::Muted
                                }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .child(Label::new(location).single_line().truncate()),
                            )
                            .when(comment.resolved, |title| {
                                title.child(comment_resolved_label())
                            }),
                    )
                    .child(
                        self.render_comment_thread_actions(comment, read_only, cx)
                            .child(
                                IconButton::new("fanta-comment-close-thread", IconName::Close)
                                    .icon_size(IconSize::Small)
                                    .icon_color(Color::Muted)
                                    .tooltip(Tooltip::text("Close"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.toggle_comment_thread(close_id.clone(), window, cx);
                                    })),
                            ),
                    ),
            )
            .children(motion_label.map(comment_motion_label));

        Some(comment_overlay(
            world,
            viewport,
            point(px(12.), px(-8.)),
            v_flex()
                .id("fanta-comment-thread")
                .debug_selector(|| "fanta-comment-thread".to_owned())
                .on_action(
                    cx.listener(|this, _: &editor::actions::Cancel, window, cx| {
                        if !this.has_unsent_comment_reply(cx)
                            && let Some(id) = this.comment_state.open_thread.clone()
                        {
                            this.toggle_comment_thread(id, window, cx);
                            this.focus_handle(cx).focus(window, cx);
                        }
                    }),
                )
                .w(rems(20.))
                .elevation_2(cx)
                .child(header)
                .child(messages)
                .children(self.render_comment_suggested_fix(comment, read_only, cx))
                .when(read_only, |thread| {
                    thread.child(comment_read_only_footer(comment, border_variant))
                })
                .children(self.render_comment_reply_composer(read_only, window, cx)),
        ))
    }

    /// The open thread as a page of the Comments panel: every message, then
    /// the reply composer, under a header whose back button returns to the
    /// list of threads.
    pub(crate) fn render_comment_thread_page(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.comment_thread_in_panel(cx) {
            return None;
        }
        let id = self.comment_state.open_thread.clone()?;
        let origin = self.comment_state.open_thread_origin?;
        let comment = self
            .comments_for_origin(origin, cx)
            .into_iter()
            .find(|comment| comment.id == id)?;
        let location = self.comment_location(&comment, cx)?;
        let motion_label = comment
            .motion_anchor
            .map(|anchor| self.motion_comment_label(anchor, cx));
        let read_only = self.is_dev_mode(cx);
        let border_variant = cx.theme().colors().border_variant;

        let header = v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border_variant)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .h_8()
                    .pl_1()
                    .pr_1()
                    .gap_1()
                    .child(
                        div()
                            .flex_none()
                            .debug_selector(|| "fanta-comment-thread-back".to_owned())
                            .child(
                                IconButton::new("fanta-comment-thread-back", IconName::ArrowLeft)
                                    .icon_size(IconSize::Small)
                                    .icon_color(Color::Muted)
                                    .tooltip(Tooltip::text("All Threads"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_comment_thread_page(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1p5()
                            .child(
                                div()
                                    .min_w_0()
                                    .child(Label::new(location).single_line().truncate()),
                            )
                            .when(comment.resolved, |title| {
                                title.child(comment_resolved_label())
                            }),
                    )
                    .child(self.render_comment_thread_actions(&comment, read_only, cx)),
            )
            .children(motion_label.map(|label| {
                div()
                    .w_full()
                    .min_w_0()
                    .px_2p5()
                    .pb_1p5()
                    .child(comment_motion_label(label))
            }));

        let mut messages = v_flex()
            .id("fanta-comment-thread-page-messages")
            .track_scroll(&self.comment_state.thread_scroll)
            .flex_1()
            .min_h_0()
            .w_full()
            .min_w_0()
            .overflow_y_scroll()
            .px_2p5()
            .py_3()
            .gap_3();
        for (index, message) in comment_thread_messages(&comment, cx).enumerate() {
            if index == 1 {
                messages = messages.child(comment_replies_divider(comment.replies.len(), cx));
            }
            messages = messages.child(
                div()
                    .w_full()
                    .min_w_0()
                    .debug_selector(move || format!("fanta-comment-thread-page-message-{index}"))
                    .child(message),
            );
        }

        Some(
            v_flex()
                .id("fanta-comment-thread-page")
                .debug_selector(|| "fanta-comment-thread-page".to_owned())
                .on_action(
                    cx.listener(|this, _: &editor::actions::Cancel, window, cx| {
                        if !this.has_unsent_comment_reply(cx) {
                            this.close_comment_thread_page(window, cx);
                            this.focus_handle(cx).focus(window, cx);
                        }
                    }),
                )
                .size_full()
                .min_w_0()
                .child(header)
                .child(messages)
                .children(
                    self.render_comment_suggested_fix(&comment, read_only, cx)
                        .map(|fix| div().flex_none().p_2().child(fix)),
                )
                .when(read_only, |page| {
                    page.child(comment_read_only_footer(&comment, border_variant))
                })
                .children(
                    self.render_comment_reply_composer(read_only, window, cx)
                        .map(|composer| composer.flex_none()),
                )
                .into_any_element(),
        )
    }

    /// Resolve and Delete for the open thread.
    fn render_comment_thread_actions(
        &self,
        comment: &Comment,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let resolve_id = comment.id.clone();
        let delete_id = comment.id.clone();
        h_flex()
            .flex_none()
            .child(
                IconButton::new(
                    "fanta-comment-resolve-thread",
                    if comment.resolved {
                        IconName::Undo
                    } else {
                        IconName::Check
                    },
                )
                .icon_size(IconSize::Small)
                .icon_color(Color::Muted)
                .disabled(read_only)
                .tooltip(Tooltip::text(if comment.resolved {
                    "Reopen Thread"
                } else {
                    "Resolve Thread"
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.resolve_comment_thread(resolve_id.clone(), cx);
                })),
            )
            .child(
                IconButton::new("fanta-comment-delete-thread", IconName::Trash)
                    .icon_size(IconSize::Small)
                    .icon_color(Color::Muted)
                    .disabled(read_only)
                    .tooltip(Tooltip::text("Delete Thread"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_comment_thread(delete_id.clone(), cx);
                    })),
            )
    }

    fn render_comment_suggested_fix(
        &self,
        comment: &Comment,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> Option<Callout> {
        let fix = comment.suggested_fix.as_ref()?;
        Some(
            Callout::new()
                .severity(Severity::Info)
                .icon(IconName::Info)
                .title(fix.summary.clone())
                .actions_slot(
                    Button::new("apply-comment-suggested-fix", "Apply Fix")
                        .label_size(LabelSize::Small)
                        .disabled(read_only)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.apply_open_comment_suggested_fix(cx)),
                        ),
                ),
        )
    }

    /// The open thread's reply editor with its context and toolbar; `None`
    /// in Dev, where comments are read-only.
    fn render_comment_reply_composer(
        &self,
        read_only: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        let editor = self
            .comment_state
            .reply_editor
            .clone()
            .filter(|_| !read_only)?;
        Some(
            v_flex()
                .key_context(COMMENT_COMPOSER_CONTEXT)
                .on_action(cx.listener(|this, _: &SubmitComment, window, cx| {
                    if let Some(id) = this.comment_state.open_thread.clone() {
                        this.post_comment_reply(id, window, cx);
                    }
                }))
                .w_full()
                .min_w_0()
                .p_2()
                .gap_2()
                .border_t_1()
                .border_color(cx.theme().colors().border_variant)
                .child(comment_input(&editor, window, cx))
                .children(self.render_comment_composer_context(CommentComposerTarget::Reply, cx))
                .child(self.render_comment_composer_toolbar(CommentComposerTarget::Reply, cx)),
        )
    }

    /// Whether Send would post anything: text, an attachment or a skill.
    fn comment_composer_has_payload(&self, target: CommentComposerTarget, cx: &App) -> bool {
        let (editor, attachments) = match target {
            CommentComposerTarget::Draft => (
                self.comment_state.draft.as_ref().map(|draft| &draft.editor),
                &self.comment_state.draft_attachments,
            ),
            CommentComposerTarget::Reply => (
                self.comment_state.reply_editor.as_ref(),
                &self.comment_state.reply_attachments,
            ),
        };
        editor.is_some_and(|editor| !editor.read(cx).text(cx).trim().is_empty())
            || !attachments.is_empty()
            || self.selected_comment_skill(target).is_some()
    }

    fn render_comment_composer_toolbar(
        &self,
        target: CommentComposerTarget,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (mention_id, attachment_id, skill_id, send_id, send_label) = match target {
            CommentComposerTarget::Draft => (
                "fanta-comment-draft-mention",
                "fanta-comment-draft-attach",
                "fanta-comment-draft-skill",
                "fanta-comment-send",
                "Post Comment",
            ),
            CommentComposerTarget::Reply => (
                "fanta-comment-reply-mention",
                "fanta-comment-reply-attach",
                "fanta-comment-reply-skill",
                "fanta-comment-reply-send",
                "Reply",
            ),
        };
        let selected_skill = self.selected_comment_skill(target);
        let can_send = self.comment_composer_has_payload(target, cx);
        let editor_focus = match target {
            CommentComposerTarget::Draft => self
                .comment_state
                .draft
                .as_ref()
                .map(|draft| draft.editor.focus_handle(cx)),
            CommentComposerTarget::Reply => self
                .comment_state
                .reply_editor
                .as_ref()
                .map(|editor| editor.focus_handle(cx)),
        };
        h_flex()
            .justify_between()
            .gap_1()
            .child(
                h_flex()
                    .gap_0p5()
                    .child(
                        IconButton::new(mention_id, IconName::AtSign)
                            .icon_size(IconSize::Small)
                            .icon_color(Color::Muted)
                            .toggle_state(self.comment_state.mention_picker == Some(target))
                            .tooltip(Tooltip::text("Mention a Layer or Agent"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.insert_comment_mention(target, window, cx);
                            })),
                    )
                    .child(
                        IconButton::new(attachment_id, IconName::Paperclip)
                            .icon_size(IconSize::Small)
                            .icon_color(Color::Muted)
                            .tooltip(Tooltip::text("Attach Images or Files"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.choose_comment_attachments(target, cx);
                            })),
                    )
                    .child(
                        IconButton::new(skill_id, IconName::Sparkle)
                            .icon_size(IconSize::Small)
                            .icon_color(if selected_skill.is_some() {
                                Color::Accent
                            } else {
                                Color::Muted
                            })
                            .toggle_state(self.comment_state.skill_picker == Some(target))
                            .tooltip(Tooltip::text(match selected_skill {
                                Some(skill) => format!("AI Skill: {}", skill.label()),
                                None => "Add an AI Skill".to_string(),
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_comment_skill_picker(target, cx);
                            })),
                    ),
            )
            .child(
                IconButton::new(send_id, IconName::Send)
                    .style(ButtonStyle::Filled)
                    .icon_size(IconSize::Small)
                    .map(|button| {
                        if can_send {
                            button.icon_color(Color::Accent)
                        } else {
                            button.icon_color(Color::Muted).disabled(true)
                        }
                    })
                    .tooltip(move |_, cx| match &editor_focus {
                        Some(focus) => {
                            Tooltip::for_action_in(send_label, &SubmitComment, focus, cx)
                        }
                        None => Tooltip::simple(send_label, cx),
                    })
                    .on_click(cx.listener(move |this, _, window, cx| match target {
                        CommentComposerTarget::Draft => this.post_comment_draft(window, cx),
                        CommentComposerTarget::Reply => {
                            if let Some(id) = this.comment_state.open_thread.clone() {
                                this.post_comment_reply(id, window, cx);
                            }
                        }
                    })),
            )
            .into_any_element()
    }

    fn render_comment_composer_context(
        &self,
        target: CommentComposerTarget,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (attachments, error, strip_id, remove_id, mention_id, skill_id) = match target {
            CommentComposerTarget::Draft => (
                self.comment_state.draft_attachments.clone(),
                self.comment_state.draft_error.clone(),
                "fanta-comment-draft-attachments",
                "fanta-comment-draft-attachment-remove",
                "fanta-comment-draft-mention-candidate",
                "fanta-comment-draft-skill-candidate",
            ),
            CommentComposerTarget::Reply => (
                self.comment_state.reply_attachments.clone(),
                self.comment_state.reply_error.clone(),
                "fanta-comment-reply-attachments",
                "fanta-comment-reply-attachment-remove",
                "fanta-comment-reply-mention-candidate",
                "fanta-comment-reply-skill-candidate",
            ),
        };
        let mention_picker_open = self.comment_state.mention_picker == Some(target);
        let skill_picker_open = self.comment_state.skill_picker == Some(target);
        let selected_skill = self.selected_comment_skill(target);
        let route_notice = match target {
            CommentComposerTarget::Draft => None,
            CommentComposerTarget::Reply => self.comment_state.skill_route_notice.clone(),
        };
        if attachments.is_empty()
            && error.is_none()
            && !mention_picker_open
            && !skill_picker_open
            && selected_skill.is_none()
            && route_notice.is_none()
        {
            return None;
        }

        let colors = cx.theme().colors();
        let (border_variant, editor_background, surface) = (
            colors.border_variant,
            colors.editor_background,
            colors.elevated_surface_background,
        );
        let mut context = v_flex().gap_1p5();
        if mention_picker_open {
            let mut menu = comment_menu("Mention", border_variant, editor_background);
            for (index, label) in self.comment_mention_candidates(cx).into_iter().enumerate() {
                let icon = if COMMENT_AGENT_MENTIONS.contains(&label.as_str()) {
                    IconName::Sparkle
                } else {
                    IconName::ToolFrame
                };
                let selected_label = label.clone();
                menu = menu.child(
                    ListItem::new((mention_id, index))
                        .inset(true)
                        .spacing(ListItemSpacing::Dense)
                        .start_slot(Icon::new(icon).size(IconSize::Small).color(Color::Muted))
                        .child(
                            Label::new(format!("@{label}"))
                                .size(LabelSize::Small)
                                .single_line()
                                .truncate(),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_comment_mention(target, selected_label.clone(), window, cx);
                        })),
                );
            }
            context = context.child(menu);
        }
        if skill_picker_open {
            let mut menu = comment_menu(
                "Run in the Agent Panel after posting",
                border_variant,
                editor_background,
            );
            for (index, skill) in CommentSkill::ALL
                .into_iter()
                .map(Some)
                .chain([None])
                .enumerate()
            {
                let selected = selected_skill == skill;
                menu = menu.child(
                    ListItem::new((skill_id, index))
                        .inset(true)
                        .spacing(ListItemSpacing::Dense)
                        .toggle_state(selected)
                        .start_slot(
                            Icon::new(if skill.is_some() {
                                IconName::Sparkle
                            } else {
                                IconName::Close
                            })
                            .size(IconSize::Small)
                            .color(Color::Muted),
                        )
                        .child(
                            Label::new(skill.map_or("No Skill", CommentSkill::label))
                                .size(LabelSize::Small),
                        )
                        .end_slot::<Icon>(selected.then(|| {
                            Icon::new(IconName::Check)
                                .size(IconSize::Small)
                                .color(Color::Accent)
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_comment_skill(target, skill, cx);
                        })),
                );
            }
            context = context.child(menu);
        } else if let Some(skill) = selected_skill {
            context = context.child(
                h_flex()
                    .gap_1p5()
                    .child(comment_tag(skill.label(), Some(IconName::Sparkle), cx))
                    .child(
                        Label::new("Opens an Agent Panel draft after posting")
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .single_line()
                            .truncate(),
                    ),
            );
        }
        if let Some(notice) = route_notice {
            context = context.child(match notice {
                SkillRouteNotice::ReadyForReview => comment_status_line(
                    IconName::Check,
                    Color::Success,
                    "Agent draft ready to review in the Agent Panel".into(),
                    Color::Muted,
                ),
                SkillRouteNotice::Failed(error) => {
                    comment_status_line(IconName::XCircle, Color::Error, error.into(), Color::Error)
                }
            });
        }
        if let Some(error) = error {
            context = context.child(comment_status_line(
                IconName::XCircle,
                Color::Error,
                error.into(),
                Color::Error,
            ));
        }
        if !attachments.is_empty() {
            let mut strip = h_flex().id(strip_id).gap_1p5().overflow_x_scroll();
            for (index, attachment) in attachments.into_iter().enumerate() {
                let tile = div()
                    .relative()
                    .flex_none()
                    .w(rems(5.5))
                    .h(rems(4.))
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(border_variant)
                    .bg(editor_background);
                let tile = match attachment
                    .path
                    .as_ref()
                    .filter(|path| is_previewable_image(path))
                {
                    Some(path) => {
                        tile.child(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
                    }
                    None => tile.child(
                        v_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .px_1()
                            .gap_1()
                            .child(
                                Icon::new(IconName::FileGeneric)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(attachment.name.clone())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .single_line()
                                    .truncate(),
                            ),
                    ),
                };
                strip = strip.child(
                    tile.child(
                        div()
                            .absolute()
                            .top_0p5()
                            .right_0p5()
                            .rounded_sm()
                            .bg(surface)
                            .child(
                                IconButton::new((remove_id, index), IconName::Close)
                                    .icon_size(IconSize::XSmall)
                                    .icon_color(Color::Muted)
                                    .tooltip(Tooltip::text("Remove Attachment"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_comment_attachment(target, index, cx);
                                    })),
                            ),
                    ),
                );
            }
            context = context.child(strip);
        }
        Some(context.into_any_element())
    }
}

/// The key context of the draft and reply composers; the keymaps bind Enter
/// to [`SubmitComment`] inside it.
const COMMENT_COMPOSER_CONTEXT: &str = "FantaCommentComposer";

/// Mention candidates that address an agent rather than a layer.
const COMMENT_AGENT_MENTIONS: [&str; 3] = ["Claude", "Codex", "Fanta"];

/// One message of a thread, borrowed for rendering.
struct CommentMessageView<'a> {
    author: &'a str,
    from_agent: bool,
    created: u64,
    body: &'a str,
    mentions: &'a [Mention],
    attachments: &'a [Attachment],
    skill: Option<CommentSkill>,
}

impl<'a> CommentMessageView<'a> {
    fn of_comment(comment: &'a Comment) -> Self {
        Self {
            author: &comment.author,
            from_agent: comment.from_agent,
            created: comment.created,
            body: &comment.text,
            mentions: &comment.mentions,
            attachments: &comment.attachments,
            skill: comment.skill,
        }
    }

    fn of_reply(reply: &'a comments::CommentReply) -> Self {
        Self {
            author: &reply.author,
            from_agent: reply.from_agent,
            created: reply.created,
            body: &reply.body,
            mentions: &reply.mentions,
            attachments: &reply.attachments,
            skill: reply.skill,
        }
    }
}

/// The thread's opening message followed by its replies.
fn comment_thread_messages<'a>(
    comment: &'a Comment,
    cx: &'a App,
) -> impl Iterator<Item = gpui::Div> + 'a {
    std::iter::once(comment_message(
        CommentMessageView::of_comment(comment),
        false,
        cx,
    ))
    .chain(
        comment
            .replies
            .iter()
            .map(|reply| comment_message(CommentMessageView::of_reply(reply), false, cx)),
    )
}

/// Separates a thread's opening message from its replies.
fn comment_replies_divider(replies: usize, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_2()
        .child(
            Label::new(if replies == 1 {
                "1 reply".to_string()
            } else {
                format!("{replies} replies")
            })
            .size(LabelSize::Small)
            .color(Color::Muted)
            .single_line(),
        )
        .child(div().flex_1().h_px().bg(cx.theme().colors().border_variant))
}

fn comment_resolved_label() -> impl IntoElement {
    Label::new("Resolved")
        .size(LabelSize::Small)
        .color(Color::Muted)
        .single_line()
}

/// Dev shows threads read-only, with a way to copy their text.
fn comment_read_only_footer(comment: &Comment, border: Hsla) -> impl IntoElement {
    let copy_text = std::iter::once(comment.text.clone())
        .chain(comment.replies.iter().map(|reply| reply.body.clone()))
        .collect::<Vec<_>>()
        .join("\n\n");
    h_flex()
        .debug_selector(|| "dev-comment-read-only".to_owned())
        .flex_none()
        .pl_2()
        .pr_1()
        .py_1()
        .gap_2()
        .justify_between()
        .border_t_1()
        .border_color(border)
        .child(
            Label::new("Read-only in Dev")
                .size(LabelSize::Small)
                .color(Color::Muted),
        )
        .child(
            div()
                .debug_selector(|| "dev-comment-copy".to_owned())
                .child(
                    IconButton::new("fanta-comment-copy-thread", IconName::Copy)
                        .icon_size(IconSize::Small)
                        .icon_color(Color::Muted)
                        .tooltip(Tooltip::text("Copy Thread Text"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                copy_text.clone(),
                            ))
                        }),
                ),
        )
}

/// A message row: avatar, author and time, then the body and its context.
/// `compact` (the hover preview) clamps the body and lists attachments by
/// count instead of previewing them.
fn comment_message(message: CommentMessageView, compact: bool, cx: &App) -> gpui::Div {
    let author = comment_author_label(message.author);
    h_flex()
        .items_start()
        .gap_2()
        .child(CommentAvatar::new(author.clone(), message.from_agent))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1p5()
                        .child(
                            div()
                                .min_w_0()
                                .child(Label::new(author).single_line().truncate()),
                        )
                        .child(
                            Label::new(relative_time(message.created))
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .single_line(),
                        ),
                )
                .when(!message.body.trim().is_empty(), |content| {
                    let body = Label::new(message.body.to_string());
                    content.child(if compact { body.line_clamp(4) } else { body })
                })
                .children(comment_message_context(&message, compact, cx)),
        )
}

fn comment_message_context(
    message: &CommentMessageView,
    compact: bool,
    cx: &App,
) -> Option<AnyElement> {
    if message.mentions.is_empty() && message.attachments.is_empty() && message.skill.is_none() {
        return None;
    }
    let mut tags = h_flex().pt_0p5().gap_1().flex_wrap();
    for mention in message.mentions {
        tags = tags.child(comment_tag(format!("@{}", mention.label), None, cx));
    }
    if let Some(skill) = message.skill {
        tags = tags.child(comment_tag(skill.label(), Some(IconName::Sparkle), cx));
    }
    let mut context = v_flex().gap_1p5();
    if compact {
        if !message.attachments.is_empty() {
            let count = message.attachments.len();
            tags = tags.child(comment_tag(
                if count == 1 {
                    "1 attachment".to_string()
                } else {
                    format!("{count} attachments")
                },
                Some(IconName::Paperclip),
                cx,
            ));
        }
        return Some(context.child(tags).into_any_element());
    }
    let mut previews = Vec::new();
    for attachment in message.attachments {
        match attachment
            .path
            .as_ref()
            .filter(|path| is_previewable_image(path))
        {
            Some(path) => previews.push(
                div()
                    .w_full()
                    .h(rems(8.))
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
                    .into_any_element(),
            ),
            None => {
                tags = tags.child(comment_tag(
                    attachment.name.clone(),
                    Some(IconName::FileGeneric),
                    cx,
                ))
            }
        }
    }
    if !message.mentions.is_empty()
        || message.skill.is_some()
        || previews.len() < message.attachments.len()
    {
        context = context.child(tags);
    }
    Some(context.children(previews).into_any_element())
}

/// A compact, muted tag for mentions, skills and file attachments.
fn comment_tag(
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    cx: &App,
) -> impl IntoElement {
    let colors = cx.theme().colors();
    h_flex()
        .flex_none()
        .max_w_full()
        .min_w_0()
        .h(rems_from_px(20.))
        .px_1p5()
        .gap_1()
        .rounded_sm()
        .bg(colors.element_background)
        .border_1()
        .border_color(colors.border_variant)
        .when_some(icon, |tag, icon| {
            tag.child(Icon::new(icon).size(IconSize::XSmall).color(Color::Muted))
        })
        .child(
            Label::new(label)
                .size(LabelSize::Small)
                .color(Color::Muted)
                .single_line()
                .truncate(),
        )
}

/// The bordered field around a composer's editor.
fn comment_input(editor: &Entity<Editor>, window: &Window, cx: &App) -> gpui::Div {
    let colors = cx.theme().colors();
    div()
        .w_full()
        .min_w_0()
        .px_2()
        .py_1p5()
        .rounded_md()
        .border_1()
        .border_color(if editor.focus_handle(cx).is_focused(window) {
            colors.border_focused
        } else {
            colors.border_variant
        })
        .bg(colors.editor_background)
        .child(editor.clone())
}

/// An inline menu inside a composer card (mention and skill pickers).
fn comment_menu(title: &'static str, border: Hsla, background: Hsla) -> gpui::Div {
    v_flex()
        .w_full()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(background)
        .child(
            div().px_2().pb_0p5().child(
                Label::new(title)
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .single_line(),
            ),
        )
}

fn comment_status_line(
    icon: IconName,
    icon_color: Color,
    message: SharedString,
    color: Color,
) -> AnyElement {
    h_flex()
        .items_start()
        .gap_1p5()
        .child(Icon::new(icon).size(IconSize::Small).color(icon_color))
        .child(Label::new(message).size(LabelSize::Small).color(color))
        .into_any_element()
}

/// The animation and timecode a time comment is anchored to.
fn comment_motion_label(label: String) -> impl IntoElement {
    h_flex()
        .min_w_0()
        .gap_1()
        .child(
            Icon::new(IconName::Clock)
                .size(IconSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            Label::new(label)
                .size(LabelSize::Small)
                .color(Color::Muted)
                .single_line()
                .truncate(),
        )
}

/// The name shown for a comment's author.
pub(crate) fn comment_author_label(author: &str) -> SharedString {
    let author = author.trim();
    if author.is_empty() {
        "Unknown".into()
    } else {
        author.to_string().into()
    }
}

/// Deterministic per-author color shared by the canvas pins and the thread
/// avatars: FNV-1a over the normalized name, hashed onto the hue wheel at the
/// original fanta's fixed saturation and value.
pub(crate) fn comment_author_color(author: &str) -> Hsla {
    let normalized = author.trim().to_lowercase();
    let mut hash: u32 = 0x811c_9dc5;
    for byte in normalized.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let hue = (hash % 360) as f32 / 360.0;
    // HSV(h, 0.52, 0.72) converted to HSL.
    let value = 0.72;
    let saturation_v = 0.52;
    let lightness = value * (1.0 - saturation_v / 2.0);
    let saturation = if lightness <= 0.0 || lightness >= 1.0 {
        0.0
    } else {
        (value - lightness) / f32::min(lightness, 1.0 - lightness)
    };
    gpui::hsla(hue, saturation, lightness, 1.0)
}

/// The author's monogram on their pin color, or a sparkle on the accent for
/// agent messages, matching the canvas pins.
#[derive(IntoElement)]
pub(crate) struct CommentAvatar {
    author: SharedString,
    from_agent: bool,
}

impl CommentAvatar {
    pub(crate) fn new(author: impl Into<SharedString>, from_agent: bool) -> Self {
        Self {
            author: author.into(),
            from_agent,
        }
    }
}

impl RenderOnce for CommentAvatar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let avatar = h_flex()
            .flex_none()
            .size(rems_from_px(20.))
            .justify_center()
            .rounded_full();
        if self.from_agent {
            return avatar.bg(cx.theme().players().local().cursor).child(
                Icon::new(IconName::Sparkle)
                    .size(IconSize::XSmall)
                    .color(Color::Custom(gpui::white())),
            );
        }
        let monogram = self
            .author
            .trim()
            .chars()
            .next()
            .map(|character| character.to_uppercase().collect::<String>())
            .unwrap_or_else(|| "?".to_string());
        avatar.bg(comment_author_color(&self.author)).child(
            Label::new(monogram)
                .size(LabelSize::XSmall)
                .color(Color::Custom(gpui::white())),
        )
    }
}

fn is_previewable_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "avif" | "gif" | "jpeg" | "jpg" | "png" | "webp"
            )
        })
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
        return format!(
            "{minutes} minute{} ago",
            if minutes == 1 { "" } else { "s" }
        );
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

#[cfg(test)]
mod tests {
    use fanta_doc::{CanvasNode, Doc, DocId, GroupNode, NodeData, NodeId, Operation};

    use super::{
        Attachment, AttachmentPickerOrigin, CommentComposerTarget, CommentMutationOutcome,
        CommentOrigin, CommentSkill, CommentState, CommentSubmitOutcome, SkillRouteNotice,
        document_has_comment_origin,
    };
    use gpui::px;

    #[gpui::test]
    async fn comment_cards_fit_canvas_edges_after_resize_and_font_changes(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::{CommentDraft, FigView, comments};
        use gpui::{AppContext as _, VisualTestContext, size};
        use project::{FakeFs, Project};
        use settings::SettingsStore;

        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            let settings = SettingsStore::test(cx);
            cx.set_global(settings);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut document = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        document.apply(Operation::create_node(page)).expect("page");
        document.add_page(page_id);
        document.set_active_page(Some(page_id));
        document.selection.select_only(page_id);
        let (comment_id, operation) = comments::add_comment_op(
            &document,
            page_id,
            [0., 0.],
            "A comment with enough text to wrap onto several lines when the canvas gets narrower.",
        )
        .expect("comment");
        document.apply(operation).expect("add comment");
        let item = crate::document::ready_item_for_test(
            &project,
            "/tmp/Comment-layout.fig".into(),
            document,
            cx,
        );
        let before = item.read_with(cx, |item, _| {
            let document = item.document().expect("document");
            (
                document.doc.scene.get(page_id).expect("page").meta.clone(),
                document.doc.history.undo_depth(),
                item.is_dirty(),
            )
        });
        let window = cx.add_window({
            let item = item.clone();
            move |window, cx| FigView::new(item, project, window, cx)
        });
        let view = window.entity(cx).expect("view");
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let exposed_canvas_position = |visual: &mut VisualTestContext| {
            let canvas = visual.debug_bounds("fig-container").expect("canvas");
            let position = gpui::point(canvas.left() + px(2.), canvas.center().y);
            assert!(canvas.contains(&position));
            for selector in [
                "fanta-comment-composer",
                "fanta-comment-thread",
                "fanta-comment-preview",
                "file-inspector-floating-card",
                "fanta-layers-sidebar",
                "properties-inspector-floating-card",
                "fanta-inspector-overlay",
                "fanta-canvas-toolbar",
            ] {
                if let Some(bounds) = visual.debug_bounds(selector) {
                    assert!(
                        !bounds.contains(&position),
                        "canvas input point {position:?} is covered by {selector}: {bounds:?}"
                    );
                }
            }
            position
        };
        visual.simulate_resize(size(px(1_200.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear());

        for selector in [
            "fanta-comment-composer",
            "fanta-comment-thread",
            "fanta-comment-preview",
        ] {
            visual.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.comment_state = CommentState::default();
                    let origin = view.active_comment_origin(cx).expect("origin");
                    match selector {
                        "fanta-comment-composer" => {
                            let editor = cx.new(|cx| {
                                let mut editor = editor::Editor::auto_height(1, 6, window, cx);
                                editor.set_text(
                                    "Draft that stays intact when resizing the canvas",
                                    window,
                                    cx,
                                );
                                editor
                            });
                            view.comment_state.draft = Some(CommentDraft {
                                world: glam::DVec2::ZERO,
                                editor,
                                motion_anchor: None,
                                composer_id: "layout-draft".to_owned(),
                                origin,
                            });
                            view.comment_state.skill_picker = Some(CommentComposerTarget::Draft);
                        }
                        "fanta-comment-thread" => {
                            view.open_comment_thread_for_origin(
                                comment_id.clone(),
                                origin,
                                window,
                                cx,
                            );
                        }
                        _ => view.comment_state.hovered_pin = Some(comment_id.clone()),
                    }
                    cx.notify();
                });
            });
            for font_size in [16., 24.] {
                for window_size in [
                    size(px(1_200.), px(800.)),
                    size(px(900.), px(560.)),
                    size(px(760.), px(420.)),
                ] {
                    visual.simulate_resize(window_size);
                    visual.update(|window, cx| {
                        window.set_rem_size(px(font_size));
                        window.refresh();
                        window.draw(cx).clear();
                    });
                    let canvas = visual.debug_bounds("fig-container").expect("canvas");
                    let resized_card = visual.debug_bounds(selector).expect("resized comment card");
                    assert!(
                        resized_card.left() >= canvas.left()
                            && resized_card.right() <= canvas.right(),
                        "{selector} immediately after resize: {resized_card:?} outside {canvas:?}"
                    );
                    assert!(
                        resized_card.top() >= canvas.top()
                            && resized_card.bottom() <= canvas.bottom(),
                        "{selector} immediately after resize: {resized_card:?} outside {canvas:?}"
                    );
                    for (horizontal, vertical) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.)] {
                        view.update(&mut visual, |view, cx| {
                            view.viewport = Some(fanta_doc::Viewport {
                                center: [
                                    f64::from(canvas.size.width) * (0.5 - horizontal),
                                    f64::from(canvas.size.height) * (0.5 - vertical),
                                ],
                                zoom: 1.,
                            });
                            cx.notify();
                        });
                        visual.update(|window, cx| window.draw(cx).clear());
                        let card = visual.debug_bounds(selector).expect("comment card");
                        assert!(card.size.width > px(0.) && card.size.height > px(0.));
                        assert!(
                            card.left() >= canvas.left() && card.right() <= canvas.right(),
                            "{selector} at font {font_size}: {card:?} outside {canvas:?}"
                        );
                        assert!(
                            card.top() >= canvas.top() && card.bottom() <= canvas.bottom(),
                            "{selector} at font {font_size}: {card:?} outside {canvas:?}"
                        );
                    }
                }
            }
            let card = visual.debug_bounds(selector).expect("comment card");
            let viewport_before = view.read_with(&visual, |view, _| view.viewport);
            let card_padding = gpui::point(card.right() - px(2.), card.top() + px(2.));
            visual.simulate_click(card_padding, gpui::Modifiers::none());
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: card_padding,
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10_000.))),
                ..Default::default()
            });
            visual.update(|window, cx| window.draw(cx).clear());
            view.read_with(&visual, |view, _| {
                assert_eq!(view.viewport, viewport_before)
            });
            item.read_with(&visual, |item, _| {
                assert_eq!(
                    item.document().expect("document").doc.selection.as_slice(),
                    &[page_id]
                );
            });
            if selector != "fanta-comment-preview" {
                let send = visual
                    .debug_bounds("ICON-Send")
                    .expect("send button after scrolling");
                assert!(
                    send.left() >= card.left() && send.right() <= card.right(),
                    "{selector} send button outside card: {send:?} outside {card:?}"
                );
                assert!(
                    send.top() >= card.top() && send.bottom() <= card.bottom(),
                    "{selector} send button unreachable after scrolling: {send:?} outside {card:?}"
                );
            }
            if selector == "fanta-comment-composer" {
                view.read_with(&visual, |view, cx| {
                    assert_eq!(
                        view.comment_state
                            .draft
                            .as_ref()
                            .expect("draft")
                            .editor
                            .read(cx)
                            .text(cx),
                        "Draft that stays intact when resizing the canvas"
                    );
                });
            }
            let position = exposed_canvas_position(&mut visual);
            visual.simulate_event(gpui::ScrollWheelEvent {
                position,
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-20.))),
                ..Default::default()
            });
            view.read_with(&visual, |view, _| {
                assert_ne!(
                    view.viewport, viewport_before,
                    "canvas outside card still accepts wheel input"
                );
            });
        }
        let position = exposed_canvas_position(&mut visual);
        visual.simulate_click(position, gpui::Modifiers::none());
        item.read_with(&visual, |item, _| {
            let document = item.document().expect("document");
            assert!(
                document.doc.selection.is_empty(),
                "canvas outside card still accepts selection input"
            );
            assert_eq!(
                document.doc.scene.get(page_id).expect("page").meta,
                before.0
            );
            assert_eq!(document.doc.history.undo_depth(), before.1);
            assert_eq!(item.is_dirty(), before.2);
        });
    }

    #[gpui::test]
    async fn comment_composer_typing_and_backspace_stay_in_the_editor(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::{FigView, comments};
        use crate::tools::ToolKind;
        use fanta_doc::{Color as DocumentColor, VectorNode};
        use gpui::{Focusable as _, MouseButton, size};
        use project::{FakeFs, Project};
        use settings::SettingsStore;

        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings = SettingsStore::test(cx);
            cx.set_global(settings);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut document = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        document.apply(Operation::create_node(page)).expect("page");
        document.add_page(page_id);
        document.set_active_page(Some(page_id));
        let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            30.0,
            DocumentColor::BLACK,
        )));
        rectangle.parent = Some(page_id);
        let rectangle_id = rectangle.id;
        document
            .apply(Operation::create_node(rectangle))
            .expect("rectangle");
        document.selection.select_only(rectangle_id);
        document.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            "/tmp/Comment-typing.fig".into(),
            document,
            cx,
        );
        let (view, cx) =
            cx.add_window_view(|window, cx| FigView::new(item.clone(), project, window, cx));
        cx.update(|window, cx| {
            window.activate_window();
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                cx,
            )
            .expect("canvas bindings");
            cx.bind_keys(bindings);
        });
        cx.simulate_resize(size(px(1400.), px(1000.)));
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.select_page(0, cx);
            view.activate_tool(ToolKind::Comment, cx);
            view.focus_handle(cx).focus(window, cx);
        });
        cx.run_until_parked();
        let canvas = view.read_with(cx, |view, _| view.container_bounds.expect("canvas bounds"));
        let target = canvas.center() + gpui::point(px(150.), px(120.));
        cx.simulate_mouse_down(target, MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            let draft = view.comment_state.draft.as_ref().expect("comment draft");
            assert!(
                draft.editor.focus_handle(cx).is_focused(window),
                "the composer owns focus after placing the pin"
            );
        });
        cx.simulate_input("Hello");
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(
                !view.space_pan_for_test(),
                "a space typed into the composer must not arm canvas panning"
            );
        });
        cx.simulate_input("world");
        cx.run_until_parked();
        cx.simulate_keystrokes("backspace backspace backspace");
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            let draft = view.comment_state.draft.as_ref().expect("comment draft");
            assert_eq!(draft.editor.read(cx).text(cx), "Hello wo");
        });
        item.read_with(cx, |item, _| {
            let document = item.document().expect("document");
            assert!(document.doc.scene.get(rectangle_id).is_some());
            assert_eq!(document.doc.selection.as_slice(), &[rectangle_id]);
        });

        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let posted = item.read_with(cx, |item, _| {
            let comments =
                comments::read_comments(&item.document().expect("document").doc, page_id);
            assert_eq!(comments.len(), 1, "Enter posts the composer");
            assert_eq!(comments[0].text, "Hello wo");
            comments[0].id.clone()
        });
        view.update_in(cx, |view, window, cx| {
            assert!(view.comment_state.draft.is_none());
            assert_eq!(
                view.comment_state.open_thread.as_deref(),
                Some(posted.as_str())
            );
            assert!(view.focus_handle(cx).is_focused(window));
        });

        view.update_in(cx, |view, window, cx| {
            view.toggle_comment_thread(posted, window, cx);
            view.activate_tool(ToolKind::Comment, cx);
        });
        cx.run_until_parked();
        let target = target + gpui::point(px(-120.), px(-80.));
        cx.simulate_mouse_down(target, MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_input("Discard me");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            assert!(
                view.comment_state.draft.is_none(),
                "Escape cancels the draft"
            );
            assert!(view.focus_handle(cx).is_focused(window));
        });
        item.read_with(cx, |item, _| {
            let document = item.document().expect("document");
            assert_eq!(comments::read_comments(&document.doc, page_id).len(), 1);
            assert!(document.doc.scene.get(rectangle_id).is_some());
        });
    }

    #[test]
    fn comment_origin_remains_bound_to_its_page_when_the_active_page_changes() {
        let mut document = Doc::new();
        let first = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let first_id = first.id;
        document
            .apply(Operation::create_node(first))
            .expect("first page");
        document.add_page(first_id);
        let second = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let second_id = second.id;
        document
            .apply(Operation::create_node(second))
            .expect("second page");
        document.add_page(second_id);
        let origin = CommentOrigin {
            document: document.id,
            page: first_id,
        };

        assert!(document_has_comment_origin(&document, origin));
        assert!(document.set_active_page(Some(second_id)));
        assert!(document_has_comment_origin(&document, origin));
        assert!(!document_has_comment_origin(&Doc::new(), origin));
    }

    #[test]
    fn delayed_attachment_results_only_match_the_originating_composer() {
        let origin = CommentOrigin {
            document: DocId::new(),
            page: NodeId::new(),
        };
        let draft = AttachmentPickerOrigin::Draft {
            origin,
            composer_id: "draft-a".to_string(),
        };
        assert_eq!(
            draft,
            AttachmentPickerOrigin::Draft {
                origin,
                composer_id: "draft-a".to_string(),
            }
        );
        assert_ne!(
            draft,
            AttachmentPickerOrigin::Draft {
                origin,
                composer_id: "draft-b".to_string(),
            }
        );
        assert_ne!(
            draft,
            AttachmentPickerOrigin::Draft {
                origin: CommentOrigin {
                    document: DocId::new(),
                    page: origin.page,
                },
                composer_id: "draft-a".to_string(),
            }
        );

        let reply = AttachmentPickerOrigin::Reply {
            origin,
            thread_id: "thread-a".to_string(),
            composer_id: "reply-a".to_string(),
        };
        assert_ne!(
            reply,
            AttachmentPickerOrigin::Reply {
                origin: CommentOrigin {
                    document: origin.document,
                    page: NodeId::new(),
                },
                thread_id: "thread-a".to_string(),
                composer_id: "reply-a".to_string(),
            }
        );
    }

    #[test]
    fn failed_comment_apply_preserves_the_composer_payload_and_opens_no_thread() {
        let failure = CommentSubmitOutcome::Failed("locked".to_string());
        let mut draft = Some("unchanged draft");
        let mut attachments = vec![Attachment {
            name: "reference.png".to_string(),
            path: Some("/tmp/reference.png".into()),
        }];
        if failure.applied_id().is_some() {
            draft = None;
            attachments.clear();
        }

        assert_eq!(failure.applied_id(), None);
        assert_eq!(draft, Some("unchanged draft"));
        assert_eq!(attachments.len(), 1);
        assert_eq!(CommentSubmitOutcome::Empty.applied_id(), None);
        assert_eq!(
            CommentSubmitOutcome::Applied("thread".to_string()).applied_id(),
            Some("thread")
        );
    }

    #[test]
    fn failed_thread_mutation_preserves_reply_payload() {
        let failure = CommentMutationOutcome::Failed("locked".to_string());
        let mut reply = Some("unsent reply");
        let mut attachments = vec![Attachment {
            name: "reference.png".to_string(),
            path: Some("/tmp/reference.png".into()),
        }];
        if failure == CommentMutationOutcome::Applied {
            reply = None;
            attachments.clear();
        }

        assert_eq!(reply, Some("unsent reply"));
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn closing_a_thread_clears_every_reply_payload_and_picker() {
        let mut state = CommentState::default();
        state.open_thread = Some("thread".to_string());
        state.open_thread_origin = Some(CommentOrigin {
            document: DocId::new(),
            page: NodeId::new(),
        });
        state.reply_composer_id = Some("composer".to_string());
        state.reply_attachments.push(Attachment {
            name: "reference.png".to_string(),
            path: Some("/tmp/reference.png".into()),
        });
        state.reply_error = Some("retry".to_string());
        state.mention_picker = Some(CommentComposerTarget::Reply);
        state.skill_picker = Some(CommentComposerTarget::Reply);
        state.reply_skill = Some(CommentSkill::Search);
        state.skill_route_notice = Some(SkillRouteNotice::ReadyForReview);

        state.close_thread();

        assert!(state.open_thread.is_none());
        assert!(state.open_thread_origin.is_none());
        assert!(state.reply_editor.is_none());
        assert!(state.reply_composer_id.is_none());
        assert!(state.reply_attachments.is_empty());
        assert!(state.reply_error.is_none());
        assert!(state.mention_picker.is_none());
        assert!(state.skill_picker.is_none());
        assert!(state.reply_skill.is_none());
        assert!(state.skill_route_notice.is_none());
    }
}
