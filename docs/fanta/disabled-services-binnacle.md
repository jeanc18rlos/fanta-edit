# Fanta Edit Disabled Services Binnacle

This document records the hosted-service defaults that were disabled or
redirected during the first Fanta Edit baseline pass, why they changed, and what
you need to own before turning them back on.

The principle is simple: Fanta should not look like Fanta while silently relying
on Zed-owned infrastructure. Local editor features can stay available, but
hosted features should be enabled only when Fanta owns the endpoint, policy,
release process, and user-facing disclosure.

## Current Baseline

| Service surface | Current Fanta default | Why it changed | Re-enable when |
| --- | --- | --- | --- |
| Telemetry metrics | `"metrics": false` | Fanta does not yet own analytics ingestion, retention, dashboards, or privacy language. | Fanta has an analytics endpoint, schema validation, retention policy, opt-out UX, and docs. |
| Diagnostics and crash upload | `"diagnostics": false` | Crash reports can contain sensitive metadata and need Fanta-owned handling. | Fanta has crash ingestion, symbolication, retention rules, alerting, and disclosure. |
| Auto-update | `"auto_update": false` | The updater expects trusted signed artifacts and release metadata owned by the product. | Fanta has signed builds, update assets, checksums/signatures, release notes, and rollback steps. |
| Hosted server URL | `"server_url": "https://fanta.dev"` | Account, docs, release, collaboration, telemetry, and cloud routes must not target `zed.dev` by default. | `fanta.dev` has compatible routes, or cloud-dependent features are gated. |
| Zed-hosted model provider | Default model uses `anthropic`; the default `zed.dev` provider entry was removed. | New agent threads should not assume access to Zed-hosted model brokerage. | Fanta owns a model gateway, or users explicitly configure their own provider. |

## Code Touchpoints Changed

- [assets/settings/default.json](/Users/jeanrojas/fanta-edit/assets/settings/default.json)
  - Default agent model provider changed from `zed.dev` to `anthropic`.
  - Default telemetry diagnostics and metrics changed to `false`.
  - Default auto-update changed to `false`.
  - Default `server_url` changed from `https://zed.dev` to `https://fanta.dev`.
  - Default `zed.dev` language model provider entry was removed.
- [crates/auto_update/src/auto_update.rs](/Users/jeanrojas/fanta-edit/crates/auto_update/src/auto_update.rs)
  - Auto-update default documentation now describes Fanta's disabled baseline.
  - The default-setting test now expects auto-update to be disabled.
- [FANTA.md](/Users/jeanrojas/fanta-edit/FANTA.md)
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
- API host decision: `api.fanta.dev` or `fanta.dev/api`.
- TLS certificates.
- Environment split for local, staging, and production.
- Route map for account, docs, releases, telemetry, collaboration, and model gateway paths.

Implementation notes:

- Keep unavailable routes explicit. Return clear 404 or feature-disabled responses instead of accepting requests you do not process.
- Keep local development settings separate so dev builds do not accidentally call production services.
- Decide whether `server_url` is a true public cloud root or only a placeholder until hosted services exist.

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

Lowest-risk first path:

- Keep bring-your-own-key providers available.
- Keep local providers such as Ollama available.
- Keep external agents available.
- Avoid Fanta-hosted model brokerage until billing, quotas, auth, and policy are real.

Hosted gateway requirements:

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

- If hosted collaboration is not part of the first Fanta baseline, gate sign-in and collaboration entry points.
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
3. Decide whether Fanta will offer hosted models or start with bring-your-own-key plus external agents.
4. Disable or archive Zed-owned GitHub Actions and Cloudflare workers before any public CI runs.
5. Create a first private signed macOS build before revisiting auto-update.
