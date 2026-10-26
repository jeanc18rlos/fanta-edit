# Engine parity with CE.SDK

IMG.LY's CreativeEditor SDK (CE.SDK) is a headless design engine: scenes of
blocks edited through typed APIs, with templating, video and print export.
This page compares it with Fanta's engine and orders the work to close the
gaps. It was written on 23 October 2026 from CE.SDK's public documentation and
API reference for `@cesdk/node-native`.

Clean-room rule: use only CE.SDK's public documentation. Do not decompile or
disassemble its binaries, and do not copy its shaders, LUTs, fonts, sample
code or names. Fanta's API keeps its own names, ids and property paths; a
compatibility adapter is a separate, optional layer.

## How CE.SDK is built

- One single-threaded core holds one scene. Everything is a block, including
  the fills, shapes, effects, blurs and animations attached to a design block.
- Properties are typed slash keys (`opacity`, `fill/image/imageFileURI`) with
  reflection: list a block's properties, read a property's type and its enum
  values.
- Events are bundled per update cycle as `{block, Created | Updated |
  Destroyed}`. Undo steps are explicit: edits apply at once and become one
  step when the caller adds it.
- Templates use `{{name}}` text variables, placeholders that content can
  replace, 25 per-block permission scopes, global scope rules and roles.
- Media goes through VideoToolbox on macOS and GStreamer on Linux and Windows.
  Video export is MP4 only (H.264 with AAC audio).
- Importers (IDML, PSD, PDF, PPTX) are separate packages that build scenes
  through the public API. CMYK print output and background removal are
  plugins; the base PDF export is RGB.

## Terms

| CE.SDK | Fanta |
| --- | --- |
| block, `DesignBlockId` | node, `NodeId` (`n_…` outside Rust) |
| scene, page | document or project, page (a root frame in `Doc.pages`) |
| fill, effect, blur blocks | values inside a node (`fills`, `effects`, `blurs`) |
| text variable | String variable in the document's variable registry |
| scope, role, placeholder | the same concepts, with Fanta's snake_case keys |

## Gap matrix

✅ have · 🟡 partial · ❌ missing. The phase column refers to the roadmap below.

| Area | CE.SDK | Fanta | Phase |
| --- | --- | --- | --- |
| Headless editing | edits in Node without a UI | ✅ `fanta-headless` applies design ops with the editor's applier, renders PNG and saves | 0 |
| Design API | typed properties, lifecycle, hierarchy, find, metadata | ✅ `fanta-api`'s `DesignSession` | 1 |
| Events | per-block Created, Updated, Destroyed | ✅ polled per-node events in `fanta-api` | 1 |
| Undo | explicit steps, several histories | 🟡 explicit steps in `fanta-api`; one history | 1 |
| Templating | text variables, placeholders, scopes, roles | ✅ in `fanta-api`: `{{name}}` text variables, placeholders, 25 scopes and three roles. 🟡 The editor's Fill mode keeps canvas gestures, Delete, Duplicate, text editing and the Design inspector's edits to the scopes, but not yet arranging or transforming a multi-selection, variable bindings or grouping. The inspector cannot show or edit the scopes, and placeholders have no overlay | 2 (editor) |
| Scene I/O | scenes in memory, strings, archives | ✅ sessions in memory, from project folders or from `.fant` snapshots | 1 |
| Export | PNG, JPEG, WebP, TGA, PDF, SVG, target size | 🟡 PNG, JPEG, SVG and PDF from the editor, the engine and `fanta-api`; no WebP or TGA | 5 |
| Blocks and shapes | graphic, text, group, page, audio; six shapes; booleans | ✅ | — |
| Fills | color, three gradients, image fit and crop, video | ✅ plus diamond, pattern and shader fills; 🟡 crop rotates in quarter turns | 3 |
| Strokes | start, end and dash caps; dash offset; overprint | 🟡 none of these | 3, 5 |
| Effects | a 22-effect stack: 12-knob adjustments, LUT, duotone, chroma key, recolor, vignette, pixelize, posterize, glow, tilt-shift… | ❌ 7-knob adjustments on image and video fills only | 3 |
| Blur and blend | 4 blur kinds, 27 blend modes | 🟡 layer and background blur. ✅ 26 blend modes plus Pass through on groups; PDF has no Linear Burn, Darker or Lighter Color, Vivid, Linear or Pin Light, Hard Mix, Subtract or Divide, so PDF export draws those as Normal, and SVG export draws every mode as Normal | 3 |
| Text | ranged case, super- and subscript, lists, per-paragraph styles, emoji, fonts by URI | 🟡 rich runs, variable fonts, auto-resize, text on a path; Linux has no emoji font and project fonts are not registered | 3 |
| Animation | 25 in, out and loop presets, per-letter text animation | 🟡 keyframe clips; no presets or per-glyph animation | 4 |
| Video timeline | page duration, offsets, trim, tracks, split, 21 transitions, captions | 🟡 trim, speed and volume on video layers | 4 |
| Media I/O | MP4 and MOV decode; MP4 export with audio | 🟡 decode on macOS only; MP4 through an external `ffmpeg`, editor-only, silent | 4 |
| Assets | asset sources, queries, URI resolver, buffers | 🟡 project assets addressed by content hash; `fanta-api` fetches template images from `data:` URIs, a directory or https, as the caller allows; no asset sources or queries | 2 |
| Automation | data merge, auto-resize, batch generation | 🟡 `fanta-api` fills a template from a data row, batch-exports one image per row, and resizes a frame by its constraints or by fitting its content; the cloud engine fills and batch-exports through its `fill_template` and `export_batch` steps | 2 |
| Color and print | CMYK, spot colors, ICC, mm and inches, DPI, bleed, underlayer, cutouts, crop marks | ❌ 8-bit sRGB and pixels only | 5 |
| Distribution | native Node package, engine plugins | ❌ engine bindings (the web runtime only plays prototypes); the engine is GPL-3.0 | 6 |
| Importers and extras | PSD, IDML, PDF, PPTX, Lottie; background removal, QR codes, dominant colors, validation | ❌ (✅ `.fig`, limited SVG; cloud segmentation) | 7 |

Fanta has what CE.SDK does not: designs as FNX source in git, components and
variants with overrides, variables with modes, auto layout and grid, a
prototype runtime with Smart Animate, keyframe motion, `.fig` import, agents
over MCP and cloud review. The plan keeps all of them.

## Roadmap

0. **Engine apply** (done). The design-op applier lives in `fanta-headless`,
   so the engine and the editor apply the same ops and the cloud worker's
   `apply` step works.
1. **Design API** (done, `crates/fanta-api`). Scenes in memory or on disk, typed
   property reflection, node lifecycle, explicit undo steps, polled change
   events, `{{name}}` text variables, placeholders and permission scopes, and
   PNG, JPEG, SVG and PDF export.
2. **Templates in the editor, and automation.** The editor honours scopes,
   placeholders and roles. Asset sources, a URI resolver and remote fetch.
   Data merge, batch export and content-aware resize as engine steps.
3. **Effects and text.** A node-level effect stack of SkSL runtime effects,
   progressive blurs, free crop rotation and flip; text case, scripts, lists,
   per-paragraph styles, a bundled emoji font and the project's own fonts. The
   missing blend modes are done, as SkSL blenders.
4. **Timeline and video.** Page duration, offsets, trim, tracks, split and clip
   transitions; animation presets compiled to motion tracks; captions; a media
   crate (AVFoundation on macOS, GStreamer or FFmpeg libraries on Linux) and
   MP4 export with audio.
5. **Print.** Units, DPI, bleed, CMYK and spot colors, and a print PDF backend
   with separations, overprint, page boxes, marks and PDF/X; cutouts,
   underlayers and exclusion areas.
6. **Distribution.** A napi-rs package over `fanta-api` with prebuilt binaries,
   an engine plugin trait and a WASM editing build. This needs a licence
   decision first: the engine crates are GPL-3.0.
7. **Importers and extras.** PSD, IDML, PPTX, PDF and Lottie importers through
   `fanta-format`'s format registry; QR codes, dominant colors, validation and
   brand rules; background removal through Apple Vision on device or a
   permissively licensed model in the cloud (not IMG.LY's AGPL library); and an
   optional CE.SDK compatibility adapter.
