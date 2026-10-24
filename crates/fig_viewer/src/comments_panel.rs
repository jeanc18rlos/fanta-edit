use std::path::PathBuf;

use gpui::{
    AnyElement, App, Context, Entity, IntoElement, ListAlignment, ListState, ObjectFit, Render,
    SharedString, Subscription, WeakEntity, Window, div, img, list, px,
};
use ui::prelude::*;
use ui::{ScrollAxes, Scrollbars, Tooltip, WithScrollbar};
use util::ResultExt as _;

use crate::comments;
use crate::comments_ui::{CommentAvatar, comment_author_label, relative_time};
use crate::document::{FigItem, FigItemEvent, FigPage};
use crate::view::FigView;

#[derive(Debug, Clone)]
pub(crate) struct DocumentCommentRow {
    page_root: fanta_doc::NodeId,
    page_name: SharedString,
    location: SharedString,
    id: String,
    author: SharedString,
    from_agent: bool,
    created: u64,
    body: SharedString,
    replies: usize,
    attachments: usize,
    image: Option<PathBuf>,
    mentions: usize,
    skill: Option<SharedString>,
    resolved: bool,
    motion_label: Option<SharedString>,
}

impl DocumentCommentRow {
    fn details(&self) -> SharedString {
        let mut details = vec![self.location.to_string()];
        if let Some(motion) = &self.motion_label {
            details.push(motion.to_string());
        }
        let mut count = |count: usize, singular: &str, plural: &str| match count {
            0 => {}
            1 => details.push(format!("1 {singular}")),
            count => details.push(format!("{count} {plural}")),
        };
        count(self.replies, "reply", "replies");
        count(self.attachments, "attachment", "attachments");
        count(self.mentions, "mention", "mentions");
        if let Some(skill) = &self.skill {
            details.push(format!("AI: {skill}"));
        }
        details.join(" • ").into()
    }
}

pub(crate) fn document_comment_rows(
    doc: &fanta_doc::Doc,
    pages: &[FigPage],
) -> Vec<DocumentCommentRow> {
    let mut rows = Vec::new();
    for page in pages {
        let Some(page_root) = page.root else {
            continue;
        };
        for comment in comments::read_comments(doc, page_root) {
            let reply_mentions = comment
                .replies
                .iter()
                .map(|reply| reply.mentions.len())
                .sum::<usize>();
            let attachments = comment
                .attachments
                .iter()
                .chain(comment.replies.iter().flat_map(|reply| &reply.attachments));
            let image = attachments
                .clone()
                .find_map(|attachment| {
                    attachment
                        .path
                        .as_ref()
                        .filter(|path| previewable_image(path))
                })
                .cloned();
            let skill = comment
                .skill
                .or_else(|| comment.replies.iter().rev().find_map(|reply| reply.skill));
            rows.push(DocumentCommentRow {
                page_root,
                page_name: page.name.clone(),
                location: comment.location_label(doc, &page.name).into(),
                author: comment_author_label(&comment.author),
                from_agent: comment.from_agent,
                created: comment.created,
                body: comment.text.trim().to_string().into(),
                replies: comment.replies.len(),
                attachments: attachments.count(),
                image,
                mentions: comment.mentions.len() + reply_mentions,
                skill: skill.map(|skill| skill.label().into()),
                resolved: comment.resolved,
                motion_label: comment
                    .motion_anchor
                    .map(|anchor| comments::motion_comment_label(doc, anchor).into()),
                id: comment.id,
            });
        }
    }
    rows
}

enum CommentsPanelEntry {
    Page { name: SharedString, count: usize },
    Comment(usize),
}

/// The Comments sidebar: every thread in the document, grouped by page, each
/// row showing the message that opened it. Opening a row shows the thread's
/// page, which `FigView` renders in this panel's place. Rows are rebuilt once
/// after each document change and rendered through a virtualized list, so a
/// redraw (one per keystroke in a canvas composer) only lays out the rows
/// that show.
pub(crate) struct CommentsPanel {
    item: Entity<FigItem>,
    view: WeakEntity<FigView>,
    rows: Vec<DocumentCommentRow>,
    /// Set by document events; the next render rebuilds the rows, so a hidden
    /// panel does no work while the canvas is edited.
    rows_stale: bool,
    entries: Vec<CommentsPanelEntry>,
    show_resolved: bool,
    list_state: ListState,
    _item_subscription: Subscription,
}

impl CommentsPanel {
    pub(crate) fn new(
        item: Entity<FigItem>,
        view: WeakEntity<FigView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let item_subscription = cx.subscribe(&item, |this, _, event: &FigItemEvent, cx| {
            if matches!(
                event,
                FigItemEvent::Edited
                    | FigItemEvent::StateChanged
                    | FigItemEvent::PageRegistryChanged
                    | FigItemEvent::ReloadedFromDisk { .. }
            ) {
                this.rows_stale = true;
                cx.notify();
            }
        });
        Self {
            item,
            view,
            rows: Vec::new(),
            rows_stale: true,
            entries: Vec::new(),
            show_resolved: true,
            list_state: ListState::new(0, ListAlignment::Top, px(240.)),
            _item_subscription: item_subscription,
        }
    }

    fn rebuild_rows(&mut self, cx: &App) {
        self.rows = self
            .item
            .read(cx)
            .document()
            .map(|document| document_comment_rows(&document.doc, &document.pages))
            .unwrap_or_default();
        self.rows_stale = false;
        self.rebuild_entries();
    }

    fn rebuild_entries(&mut self) {
        let visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| self.show_resolved || !row.resolved)
            .collect::<Vec<_>>();
        // Page headers only help once threads span more than one page.
        let grouped = visible
            .first()
            .zip(visible.last())
            .is_some_and(|((_, first), (_, last))| first.page_root != last.page_root);
        let mut entries = Vec::with_capacity(visible.len());
        for group in visible.chunk_by(|(_, left), (_, right)| left.page_root == right.page_root) {
            if grouped && let Some((_, first)) = group.first() {
                entries.push(CommentsPanelEntry::Page {
                    name: first.page_name.clone(),
                    count: group.len(),
                });
            }
            entries.extend(
                group
                    .iter()
                    .map(|(index, _)| CommentsPanelEntry::Comment(*index)),
            );
        }
        // Splicing (not resetting) keeps the scroll position across edits.
        self.list_state
            .splice(0..self.list_state.item_count(), entries.len());
        self.entries = entries;
    }

    fn toggle_show_resolved(&mut self, cx: &mut Context<Self>) {
        self.show_resolved = !self.show_resolved;
        self.rebuild_entries();
        cx.notify();
    }

    fn open_thread(&self, cx: &App) -> Option<(fanta_doc::NodeId, String)> {
        self.view
            .upgrade()
            .and_then(|view| view.read(cx).open_comment_thread())
    }

    fn render_entry(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.entries.get(index) {
            Some(CommentsPanelEntry::Page { name, count }) => h_flex()
                .w_full()
                .min_w_0()
                .px_2p5()
                .pt_3()
                .pb_1()
                .gap_1p5()
                .child(
                    div().min_w_0().child(
                        Label::new(name.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .single_line()
                            .truncate(),
                    ),
                )
                .child(
                    Label::new(count.to_string())
                        .size(LabelSize::Small)
                        .color(Color::Disabled),
                )
                .into_any_element(),
            Some(CommentsPanelEntry::Comment(row)) => match self.rows.get(*row) {
                Some(row) => {
                    let selected = self
                        .open_thread(cx)
                        .is_some_and(|(page, id)| page == row.page_root && id == row.id);
                    self.render_row(index, row, selected, cx)
                }
                None => div().into_any_element(),
            },
            None => div().into_any_element(),
        }
    }

    /// One thread, laid out like Zed's thread list rows: the author and time
    /// on the first line, then the body and its details, indented under the
    /// avatar. List items are sized to their content unless the row asks
    /// for the full width, which is what keeps text wrapping and truncating
    /// inside the panel.
    fn render_row(
        &self,
        index: usize,
        row: &DocumentCommentRow,
        selected: bool,
        cx: &App,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let view = self.view.clone();
        let page_root = row.page_root;
        let comment_id = row.id.clone();
        div()
            .w_full()
            .px_1()
            .py_px()
            .child(
                v_flex()
                    .id(("fanta-document-comment", index))
                    .debug_selector(move || format!("fanta-document-comment-{index}"))
                    .w_full()
                    .min_w_0()
                    .px_1p5()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |row| row.bg(colors.element_selected))
                    .when(!selected, |row| {
                        row.hover(|row| row.bg(colors.element_hover))
                    })
                    .on_click(move |_, window, cx| {
                        view.update(cx, |view, cx| {
                            view.show_comment_thread_in_panel(
                                page_root,
                                comment_id.clone(),
                                window,
                                cx,
                            );
                        })
                        .log_err();
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .h_6()
                            .gap_2()
                            .child(CommentAvatar::new(row.author.clone(), row.from_agent))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .debug_selector(move || {
                                        format!("fanta-comment-row-author-{index}")
                                    })
                                    .child(Label::new(row.author.clone()).single_line().truncate()),
                            )
                            .when(row.resolved, |line| {
                                line.child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::Small)
                                        .color(Color::Success),
                                )
                            })
                            .child(
                                div().flex_none().child(
                                    Label::new(relative_time(row.created))
                                        .size(LabelSize::Small)
                                        .color(Color::Muted)
                                        .single_line(),
                                ),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_start()
                            .gap_2()
                            .pl_7()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_0p5()
                                    .when(!row.body.is_empty(), |content| {
                                        content.child(
                                            div()
                                                .w_full()
                                                .min_w_0()
                                                .debug_selector(move || {
                                                    format!("fanta-comment-row-body-{index}")
                                                })
                                                .child(
                                                    Label::new(row.body.clone())
                                                        .color(if row.resolved {
                                                            Color::Muted
                                                        } else {
                                                            Color::Default
                                                        })
                                                        .line_clamp(3),
                                                ),
                                        )
                                    })
                                    .child(
                                        div().w_full().min_w_0().child(
                                            Label::new(row.details())
                                                .size(LabelSize::Small)
                                                .color(Color::Muted)
                                                .single_line()
                                                .truncate(),
                                        ),
                                    ),
                            )
                            .when_some(row.image.clone(), |content, path| {
                                content.child(
                                    div()
                                        .flex_none()
                                        .size(rems(2.5))
                                        .mt_0p5()
                                        .rounded_sm()
                                        .overflow_hidden()
                                        .border_1()
                                        .border_color(colors.border_variant)
                                        .child(img(path).size_full().object_fit(ObjectFit::Cover)),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for CommentsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.rows_stale {
            self.rebuild_rows(cx);
        }
        let resolved = self.rows.iter().filter(|row| row.resolved).count();
        let open = self.rows.len() - resolved;
        let summary = match (open, resolved) {
            (open, 0) => format!("{open} open"),
            (open, resolved) => format!("{open} open • {resolved} resolved"),
        };
        let header = h_flex()
            .w_full()
            .h_8()
            .flex_none()
            .pl_2p5()
            .pr_1()
            .gap_2()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(if self.rows.is_empty() {
                        "No threads".to_string()
                    } else {
                        summary
                    })
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .single_line()
                    .truncate(),
                ),
            )
            .child(
                div()
                    .debug_selector(|| "fanta-comments-show-resolved".to_owned())
                    .child(
                        IconButton::new("fanta-comments-show-resolved", IconName::CheckDouble)
                            .icon_size(IconSize::Small)
                            .icon_color(Color::Muted)
                            .toggle_state(self.show_resolved)
                            .disabled(resolved == 0)
                            .tooltip(Tooltip::text(if self.show_resolved {
                                "Hide Resolved Threads"
                            } else {
                                "Show Resolved Threads"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_show_resolved(cx))),
                    ),
            );
        let body = if self.rows.is_empty() {
            comments_empty_state(
                "No comments yet",
                "Press C, then click the canvas to start a thread.",
            )
        } else if self.entries.is_empty() {
            comments_empty_state(
                "No open threads",
                "Every thread is resolved. Show resolved threads to read them again.",
            )
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .pb_2()
                .overflow_hidden()
                .child(
                    list(self.list_state.clone(), cx.processor(Self::render_entry))
                        .flex_1()
                        .size_full(),
                )
                .custom_scrollbars(
                    Scrollbars::new(ScrollAxes::Vertical).tracked_scroll_handle(&self.list_state),
                    window,
                    cx,
                )
                .into_any_element()
        };
        v_flex()
            .id("fanta-document-comments")
            .debug_selector(|| "fanta-document-comments".to_owned())
            .size_full()
            .min_w_0()
            .child(header)
            .child(body)
    }
}

fn comments_empty_state(title: &'static str, message: &'static str) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_1()
        .px_6()
        .text_center()
        .child(Label::new(title))
        .child(
            Label::new(message)
                .size(LabelSize::Small)
                .color(Color::Muted),
        )
        .into_any_element()
}

fn previewable_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "avif" | "gif" | "jpeg" | "jpg" | "png" | "webp"
            )
        })
}

#[cfg(test)]
mod tests {
    use fanta_doc::{CanvasNode, GroupNode, NodeData, Operation};
    use gpui::Focusable as _;

    use super::*;

    #[gpui::test]
    async fn panel_rows_open_thread_pages_across_pages_and_hide_resolved(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::editor_session::EditorMode;
        use gpui::size;
        use project::{FakeFs, Project};

        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = fanta_doc::Doc::new();
        let mut page_ids = Vec::new();
        for name in ["First", "Second"] {
            let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
            page.name = name.into();
            page_ids.push(page.id);
            doc.apply(Operation::create_node(page)).expect("page");
            doc.add_page(*page_ids.last().expect("page id"));
        }
        let (first_page, second_page) = (page_ids[0], page_ids[1]);
        doc.set_active_page(Some(first_page));
        for text in [
            "A long comment body that has to wrap inside the panel instead of running past its edge, so it stays readable.",
            "Resolved on first",
        ] {
            let (_, add) =
                comments::add_comment_op(&doc, first_page, [0.0, 0.0], text).expect("comment");
            doc.apply(add).expect("add comment");
        }
        let resolved = comments::read_comments(&doc, first_page)[1].id.clone();
        let resolve =
            comments::toggle_resolved_op(&doc, first_page, &resolved).expect("resolve comment");
        doc.apply(resolve).expect("apply resolve");
        let (second_comment, add) =
            comments::add_comment_op(&doc, second_page, [10.0, 10.0], "Open on second")
                .expect("comment");
        doc.apply(add).expect("add comment");
        let reply = comments::reply_comment_full_op(
            &doc,
            second_page,
            &second_comment,
            "First reply",
            Vec::new(),
            Vec::new(),
            None,
        )
        .expect("reply");
        doc.apply(reply).expect("add reply");
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            "/tmp/Comments-panel.fig".into(),
            doc,
            cx,
        );
        let (view, cx) = cx.add_window_view(|window, cx| {
            crate::view::FigView::new(item.clone(), project, window, cx)
        });
        cx.simulate_resize(size(px(1400.), px(1000.)));
        view.update(cx, |view, cx| {
            view.set_editor_mode(EditorMode::Comments, cx)
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.run_until_parked();

        // Page headers sit at 0 and 3; the second page's thread is entry 4.
        for selector in [
            "fanta-document-comment-1",
            "fanta-document-comment-2",
            "fanta-document-comment-4",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "{selector} renders");
        }
        let panel = cx
            .debug_bounds("fanta-document-comments")
            .expect("comments panel");
        let author = cx
            .debug_bounds("fanta-comment-row-author-1")
            .expect("row author");
        let body = cx
            .debug_bounds("fanta-comment-row-body-1")
            .expect("row body");
        assert!(
            author.size.width > px(30.),
            "the author name is readable, not collapsed: {author:?}"
        );
        assert!(
            body.right() <= panel.right() && body.left() >= panel.left(),
            "the body wraps inside the panel: {body:?} in {panel:?}"
        );
        assert!(
            body.size.height > px(30.),
            "a long body wraps onto more than one line: {body:?}"
        );
        let row = cx
            .debug_bounds("fanta-document-comment-4")
            .expect("second page row");
        cx.simulate_click(row.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.open_comment_thread(),
                Some((second_page, second_comment.clone()))
            );
            assert_eq!(
                item.read(cx).doc().and_then(|doc| doc.active_page()),
                Some(second_page)
            );
        });

        // The thread opens as a page of the panel, not as a canvas popover.
        cx.update(|window, cx| window.draw(cx).clear());
        let page = cx
            .debug_bounds("fanta-comment-thread-page")
            .expect("the thread opens as a panel page");
        assert!(cx.debug_bounds("fanta-document-comments").is_none());
        assert!(cx.debug_bounds("fanta-comment-thread").is_none());
        for selector in [
            "fanta-comment-thread-page-message-0",
            "fanta-comment-thread-page-message-1",
        ] {
            let message = cx.debug_bounds(selector).expect(selector);
            assert!(
                message.left() >= page.left() && message.right() <= page.right(),
                "{selector} fits the page: {message:?} in {page:?}"
            );
        }

        let reply_editor = view
            .read_with(cx, |view, _| view.comment_state.reply_editor.clone())
            .expect("reply composer");
        cx.update(|window, cx| reply_editor.read(cx).focus_handle(cx).focus(window, cx));
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_input("Second reply");
        cx.dispatch_action(crate::view::SubmitComment);
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            let thread = comments::read_comments(doc, second_page)
                .into_iter()
                .find(|comment| comment.id == second_comment)
                .expect("thread");
            assert_eq!(
                thread
                    .replies
                    .iter()
                    .map(|reply| reply.body.as_str())
                    .collect::<Vec<_>>(),
                ["First reply", "Second reply"]
            );
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(
            cx.debug_bounds("fanta-comment-thread-page-message-2")
                .is_some()
        );

        let back = cx
            .debug_bounds("fanta-comment-thread-back")
            .expect("back button");
        cx.simulate_click(back.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, _| assert_eq!(view.open_comment_thread(), None));
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("fanta-comment-thread-page").is_none());
        assert!(cx.debug_bounds("fanta-document-comments").is_some());

        // A pin click keeps the thread on the canvas.
        view.update_in(cx, |view, window, cx| {
            view.toggle_comment_thread(second_comment.clone(), window, cx)
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("fanta-comment-thread").is_some());
        assert!(cx.debug_bounds("fanta-comment-thread-page").is_none());
        view.update_in(cx, |view, window, cx| {
            view.toggle_comment_thread(second_comment.clone(), window, cx)
        });
        cx.update(|window, cx| window.draw(cx).clear());

        let toggle = cx
            .debug_bounds("fanta-comments-show-resolved")
            .expect("resolved toggle");
        cx.simulate_click(toggle.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear());
        // Without the resolved thread: header, first, header, second.
        assert!(cx.debug_bounds("fanta-document-comment-3").is_some());
        assert!(cx.debug_bounds("fanta-document-comment-4").is_none());
    }

    #[test]
    fn snapshot_collects_open_and_resolved_comments_across_pages() {
        let mut doc = fanta_doc::Doc::new();
        let mut first = CanvasNode::new(NodeData::Group(GroupNode::default()));
        first.name = "First".into();
        let first_id = first.id;
        doc.apply(Operation::create_node(first))
            .expect("first page");
        doc.add_page(first_id);
        let motion_anchor = comments::MotionCommentAnchor {
            clip: fanta_doc::AnimationClipId::from_u128(7),
            time_ms: 1_250,
        };
        let (first_comment_id, add) = comments::add_comment_full_op(
            &doc,
            first_id,
            [1.0, 2.0],
            "Open",
            Vec::new(),
            Vec::new(),
            None,
            Some(motion_anchor),
        )
        .expect("open comment");
        doc.apply(add).expect("add open comment");
        doc.apply(
            comments::reply_comment_full_op(
                &doc,
                first_id,
                &first_comment_id,
                "@Codex see the image",
                comments::mentions_for_body(&doc, first_id, "@Codex see the image"),
                vec![comments::Attachment {
                    name: "reference.png".to_string(),
                    path: Some("/tmp/reference.png".into()),
                }],
                Some(comments::CommentSkill::Search),
            )
            .expect("rich reply"),
        )
        .expect("apply rich reply");

        let mut second = CanvasNode::new(NodeData::Group(GroupNode::default()));
        second.name = "Second".into();
        let second_id = second.id;
        doc.apply(Operation::create_node(second))
            .expect("second page");
        doc.add_page(second_id);
        let (resolved_id, add) = comments::add_comment_op(&doc, second_id, [3.0, 4.0], "Done")
            .expect("resolved comment");
        doc.apply(add).expect("add resolved comment");
        let resolve =
            comments::toggle_resolved_op(&doc, second_id, &resolved_id).expect("resolve comment");
        doc.apply(resolve).expect("apply resolve");

        let pages = vec![
            FigPage {
                root: Some(first_id),
                name: "First".into(),
                bounds: fanta_doc::Bounds::ZERO,
                hidden: false,
            },
            FigPage {
                root: Some(second_id),
                name: "Second".into(),
                bounds: fanta_doc::Bounds::ZERO,
                hidden: false,
            },
        ];
        let rows = document_comment_rows(&doc, &pages);
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].resolved);
        assert!(rows[1].resolved);
        assert_eq!(rows[0].replies, 1);
        assert_eq!(rows[0].attachments, 1);
        assert_eq!(
            rows[0].image.as_deref(),
            Some("/tmp/reference.png".as_ref())
        );
        assert_eq!(rows[0].mentions, 1);
        assert_eq!(rows[0].skill.as_deref(), Some("Search"));
        assert_eq!(rows[0].location.as_ref(), "First");
        assert_eq!(rows[0].page_root, first_id);
        assert_eq!(
            rows[0].motion_label.as_deref(),
            Some("Animation unavailable · 00:01.250")
        );
        assert_eq!(
            rows[0].details().as_ref(),
            "First • Animation unavailable · 00:01.250 • 1 reply • 1 attachment • 1 mention • AI: Search"
        );
        assert_eq!(rows[1].location.as_ref(), "Second");
        assert_eq!(rows[1].page_root, second_id);
    }
}
