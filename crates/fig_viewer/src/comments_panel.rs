use std::rc::Rc;

use gpui::{App, IntoElement, ObjectFit, RenderOnce, SharedString, Window, div, img};
use ui::prelude::*;

use crate::comments;
use crate::document::FigPage;
use crate::inspector_components::{InspectorMessage, InspectorSectionHeader};

#[derive(Debug, Clone)]
pub(crate) struct DocumentCommentRow {
    page_root: fanta_doc::NodeId,
    page_name: SharedString,
    id: String,
    author: SharedString,
    body: SharedString,
    replies: usize,
    attachments: Vec<comments::Attachment>,
    mentions: usize,
    skill: Option<SharedString>,
    resolved: bool,
    motion_label: Option<SharedString>,
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
            let reply_attachments = comment
                .replies
                .iter()
                .flat_map(|reply| reply.attachments.iter().cloned());
            let reply_mentions = comment
                .replies
                .iter()
                .map(|reply| reply.mentions.len())
                .sum::<usize>();
            let skill = comment
                .skill
                .or_else(|| comment.replies.iter().rev().find_map(|reply| reply.skill));
            rows.push(DocumentCommentRow {
                page_root,
                page_name: page.name.clone(),
                id: comment.id,
                author: if comment.author.trim().is_empty() {
                    "Unknown".into()
                } else {
                    comment.author.into()
                },
                body: comment.text.into(),
                replies: comment.replies.len(),
                attachments: comment
                    .attachments
                    .into_iter()
                    .chain(reply_attachments)
                    .collect(),
                mentions: comment.mentions.len() + reply_mentions,
                skill: skill.map(|skill| skill.label().into()),
                resolved: comment.resolved,
                motion_label: comment
                    .motion_anchor
                    .map(|anchor| comments::motion_comment_label(doc, anchor).into()),
            });
        }
    }
    rows
}

type OpenHandler = Rc<dyn Fn(fanta_doc::NodeId, String, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct FantaCommentsPanel {
    rows: Vec<DocumentCommentRow>,
    on_open: OpenHandler,
}

impl FantaCommentsPanel {
    pub(crate) fn new(
        rows: Vec<DocumentCommentRow>,
        on_open: impl Fn(fanta_doc::NodeId, String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            rows,
            on_open: Rc::new(on_open),
        }
    }
}

impl RenderOnce for FantaCommentsPanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.rows.is_empty() {
            return div()
                .size_full()
                .child(InspectorSectionHeader::new("Comments"))
                .child(InspectorMessage::new("This document has no comments yet."))
                .into_any_element();
        }

        let open_count = self.rows.iter().filter(|row| !row.resolved).count();
        let resolved_count = self.rows.len().saturating_sub(open_count);
        let mut list = v_flex()
            .id("fanta-document-comments")
            .size_full()
            .overflow_y_scroll()
            .child(InspectorSectionHeader::new("Comments"))
            .child(
                h_flex()
                    .px_3()
                    .pb_2()
                    .gap_2()
                    .child(
                        Label::new(format!("{open_count} open"))
                            .size(LabelSize::XSmall)
                            .color(Color::Default),
                    )
                    .child(
                        Label::new(format!("{resolved_count} resolved"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );

        for (index, row) in self.rows.into_iter().enumerate() {
            let on_open = self.on_open.clone();
            let comment_id = row.id.clone();
            let page_root = row.page_root;
            let motion_label = row.motion_label.clone();
            list = list.child(
                v_flex()
                    .id(("fanta-document-comment", index))
                    .mx_2()
                    .mb_1()
                    .p_2()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .border_1()
                    .border_color(cx.theme().colors().border_variant)
                    .hover(|row| row.bg(cx.theme().colors().element_hover))
                    .on_click(move |_, window, cx| {
                        on_open(page_root, comment_id.clone(), window, cx)
                    })
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        Icon::new(if row.resolved {
                                            IconName::Check
                                        } else {
                                            IconName::Chat
                                        })
                                        .size(IconSize::XSmall)
                                        .color(
                                            if row.resolved {
                                                Color::Success
                                            } else {
                                                Color::Accent
                                            },
                                        ),
                                    )
                                    .child(
                                        Label::new(row.page_name)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .single_line(),
                                    )
                                    .when_some(motion_label, |this, label| {
                                        this.child(
                                            Label::new(label)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Accent),
                                        )
                                    }),
                            )
                            .child(
                                Label::new(if row.resolved { "Resolved" } else { "Open" })
                                    .size(LabelSize::XSmall)
                                    .color(if row.resolved {
                                        Color::Muted
                                    } else {
                                        Color::Accent
                                    }),
                            ),
                    )
                    .when(!row.body.is_empty(), |this| {
                        this.child(Label::new(row.body).size(LabelSize::Small))
                    })
                    .when_some(
                        row.attachments.iter().find_map(|attachment| {
                            attachment
                                .path
                                .as_ref()
                                .filter(|path| previewable_image(path))
                                .cloned()
                        }),
                        |this, path| {
                            this.child(
                                div()
                                    .w_full()
                                    .h(px(112.))
                                    .rounded_md()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(cx.theme().colors().border_variant)
                                    .child(img(path).size_full().object_fit(ObjectFit::Cover)),
                            )
                        },
                    )
                    .child(
                        Label::new(format!(
                            "{}{}{}{}",
                            row.author,
                            if row.replies == 0 {
                                String::new()
                            } else {
                                format!(" · {} replies", row.replies)
                            },
                            if row.attachments.is_empty() {
                                String::new()
                            } else {
                                format!(" · {} attachments", row.attachments.len())
                            },
                            match (row.mentions, row.skill.as_deref()) {
                                (0, None) => String::new(),
                                (mentions, None) => format!(" · {mentions} mentions"),
                                (0, Some(skill)) => format!(" · AI: {skill}"),
                                (mentions, Some(skill)) => {
                                    format!(" · {mentions} mentions · AI: {skill}")
                                }
                            },
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            );
        }
        list.into_any_element()
    }
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

    use super::*;

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
        assert_eq!(rows[0].attachments.len(), 1);
        assert_eq!(rows[0].mentions, 1);
        assert_eq!(rows[0].page_name.as_ref(), "First");
        assert_eq!(rows[0].page_root, first_id);
        assert_eq!(
            rows[0].motion_label.as_deref(),
            Some("Animation unavailable · 00:01.250")
        );
        assert_eq!(rows[1].page_name.as_ref(), "Second");
        assert_eq!(rows[1].page_root, second_id);
    }
}
