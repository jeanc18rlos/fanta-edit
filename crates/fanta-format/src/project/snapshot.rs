//! `.fant` snapshot export/import — spec 09 §A.4 (".fant becomes the export").
//!
//! A `.fant` is no longer the working format; it is a *moment-in-time*
//! portable snapshot of a project directory:
//!
//! - [`export_fant_snapshot`] — project tree → `.fant`. The workspace as of
//!   now: doc + assets, **no history, no `.git`**. A snapshot never carries an
//!   embedded vcs bundle (that path belongs to the legacy single-file flow).
//! - [`import_fant_snapshot`] — `.fant` → fresh project tree (scaffold +
//!   write). Existing `.fant` files migrate through exactly this path; the
//!   container's schema migration (`migrate.rs`) stays the version gate.
//!   Git init is deliberately *not* done here — the lifecycle layer owns git
//!   (spec 09 §A.1).
//!
//! Round-trip contract: export → import → tree-identical designs (modulo
//! `.git` and derived dirs). Presence state (`active_page`, `selection`,
//! `viewport`, `history`) that a legacy `.fant` may still carry is dropped on
//! import by the tree writer's own invariant.

use crate::container::FantaFile;
use crate::error::{FormatError, Result};
use crate::project::{
    is_project_dir, read_project_tree, scaffold_project_tree, write_project_tree,
};
use std::collections::BTreeMap;
use std::path::Path;

/// Export the project tree at `project_dir` as a `.fant` snapshot at `out` —
/// the workspace as of now: doc + assets, no history, no `.git`.
///
/// `out` is explicit; the `<project>/exports/` default location is a caller
/// convention, not enforced here. Overwrite is safe: the container writes a
/// sibling temp file and renames atomically.
pub fn export_fant_snapshot(project_dir: &Path, out: &Path) -> Result<()> {
    let (doc, assets) = read_project_tree(project_dir)?;
    let mut fant = FantaFile::create(out)?;
    fant.save_doc(&doc)?;
    // BTreeMap iterates in id order — deterministic archive entry order.
    // The tree's ids are stored verbatim (`put_asset_as`): ids that are not
    // content-addressed (`.fig`-imported assets) are what the doc's
    // `Fill::Image` references point at, so re-minting would orphan them.
    // Explicitly no vcs bundle: a snapshot has no history.
    for (id, bytes) in &assets {
        fant.put_asset_as(*id, bytes)?;
    }
    Ok(())
}

/// Import a `.fant` into `dest_dir` as a fresh project tree (scaffold +
/// write). `dest_dir` must not already be a project (error if
/// [`is_project_dir`]). Does NOT git-init — the lifecycle layer owns git.
pub fn import_fant_snapshot(fant_path: &Path, dest_dir: &Path) -> Result<()> {
    if is_project_dir(dest_dir) {
        return Err(FormatError::InvalidProjectTree(format!(
            "import destination is already a fanta project: {}",
            dest_dir.display()
        )));
    }
    let fant = FantaFile::open(fant_path)?;
    let doc = fant.load_doc()?;
    let mut assets = BTreeMap::new();
    for id in fant.list_assets() {
        assets.insert(id, fant.get_asset(id)?);
    }
    scaffold_project_tree(dest_dir)?;
    write_project_tree(dest_dir, &doc, &assets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_id_for_bytes;
    use fanta_doc::{AssetId, CanvasNode, Doc, GroupNode, NodeData, NodeId, Viewport};
    use serde_json::Value;
    use std::fs;
    use tempfile::tempdir;

    fn insert_group(doc: &mut Doc, parent: Option<NodeId>, name: &str) -> NodeId {
        let mut n = CanvasNode::new(NodeData::Group(GroupNode::default()));
        n.parent = parent;
        n.name = name.to_owned();
        let id = n.id;
        doc.scene.insert(n).unwrap();
        id
    }

    /// Two pages, a nested frame, presence state (which must never persist),
    /// and two content-addressed assets.
    fn fixture() -> (Doc, BTreeMap<AssetId, Vec<u8>>) {
        let mut doc = Doc::new();
        doc.metadata.title = "Snapshot Fixture".into();
        doc.metadata.created_at = 1_700_000_000;
        doc.metadata.modified_at = 1_700_000_001;
        let page1 = insert_group(&mut doc, None, "Page 1");
        let frame = insert_group(&mut doc, Some(page1), "Hero");
        let page2 = insert_group(&mut doc, None, "Page 2");
        doc.add_page(page1);
        doc.add_page(page2);

        // Presence state — must be stripped by the tree projection.
        doc.set_active_page(Some(page2));
        doc.selection.select_only(frame);
        doc.viewport = Viewport {
            center: [3.0, 4.0],
            zoom: 1.5,
        };

        let blobs: [&[u8]; 2] = [b"\x89PNG\r\n\x1a\nsnapshot-image", b"plain snapshot bytes"];
        let mut assets = BTreeMap::new();
        for bytes in blobs {
            assets.insert(asset_id_for_bytes(bytes), bytes.to_vec());
        }
        (doc, assets)
    }

    /// The persisted projection of a doc: its JSON minus presence state.
    fn persisted(doc: &Doc) -> Value {
        let mut v = serde_json::to_value(doc).unwrap();
        if let Value::Object(map) = &mut v {
            for key in ["selection", "history", "viewport", "active_page"] {
                map.remove(key);
            }
        }
        v
    }

    /// Every file under `root`, as (relative path, bytes).
    fn files_under(root: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.push((rel, fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out
    }

    #[test]
    fn export_import_round_trips_doc_and_assets() {
        let (doc, assets) = fixture();
        let src = tempdir().unwrap();
        write_project_tree(src.path(), &doc, &assets).unwrap();

        let out_dir = tempdir().unwrap();
        let fant_path = out_dir.path().join("snap.fant");
        export_fant_snapshot(src.path(), &fant_path).unwrap();

        let dest = tempdir().unwrap();
        let dest_dir = dest.path().join("imported");
        import_fant_snapshot(&fant_path, &dest_dir).unwrap();
        assert!(is_project_dir(&dest_dir));

        let (doc_src, assets_src) = read_project_tree(src.path()).unwrap();
        let (doc_dest, assets_dest) = read_project_tree(&dest_dir).unwrap();
        assert_eq!(persisted(&doc_dest), persisted(&doc_src));
        assert_eq!(persisted(&doc_dest), persisted(&doc));
        assert_eq!(assets_dest, assets_src);
        assert_eq!(assets_dest, assets);
        assert_eq!(doc_dest.pages(), doc.pages(), "page order survives");

        // import → export again: byte-comparing zips is not meaningful
        // (timestamps), so compare re-read contents instead.
        let fant2_path = out_dir.path().join("snap2.fant");
        export_fant_snapshot(&dest_dir, &fant2_path).unwrap();
        let a = FantaFile::open(&fant_path).unwrap();
        let b = FantaFile::open(&fant2_path).unwrap();
        assert_eq!(
            persisted(&b.load_doc().unwrap()),
            persisted(&a.load_doc().unwrap())
        );
        assert_eq!(b.list_assets(), a.list_assets());
        for id in a.list_assets() {
            assert_eq!(b.get_asset(id).unwrap(), a.get_asset(id).unwrap());
        }
    }

    #[test]
    fn export_preserves_non_content_addressed_asset_ids() {
        // `.fig`-imported assets carry ids that are NOT the content hash; the
        // doc's Fill::Image references use those ids verbatim, so the export
        // must store them as-is rather than re-minting content-addressed ones.
        let (doc, _) = fixture();
        let foreign_id = AssetId::new();
        let bytes = b"fig-imported image bytes".to_vec();
        assert_ne!(foreign_id, asset_id_for_bytes(&bytes));
        let mut assets = BTreeMap::new();
        assets.insert(foreign_id, bytes.clone());

        let src = tempdir().unwrap();
        write_project_tree(src.path(), &doc, &assets).unwrap();
        let out = tempdir().unwrap();
        let fant_path = out.path().join("snap.fant");
        export_fant_snapshot(src.path(), &fant_path).unwrap();

        let fant = FantaFile::open(&fant_path).unwrap();
        assert_eq!(fant.list_assets(), vec![foreign_id]);
        assert_eq!(fant.get_asset(foreign_id).unwrap(), bytes);

        // And the id survives the full export → import round trip.
        let dest = tempdir().unwrap();
        let dest_dir = dest.path().join("imported");
        import_fant_snapshot(&fant_path, &dest_dir).unwrap();
        let (_, assets2) = read_project_tree(&dest_dir).unwrap();
        assert_eq!(assets2.get(&foreign_id), Some(&bytes));
    }

    #[test]
    fn exporting_a_non_project_dir_errors() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("snap.fant");
        let err = export_fant_snapshot(dir.path(), &out).unwrap_err();
        assert!(
            matches!(err, FormatError::NotAProject { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn importing_onto_an_existing_project_errors() {
        let (doc, _) = fixture();
        let dir = tempdir().unwrap();
        let fant_path = dir.path().join("snap.fant");
        let mut fant = FantaFile::create(&fant_path).unwrap();
        fant.save_doc(&doc).unwrap();

        let dest = tempdir().unwrap();
        scaffold_project_tree(dest.path()).unwrap();
        let err = import_fant_snapshot(&fant_path, dest.path()).unwrap_err();
        assert!(
            matches!(err, FormatError::InvalidProjectTree(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn vcs_bundle_is_imported_cleanly_and_stripped_on_reexport() {
        let (doc, assets) = fixture();
        let dir = tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.fant");
        let mut legacy = FantaFile::create(&legacy_path).unwrap();
        legacy.save_doc(&doc).unwrap();
        for bytes in assets.values() {
            legacy.put_asset(bytes).unwrap();
        }
        legacy
            .set_vcs_bundle(Some(b"opaque git bundle".to_vec()))
            .unwrap();

        let dest = tempdir().unwrap();
        let dest_dir = dest.path().join("imported");
        import_fant_snapshot(&legacy_path, &dest_dir).unwrap();
        let (doc2, assets2) = read_project_tree(&dest_dir).unwrap();
        assert_eq!(persisted(&doc2), persisted(&doc));
        assert_eq!(assets2, assets);

        // History is stripped: the re-exported snapshot carries no bundle.
        let snap_path = dir.path().join("snap.fant");
        export_fant_snapshot(&dest_dir, &snap_path).unwrap();
        let snap = FantaFile::open(&snap_path).unwrap();
        assert!(snap.vcs_bundle().is_none(), "snapshot must have no history");
    }

    #[test]
    fn legacy_presence_state_never_reaches_the_imported_tree() {
        // A legacy `.fant` doc body carries presence state (active_page,
        // selection, viewport) in full.
        let (doc, _) = fixture();
        let dir = tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.fant");
        let mut legacy = FantaFile::create(&legacy_path).unwrap();
        legacy.save_doc(&doc).unwrap();
        let raw = legacy.load_doc().unwrap();
        assert!(raw.active_page().is_some(), "fixture carries presence");

        let dest = tempdir().unwrap();
        let dest_dir = dest.path().join("imported");
        import_fant_snapshot(&legacy_path, &dest_dir).unwrap();

        for (rel, bytes) in files_under(&dest_dir) {
            let text = String::from_utf8_lossy(&bytes);
            for needle in [
                "\"active_page\"",
                "\"viewport\"",
                "\"selection\"",
                "\"history\"",
            ] {
                assert!(
                    !text.contains(needle),
                    "{rel} contains presence key {needle}"
                );
            }
        }
        let (doc2, _) = read_project_tree(&dest_dir).unwrap();
        assert_eq!(doc2.active_page(), None);
    }
}
