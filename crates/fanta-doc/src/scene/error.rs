//! [`SceneError`] — the error type returned by [`Scene`](crate::scene::Scene)
//! mutating methods.

use crate::id::NodeId;

/// Errors returned by [`Scene`](crate::scene::Scene) mutating methods.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("node {0} not found")]
    NotFound(NodeId),
    #[error("node {0} already exists in scene")]
    Duplicate(NodeId),
    #[error("parent {0} does not exist")]
    ParentMissing(NodeId),
    #[error("parent {0} cannot have children (variant does not support containment)")]
    ParentNotContainer(NodeId),
    #[error("reparent would create a cycle (descendant {descendant} -> ancestor {ancestor})")]
    Cycle {
        descendant: NodeId,
        ancestor: NodeId,
    },
    #[error("scene invariant violated: {0}")]
    InvariantViolated(String),
}
