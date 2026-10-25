//! Change events. A subscriber names the nodes it follows, or all of them, and
//! polls for what happened to them since its last poll. Each node appears at
//! most once per poll, as created, updated or destroyed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fanta_doc::{Doc, ModeScope, NodeId, Operation};

use crate::{ApiError, DesignSession, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeEventKind {
    Created,
    Updated,
    Destroyed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeEvent {
    pub node: NodeId,
    pub kind: NodeEventKind,
}

/// Which nodes a subscription follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeFilter {
    All,
    Nodes(BTreeSet<NodeId>),
}

impl NodeFilter {
    fn matches(&self, node: NodeId) -> bool {
        match self {
            Self::All => true,
            Self::Nodes(nodes) => nodes.contains(&node),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubscriptionId(u64);

/// Whether an operation is being applied or reverted by undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Apply,
    Revert,
}

#[derive(Default)]
pub(crate) struct EventHub {
    next_id: u64,
    subscriptions: BTreeMap<SubscriptionId, Subscription>,
}

struct Subscription {
    filter: NodeFilter,
    /// Coalesced events in first-seen order; `None` marks an entry a later
    /// event cancelled.
    pending: Vec<Option<NodeEvent>>,
    positions: HashMap<NodeId, usize>,
}

impl EventHub {
    pub(crate) fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }

    fn subscribe(&mut self, filter: NodeFilter) -> SubscriptionId {
        self.next_id += 1;
        let id = SubscriptionId(self.next_id);
        self.subscriptions.insert(
            id,
            Subscription {
                filter,
                pending: Vec::new(),
                positions: HashMap::new(),
            },
        );
        id
    }

    pub(crate) fn record(&mut self, events: &[NodeEvent]) {
        for subscription in self.subscriptions.values_mut() {
            for event in events {
                if subscription.filter.matches(event.node) {
                    subscription.push(*event);
                }
            }
        }
    }
}

impl Subscription {
    fn push(&mut self, event: NodeEvent) {
        let Some(&position) = self.positions.get(&event.node) else {
            self.positions.insert(event.node, self.pending.len());
            self.pending.push(Some(event));
            return;
        };
        let Some(slot) = self.pending.get_mut(position) else {
            return;
        };
        let Some(previous) = *slot else {
            *slot = Some(event);
            return;
        };
        use NodeEventKind::{Created, Destroyed, Updated};
        let kind = match (previous.kind, event.kind) {
            // Made and removed within one poll: the subscriber never saw it.
            (Created, Destroyed) => None,
            (Created, Created | Updated) => Some(Created),
            // Removed and restored (an undone delete): it existed before and
            // still does.
            (Destroyed, Created) => Some(Updated),
            (Destroyed, Destroyed | Updated) => Some(Destroyed),
            (Updated, Destroyed) => Some(Destroyed),
            (Updated, Created | Updated) => Some(Updated),
        };
        *slot = kind.map(|kind| NodeEvent {
            node: event.node,
            kind,
        });
    }

    fn drain(&mut self) -> Vec<NodeEvent> {
        self.positions.clear();
        self.pending.drain(..).flatten().collect()
    }
}

impl DesignSession {
    /// Follow changes to the nodes `filter` names, from now on.
    pub fn subscribe(&mut self, filter: NodeFilter) -> SubscriptionId {
        self.events.subscribe(filter)
    }

    /// Stop following changes. Returns whether the subscription existed.
    pub fn unsubscribe(&mut self, subscription: SubscriptionId) -> bool {
        self.events.subscriptions.remove(&subscription).is_some()
    }

    /// What happened to the followed nodes since the last poll, one event
    /// per node in the order they first changed.
    pub fn poll_events(&mut self, subscription: SubscriptionId) -> Result<Vec<NodeEvent>> {
        self.events
            .subscriptions
            .get_mut(&subscription)
            .map(Subscription::drain)
            .ok_or_else(|| ApiError::InvalidArgument("the subscription does not exist".into()))
    }
}

/// The node events `operation` causes when applied or reverted, given the
/// document as the operation left it. Every variant is listed, so a new
/// operation does not compile until it is classified here.
pub(crate) fn node_events(
    doc: &Doc,
    operation: &Operation,
    direction: Direction,
    out: &mut Vec<NodeEvent>,
) {
    let mut updated = |node: NodeId| {
        out.push(NodeEvent {
            node,
            kind: NodeEventKind::Updated,
        })
    };
    let lifecycle = |node: NodeId, created_when_applied: bool| NodeEvent {
        node,
        kind: if created_when_applied == (direction == Direction::Apply) {
            NodeEventKind::Created
        } else {
            NodeEventKind::Destroyed
        },
    };
    let component_root = |component| {
        doc.components
            .def(component)
            .map(|definition| definition.root)
    };
    match operation {
        Operation::CreateNode { node } | Operation::CreateInstance { node } => {
            out.push(lifecycle(node.id, true));
        }
        Operation::DeleteSubtree { snapshot } => {
            out.extend(snapshot.iter().map(|node| lifecycle(node.id, false)));
        }
        Operation::DetachInstance { id, expanded, .. } => {
            updated(*id);
            out.extend(expanded.iter().map(|node| lifecycle(node.id, true)));
        }
        Operation::Reparent {
            id,
            old_parent,
            new_parent,
            ..
        } => {
            updated(*id);
            old_parent
                .iter()
                .chain(new_parent.iter())
                .for_each(|parent| updated(*parent));
        }
        Operation::SetIndex { id, .. }
        | Operation::SetTransform { id, .. }
        | Operation::SetName { id, .. }
        | Operation::SetMeta { id, .. }
        | Operation::SetOpacity { id, .. }
        | Operation::SetBlendMode { id, .. }
        | Operation::SetEffects { id, .. }
        | Operation::SetBlurs { id, .. }
        | Operation::SetFlags { id, .. }
        | Operation::SetMask { id, .. }
        | Operation::SetMaskType { id, .. }
        | Operation::SetLayoutChild { id, .. }
        | Operation::SetConstraints { id, .. }
        | Operation::ReplaceData { id, .. }
        | Operation::SetInstanceOverride { id, .. }
        | Operation::SwapInstance { id, .. }
        | Operation::SetInstanceProp { id, .. } => updated(*id),
        Operation::BindProperty { node, .. }
        | Operation::UnbindProperty { node, .. }
        | Operation::AddReaction { node, .. }
        | Operation::RemoveReaction { node, .. }
        | Operation::SetReaction { node, .. } => updated(*node),
        Operation::DefineComponent { def } | Operation::DeleteComponent { def, .. } => {
            updated(def.root)
        }
        Operation::SetComponentProps { component, .. } => component_root(*component)
            .into_iter()
            .for_each(&mut updated),
        Operation::SetComponentName { id, .. } | Operation::SetVariantMembership { id, .. } => {
            component_root(*id).into_iter().for_each(&mut updated)
        }
        Operation::DefineComponentSet { set } | Operation::DeleteComponentSet { set, .. } => {
            set.root.into_iter().for_each(&mut updated)
        }
        Operation::SetComponentSet { old, new, .. } => old
            .root
            .iter()
            .chain(new.root.iter())
            .for_each(|root| updated(*root)),
        Operation::SetVariableValue { variable, .. } => {
            for root in doc.scene.roots() {
                for id in doc.scene.descendants_of(*root) {
                    if doc
                        .scene
                        .get(id)
                        .is_some_and(|node| node.bindings.values().any(|bound| bound == variable))
                    {
                        updated(id);
                    }
                }
            }
        }
        Operation::SetActiveMode {
            scope: ModeScope::Frame { node },
            ..
        } => updated(*node),
        // Pages, the variable registry, motion clips and prototype flows are
        // document-level state, not node changes. A page's root is created
        // and destroyed by its own node operations.
        Operation::SetActiveMode {
            scope: ModeScope::Doc,
            ..
        }
        | Operation::SetPages { .. }
        | Operation::SetPageRegistry { .. }
        | Operation::CreateVariableCollection { .. }
        | Operation::DeleteVariableCollection { .. }
        | Operation::AddMode { .. }
        | Operation::RemoveMode { .. }
        | Operation::SetCollectionDefaultMode { .. }
        | Operation::CreateVariable { .. }
        | Operation::DeleteVariable { .. }
        | Operation::RenameVariable { .. }
        | Operation::SetVariableOptions { .. }
        | Operation::RenameVariableCollection { .. }
        | Operation::RenameMode { .. }
        | Operation::CreateAnimationClip { .. }
        | Operation::DeleteAnimationClip { .. }
        | Operation::SetAnimationClipName { .. }
        | Operation::SetAnimationClipDuration { .. }
        | Operation::SetAnimationClipSettings { .. }
        | Operation::SetAnimationTrack { .. }
        | Operation::SetKeyframe { .. }
        | Operation::SetFlowStart { .. }
        | Operation::SetFlows { .. }
        | Operation::SetPresentation { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    #[test]
    fn created_updated_destroyed_arrive_once_per_poll() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let subscription = session.subscribe(NodeFilter::All);
        let card = session
            .create_node(page, NodeKind::Rectangle)
            .expect("card");
        session.set_name(card, "Card").expect("rename");
        session.set_float(card, "opacity", 0.5).expect("fade");
        let events = session.poll_events(subscription).expect("poll");
        assert!(events.contains(&NodeEvent {
            node: card,
            kind: NodeEventKind::Created
        }));
        assert_eq!(events.iter().filter(|event| event.node == card).count(), 1);
        assert!(session.poll_events(subscription).expect("poll").is_empty());

        session.set_name(card, "Hero").expect("rename");
        assert_eq!(
            session.poll_events(subscription).expect("poll"),
            vec![NodeEvent {
                node: card,
                kind: NodeEventKind::Updated
            }]
        );
        session.destroy(card).expect("destroy");
        assert!(
            session
                .poll_events(subscription)
                .expect("poll")
                .contains(&NodeEvent {
                    node: card,
                    kind: NodeEventKind::Destroyed
                })
        );
        assert!(session.unsubscribe(subscription));
        assert!(session.poll_events(subscription).is_err());
    }

    #[test]
    fn a_node_created_and_destroyed_before_polling_reports_nothing() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let subscription = session.subscribe(NodeFilter::All);
        let flash = session
            .create_node(page, NodeKind::Ellipse)
            .expect("ellipse");
        session.destroy(flash).expect("destroy");
        let events = session.poll_events(subscription).expect("poll");
        assert!(events.iter().all(|event| event.node != flash), "{events:?}");
    }

    #[test]
    fn filtered_subscriptions_see_only_their_nodes() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let watched = session
            .create_node(page, NodeKind::Rectangle)
            .expect("watched");
        let other = session
            .create_node(page, NodeKind::Rectangle)
            .expect("other");
        let subscription = session.subscribe(NodeFilter::Nodes(BTreeSet::from([watched])));
        session.set_name(other, "Other").expect("rename other");
        session
            .set_name(watched, "Watched")
            .expect("rename watched");
        assert_eq!(
            session.poll_events(subscription).expect("poll"),
            vec![NodeEvent {
                node: watched,
                kind: NodeEventKind::Updated
            }]
        );
    }

    #[test]
    fn undoing_a_create_reports_destroyed() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let card = session
            .create_node(page, NodeKind::Rectangle)
            .expect("card");
        session.add_undo_step("Card").expect("commit");
        let subscription = session.subscribe(NodeFilter::All);
        assert!(session.undo().expect("undo"));
        assert!(
            session
                .poll_events(subscription)
                .expect("poll")
                .contains(&NodeEvent {
                    node: card,
                    kind: NodeEventKind::Destroyed
                })
        );
        assert!(session.redo().expect("redo"));
        assert!(
            session
                .poll_events(subscription)
                .expect("poll")
                .contains(&NodeEvent {
                    node: card,
                    kind: NodeEventKind::Created
                })
        );
    }

    #[test]
    fn layout_settle_reports_reflowed_children() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let row = session.create_node(page, NodeKind::Frame).expect("row");
        let first = session
            .create_node(row, NodeKind::Rectangle)
            .expect("first");
        let second = session
            .create_node(row, NodeKind::Rectangle)
            .expect("second");
        let ops: Vec<fanta_design_ops::DesignOp> = serde_json::from_value(serde_json::json!([
            {"op": "set_auto_layout", "id": row.to_string(), "direction": "horizontal", "gap": 10},
        ]))
        .expect("ops");
        let subscription = session.subscribe(NodeFilter::Nodes(BTreeSet::from([second])));
        session.edit_with_design_op(&ops[0]).expect("auto layout");
        assert_eq!(
            session.poll_events(subscription).expect("poll"),
            vec![NodeEvent {
                node: second,
                kind: NodeEventKind::Updated
            }],
            "the solve moved the second child beside the first"
        );
        assert!(session.is_valid(first));
    }
}
