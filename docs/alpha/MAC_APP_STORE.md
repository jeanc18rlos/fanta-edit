# Fanta Mac App Store and Shipaton release

Status on 25 September 2026: the App Store build path and RevenueCat integration
are implemented locally. The Apple app record and purchase products are created.
The Paid Apps Agreement, bank account, and tax forms show Active in App Store
Connect. The EU trader submission shows In Review. The nine-type App Privacy
label is published. Apple Developer Finance has
received a request to correct the submitted W-8BEN. RevenueCat now has the
Apple app with valid credentials, both products, a `pro` entitlement for the
subscription, and the default offering. Apple issued the Mac App Distribution
and Mac Installer Distribution certificates and Fanta's Mac App Store
provisioning profile, which Xcode has synced to this Mac. Both issued
certificates are installed in the login Keychain. The distribution-signed
version 1.0 (build 2) app and installer package include the restored UI and
account-deletion entry point. Both pass Apple's signature checks.
Production authentication and backend deployment, purchase verification,
upload, and App Review remain.
The package is ready for upload; the production and review prerequisites below
still gate submission. Approval and publication depend on Apple.

## Account setup

1. In Apple Developer and App Store Connect, use an Account Holder/Admin account
   with an active Developer Program membership. The Account Holder must accept
   the current Paid Apps Agreement and complete banking and tax information.
   These show Active, but the submitted W-8BEN lists Spain as citizenship;
   request a replacement with Venezuela as citizenship while retaining Spain
   as the user's ordinary IRPF tax residence. Ask Apple to confirm how to align
   the beneficial-owner and signer names with the legal identity document.
2. The explicit macOS bundle ID `dev.fanta.Fanta` and App Store Connect record
   **Fanta — Design Editor** (Apple app ID `6815642103`, SKU `FANTA-MAC-2026`)
   are created. Use this bundle ID in the provisioning profile and RevenueCat.
3. Apple issued a **Mac App Distribution** certificate (`R3ZLJB5Z33`), a
   **Mac Installer Distribution** certificate (`H976XV3585`), and the
   **Fanta Mac App Store 2026** provisioning profile (`FAQK699VP4`) for
   `SP6J7Q6M3J.dev.fanta.Fanta`. Both certificates expire 24 September 2027.
   The verified CSR is at
   `/private/tmp/fanta-mas-signing-20260924/FantaMacApp.certSigningRequest`;
   its private key was created in the login Keychain. Chrome blocked the
   automated certificate downloads. After the Apple team was added to Xcode,
   **Download Manual Profiles** synced the profile to
   `/Users/jeanrojas/Library/Developer/Xcode/UserData/Provisioning Profiles/48232fbe-6b4a-4ceb-84b0-1b50e3fff671.provisionprofile`.
   Both `.cer` files are installed in the login Keychain. The app and installer
   signing identities were verified before the distribution build.
4. The **Fanta Pro** subscription group (`22410157`) contains a monthly
   auto-renewable subscription `dev.fanta.Fanta.pro.monthly` (`6815642543`),
   priced at Apple's US tier of **$44.99/month** for 3,000 credits. The
   consumable `dev.fanta.Fanta.credits.500` (`6815643967`) is priced at
   **$8.99** for 500 credits. Both have English (U.S.) customer-facing text and
   US-only availability. Add review screenshots and the subscription terms.
   Confirm the public pricing page matches the final prices before publication.
   The currently published Terms of Service describe Polar as the billing
   provider; update them to cover Apple in-app purchases before submission.
   A tested, isolated site candidate with the exact Apple prices, updated
   Terms, and Apple/RevenueCat privacy disclosures is at
   `/private/tmp/fanta-site-appstore-eEECAZ` (commit `f12707b`).
   The owner approved publishing it after a staged deployment. Preview and
   production builds of commit `f12707b` were verified, then promoted to
   `fantaisa.net` and `www.fantaisa.net` on 24 September. The live pricing,
   Terms, and Privacy pages return HTTP 200 with those changes. The original
   site checkout has unrelated uncommitted work and was not deployed.
5. In RevenueCat project `fantaisa` (`9eb97ebe`), connect the Apple app with
   its In-App Purchase key, configure the products and a current offering, and
   set its **public Apple SDK key** as `FANTA_REVENUECAT_PUBLIC_API_KEY` for the
   App Store build. Add an app-specific shared secret only if a StoreKit 1
   purchase path needs legacy receipt validation.
   Use the backend Fanta user UUID as RevenueCat App User ID. Keep a purchase
   with its original App User ID on account transfer and disable Family Sharing
   for this subscription.
   RevenueCat's project-wide restore behavior is **Keep with original App User
   ID** so one person's purchase cannot move to another Fanta account on the
   same Mac. A customer using a second Fanta account with the same Apple ID
   may need to return to the original account to restore or buy.
   The Apple In-App Purchase key named **Fanta RevenueCat** was uploaded to
   RevenueCat, and the app shows **Valid credentials**. The `Fanta Pro Monthly`
   and `500 Fanta Credits` products match the Apple product IDs. The
   subscription is attached to the `pro` entitlement; the consumable is not.
   The `default` offering contains `$rc_monthly` and `credits_500` packages.
   The app's public Apple SDK key is `appl_rpAPkYVLOHhdDSzQMkOxeYBbBew`.
   RevenueCat Billing shows that this account is already on the **Pro** plan,
   which supports the webhook needed for server-side credit fulfillment.
   Its Sandbox testing access is currently **Anybody**, the default for early
   QA. The production backend webhook rejects `SANDBOX` events and cannot
   grant live Fanta credits from a test purchase; the isolated Preview webhook
   accepts them only against a separate staging database. Restrict sandbox
   App User IDs after QA if the project starts handling unrelated test users.

## Build and backend

- The local ad hoc sandbox bundle is `./script/bundle-mac -s -d`; it uses embedded
  release assets and is written to `target/<host-target>/release/app-store-dev/Fanta.app`.
  The Mac App Store build must report marketing version `1.0` and a numeric
  build number so App Store Connect can attach it to the version 1.0 record.
  The current ad hoc build reports `1.0`/`1`, embeds the RevenueCat public SDK
  key, passes strict code signature verification, launches through macOS
  LaunchServices, opens a valid external design after relaunch, and shows a
  visible error for a loose `.fnx` outside a Fanta project. It is not a
  distribution-signed package.
  The distributable command is `./script/bundle-mac -s`. Release signing requires
  `MACOS_APP_STORE_PROVISIONING_PROFILE`, the public RevenueCat key, and both
  Apple distribution identities in Keychain (or explicit signing identity
  environment variables). The
  workflow's manual `app-store` distribution builds and stores a signed `.pkg`;
  it does not upload it to Apple. The signed 1.0/2 package is at
  `target/aarch64-apple-darwin/release/app-store/Fanta-aarch64.pkg`.
  `codesign --verify --deep --strict` and `pkgutil --check-signature` passed
  against the installed Apple certificates on 25 September. Its SHA-256 is
  `e8f7f9f9cc2de158fa84d57b87c408ec4ac11f76150f2a1fe6fdf374f4cf723b`.
  The bundled binary identifies source commit `cc6501acf6`.
  Upload is pending. Xcode's
  `altool` requires an App Store Connect API key or app-specific password;
  Apple's Transporter app is not installed on this Mac. Repeated attempts to
  install it from the official Mac App Store listing reached a blank Apple
  authorization sheet; the owner needs to complete that install or provide an
  approved `altool` authentication path.
- Deploy the backend RevenueCat webhook only after verifying the current
  production database target, applying pending Drizzle migration `0023`, and setting
  `REVENUECAT_APP_ID` and `REVENUECAT_WEBHOOK_AUTHORIZATION`. The backend
  handoff is in the isolated backend candidate at
  `/private/tmp/fanta-backend-revenuecat-20260924/docs/apple-revenuecat.md` and
  [draft PR #9](https://github.com/jeanc18rlos/fanta-backend/pull/9).
  Vercel's `fanta-db` opens Neon project `small-hill-03400665`; its production
  branch is `main` (`br-snowy-cake-ahzz1slc`) with four Fanta users. A named
  data-and-schema recovery child branch
  `fanta-recovery-before-revenuecat-2026-09-25`
  (`br-young-dust-ah9at528`) was created from `main` at
  2026-09-25 04:36 Madrid time with no expiration. No app is connected to it.
  Do not deploy from the original dirty backend checkout: it is 26 commits
  behind the available `origin/main`, and its local `0019`
  migration conflicts with upstream history. A read-only check of the
  production connection found migrations `0000`–`0022` match the candidate;
  `0023_revenuecat_billing` is pending. Claude is concurrently changing the
  original backend checkout, so reconcile that work before migration or
  deployment. The attempted local production export was blocked by automatic
  approval review because it could copy customer data. The recovery branch
  stays inside the same Neon project; migration `0023` has not run and the
  production database has not been changed.
  Point RevenueCat's
  production webhook to `https://api.fantaisa.net/webhooks/revenuecat`.
- Test a real Apple sandbox purchase for each product, subscription renewal,
  cancellation, refund, and restore. Check that the backend credits exactly
  once for each transaction and that generation spends those credits. Also
  verify sign-out/account switching and opening, saving, and reopening a local
  design in the sandboxed app.
- The 25 September UI update restores the Agent panel, Threads rail, project
  file tree, Git panel, bottom panel toggles, and a focused settings modal.
  FNX source is editable and validates before updating the canvas. The
  Mac App Store feature build and 18 focused FNX tests pass. The signed ad hoc
  app shows the restored panels and settings; live Fanta account, RevenueCat
  purchase, and backend tool flows still need signed-in sandbox QA. Settings →
  AI & Billing now offers account deletion with a confirmation and a warning
  that an Apple subscription must be canceled separately. The backend's
  `DELETE /v1/me` route needs production verification after the Clerk cutover;
  the isolated candidate now stops and keeps local data when Clerk deletion
  fails (commit `0dfcf13`). Deletion does not cancel Apple billing.
  The app changes are in
  [draft PR #10](https://github.com/jeanc18rlos/fanta-edit/pull/10).

## Submit and publish

1. Install Apple's Transporter Mac app, sign in with the existing App Store
   Connect Apple ID, add the signed `.pkg`, and click Deliver. Alternatively,
   use `xcrun altool` with an App Store Connect API key or Apple ID app-specific
   password. After Apple processes the upload, select build 1.0/2 and both
   in-app purchases for review.
2. Complete the macOS listing: icon, screenshots, description, category,
   privacy policy (`https://www.fantaisa.net/privacy`), Terms of Use
   (`https://www.fantaisa.net/terms`), age rating, export compliance, support
   URL, reviewer account/instructions, and US availability. Validate every URL
   and screenshot before submission.
   The current draft has the description, keywords, subtitle, support and
   privacy URLs, Graphics & Design category, 13+ age rating, free price, and
   US-only availability. A genuine Mac screenshot (2880 × 1800 PNG) has been
   uploaded to the version 1.0 record, but it predates the 25 September UI
   update. A stronger real app showcase capture replaced it in the draft on
   25 September. Recapture the final build with the restored workspace before
   submission. The App Privacy
   label was published on 25 September with nine data types, including
   Product Interaction, Other Usage Data, and Diagnostics → Performance Data.
   Performance Data is disclosed for app functionality, linked to identity,
   and not used for tracking. The private App Review contact (Jean Rojas, the
   supplied phone and email), testing notes, and
   explanations for user-selected file access and outbound networking are
   saved. The review account, build, and other review fields remain.
   The current uploaded capture is
   `/private/tmp/fanta-release-assets/fanta-mac-app-store-showcase-2880x1800.png`;
   the existing `crates/zed/resources/app-icon@2x.png` is 1024 × 1024.
   A populated original
   dashboard design is ready at
   `/private/tmp/fanta-mas-smoke-qa/MAS Smoke Design Copy`; its canvas render
   is `/private/tmp/fanta-mas-smoke-qa/fanta-showcase-canvas.png`.
   The older website beta screenshot shows features
   absent from this Mac App Store build and should not be submitted.
   The live Fanta sign-in currently redirects to a Clerk **Development**
   instance (`accounts.dev`). The Vercel `fanta-auth` Clerk integration has a
   Production instance with no users. Its domain was changed to `fantaisa.net`
   on 25 September; all five Clerk CNAMEs are present in Vercel DNS, Clerk
   verified frontend, account-portal, and email records, and both SSL
   certificates were issued. Do not move the backend, dashboard, and admin app
   to production Clerk keys until the user-ID mapping is applied. Development users
   cannot be moved automatically; map existing Clerk IDs to Fanta accounts,
   credits, and admin access before cutover. The isolated backend candidate
   has a guarded, dry-run-first mapping script and runbook in
   `docs/clerk-production-identity-cutover.md` (commit `d00a187`); no
   production mapping has run. Create the dedicated reviewer
   account only after the production sign-in is verified. Production Clerk has
   email/password sign-up enabled, so the reviewer needs a Fanta account, not
   an Apple ID. Device Trust currently asks for extra verification on a new
   device, which must be addressed for App Review. The current Google
   login also needs an Apple Guideline 4.8 review: add an equivalent private
   login such as Sign in with Apple, or remove Google after providing existing
   users another working sign-in path.
   Both initial in-app purchases need **Review Information screenshots**;
   App Store Connect rejects Add for Review until each screenshot is present.
   Product-specific review notes are saved. The 500-credit consumable's
   availability has been corrected to United States only and saved.
3. Submit to App Review. Once approved, publish the first public version and
   confirm it can be downloaded in the US. A TestFlight build does not satisfy
   the standard Shipaton eligibility rules.
4. Prepare the Devpost entry while App Review runs: a public YouTube or Vimeo
   demo with the essential footage under two minutes, a 1024 × 1024 icon, at
   least one 1179 × 2556 unframed screenshot, the RevenueCat project ID, a
   public store link, description, and a free trial or promo code for judges.
   Judge access is still unresolved: the backend currently grants zero credits
   for a subscription trial, so a free trial alone would not let judges test
   hosted generation. The simplest path is an Apple free offer code for the
   500-credit consumable, redeemed through the App Store after the app and
   purchase are approved. Apple requires both to be Ready for Distribution
   before real offer codes can be generated. Verify a code from a fresh account
   and include judge redemption instructions in Devpost.
   Submit by **30 September 2026, 11:45 PM PDT**.
   The existing Fanta Shipaton draft (project `1192066`) now has its project
   story, technology tags, website, RevenueCat project ID, and monetization
   and design answers saved. Devpost still shows 2 of 5 steps complete. The
   1024 × 1024 icon is `crates/zed/resources/app-icon@2x.png`; a real app
   portrait screenshot candidate is at
   `/private/tmp/fanta-release-assets/fanta-shipaton-showcase-1179x2556.png`.
   The icon and genuine Mac app screenshot were attached to the Devpost image
   gallery and remained after reloading the draft on 25 September. The
   portrait candidate is not attached because it has large blank or clipped
   areas; recapture from the final build. The public demo, store link, and
   judge access instructions are still needed before final submission.

References: [Apple app records](https://developer.apple.com/help/app-store-connect/create-an-app-record/add-a-new-app), [RevenueCat Apple credentials](https://www.revenuecat.com/docs/store-configuration/app-store/service-credentials-index), [Shipaton submission guide](https://www.revenuecat.com/blog/engineering/how-to-submit-your-app-for-shipaton).
The [official Shipaton rules](https://revenuecat-shipaton-2026.devpost.com/rules)
allow a project that existed earlier if it was not publicly released on an
eligible app store before the submission period; the first public Mac App Store
version must be live by the deadline.

## Listing and reviewer draft

**Name:** Fanta — Design Editor

**Description:** Create designs on a native macOS canvas with layers, text,
vectors, layout, and editable FNX source. Open Figma `.fig` files and export
designs as PNG, SVG, or PDF. Local editing is available without a subscription.
Optional Apple in-app purchases add credits for hosted AI image, video, and
vector generation.

**Reviewer notes:** The local design editor works before sign-in. To test Apple
purchases, sign in to the supplied Fanta review account, then choose **Fanta →
Credits & Billing**. The Pro monthly subscription grants 3,000 AI credits per
paid period; the 500-credit pack is consumable. Use **Restore purchases** in
the same menu to test restoration. Put review account credentials in App Store
Connect's private review fields, never in the public description.

**Shipaton video outline:** Show a design on the canvas, its editable FNX
source, one export, a hosted generation, and the native Apple purchase flow.

## Shipaton entry draft

**Project name:** Fanta — Design Editor

**Tagline:** Design on a native Mac canvas, then generate assets with credits.

**Short description:** Fanta is a macOS design editor with layers, text,
vectors, layout, editable FNX source, Figma `.fig` import, and PNG/SVG/PDF
export. Local editing is free. Optional Apple in-app purchases provide credits
for hosted image, video, and vector generation; RevenueCat connects the
purchase to the signed-in Fanta account.

**Suggested categories:** Grand Prize and RevenueCat Design Award, if the
final editor and demo meet their published criteria. Claim no sponsor category
without its required integration and evidence.

**Two-minute demo:** Open an existing local design; edit a shape and its
properties; show editable FNX source; export an image; sign in, show the
localized Apple credit-pack price, and complete a sandbox purchase; run one
hosted generation and show the credit balance. Record the actual release build
and publish the video on YouTube or Vimeo. Replace sandbox footage with a
real approved purchase flow only if it can be shown without spending money
or exposing account details.
