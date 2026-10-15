# fanta-fnx rules

FNX is the design-as-source text format. The in-memory `FnxElement` tree is always canonical; readable sugar exists only at the text boundary (`sugar.rs`).

- Preserve every finite number exactly: its JSON number kind and its `f64` bit pattern, including signed zero. Formatting preferences must never clamp or round persisted values. Cover changes with exact round-trip tests for scalar and nested JSON attributes. (#91)
- Print sugar only when it is strictly lossless: print → parse → print must be a fixpoint, with no float tolerance. A sugared spelling must never reach sidecar fingerprints, reconciliation or the document projection.
- `width`/`height` sugar folds into `clip_size` for frames and `local_size` for other nodes. It never folds on `Vector` (whose `local_size` is the SVG viewport clip, so folding would crop instead of resize), and page roots stay unsized.
- Output must not depend on map iteration order: stage `source-order` runs this crate with `serde_json/preserve_order` enabled.
