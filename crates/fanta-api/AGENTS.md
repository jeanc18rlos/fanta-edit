# fanta-api rules

The programmatic design API (`DesignSession`): node lifecycle, typed properties, events, text variables, template permissions and export over `fanta-headless`. Language bindings wrap it, so its calls and errors are a public surface.

- Change the document only through `DesignSession::edit` or `edit_with_design_op`. They open the pending undo step, roll a failed call back to its savepoint (`Doc::rollback_open_transaction`), and settle layout into the same step. They also refuse a Viewer's edits. A write through `doc_mut()` skips all of this.
- A call that changes a node checks its scope with `permit` before it builds operations, and every `PROPERTIES` entry names its scope (`None` only for what no template locks, such as the name). Nothing else catches a missing check: an Adopter could then change what the template locked.
- After undo or redo, catch the layout baseline up rather than settling it (`HeadlessDocument::undo` and `redo` do this). Settling there tries to amend a transaction that is now on the redo stack, and fails.
- `events::node_events` matches every `Operation` variant with no wildcard arm, so a new operation does not compile until it is classified. Keep it that way: a wildcard would silently drop that operation's node events.
- Ids cross the API as `NodeId`; bindings convert with `Display`/`FromStr` (`n_…`), never the serde form (see the root rules).
