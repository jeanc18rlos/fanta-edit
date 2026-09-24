# Fanta Mac App Store and Shipaton release

Status on 24 September 2026: the App Store build path and RevenueCat integration
are implemented locally. The Apple app record and purchase products are created.
The Paid Apps Agreement, bank account, and tax forms show Active in App Store
Connect. The EU trader submission shows In Review. Apple Developer Finance has
received a request to correct the submitted W-8BEN. RevenueCat now has the
Apple app with valid credentials, both products, a `pro` entitlement for the
subscription, and the default offering. Signing, production backend deployment,
purchase verification, and App Review remain.
Submitting to review on 24–25 September is the target; approval and publication
depend on Apple.

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
3. Create an Apple Distribution application certificate, a Mac Installer
   Distribution certificate, and an App Store provisioning profile. The local
   keychain currently has no valid signing identity, so import them or configure
   the GitHub Actions secrets listed in the release workflow.
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
   `/private/tmp/fanta-site-appstore-eEECAZ` (commit `cb8343c`).
   Review its legal text before publishing; the original site checkout has
   unrelated uncommitted work and must not be deployed wholesale.
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
  The final ad hoc build reports `1.0`/`1`, embeds the RevenueCat public SDK
  key, passes strict code signature verification, launches through macOS
  LaunchServices, opens a valid external design after relaunch, and shows a
  visible error for a loose `.fnx` outside a Fanta project. It is not a
  distribution-signed package.
  The distributable command is `./script/bundle-mac -s`. Release signing requires
  `MACOS_APP_STORE_PROVISIONING_PROFILE`,
  `MACOS_APP_STORE_SIGNING_IDENTITY`,
  `MACOS_APP_STORE_INSTALLER_IDENTITY`, and the public RevenueCat key. The
  workflow's manual `app-store` distribution builds and stores a signed `.pkg`;
  it does not upload it to Apple.
- Deploy the backend RevenueCat webhook only after verifying the current
  production database target, backing it up, applying pending Drizzle
  migrations `0020`–`0023` in order, and setting
  `REVENUECAT_APP_ID` and `REVENUECAT_WEBHOOK_AUTHORIZATION`. The backend
  handoff is in the isolated backend candidate at
  `/private/tmp/fanta-backend-revenuecat-20260924/docs/apple-revenuecat.md`.
  Do not deploy from the original dirty backend checkout: its local `0019`
  migration conflicts with upstream history. A read-only check of the local
  production connection found migrations `0000`–`0019` match the candidate,
  and a Vercel Production environment pull confirmed the database URL matches
  that inspected target. Back up the database before any migration.
  Point RevenueCat's
  production webhook to `https://api.fantaisa.net/webhooks/revenuecat`.
- Test a real Apple sandbox purchase for each product, subscription renewal,
  cancellation, refund, and restore. Check that the backend credits exactly
  once for each transaction and that generation spends those credits. Also
  verify sign-out/account switching and opening, saving, and reopening a local
  design in the sandboxed app.

## Submit and publish

1. Upload the signed `.pkg` using Apple's Transporter or App Store Connect
   upload workflow. Select the build and the two in-app purchases for review.
2. Complete the macOS listing: icon, screenshots, description, category,
   privacy policy (`https://www.fantaisa.net/privacy`), Terms of Use
   (`https://www.fantaisa.net/terms`), age rating, export compliance, support
   URL, reviewer account/instructions, and US availability. Validate every URL
   and screenshot before submission.
   The current draft has the description, keywords, subtitle, support and
   privacy URLs, Graphics & Design category, 13+ age rating, free price, and
   US-only availability. Screenshots, build, review sign-in, privacy details,
   and other review fields remain.
   A genuine current-build Mac screenshot (2880 × 1800 PNG) is saved at
   `docs/alpha/assets/fanta-mac-app-store-2880x1800.png`; the existing
   `crates/zed/resources/app-icon@2x.png` is 1024 × 1024. The screenshot
   is a sparse smoke-test design and should be replaced with a stronger real
   design if time allows. The older website beta screenshot shows features
   absent from this Mac App Store build and should not be submitted.
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
