use std::sync::Arc;

use fanta_doc::{Doc, NodeId};
use fanta_present::{KeyEvent, PointerEvent, PresentResponse, PresentSession};
use fanta_render::AssetResolver;
use glam::DVec2;

pub(crate) struct PrototypePlayerState {
    session: PresentSession,
    entry_frame: NodeId,
    presentation_frames: Vec<NodeId>,
    screen_size: DVec2,
}

impl PrototypePlayerState {
    pub(crate) fn try_start(
        document: &Doc,
        asset_resolver: Option<Arc<dyn AssetResolver>>,
        screen_size: DVec2,
    ) -> anyhow::Result<Self> {
        let entry_frame = prototype_entry_frame(document)
            .ok_or_else(|| anyhow::anyhow!("Add a frame before presenting this prototype"))?;
        let screen_size = valid_screen_size(screen_size);
        let mut session = PresentSession::new(document, Some(entry_frame), screen_size)?;
        if let Some(asset_resolver) = asset_resolver {
            session.set_asset_resolver(asset_resolver);
        }
        Ok(Self {
            session,
            entry_frame,
            presentation_frames: prototype_frames(document, entry_frame),
            screen_size,
        })
    }

    pub(crate) fn current_frame(&self) -> NodeId {
        self.session.current_frame()
    }

    pub(crate) fn active_frame(&self) -> NodeId {
        self.session.current_frame()
    }

    pub(crate) fn frame_position(&self) -> Option<(usize, usize)> {
        let index = self
            .presentation_frames
            .iter()
            .position(|frame| *frame == self.current_frame())?;
        Some((index + 1, self.presentation_frames.len()))
    }

    pub(crate) fn show_previous_frame(&mut self) -> bool {
        let Some(index) = self
            .presentation_frames
            .iter()
            .position(|frame| *frame == self.current_frame())
        else {
            return false;
        };
        let Some(previous) = index
            .checked_sub(1)
            .and_then(|index| self.presentation_frames.get(index))
            .copied()
        else {
            return false;
        };
        self.rebuild(previous)
    }

    pub(crate) fn show_next_frame(&mut self) -> bool {
        let Some(index) = self
            .presentation_frames
            .iter()
            .position(|frame| *frame == self.current_frame())
        else {
            return false;
        };
        let Some(next) = self.presentation_frames.get(index + 1).copied() else {
            return false;
        };
        self.rebuild(next)
    }

    pub(crate) fn restart(&mut self) -> bool {
        self.rebuild(self.entry_frame)
    }

    pub(crate) fn resize(&mut self, screen_size: DVec2, display_scale: f64) -> anyhow::Result<()> {
        let screen_size = valid_screen_size(screen_size);
        self.session.resize_with_scale(screen_size, display_scale)?;
        self.screen_size = screen_size;
        Ok(())
    }

    pub(crate) fn handle_pointer(&mut self, point: DVec2, event: PointerEvent) -> PresentResponse {
        self.session.handle_pointer(point, event)
    }

    pub(crate) fn handle_key(&mut self, key: &str) -> PresentResponse {
        self.session.handle_key(key, KeyEvent::Down)
    }

    pub(crate) fn tick_elapsed(&mut self, elapsed: std::time::Duration) -> PresentResponse {
        self.session.tick(elapsed.as_secs_f64())
    }

    pub(crate) fn take_open_url(&mut self) -> Option<String> {
        self.session.take_open_url()
    }

    pub(crate) fn present_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        let (width, height) = self.session.pixel_size();
        (width, height, self.session.present_rgba())
    }

    fn rebuild(&mut self, start: NodeId) -> bool {
        if !self.session.restart_at(start) {
            return false;
        }
        true
    }
}

pub(crate) fn prototype_entry_frame(document: &Doc) -> Option<NodeId> {
    if let Some(start) = document.flow_start()
        && is_frame_surface(document, start)
    {
        return Some(start);
    }
    let candidates = document
        .active_page()
        .or_else(|| document.pages().first().copied())
        .map(|page| document.scene.children_of(Some(page)).to_vec())
        .unwrap_or_else(|| document.scene.roots().to_vec());
    candidates
        .iter()
        .copied()
        .find(|node| is_frame_surface(document, *node))
}

fn prototype_frames(document: &Doc, entry_frame: NodeId) -> Vec<NodeId> {
    let page = document.pages().iter().copied().find(|page| {
        *page == entry_frame
            || document
                .scene
                .ancestors_of(entry_frame)
                .any(|ancestor| ancestor.id == *page)
    });
    let mut frames = match page {
        Some(page) => document
            .scene
            .children_of(Some(page))
            .iter()
            .copied()
            .filter(|node| is_frame_surface(document, *node))
            .collect::<Vec<_>>(),
        None => document
            .scene
            .roots()
            .iter()
            .copied()
            .filter(|node| is_frame_surface(document, *node))
            .collect(),
    };
    if !frames.contains(&entry_frame) {
        frames.insert(0, entry_frame);
    }
    frames
}

fn is_frame_surface(document: &Doc, node: NodeId) -> bool {
    document.scene.get(node).is_some_and(|node| {
        matches!(
            &node.data,
            fanta_doc::NodeData::Group(group) if group.is_frame_surface()
        )
    })
}

fn valid_screen_size(screen_size: DVec2) -> DVec2 {
    DVec2::new(screen_size.x.max(1.0), screen_size.y.max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, NodeData, Operation};

    fn page(document: &mut Doc, name: &str) -> NodeId {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.name = name.into();
        let id = node.id;
        document
            .apply(Operation::create_node(node))
            .expect("create page");
        document.add_page(id);
        id
    }

    fn frame(document: &mut Doc, parent: NodeId, name: &str) -> NodeId {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([320.0, 180.0]),
            ..GroupNode::default()
        }));
        node.parent = Some(parent);
        node.index = document.scene.next_child_index(Some(parent));
        node.name = name.into();
        let id = node.id;
        document
            .apply(Operation::create_node(node))
            .expect("create frame");
        id
    }

    fn set_flow_start(document: &mut Doc, node: NodeId) {
        document
            .apply(Operation::SetFlowStart {
                old: document.flow_start(),
                new: Some(node),
            })
            .expect("set flow start");
    }

    #[test]
    fn flow_start_wins_and_missing_start_falls_back_to_scene_order() {
        let mut document = Doc::new();
        let page = page(&mut document, "Page");
        frame(&mut document, page, "First");
        let second = frame(&mut document, page, "Second");
        let fallback = document.scene.children_of(Some(page))[0];
        assert_eq!(prototype_entry_frame(&document), Some(fallback));
        set_flow_start(&mut document, second);
        assert_eq!(prototype_entry_frame(&document), Some(second));
    }

    #[test]
    fn presentation_controls_walk_top_level_frames_and_restart() {
        let mut document = Doc::new();
        let page = page(&mut document, "Page");
        frame(&mut document, page, "First");
        frame(&mut document, page, "Second");
        let frames = document.scene.children_of(Some(page)).to_vec();
        let first = frames[0];
        let second = frames[1];
        set_flow_start(&mut document, first);
        let mut player = PrototypePlayerState::try_start(&document, None, DVec2::new(800.0, 600.0))
            .expect("player");
        assert_eq!(player.frame_position(), Some((1, 2)));
        assert!(!player.show_previous_frame());
        assert!(player.show_next_frame());
        assert_eq!(player.current_frame(), second);
        assert_eq!(player.frame_position(), Some((2, 2)));
        assert!(player.restart());
        assert_eq!(player.current_frame(), first);
    }

    #[test]
    fn resize_keeps_the_active_frame() {
        let mut document = Doc::new();
        let page = page(&mut document, "Page");
        let frame = frame(&mut document, page, "Frame");
        set_flow_start(&mut document, frame);
        let mut player = PrototypePlayerState::try_start(&document, None, DVec2::new(800.0, 600.0))
            .expect("player");
        player
            .resize(DVec2::new(1200.0, 700.0), 1.0)
            .expect("resize presentation");
        assert_eq!(player.current_frame(), frame);
        let (width, height, pixels) = player.present_rgba();
        assert_eq!((width, height), (1200, 700));
        assert_eq!(pixels.len(), 1200 * 700 * 4);
    }

    #[test]
    fn a_page_without_frames_is_not_presentable() {
        let mut document = Doc::new();
        page(&mut document, "Page");
        let result = PrototypePlayerState::try_start(&document, None, DVec2::new(800.0, 600.0));
        let Err(error) = result else {
            panic!("a page is not a presentation frame");
        };
        assert!(error.to_string().contains("Add a frame"));
    }

    #[test]
    fn an_invalid_flow_start_falls_back_to_the_active_pages_first_frame() {
        let mut document = Doc::new();
        let page = page(&mut document, "Page");
        let first = frame(&mut document, page, "First");
        let invalid = NodeId::new();
        document.flow_start = Some(invalid);
        assert_eq!(prototype_entry_frame(&document), Some(first));
    }

    #[test]
    fn a_legacy_root_frame_is_presentable_without_an_explicit_page() {
        let mut document = Doc::new();
        let node = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([320.0, 180.0]),
            ..GroupNode::default()
        }));
        let frame = node.id;
        document
            .apply(Operation::create_node(node))
            .expect("create root frame");
        assert_eq!(prototype_entry_frame(&document), Some(frame));
        assert!(PrototypePlayerState::try_start(&document, None, DVec2::new(800.0, 600.0)).is_ok());
    }

    #[test]
    fn manual_navigation_reuses_the_scaled_render_surface() {
        let mut document = Doc::new();
        let page = page(&mut document, "Page");
        let first = frame(&mut document, page, "First");
        frame(&mut document, page, "Second");
        set_flow_start(&mut document, first);
        let mut player = PrototypePlayerState::try_start(&document, None, DVec2::new(400.0, 300.0))
            .expect("player");
        player
            .resize(DVec2::new(400.0, 300.0), 2.0)
            .expect("retina surface");
        assert!(player.show_next_frame());
        let (width, height, _) = player.present_rgba();
        assert_eq!((width, height), (800, 600));
    }
}
