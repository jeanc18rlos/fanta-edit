# Fanta 0.1.0-alpha.1

First alpha. Fanta opens Figma `.fig` files and Fanta projects on a canvas, and
gives an agent the same editing operations you have, over a design source you
can read.

## What works

- Open a `.fig` file or a Fanta project folder; the project materialises beside
  the `.fig` and becomes the editable source of truth.
- Canvas editing: shapes, frames, text, selection, transforms, auto-layout,
  gradients, shadows and blurs, components and instances, variables and modes.
- Prototype flows and presentation, motion clips and a timeline, and pinned
  comments.
- An agent that reads and edits the open design through typed operations, takes
  screenshots of the canvas to check its own work, and places generated images.
  Its edits are ordinary undo steps.
- A read-only view of the `.fnx` and JSON source behind whatever is on screen.
  Edit those files in your own editor and the canvas reloads.
- PNG, JPEG, SVG and PDF export.

## Getting a model

The default is your own Anthropic API key, set in the agent panel's settings.
Signing in additionally enables the managed Fanta provider.

## Known issues

See KNOWN_ISSUES.md. The short version: parts of the toolbar are not wired up
yet, cross-document paste and external image paste do not work, and this build
is Apple Silicon only.
