//! Identifier types.
//!
//! All IDs wrap [`ulid::Ulid`] — globally unique without coordination (good for
//! collab), 128-bit so collision probability is negligible, and lexicographically
//! sortable by time (the first 48 bits are a millisecond timestamp). The
//! separate newtypes prevent mixing a `NodeId` with an `AssetId` at the type
//! level, which closes off a whole class of bugs.

use serde::{Deserialize, Serialize};
use std::fmt;
use ulid::Ulid;

macro_rules! id_type {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Ulid);

        impl $name {
            /// Mint a new, time-ordered, globally unique id.
            pub fn new() -> Self {
                Self(Ulid::new())
            }

            /// Construct from a 128-bit value (used by deserializers and tests).
            pub const fn from_u128(value: u128) -> Self {
                Self(Ulid(value))
            }

            /// The raw 128-bit value.
            pub const fn to_u128(self) -> u128 {
                self.0.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // Prefix makes IDs visually self-describing in logs and JSON
                // dumps — "n_01JC..." is obviously a NodeId.
                write!(f, "{}_{}", $prefix, self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let body = s
                    .strip_prefix(concat!($prefix, "_"))
                    .ok_or(IdParseError::MissingPrefix)?;
                Ulid::from_string(body)
                    .map(Self)
                    .map_err(|_| IdParseError::InvalidUlid)
            }
        }
    };
}

id_type!(NodeId, "n", "Stable identity for a [`CanvasNode`].");
id_type!(
    AssetId,
    "a",
    "Reference to a binary blob (image, video, model, audio, ai cache)."
);
id_type!(
    WorkflowNodeId,
    "wn",
    "Identity for a node inside a `NodeGraph` subgraph (separate namespace from canvas-level `NodeId`)."
);
id_type!(
    LinkId,
    "l",
    "Identity for a link/edge between workflow nodes inside a `NodeGraph`."
);
id_type!(
    DocId,
    "d",
    "Document identity. Used for cross-doc references and the persisted file's manifest."
);

// ---- Design-system & component ids (spec 07 §1) -----------------------------
//
// Each addresses a different cross-feature concept and is a distinct newtype
// for the same reason `NodeId` is: a `VariableId` must never be passed where a
// `ComponentId` is expected. The short prefixes mirror Figma's own
// `c:`/`vc:`/`v:` shorthands so logs and JSON dumps read familiarly.
id_type!(
    ComponentId,
    "c",
    "Identity for a component master in the library (`ComponentDef` or `ComponentSet`)."
);
id_type!(
    ComponentPropId,
    "cp",
    "Identity for one component property definition (`ComponentPropDef`)."
);
id_type!(
    VariableCollectionId,
    "vc",
    "Identity for a variable collection (a named group of variables sharing a mode axis)."
);
id_type!(
    VariableId,
    "v",
    "Identity for a single design-system variable (token)."
);
id_type!(
    ModeId,
    "vm",
    "Identity for one mode within a variable collection (e.g. Light / Dark)."
);
id_type!(
    ReactionId,
    "rx",
    "Identity for a prototype reaction (trigger + action) on a node."
);

/// Error returned when parsing an ID string fails.
#[derive(Debug, thiserror::Error)]
pub enum IdParseError {
    #[error("id is missing the expected prefix")]
    MissingPrefix,
    #[error("id body is not a valid ULID")]
    InvalidUlid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a = NodeId::new();
        let b = NodeId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_round_trip_through_string() {
        let id = NodeId::new();
        let s = id.to_string();
        assert!(s.starts_with("n_"));
        let parsed: NodeId = s.parse().expect("round trip");
        assert_eq!(id, parsed);
    }

    #[test]
    fn distinct_id_types_dont_mix() {
        // This test is here for documentation; the real assertion is that the
        // following would fail to compile:
        //   let n: NodeId = AssetId::new();
        let _n = NodeId::new();
        let _a = AssetId::new();
    }

    #[test]
    fn ids_serialize_as_strings_in_json() {
        let id = NodeId::from_u128(0x01_8000_0000_0000_0000_0000_0000_0000);
        let json = serde_json::to_string(&id).unwrap();
        // ULID JSON encoding is the 26-char base32 string (not the prefix form).
        assert_eq!(json.len(), 28); // 26 chars + 2 quotes
    }
}
