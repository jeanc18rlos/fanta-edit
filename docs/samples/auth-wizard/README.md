# Auth wizard

A four-step sign-up prototype that exercises what the Fanta runtime plays:
component variants with Smart Animate hover, press and focus states, labels
driven by variables that each step sets on entry, conditional submits with an
inline error and a shake clip, push, dissolve and Smart Animate transitions, a
self-closing toast overlay, a ring pulse clip, and keyboard interactions.

| Key | Step 1 | Step 2 | Step 3 | Step 4 |
| --- | --- | --- | --- | --- |
| Enter | Continue | Verify | Finish | Start over |
| Backspace | clears the field | clears the code | clears the name | |
| A / X | valid / invalid email | | | |
| 1 2 3 4 | | type the code, one digit at a time | | |
| 0 | | a wrong code | | |
| N | | | types a name | |
| R | | resend (toast) | | start over |
| Esc | | back, or close the toast | back | |

Open the folder in Fanta to edit it. To play it in a browser:

```sh
cargo run --locked -p fanta-site --bin fanta-compile -- docs/samples/auth-wizard docs/samples/auth-wizard/example/program.fanta --offline
```

Then put a built `fanta-runtime` package (Fanta-JS `packages/runtime`, or
`node_modules/fanta-runtime`) at `example/runtime`, serve `example/` over HTTP
and open `index.html`. The page loads the bundle with `loadBundle`, forwards
keys to the prototype, and **Play walkthrough** drives every interaction with
pointer and keyboard events while recording per-frame cost. `layout.json`
holds the click targets that walkthrough uses.
