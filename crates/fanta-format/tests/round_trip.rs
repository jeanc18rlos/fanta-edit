//! End-to-end tests that exercise `FantaFile` against the real filesystem.
//!
//! Unit tests inside the crate cover individual pieces (manifest serde, hash
//! determinism, migration). These tests verify the full surface — create, open,
//! save_doc, load_doc, put_asset, get_asset, list_assets — through the public
//! API. Everything writes to a `tempfile::TempDir` so the suite is clean to
//! re-run and parallel-safe.

use fanta_doc::{
    AiArtifactNode, AssetId, BitmapNode, CanvasNode, Color, Doc, GenerationStatus, NodeData,
    Operation, SCHEMA_VERSION, VectorNode,
};
use fanta_format::{FantaFile, FormatError};
use std::io::{Seek, SeekFrom, Write};
use tempfile::TempDir;

/// Build a populated [`Doc`] with several variant types so persistence
/// exercises the polymorphism path through `NodeData`.
fn populated_doc() -> Doc {
    let mut doc = Doc::new();
    let v = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        100.0,
        50.0,
        Color::rgb(255, 100, 50),
    )));
    doc.apply(Operation::create_node(v)).unwrap();
    let b = CanvasNode::new(NodeData::Bitmap(BitmapNode {
        asset: AssetId::from_u128(0xCAFE),
        natural_size: [256, 256],
        local_size: [256.0, 256.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Fill,
        tint: None,
    }));
    doc.apply(Operation::create_node(b)).unwrap();
    let ai = CanvasNode::new(NodeData::AiArtifact(AiArtifactNode {
        local_size: [512.0, 512.0],
        prompt: "rolling hills".into(),
        model: "flux-pro".into(),
        params: serde_json::json!({"steps": 30}),
        inputs: vec![],
        lineage_parent: None,
        output: None,
        status: GenerationStatus::Done,
        seed: Some(7),
    }));
    doc.apply(Operation::create_node(ai)).unwrap();
    doc
}

#[test]
fn empty_container_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.fant");

    let mut f = FantaFile::create(&path).unwrap();
    let doc = Doc::new();
    let doc_id = doc.id;
    f.save_doc(&doc).unwrap();
    drop(f);

    let f2 = FantaFile::open(&path).unwrap();
    let back = f2.load_doc().unwrap();
    assert_eq!(back.id, doc_id);
    assert_eq!(back.schema_version, SCHEMA_VERSION);
    assert!(f2.list_assets().is_empty());
}

#[test]
fn populated_container_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("project.fant");

    let mut f = FantaFile::create(&path).unwrap();
    let doc = populated_doc();
    let doc_id = doc.id;
    let node_count = doc.scene.len();
    f.save_doc(&doc).unwrap();
    drop(f);

    let f2 = FantaFile::open(&path).unwrap();
    let back = f2.load_doc().unwrap();
    assert_eq!(back.id, doc_id);
    assert_eq!(back.scene.len(), node_count);
}

#[test]
fn put_same_bytes_returns_same_id() {
    let dir = TempDir::new().unwrap();
    let mut f = FantaFile::create(dir.path().join("a.fant")).unwrap();
    let bytes = b"identical content".to_vec();
    let id1 = f.put_asset(&bytes).unwrap();
    let id2 = f.put_asset(&bytes).unwrap();
    assert_eq!(id1, id2);
    assert_eq!(f.list_assets().len(), 1);
}

#[test]
fn put_different_bytes_returns_different_ids() {
    let dir = TempDir::new().unwrap();
    let mut f = FantaFile::create(dir.path().join("a.fant")).unwrap();
    let id1 = f.put_asset(b"one").unwrap();
    let id2 = f.put_asset(b"two").unwrap();
    assert_ne!(id1, id2);
    let mut ids = f.list_assets();
    ids.sort_by_key(|a| a.to_u128());
    let mut expected = vec![id1, id2];
    expected.sort_by_key(|a| a.to_u128());
    assert_eq!(ids, expected);
}

#[test]
fn assets_survive_save_and_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a.fant");
    let blob_a = b"alpha".to_vec();
    let blob_b = vec![0u8; 1024];

    let id_a;
    let id_b;
    {
        let mut f = FantaFile::create(&path).unwrap();
        id_a = f.put_asset(&blob_a).unwrap();
        id_b = f.put_asset(&blob_b).unwrap();
        f.save_doc(&Doc::new()).unwrap();
    }

    let f = FantaFile::open(&path).unwrap();
    let got_a = f.get_asset(id_a).unwrap();
    let got_b = f.get_asset(id_b).unwrap();
    assert_eq!(got_a, blob_a);
    assert_eq!(got_b, blob_b);
    let mut ids = f.list_assets();
    ids.sort_by_key(|a| a.to_u128());
    let mut want = vec![id_a, id_b];
    want.sort_by_key(|a| a.to_u128());
    assert_eq!(ids, want);
}

#[test]
fn vcs_bundle_survives_save_and_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("versioned.fant");
    let bundle = b"fake git bundle bytes".to_vec();

    {
        let mut f = FantaFile::create(&path).unwrap();
        f.save_doc(&Doc::new()).unwrap();
        f.set_vcs_bundle(Some(bundle.clone())).unwrap();
    }

    let mut f = FantaFile::open(&path).unwrap();
    assert_eq!(f.vcs_bundle(), Some(bundle.as_slice()));
    f.set_vcs_bundle(None).unwrap();

    let f = FantaFile::open(&path).unwrap();
    assert!(f.vcs_bundle().is_none());
}

#[test]
fn missing_asset_returns_asset_not_found() {
    let dir = TempDir::new().unwrap();
    let f = FantaFile::create(dir.path().join("a.fant")).unwrap();
    let err = f.get_asset(AssetId::from_u128(0xDEADBEEF)).unwrap_err();
    assert!(matches!(err, FormatError::AssetNotFound(_)));
}

#[test]
fn higher_schema_version_is_rejected() {
    // Hand-craft a zip whose manifest declares a future schema.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("future.fant");
    let mut buf = std::io::Cursor::new(Vec::<u8>::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zw.start_file("manifest.json", opts).unwrap();
        let m = serde_json::json!({
            "schema_version": SCHEMA_VERSION + 1,
            "doc_id": "d_future",
            "app_version": "999.0.0",
            "assets": {},
            "created_at": 0,
            "modified_at": 0,
        });
        zw.write_all(m.to_string().as_bytes()).unwrap();
        zw.finish().unwrap();
    }
    std::fs::write(&path, buf.into_inner()).unwrap();

    let err = FantaFile::open(&path).unwrap_err();
    match err {
        FormatError::UnsupportedSchema { found, supported } => {
            assert_eq!(found, SCHEMA_VERSION + 1);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        other => panic!("expected UnsupportedSchema, got {other:?}"),
    }
}

#[test]
fn corrupt_zip_returns_zip_error_not_panic() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("garbage.fant");
    std::fs::write(&path, b"this is not a zip file").unwrap();
    let err = FantaFile::open(&path).unwrap_err();
    assert!(matches!(err, FormatError::Zip(_)));
}

#[test]
fn missing_manifest_returns_missing_file() {
    // A valid zip with no `manifest.json`.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nomanifest.fant");
    let mut buf = std::io::Cursor::new(Vec::<u8>::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        zw.start_file("somethingelse.txt", opts).unwrap();
        zw.write_all(b"hello").unwrap();
        zw.finish().unwrap();
    }
    std::fs::write(&path, buf.into_inner()).unwrap();
    let err = FantaFile::open(&path).unwrap_err();
    match err {
        FormatError::MissingFile { name } => assert_eq!(name, "manifest.json"),
        other => panic!("expected MissingFile, got {other:?}"),
    }
}

#[test]
fn malformed_manifest_json_is_reported() {
    // Valid zip, manifest entry is garbage JSON.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("badjson.fant");
    let mut buf = std::io::Cursor::new(Vec::<u8>::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        zw.start_file("manifest.json", opts).unwrap();
        zw.write_all(b"{not actually json").unwrap();
        zw.finish().unwrap();
    }
    std::fs::write(&path, buf.into_inner()).unwrap();
    let err = FantaFile::open(&path).unwrap_err();
    assert!(
        matches!(err, FormatError::Json(_) | FormatError::InvalidManifest(_)),
        "got {err:?}"
    );
}

#[test]
fn manifest_with_wrong_field_type_is_invalid() {
    // Valid JSON, wrong shape for our Manifest.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wrongshape.fant");
    let mut buf = std::io::Cursor::new(Vec::<u8>::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        zw.start_file("manifest.json", opts).unwrap();
        // schema_version should be a u32; we pass a string. serde_json
        // classifies that as a Data error, which we route to InvalidManifest.
        let bad = serde_json::json!({
            "schema_version": "one",
            "doc_id": "d_x",
            "app_version": "0",
            "assets": {},
            "created_at": 0,
            "modified_at": 0,
        });
        zw.write_all(bad.to_string().as_bytes()).unwrap();
        zw.finish().unwrap();
    }
    std::fs::write(&path, buf.into_inner()).unwrap();
    let err = FantaFile::open(&path).unwrap_err();
    assert!(
        matches!(err, FormatError::InvalidManifest(_) | FormatError::Json(_)),
        "got {err:?}"
    );
}

#[test]
fn end_to_end_through_public_api() {
    // The "happy path" smoke test: create a file, populate doc + assets,
    // close it, re-open, and verify every piece matches.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("project.fant");

    let mut f = FantaFile::create(&path).unwrap();
    let doc = populated_doc();
    let doc_id = doc.id;
    let node_count = doc.scene.len();
    f.save_doc(&doc).unwrap();
    let blob = b"PNG_BYTES".to_vec();
    let asset_id = f.put_asset(&blob).unwrap();
    drop(f);

    let f2 = FantaFile::open(&path).unwrap();
    assert_eq!(f2.manifest().doc_id, doc_id.to_string());
    assert_eq!(f2.manifest().schema_version, SCHEMA_VERSION);
    assert_eq!(f2.list_assets(), vec![asset_id]);
    assert_eq!(f2.get_asset(asset_id).unwrap(), blob);
    let back = f2.load_doc().unwrap();
    assert_eq!(back.id, doc_id);
    assert_eq!(back.scene.len(), node_count);
}

#[test]
fn manifest_app_version_matches_crate_version() {
    let dir = TempDir::new().unwrap();
    let mut f = FantaFile::create(dir.path().join("a.fant")).unwrap();
    f.save_doc(&Doc::new()).unwrap();
    assert_eq!(f.manifest().app_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn list_assets_returns_pending_after_open() {
    // Asset blobs are loaded lazily on `open`. `list_assets` should still
    // see all of them via the manifest's asset table, not just the ones
    // already hydrated.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a.fant");
    let id_a;
    let id_b;
    {
        let mut f = FantaFile::create(&path).unwrap();
        id_a = f.put_asset(b"alpha").unwrap();
        id_b = f.put_asset(b"beta").unwrap();
        f.save_doc(&Doc::new()).unwrap();
    }
    let f = FantaFile::open(&path).unwrap();
    let mut ids = f.list_assets();
    ids.sort_by_key(|a| a.to_u128());
    let mut expected = vec![id_a, id_b];
    expected.sort_by_key(|a| a.to_u128());
    assert_eq!(ids, expected);
}

#[test]
fn save_doc_updates_doc_id_and_modified_at() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a.fant");
    let mut f = FantaFile::create(&path).unwrap();
    let initial_doc_id = f.manifest().doc_id.clone();
    let initial_modified = f.manifest().modified_at;
    assert!(initial_doc_id.is_empty(), "fresh container has no doc id");

    // Force a measurable delta on the modified_at by writing into the future.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let doc = populated_doc();
    let doc_id = doc.id;
    f.save_doc(&doc).unwrap();
    assert_eq!(f.manifest().doc_id, doc_id.to_string());
    // modified_at is seconds-resolution; assert it is at least equal.
    assert!(f.manifest().modified_at >= initial_modified);
}

#[test]
fn dedup_survives_through_save_and_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("dedup.fant");
    let bytes = b"shared".to_vec();
    {
        let mut f = FantaFile::create(&path).unwrap();
        let a = f.put_asset(&bytes).unwrap();
        let b = f.put_asset(&bytes).unwrap();
        assert_eq!(a, b);
        f.save_doc(&Doc::new()).unwrap();
    }
    let f = FantaFile::open(&path).unwrap();
    assert_eq!(f.list_assets().len(), 1);
}

#[test]
fn truncated_zip_is_zip_error_not_panic() {
    // Build a valid container, then truncate its bytes to provoke ZipError.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("trunc.fant");
    {
        let mut f = FantaFile::create(&path).unwrap();
        f.save_doc(&populated_doc()).unwrap();
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        // Lop off the central directory.
        let len = f.seek(SeekFrom::End(0)).unwrap();
        f.set_len(len.saturating_sub(64)).unwrap();
    }
    let err = FantaFile::open(&path).unwrap_err();
    assert!(matches!(err, FormatError::Zip(_)), "got {err:?}");
}
