//! Motion dual-read: prefer `motion/<slug>/` stubs; fall back to `doc/motion.json`.

use super::error::SessionError;
use crate::project::layout::{DOC_DIR, MOTION_DIR, MOTION_JSON, sorted_entries};
use fanta_doc::MotionLibrary;
use serde_json::Value;
use std::path::Path;

/// Result of dual-read (design Issue 26).
#[derive(Debug, Clone)]
pub struct MotionIndex {
    /// Singleton library when only `doc/motion.json` exists.
    pub library: MotionLibrary,
    /// Slug names under `motion/` when multi-artifact layout is present.
    pub artifact_slugs: Vec<String>,
    /// Which source was used.
    pub source: MotionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionSource {
    /// `motion/*/motion.json` directories exist (index-only in v1).
    ArtifactDirs,
    /// Legacy singleton `doc/motion.json`.
    DocSingleton,
    /// Neither present.
    Empty,
}

/// Read motion without migrating layout (no silent split on open).
pub fn read_motion_dual(project_root: &Path) -> Result<MotionIndex, SessionError> {
    let motion_root = project_root.join(MOTION_DIR);
    if motion_root.is_dir() {
        let mut slugs = Vec::new();
        for entry in sorted_entries(&motion_root)? {
            if entry.is_dir() {
                if let Some(name) = entry.file_name().and_then(|s| s.to_str()) {
                    if !name.starts_with('.') {
                        slugs.push(name.to_owned());
                    }
                }
            }
        }
        if !slugs.is_empty() {
            // v1: index only — library stays empty until full schemas land.
            return Ok(MotionIndex {
                library: MotionLibrary::new(),
                artifact_slugs: slugs,
                source: MotionSource::ArtifactDirs,
            });
        }
    }
    let singleton = project_root.join(DOC_DIR).join(MOTION_JSON);
    if singleton.is_file() {
        let value: Value = crate::project::layout::read_json_file(&singleton)?;
        let library = serde_json::from_value(value).unwrap_or_else(|_| MotionLibrary::new());
        return Ok(MotionIndex {
            library,
            artifact_slugs: Vec::new(),
            source: MotionSource::DocSingleton,
        });
    }
    Ok(MotionIndex {
        library: MotionLibrary::new(),
        artifact_slugs: Vec::new(),
        source: MotionSource::Empty,
    })
}
