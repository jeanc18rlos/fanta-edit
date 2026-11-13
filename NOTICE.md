# Notices

Portions of Fanta are modified from source code originally published by Zed
Industries, Inc. at https://github.com/zed-industries/zed. Fanta is not
affiliated with or endorsed by Zed Industries, Inc.

- Fanta, including the modified code, is licensed **GPL-3.0-or-later**.
  See `LICENSE-GPL`.
- Portions of that code are Apache-2.0; see `LICENSE-APACHE`, which retains the
  original copyright notice.
- The Fanta design engine is (c) Fanta and licensed **Apache-2.0**
  ([ADR 0026](docs/decisions/0026-choose-the-engine-and-sdk-licence.md)). It
  is the engine group of `docs/architecture/layers.toml`: `fanta-doc`,
  `fanta-design-ops`, `fanta-canvas`, `fanta-tools`, `fanta-fnx`,
  `fanta-format`, `fanta-bundle`, `fanta-text`, `fanta-render`,
  `fanta-present`, `fanta-headless`, `fanta-api`, `fanta-worker`, `fanta-mcp`,
  `fanta-runtime`, `fanta-site` and `fanta-wasm-engine`. Their `LICENSE-APACHE`
  links point at the licence text only: the Zed Industries copyright line at
  the top of that file covers the code inherited from Zed, not the engine.
  `fanta-fig-interop` belongs to the same group and stays GPL-3.0 until the
  origin of its `tests/golden` fixtures is settled (spec 0002, T2). See
  `crates/VENDORED.md` for the engine's upstream provenance. The reusable UI
  packages are published from [fanta-ui](https://github.com/squidred-dev/fanta-ui).

## Third-party material in the engine

The provenance sweep of 2026-11-13 (recorded in
`specs/0002-fanta-engine-repository/design.md`) found these. Each keeps its
own licence.

- `vendor/skia-bindings-0.153.3` is rust-skia's `skia-bindings`, MIT,
  Copyright (c) 2019 LongYinan & Armin Sander, with Fanta's changes to three
  files listed in its `FANTA-PROVENANCE.txt`. Its `LICENSE` is upstream's.
- `crates/fanta-text/assets` bundles Inter (Copyright (c) 2016 The Inter
  Project Authors) and Source Sans 3, Source Serif 4 and Source Code Pro
  (Adobe), all under the SIL Open Font License 1.1. Each folder carries its
  `OFL.txt`; the fonts are not relicensed.
- `crates/fanta-fig-interop/src/kiwi` is a Rust port of the Kiwi wire format,
  https://github.com/evanw/kiwi, MIT, Copyright (c) 2016-2023 Evan Wallace.
- The `.fig` mapping in `crates/fanta-fig-interop/src/mapping` and
  drag-to-reparent in `crates/fanta-tools/src/select/reparent.rs` port or
  mirror logic from two MIT projects named OpenPencil, and the layout and
  instance resolution in `crates/fanta-doc` follow their behaviour:
  https://github.com/open-pencil/open-pencil, Copyright (c) 2026 Danila
  Poyarkov and OpenPencil contributors, and
  https://github.com/ZSeven-W/openpencil, Copyright (c) 2026 ZSeven—W.
- `crates/fanta-tools/src/ink.rs` reimplements the algorithm of
  `perfect-freehand`, https://github.com/steveruizok/perfect-freehand, MIT,
  Copyright (c) 2021 Stephen Ruiz Ltd. The fixtures in `fixtures/ink` are that
  package's output.

The MIT License, which applies to the four MIT items above:

> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## Dependencies

Third-party dependency licenses are generated for each editor bundle into
`assets/licenses.md`, alongside the shared icon and theme notices, and are
readable in the app under **Help > View Dependency Licenses**. Without a
generated editor file, the shared UI asset package supplies its bundled notices.
