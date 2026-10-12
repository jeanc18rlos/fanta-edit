# Public Alpha visual assets

The reviewed recordings and screenshots use the accepted signed Fanta 1.0.0
Build45 app. The old README's `canvas.png` and `code.png` files were absent.
Current guidance links actual native captures, silent action GIFs and complete
captioned workflows. Public availability remains on hold pending website review
and publication checks.

## Reviewed launch media — 12 October 2026

Files under `public-alpha-20261012/` were copied from reviewed actual recordings.
The native and agent videos are 68 and 90 seconds; the source workflow is about
70 seconds. A 53-second overview combines actual intervals from those workflows.
Captions identify later PNG, Git and reopened-source checkpoints. The agent
route is external Opus 5.5 through Fanta MCP. Full capture identities and hashes
remain in the private media evidence; the [distribution guide](../alpha/PUBLIC_ALPHA.md#accepted-package-and-publication-status)
records the accepted package's identity and scope.

| Workflow | Current files | Observable result |
| --- | --- | --- |
| Native editing | `design-in-fanta-poster.jpg`, `-01.png`, `-02.png`, `-03-png-preview.png`, `design-in-fanta.gif` | Imported native layers, Undo/Redo, reopened saved project and actual PNG |
| Source and Git | `your-design-is-source-poster.jpg`, `-01.png`, `-02.png`, `-03-git.png`, `your-design-is-source.gif` | Editable FNX, canvas update and the saved project's Git diff |
| Agent edit | `agent-edits-your-design-poster.jpg`, `-01.png`, `-02.png`, `agent-edits-your-design.gif` | Bounded native CTA edit, saved FNX and Undo/Redo |
| Overview | `fanta-launch-montage-poster.jpg` | Actual excerpts of the three workflows |

The [demo guide](../alpha/DEMO_GUIDE.md) links each real poster to the complete
captioned website player. Documentation retains eight screenshots
and three short GIFs; original recordings, music and editing projects stay
private. The owned sample and its original `.fig` live in `docs/samples/`.

## Historical native QA screenshots — 12 October 2026

Captured **12 October 2026** from the payload-equivalent isolated **Build45 QA**
app. Bounded native canvas, FNX edit/Save/reopen and Git-review checks passed.
These retain their internal QA capture identity. The separate signed Build45
package has passed notarization, stapling and bounded installation checks,
recorded in [the distribution guide](../alpha/PUBLIC_ALPHA.md#accepted-package-and-publication-status).
This earlier QA record retains its original build identity. Files were copied
without cropping, reconstruction or pixel changes.

- Source: `57d86330289153ead8a3bdbd114e1fa530a74e03`.
- Tree: `b59e039caa1d0197095f90f5589179eba76b8a90`.
- Hosted build: `38180199038`, attempt 1, internal build 45.
- QA executable SHA-256: `a4e3526819d41b247a7f5667a599093482171cad43d1c89573b4feafb43bf886`.
- Tested native host: macOS 27.0 build 26A428, Apple Silicon. This is not a
  minimum-version or signed-installer compatibility claim.

| File | Actual native surface | SHA-256 |
| --- | --- | --- |
| `build45-canvas.png` | Owned Launch Card canvas and frame inspector before editing | `ef06fecea5da54bc4f8a32fbf569c5bae8ddb20420d91b6857a2a3d0e156fde7` |
| `build45-source.png` | Code workspace with the saved FNX text change | `c5277db67a36a9ba85146ec5ca3bf1a624bd0ed9ac51583d1a37ea90638cf036` |
| `build45-diff.png` | Review Changes after the frame's saved X-position edit | `fc302ba1d8dfbbe81c5dbd22e4fdd0a17e544548777d84b05cf893f6bfa40897` |
