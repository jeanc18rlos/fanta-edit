#![no_main]

use std::io::{Cursor, Write};

use fanta_bundle::{RUNTIME_BUNDLE_VERSION, RuntimeProgram, checksum, strict_json};
use libfuzzer_sys::fuzz_target;
use serde_json::{Value, json};

// The archive as it arrives from outside, then the same bytes as the document
// of a well-formed bundle: a mutated archive rarely passes the zip CRC and the
// manifest's SHA-256, so the first call alone never reaches the document.
fuzz_target!(|data: &[u8]| {
    RuntimeProgram::load_bundle(data).ok();
    if let Some(bundle) = bundle_around(data) {
        RuntimeProgram::load_bundle(&bundle).ok();
    }
});

fn bundle_around(document: &[u8]) -> Option<Vec<u8>> {
    let schema_version = strict_json(document)
        .ok()?
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let manifest = json!({
        "format": "fanta-runtime",
        "version": RUNTIME_BUNDLE_VERSION,
        "schema_version": schema_version,
        "document": {
            "path": "doc.json",
            "byte_length": document.len(),
            "sha256": checksum(document),
        },
        "assets": {},
        "fonts": [],
        "font_substitutions": {},
    });
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("doc.json", options).ok()?;
    archive.write_all(document).ok()?;
    archive.start_file("manifest.json", options).ok()?;
    archive.write_all(manifest.to_string().as_bytes()).ok()?;
    Some(archive.finish().ok()?.into_inner())
}
