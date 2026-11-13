#![no_main]

use fanta_fnx::{
    FnxError, FnxSidecar, FnxSourceMirror, canonicalize_legacy_source, decode_subtree,
    reconcile_sidecar,
};
use libfuzzer_sys::fuzz_target;

// One `.fnx` source through the parser, the source scanner, the legacy rewrite
// and, with the sidecar that reconciliation builds for it, the decode into node
// records. The parser and the scanner read the same text (AGENTS.md), so a
// source one accepts and the other rejects is a finding, like a panic or a hang.
fuzz_target!(|source: &str| {
    canonicalize_legacy_source(source);
    let empty = FnxSidecar {
        root_parent: None,
        ids: Vec::new(),
    };
    let mut next = 0u32;
    let reconciled = reconcile_sidecar(source, &empty, || {
        next += 1;
        format!("fuzz-{next}")
    });
    match reconciled {
        Ok(sidecar) => {
            assert!(
                FnxSourceMirror::from_source(source, &sidecar).is_ok(),
                "the parser accepts a source the scanner rejects"
            );
            decode_subtree(source, &sidecar).ok();
        }
        Err(FnxError::Parse(_)) | Err(FnxError::UnknownTag(_)) => {}
        Err(error) => panic!("unexpected reconcile error: {error}"),
    }
});
