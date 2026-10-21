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

use fanta_doc::{AnimationClipId, Doc, NodeId, Operation};
use glam::DVec2;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const META_KEY: &str = "comments";

/// What an `@label` in a comment body points at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum MentionTarget {
    Node(NodeId),
    Agent(String),
    /// Unresolved free-text handle; kept so it round-trips.
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mention {
    /// The body contains the literal `@{label}`.
    pub label: String,
    pub target: MentionTarget,
}

/// A path reference only — bytes are never embedded in the page meta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

/// Optional AI context attached to a message. This is persisted as comment
/// metadata so an agent integration can act on it without rewriting the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentSkill {
    Summarize,
    Search,
    Format,
    Review,
}

impl CommentSkill {
    pub const ALL: [Self; 4] = [Self::Summarize, Self::Search, Self::Format, Self::Review];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Summarize => "Summarize",
            Self::Search => "Search",
            Self::Format => "Format",
            Self::Review => "Review",
        }
    }

    pub const fn prompt_instruction(self) -> &'static str {
        match self {
            Self::Summarize => {
                "Summarize the discussion and its design context. Return a concise summary, decisions, and concrete action items."
            }
            Self::Search => {
                "Research the request. Inspect the repository and referenced local attachments, and use web search when it is useful. Return evidence-backed findings, relevant paths or sources, and recommended next steps."
            }
            Self::Format => {
                "Rewrite or format the requested content so it is clear, consistent, and ready to use. Preserve the original meaning and call out any ambiguity that needs a decision."
            }
            Self::Review => {
                "Review the design on the referenced page with its unresolved canvas comments. Inspect the actual source and a canvas screenshot, identify actionable visual or interaction issues, post each finding as its own canvas comment with add_comment / design_comments create (anchored to the layer it concerns, with a severity and a category), and reply to the original comments with a summary. Do not change scene content during review."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentReply {
    pub author: String,
    pub body: String,
    /// Unix seconds.
    pub created: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_agent: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<Mention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<CommentSkill>,
}

/// A comment's stable location on one authored animation timeline.
///
/// The canvas position remains the visual pin anchor. Keeping both the clip
/// and time avoids silently retargeting a comment when another animation is
/// selected or becomes the document's first clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionCommentAnchor {
    pub clip: AnimationClipId,
    pub time_ms: u32,
}

/// Where a comment sits on a layer: the node and a point in its local units,
/// so the pin follows the layer when it moves. [`Comment::world`] keeps the
/// authored position for readers that do not resolve anchors.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CommentAnchor {
    pub node: NodeId,
    pub offset: [f64; 2],
}

/// How much a review finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentSeverity {
    Info,
    Minor,
    Major,
    Blocker,
}

impl CommentSeverity {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Minor => "Minor",
            Self::Major => "Major",
            Self::Blocker => "Blocker",
        }
    }
}

/// What a review finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentCategory {
    Layout,
    Typography,
    Color,
    Accessibility,
    Consistency,
    Content,
    Interaction,
    Motion,
    Other,
}

impl CommentCategory {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Layout => "Layout",
            Self::Typography => "Typography",
            Self::Color => "Color",
            Self::Accessibility => "Accessibility",
            Self::Consistency => "Consistency",
            Self::Content => "Content",
            Self::Interaction => "Interaction",
            Self::Motion => "Motion",
            Self::Other => "Other",
        }
    }
}

/// A change that would address a finding: a `design_edit` / `batch_design`
/// ops list the user can apply from the thread. Kept as JSON so a comment
/// written by a newer build still loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestedFix {
    pub summary: String,
    pub ops: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    /// Stable identity for panel rows and pin hit-testing.
    pub id: String,
    /// World-space anchor of the pin.
    pub world: [f64; 2],
    #[serde(default)]
    pub author: String,
    pub text: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_agent: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<Mention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<CommentReply>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<CommentSkill>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub question: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_anchor: Option<MotionCommentAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommentAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<CommentSeverity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<CommentCategory>,
    /// Groups the findings one review run produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<SuggestedFix>,
}

impl Comment {
    pub fn location_label(&self, doc: &Doc, page_name: &str) -> String {
        let mut labels = Vec::new();
        if let Some(severity) = self.severity {
            labels.push(severity.label());
        }
        if let Some(category) = self.category {
            labels.push(category.label());
        }
        labels.push(
            self.anchor
                .and_then(|anchor| doc.scene.get(anchor.node))
                .map(|node| node.name.as_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(page_name),
        );
        labels.join(" · ")
    }

    pub fn message_count(&self) -> usize {
        1 + self.replies.len()
    }

    /// The newest activity in the thread, for unread tracking and sorting.
    pub fn newest_created(&self) -> u64 {
        self.replies
            .iter()
            .map(|reply| reply.created)
            .fold(self.created, u64::max)
    }
}

/// The commenting identity: the OS user, like the original app.
pub fn author_name() -> String {
    std::env::var("USER").unwrap_or_else(|_| "you".into())
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// The page's comments, oldest first (pin numbers are 1-based list positions).
pub fn read_comments(doc: &Doc, page: NodeId) -> Vec<Comment> {
    doc.scene
        .get(page)
        .and_then(|node| node.meta.get(META_KEY))
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| serde_json::from_value(value.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The layer under `world` on `page`, with the point in its local units, so a
/// pin placed on a layer follows it. Hit testing skips groups and frames, so a
/// pin on empty background stays a page position.
pub fn anchor_for_world(doc: &Doc, page: NodeId, world: [f64; 2]) -> Option<CommentAnchor> {
    let point = DVec2::new(world[0], world[1]);
    let node = doc.scene.topmost_hit_where(point, |candidate| {
        doc.scene
            .ancestors_of(candidate)
            .any(|ancestor| ancestor.id == page)
    })?;
    let local = doc
        .scene
        .world_transform(node)?
        .inverse()
        .transform_point(point);
    (local.x.is_finite() && local.y.is_finite()).then_some(CommentAnchor {
        node,
        offset: [local.x, local.y],
    })
}

/// The pin's current position: the anchored layer's placement of the stored
/// offset, or the authored position when there is no anchor or the layer is
/// gone.
pub fn resolve_world(doc: &Doc, comment: &Comment) -> [f64; 2] {
    comment
        .anchor
        .and_then(|anchor| {
            let transform = doc.scene.world_transform(anchor.node)?;
            let point = transform.transform_point(DVec2::new(anchor.offset[0], anchor.offset[1]));
            (point.x.is_finite() && point.y.is_finite()).then_some([point.x, point.y])
        })
        .unwrap_or(comment.world)
}

fn merge_comment_value(
    original_value: &serde_json::Value,
    original: &Comment,
    comment: &Comment,
) -> Option<serde_json::Value> {
    if original == comment {
        return Some(original_value.clone());
    }
    let serialized = serde_json::to_value(comment).ok()?;
    let (Some(original_value), Some(serialized)) =
        (original_value.as_object(), serialized.as_object())
    else {
        return Some(serialized);
    };
    let mut merged = original_value.clone();
    let mut replace_if_changed = |field: &str, changed: bool| {
        if !changed {
            return;
        }
        if let Some(value) = serialized.get(field) {
            merged.insert(field.to_string(), value.clone());
        } else {
            merged.remove(field);
        }
    };
    replace_if_changed("id", original.id != comment.id);
    replace_if_changed("world", original.world != comment.world);
    replace_if_changed("author", original.author != comment.author);
    replace_if_changed("text", original.text != comment.text);
    replace_if_changed("created", original.created != comment.created);
    replace_if_changed("resolved", original.resolved != comment.resolved);
    replace_if_changed("from_agent", original.from_agent != comment.from_agent);
    replace_if_changed("mentions", original.mentions != comment.mentions);
    replace_if_changed("attachments", original.attachments != comment.attachments);
    replace_if_changed("skill", original.skill != comment.skill);
    replace_if_changed("question", original.question != comment.question);
    replace_if_changed("anchor", original.anchor != comment.anchor);
    replace_if_changed("severity", original.severity != comment.severity);
    replace_if_changed("category", original.category != comment.category);
    replace_if_changed(
        "review_run_id",
        original.review_run_id != comment.review_run_id,
    );
    replace_if_changed(
        "suggested_fix",
        original.suggested_fix != comment.suggested_fix,
    );

    if original.replies != comment.replies {
        let preserved_prefix = if comment.replies.starts_with(&original.replies) {
            let mut values = original_value
                .get("replies")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let serialized_replies = serialized
                .get("replies")
                .and_then(serde_json::Value::as_array);
            if values.len() == original.replies.len()
                && serialized_replies.is_some_and(|replies| replies.len() == comment.replies.len())
            {
                let serialized_replies = serialized_replies?;
                values.extend(
                    serialized_replies
                        .iter()
                        .skip(original.replies.len())
                        .cloned(),
                );
                Some(values)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(values) = preserved_prefix {
            merged.insert("replies".to_string(), serde_json::Value::Array(values));
        } else if let Some(value) = serialized.get("replies") {
            merged.insert("replies".to_string(), value.clone());
        } else {
            merged.remove("replies");
        }
    }

    if original.motion_anchor != comment.motion_anchor {
        if let Some(value) = serialized.get("motion_anchor") {
            let value = match (
                original_value
                    .get("motion_anchor")
                    .and_then(serde_json::Value::as_object),
                value.as_object(),
            ) {
                (Some(original_anchor), Some(serialized_anchor)) => {
                    let mut merged_anchor = original_anchor.clone();
                    merged_anchor.extend(serialized_anchor.clone());
                    serde_json::Value::Object(merged_anchor)
                }
                _ => value.clone(),
            };
            merged.insert("motion_anchor".to_string(), value);
        } else {
            merged.remove("motion_anchor");
        }
    }
    Some(serde_json::Value::Object(merged))
}

fn merge_comment_list(
    existing: Option<&serde_json::Value>,
    comments: &[Comment],
) -> Option<Vec<serde_json::Value>> {
    let Some(existing) = existing else {
        return serde_json::to_value(comments).ok()?.as_array().cloned();
    };
    let existing = existing.as_array()?;
    let mut used = vec![false; comments.len()];
    let mut merged = Vec::with_capacity(existing.len().max(comments.len()));
    for value in existing {
        let Ok(original) = serde_json::from_value::<Comment>(value.clone()) else {
            merged.push(value.clone());
            continue;
        };
        let Some((index, comment)) = comments
            .iter()
            .enumerate()
            .find(|(index, comment)| !used[*index] && comment.id == original.id)
        else {
            continue;
        };
        used[index] = true;
        merged.push(merge_comment_value(value, &original, comment)?);
    }
    for (index, comment) in comments.iter().enumerate() {
        if !used[index] {
            merged.push(serde_json::to_value(comment).ok()?);
        }
    }
    Some(merged)
}

/// One undoable [`Operation::SetMeta`] replacing the page's comment list while
/// preserving every unrelated meta key and any comment entries written by a
/// newer client that this build cannot decode. `None` when the page is gone,
/// the existing comments value is not a list, or the list is unchanged.
fn set_comments_op(doc: &Doc, page: NodeId, comments: &[Comment]) -> Option<Operation> {
    let node = doc.scene.get(page)?;
    let old = node.meta.clone();
    let mut new = match &old {
        serde_json::Value::Object(map) => serde_json::Value::Object(map.clone()),
        serde_json::Value::Null => serde_json::json!({}),
        _ => return None,
    };
    let list = merge_comment_list(node.meta.get(META_KEY), comments)?;
    if list.is_empty() {
        new.as_object_mut()?.remove(META_KEY);
    } else {
        new.as_object_mut()?
            .insert(META_KEY.to_string(), serde_json::Value::Array(list));
    }
    (new != old).then_some(Operation::SetMeta { id: page, old, new })
}

pub fn add_comment_op(
    doc: &Doc,
    page: NodeId,
    world: [f64; 2],
    text: &str,
) -> Option<(String, Operation)> {
    add_comment_full_op(doc, page, world, text, Vec::new(), Vec::new(), None, None)
}

pub fn add_comment_full_op(
    doc: &Doc,
    page: NodeId,
    world: [f64; 2],
    text: &str,
    mentions: Vec<Mention>,
    attachments: Vec<Attachment>,
    skill: Option<CommentSkill>,
    motion_anchor: Option<MotionCommentAnchor>,
) -> Option<(String, Operation)> {
    let text = text.trim();
    if text.is_empty() && attachments.is_empty() && skill.is_none() {
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
        mentions,
        attachments,
        replies: Vec::new(),
        skill,
        question: false,
        motion_anchor,
        anchor: anchor_for_world(doc, page, world),
        severity: None,
        category: None,
        review_run_id: None,
        suggested_fix: None,
    });
    set_comments_op(doc, page, &comments).map(|op| (id, op))
}

/// Place a comment as an agent: a review finding, a question or a note. With
/// `node`, the pin anchors to that layer at `offset` (its centre by default);
/// otherwise `world` is required and the layer under it, if any, becomes the
/// anchor. Returns the new comment's id with the operation.
pub fn agent_add_comment_op(
    doc: &Doc,
    page: NodeId,
    draft: &fanta_design_ops::CommentDraft,
) -> anyhow::Result<(String, Operation)> {
    let comment = agent_comment(doc, page, draft)?;
    let id = comment.id.clone();
    let mut comments = read_comments(doc, page);
    comments.push(comment);
    set_comments_op(doc, page, &comments)
        .map(|op| (id, op))
        .ok_or_else(|| anyhow::anyhow!("the comment could not be saved"))
}

/// Place agent comments on `page` with one operation, so a review lands as a
/// single undo step. Each draft gets its new comment's id or the reason it was
/// refused; the operation is `None` when no draft was accepted.
pub fn agent_add_comments_op(
    doc: &Doc,
    page: NodeId,
    drafts: &[fanta_design_ops::CommentDraft],
) -> (Vec<anyhow::Result<String>>, Option<Operation>) {
    let mut comments = read_comments(doc, page);
    let mut results = Vec::with_capacity(drafts.len());
    for draft in drafts {
        results.push(agent_comment(doc, page, draft).map(|comment| {
            let id = comment.id.clone();
            comments.push(comment);
            id
        }));
    }
    if !results.iter().any(Result::is_ok) {
        return (results, None);
    }
    match set_comments_op(doc, page, &comments) {
        Some(operation) => (results, Some(operation)),
        None => (
            results
                .into_iter()
                .map(|result| {
                    result.and_then(|_| Err(anyhow::anyhow!("the comment could not be saved")))
                })
                .collect(),
            None,
        ),
    }
}

/// Check `draft` and build the comment it describes on `page`.
fn agent_comment(
    doc: &Doc,
    page: NodeId,
    draft: &fanta_design_ops::CommentDraft,
) -> anyhow::Result<Comment> {
    let text = draft.text.trim();
    anyhow::ensure!(!text.is_empty(), "a comment must contain text");
    anyhow::ensure!(
        doc.scene.get(page).is_some(),
        "the comment page no longer exists"
    );
    let (world, anchor) = match draft.node.as_deref() {
        Some(key) => {
            let node = key
                .parse::<NodeId>()
                .map_err(|_| anyhow::anyhow!("{key:?} is not a node id"))?;
            let canvas_node = doc
                .scene
                .get(node)
                .ok_or_else(|| anyhow::anyhow!("node {key} does not exist"))?;
            anyhow::ensure!(
                node == page
                    || doc
                        .scene
                        .ancestors_of(node)
                        .any(|ancestor| ancestor.id == page),
                "node {key} is not on the comment page"
            );
            let offset = draft.offset.unwrap_or_else(|| {
                canvas_node
                    .data
                    .local_bounds()
                    .map(|bounds| {
                        let centre = bounds.center();
                        [centre.x, centre.y]
                    })
                    .unwrap_or([0.0, 0.0])
            });
            anyhow::ensure!(
                offset.iter().all(|value| value.is_finite()),
                "the comment offset must be finite"
            );
            let transform = doc
                .scene
                .world_transform(node)
                .ok_or_else(|| anyhow::anyhow!("node {key} has no placement yet"))?;
            let point = transform.transform_point(DVec2::new(offset[0], offset[1]));
            ([point.x, point.y], Some(CommentAnchor { node, offset }))
        }
        None => {
            let world = draft
                .world
                .ok_or_else(|| anyhow::anyhow!("a comment needs a node or a world position"))?;
            anyhow::ensure!(
                world.iter().all(|value| value.is_finite()),
                "the comment position must be finite"
            );
            (world, anchor_for_world(doc, page, world))
        }
    };
    anyhow::ensure!(
        world.iter().all(|value| value.is_finite()),
        "the comment position must be finite"
    );
    let severity = draft
        .severity
        .as_deref()
        .map(|severity| {
            serde_json::from_value::<CommentSeverity>(serde_json::Value::String(
                severity.trim().to_ascii_lowercase(),
            ))
            .map_err(|_| anyhow::anyhow!("severity must be info, minor, major or blocker"))
        })
        .transpose()?;
    let category = draft
        .category
        .as_deref()
        .map(|category| {
            serde_json::from_value::<CommentCategory>(serde_json::Value::String(
                category.trim().to_ascii_lowercase(),
            ))
            .map_err(|_| {
                anyhow::anyhow!(
                    "category must be layout, typography, color, accessibility, consistency, content, interaction, motion or other"
                )
            })
        })
        .transpose()?;
    let suggested_fix = draft
        .suggested_fix
        .as_ref()
        .map(|fix| {
            anyhow::ensure!(
                !fix.summary.trim().is_empty(),
                "a suggested fix needs a summary"
            );
            anyhow::ensure!(!fix.ops.is_empty(), "a suggested fix needs at least one op");
            for (index, op) in fix.ops.iter().enumerate() {
                let operation = serde_json::from_value::<fanta_design_ops::DesignOp>(op.clone()).map_err(
                    |error| anyhow::anyhow!("suggested fix op {index} is invalid: {error}"),
                )?;
                anyhow::ensure!(!matches!(operation, fanta_design_ops::DesignOp::Select { .. } | fanta_design_ops::DesignOp::SetViewport { .. }),
                    "suggested fixes must change design content; selection and viewport operations cannot be undone");
            }
            Ok(SuggestedFix {
                summary: clean_context_line(fix.summary.trim()),
                ops: fix.ops.clone(),
            })
        })
        .transpose()?;
    let author = draft
        .author
        .as_deref()
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .map(clean_context_line)
        .unwrap_or_else(|| "Fanta Agent".to_string());
    Ok(Comment {
        id: fanta_doc::NodeId::new().to_string(),
        world,
        author,
        text: text.to_string(),
        created: now_secs(),
        resolved: false,
        from_agent: true,
        mentions: mentions_for_body(doc, page, text),
        attachments: Vec::new(),
        replies: Vec::new(),
        skill: None,
        question: false,
        motion_anchor: None,
        anchor,
        severity,
        category,
        review_run_id: draft
            .review_run_id
            .as_deref()
            .map(str::trim)
            .filter(|run| !run.is_empty())
            .map(clean_context_line),
        suggested_fix,
    })
}

/// Static comments are visible on every canvas surface. Motion comments are
/// pins for one clip and are only visible while that clip is being authored.
pub fn comment_is_visible(comment: &Comment, active_motion_clip: Option<AnimationClipId>) -> bool {
    comment
        .motion_anchor
        .is_none_or(|anchor| active_motion_clip == Some(anchor.clip))
}

/// Append a reply to a thread. Blank replies (empty trimmed body) are dropped.
pub fn reply_comment_op(doc: &Doc, page: NodeId, id: &str, body: &str) -> Option<Operation> {
    reply_comment_full_op(doc, page, id, body, Vec::new(), Vec::new(), None)
}

pub fn reply_comment_full_op(
    doc: &Doc,
    page: NodeId,
    id: &str,
    body: &str,
    mentions: Vec<Mention>,
    attachments: Vec<Attachment>,
    skill: Option<CommentSkill>,
) -> Option<Operation> {
    let body = body.trim();
    if body.is_empty() && attachments.is_empty() && skill.is_none() {
        return None;
    }
    let mut comments = read_comments(doc, page);
    let comment = comments.iter_mut().find(|comment| comment.id == id)?;
    comment.replies.push(CommentReply {
        author: author_name(),
        body: body.to_string(),
        created: now_secs(),
        from_agent: false,
        mentions,
        attachments,
        skill,
    });
    set_comments_op(doc, page, &comments)
}

pub fn agent_reply_comment_op(
    doc: &Doc,
    page: NodeId,
    id: &str,
    body: &str,
    author: &str,
    resolve: bool,
) -> anyhow::Result<Operation> {
    let body = body.trim();
    anyhow::ensure!(!body.is_empty(), "a comment reply must contain text");
    anyhow::ensure!(
        doc.scene.get(page).is_some(),
        "the comment page no longer exists"
    );
    let mut comments = read_comments(doc, page);
    let comment = comments
        .iter_mut()
        .find(|comment| comment.id == id)
        .ok_or_else(|| anyhow::anyhow!("comment {id} does not exist on this page"))?;
    comment.replies.push(CommentReply {
        author: if author.trim().is_empty() {
            "Fanta Agent".to_string()
        } else {
            clean_context_line(author)
        },
        body: body.to_string(),
        created: now_secs(),
        from_agent: true,
        mentions: mentions_for_body(doc, page, body),
        attachments: Vec::new(),
        skill: None,
    });
    if resolve {
        comment.resolved = true;
    }
    set_comments_op(doc, page, &comments)
        .ok_or_else(|| anyhow::anyhow!("the comment reply could not be saved"))
}

/// Resolve the `@handles` currently present in a message. Agent names and
/// layer names keep stable targets; unknown handles remain lossless text
/// mentions so importing or later resolving them cannot discard information.
pub fn mentions_for_body(doc: &Doc, page: NodeId, body: &str) -> Vec<Mention> {
    const AGENTS: [&str; 3] = ["Claude", "Codex", "Fanta"];
    let mut labels = Vec::new();
    for word in body.split_whitespace() {
        let Some(label) = word.strip_prefix('@') else {
            continue;
        };
        let label = label
            .trim_matches(|character: char| !character.is_alphanumeric() && character != '_')
            .trim();
        if !label.is_empty() && !labels.iter().any(|existing| existing == label) {
            labels.push(label.to_string());
        }
    }

    labels
        .into_iter()
        .map(|label| {
            let target = AGENTS
                .iter()
                .find(|agent| agent.eq_ignore_ascii_case(&label))
                .map(|agent| MentionTarget::Agent((*agent).to_string()))
                .or_else(|| {
                    doc.scene.descendants_of(page).find_map(|id| {
                        let node = doc.scene.get(id)?;
                        let handle = node
                            .name
                            .split_whitespace()
                            .filter(|part| !part.is_empty())
                            .collect::<Vec<_>>()
                            .join("_");
                        handle
                            .eq_ignore_ascii_case(&label)
                            .then_some(MentionTarget::Node(id))
                    })
                })
                .unwrap_or_else(|| MentionTarget::Text(label.clone()));
            Mention { label, target }
        })
        .collect()
}

pub fn clean_context_line(text: &str) -> String {
    text.replace(['\r', '\n', '\u{2028}', '\u{2029}'], " ")
        .trim()
        .to_string()
}

pub fn update_comment_text_op(doc: &Doc, page: NodeId, id: &str, text: &str) -> Option<Operation> {
    let mut comments = read_comments(doc, page);
    let comment = comments.iter_mut().find(|comment| comment.id == id)?;
    comment.text = text.to_string();
    set_comments_op(doc, page, &comments)
}

pub fn toggle_resolved_op(doc: &Doc, page: NodeId, id: &str) -> Option<Operation> {
    let mut comments = read_comments(doc, page);
    let comment = comments.iter_mut().find(|comment| comment.id == id)?;
    comment.resolved = !comment.resolved;
    set_comments_op(doc, page, &comments)
}

pub fn remove_comment_op(doc: &Doc, page: NodeId, id: &str) -> Option<Operation> {
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
        assert!(comment_is_visible(&comments[0], None));
        assert!(comment_is_visible(
            &comments[0],
            Some(AnimationClipId::from_u128(5))
        ));

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

    #[test]
    fn agent_replies_are_attributed_persistent_and_undoable() {
        let (mut doc, page) = doc_with_page();
        let (id, operation) =
            add_comment_op(&doc, page, [1.0, 2.0], "Check contrast").expect("create comment");
        doc.apply(operation).expect("add comment");
        let operation = agent_reply_comment_op(
            &doc,
            page,
            &id,
            "Verified the label contrast on the canvas.",
            "Codex",
            true,
        )
        .expect("agent reply");
        doc.apply(operation).expect("save agent reply");
        let comment = read_comments(&doc, page).remove(0);
        assert!(comment.resolved);
        let reply = comment.replies.first().expect("reply persisted");
        assert_eq!(reply.author, "Codex");
        assert!(reply.from_agent);
        let serialized = serde_json::to_string(&comment).expect("serialize");
        let restored: Comment = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(restored, comment);
        doc.undo().expect("undo reply");
        let comment = read_comments(&doc, page).remove(0);
        assert!(!comment.resolved);
        assert!(comment.replies.is_empty());
    }

    #[test]
    fn invalid_agent_replies_return_actionable_errors() {
        let (doc, page) = doc_with_page();
        assert!(
            agent_reply_comment_op(&doc, page, "missing", "  ", "Codex", false)
                .expect_err("blank reply")
                .to_string()
                .contains("text")
        );
        assert!(
            agent_reply_comment_op(&doc, page, "missing", "Checked", "Codex", false)
                .expect_err("missing comment")
                .to_string()
                .contains("does not exist")
        );
        assert!(read_comments(&doc, page).is_empty());
    }

    #[test]
    fn rich_messages_round_trip_mentions_attachments_and_skill() {
        let (mut doc, page) = doc_with_page();
        let mut layer = CanvasNode::new(NodeData::Group(GroupNode::default()));
        layer.name = "Hero Card".to_string();
        layer.parent = Some(page);
        let layer_id = layer.id;
        doc.apply(Operation::create_node(layer))
            .expect("named layer");
        let attachment = Attachment {
            name: "reference.png".to_string(),
            path: Some(PathBuf::from("/tmp/reference.png")),
        };
        let mentions =
            mentions_for_body(&doc, page, "Please check @Codex, @Hero_Card and @unknown.");
        assert!(matches!(mentions[0].target, MentionTarget::Agent(_)));
        assert_eq!(mentions[1].target, MentionTarget::Node(layer_id));
        assert!(matches!(mentions[2].target, MentionTarget::Text(_)));

        let (id, add) = add_comment_full_op(
            &doc,
            page,
            [4.0, 5.0],
            "Please check @Codex, @Hero_Card and @unknown.",
            mentions,
            vec![attachment.clone()],
            Some(CommentSkill::Search),
            None,
        )
        .expect("rich add");
        doc.apply(add).expect("apply rich add");
        doc.apply(
            reply_comment_full_op(
                &doc,
                page,
                &id,
                "",
                Vec::new(),
                vec![attachment],
                Some(CommentSkill::Summarize),
            )
            .expect("attachment-only reply"),
        )
        .expect("apply rich reply");

        let comment = read_comments(&doc, page).remove(0);
        assert_eq!(comment.mentions.len(), 3);
        assert_eq!(comment.attachments.len(), 1);
        assert_eq!(comment.skill, Some(CommentSkill::Search));
        assert_eq!(comment.replies[0].attachments.len(), 1);
        assert_eq!(comment.replies[0].skill, Some(CommentSkill::Summarize));
    }

    /// Comments written by the placeholder v1 (id/world/text/resolved only)
    /// must keep deserializing after the model grew author/replies/etc.
    fn page_with_rect() -> (Doc, NodeId, NodeId) {
        use fanta_doc::{Transform2D, VectorNode};
        let (mut doc, page) = doc_with_page();
        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            50.0,
            50.0,
            fanta_doc::color::Color::WHITE,
        )));
        rect.name = "Card".into();
        rect.parent = Some(page);
        rect.transform = Transform2D::translation(10.0, 10.0);
        let rect_id = rect.id;
        doc.apply(Operation::create_node(rect)).expect("rect");
        (doc, page, rect_id)
    }

    #[test]
    fn anchored_comment_follows_its_node_and_falls_back_when_it_is_gone() {
        use fanta_doc::Transform2D;
        let (mut doc, page, rect) = page_with_rect();
        let (id, op) = add_comment_op(&doc, page, [20.0, 30.0], "on the card").expect("add");
        doc.apply(op).expect("apply add");
        let comment = read_comments(&doc, page).remove(0);
        assert_eq!(comment.id, id);
        assert_eq!(
            comment.anchor,
            Some(CommentAnchor {
                node: rect,
                offset: [10.0, 20.0]
            })
        );
        assert_eq!(resolve_world(&doc, &comment), [20.0, 30.0]);

        let old = doc.scene.get(rect).expect("rect").transform;
        doc.apply(Operation::SetTransform {
            id: rect,
            old,
            new: Transform2D::translation(110.0, 10.0),
        })
        .expect("move");
        let comment = read_comments(&doc, page).remove(0);
        assert_eq!(comment.world, [20.0, 30.0], "the authored position is kept");
        assert_eq!(
            resolve_world(&doc, &comment),
            [120.0, 30.0],
            "the pin follows the layer"
        );

        let orphan = Comment {
            anchor: Some(CommentAnchor {
                node: NodeId::from_u128(999),
                offset: [1.0, 1.0],
            }),
            ..comment
        };
        assert_eq!(
            resolve_world(&doc, &orphan),
            [20.0, 30.0],
            "a missing layer falls back"
        );

        doc.apply(Operation::DeleteSubtree {
            snapshot: vec![doc.scene.get(rect).expect("rect").clone()],
        })
        .expect("delete anchored layer");
        let stored = read_comments(&doc, page).remove(0);
        assert_eq!(resolve_world(&doc, &stored), [20.0, 30.0]);
        assert!(doc.undo().expect("undo deletion"));
        assert_eq!(resolve_world(&doc, &stored), [120.0, 30.0]);

        let (_, off_canvas) = add_comment_op(&doc, page, [500.0, 500.0], "empty").expect("add");
        doc.apply(off_canvas).expect("apply");
        assert!(
            read_comments(&doc, page)[1].anchor.is_none(),
            "empty background has no anchor"
        );
    }

    #[test]
    fn agent_comments_carry_severity_category_run_id_and_fix_and_undo() {
        let (mut doc, page, rect) = page_with_rect();
        let draft = fanta_design_ops::CommentDraft {
            node: Some(rect.to_string()),
            text: "  Contrast is too low on @Card  ".into(),
            severity: Some("Major".into()),
            category: Some("accessibility".into()),
            review_run_id: Some("run-1".into()),
            suggested_fix: Some(fanta_design_ops::SuggestedFixDraft {
                summary: "Darken the fill".into(),
                ops: vec![
                    serde_json::json!({"op":"set_props","id":rect.to_string(),"props":{"fill":"#111111"}}),
                ],
            }),
            ..Default::default()
        };
        let (id, op) = agent_add_comment_op(&doc, page, &draft).expect("agent comment");
        doc.apply(op).expect("apply");
        let comment = read_comments(&doc, page).remove(0);
        assert_eq!(comment.id, id);
        assert!(comment.from_agent);
        assert_eq!(comment.author, "Fanta Agent");
        assert_eq!(comment.text, "Contrast is too low on @Card");
        assert_eq!(comment.severity, Some(CommentSeverity::Major));
        assert_eq!(comment.category, Some(CommentCategory::Accessibility));
        assert_eq!(comment.review_run_id.as_deref(), Some("run-1"));
        assert_eq!(
            comment.anchor,
            Some(CommentAnchor {
                node: rect,
                offset: [25.0, 25.0]
            }),
            "a node anchor defaults to the layer's centre"
        );
        assert_eq!(comment.world, [35.0, 35.0]);
        assert_eq!(
            comment
                .mentions
                .iter()
                .map(|mention| &mention.target)
                .collect::<Vec<_>>(),
            vec![&MentionTarget::Node(rect)]
        );
        assert_eq!(
            comment
                .suggested_fix
                .as_ref()
                .map(|fix| fix.summary.as_str()),
            Some("Darken the fill")
        );
        let json = serde_json::to_value(&comment).expect("json");
        assert_eq!(json["severity"], "major");
        assert_eq!(json["category"], "accessibility");
        assert!(doc.undo().expect("undo"));
        assert!(read_comments(&doc, page).is_empty());

        let bad = fanta_design_ops::CommentDraft {
            world: Some([1.0, 1.0]),
            text: "x".into(),
            severity: Some("urgent".into()),
            ..Default::default()
        };
        assert!(agent_add_comment_op(&doc, page, &bad).is_err());
        let bad_fix = fanta_design_ops::CommentDraft {
            world: Some([1.0, 1.0]),
            text: "x".into(),
            suggested_fix: Some(fanta_design_ops::SuggestedFixDraft {
                summary: "nope".into(),
                ops: vec![serde_json::json!({"op":"teleport"})],
            }),
            ..Default::default()
        };
        assert!(agent_add_comment_op(&doc, page, &bad_fix).is_err());
        let presence_fix = fanta_design_ops::CommentDraft {
            world: Some([1.0, 1.0]),
            text: "x".into(),
            suggested_fix: Some(fanta_design_ops::SuggestedFixDraft {
                summary: "Move the camera".into(),
                ops: vec![serde_json::json!({"op":"set_viewport", "center":[0,0],"zoom":1})],
            }),
            ..Default::default()
        };
        assert!(agent_add_comment_op(&doc, page, &presence_fix).is_err());
        let nowhere = fanta_design_ops::CommentDraft {
            text: "x".into(),
            ..Default::default()
        };
        assert!(agent_add_comment_op(&doc, page, &nowhere).is_err());
    }

    #[test]
    fn a_review_lands_as_one_undo_step_and_reports_each_refused_draft() {
        let (mut doc, page, rect) = page_with_rect();
        let finding = |text: &str| fanta_design_ops::CommentDraft {
            node: Some(rect.to_string()),
            text: text.into(),
            author: Some("Fanta Review".into()),
            severity: Some("minor".into()),
            category: Some("layout".into()),
            review_run_id: Some("rev-1".into()),
            ..Default::default()
        };
        let refused = fanta_design_ops::CommentDraft {
            text: "anchored nowhere".into(),
            ..Default::default()
        };
        let drafts = [finding("First"), refused, finding("Second")];
        let (results, operation) = agent_add_comments_op(&doc, page, &drafts);
        assert_eq!(results.len(), 3);
        assert!(
            results[1].is_err(),
            "a draft without a node or position is refused"
        );
        doc.apply(operation.expect("accepted drafts produce one operation"))
            .expect("apply");
        let comments = read_comments(&doc, page);
        assert_eq!(
            comments
                .iter()
                .map(|comment| (
                    comment.id.as_str(),
                    comment.text.as_str(),
                    comment.author.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                (
                    results[0].as_ref().expect("first").as_str(),
                    "First",
                    "Fanta Review"
                ),
                (
                    results[2].as_ref().expect("second").as_str(),
                    "Second",
                    "Fanta Review"
                ),
            ]
        );
        assert!(doc.undo().expect("undo"));
        assert!(
            read_comments(&doc, page).is_empty(),
            "one undo removes the whole review"
        );

        let (results, operation) = agent_add_comments_op(&doc, page, &drafts[1..2]);
        assert!(results[0].is_err());
        assert!(operation.is_none(), "nothing to place, nothing to apply");
    }

    #[test]
    fn review_fields_merge_into_existing_comment_values() {
        let (mut doc, page, _) = page_with_rect();
        let (id, op) = add_comment_op(&doc, page, [5.0, 5.0], "note").expect("add");
        doc.apply(op).expect("apply");
        let mut comments = read_comments(&doc, page);
        comments[0].severity = Some(CommentSeverity::Minor);
        comments[0].category = Some(CommentCategory::Layout);
        comments[0].suggested_fix = Some(SuggestedFix {
            summary: "Nudge".into(),
            ops: vec![serde_json::json!({"op":"set_props","id":"n_x","props":{"x":1}})],
        });
        doc.apply(set_comments_op(&doc, page, &comments).expect("edit"))
            .expect("apply edit");
        let stored = read_comments(&doc, page).remove(0);
        assert_eq!(stored.id, id);
        assert_eq!(stored.severity, Some(CommentSeverity::Minor));
        assert_eq!(stored.category, Some(CommentCategory::Layout));
        assert_eq!(stored.suggested_fix, comments[0].suggested_fix);
        assert_eq!(stored.anchor, comments[0].anchor);
    }

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
        assert_eq!(comments[0].motion_anchor, None);
    }

    #[test]
    fn non_object_page_meta_refuses_comment_writes_without_replacing_it() {
        let (mut doc, page) = doc_with_page();
        let opaque_meta = serde_json::json!("metadata from a newer client");
        let old = doc.scene.get(page).expect("page").meta.clone();
        doc.apply(Operation::SetMeta {
            id: page,
            old,
            new: opaque_meta.clone(),
        })
        .expect("seed opaque meta");

        assert!(add_comment_op(&doc, page, [0.0, 0.0], "Comment").is_none());
        assert_eq!(doc.scene.get(page).expect("page").meta, opaque_meta);
    }

    #[test]
    fn malformed_future_anchor_does_not_hide_or_get_overwritten_with_valid_comments() {
        let (mut doc, page) = doc_with_page();
        let known_anchor = MotionCommentAnchor {
            clip: AnimationClipId::from_u128(7),
            time_ms: 875,
        };
        let mut known_anchor_value = serde_json::to_value(known_anchor).expect("serialize anchor");
        known_anchor_value
            .as_object_mut()
            .expect("anchor object")
            .insert("track_id".to_string(), serde_json::json!("future-track"));
        let mut malformed_anchor = serde_json::to_value(MotionCommentAnchor {
            clip: AnimationClipId::from_u128(42),
            time_ms: 875,
        })
        .expect("serialize anchor");
        malformed_anchor
            .as_object_mut()
            .expect("anchor object")
            .insert("time_ms".to_string(), serde_json::json!("future-time"));
        let malformed = serde_json::json!({
            "id": "future",
            "world": [2.0, 3.0],
            "text": "Newer comment",
            "motion_anchor": malformed_anchor,
            "future_entry": { "keep": true }
        });
        let old = doc.scene.get(page).expect("page").meta.clone();
        doc.apply(Operation::SetMeta {
            id: page,
            old,
            new: serde_json::json!({
                "comments": [
                    {
                        "id": "known",
                        "world": [0.0, 1.0],
                        "text": "Known comment",
                        "avatar": "future-avatar",
                        "future_field": { "keep": true },
                        "motion_anchor": known_anchor_value,
                        "replies": [{
                            "author": "Reviewer",
                            "body": "Existing reply",
                            "created": 1,
                            "future_reply": { "keep": true }
                        }]
                    },
                    malformed
                ]
            }),
        })
        .expect("seed mixed-version comments");

        let comments = read_comments(&doc, page);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "known");
        assert_eq!(comments[0].motion_anchor, Some(known_anchor));
        assert_eq!(comments[0].replies.len(), 1);
        doc.apply(update_comment_text_op(&doc, page, "known", "Updated").expect("update"))
            .expect("apply update");
        doc.apply(reply_comment_op(&doc, page, "known", "Another reply").expect("reply"))
            .expect("apply reply");
        let (_, add) = add_comment_op(&doc, page, [4.0, 5.0], "Another").expect("add");
        doc.apply(add).expect("apply add");

        let stored = doc
            .scene
            .get(page)
            .and_then(|node| node.meta.get(META_KEY))
            .and_then(serde_json::Value::as_array)
            .expect("stored comments");
        assert_eq!(stored.len(), 3);
        assert!(stored.iter().any(|value| value == &malformed));
        let known = stored
            .iter()
            .find(|value| value.get("id").and_then(serde_json::Value::as_str) == Some("known"))
            .expect("known comment");
        assert_eq!(
            known.get("future_field"),
            Some(&serde_json::json!({ "keep": true }))
        );
        assert_eq!(
            known.get("avatar").and_then(serde_json::Value::as_str),
            Some("future-avatar")
        );
        assert_eq!(
            known
                .get("motion_anchor")
                .and_then(|anchor| anchor.get("track_id"))
                .and_then(serde_json::Value::as_str),
            Some("future-track")
        );
        let replies = known
            .get("replies")
            .and_then(serde_json::Value::as_array)
            .expect("stored replies");
        assert_eq!(replies.len(), 2);
        assert_eq!(
            replies.first().and_then(|reply| reply.get("future_reply")),
            Some(&serde_json::json!({ "keep": true }))
        );
        assert_eq!(
            replies
                .get(1)
                .and_then(|reply| reply.get("body"))
                .and_then(serde_json::Value::as_str),
            Some("Another reply")
        );
        assert_eq!(read_comments(&doc, page).len(), 2);
    }

    #[test]
    fn motion_anchor_round_trips_and_undoes_with_the_comment() {
        let (mut doc, page) = doc_with_page();
        let anchor = MotionCommentAnchor {
            clip: AnimationClipId::from_u128(42),
            time_ms: 875,
        };
        let (id, operation) = add_comment_full_op(
            &doc,
            page,
            [11.0, 22.0],
            "Check this transition",
            Vec::new(),
            Vec::new(),
            None,
            Some(anchor),
        )
        .expect("anchored comment");

        doc.apply(operation).expect("apply anchored comment");
        let comment = read_comments(&doc, page).remove(0);
        assert_eq!(comment.id, id);
        assert_eq!(comment.motion_anchor, Some(anchor));
        assert!(comment_is_visible(&comment, Some(anchor.clip)));
        assert!(!comment_is_visible(
            &comment,
            Some(AnimationClipId::from_u128(99))
        ));
        assert!(!comment_is_visible(&comment, None));

        assert!(doc.undo().expect("undo"));
        assert!(read_comments(&doc, page).is_empty());
        assert!(doc.redo().expect("redo"));
        assert_eq!(read_comments(&doc, page)[0].motion_anchor, Some(anchor));

        doc.apply(reply_comment_op(&doc, page, &id, "Looks good").expect("reply"))
            .expect("apply reply");
        assert_eq!(read_comments(&doc, page)[0].motion_anchor, Some(anchor));
        doc.apply(update_comment_text_op(&doc, page, &id, "Updated").expect("edit"))
            .expect("apply edit");
        assert_eq!(read_comments(&doc, page)[0].motion_anchor, Some(anchor));
        doc.apply(toggle_resolved_op(&doc, page, &id).expect("resolve"))
            .expect("apply resolve");
        assert_eq!(read_comments(&doc, page)[0].motion_anchor, Some(anchor));
    }

    #[test]
    fn motion_anchor_survives_fnx_write_and_reopen() -> anyhow::Result<()> {
        let (mut doc, page) = doc_with_page();
        let anchor = MotionCommentAnchor {
            clip: AnimationClipId::from_u128(42),
            time_ms: 875,
        };
        let (_, operation) = add_comment_full_op(
            &doc,
            page,
            [11.0, 22.0],
            "Persist this moment",
            Vec::new(),
            Vec::new(),
            None,
            Some(anchor),
        )
        .expect("anchored comment");
        doc.apply(operation)?;

        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        let (reopened, _) = fanta_format::read_project_tree(directory.path())?;
        let comment = read_comments(&reopened, page).remove(0);
        assert_eq!(comment.text, "Persist this moment");
        assert_eq!(comment.world, [11.0, 22.0]);
        assert_eq!(comment.motion_anchor, Some(anchor));
        Ok(())
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
