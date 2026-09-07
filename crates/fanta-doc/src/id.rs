//! Identifier types.
//!
//! All IDs wrap [`ulid::Ulid`] — globally unique without coordination (good for
//! collab), 128-bit so collision probability is negligible, and lexicographically
//! sortable by time (the first 48 bits are a millisecond timestamp). The
//! separate newtypes prevent mixing a `NodeId` with an `AssetId` at the type
//! level, which closes off a whole class of bugs.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};
use ulid::Ulid;

/// Fast non-cryptographic hasher for id-keyed maps ([`NodeId`] & co.).
///
/// The scene graph and every renderer cache are keyed by these ids, and the
/// per-frame walk performs several map lookups per visited node (node, world
/// transform memo, local-bounds memo, child bucket, geometry stamp, path
/// cache). `std`'s default SipHash-1-3 dominated that walk on large pages
/// (~13% of a 15k-node frame). Ids are 128-bit ULIDs whose low 80 bits are
/// random, so a cheap word-mixing hash (the FxHash construction rustc uses:
/// `h = (rotl(h, 5) ^ word) · K`) spreads them perfectly well; the maps
/// hold trusted, locally generated keys, so DoS resistance is not a concern.
/// Deterministic across processes as a bonus: map iteration order (and thus
/// serialization order) no longer varies run to run.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdHasher(u64);

impl IdHasher {
    const K: u64 = 0x517c_c1b7_2722_0a95;

    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::K);
    }
}

impl Hasher for IdHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_u128(&mut self, i: u128) {
        // High word (timestamp-heavy) first, random low word last so the low
        // hash bits — the ones the table indexes on — come from the random
        // half of the ULID.
        self.add((i >> 64) as u64);
        self.add(i as u64);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// [`BuildHasher`](std::hash::BuildHasher) for [`IdHasher`].
pub type IdBuildHasher = BuildHasherDefault<IdHasher>;

/// A `HashMap` hashed with [`IdHasher`]. Use for maps keyed by an id newtype
/// (or a small struct of ids/integers) on a hot path; construct with
/// `IdHashMap::default()`.
pub type IdHashMap<K, V> = std::collections::HashMap<K, V, IdBuildHasher>;

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
id_type!(
    AnimationClipId,
    "ac",
    "Identity for one persistent animation clip in a document's motion library."
);
id_type!(
    AnimationTrackId,
    "at",
    "Identity for one property track within an animation clip."
);
id_type!(
    KeyframeId,
    "kf",
    "Identity for one independently editable animation keyframe."
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
