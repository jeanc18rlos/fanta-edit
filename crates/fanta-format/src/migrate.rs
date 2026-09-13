//! Schema migrations for `doc.json` payloads.
//!
//! The shape of `fanta_doc::Doc` will change over time. Rather than version the
//! Rust type, we keep the *current* type the only one Rust knows about and
//! migrate older JSON payloads in `serde_json::Value` space up to current
//! before we hand them to `Doc::from_json_str`. This means:
//!
//! - There is only one `Doc` type at compile time. No `DocV1`, `DocV2`, ...
//! - Old containers stay readable forever (assuming a migration is written).
//! - New containers are never produced at an old version, which protects
//!   downstream tools from accidentally writing forward-incompatible files.
//!
//! Each schema transition is an adjacent step in the ladder below.

use crate::error::{FormatError, Result};

/// Walk a `doc.json` value forward from version `from` to version `to`.
///
/// Steps are applied in sequence, each one bumping the version by one. If
/// `from > to`, the file is from the future and we refuse to load it; loaders
/// upstream of this should already have produced [`FormatError::UnsupportedSchema`],
/// but we guard here as well so this function is never called wrong.
pub fn migrate(mut value: serde_json::Value, from: u32, to: u32) -> Result<serde_json::Value> {
    if from == to {
        return Ok(value);
    }
    if from > to {
        return Err(FormatError::UnsupportedSchema {
            found: from,
            supported: to,
        });
    }

    let mut current = from;
    while current < to {
        let next = current + 1;
        value = step(value, current, next)?;
        current = next;
    }
    Ok(value)
}

/// Apply a single migration step from `current` to `next`.
///
fn step(value: serde_json::Value, current: u32, next: u32) -> Result<serde_json::Value> {
    tracing::debug!(current, next, "migrating doc payload one step");
    match (current, next) {
        // v1 → v2: `PathData` became an object `{ "segments": [...] }` (was a
        // bare array). Delegate to `fanta_doc`'s migration so the wrap logic is
        // single-sourced with `Doc::from_json_str` and walks history snapshots
        // too. (Idempotent; also stamps `schema_version`.)
        (1, 2) => {
            let mut value = value;
            fanta_doc::migrate_doc_json_to(&mut value, 1, 2)
                .map_err(|error| FormatError::InvalidManifest(error.to_string()))?;
            Ok(value)
        }
        // v2 → v3: no existing field changes shape. The version gate reserves
        // the new `text_path` node tag so v2 readers reject it instead of
        // partially loading a document they cannot represent.
        (2, 3) => {
            let mut value = value;
            fanta_doc::migrate_doc_json_to(&mut value, 2, 3)
                .map_err(|error| FormatError::InvalidManifest(error.to_string()))?;
            Ok(value)
        }
        _ => Err(FormatError::InvalidManifest(format!(
            "no migration registered from schema {current} to {next}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_same_version_is_identity() {
        let v = serde_json::json!({"foo": "bar"});
        let out = migrate(v.clone(), 1, 1).unwrap();
        assert_eq!(out, v);
    }

    #[test]
    fn migrate_future_to_past_is_unsupported() {
        let v = serde_json::json!({});
        let err = migrate(v, 2, 1).unwrap_err();
        match err {
            FormatError::UnsupportedSchema { found, supported } => {
                assert_eq!(found, 2);
                assert_eq!(supported, 1);
            }
            other => panic!("expected UnsupportedSchema, got {other:?}"),
        }
    }

    #[test]
    fn unknown_step_is_invalid_manifest() {
        let v = serde_json::json!({});
        let err = migrate(v, 3, 4).unwrap_err();
        assert!(matches!(err, FormatError::InvalidManifest(_)));
    }

    #[test]
    fn v1_to_v2_wraps_vector_path_array_into_segments_object() {
        // A v1 doc with a bare-array vector path migrates to the object form.
        let v1 = serde_json::json!({
            "schema_version": 1,
            "scene": {
                "nodes": {
                    "01000000000000000000000001": {
                        "id": "01000000000000000000000001",
                        "index": 1.0,
                        "name": "Shape",
                        "type": "vector",
                        "path": [{ "kind": "move", "x": 0.0, "y": 0.0 }]
                    }
                }
            }
        });
        let out = migrate(v1, 1, 2).unwrap();
        let path = &out["scene"]["nodes"]["01000000000000000000000001"]["path"];
        assert!(path.is_object(), "path should be wrapped into an object");
        assert!(path["segments"].is_array(), "segments holds the old array");
        assert_eq!(out["schema_version"], 2, "version stamped to 2");
    }

    #[test]
    fn v2_to_v3_only_stamps_the_schema_version() {
        let v2 = serde_json::json!({
            "schema_version": 2,
            "scene": { "nodes": {} },
            "opaque": { "preserved": [1, 2, 3] }
        });
        let mut expected = v2.clone();
        expected["schema_version"] = serde_json::Value::from(3);

        assert_eq!(migrate(v2, 2, 3).unwrap(), expected);
    }
}
