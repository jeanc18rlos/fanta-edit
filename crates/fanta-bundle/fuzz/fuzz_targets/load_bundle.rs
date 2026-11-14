#![no_main]

use std::io::{Cursor, Write};

use fanta_bundle::{RUNTIME_BUNDLE_VERSION, RuntimeProgram, checksum, strict_json};
use libfuzzer_sys::fuzz_target;
use serde_json::{Value, json};

// The archive as it arrives from outside, then the same bytes as the contents
// of a well-formed bundle: a mutated archive rarely passes the zip CRC and the
// manifest's SHA-256, so the first call alone never reaches the document. Up to
// the first NUL byte is the document; what follows, when there is a NUL, is
// declared as one asset and one font so resource verification runs too.
fuzz_target!(|data: &[u8]| {
    RuntimeProgram::load_bundle(data).ok();
    if let Some(bundle) = bundle_around(data) {
        RuntimeProgram::load_bundle(&bundle).ok();
    }
});

const ASSET_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

fn bundle_around(data: &[u8]) -> Option<Vec<u8>> {
    let mut parts = data.splitn(2, |byte| *byte == 0);
    let document = parts.next()?;
    let resource = parts.next();
    let schema_version = strict_json(document)
        .ok()?
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let mut manifest = json!({
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
    if let Some(resource) = resource {
        let digest = |path: &str| {
            json!({
                "path": path,
                "byte_length": resource.len(),
                "sha256": checksum(resource),
            })
        };
        manifest["assets"][ASSET_ID] = digest("assets/fuzz.bin");
        manifest["fonts"] = json!([{
            "family": "Fuzz",
            "weight": 400,
            "italic": false,
            "resource": digest("fonts/fuzz.bin"),
        }]);
    }
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("doc.json", options).ok()?;
    archive.write_all(document).ok()?;
    if let Some(resource) = resource {
        for path in ["assets/fuzz.bin", "fonts/fuzz.bin"] {
            archive.start_file(path, options).ok()?;
            archive.write_all(resource).ok()?;
        }
    }
    archive.start_file("manifest.json", options).ok()?;
    archive.write_all(manifest.to_string().as_bytes()).ok()?;
    Some(archive.finish().ok()?.into_inner())
}
