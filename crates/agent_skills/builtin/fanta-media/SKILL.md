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
authenticated automatically with the user's signed-in Fanta account). If its tools
(`generate_image`, `edit_image`, `upscale_image`, `vectorize`,
`animate_image`, `plan_compose`, `get_generation`, `list_models`,
`search_assets`, `upload_asset`, `get_credits`) are not available, the server
is disconnected — ask the user to sign in to Fanta and check that the `fanta`
context server is enabled rather than guessing at endpoints.

## The loop

1. **Pick a model deliberately** — `list_models` shows what's deployed and
   the credit cost. Don't assume a model name.
2. **Generate** — `generate_image` (text → image), `edit_image`
   (image + instruction), `upscale_image`, `animate_image` (still → video).
   Long jobs return a `generation_id` when they exceed the request budget.
3. **Poll** — `get_generation {generation_id}` until `status` is `succeeded`
   or `failed`. Space polls out (a few seconds apart); report failures
   honestly instead of retrying blindly.
4. **Place** — a finished generation has an asset URL. Put it on the open
   canvas with the native `place_generation` tool (see below), or just share
   the URL when no design is open.

Generations cost credits (`get_credits`); confirm with the user before
batch-generating many variants.

## Placing results in the open design

Use the native **`place_generation`** tool — it downloads the URL, ingests
the bytes as a project asset (persisted to `assets/images/` on save), creates
the image layer, and records provenance in the node's metadata in one step:

```json
{
  "url": "<asset URL from get_generation>",
  "x": 120, "y": 80,
  "width": 480,                  // omit both to keep natural pixel size
  "parent": "<frame id>",        // omit for the active page root
  "name": "hero-sunset",
  "prompt": "<the prompt used>", // provenance — pass what you know
  "model": "<model id>",
  "generation_id": "<id>"
}
```

- Coordinates are world-space; get frame ids and geometry from
  `design_state`. Generated images are usually large — set `width` (or
  `height`) to the size the layout needs instead of natural pixels.
- Resize/reposition afterwards with `design_edit` `set_props`; the layer
  behaves like any bitmap node and the placement is one undo step.
- If you already have raw bytes (not a URL), `design_edit`'s `create_image`
  op takes base64 directly.
- A `.fig` file that was never saved as a Fanta project still places the
  image in memory; remind the user to save so the asset lands on disk.

Never claim an image was placed on the canvas unless a `design_screenshot`
actually shows it.

## Video

`animate_image` targets the backend's video family. If it reports that video
generation is unavailable, the `videogen` deployment is not live yet — say
so and stop; do not substitute a different output format silently.
