//! The `.fig` container reader (and a symmetric writer).
//!
//! **Verified against a real Figma export** (the Adobe Spectrum Design System
//! community file). The format has two nestings:
//!
//! ```text
//! Outer: a ZIP archive (magic "PK\x03\x04") containing
//!        canvas.fig, thumbnail.png, meta.json, images/<sha1>
//! Inner (canvas.fig):
//!   offset 0  : 8-byte ASCII magic "fig-kiwi"
//!   offset 8  : u32 little-endian version (observed: 101)
//!   offset 12 : chunk[0] -> embedded Kiwi binary schema, RAW DEFLATE
//!   ...       : chunk[1] -> Kiwi-encoded document message, ZSTANDARD
//!   ...       : chunk[2..] (rare) -> additional blobs, ignored
//! ```
//!
//! Each chunk is `[u32 little-endian length][length bytes of payload]`. The
//! **schema** chunk is *raw* DEFLATE (no zlib header). The **data** chunk in
//! current Figma exports is **Zstandard** (frame magic `28 B5 2F FD`); older
//! files used DEFLATE there too. We sniff each chunk's payload by magic and
//! decompress accordingly, so both vintages parse. Some bare `.fig` streams
//! (clipboard, older exports) are *not* zipped — we accept a raw fig-kiwi
//! stream as well as the zipped form.
//!
//! The *Kiwi* layer underneath (see [`crate::kiwi`]) is exact and
//! round-trip-proven; this framing is now confirmed end-to-end on a real file.

use crate::error::{FigError, FigResult};
use crate::kiwi::{KiwiType, KiwiValue, Schema};
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod tests;

/// The 8-byte fig-kiwi stream magic.
pub const FIG_MAGIC: &[u8; 8] = b"fig-kiwi";

/// ZIP local-file-header magic. Modern `.fig` files are zip archives whose
/// `canvas.fig` entry holds the fig-kiwi stream.
const ZIP_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];

/// The entry inside the `.fig` zip that holds the fig-kiwi stream.
const ZIP_CANVAS_ENTRY: &str = "canvas.fig";

/// The ZIP directory prefix under which a `.fig` stores embedded bitmap bytes.
/// Each entry is `images/<hash>`, where `<hash>` is the lowercase-hex SHA-1 a
/// `Paint.image.hash` references; the bytes are the raw PNG/JPEG file.
const ZIP_IMAGES_PREFIX: &str = "images/";

/// Zstandard frame magic — the data chunk's compression in current exports.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Embedded bitmap bytes keyed by their lowercase-hex SHA-1 hash — the same
/// hash an IMAGE `Paint.image.hash` references and the key under which the
/// `.fig` ZIP stores the file at `images/<hash>`.
pub type ImageMap = HashMap<String, Vec<u8>>;

/// Versions seen in the wild (101). The chunk framing has been stable across
/// fig-kiwi versions, so we read any version leniently and record it rather
/// than gate on an allow-list a routine Figma bump would trip.
pub const OBSERVED_VERSION: u32 = 101;

/// A parsed `.fig` document: its embedded schema, the decoded root value, and
/// any trailing chunks left undecoded (images and other blobs that v0 does not
/// interpret).
#[derive(Debug)]
pub struct FigDocument {
    /// The version read from the header.
    pub version: u32,
    /// The Kiwi schema embedded in the file.
    pub schema: Schema,
    /// The decoded document tree (the root message).
    pub root: KiwiValue,
    /// The name of the root message type, for the mapping layer's convenience.
    pub root_type_name: String,
    /// The root message's top-level `blobs` array, flattened to raw byte
    /// vectors indexable by a `commandsBlob` index. Figma stores vector path
    /// commands (and other binary payloads) here: a `Path.commandsBlob` is a
    /// `uint` index into this vector, and `blobs[i]` is that path's command
    /// byte stream (see [`crate::mapping`] for the decode). Empty when the file
    /// carries no `blobs` field (older / synthetic / hand-built documents).
    pub blobs: Vec<Vec<u8>>,
    /// Embedded bitmap bytes, keyed by their lowercase-hex SHA-1 hash (the same
    /// hash an IMAGE `Paint.image.hash` references). Each value is the raw image
    /// file (PNG or JPEG) exactly as stored in the `.fig` ZIP's `images/<hash>`
    /// entry — *not* decoded here (the codec lives in the app / asset layer, so
    /// this crate stays dependency-light). Empty for a bare fig-kiwi stream
    /// (clipboard / older exports) or a `.fig` with no image fills. See
    /// [`crate::mapping::fig_to_doc`], which mints a deterministic `AssetId` per
    /// referenced hash and hands the bytes back for the renderer's resolver.
    pub images: ImageMap,
}

/// Extract the root message's `blobs: Blob[]` array as raw byte vectors.
///
/// Each `Blob` is the Kiwi struct `{ bytes: byte[] }` (confirmed against the
/// real Figma schema: def `Blob`, single array field `bytes` of `byte`), so it
/// decodes to an object whose `bytes` field is a [`KiwiValue::Array`] of
/// [`KiwiValue::Byte`]. We flatten each to a `Vec<u8>` so the mapping layer can
/// index `blobs[commandsBlob]` directly. A blob in an unexpected shape becomes
/// an empty vec rather than aborting the import (tolerant by design).
fn extract_blobs(root: &KiwiValue) -> Vec<Vec<u8>> {
    let Some(arr) = root.get("blobs").and_then(KiwiValue::as_array) else {
        return Vec::new();
    };
    arr.iter().map(blob_to_bytes).collect()
}

/// Flatten one `Blob` value (`{ bytes: byte[] }`, or a bare byte array) to a
/// `Vec<u8>`.
fn blob_to_bytes(blob: &KiwiValue) -> Vec<u8> {
    let bytes = match blob {
        KiwiValue::Array(a) => a,
        KiwiValue::Object { fields, .. } => {
            match fields.get("bytes").and_then(KiwiValue::as_array) {
                Some(a) => a,
                None => return Vec::new(),
            }
        }
        _ => return Vec::new(),
    };
    bytes
        .iter()
        .map(|b| match *b {
            KiwiValue::Byte(x) => x,
            // Defensive: a `byte` field always decodes to `Byte`, but accept
            // small uints/ints too so a quirk in a future schema can't panic.
            KiwiValue::Uint(x) => x as u8,
            KiwiValue::Int(x) => x as u8,
            _ => 0,
        })
        .collect()
}

/// The Kiwi message type Figma uses as the document root.
///
/// ASSUMPTION: `fig-kiwi` decodes the data chunk as the `Message` type, which
/// is Figma's root. If a particular file names its root differently we fall
/// back to the first message definition in the schema (see [`pick_root_type`]).
const FIGMA_ROOT_TYPE: &str = "Message";

/// Read and decode a `.fig` file from its raw bytes.
///
/// Accepts both forms: a ZIP archive (current Figma exports — we pull the
/// `canvas.fig` entry plus every `images/<hash>` bitmap) and a bare fig-kiwi
/// stream (clipboard / older exports — no sibling images, so the returned
/// [`FigDocument::images`] is empty).
pub fn read_fig(bytes: &[u8]) -> FigResult<FigDocument> {
    if bytes.len() >= 4 && bytes[0..4] == ZIP_MAGIC {
        let (canvas, images) = extract_zip_entries(bytes)?;
        decode_fig_kiwi(&canvas, images)
    } else {
        decode_fig_kiwi(bytes, ImageMap::new())
    }
}

/// Pull the `canvas.fig` stream and every `images/<hash>` bitmap out of a `.fig`
/// ZIP archive in a single pass.
///
/// The `images` map is keyed by the entry's hash (the bytes after `images/`),
/// which is exactly what a `Paint.image.hash` references once hex-encoded. The
/// zero-length `images/` directory entry itself is skipped. A bitmap entry that
/// fails to read is logged and skipped rather than aborting the whole import —
/// a single corrupt image must not cost the entire document.
fn extract_zip_entries(bytes: &[u8]) -> FigResult<(Vec<u8>, ImageMap)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| FigError::BadHeader(format!("not a valid .fig zip: {e}")))?;

    let mut canvas: Option<Vec<u8>> = None;
    let mut images: ImageMap = ImageMap::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| FigError::BadHeader(format!("reading .fig zip entry {i}: {e}")))?;
        let name = entry.name().to_owned();
        if name == ZIP_CANVAS_ENTRY {
            let mut out = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut out)
                .map_err(|e| FigError::Inflate(format!("reading {ZIP_CANVAS_ENTRY}: {e}")))?;
            canvas = Some(out);
        } else if let Some(hash) = name.strip_prefix(ZIP_IMAGES_PREFIX) {
            // Skip the directory entry itself (`images/`, empty hash).
            if hash.is_empty() {
                continue;
            }
            let mut out = Vec::with_capacity(entry.size() as usize);
            match entry.read_to_end(&mut out) {
                Ok(_) => {
                    images.insert(hash.to_owned(), out);
                }
                Err(e) => {
                    tracing::warn!(
                        target: "fanta-fig-interop",
                        "skipping unreadable image entry {name}: {e}"
                    );
                }
            }
        }
    }

    let canvas = canvas.ok_or_else(|| {
        FigError::BadHeader(format!("'{ZIP_CANVAS_ENTRY}' missing from .fig zip"))
    })?;
    Ok((canvas, images))
}

/// Decode a bare fig-kiwi stream (the `canvas.fig` contents), attaching the
/// already-extracted `images` map.
///
/// Steps: validate the header, split the length-prefixed chunks, decompress
/// the schema chunk and decode it with the Kiwi codec, then decompress the
/// data chunk and decode it into a [`KiwiValue`] tree against that schema.
fn decode_fig_kiwi(bytes: &[u8], images: ImageMap) -> FigResult<FigDocument> {
    // --- header ---
    if bytes.len() < 12 {
        return Err(FigError::Truncated);
    }
    if &bytes[0..8] != FIG_MAGIC {
        return Err(FigError::BadHeader(format!(
            "expected magic {:?}, found {:?}",
            std::str::from_utf8(FIG_MAGIC).unwrap_or("<magic>"),
            String::from_utf8_lossy(&bytes[0..8])
        )));
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    // Versions are accepted leniently (framing is version-stable); record it.
    tracing::debug!(target: "fanta-fig-interop", "fig-kiwi version {version}");

    // --- chunks ---
    let chunks = split_chunks(&bytes[12..])?;
    if chunks.len() < 2 {
        return Err(FigError::BadHeader(format!(
            "expected at least 2 chunks (schema + data), found {}",
            chunks.len()
        )));
    }

    // --- schema (raw DEFLATE) ---
    let schema_bytes = decompress_chunk(chunks[0])?;
    let schema = Schema::decode_binary(&schema_bytes)?;

    // --- data (Zstd in current exports, DEFLATE in older) ---
    let data_bytes = decompress_chunk(chunks[1])?;
    let root_type_name = pick_root_type(&schema)?;
    let root_index = schema.def_index(&root_type_name).ok_or_else(|| {
        FigError::Schema(format!("root type '{root_type_name}' missing from schema"))
    })?;
    let root = KiwiValue::decode(&schema, KiwiType::user(root_index), &data_bytes)?;
    let blobs = extract_blobs(&root);

    Ok(FigDocument {
        version,
        schema,
        root,
        root_type_name,
        blobs,
        images,
    })
}

/// Write a `.fig` file: the symmetric inverse of [`read_fig`].
///
/// Produces a header, the zlib-compressed binary schema chunk, and the zlib-
/// compressed encoded-data chunk. This is primarily a test aid (it lets the
/// suite construct real container bytes and read them back) — `.fig` *export*
/// is an explicit non-goal for the product (ARCHITECTURE.md §12), and a file
/// written here is not guaranteed byte-identical to one Figma would emit
/// (Figma may use zstd for the data chunk and append image chunks).
pub fn write_fig(doc: &FigDocument) -> FigResult<Vec<u8>> {
    let root_index = doc
        .schema
        .def_index(&doc.root_type_name)
        .ok_or_else(|| FigError::Schema(format!("root type '{}' missing", doc.root_type_name)))?;
    // The root index pins which message we encode; the value carries its own
    // type name, so this is just an existence check.
    let _ = root_index;

    let schema_bytes = doc.schema.encode_binary();
    let data_bytes = doc.root.encode(&doc.schema)?;

    let mut out = Vec::new();
    out.extend_from_slice(FIG_MAGIC);
    out.extend_from_slice(&doc.version.to_le_bytes());
    write_chunk(&mut out, &deflate(&schema_bytes)?)?;
    write_chunk(&mut out, &deflate(&data_bytes)?)?;
    Ok(out)
}

/// Pick the root message type from a schema.
///
/// ASSUMPTION: Figma's root is named `Message`. When that is absent (e.g. a
/// hand-built schema in tests, or a future rename) we fall back to the first
/// message definition, which is a reasonable heuristic for a Kiwi document.
fn pick_root_type(schema: &Schema) -> FigResult<String> {
    if schema.def(FIGMA_ROOT_TYPE).is_some() {
        return Ok(FIGMA_ROOT_TYPE.to_owned());
    }
    schema
        .defs
        .iter()
        .find(|d| matches!(d.kind, crate::kiwi::DefKind::Message))
        .map(|d| d.name.clone())
        .ok_or_else(|| FigError::Schema("schema contains no message definitions".to_owned()))
}

/// Split a buffer into `[u32 LE length][payload]` chunks until exhausted.
///
/// ASSUMPTION: chunks are tightly packed with no inter-chunk padding and a
/// little-endian 32-bit length prefix. This is the layout the `fig-kiwi`
/// readers use. A length that overruns the buffer is reported as truncation.
fn split_chunks(mut data: &[u8]) -> FigResult<Vec<&[u8]>> {
    let mut chunks = Vec::new();
    while !data.is_empty() {
        if data.len() < 4 {
            return Err(FigError::Truncated);
        }
        let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let start = 4usize;
        let end = start.checked_add(len).ok_or(FigError::Truncated)?;
        if end > data.len() {
            return Err(FigError::Truncated);
        }
        chunks.push(&data[start..end]);
        data = &data[end..];
    }
    Ok(chunks)
}

/// Append a `[u32 LE length][payload]` chunk to `out`.
fn write_chunk(out: &mut Vec<u8>, payload: &[u8]) -> FigResult<()> {
    let len: u32 = payload
        .len()
        .try_into()
        .map_err(|_| FigError::Schema("chunk exceeds u32 length".to_owned()))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

/// Decompress one chunk's payload, sniffing the codec by leading magic:
/// Zstandard (`28 B5 2F FD`) for current data chunks, raw DEFLATE otherwise
/// (the schema chunk, and data chunks in older exports).
fn decompress_chunk(payload: &[u8]) -> FigResult<Vec<u8>> {
    if payload.len() >= 4 && payload[0..4] == ZSTD_MAGIC {
        zstd::stream::decode_all(payload).map_err(|e| FigError::Inflate(format!("zstd: {e}")))
    } else {
        let mut decoder = DeflateDecoder::new(payload);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| FigError::Inflate(e.to_string()))?;
        Ok(out)
    }
}

/// Raw-DEFLATE a byte slice (default compression). Used by [`write_fig`], the
/// test-aid writer; matches the schema chunk's real compression.
fn deflate(raw: &[u8]) -> FigResult<Vec<u8>> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw)
        .map_err(|e| FigError::Inflate(e.to_string()))?;
    encoder
        .finish()
        .map_err(|e| FigError::Inflate(e.to_string()))
}
