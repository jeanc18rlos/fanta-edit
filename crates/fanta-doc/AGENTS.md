# fanta-doc rules

The document model: `Scene` and its nodes, operations, history, components and variables. Ids and their persisted form are covered in the root rules.

- Component revisions: `ComponentDef.rev` is persisted, and `rev != 0` records that the master was edited since import, which decides whether Figma's baked `derivedSymbolData` fills still apply. Only committed ops move it (`Doc::apply` does this). A transient preview that writes into the scene without history (drag frame, text-edit preview, slider) calls `bump_preview_for_node(s)`, never `bump_rev_for_node(s)`.
- Abort a transaction that may contain doc-level ops (`DefineComponent`, motion tracks) with `Doc::abort_transaction`. `History::abort(&mut scene)` reverts through empty registries and leaves those ops applied; it is only correct for scene-only transactions such as a drag's `SetTransform`.
- Never call `Scene::invalidate_world_cache()` from a hot path. It drops every derived cache and logs `SceneChange::Unknown`, which forces the canvas to recopy the whole scene to the render thread. Mutate through the graph's own mutators, which log precise changes.
- Node stamps are not "did this move" signals: `set_transform` stamps only inside boolean ancestors. Use `Scene::changes_since` (the change log) to detect transform edits.
- A node kind that has children but no intrinsic bounds must be unioned in `Scene::compute_local_bounds` (as `Group` and boolean nodes are). Otherwise its bounds are `None` and it is never indexed, hit-tested or culled correctly.
- Siblings with equal `IndexKey`s paint in `NodeId` order, and ULIDs minted in the same millisecond order at random. Give each stacked sibling its own index (`IndexKey::after`), tests included: fanta-tools' `rect_at_origin` puts every rect at `IndexKey::FIRST`, and a click-through test that relied on creation order failed about a third of the time. (#165)
