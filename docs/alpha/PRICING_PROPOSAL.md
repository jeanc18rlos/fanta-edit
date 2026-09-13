# Fanta launch pricing proposal

Status: proposed, not approved or configured. No Polar product, price, customer,
credit balance, or production setting has been changed.

## Recommendation

Launch in USD with two paid plans and three one-time credit packs. Keep annual
checkout disabled until the existing annual-allocation implementation has passed
provider sandbox and authenticated scheduler validation.

| Plan | Monthly price | Credit grant | Seats | Proposed positioning |
| --- | ---: | ---: | ---: | --- |
| Free | $0 | 500 once, at signup | 1 | Core editor and any enabled metered operation while credits remain |
| Pro | $45 | 3,000 per verified paid month | 1 | Solo paid plan; API/priority labels remain proposals until enforcement is completed |
| Team | $89 | 6,000 per verified paid month | 5 | Paid organization plan with a five-seat admission cap |

The backend does not currently reserve creative tools for Pro, and its seeded
`priority_queue` value does not affect scheduling. Free currently advertises
API access. Existing named keys can also survive a downgrade. Product copy must
not promise those distinctions until request and queue enforcement is added and
tested. Team limits new member admission, but downgrade/cancellation does not
yet suspend members above the new plan's limit.

The matching future annual planning prices are $499 for Pro and $999 for Team,
with the same monthly allowances granted in 12 installments. Under the downside
assumptions below they retain only about 20.9% contribution, rather than the
monthly plans' roughly 24%. These are not launch products.

| Pack | One-time price | Credits | Treatment |
| --- | ---: | ---: | --- |
| Small | $9 | 500 | Non-expiring, organization balance |
| Medium | $24 | 1,500 | Non-expiring, organization balance |
| Large | $79 | 5,000 | Non-expiring, organization balance |

## Catalog reconciliation

The live Polar catalog inspected during release preparation does not represent
the backend entitlements and must not be connected by display name.

| Current source | Current offer | Decision in this proposal |
| --- | --- | --- |
| Polar | Starter $20/month, Pro $80/month, Max $200/month | Leave unmapped and remove from new-customer checkout after checking for existing subscribers. Do not repurpose the legacy product IDs. |
| Polar | 100 credits/$5, 500/$20, 2,000/$60 | Leave unmapped. Their quantities do not match the backend pack records. |
| Backend seed | Pro $24/month with 3,000 credits | Replace display price with $45 only after approval and provider setup. |
| Backend seed | Team $40/month with 6,000 credits | Replace display price with $89 only after approval and provider setup. |
| Backend seed | 500/$5, 1,500/$12, 5,000/$35 packs | Replace with $9/$24/$79 only after approval and provider setup. |
| Backend behavior | Free exposes 500 as `monthly_credits`, but provisioning grants it once | Change the label/API contract or implement replenishment; do not advertise 500 per month today. |
| Desktop dashboard | Hardcodes `€` while this proposal is USD | Add an explicit currency contract and render it consistently before checkout. |

The legacy Starter product also grants an `AI Image Editor Access` benefit and
has no credit metadata. That is not equivalent to any Fanta plan.

## Exact entitlement mapping

Create new products after approval and bind their immutable Polar product IDs as
follows. Until those IDs exist, every listed backend product field must remain
null and checkout must remain unavailable.

| New Polar product | Backend binding | Grant and access |
| --- | --- | --- |
| Fanta Pro Monthly, recurring monthly, USD $45 | `plans.id = 'pro'`; store the exact ID in `plans.polar_product_id_monthly` via `POLAR_PRODUCT_PRO_MONTHLY` | 3,000 credits for each verified paid term; one seat. No distinct API/priority promise until enforcement passes. |
| Fanta Team Monthly, recurring monthly, USD $89 | `plans.id = 'team'`; store the exact ID in `plans.polar_product_id_monthly` via `POLAR_PRODUCT_TEAM_MONTHLY` | 6,000 credits for each verified paid term; admission capped at five seats. No distinct queue promise until enforcement passes. |
| Fanta Credits 500, one-time, USD $9 | `POLAR_CREDIT_PRODUCT_SMALL`; server creates `credit_purchases.id` and sends `type=credit_purchase`, `purchaseId`, `orgId`, `userId`, and display-only `credits` metadata | Exactly one idempotent 500-credit ledger grant after verified payment |
| Fanta Credits 1,500, one-time, USD $24 | `POLAR_CREDIT_PRODUCT_MEDIUM`; same server-owned purchase identity contract | Exactly one idempotent 1,500-credit ledger grant after verified payment |
| Fanta Credits 5,000, one-time, USD $79 | `POLAR_CREDIT_PRODUCT_LARGE`; same server-owned purchase identity contract | Exactly one idempotent 5,000-credit ledger grant after verified payment |

Product identity starts with the Polar product ID, never the name, benefit, or
webhook metadata alone. Product ID is not sufficient payment proof: record and
verify the fixed price configuration, USD amount, flat-rate type, interval and
count, tax behavior, trial/discount state, checkout/order identity, and paid
evidence. The current webhook prefers metadata `planId` over the product lookup,
and pack fulfillment does not validate product, checkout, amount, currency, or
stored sticker amount. Fix those paths before activation. Record the approved
environment-specific IDs and configuration in this table and the deployment
record. Do not run the catalog seed against production as a substitute for
targeted updates: missing product environment variables can clear stored IDs.

## Customer-facing operation costs

These selected launch values follow the current backend seed and its billing
rule. They remain capability data returned by the backend; the desktop must not
hardcode them.

| Operation | Advertised alias | Credits per output |
| --- | --- | ---: |
| Standard image | `fanta-image-1` | 1 |
| Image edit / inpaint / outpaint | `fanta-image-edit-1` | 1 |
| Fast image | `fanta-image-fast-1` | 1 |
| Composition | `fanta-compose-1` | 14 |
| Prompt-to-vector | `fanta-svg-1` | 1 |
| Image-to-vector | `fanta-vectorize-1` | 1 |
| Background selection/removal | `fanta-segment-1` | 1 |
| Fast video | `fanta-video-1` | 4 |
| HD text/image-to-video | `fanta-video-hd-1` / `fanta-animate-1` | 126 |
| Video edit / subject swap | `fanta-video-edit-1` / `fanta-subject-swap-1` | 133 |
| Frame interpolation | `fanta-interpolate-1` | 1 |
| Video enhancement | `fanta-video-enhance-1` | 21 |

For chat, charge measured usage. At the current default 1.4 multiplier, Sonnet
is approximately 420 credits per million input tokens, 2,100 per million output
tokens, 42 per million cache-read tokens, and 525 per million cache-write tokens
before request-level rounding. The current models response omits the two cache
rates. The app should show an estimate and the final measured debit rather than
promise a flat prompt price.

Examples for a fully used monthly allowance:

- Pro covers 23 HD video outputs at 126 credits each, with 102 credits left.
- Team covers about 45 video edits at 133 credits each, with 15 credits left.
- A 14-credit composition plus five ordinary image/edit operations costs 19
  credits.

The seed also contains aliases outside this release's agreed scope, or not yet
cleared for release. They must be inactive/hidden unless deliberately added to
the capability inventory: Track and Cutout cost 1 credit; three Voice aliases
cost 1; Music costs 4; 3D costs 2 and 3D Fast 1; Upscale 1 and Enhance 2; Parts
3; Remaster 6; Workflow 2; Stems, Captions and Audio Clean 1; Rig 2. The seeded
Composer language alias costs 196 input and 616 output credits per million
tokens. Some 3D aliases retain a separate commercial-license blocker.

## Conservative economics

One Fanta credit has a nominal metering value of $0.01. It is not a purchase
price: every proposed plan and pack sells credits above one cent each. At the
current 1.4 multiplier, $0.00714 per consumed credit is a catalog-cost scenario,
not a guaranteed realized GPU cost. Media charges use seeded per-output prices
while runtime records actual GPU cost separately, so overruns can make realized
cost higher. The proposal assumes all included credits are consumed, even
though actual breakage may be material.

The downside case also assumes:

- a 25% tax-inclusive sale, so tax is extracted from the displayed price;
- Polar's current Starter fee of 5% + $0.50 and the additional 1.5% for an
  international card;
- a further 1% allowance for payout/currency conversion, which may not cover
  Polar's $2 active payout-month charge, 0.25% + $0.25 payout fee, and the full
  0.25%–1% cross-border conversion range; and
- no benefit from unused credits.

Under those assumptions, Pro and Team retain 23.77% and 23.78% of displayed
monthly price after tax, payment fees, payout allowance, and fully consumed
catalog cost. The proposed Small, Medium, and Large packs retain 27.26%, 25.77%,
and 26.66%. These are order-level contribution margins, not profit. They exclude
the signup grant, storage, bandwidth, idle capacity beyond the pricing snapshot,
support, refunds, disputes, and general infrastructure. Fully consuming the
500-credit signup grant adds about $3.57 of assumed cost per new customer; if
charged entirely to the first order, Pro and Team fall to roughly 15.8% and
19.8%, and the Small pack becomes negative. Polar currently lists a $15 dispute
fee. Current analytics do not ingest provider net, tax, fees, discounts, or
refunds, so contribution-by-cohort/model monitoring must be implemented before
discounts.

Polar's default location-based tax behavior is appropriate: it presents tax as
exclusive in the United States, Canada, and India and inclusive in most other
locations. Polar acts as merchant of record for sales-tax/VAT calculation and
remittance, but Fanta remains responsible for its own income and revenue taxes.
See [Polar pricing](https://polar.sh/resources/pricing),
[tax-inclusive pricing](https://polar.sh/docs/features/tax-inclusive-pricing),
and [merchant-of-record treatment](https://polar.sh/docs/merchant-of-record/introduction).

## Existing customers, cancellation, refunds, and rollover

- Do not charge, migrate, or reduce access for the existing owner account during
  catalog setup. Preserve its balance and current internal plan until a
  deliberate migration is approved.
- Inventory subscribers on every legacy Polar product ID before hiding it from
  checkout. Archiving a product does not stop its existing subscriptions from
  renewing; choose and communicate whether each real subscriber remains
  grandfathered or is explicitly scheduled to cancel. Do not infer an
  entitlement from a similarly named product.
- Scheduled cancellation should preserve paid access through the verified paid
  term, then move the organization to Free. Today that downgrade depends on a
  received `subscription.revoked` event: a missed revocation is not repaired by
  the current sync path. Add reconciliation before relying on the policy.
- Decide and document payment-recovery grace. The current backend treats
  `past_due` as an immediate downgrade even if Polar is configured to retry.
- Subscription and pack credits roll over and do not expire in this proposal.
- The current backend does not ingest refunds, link pack ledger grants to their
  order, track credit-lot spend, or provide a billing-hold gate. Until those
  exist, record each refund for manual review and make no automatic credit
  debit. A subscription refund requires a separate explicit cancel/revoke
  decision; refunding an order does not itself cancel the subscription.
- Keep proration, partial-refund credit adjustments, mid-term plan changes, and
  annual checkout disabled until their provider event ordering and ledger
  effects are specified and tested.

Provider behavior references: [refunds](https://polar.sh/docs/features/refunds),
[subscription management](https://polar.sh/docs/features/subscriptions/manage),
[Customer Portal settings](https://polar.sh/docs/features/customer-portal/settings),
and [webhook events](https://polar.sh/docs/integrate/webhooks/events).

## Approval and activation gates

The owner must approve the prices, allowances, USD launch currency, rollover,
payment-recovery, legacy-customer, and refund policies before product creation.
Approval does not by itself authorize production activation. Before any
checkout is enabled:

1. Reconcile the USD desktop UI, public currency contract, yearly amounts, pack
   constants, Free signup-grant wording, and any API/priority/tool claims.
2. Add an independent default-off `BILLING_CHECKOUT_ENABLED` gate. Enforce
   `plans.is_active`, block a second active subscription, require owner/admin
   for every pack route, and prevent existing named API keys from bypassing a
   downgraded plan policy.
3. Bind subscription entitlement to the product mapping rather than trusted
   `planId` metadata. For packs, verify the server-created purchase, product,
   checkout, order, fixed USD amount, currency, and paid evidence before an
   idempotent grant.
4. Create the five products in Polar sandbox as USD-only, fixed flat-rate
   prices: monthly interval count 1 for plans, one-time for packs, no trial,
   no discount codes, no metered or seat pricing. Disable portal plan changes.
   Record environment, Polar organization, product ID, price/configuration,
   amount, type, interval, tax behavior, trial/discount state, backend binding,
   and verification time.
5. Add refund ingestion, purchase/order-linked ledger provenance, an explicit
   manual-review/hold mechanism, missed-revocation reconciliation, and the
   chosen `past_due` grace behavior.
6. Apply migrations 0020 and 0021 to a production-shaped PostgreSQL copy and
   rehearse the application rollback without reseeding balances.
7. Deploy a compatible backend/worker candidate, then verify checkout, portal,
   signed `order.created` and `order.paid`, reordered and duplicate delivery,
   renewal, cancellation, revocation, refunds, and concurrent grants/debits in
   sandbox—including zero/discounted totals being rejected.
8. Repeat configuration with new production IDs, update only the intended plan
   and product fields, independently read them back, then enable checkout in a
   separate change.
9. Keep annual product IDs null until annual provider behavior and authenticated
   `/internal/tick` replenishment have passed. Do not expose stale annual prices
   while checkout is disabled.

No production payment or billable AI request is part of this proposal.
