# Design: Internal operations platform

Written after the founder accepts [spec.md](spec.md) (gate 1). It will cover:

- How the backend delivers events to Slack, Attio, Linear, PostHog and
  Discord: handlers on v2's transactional outbox (`v2/src/fanta_v2/outbox.py`),
  the alert catalog and its routing, retries and dead letters. This choice
  gets its own decision record.
- The intake paths into Linear: inbound email, the Discord help forum, the
  in-app report and GitHub issues, with de-duplication and replies.
- The morning digest and its queries, shared with the founder console.
- Consent, events and crash reports in the editor, and the binnacle changes
  they need.
- The console's integration-health and cost routes on v2.
- The handbook's layout and its secret scan.
