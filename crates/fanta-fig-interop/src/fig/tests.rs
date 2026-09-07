//! Unit tests for the `.fig` container reader/writer using a small synthetic,
//! Figma-shaped document. The opt-in real-fixture diagnostics live in
//! `fixture_tests.rs`.

use super::*;
use crate::kiwi::{Def, DefKind, Field};

/// Build a minimal but realistic Figma-shaped schema and document.
///
/// We name the node-type enum `NodeType` with `DOCUMENT`/`CANVAS`/`FRAME`/
/// `RECTANGLE` members and a root `Message` carrying a flat node list, which
/// mirrors how `.fig` actually stores nodes (a flat array, parent links by
/// id) closely enough to exercise the whole container pipeline.
pub(super) fn figma_like_doc() -> FigDocument {
    let schema = Schema::new(vec![
        Def::new(
            "NodeType",
            DefKind::Enum,
            vec![
                Field::new("DOCUMENT", KiwiType(0), 0),
                Field::new("CANVAS", KiwiType(0), 1),
                Field::new("FRAME", KiwiType(0), 2),
                Field::new("RECTANGLE", KiwiType(0), 3),
            ],
        ),
        Def::new(
            "GUID",
            DefKind::Struct,
            vec![
                Field::new("sessionID", KiwiType::UINT, 0),
                Field::new("localID", KiwiType::UINT, 0),
            ],
        ),
        Def::new(
            "NodeChange",
            DefKind::Message,
            vec![
                Field::new("guid", KiwiType::user(1), 1),
                Field::new("type", KiwiType::user(0), 2),
                Field::new("name", KiwiType::STRING, 3),
            ],
        ),
        Def::new(
            "Message",
            DefKind::Message,
            vec![Field::array("nodeChanges", KiwiType::user(2), 1)],
        ),
    ]);

    let node = |sid: u32, lid: u32, ty: &str, name: &str| KiwiValue::Object {
        type_name: "NodeChange".into(),
        fields: [
            (
                "guid".to_owned(),
                KiwiValue::Object {
                    type_name: "GUID".into(),
                    fields: [
                        ("sessionID".to_owned(), KiwiValue::Uint(sid)),
                        ("localID".to_owned(), KiwiValue::Uint(lid)),
                    ]
                    .into_iter()
                    .collect(),
                },
            ),
            ("type".to_owned(), KiwiValue::Enum(ty.into())),
            ("name".to_owned(), KiwiValue::String(name.to_owned())),
        ]
        .into_iter()
        .collect(),
    };

    let root = KiwiValue::Object {
        type_name: "Message".into(),
        fields: [(
            "nodeChanges".to_owned(),
            KiwiValue::Array(vec![
                node(0, 0, "DOCUMENT", "Document"),
                node(0, 1, "CANVAS", "Page 1"),
                node(0, 2, "FRAME", "Frame 1"),
                node(0, 3, "RECTANGLE", "Rectangle 1"),
            ]),
        )]
        .into_iter()
        .collect(),
    };

    let blobs = extract_blobs(&root);
    FigDocument {
        version: 0,
        schema,
        root,
        root_type_name: "Message".into(),
        blobs,
        images: HashMap::new(),
    }
}

#[test]
fn write_then_read_round_trips_the_container() {
    let doc = figma_like_doc();
    let bytes = write_fig(&doc).unwrap();
    // Sanity-check the framing we just produced.
    assert_eq!(&bytes[0..8], FIG_MAGIC);
    assert_eq!(
        u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        0
    );

    let back = read_fig(&bytes).unwrap();
    assert_eq!(back.version, 0);
    assert_eq!(back.root_type_name, "Message");
    assert_eq!(back.root, doc.root);
    assert_eq!(back.schema, doc.schema);
}

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = write_fig(&figma_like_doc()).unwrap();
    bytes[0] = b'X';
    assert!(matches!(read_fig(&bytes), Err(FigError::BadHeader(_))));
}

#[test]
fn high_version_is_accepted_leniently() {
    // Real exports carry version 101; the framing is version-stable, so a
    // non-zero version must parse rather than be rejected.
    let mut doc = figma_like_doc();
    doc.version = OBSERVED_VERSION;
    let bytes = write_fig(&doc).unwrap();
    let back = read_fig(&bytes).unwrap();
    assert_eq!(back.version, OBSERVED_VERSION);
    assert_eq!(back.root, doc.root);
}

#[test]
fn truncated_header_is_rejected() {
    assert!(matches!(read_fig(b"fig"), Err(FigError::Truncated)));
}

#[test]
fn truncated_chunk_length_is_rejected() {
    // Valid header, then a chunk claiming more bytes than provided.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(FIG_MAGIC);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&999u32.to_le_bytes()); // length 999
    bytes.extend_from_slice(&[1, 2, 3]); // but only 3 bytes
    assert!(matches!(read_fig(&bytes), Err(FigError::Truncated)));
}

#[test]
fn zstd_compressed_data_chunk_decodes() {
    // Current Figma exports Zstd-compress the data chunk. Build a fig-kiwi
    // stream with a raw-DEFLATE schema chunk and a real Zstd data chunk,
    // and confirm read_fig decompresses + decodes it correctly.
    let doc = figma_like_doc();
    let schema_chunk = deflate(&doc.schema.encode_binary()).unwrap();
    let raw_data = doc.root.encode(&doc.schema).unwrap();
    let zstd_data = zstd::stream::encode_all(&raw_data[..], 3).unwrap();
    assert_eq!(&zstd_data[0..4], &ZSTD_MAGIC, "zstd frame magic");

    let mut bytes = Vec::new();
    bytes.extend_from_slice(FIG_MAGIC);
    bytes.extend_from_slice(&OBSERVED_VERSION.to_le_bytes());
    write_chunk(&mut bytes, &schema_chunk).unwrap();
    write_chunk(&mut bytes, &zstd_data).unwrap();

    let back = read_fig(&bytes).unwrap();
    assert_eq!(back.root, doc.root, "zstd data chunk round-trips");
}

#[test]
fn zip_wrapped_fig_is_unwrapped() {
    // Current `.fig` files are a ZIP with a `canvas.fig` entry. Build that
    // shape in-memory and confirm read_fig unwraps and decodes it.
    let doc = figma_like_doc();
    let canvas = write_fig(&doc).unwrap();
    let mut zip_bytes = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut zip_bytes));
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zw.start_file(ZIP_CANVAS_ENTRY, opts).unwrap();
        zw.write_all(&canvas).unwrap();
        // A sibling entry the reader must ignore.
        zw.start_file("meta.json", opts).unwrap();
        zw.write_all(b"{}").unwrap();
        zw.finish().unwrap();
    }
    assert_eq!(&zip_bytes[0..4], &ZIP_MAGIC, "zip local header magic");
    let back = read_fig(&zip_bytes).unwrap();
    assert_eq!(back.root, doc.root, "zip-wrapped fig decodes");
    // A `.fig` with no `images/` entries exposes an empty image map.
    assert!(back.images.is_empty(), "no images present => empty map");
}

#[test]
fn zip_images_are_collected_keyed_by_hash() {
    // Build a `.fig` ZIP carrying `canvas.fig`, the `images/` directory
    // entry, and two `images/<hash>` bitmaps; confirm read_fig exposes them
    // keyed by their hash with the exact bytes, skipping the dir entry and
    // unrelated siblings.
    let doc = figma_like_doc();
    let canvas = write_fig(&doc).unwrap();
    let png = b"\x89PNG\r\n\x1a\n\x00\x01\x02".to_vec();
    let jpg = b"\xff\xd8\xff\xe0jpegbytes".to_vec();
    let h_png = "1a5a0446424e26b9ed26c1f6e8b0dc7844032fe5";
    let h_jpg = "de70b1d56e48d79ae8ed4b5ddd9634def0335f67";

    let mut zip_bytes = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut zip_bytes));
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zw.start_file(ZIP_CANVAS_ENTRY, opts).unwrap();
        zw.write_all(&canvas).unwrap();
        // The zero-length directory entry, which must be skipped.
        zw.start_file("images/", opts).unwrap();
        zw.start_file(format!("images/{h_png}"), opts).unwrap();
        zw.write_all(&png).unwrap();
        zw.start_file(format!("images/{h_jpg}"), opts).unwrap();
        zw.write_all(&jpg).unwrap();
        // An unrelated sibling the reader ignores.
        zw.start_file("meta.json", opts).unwrap();
        zw.write_all(b"{}").unwrap();
        zw.finish().unwrap();
    }

    let back = read_fig(&zip_bytes).unwrap();
    assert_eq!(
        back.images.len(),
        2,
        "two bitmaps collected, dir entry skipped"
    );
    assert_eq!(back.images.get(h_png), Some(&png), "PNG bytes by hash");
    assert_eq!(back.images.get(h_jpg), Some(&jpg), "JPEG bytes by hash");
    assert!(
        !back.images.contains_key("images/"),
        "directory entry not keyed"
    );
}

#[test]
fn every_strict_prefix_errors_cleanly() {
    // Malformed-input guard: a `.fig` truncated at ANY byte offset must come
    // back as an `Err`, never a panic (and never a silent partial Ok). Every
    // strict prefix lands either mid-header, mid-chunk, or at a chunk boundary
    // short of the required schema+data pair, so all of them must error. Run
    // over both container forms: the bare fig-kiwi stream and the ZIP wrapper.
    let doc = figma_like_doc();
    let bare = write_fig(&doc).unwrap();

    let mut zipped = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut zipped));
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zw.start_file(ZIP_CANVAS_ENTRY, opts).unwrap();
        zw.write_all(&bare).unwrap();
        zw.finish().unwrap();
    }

    for (label, bytes) in [("bare", &bare), ("zip", &zipped)] {
        for len in 0..bytes.len() {
            assert!(
                read_fig(&bytes[..len]).is_err(),
                "{label} container truncated to {len}/{} bytes must error",
                bytes.len()
            );
        }
    }
}

#[test]
fn missing_data_chunk_is_rejected() {
    // Only a schema chunk, no data chunk.
    let doc = figma_like_doc();
    let schema_chunk = deflate(&doc.schema.encode_binary()).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(FIG_MAGIC);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    write_chunk(&mut bytes, &schema_chunk).unwrap();
    assert!(matches!(read_fig(&bytes), Err(FigError::BadHeader(_))));
}
