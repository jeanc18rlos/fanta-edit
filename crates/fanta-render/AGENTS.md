# fanta-render rules

Skia rendering of Fanta documents: the raster renderer, layer and instance caches, and Graphite on Metal. Graphite needs Fanta's native shims in `vendor/skia-bindings-0.153.3/src/graphite.cpp`; see `FANTA-PROVENANCE.txt` there.

PR numbers point at the change that found each trap (`gh pr view <n>`).

- `RenderInputs::mode_generation` advances whenever a variable value, active mode, frame mode pin or mode-affecting ancestor changes, and only then. It keys instance expansions: tying it to the render generation re-expands every instance on every frame, and forgetting to advance it serves stale alias-backed component properties.
- When retaining an effect layer across scene edits (`Scene::changes_since`), account for `PatternFill` sources outside its subtree. Ancestor and descendant invalidation alone misses those consumers; layers containing cross-tree pattern sources stay volatile. (#106)
- Images kept beyond the current draw (cached layers, retained frames) come from `immutable_surface_snapshot`. On Graphite, a plain `image_snapshot` shares the surface's texture and changes when the surface is reused. (#127)
- Raster-backed images and image shaders need a Graphite image provider or an explicit recorder upload; without one, Skia m153 silently omits them. The vendored shim installs a provider for recorders created without one, so keep that patch when updating skia-bindings. (#127)
- Clear GPU caches before replacing a backend, context or recorder: a freed recorder's address can be reused, and stale entries would match it.
- The canvas draws with `ImageDecode::Background`: `draw_image_cached` never decodes on the drawing thread and paints a pending wash or a coarser level until the resolver's workers land the level it needs. A resolver that decodes must override `resolve_level_nonblocking`, whose default decodes synchronously. Exports, screenshots and sampling need complete pixels, so they keep `ImageDecode::Blocking`. (#152)
- Skia in this workspace is built without WebP: `encoded_image_dimensions` and `decode_image_bytes` return `None` for WebP bytes. A `LazyAssetResolver` over app assets takes the app's header reader (`with_size_reader`); without it, a background draw cannot pick a level and decodes WebP at natural resolution. (#152)
