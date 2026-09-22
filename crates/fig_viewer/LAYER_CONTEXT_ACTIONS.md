# Layers context actions

The sidebar renders `fanta_gpui::layers::LayersPanel`. Its public
`context_actions_for_kind` function supplies the menu definition used by both
Storybook and the editor. The editor adapter only applies document capabilities
(locks, actual strokes, available pages, and resolvable component definitions).
There is no second editor menu or generic "unavailable" action handler.

| Action | Editor behavior |
| --- | --- |
| Copy | Existing canvas clipboard, with assets and component references retained |
| Duplicate | Copy selected roots; remap component definitions and complete variant sets; select the copies |
| Delete | Delete selected roots and animation tracks; detach dependent instances before removing masters |
| Paste to replace | Replace each selected root at its position and stacking slot; select the replacements |
| Copy/Paste as | Copy SVG, PNG, or appearance properties; paste appearance without changing geometry |
| Move to page | Choose another page; retain world transforms |
| Bring to front / Send to back | Reorder selected roots while retaining their relative order |
| Convert to frame / section | Preserve children and world positions; persist explicit section identity |
| Group / Frame selection | Existing structural commands, including grouping component masters |
| Ungroup / Remove frame | Existing structural command that preserves child transforms |
| Rename | Shared panel inline editor and typed rename event |
| Flatten | Convert paths, text, image rectangles, containers, booleans, and detached instances into vector geometry |
| Outline stroke | Convert stroke geometry to filled paths; retain separate paints, alignment, dashes, and individual border widths |
| Use as mask / Remove mask | Undoable document mask operation |
| Set as thumbnail | Persist the selected layer; project saves render `.fant.preview.png` |
| Edit text | Enter canvas text editing with the text selected |
| Crop image | Custom percentage coordinates, aspect presets, and reset |
| Replace media | Native file chooser for images or videos; retain identity, geometry, and layer styling; validate before committing |
| Add auto layout | Add horizontal layout to a container or wrap a leaf in a layout frame |
| More layout options | Horizontal/vertical flow, wrapping, and removal when applicable |
| Create component | Register a container, or wrap a leaf and register the new frame |
| Go to main component | Navigate to the master and its page |
| Detach instance | Materialize the instance through existing component operations |
| Reset all overrides | Clear instance overrides, property values, and derived data; restore master dimensions |
| Show/Hide and Lock/Unlock | Shared dedicated flag intents |
| Flip horizontal / vertical | Reflect the selected roots around their combined world bounds |

All document mutations use undoable operations. Choice menus use shared Fanta
menu surfaces and rows, own wheel events, support dismissal, and reject stale
document targets. The shared menu restores focus before dispatching host actions
so editor text fields and pickers retain focus.

Imported `figma_type: SECTION` provenance and explicit editor section/frame
metadata are both understood. Other engine types keep the generic menu. The
legacy Figma-service enum variants remain upstream for source compatibility but
are never offered or dispatched as editor features.

Canvas clipboard and property payloads remain document-scoped: cross-document
paste is rejected until asset, component, and variable transfer is supported.
