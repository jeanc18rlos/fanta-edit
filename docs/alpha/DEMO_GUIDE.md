# Fanta demos and the ten-minute exercise

Prepared **11 October 2026**. The three launch recordings will use the accepted
Public Alpha build. They are not yet available. Recording length is flexible: complete workflows and readability take priority, and the creator will edit the final cuts. Until review finishes, this page
does not embed old recordings or missing images as evidence of the release.

[Watch the reviewed demos](https://www.fantaisa.net/#demo) when available.
Documentation uses silent footage with captions. Website and deck players start
muted and enable music only after you choose to enable it.

| Demo | Result to watch for |
| --- | --- |
| Design in Fanta | Native imported layers change, Undo restores an edit, saved content reopens, and an exported PNG opens |
| Your design is source | A canvas edit produces an FNX/Git change, then a valid Code edit updates the canvas |
| An agent edits your design | A verified agent makes one bounded edit, followed by inspected source and Undo/Redo |

## The ten-minute exercise

Use the small public sample linked from the download page when available. Until
then, use **New Design** to create your own simple card or import a small `.fig`
you have permission to use. Keep a copy of the original. Do not download or
redistribute an entire third-party UI kit for this exercise.

| Time | Action | Observable result |
| --- | --- | --- |
| 0–2 min | Open the sample project, or create a design and save it into a new folder | The canvas and layer list show the same card |
| 2–4 min | Select one shape and change its fill or position. Use Undo, then Redo | The layer returns to its before and after states |
| 4–6 min | Save and open Code. Locate the selected layer's FNX and inspect Review Changes | The source and Git diff reflect the edit |
| 6–8 min | In Code, change a text layer's existing `content` literal and save valid source | The canvas shows your new text |
| 8–10 min | Save, close and reopen the project, then export a selected frame as PNG | The change survives reopening and the PNG opens outside Fanta |

Use a literal already present in your file, preserve node IDs and avoid changing
manifest or identity sidecars. An invalid source draft retains the last valid
canvas and reports an error. Resolve or discard the draft before visual edits.

## Agent extension

Follow the [agent quickstart](../fanta/agent-quickstart.md) and verify the correct
open project before asking for one change:

> On the open sample, change only the existing button label to “Explore Fanta”.
> Keep its geometry, styles, IDs and all other layers unchanged. Inspect the
> result and stop. Do not generate media or call paid services.

Check the changed layer and its FNX, then use Undo and Redo. The first exercise
does not require a paid generation job.

## Visual asset placement

The asset inventory lives in [screenshots/README](../screenshots/README.md).
Release integration adds orientation screenshots, three short silent GIFs for
individual actions, and linked poster frames for complete silent videos only
after their files and build provenance have been reviewed.
