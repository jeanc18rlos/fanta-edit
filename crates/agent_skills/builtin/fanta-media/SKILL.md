---
name: fanta-media
description: >-
  Generate and use AI media (images, edits, upscales, animations) via the
  Fanta backend MCP server at api.fantaisa.net. Use when the user wants to
  generate an image, edit or upscale an existing one, animate a still, or
  bring generated media into a design. Covers the generate → poll → deliver
  loop and current canvas-placement limitations.
---

# Generating media with the Fanta backend

Media generation runs on the `fanta` MCP server (`https://api.fantaisa.net/mcp`,
authenticated with the user's `fnt_live_…` key). If its tools
(`generate_image`, `edit_image`, `upscale_image`, `vectorize`,
`animate_image`, `plan_compose`, `get_generation`, `list_models`,
`search_assets`, `upload_asset`, `get_credits`) are not available, the server
is not configured — ask the user to add the `fanta` context server rather
than guessing at endpoints.

## The loop

1. **Pick a model deliberately** — `list_models` shows what's deployed and
   the credit cost. Don't assume a model name.
2. **Generate** — `generate_image` (text → image), `edit_image`
   (image + instruction), `upscale_image`, `animate_image` (still → video).
   Long jobs return a `generation_id` when they exceed the request budget.
3. **Poll** — `get_generation {generation_id}` until `status` is `succeeded`
   or `failed`. Space polls out (a few seconds apart); report failures
   honestly instead of retrying blindly.
4. **Deliver** — a finished generation has an asset URL. Share it with the
   user, and place it in the design if they want it there (see below).

Generations cost credits (`get_credits`); confirm with the user before
batch-generating many variants.

## Placing results in the open design — current limitations

The native canvas ops (`design_edit`) can NOT create image nodes yet — image
placement is a planned op. Until it lands, be explicit about the options
instead of pretending:

- **Project-asset route (works today):** download the generated file into the
  open Fanta project's `assets/` directory (the project root is reported by
  `design_state`), then reference it from the page's FNX source
  (`pages/<page-id>/page.fnx`) — the canvas hot-reloads on save. Only do this
  when an existing image node in the source shows you the exact attribute
  shape to copy; otherwise prefer the next option.
- **Hand off (always works):** give the user the asset URL and tell them to
  drop the image onto the canvas; then continue styling around it with
  `design_edit`.

Never claim an image was placed on the canvas unless a `design_screenshot`
actually shows it.

## Video

`animate_image` targets the backend's video family. If it reports that video
generation is unavailable, the `videogen` deployment is not live yet — say
so and stop; do not substitute a different output format silently.
