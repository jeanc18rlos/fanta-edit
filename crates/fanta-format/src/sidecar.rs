//! `.fnx` identity sidecars resolved against their parsed source. The native
//! project reader and the portable runtime compiler both use these, so a
//! sidecar the source has outgrown mints the same node ids in the editor and
//! in an exported site.

use crate::error::{FormatError, Result};
use fanta_doc::NodeId;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub(crate) fn reconcile_fnx_sidecar_from_tree(
    source: &str,
    root: &fanta_fnx::FnxElement,
    sidecar: &fanta_fnx::FnxSidecar,
) -> Result<fanta_fnx::FnxSidecar> {
    let mut identity_generator: Option<([u8; 32], u64, HashSet<String>)> = None;
    fanta_fnx::reconcile_sidecar_from_tree(root, sidecar, || {
        // Most loads preserve every identity. Hashing the whole source and
        // copying its ids only helps when reconciliation must mint a new id.
        let (seed, ordinal, used_ids) = identity_generator.get_or_insert_with(|| {
            let mut seed_hasher = Sha256::new();
            seed_hasher.update(b"fanta-fnx-node-ids-v2");
            seed_hasher.update((source.len() as u64).to_le_bytes());
            seed_hasher.update(source.as_bytes());
            for entry in &sidecar.ids {
                seed_hasher.update((entry.id.len() as u64).to_le_bytes());
                seed_hasher.update(entry.id.as_bytes());
            }
            (
                seed_hasher.finalize().into(),
                sidecar.ids.len() as u64,
                sidecar.ids.iter().map(|entry| entry.id.clone()).collect(),
            )
        });
        loop {
            let mut id_hasher = Sha256::new();
            id_hasher.update(*seed);
            id_hasher.update(ordinal.to_le_bytes());
            *ordinal += 1;
            let digest = id_hasher.finalize();
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            let id = NodeId::from_u128(u128::from_be_bytes(bytes)).0.to_string();
            if used_ids.insert(id.clone()) {
                break id;
            }
        }
    })
    .map_err(|error| FormatError::InvalidProjectTree(error.to_string()))
}

pub(crate) fn persisted_fnx_sidecar_from_tree(
    source: &str,
    root: &fanta_fnx::FnxElement,
    bytes: Option<&[u8]>,
    mut identity_current: Option<&mut bool>,
) -> Result<fanta_fnx::FnxSidecar> {
    let persisted = bytes
        .ok_or_else(|| FormatError::InvalidProjectTree("missing identity sidecar".into()))
        .and_then(|bytes| {
            serde_json::from_slice::<fanta_fnx::FnxSidecar>(bytes).map_err(|error| {
                FormatError::InvalidProjectTree(format!("invalid identity sidecar: {error}"))
            })
        });
    let result = persisted
        .as_ref()
        .map_err(|error| error.to_string())
        .and_then(|sidecar| {
            if sidecar.ids.iter().any(|entry| {
                !is_node_key(&entry.id) || (!entry.index.is_null() && !entry.index.is_number())
            }) || sidecar
                .root_parent
                .as_ref()
                .is_some_and(|parent| !is_node_key(parent))
            {
                return Err("invalid node identity or sibling order in identity sidecar".into());
            }
            let matches = parsed_tree_matches_sidecar(root, sidecar);
            if let Some(current) = &mut identity_current {
                **current = matches;
            }
            if matches {
                return Ok(sidecar.clone());
            }
            reconcile_fnx_sidecar_from_tree(source, root, sidecar)
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(sidecar) => Ok(sidecar),
        Err(error) => {
            // Modern source pins every identity itself. Reconstruct its order
            // from nesting rather than letting a damaged compatibility file
            // make the whole project unopenable or invent legacy identities.
            fn has_explicit_ids(element: &fanta_fnx::FnxElement) -> bool {
                element
                    .attrs
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(is_node_key)
                    && element.children.iter().all(has_explicit_ids)
            }
            if !has_explicit_ids(root) {
                return Err(FormatError::InvalidProjectTree(format!(
                    "{error}; restore the sidecar for source without explicit node IDs"
                )));
            }
            let root_parent = persisted
                .ok()
                .and_then(|sidecar| sidecar.root_parent)
                .filter(|parent| is_node_key(parent));
            reconcile_fnx_sidecar_from_tree(
                source,
                root,
                &fanta_fnx::FnxSidecar {
                    root_parent,
                    ids: Vec::new(),
                },
            )
        }
    }
}

fn parsed_tree_matches_sidecar(
    root: &fanta_fnx::FnxElement,
    sidecar: &fanta_fnx::FnxSidecar,
) -> bool {
    // Parsing already validated every attribute and desugared geometry. Only
    // exact modern identity fingerprints can replace the indexer's second
    // source scan; legacy or repaired sidecars remain unverified for saving.
    let mut pending = vec![(root, None)];
    let mut identities = HashSet::with_capacity(sidecar.ids.len());
    let mut position = 0usize;
    while let Some((element, parent)) = pending.pop() {
        let Some(entry) = sidecar.ids.get(position) else {
            return false;
        };
        if element.attrs.get("id").and_then(Value::as_str) != Some(entry.id.as_str())
            || entry.tag.as_deref() != Some(element.tag.as_str())
            || entry.name.as_deref() != element.attrs.get("name").and_then(Value::as_str)
            || entry.parent_index != parent
            || !identities.insert(entry.id.as_str())
        {
            return false;
        }
        let Ok(parent) = u32::try_from(position) else {
            return false;
        };
        pending.extend(
            element
                .children
                .iter()
                .rev()
                .map(|child| (child, Some(parent))),
        );
        position += 1;
    }
    position == sidecar.ids.len()
}

/// Whether `key` is a node id in its persisted serde form, a bare ULID.
fn is_node_key(key: &str) -> bool {
    serde_json::from_value::<NodeId>(Value::String(key.to_owned())).is_ok()
}
