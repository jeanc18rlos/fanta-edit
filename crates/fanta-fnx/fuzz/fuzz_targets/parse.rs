#![no_main]

use fanta_fnx::{
    FnxSidecar, FnxSourceMirror, canonicalize_legacy_source, decode_subtree, parse_doc,
};
use libfuzzer_sys::fuzz_target;

// One `.fnx` source through the parser, the source scanner (they read the same
// text and must agree, see AGENTS.md), the legacy rewrite and the decode into
// node records. Errors are expected; only a panic or a hang is a finding.
fuzz_target!(|source: &str| {
    let sidecar = FnxSidecar {
        root_parent: None,
        ids: Vec::new(),
    };
    parse_doc(source).ok();
    FnxSourceMirror::from_source(source, &sidecar).ok();
    canonicalize_legacy_source(source);
    decode_subtree(source, &sidecar).ok();
});
