# fanta-api rules

The programmatic design API (`DesignSession`): node lifecycle, typed properties, events and export over `fanta-headless`. Language bindings wrap it, so its calls and errors are a public surface.

- Change the document only through `DesignSession::edit` or `edit_with_design_op`. They open the pending undo step, roll a failed call back to its savepoint (`Doc::rollback_open_transaction`), and settle layout into the same step. A write through `doc_mut()` skips all three.
- After undo or redo, catch the layout baseline up rather than settling it (`HeadlessDocument::undo` and `redo` do this). Settling there tries to amend a transaction that is now on the redo stack, and fails.
- Ids cross the API as `NodeId`; bindings convert with `Display`/`FromStr` (`n_…`), never the serde form (see the root rules).
