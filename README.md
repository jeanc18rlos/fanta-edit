# Fanta

Fanta is a design app where the file and the agent share one model. It opens
Figma `.fig` files and Fanta projects on a real canvas, and the same document is
readable and writable as `.fnx` source, so an agent can change a design the same
way you can, and you can read the diff afterwards.

This is an **alpha**. It is rough in places, and the Known Issues list below is
honest rather than short.

## Requirements

- macOS 14 or later
- Apple Silicon (Intel is not supported yet)

## Install

1. Download the `.dmg` from the [latest release](RELEASES_URL_TODO).
2. Open it and drag **Fanta** to Applications.
3. Launch it.

If macOS says the app cannot be verified, the build you have is unsigned; either
grab the signed build or run:

```
xattr -dr com.apple.quarantine /Applications/Fanta.app
```

## Getting started

1. **File > Open...** a `.fig` file, or **File > New Design...** to start empty.
   Opening a `.fig` writes a Fanta project folder beside it; that folder, not the
   original `.fig`, is what your edits go to.
2. Give the agent a model. Either paste an Anthropic API key under the agent
   panel's settings, or sign in to use the managed Fanta provider.
3. Ask it for something: *"add a 200x100 blue rectangle called Hero to this
   page"*. It edits the canvas through the same operations your cursor does, so
   undo works on its changes too.
4. Open the **Code** tab to read the `.fnx` source of what you are looking at.
   It is read-only in the app on purpose: edit it in your own editor and the
   canvas follows the file.

## Known issues in this alpha

- The canvas toolbar shows some tools and commands that are not wired up yet.
  They tell you so when clicked rather than failing silently.
- Grid auto-layout, several blend modes, and most effect kinds beyond shadows
  and blurs are not supported by the engine yet.
- Copy and paste works within one document only. Pasting an image or a file from
  another app onto the canvas does nothing.
- Export lives in the inspector, not the toolbar, and needs the project to have
  been saved once.
- Very large community `.fig` files (100 MB and up) are slow to open.
- The Code tab is read-only by design.
- Design tabs are restored on relaunch via the project folder; a `.fig` that
  never materialised a project is not restored.

## Feedback

Please file issues at [ISSUES_URL_TODO]. Logs live in `~/Library/Logs/Fanta/`
and attaching `Fanta.log` makes almost every report easier to act on.

## License

GPL-3.0-or-later. Fanta is a fork of Zed; see `NOTICE.md` and `LICENSE-GPL`.
Source for every binary we ship is this repository, including the vendored
engine crates under `crates/fanta-*`.
