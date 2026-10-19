# Export a website

`fanta-site` exports a saved Fanta project as a static website. Each design page
gets a URL with a canvas viewer, page navigation, pan and zoom. The pages use the
same Rust rendering engine as Fanta through `@fanta/runtime` WebAssembly.
Prototype interactions and motion playback are disabled in this read-only
viewer. Trigger inputs and a state-machine view are deferred to v2.1.

Build the runtime in the separate `Fanta-JS` repository first. Pass its
`packages/runtime` directory to the exporter; all five files must be present:

```text
packages/runtime/
  dist/fanta-runtime.mjs
  dist/fanta_runtime_wasm.wasm
  src/index.js
  site/viewer.js
  site/viewer.css
```

Use a runtime build pinned to the same Fanta engine revision as the exporter.
The runtime assets are copied from this directory at export time.

```sh
cargo run --locked -p fanta-site -- \
  "/path/to/My Design" "/path/to/site-output" \
  --runtime "/path/to/Fanta-JS/packages/runtime" \
  --offline
```

The project argument is the saved project directory containing `fanta.json`.
The output must be empty or a previous Fanta website export. Re-exporting
replaces generated files, removes obsolete page HTML and preserves other
hosting files such as `CNAME`.

Text faces are embedded in the runtime bundle. Add repeatable
`--font-dir "/path/to/licensed-fonts"` arguments for fonts you may publish, or
`--substitute "Brand Font=Source Sans 3"` to choose a replacement. Without
`--offline`, missing open-licensed fonts may be downloaded. Otherwise unavailable
families use bundled substitutes; the export report identifies substitutions.
If no embedded face can render the text, the exporter reports the missing
glyphs instead of creating a broken page. `--title "My site"` overrides the
document title.

Upload the complete output directory to a static host. For a local preview,
serve the directory over HTTP, for example `python3 -m http.server --directory
"/path/to/site-output" 8000`. The site loads its JavaScript, WebAssembly and
bundle from the generated `fanta/` directory. Live video and audio playback are
not exported; the runtime uses their authored posters.

The Rust export tests cover page naming, HTML escaping, font registration,
native bundle loading, re-export and refusal to overwrite an unrelated folder.
Browser playback is validated in the `Fanta-JS` repository.
