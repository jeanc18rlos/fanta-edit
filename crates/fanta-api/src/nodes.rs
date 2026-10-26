//! Node lifecycle and lookup: create, duplicate and destroy nodes, move them
//! between parents, find them, and read or write their names, kinds and
//! metadata.

use anyhow::anyhow;
use fanta_design_ops::{DesignNodeType, DesignOp, DesignShape};
use fanta_doc::{CanvasNode, Doc, NodeData, NodeId, Operation, Scope};
use glam::DVec2;
use serde_json::{Map, Value};

use crate::{ApiError, DesignSession, Result};

/// The box a new node gets, in its parent's units.
const NEW_NODE_SIZE: f64 = 100.0;
/// The metadata key holding a node's kind label.
const KIND_KEY: &str = "kind";
/// The metadata key holding a node's string metadata map.
const METADATA_KEY: &str = "metadata";

/// What [`DesignSession::create_node`] makes.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// A clipping container with a background.
    Frame,
    Rectangle,
    Ellipse,
    /// A horizontal line across the node's width.
    Line,
    /// A regular polygon.
    Polygon {
        sides: u32,
    },
    /// A star whose inner radius is `inner_ratio` of its outer one.
    Star {
        points: u32,
        inner_ratio: f64,
    },
    /// An outline from SVG path data, in the node's own 100×100 box.
    Path(String),
    Text(String),
    /// An image from its encoded bytes (PNG, JPEG, WebP or GIF), at its
    /// natural size.
    Image(Vec<u8>),
}

/// A node's broad type, for [`DesignSession::find_by_type`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeType {
    Page,
    Frame,
    Group,
    Vector,
    Text,
    TextPath,
    Image,
    Video,
    Audio,
    Instance,
    Boolean,
    Other,
}

impl NodeType {
    pub fn of(doc: &Doc, node: &CanvasNode) -> Self {
        if doc.pages().contains(&node.id) {
            return Self::Page;
        }
        match &node.data {
            NodeData::Group(_) if node.data.kind_tag() == "frame" => Self::Frame,
            NodeData::Group(_) => Self::Group,
            NodeData::Vector(_) => Self::Vector,
            NodeData::Text(_) => Self::Text,
            NodeData::TextPath(_) => Self::TextPath,
            NodeData::Bitmap(_) => Self::Image,
            NodeData::Video(_) => Self::Video,
            NodeData::Audio(_) => Self::Audio,
            NodeData::Instance(_) => Self::Instance,
            NodeData::Boolean(_) => Self::Boolean,
            NodeData::NodeGraph(_)
            | NodeData::Model3d(_)
            | NodeData::AiArtifact(_)
            | NodeData::Embed(_) => Self::Other,
        }
    }
}

impl DesignSession {
    /// Create a node of `kind` inside `parent`, at the parent's origin.
    pub fn create_node(&mut self, parent: NodeId, kind: NodeKind) -> Result<NodeId> {
        self.permit(Some(parent), Scope::EditorAdd)?;
        let origin = self.child_origin(parent)?;
        let operation = match kind {
            NodeKind::Image(bytes) => return self.create_image(parent, bytes, origin),
            NodeKind::Frame | NodeKind::Rectangle | NodeKind::Ellipse | NodeKind::Text(_) => {
                let (node_type, text) = match kind {
                    NodeKind::Frame => (DesignNodeType::Frame, None),
                    NodeKind::Rectangle => (DesignNodeType::Rectangle, None),
                    NodeKind::Ellipse => (DesignNodeType::Ellipse, None),
                    NodeKind::Text(text) => (DesignNodeType::Text, Some(text)),
                    _ => unreachable!("matched by the outer arm"),
                };
                DesignOp::CreateNode {
                    node_type,
                    parent: Some(parent.to_string()),
                    name: None,
                    x: origin.x,
                    y: origin.y,
                    width: NEW_NODE_SIZE,
                    height: NEW_NODE_SIZE,
                    fill: None,
                    text,
                    font_size: None,
                }
            }
            NodeKind::Line
            | NodeKind::Polygon { .. }
            | NodeKind::Star { .. }
            | NodeKind::Path(_) => {
                let (shape, height, points, inner_ratio, path) = match kind {
                    NodeKind::Line => (DesignShape::Line, 0.0, None, None, None),
                    NodeKind::Polygon { sides } => {
                        (DesignShape::Polygon, NEW_NODE_SIZE, Some(sides), None, None)
                    }
                    NodeKind::Star {
                        points,
                        inner_ratio,
                    } => (
                        DesignShape::Star,
                        NEW_NODE_SIZE,
                        Some(points),
                        Some(inner_ratio),
                        None,
                    ),
                    NodeKind::Path(path) => {
                        (DesignShape::Path, NEW_NODE_SIZE, None, None, Some(path))
                    }
                    _ => unreachable!("matched by the outer arm"),
                };
                DesignOp::CreateShape {
                    shape,
                    parent: Some(parent.to_string()),
                    name: None,
                    x: origin.x,
                    y: origin.y,
                    width: NEW_NODE_SIZE,
                    height,
                    points,
                    inner_ratio,
                    path,
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                }
            }
        };
        self.edit_with_design_op(&operation)?
            .ok_or_else(|| anyhow!("the node was not created").into())
    }

    /// Copy `node` and its subtree into the same parent, one slot above it,
    /// and return the copy.
    pub fn duplicate(&mut self, node: NodeId) -> Result<NodeId> {
        self.permit(Some(node), Scope::LifecycleDuplicate)?;
        self.edit_with_design_op(&DesignOp::Duplicate {
            id: node.to_string(),
            dx: Some(0.0),
            dy: Some(0.0),
        })?
        .ok_or_else(|| anyhow!("the node was not copied").into())
    }

    /// Delete `node` and its subtree. Components defined inside it are
    /// deleted too, and instances of them elsewhere are detached first, as the
    /// editor's Delete does. Every node in the subtree must allow it.
    pub fn destroy(&mut self, node: NodeId) -> Result<()> {
        self.node(node)?;
        for id in self.doc().scene.descendants_of(node) {
            self.permit(Some(id), Scope::LifecycleDestroy)?;
        }
        let operations = fanta_headless::structure::delete_layers_operations(self.doc(), &[node])?;
        self.edit(operations)?;
        let doc = self.document_mut().doc_mut();
        let surviving: Vec<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|selected| doc.scene.get(*selected).is_some())
            .collect();
        doc.selection.replace_with(surviving);
        Ok(())
    }

    /// Wrap `nodes` in a new group and return it.
    pub fn group(&mut self, nodes: &[NodeId]) -> Result<NodeId> {
        for node in nodes {
            self.permit(Some(*node), Scope::LayerMove)?;
        }
        self.edit_with_design_op(&DesignOp::Group {
            ids: nodes.iter().map(ToString::to_string).collect(),
            name: None,
        })?
        .ok_or_else(|| anyhow!("the group was not created").into())
    }

    /// Replace the group `node` with its children, which keep their place on
    /// the canvas.
    pub fn ungroup(&mut self, node: NodeId) -> Result<()> {
        self.permit(Some(node), Scope::LayerMove)?;
        for child in self.doc().scene.children_of(Some(node)) {
            self.permit(Some(*child), Scope::LayerMove)?;
        }
        self.edit_with_design_op(&DesignOp::Ungroup {
            id: node.to_string(),
        })?;
        Ok(())
    }

    pub fn is_valid(&self, node: NodeId) -> bool {
        self.doc().scene.get(node).is_some()
    }

    pub fn node_type(&self, node: NodeId) -> Result<NodeType> {
        Ok(NodeType::of(self.doc(), self.node(node)?))
    }

    pub fn parent(&self, node: NodeId) -> Result<Option<NodeId>> {
        Ok(self.node(node)?.parent)
    }

    /// `node`'s children, bottom to top.
    pub fn children(&self, node: NodeId) -> Result<Vec<NodeId>> {
        self.node(node)?;
        Ok(self.doc().scene.children_of(Some(node)).to_vec())
    }

    /// Move `child` into `parent` at `index` among its children (0 is the
    /// bottom), keeping its position on the canvas.
    pub fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) -> Result<()> {
        self.reparent(parent, child, Some(index))
    }

    /// Move `child` to the top of `parent`'s children, keeping its position
    /// on the canvas.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        self.reparent(parent, child, None)
    }

    /// Every node named `name`, in tree order.
    pub fn find_by_name(&self, name: &str) -> Vec<NodeId> {
        self.find(|_, node| node.name == name)
    }

    pub fn find_by_type(&self, node_type: NodeType) -> Vec<NodeId> {
        self.find(|doc, node| NodeType::of(doc, node) == node_type)
    }

    /// Every node whose kind label is `kind`.
    pub fn find_by_kind(&self, kind: &str) -> Vec<NodeId> {
        self.find(|_, node| node.meta.get(KIND_KEY).and_then(Value::as_str) == Some(kind))
    }

    pub fn name(&self, node: NodeId) -> Result<String> {
        Ok(self.node(node)?.name.clone())
    }

    pub fn set_name(&mut self, node: NodeId, name: &str) -> Result<()> {
        let old = self.node(node)?.name.clone();
        if old == name {
            return Ok(());
        }
        self.edit(vec![Operation::SetName {
            id: node,
            old,
            new: name.to_owned(),
        }])
    }

    /// The free-form kind label a caller gave `node`, such as `"logo"`.
    pub fn kind(&self, node: NodeId) -> Result<Option<String>> {
        Ok(self
            .node(node)?
            .meta
            .get(KIND_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    pub fn set_kind(&mut self, node: NodeId, kind: Option<&str>) -> Result<()> {
        self.update_meta(node, |meta| match kind {
            Some(kind) => {
                meta.insert(KIND_KEY.into(), Value::String(kind.to_owned()));
            }
            None => {
                meta.remove(KIND_KEY);
            }
        })
    }

    pub fn metadata(&self, node: NodeId, key: &str) -> Result<Option<String>> {
        Ok(self
            .node(node)?
            .meta
            .get(METADATA_KEY)
            .and_then(|metadata| metadata.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    /// The keys of `node`'s string metadata, sorted.
    pub fn metadata_keys(&self, node: NodeId) -> Result<Vec<String>> {
        let mut keys: Vec<String> = self
            .node(node)?
            .meta
            .get(METADATA_KEY)
            .and_then(Value::as_object)
            .map(|metadata| metadata.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        Ok(keys)
    }

    pub fn set_metadata(&mut self, node: NodeId, key: &str, value: &str) -> Result<()> {
        self.update_meta(node, |meta| {
            let metadata = meta
                .entry(METADATA_KEY)
                .or_insert_with(|| Value::Object(Map::new()));
            if !metadata.is_object() {
                *metadata = Value::Object(Map::new());
            }
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.insert(key.to_owned(), Value::String(value.to_owned()));
            }
        })
    }

    pub fn remove_metadata(&mut self, node: NodeId, key: &str) -> Result<()> {
        self.update_meta(node, |meta| {
            let emptied = match meta.get_mut(METADATA_KEY).and_then(Value::as_object_mut) {
                Some(metadata) => {
                    metadata.remove(key);
                    metadata.is_empty()
                }
                None => false,
            };
            if emptied {
                meta.remove(METADATA_KEY);
            }
        })
    }

    /// Replace the selection. Selection is not part of undo history.
    pub fn select(&mut self, nodes: &[NodeId]) -> Result<()> {
        for node in nodes {
            self.permit(Some(*node), Scope::EditorSelect)?;
        }
        self.document_mut()
            .doc_mut()
            .selection
            .replace_with(nodes.iter().copied());
        Ok(())
    }

    pub fn selection(&self) -> Vec<NodeId> {
        self.doc().selection.iter().copied().collect()
    }

    pub(crate) fn node(&self, node: NodeId) -> Result<&CanvasNode> {
        self.doc()
            .scene
            .get(node)
            .ok_or(ApiError::NodeNotFound(node))
    }

    /// The world point a new child of `parent` is placed at: its origin.
    fn child_origin(&self, parent: NodeId) -> Result<DVec2> {
        let node = self.node(parent)?;
        if !node.can_have_children() {
            return Err(ApiError::InvalidArgument(format!(
                "{parent} cannot contain other nodes"
            )));
        }
        let transform = self
            .doc()
            .scene
            .world_transform(parent)
            .ok_or(ApiError::NodeNotFound(parent))?;
        Ok(transform.transform_point(DVec2::ZERO))
    }

    fn create_image(&mut self, parent: NodeId, bytes: Vec<u8>, origin: DVec2) -> Result<NodeId> {
        let (asset, natural_size, inserted) = self.document_mut().ingest_image(bytes)?;
        let size = natural_size.map(|side| f64::from(side.max(1)));
        let node = match fanta_headless::structure::image_layer_node(
            self.doc(),
            asset,
            natural_size,
            size,
            Some(parent),
            origin.x,
            origin.y,
            None,
        ) {
            Ok(node) => node,
            Err(error) => {
                if inserted {
                    self.document_mut().remove_image(asset);
                }
                return Err(error.into());
            }
        };
        let id = node.id;
        if let Err(error) = self.edit(vec![Operation::create_node(node)]) {
            if inserted {
                self.document_mut().remove_image(asset);
            }
            return Err(error);
        }
        Ok(id)
    }

    fn reparent(&mut self, parent: NodeId, child: NodeId, index: Option<usize>) -> Result<()> {
        self.permit(Some(child), Scope::LayerMove)?;
        if !self.node(parent)?.can_have_children() {
            return Err(ApiError::InvalidArgument(format!(
                "{parent} cannot contain other nodes"
            )));
        }
        self.edit_with_design_op(&DesignOp::Reparent {
            id: child.to_string(),
            parent: Some(parent.to_string()),
            index,
        })?;
        Ok(())
    }

    pub(crate) fn find(&self, matches: impl Fn(&Doc, &CanvasNode) -> bool) -> Vec<NodeId> {
        let doc = self.doc();
        doc.scene
            .roots()
            .iter()
            .flat_map(|root| doc.scene.descendants_of(*root))
            .filter(|id| doc.scene.get(*id).is_some_and(|node| matches(doc, node)))
            .collect()
    }

    fn update_meta(
        &mut self,
        node: NodeId,
        change: impl FnOnce(&mut Map<String, Value>),
    ) -> Result<()> {
        let old = self.node(node)?.meta.clone();
        let mut meta = match &old {
            Value::Object(meta) => meta.clone(),
            _ => Map::new(),
        };
        change(&mut meta);
        let new = if meta.is_empty() && !old.is_object() {
            old.clone()
        } else {
            Value::Object(meta)
        };
        if new == old {
            return Ok(());
        }
        self.edit(vec![Operation::SetMeta { id: node, old, new }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([0, 128, 255, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        bytes
    }

    #[test]
    fn create_duplicate_destroy_and_undo_restore_the_tree() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let frame = session.create_node(page, NodeKind::Frame).expect("frame");
        let text = session
            .create_node(frame, NodeKind::Text("Hello".into()))
            .expect("text");
        let star = session
            .create_node(
                frame,
                NodeKind::Star {
                    points: 5,
                    inner_ratio: 0.4,
                },
            )
            .expect("star");
        let image = session
            .create_node(frame, NodeKind::Image(png(4, 2)))
            .expect("image");
        assert_eq!(
            session.children(frame).expect("children"),
            vec![text, star, image]
        );
        assert_eq!(session.node_type(image).expect("type"), NodeType::Image);
        assert_eq!(session.node_type(frame).expect("type"), NodeType::Frame);
        assert_eq!(session.node_type(page).expect("type"), NodeType::Page);
        session.add_undo_step("Build").expect("commit");

        let copy = session.duplicate(frame).expect("duplicate");
        assert_eq!(session.parent(copy).expect("parent"), Some(page));
        assert_eq!(session.children(copy).expect("copy children").len(), 3);
        session.destroy(frame).expect("destroy");
        assert!(!session.is_valid(frame) && !session.is_valid(text));
        assert!(session.is_valid(copy));
        session.add_undo_step("Copy and delete").expect("commit");

        assert!(session.undo().expect("undo"));
        assert!(session.is_valid(frame) && session.is_valid(text));
        assert!(!session.is_valid(copy));
    }

    #[test]
    fn destroy_releases_component_masters_like_the_editor() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let card = session.create_node(page, NodeKind::Frame).expect("card");
        session
            .edit_with_design_op(&DesignOp::CreateComponent {
                id: card.to_string(),
            })
            .expect("componentize");
        let component = session
            .doc()
            .components
            .defs
            .values()
            .find(|definition| definition.root == card)
            .map(|definition| definition.id)
            .expect("a component");
        let instance = session
            .edit_with_design_op(&DesignOp::CreateInstance {
                component: component.to_string(),
                x: 300.0,
                y: 0.0,
                parent: None,
                name: None,
            })
            .expect("place an instance")
            .expect("instance id");
        session.destroy(card).expect("delete the master");
        assert!(session.doc().components.defs.is_empty());
        assert_eq!(
            session.node_type(instance).expect("detached copy"),
            NodeType::Frame,
            "the surviving instance is detached, not left dangling"
        );
    }

    #[test]
    fn insert_child_keeps_world_position_and_order() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let first = session.create_node(page, NodeKind::Frame).expect("first");
        let second = session.create_node(page, NodeKind::Frame).expect("second");
        let dot = session.create_node(page, NodeKind::Ellipse).expect("dot");
        let before = session.doc().scene.world_bounds(dot).expect("bounds");
        session.append_child(first, dot).expect("move into first");
        session
            .insert_child(first, second, 0)
            .expect("move second under the dot");
        assert_eq!(
            session.children(first).expect("children"),
            vec![second, dot]
        );
        assert_eq!(session.doc().scene.world_bounds(dot), Some(before));
        assert!(
            session.append_child(dot, first).is_err(),
            "an ellipse cannot hold children"
        );
    }

    #[test]
    fn find_by_name_type_and_kind() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let logo = session
            .create_node(page, NodeKind::Rectangle)
            .expect("logo");
        let title = session
            .create_node(page, NodeKind::Text("Title".into()))
            .expect("title");
        session.set_name(logo, "Logo").expect("name");
        session.set_kind(logo, Some("brand")).expect("kind");
        assert_eq!(session.find_by_name("Logo"), vec![logo]);
        assert_eq!(session.find_by_type(NodeType::Text), vec![title]);
        assert_eq!(session.find_by_kind("brand"), vec![logo]);
        assert_eq!(session.kind(logo).expect("kind"), Some("brand".into()));
        session.set_kind(logo, None).expect("clear kind");
        assert!(session.find_by_kind("brand").is_empty());
        assert!(session.find_by_type(NodeType::Page).contains(&page));
    }

    #[test]
    fn metadata_survives_writing_and_reopening_a_project() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let badge = session
            .create_node(page, NodeKind::Rectangle)
            .expect("badge");
        session
            .set_metadata(badge, "sku", "A-17")
            .expect("metadata");
        session
            .set_metadata(badge, "campaign", "fall")
            .expect("metadata");
        session.remove_metadata(badge, "campaign").expect("remove");
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("Design");
        session.write_project(&root).expect("write the project");
        assert!(
            session.write_project(&root).is_err(),
            "refuses an existing project"
        );

        let mut reopened = DesignSession::open_project(&root).expect("reopen");
        assert_eq!(
            reopened.metadata(badge, "sku").expect("sku"),
            Some("A-17".into())
        );
        assert_eq!(
            reopened.metadata_keys(badge).expect("keys"),
            vec!["sku".to_owned()]
        );
        reopened.set_name(badge, "Badge").expect("rename");
        reopened.save().expect("save");
        let again = DesignSession::open_project(&root).expect("reopen again");
        assert_eq!(again.name(badge).expect("name"), "Badge");
        assert!(matches!(
            DesignSession::new().expect("session").save(),
            Err(ApiError::NoProject)
        ));
    }

    #[test]
    fn snapshots_round_trip_the_design_and_its_assets() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let image = session
            .create_node(page, NodeKind::Image(png(3, 3)))
            .expect("image");
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("design.fant");
        session.write_snapshot(&path).expect("write the snapshot");
        let reopened = DesignSession::open_snapshot(&path).expect("open the snapshot");
        let NodeData::Bitmap(bitmap) = &reopened.node(image).expect("image").data else {
            panic!("the image survives");
        };
        assert!(reopened.document().raw_assets().contains_key(&bitmap.asset));
        assert_eq!(reopened.active_page(), Some(page));
    }

    #[test]
    fn design_sessions_can_move_between_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<DesignSession>();
    }
}
