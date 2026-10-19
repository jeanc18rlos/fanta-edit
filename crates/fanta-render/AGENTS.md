# fanta-render rules

Skia rendering of Fanta documents: the raster renderer, layer and instance caches, and Graphite on Metal. Graphite needs Fanta's native shims in `vendor/skia-bindings-0.153.3/src/graphite.cpp`; see `FANTA-PROVENANCE.txt` there.

PR numbers point at the change that found each trap (`gh pr view <n>`).

- `RenderInputs::mode_generation` advances whenever a variable value, active mode, frame mode pin or mode-affecting ancestor changes, and only then. It keys instance expansions: tying it to the render generation re-expands every instance on every frame, and forgetting to advance it serves stale alias-backed component properties.
- When retaining an effect layer across scene edits (`Scene::changes_since`), account for `PatternFill` sources outside its subtree. Ancestor and descendant invalidation alone misses those consumers; layers containing cross-tree pattern sources stay volatile. (#106)
- Images kept beyond the current draw (cached layers, retained frames) come from `immutable_surface_snapshot`. On Graphite, a plain `image_snapshot` shares the surface's texture and changes when the surface is reused. (#127)
- Raster-backed images and image shaders need a Graphite image provider or an explicit recorder upload; without one, Skia m153 silently omits them. The vendored shim installs a provider for recorders created without one, so keep that patch when updating skia-bindings. (#127)
- Clear GPU caches before replacing a backend, context or recorder: a freed recorder's address can be reused, and stale entries would match it.
- On Windows, skia-safe getters that call a C++ method returning a struct by value crash: MSVC passes the result pointer after `this`, and the binding passes it first. In 0.153.3 these are `Canvas::top_props`, `Canvas::base_props`, `Path::get_point` and `Pixmap::get_color_4f`. Use `layer_cache::top_surface_props`, and read a skia-safe getter's body before relying on it in code Windows runs; the safe ones call a `C_` shim. (#149)
