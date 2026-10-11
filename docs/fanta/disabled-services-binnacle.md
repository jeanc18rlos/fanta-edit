# Fanta Edit Disabled Services Binnacle

This document records the hosted-service defaults that were disabled or
redirected during the first Fanta Edit baseline pass, why they changed, and what
you need to own before turning them back on.

The principle is simple: Fanta should not look like Fanta while silently relying
on Zed-owned infrastructure. Local editor features can stay available, but
hosted features should be enabled only when Fanta owns the endpoint, policy,
release process, and user-facing disclosure.

## Current baseline — 9 October 2026

| Service surface | Current Fanta default | Why it changed | Re-enable when |
| --- | --- | --- | --- |
| Telemetry metrics | `"metrics": false` | Fanta does not yet own analytics ingestion, retention, dashboards, or privacy language. | Fanta has an analytics endpoint, schema validation, retention policy, opt-out UX, and docs. |
| Diagnostics and crash upload | `"diagnostics": false` | Crash reports can contain sensitive metadata and need Fanta-owned handling. | Fanta has crash ingestion, symbolication, retention rules, alerting, and disclosure. |
| Auto-update | `"auto_update": false` | The updater expects trusted signed artifacts and release metadata owned by the product. | Fanta has signed builds, update assets, checksums/signatures, release notes, and rollback steps. |
| Hosted server URL | `"server_url": "https://api.fantaisa.net"` | Account, docs, release, collaboration, telemetry, and cloud routes must not target `zed.dev` by default. | `api.fantaisa.net` has compatible routes, or cloud-dependent features are gated. |
| Managed model provider | New threads default to `Fanta`; account-backed catalog and requests use Fanta endpoints. Explicitly configured providers remain available. | The old `zed.dev` default was removed; the earlier Anthropic default is historical. | Live model execution, pricing, quotas and recovery need their own acceptance; a populated catalog is insufficient. |
| Realtime cloud updates | `"cloud_updates_enabled": false` | The account API does not expose the inherited realtime update socket. | Fanta has a compatible realtime service and validated failure/recovery behavior. |

Stable defaults target `https://api.fantaisa.net`; development/preview overrides
use the separate `https://api-v2.fantaisa.net` account, provider and MCP endpoints.
An environment or user override can change routing. Sign-in, catalog, billing
and configured HTTP MCP are not disabled by the realtime-cloud setting. See the
[account workflow](capabilities.md#accounts-models-and-connected-tools) and
[current acceptance scope](../alpha/CAPABILITY_COVERAGE.md#bounded-checks-through-8-october).

## Historical baseline changes

- [assets/settings/default.json](../../assets/settings/default.json)
  - The first pass changed the agent model from `zed.dev` to `anthropic`;
    current defaults instead use the managed `Fanta` provider.
  - Default telemetry diagnostics and metrics changed to `false`.
  - Default auto-update changed to `false`.
  - Default `server_url` changed from `https://zed.dev` to `https://api.fantaisa.net`.
  - Default `zed.dev` language model provider entry was removed.
- [crates/auto_update/src/auto_update.rs](../../crates/auto_update/src/auto_update.rs)
  - Auto-update default documentation now describes Fanta's disabled baseline.
  - The default-setting test now expects auto-update to be disabled.
- [FANTA.md](../../FANTA.md)
  - Records the fork contract, service-safety baseline, release resources, and first feature substrate.

## Self-Hosting Binnacle

Use this section as the operating log for bringing services back. Each service
should move through four states:

1. Disabled.
2. Local or development-only.
3. Private beta.
4. Default-on.

Do not skip the policy and observability work. Otherwise the app will be branded
as Fanta but still behave like an unowned cloud client.

## 1. Domain And Service Routing

Minimum resources:

- Fanta-owned domain, for example `fanta.dev`.
- API host: `api.fantaisa.net`.
- TLS certificates.
- Environment split for local, staging, and production.
- Route map for account, docs, releases, telemetry, collaboration, and model gateway paths.

Implementation notes:

- Keep unavailable routes explicit. Return clear 404 or feature-disabled responses instead of accepting requests you do not process.
- Keep local development settings separate so dev builds do not accidentally call production services.
- Keep account, model and managed-MCP endpoints on the intended environment;
  configured routes do not establish that every service is available.

Verification:

- Fresh app launch should not call `zed.dev`.
- Fresh app launch should not call Fanta production services unless the corresponding feature is enabled.
- A bad or unavailable Fanta service should fail visibly and recoverably.

## 2. Telemetry Metrics

Minimum resources:

- HTTPS ingestion endpoint.
- Event schema validation.
- Queue and backpressure behavior.
- Dashboard or query path.
- Retention policy.
- User deletion/export path if account-linked data is stored.
- Settings UX and docs for opt-out.

Recommended first version:

- Keep metrics default-off.
- Add a local development endpoint that logs received events.
- Add tests or manual QA proving disabled metrics produce no network request.
- Only later add production ingestion.

Policy requirements:

- Document what is collected.
- Document why it is collected.
- Document how long it is retained.
- Document how users disable it.
- Avoid collecting source text, prompts, secrets, file contents, or path details unless there is explicit consent and a narrow purpose.

## 3. Diagnostics And Crash Reporting

Minimum resources:

- Crash endpoint or Sentry project.
- Symbol upload process.
- Release/version mapping.
- PII scrub rules.
- Alert routing.
- Retention and deletion process.

Implementation notes:

- Keep local crash dump generation separate from crash upload consent.
- macOS and Windows releases need signing plus symbol discipline, otherwise reports are hard to trust and hard to debug.
- Do not enable upload until the privacy policy describes diagnostics behavior.

Verification:

- Force a dev crash.
- Confirm the report is symbolicated.
- Confirm it is attributed to a Fanta release.
- Confirm it does not include source text or prompt content.
- Confirm the user can disable upload.

## 4. Auto-Update And Release Assets

Minimum resources:

- Signed app artifacts.
- Release manifest or compatible release asset endpoint.
- Release notes URL.
- Checksums or signatures.
- Rollback procedure.
- Private release channel for testing.

Platform notes:

- macOS requires Apple Developer signing and notarization before auto-update should be trusted.
- Windows should wait for a code-signing certificate and installer/update helper validation.
- Linux packaging may ignore app-level update settings when installed through a package manager.

Recommended rollout:

1. Start with manual GitHub Releases.
2. Produce signed private builds.
3. Add a private update channel.
4. Exercise update, rollback, and failed-download paths.
5. Enable auto-update by default only after the private channel is boring.

## 5. Model Gateway Or Bring-Your-Own-Key

The managed Fanta provider is implemented and is the current default. Users can
also explicitly configure their own providers, local models or supported
external agents. The earlier recommendation to defer all hosted brokerage is
historical; it must not be read as the current application's behavior.

Account/catalog/billing reads have bounded evidence. One historical internal
build33 Seedream image journey also passes generation, placement, Undo/Redo,
Save and reopening with a matching five-credit usage row. That does not prove
ordinary credit-ledger debit, a currency charge or exactly-once upstream
execution. Broader generation, charged-cost limits, managed-MCP execution and
recovery remain release gates. A visible price chip or local token limit does
not bound an agent loop, retries or auxiliary requests. The
[dated coverage record](../alpha/CAPABILITY_COVERAGE.md#current-acceptance-status)
keeps that sample separate from pending latest-build service acceptance.

Hosted gateway requirements still requiring operational verification:

- Account and auth model.
- Provider API key management.
- Per-user quotas or billing.
- Provider failover behavior.
- Request logging policy.
- Abuse controls.
- Subprocessor list.
- Retention and training guarantees.

Verification:

- New agent threads should never silently fall back to a Zed-hosted provider.
- A user without a configured provider should receive clear setup guidance.
- Hosted Fanta model requests should be traceable to Fanta-owned logs and policies.

## 6. Collaboration, Accounts, And Cloud State

Treat collaboration as its own product surface, not a minor setting.

Minimum resources:

- Auth provider.
- Account database.
- Realtime infrastructure.
- Access control model.
- Invite and membership UX.
- Abuse controls.
- Data retention policy.
- Incident response path.

Implementation notes:

- Keep unimplemented collaboration separate from account sign-in and billing;
  working account access does not establish shared realtime editing.
- Preserve local editing, git, terminal, and external agent workflows independently from hosted collaboration.
- Define keychain credential names before enabling sign-in so Fanta credentials cannot collide with Zed credentials.

## Operational Checklist Before Enabling Any Hosted Service

- Fanta owns the endpoint, credentials, logs, dashboards, and incident contact.
- The README or docs explain the service and whether it is optional.
- The privacy policy and terms match actual behavior.
- The setting defaults are intentional for fresh installs.
- A local/offline path still works when the service is unavailable.
- Tests or manual verification prove that disabled means no network call.
- Release notes explain any new default-on network behavior.

## Recommended Next Decisions

1. Choose whether project-local settings stay in `.zed/` for compatibility or move to `.fanta-edit/` for full identity separation.
2. Choose the CLI binary name: `fanta`, `fanta-edit`, or keep `zed` temporarily for upstream compatibility.
3. Complete production validation and policy coverage for the implemented Fanta
   gateway, while preserving explicitly configured provider alternatives.
4. Disable or archive Zed-owned GitHub Actions and Cloudflare workers before any public CI runs.
5. Create a first private signed macOS build before revisiting auto-update.
