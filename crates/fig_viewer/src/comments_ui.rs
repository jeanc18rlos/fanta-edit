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
use gpui::{AnyElement, App, Entity, Focusable as _, ObjectFit, SharedString, img};
use ui::prelude::*;
use ui::{IconButton, IconButtonShape, Tooltip};
use workspace::MultiWorkspace;

use crate::canvas::{bounds_size, screen_position_in_bounds};
use crate::comments::{self, Attachment, Comment, CommentSkill, Mention};
use crate::tools::ToolKind;
use crate::view::FigView;

/// Screen-fixed pin size; must match `canvas::COMMENT_PIN_SIZE`.
pub(crate) const PIN_SIZE: f64 = 30.0;

/// A comment being composed: the pin is placed but nothing is in the doc yet.
pub(crate) struct CommentDraft {
    pub(crate) world: DVec2,
    pub(crate) editor: Entity<Editor>,
    composer_id: String,
    origin: CommentOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CommentOrigin {
    document: DocId,
    page: NodeId,
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
    pub(crate) open_thread: Option<String>,
    open_thread_origin: Option<CommentOrigin>,
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

    fn active_comment_origin(&self, cx: &App) -> Option<CommentOrigin> {
        self.item.read(cx).document().and_then(|document| {
            Some(CommentOrigin {
                document: document.doc.id,
                page: document.doc.active_page()?,
            })
        })
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

    fn close_comment_thread_state(&mut self) {
        self.comment_state.open_thread = None;
        self.comment_state.open_thread_origin = None;
        self.comment_state.reply_editor = None;
        self.comment_state.reply_composer_id = None;
        self.comment_state.reply_attachments.clear();
        self.comment_state.reply_error = None;
        self.comment_state.mention_picker = None;
        self.comment_state.skill_picker = None;
        self.comment_state.reply_skill = None;
        self.comment_state.skill_route_notice = None;
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
        let Some(origin) = self.active_comment_origin(cx) else {
            return false;
        };
        let (width, height) = bounds_size(bounds);
        let world = fanta_canvas::screen_to_world(screen, &viewport, DVec2::new(width, height));
        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(1, 6, window, cx);
            editor.set_placeholder_text("Add a comment…", window, cx);
            editor
        });
        editor.read(cx).focus_handle(cx).focus(window, cx);
        self.comment_state.draft = Some(CommentDraft {
            world,
            editor,
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
        let Some(origin) = self.active_comment_origin(cx) else {
            return;
        };
        if self.comment_state.open_thread.as_deref() == Some(id.as_str())
            && self.comment_state.open_thread_origin == Some(origin)
        {
            self.close_comment_thread_state();
        } else {
            self.open_comment_thread_for_origin(id, origin, window, cx);
        }
        self.invalidate_canvas_cache();
        cx.notify();
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
        }
        self.comment_state.open_thread = Some(id);
        self.comment_state.open_thread_origin = Some(origin);
        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(1, 4, window, cx);
            editor.set_placeholder_text("Reply…", window, cx);
            editor
        });
        self.comment_state.reply_editor = Some(editor);
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
        let Some(draft) = self.comment_state.draft.as_ref() else {
            return;
        };
        let text = draft.editor.read(cx).text(cx);
        let origin = draft.origin;
        let page = origin.page;
        let world = [draft.world.x, draft.world.y];
        let attachments = self.comment_state.draft_attachments.clone();
        let skill = self.comment_state.draft_skill;
        let has_payload = !text.trim().is_empty() || !attachments.is_empty() || skill.is_some();
        self.comment_state.draft_error = None;
        let outcome = self.item.update(cx, |item, cx| {
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
            match item.apply(operation, cx) {
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
            self.comment_state
                .read_until
                .insert(id.clone(), comments::now_secs());
            self.open_comment_thread_for_origin(id, origin, window, cx);
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
        false
    }

    fn post_comment_reply(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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
        let outcome = self.item.update(cx, |item, cx| {
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
            match item.apply(operation, cx) {
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
        let outcome = self.item.update(cx, |item, cx| {
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
            match item.apply(operation, cx) {
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
        let outcome = self.item.update(cx, |item, cx| {
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
            match item.apply(operation, cx) {
                Ok(()) => CommentMutationOutcome::Applied,
                Err(error) => CommentMutationOutcome::Failed(format!(
                    "Could not delete the thread: {error:#}. The thread, reply draft, and attachments were kept."
                )),
            }
        });
        match outcome {
            CommentMutationOutcome::Applied => self.close_comment_thread_state(),
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
    pub(crate) fn render_comment_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(draft) = &self.comment_state.draft
            && self.origin_is_active(draft.origin, cx)
        {
            return self.render_comment_composer(draft, cx);
        }
        if let (Some(open), Some(origin)) = (
            self.comment_state.open_thread.clone(),
            self.comment_state.open_thread_origin,
        ) && self.origin_is_active(origin, cx)
        {
            let comment = self
                .comments_for_origin(origin, cx)
                .into_iter()
                .find(|comment| comment.id == open)?;
            return self.render_comment_thread(&comment, cx);
        }
        if let Some(hovered) = self.comment_state.hovered_pin.clone() {
            let comment = self
                .page_comments(cx)
                .into_iter()
                .find(|comment| comment.id == hovered)?;
            return self.render_comment_preview(&comment, cx);
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
        let context = self.render_comment_composer_context(CommentComposerTarget::Draft, cx);
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
                .children(context)
                .child(
                    h_flex()
                        .justify_between()
                        .child(self.render_comment_composer_tools(CommentComposerTarget::Draft, cx))
                        .child(
                            IconButton::new("fanta-comment-send", IconName::ArrowUp)
                                .shape(IconButtonShape::Square)
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Post comment"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.post_comment_draft(window, cx);
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_comment_preview(
        &self,
        comment: &Comment,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
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
                        .child(
                            Label::new(author)
                                .size(LabelSize::Small)
                                .weight(gpui::FontWeight::BOLD),
                        )
                        .child(Label::new(when).size(LabelSize::XSmall).color(Color::Muted)),
                )
                .child(Label::new(body).size(LabelSize::Small))
                .children(Self::render_message_context(
                    &comment.mentions,
                    &comment.attachments,
                    comment.skill,
                    cx,
                ))
                .child(
                    Label::new(format!(
                        "{}{}",
                        if comment.resolved { "Resolved" } else { "Open" },
                        if comment.replies.is_empty() {
                            String::new()
                        } else {
                            format!(" · {} replies", comment.replies.len())
                        }
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
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
            &comment.mentions,
            &comment.attachments,
            comment.skill,
            cx,
        ));
        for reply in &comment.replies {
            messages = messages.child(Self::comment_message_row(
                reply.author.clone(),
                relative_time(reply.created),
                reply.body.clone(),
                &reply.mentions,
                &reply.attachments,
                reply.skill,
                cx,
            ));
        }

        let reply_editor = self.comment_state.reply_editor.clone();
        let reply_context = self.render_comment_composer_context(CommentComposerTarget::Reply, cx);
        let reply_tools = self.render_comment_composer_tools(CommentComposerTarget::Reply, cx);
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
                            h_flex()
                                .gap_2()
                                .child(
                                    Label::new("Comment")
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::BOLD),
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
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.resolve_comment_thread(resolve_id.clone(), cx);
                                        },
                                    )),
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
                        v_flex()
                            .px_3p5()
                            .py_2p5()
                            .gap_1p5()
                            .border_t_1()
                            .border_color(cx.theme().colors().border)
                            .child(div().flex_1().child(editor))
                            .children(reply_context)
                            .child(
                                h_flex().justify_between().child(reply_tools).child(
                                    IconButton::new("fanta-comment-reply-send", IconName::ArrowUp)
                                        .shape(IconButtonShape::Square)
                                        .icon_size(IconSize::Small)
                                        .tooltip(Tooltip::text("Reply"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.post_comment_reply(reply_id.clone(), window, cx);
                                        })),
                                ),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_comment_composer_tools(
        &self,
        target: CommentComposerTarget,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (mention_id, attachment_id, skill_id) = match target {
            CommentComposerTarget::Draft => (
                "fanta-comment-draft-mention",
                "fanta-comment-draft-attach",
                "fanta-comment-draft-skill",
            ),
            CommentComposerTarget::Reply => (
                "fanta-comment-reply-mention",
                "fanta-comment-reply-attach",
                "fanta-comment-reply-skill",
            ),
        };
        let selected_skill = self.selected_comment_skill(target);
        h_flex()
            .gap_1()
            .child(
                IconButton::new(mention_id, IconName::AtSign)
                    .icon_size(IconSize::XSmall)
                    .tooltip(Tooltip::text("Mention a layer or agent"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.insert_comment_mention(target, window, cx);
                    })),
            )
            .child(
                IconButton::new(attachment_id, IconName::Paperclip)
                    .icon_size(IconSize::XSmall)
                    .tooltip(Tooltip::text("Attach images or files"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_comment_attachments(target, cx);
                    })),
            )
            .child(
                IconButton::new(skill_id, IconName::Sparkle)
                    .icon_size(IconSize::XSmall)
                    .toggle_state(selected_skill.is_some())
                    .tooltip(Tooltip::text(match selected_skill {
                        Some(skill) => format!("AI skill: {}", skill.label()),
                        None => "Add an AI skill".to_string(),
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_comment_skill_picker(target, cx);
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

        let colors = cx.theme().colors().clone();
        let mut context = v_flex().gap_1();
        if mention_picker_open {
            let mut candidates = h_flex().gap_1().flex_wrap();
            for (index, label) in self.comment_mention_candidates(cx).into_iter().enumerate() {
                let selected_label = label.clone();
                candidates = candidates.child(
                    Button::new((mention_id, index), format!("@{label}"))
                        .size(ButtonSize::Compact)
                        .label_size(LabelSize::XSmall)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_comment_mention(target, selected_label.clone(), window, cx);
                        })),
                );
            }
            context = context
                .child(
                    Label::new("Mention an agent or layer")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(candidates);
        }
        if skill_picker_open {
            let mut skills = h_flex().gap_1().flex_wrap();
            for (index, skill) in CommentSkill::ALL.into_iter().enumerate() {
                skills = skills.child(
                    Button::new((skill_id, index), skill.label())
                        .size(ButtonSize::Compact)
                        .label_size(LabelSize::XSmall)
                        .toggle_state(selected_skill == Some(skill))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_comment_skill(target, Some(skill), cx);
                        })),
                );
            }
            skills = skills.child(
                Button::new((skill_id, CommentSkill::ALL.len()), "No skill")
                    .size(ButtonSize::Compact)
                    .label_size(LabelSize::XSmall)
                    .toggle_state(selected_skill.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_comment_skill(target, None, cx);
                    })),
            );
            context = context
                .child(
                    Label::new("Run in Agent Panel after posting")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(skills);
        } else if let Some(skill) = selected_skill {
            context = context.child(
                h_flex()
                    .px_2()
                    .py_0p5()
                    .gap_1()
                    .rounded_md()
                    .bg(colors.element_active)
                    .child(
                        Icon::new(IconName::Sparkle)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new(format!("{} · opens a reviewed Agent draft", skill.label()))
                            .size(LabelSize::XSmall)
                            .color(Color::Accent),
                    ),
            );
        }
        if let Some(notice) = route_notice {
            let (icon, color, message) = match notice {
                SkillRouteNotice::ReadyForReview => (
                    IconName::Check,
                    Color::Success,
                    "Agent draft opened — review and send it in the Agent Panel.".to_string(),
                ),
                SkillRouteNotice::Failed(error) => (IconName::CircleHelp, Color::Error, error),
            };
            context = context.child(
                h_flex()
                    .px_2()
                    .py_0p5()
                    .gap_1()
                    .rounded_md()
                    .bg(colors.element_active)
                    .child(Icon::new(icon).size(IconSize::XSmall).color(color))
                    .child(Label::new(message).size(LabelSize::XSmall).color(color)),
            );
        }
        if let Some(error) = error {
            context = context.child(
                h_flex()
                    .px_2()
                    .py_0p5()
                    .gap_1()
                    .rounded_md()
                    .bg(cx.theme().status().error.opacity(0.12))
                    .child(
                        Icon::new(IconName::CircleHelp)
                            .size(IconSize::XSmall)
                            .color(Color::Error),
                    )
                    .child(
                        Label::new(error)
                            .size(LabelSize::XSmall)
                            .color(Color::Error),
                    ),
            );
        }
        if !attachments.is_empty() {
            let mut strip = h_flex().id(strip_id).gap_1p5().overflow_x_scroll();
            for (index, attachment) in attachments.into_iter().enumerate() {
                let remove_target = target;
                let mut tile = div()
                    .relative()
                    .flex_none()
                    .w(px(88.))
                    .h(px(66.))
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(colors.border_variant)
                    .bg(colors.editor_background);
                if let Some(path) = attachment
                    .path
                    .as_ref()
                    .filter(|path| is_previewable_image(path))
                {
                    tile = tile.child(img(path.clone()).size_full().object_fit(ObjectFit::Cover));
                } else {
                    tile = tile.child(
                        v_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .px_1()
                            .gap_1()
                            .child(
                                Icon::new(IconName::File)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(attachment.name.clone())
                                    .size(LabelSize::XSmall)
                                    .single_line()
                                    .truncate(),
                            ),
                    );
                }
                strip = strip.child(
                    tile.child(
                        div().absolute().top_0().right_0().child(
                            IconButton::new((remove_id, index), IconName::Close)
                                .icon_size(IconSize::XSmall)
                                .tooltip(Tooltip::text("Remove attachment"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_comment_attachment(remove_target, index, cx);
                                })),
                        ),
                    ),
                );
            }
            context = context.child(strip);
        }
        Some(context.into_any_element())
    }

    fn render_message_context(
        mentions: &[Mention],
        attachments: &[Attachment],
        skill: Option<CommentSkill>,
        cx: &App,
    ) -> Option<AnyElement> {
        if mentions.is_empty() && attachments.is_empty() && skill.is_none() {
            return None;
        }
        let colors = cx.theme().colors().clone();
        let mut context = v_flex().gap_1p5();
        if !mentions.is_empty() || skill.is_some() {
            let mut chips = h_flex().gap_1().flex_wrap();
            for mention in mentions {
                chips = chips.child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded_sm()
                        .bg(colors.element_active)
                        .child(
                            Label::new(format!("@{}", mention.label))
                                .size(LabelSize::XSmall)
                                .color(Color::Accent),
                        ),
                );
            }
            if let Some(skill) = skill {
                chips = chips.child(
                    h_flex()
                        .px_1p5()
                        .py_0p5()
                        .gap_1()
                        .rounded_sm()
                        .bg(colors.element_active)
                        .child(
                            Icon::new(IconName::Sparkle)
                                .size(IconSize::XSmall)
                                .color(Color::Accent),
                        )
                        .child(
                            Label::new(skill.label())
                                .size(LabelSize::XSmall)
                                .color(Color::Accent),
                        ),
                );
            }
            context = context.child(chips);
        }
        for attachment in attachments {
            let attachment_element = match attachment
                .path
                .as_ref()
                .filter(|path| is_previewable_image(path))
            {
                Some(path) => div()
                    .w_full()
                    .h(px(132.))
                    .rounded_lg()
                    .overflow_hidden()
                    .border_1()
                    .border_color(colors.border_variant)
                    .child(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
                    .into_any_element(),
                None => h_flex()
                    .px_2()
                    .py_1p5()
                    .gap_1p5()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border_variant)
                    .child(
                        Icon::new(IconName::File)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(attachment.name.clone())
                            .size(LabelSize::XSmall)
                            .single_line()
                            .truncate(),
                    )
                    .into_any_element(),
            };
            context = context.child(attachment_element);
        }
        Some(context.into_any_element())
    }

    fn comment_message_row(
        author: String,
        when: String,
        body: String,
        mentions: &[Mention],
        attachments: &[Attachment],
        skill: Option<CommentSkill>,
        cx: &App,
    ) -> gpui::Div {
        v_flex()
            .gap_1()
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
            .when(!body.trim().is_empty(), |this| {
                this.child(Label::new(SharedString::from(body)).size(LabelSize::Small))
            })
            .children(Self::render_message_context(
                mentions,
                attachments,
                skill,
                cx,
            ))
    }
}

fn display_author(comment: &Comment) -> String {
    if comment.author.trim().is_empty() {
        "you".to_string()
    } else {
        comment.author.clone()
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
        Attachment, AttachmentPickerOrigin, CommentMutationOutcome, CommentOrigin,
        CommentSubmitOutcome, document_has_comment_origin,
    };

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
}
