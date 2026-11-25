#![no_main]

use fanta_fig_interop::{fig_to_doc, read_fig};
use libfuzzer_sys::fuzz_target;

// A `.fig` as it arrives from outside: the zip or bare fig-kiwi container,
// then the mapping into a document. Errors are expected; only a panic, a hang
// or runaway memory is a finding.
fuzz_target!(|data: &[u8]| {
    if let Ok(fig) = read_fig(data) {
        fig_to_doc(&fig).ok();
    }
});
