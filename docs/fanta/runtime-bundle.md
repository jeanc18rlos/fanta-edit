# Compile a runtime bundle

`fanta-compile` turns a saved Fanta project into one runtime bundle: a
`program.fanta` zip holding the compiled document, its assets and the fonts the
player needs, each pinned by checksum. The `fanta-runtime` package loads that
file with `loadBundle`. It has no source compiler: FNX and FSS are parsed here,
natively, and a page never receives project sources or the parser.

```sh
cargo run --locked -p fanta-site --bin fanta-compile -- \
  "/path/to/My Design" "/path/to/dist/program.fanta" --offline
```

The project argument is the saved project directory containing `fanta.json`.
Fonts are embedded exactly as for a [website export](./website-export.md):
`--font-dir` for fonts you may publish, `--substitute "Brand Font=Source Sans 3"`
to choose a replacement, and `--offline` to never download. Before the file is
written, the bundle is loaded in the native runtime and a player is created for
every page, so a bundle that compiles also plays.

In a page:

```js
import {createRuntime} from "fanta-runtime";

const runtime = await createRuntime();
const bytes = await fetch("/program.fanta").then(response => response.arrayBuffer());
const program = await runtime.loadBundle(bytes);
const player = await runtime.createPlayer({program, canvas});
```

Recompile whenever the project changes. The bundle records the document schema
version it was compiled with, and a runtime rejects a bundle from a newer
schema instead of guessing.

The runtime itself is built for the web with the `release-runtime` Cargo
profile (`cargo build --profile release-runtime --target
wasm32-unknown-emscripten -p fanta-runtime --bin fanta-runtime-wasm`, driven
from the `Fanta-JS` repository): size first, no debug names, and Skia compiled
with the same `-Os`. The editor and the headless engine keep the full
`release` profile and every Skia feature.

`fanta-site` runs the same compiler and wraps the bundle in a static website.
The Rust runtime's parity fixture (`playback_fixture`) compiles its input
natively and prints the bundle it played, so the browser tests in `Fanta-JS`
replay identical bytes.
