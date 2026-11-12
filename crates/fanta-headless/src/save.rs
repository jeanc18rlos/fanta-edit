//! Saving an edited document back to its project directory the way the
//! editor does.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context as _, Result};
use fanta_doc::{AssetId, Doc};
use fanta_format::{WebRenderSource, WorkspaceSession, WriteReport};

/// Write `doc` over the project at `root` as web storage (spec 0007 R1). A
/// legacy FNX project converts on its first save: the web writer prunes the
/// FNX sources it replaces (R9), so the session's disk snapshot is
/// reconciled afterwards. `warnings` are the open-time loader reports that
/// belong in the written `report.json` (R7).
pub fn save_project(
    root: &Path,
    session: &mut WorkspaceSession,
    doc: &Doc,
    raw_assets: &BTreeMap<AssetId, Vec<u8>>,
    render: Option<&dyn WebRenderSource>,
    warnings: &[String],
) -> Result<WriteReport> {
    let (report, _) = fanta_format::write_web_project_tree(root, doc, raw_assets, render, warnings)
        .with_context(|| format!("writing Fanta project at {}", root.display()))?;
    session.reconcile_disk_snapshot()?;
    Ok(report)
}
