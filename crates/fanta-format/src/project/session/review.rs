//! In-memory agent proposal review.
//!
//! Reviews are drafts: resolving one conflict never mutates the live canvas.
//! The complete draft is installed atomically only after every conflict has a
//! decision and the artifact generation still matches.

use super::error::SessionError;
use super::hash::ContentHash;
use super::types::{ArtifactId, SourceSync};
use crate::project::merge::{
    ArtifactAddress, ArtifactMerge, JsonPathSegment, NodeMapEdition, PresenceValue,
    PropertyConflict,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct MergeReview {
    pub artifact: ArtifactId,
    pub expected_generation: u64,
    pub expected_base_hash: ContentHash,
    pub candidate_hash: ContentHash,
    pub base: Arc<NodeMapEdition>,
    pub ours: Arc<NodeMapEdition>,
    pub theirs: Arc<NodeMapEdition>,
    pub draft: NodeMapEdition,
    pub conflicts: Vec<ReviewConflict>,
}

impl MergeReview {
    pub(crate) fn new(
        artifact: ArtifactId,
        expected_generation: u64,
        expected_base_hash: ContentHash,
        candidate_hash: ContentHash,
        base: Arc<NodeMapEdition>,
        ours: NodeMapEdition,
        theirs: NodeMapEdition,
        merged: ArtifactMerge,
    ) -> Self {
        let conflicts = merged
            .property_conflicts
            .into_iter()
            .enumerate()
            .map(|(index, conflict)| ReviewConflict {
                id: index as u32,
                conflict,
                resolution: None,
            })
            .collect();
        Self {
            artifact,
            expected_generation,
            expected_base_hash,
            candidate_hash,
            base,
            ours: Arc::new(ours),
            theirs: Arc::new(theirs),
            draft: NodeMapEdition::new(merged.header, merged.nodes),
            conflicts,
        }
    }

    pub fn is_resolved(&self) -> bool {
        self.conflicts
            .iter()
            .all(|conflict| conflict.resolution.is_some())
    }

    /// Resolve one draft conflict. This changes only the review draft.
    pub fn resolve(
        &mut self,
        conflict_id: u32,
        resolution: ReviewResolution,
    ) -> Result<(), SessionError> {
        let conflict = self
            .conflicts
            .iter_mut()
            .find(|conflict| conflict.id == conflict_id)
            .ok_or(SessionError::ReviewConflictNotFound { conflict_id })?;
        let value = match &resolution {
            ReviewResolution::Ours => conflict.conflict.ours.clone(),
            ReviewResolution::Theirs => conflict.conflict.theirs.clone(),
            ReviewResolution::Base => conflict.conflict.base.clone(),
            ReviewResolution::Custom { value } => value.clone(),
        };
        apply_presence(&mut self.draft, &conflict.conflict.address, value)?;
        conflict.resolution = Some(resolution);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewConflict {
    pub id: u32,
    pub conflict: PropertyConflict,
    pub resolution: Option<ReviewResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "choice")]
pub enum ReviewResolution {
    Ours,
    Theirs,
    Base,
    Custom { value: PresenceValue },
}

#[derive(Debug, Clone)]
pub enum SourceProposalOutcome {
    Applied(ProposalApplied),
    Review(MergeReview),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalApplied {
    pub generation: u64,
    pub source_sync: SourceSync,
}

fn apply_presence(
    edition: &mut NodeMapEdition,
    address: &ArtifactAddress,
    value: PresenceValue,
) -> Result<(), SessionError> {
    match address {
        ArtifactAddress::Header { path } => set_path(&mut edition.header, path, value),
        ArtifactAddress::Node { id, path } if path.is_empty() => {
            match value {
                PresenceValue::Missing => {
                    edition.nodes.remove(id);
                }
                PresenceValue::Present(value) => {
                    edition.nodes.insert(id.clone(), value);
                }
            }
            Ok(())
        }
        ArtifactAddress::Node { id, path } => {
            let node = edition
                .nodes
                .get_mut(id)
                .ok_or_else(|| SessionError::other(format!("review node {id} is missing")))?;
            set_path(node, path, value)
        }
        ArtifactAddress::Artifact { path } => {
            let mut root = Value::Object(Map::from_iter([
                ("header".to_owned(), edition.header.clone()),
                ("nodes".to_owned(), Value::Object(edition.nodes.clone())),
            ]));
            set_path(&mut root, path, value)?;
            let Value::Object(mut root) = root else {
                return Err(SessionError::other(
                    "artifact-level review produced a non-object root",
                ));
            };
            edition.header = root.remove("header").unwrap_or(Value::Null);
            edition.nodes = root
                .remove("nodes")
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            Ok(())
        }
    }
}

fn set_path(
    root: &mut Value,
    path: &[JsonPathSegment],
    value: PresenceValue,
) -> Result<(), SessionError> {
    let Some((head, tail)) = path.split_first() else {
        return match value {
            PresenceValue::Present(value) => {
                *root = value;
                Ok(())
            }
            PresenceValue::Missing => Err(SessionError::other(
                "cannot remove the root value of a review scope",
            )),
        };
    };
    match head {
        JsonPathSegment::Key(key) => {
            let object = root.as_object_mut().ok_or_else(|| {
                SessionError::other(format!("review path expected object before key {key}"))
            })?;
            if tail.is_empty() {
                match value {
                    PresenceValue::Missing => {
                        object.remove(key);
                    }
                    PresenceValue::Present(value) => {
                        object.insert(key.clone(), value);
                    }
                }
                return Ok(());
            }
            let child = object
                .entry(key.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            set_path(child, tail, value)
        }
        JsonPathSegment::Index(index) => {
            let array = root.as_array_mut().ok_or_else(|| {
                SessionError::other(format!("review path expected array before index {index}"))
            })?;
            if *index >= array.len() {
                return Err(SessionError::other(format!(
                    "review array index {index} is out of bounds"
                )));
            }
            if tail.is_empty() {
                match value {
                    PresenceValue::Missing => {
                        array.remove(*index);
                    }
                    PresenceValue::Present(value) => {
                        array[*index] = value;
                    }
                }
                return Ok(());
            }
            set_path(&mut array[*index], tail, value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::merge::{ArtifactAddress, JsonPathSegment};
    use serde_json::json;

    #[test]
    fn resolving_missing_is_distinct_from_resolving_null() {
        let mut edition = NodeMapEdition::new(
            json!({}),
            Map::from_iter([("n".into(), json!({"meta": {"key": "value"}}))]),
        );
        let address = ArtifactAddress::Node {
            id: "n".into(),
            path: vec![
                JsonPathSegment::Key("meta".into()),
                JsonPathSegment::Key("key".into()),
            ],
        };
        apply_presence(&mut edition, &address, PresenceValue::Missing).unwrap();
        assert!(edition.nodes["n"]["meta"].get("key").is_none());
        apply_presence(&mut edition, &address, PresenceValue::Present(Value::Null)).unwrap();
        assert!(edition.nodes["n"]["meta"]["key"].is_null());
    }
}
