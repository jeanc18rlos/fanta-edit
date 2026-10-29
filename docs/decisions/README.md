# Decision records

Each record here captures one decision that shapes Fanta: the context, the
options, what we chose and what it costs. Read the records that touch an
area before changing it. The [process guide](../process/spec-driven-development.md)
explains when a change needs one.

## Writing a record

```bash
script/architecture new adr "Move the engine into its own repository"
```

The command copies [`template.md`](template.md) to the next number and adds
the record to the index below. Fill it in, then open a pull request: merging
it with `status: accepted` is the approval. Agents write `proposed` and leave
acceptance to the founder.

| Front matter | Meaning |
| --- | --- |
| `status` | `proposed`, `accepted`, `rejected`, `deprecated` or `superseded` |
| `date` | When the decision was made (or proposed) |
| `recorded` | When the record was written, if later than `date` |
| `decision-makers` | Who decided |
| `supersedes`, `superseded-by` | Record numbers, kept consistent on both sides |
| `tags` | Optional, for example `[engine, repository]` |

Records 0002 to 0022 were reconstructed on 2026-10-27 from the history. Their
`date` is when the decision was made and `recorded` is when it was written
down; each cites the commits, pull requests or notes it rests on.

`script/architecture index --check` runs in CI. It fails when a record breaks
these rules or when the index below is stale.

## Index

<!-- decisions:start -->
| ADR | Decision | Status | Date |
| --- | --- | --- | --- |
| [0001](0001-record-decisions-and-plan-changes-in-specs.md) | Record decisions in ADRs and plan changes in specs | proposed | 2026-10-27 |
| [0002](0002-build-fanta-on-a-fork-of-zed.md) | Build Fanta on a fork of Zed | accepted | 2026-07-07 |
| [0003](0003-keep-skia-out-of-the-document-model.md) | Keep Skia out of the document model | accepted | 2026-07-09 |
| [0004](0004-store-designs-as-project-folders-of-fnx-source.md) | Store designs as project folders of FNX source | accepted | 2026-07-07 |
| [0005](0005-let-agents-edit-designs-through-project-files.md) | Let agents edit designs through project files | superseded by [0007](0007-give-agents-a-live-design-seam-and-mcp-server.md) | 2026-07-07 |
| [0006](0006-build-the-engine-from-a-sibling-checkout.md) | Build the engine from a sibling checkout | superseded by [0008](0008-vendor-the-engine-into-this-repository.md) | 2026-07-10 |
| [0007](0007-give-agents-a-live-design-seam-and-mcp-server.md) | Give agents a live design seam and an MCP server | accepted | 2026-07-11 |
| [0008](0008-vendor-the-engine-into-this-repository.md) | Vendor the engine into this repository | accepted | 2026-09-08 |
| [0009](0009-make-project-files-authoritative.md) | Make project files authoritative | accepted | 2026-09-23 |
| [0010](0010-ship-through-the-mac-app-store.md) | Ship through the Mac App Store | superseded by [0012](0012-distribute-a-notarized-dmg-with-web-billing.md) | 2026-09-24 |
| [0011](0011-consume-gpui-and-shared-ui-from-fanta-ui.md) | Consume GPUI and the shared UI from fanta-ui | accepted | 2026-09-27 |
| [0012](0012-distribute-a-notarized-dmg-with-web-billing.md) | Distribute a notarized DMG with web billing | accepted | 2026-09-30 |
| [0013](0013-gate-releases-on-named-verification-suites.md) | Gate releases on named verification suites | accepted | 2026-10-05 |
| [0014](0014-persist-ids-in-serde-form.md) | Persist ids in serde form and prefix them only at the edges | accepted | 2026-10-07 |
| [0015](0015-render-the-metal-canvas-with-skia-graphite.md) | Render the Metal canvas with Skia Graphite | accepted | 2026-10-14 |
| [0016](0016-keep-agent-instructions-in-the-repository.md) | Keep agent instructions in the repository | accepted | 2026-10-15 |
| [0017](0017-update-the-app-in-place-from-signed-releases.md) | Update the app in place from signed releases | accepted | 2026-10-19 |
| [0018](0018-build-the-portable-runtime-here-and-package-it-in-fanta-js.md) | Build the portable runtime here and package it in Fanta-JS | accepted | 2026-10-19 |
| [0019](0019-build-and-release-for-windows-and-linux.md) | Build and release for Windows and Linux | accepted | 2026-10-21 |
| [0020](0020-share-one-headless-engine-between-the-editor-and-the-cloud.md) | Share one headless engine between the editor and the cloud | accepted | 2026-10-21 |
| [0021](0021-run-cloud-work-in-a-stateless-engine-worker.md) | Run cloud work in a stateless engine worker behind a frozen contract | accepted | 2026-10-21 |
| [0022](0022-offer-a-fanta-native-design-api.md) | Offer a Fanta-native design API, built clean-room | accepted | 2026-10-24 |
| [0023](0023-enforce-layered-crate-boundaries.md) | Enforce layered crate boundaries | proposed | 2026-10-27 |
| [0024](0024-move-the-engine-into-its-own-repository.md) | Move the engine into its own repository | proposed | 2026-10-27 |
| [0025](0025-organize-the-repositories-under-one-github-organization.md) | Organize the repositories under one GitHub organization | proposed | 2026-10-27 |
| [0026](0026-choose-the-engine-and-sdk-licence.md) | Choose the engine and SDK licence | proposed | 2026-10-27 |
<!-- decisions:end -->
