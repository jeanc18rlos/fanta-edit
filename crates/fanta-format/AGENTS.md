# fanta-format rules

The on-disk project format: reading, writing, three-way merge and the editing session that reconciles disk and memory. FNX printing and parsing rules live in `crates/fanta-fnx/AGENTS.md`.

PR numbers point at the change that found each trap (`gh pr view <n>`).

## Output and identity

- Written bytes are a pure function of the document (the byte-determinism contract). Never depend on `HashMap` or `serde_json::Map` iteration order: stage `source-order` runs this crate with `serde_json/preserve_order` enabled, and feature unification can enable it in any build.
- Persisted ids are bare ULIDs; file and directory names use the prefixed `Display` form. Convert with `json_key`/`id_from_key`, never `to_string()`/`FromStr` (see the root rules). (#85, #102, #110, #128)
- Never derive a design's directory from its id: page and component directories are name slugs. Use `locate_page_source`, `locate_master_source` and `page_scope_of_source`.
- Save ownership is the nearest ancestor-or-self registered component root, otherwise the page. When projecting one artifact from the shared scene, exclude descendant component roots owned by another artifact: geometric containment is not ownership. (#86)
- Never reformat `.fnx` or managed project JSON with another formatter. The engine printer is the canonical form, and a second formatter makes every open and save rewrite the file.

## Saving and reconciliation

- Saved geometry is authoritative. Explicit zero coordinates are complete geometry, and only values omitted from the decoded source (before node defaults apply) count as missing. The page root's omitted size is deliberate. Apply the same rule on cold reads and scoped materialization. (#51)
- Reuse a source aggregate hash only together with the exact per-file digests and filename order that produced it. The reconciliation base can lag behind disk during a conflict. (#53)
- `merge_docs` is an undo barrier and drops history on purpose: undo across a merge would revert the other side's edits. Do not carry history through a merge.
- Geometry signatures stored in project data use the source format's numeric normalization and stable object ordering. Otherwise a save, or a change in serde feature unification, invalidates preserved imported artwork. (#41)
- Move owned JSON fragments as `Value::Object`/`Value::Array`. `json!` serializes its arguments by reference and copies large geometry. (#126)

## Seeded project guide

- `AGENTS_MD` in `src/project/layout.rs` is written into every project. Only the exact previous seed is upgraded, so when you change `AGENTS_MD`, add the SHA-256 of the text you are replacing to `previous_seeds` in `seed_agents_md`. Otherwise existing projects keep the old guide, because any other content counts as user-customized. See `crates/design_surface/AGENTS.md` for the other places agent guidance lives.

## Tests

- When changing scene-to-FNX save projections, test a structural edit in the same artifact that carries authored source-only fields and comments, including Undo/Redo and reopen. Preserving an untouched artifact proves nothing about the edited one. (#87)
