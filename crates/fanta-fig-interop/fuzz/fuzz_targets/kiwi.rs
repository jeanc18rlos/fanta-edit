#![no_main]

use fanta_fig_interop::{KiwiType, KiwiValue, Schema};
use libfuzzer_sys::fuzz_target;

// The Kiwi codec without the compressed container around it, which a mutated
// input rarely gets through: a little-endian u16 schema length, the binary
// schema, then a value of its `Message` definition (the root `read_fig` decodes),
// or of the first one.
fuzz_target!(|data: &[u8]| {
    let Some((length, rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let Some((schema, value)) = rest.split_at_checked(u16::from_le_bytes(*length) as usize) else {
        return;
    };
    if let Ok(schema) = Schema::decode_binary(schema) {
        let root = schema.def_index("Message").unwrap_or(0);
        KiwiValue::decode(&schema, KiwiType::user(root), value).ok();
    }
});
