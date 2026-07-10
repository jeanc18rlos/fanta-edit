//! Canvas comments — Figma-style numbered pins anchored at world positions.
//!
//! Comments are presence-adjacent ANNOTATION data, not scene content: they
//! never render in exports and live outside the node tree. They are stored on
//! the page node's open [`meta`] blob (the engine's designated extension slot)
//! under the `"comments"` key, and every mutation goes through one undoable
//! [`Operation::SetMeta`] — so comments persist with the project (`meta`
//! round-trips through `.fnx`), travel with Cmd-S, and undo like any edit.
//!
//! [`meta`]: fanta_doc::CanvasNode::meta

use fanta_doc::{Doc, NodeId, Operation};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const META_KEY: &str = "comments";

/// What an `@label` in a comment body points at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub(crate) enum MentionTarget {
    Node(NodeId),
    Agent(String),
    /// Unresolved free-text handle; kept so it round-trips.
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Mention {
    /// The body contains the literal `@{label}`.
    pub(crate) label: String,
    pub(crate) target: MentionTarget,
}

/// A path reference only — bytes are never embedded in the page meta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Attachment {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CommentReply {
    pub(crate) author: String,
    pub(crate) body: String,
    /// Unix seconds.
    pub(crate) created: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) from_agent: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) mentions: Vec<Mention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Comment {
    /// Stable identity for panel rows and pin hit-testing.
    pub(crate) id: String,
    /// World-space anchor of the pin.
    pub(crate) world: [f64; 2],
    #[serde(default)]
    pub(crate) author: String,
    pub(crate) text: String,
    /// Unix seconds.
    #[serde(default)]
    pub(crate) created: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) resolved: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) from_agent: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) mentions: Vec<Mention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) replies: Vec<CommentReply>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) question: bool,
}

impl Comment {
    pub(crate) fn message_count(&self) -> usize {
        1 + self.replies.len()
    }

    /// The newest activity in the thread, for unread tracking and sorting.
    pub(crate) fn newest_created(&self) -> u64 {
        self.replies
            .iter()
            .map(|reply| reply.created)
            .fold(self.created, u64::max)
    }
}

/// The commenting identity: the OS user, like the original app.
pub(crate) fn author_name() -> String {
    std::env::var("USER").unwrap_or_else(|_| "you".into())
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// The page's comments, oldest first (pin numbers are 1-based list positions).
pub(crate) fn read_comments(doc: &Doc, page: NodeId) -> Vec<Comment> {
    doc.scene
        .get(page)
        .and_then(|node| node.meta.get(META_KEY))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// One undoable [`Operation::SetMeta`] replacing the page's comment list while
/// preserving every unrelated meta key. `None` when the page is gone or the
/// list is unchanged.
fn set_comments_op(doc: &Doc, page: NodeId, comments: &[Comment]) -> Option<Operation> {
    let node = doc.scene.get(page)?;
    let old = node.meta.clone();
    let mut new = match &old {
        serde_json::Value::Object(map) => serde_json::Value::Object(map.clone()),
        _ => serde_json::json!({}),
    };
    let list = serde_json::to_value(comments).ok()?;
    if comments.is_empty() {
        new.as_object_mut()?.remove(META_KEY);
    } else {
        new.as_object_mut()?.insert(META_KEY.to_string(), list);
    }
    (new != old).then_some(Operation::SetMeta {
        id: page,
        old,
        new,
    })
}

pub(crate) fn add_comment_op(
    doc: &Doc,
    page: NodeId,
    world: [f64; 2],
    text: &str,
) -> Option<(String, Operation)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut comments = read_comments(doc, page);
    let id = fanta_doc::NodeId::new().to_string();
    comments.push(Comment {
        id: id.clone(),
        world,
        author: author_name(),
        text: text.to_string(),
        created: now_secs(),
        resolved: false,
        from_agent: false,
        mentions: Vec::new(),
        attachments: Vec::new(),
        replies: Vec::new(),
        question: false,
    });
    set_comments_op(doc, page, &comments).map(|op| (id, op))
}

/// Append a reply to a thread. Blank replies (empty trimmed body) are dropped.
pub(crate) fn reply_comment_op(
    doc: &Doc,
    page: NodeId,
    id: &str,
    body: &str,
) -> Option<Operation> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    let mut comments = read_comments(doc, page);
    let comment = comments.iter_mut().find(|comment| comment.id == id)?;
    comment.replies.push(CommentReply {
        author: author_name(),
        body: body.to_string(),
        created: now_secs(),
        from_agent: false,
        mentions: Vec::new(),
        attachments: Vec::new(),
    });
    set_comments_op(doc, page, &comments)
}

pub(crate) fn update_comment_text_op(
    doc: &Doc,
    page: NodeId,
    id: &str,
    text: &str,
) -> Option<Operation> {
    let mut comments = read_comments(doc, page);
    let comment = comments.iter_mut().find(|comment| comment.id == id)?;
    comment.text = text.to_string();
    set_comments_op(doc, page, &comments)
}

pub(crate) fn toggle_resolved_op(doc: &Doc, page: NodeId, id: &str) -> Option<Operation> {
    let mut comments = read_comments(doc, page);
    let comment = comments.iter_mut().find(|comment| comment.id == id)?;
    comment.resolved = !comment.resolved;
    set_comments_op(doc, page, &comments)
}

pub(crate) fn remove_comment_op(doc: &Doc, page: NodeId, id: &str) -> Option<Operation> {
    let mut comments = read_comments(doc, page);
    let before = comments.len();
    comments.retain(|comment| comment.id != id);
    (comments.len() != before)
        .then(|| set_comments_op(doc, page, &comments))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, NodeData};

    fn doc_with_page() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(id);
        (doc, id)
    }

    #[test]
    fn add_edit_resolve_remove_round_trip_through_set_meta() {
        let (mut doc, page) = doc_with_page();
        assert!(read_comments(&doc, page).is_empty());

        let (id, op) = add_comment_op(&doc, page, [10.0, 20.0], "first").expect("add");
        doc.apply(op).expect("apply add");
        let comments = read_comments(&doc, page);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "first");
        assert_eq!(comments[0].world, [10.0, 20.0]);
        assert!(!comments[0].resolved);

        doc.apply(update_comment_text_op(&doc, page, &id, "edited").expect("edit"))
            .expect("apply edit");
        assert_eq!(read_comments(&doc, page)[0].text, "edited");

        doc.apply(toggle_resolved_op(&doc, page, &id).expect("resolve"))
            .expect("apply resolve");
        assert!(read_comments(&doc, page)[0].resolved);

        // One undo steps back exactly one mutation.
        assert!(doc.undo().expect("undo"));
        assert!(!read_comments(&doc, page)[0].resolved);

        doc.apply(remove_comment_op(&doc, page, &id).expect("remove"))
            .expect("apply remove");
        assert!(read_comments(&doc, page).is_empty());
    }

    #[test]
    fn replies_round_trip_and_blank_replies_are_dropped() {
        let (mut doc, page) = doc_with_page();
        let (id, op) = add_comment_op(&doc, page, [1.0, 2.0], "thread root").expect("add");
        doc.apply(op).expect("apply add");

        assert!(reply_comment_op(&doc, page, &id, "   ").is_none());

        doc.apply(reply_comment_op(&doc, page, &id, "first reply").expect("reply"))
            .expect("apply reply");
        let comment = read_comments(&doc, page).remove(0);
        assert_eq!(comment.message_count(), 2);
        assert_eq!(comment.replies[0].body, "first reply");
        assert!(!comment.author.is_empty());
        assert!(comment.newest_created() >= comment.created);
    }

    /// Comments written by the placeholder v1 (id/world/text/resolved only)
    /// must keep deserializing after the model grew author/replies/etc.
    #[test]
    fn legacy_minimal_comments_still_deserialize() {
        let (mut doc, page) = doc_with_page();
        let old = doc.scene.get(page).expect("page").meta.clone();
        doc.apply(Operation::SetMeta {
            id: page,
            old,
            new: serde_json::json!({
                "comments": [{ "id": "legacy", "world": [3.0, 4.0], "text": "old note" }]
            }),
        })
        .expect("seed legacy comment");
        let comments = read_comments(&doc, page);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "old note");
        assert_eq!(comments[0].author, "");
        assert!(comments[0].replies.is_empty());
    }

    /// Comments share the meta blob with other extensions — writing the list
    /// must not clobber unrelated keys.
    #[test]
    fn comment_writes_preserve_unrelated_meta_keys() {
        let (mut doc, page) = doc_with_page();
        let old = doc.scene.get(page).expect("page").meta.clone();
        doc.apply(Operation::SetMeta {
            id: page,
            old,
            new: serde_json::json!({ "sidebarIcon": "star" }),
        })
        .expect("seed unrelated meta");

        let (id, op) = add_comment_op(&doc, page, [0.0, 0.0], "note").expect("add");
        doc.apply(op).expect("apply add");
        let meta = &doc.scene.get(page).expect("page").meta;
        assert_eq!(meta["sidebarIcon"], "star");

        doc.apply(remove_comment_op(&doc, page, &id).expect("remove"))
            .expect("apply remove");
        let meta = &doc.scene.get(page).expect("page").meta;
        assert_eq!(meta["sidebarIcon"], "star");
        assert!(meta.get("comments").is_none(), "empty list removes the key");
    }
}
