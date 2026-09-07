# Known issues, Fanta 0.1.0-alpha.1

Written from the code, not from wishful thinking. Every entry here is something
a tester can hit on purpose.

## Canvas and tools

- Parts of the toolbar are not wired up. Seventeen tool faces and most toolbar
  commands are inert; they now say so when clicked instead of doing nothing
  silently. The tools that do work: select, hand, frame, rectangle, ellipse,
  line, polygon, star, text, pen, pencil, node edit.
- The Scale, Path Select and Text-on-Path tools are placeholders.
- Grid auto-layout is offered in the inspector but the engine ignores it.
- Several blend modes silently fall back to normal.
- Effects are limited to drop shadow, inner shadow, layer blur and background
  blur. Other effect kinds report that the engine does not support them.

## Editing and files

- Copy and paste works inside one document only. Cross-document paste is
  refused with a message; pasting an image or a file from another app onto the
  canvas does nothing at all.
- Export is in the inspector, not the toolbar, and needs the project to have
  been saved at least once.
- The Code tab is read-only by design. Edit `.fnx` in your own editor and the
  canvas follows the file.
- A design tab is restored on relaunch through its project folder. A `.fig` that
  never materialised a project is not restored; reopen it from File > Open
  Recent.
- Opening a very large community `.fig` (100 MB and up) is slow.

## Agent

- The agent needs a project open before it will chat. Opening a `.fig` or a
  Fanta project satisfies this.
- The managed Fanta provider requires signing in. Without it, set an Anthropic
  API key in the agent panel's settings; that is the alpha's default path.

## Platform

- Apple Silicon only. There is no Intel build.
- Windows and Linux are not supported in this alpha.

## Leftovers from the Zed fork

- Some command-palette entries belong to editor features this app does not
  expose. Most do nothing. "Check for Updates" in particular reports an error;
  alpha builds are updated by downloading a new DMG.
