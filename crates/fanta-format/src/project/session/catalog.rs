//! Component catalog with side master Scenes (N18).

use super::error::SessionError;
use fanta_doc::{ComponentDef, ComponentId, ComponentLibrary, Scene};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Workspace-level component index + optional loaded master scenes.
#[derive(Debug, Default)]
pub struct ComponentCatalog {
    pub defs: ComponentLibrary,
    /// Side scenes holding master subtrees (not page scenes).
    pub master_scenes: BTreeMap<ComponentId, Scene>,
    /// Design dir per component for lazy load.
    pub paths: BTreeMap<ComponentId, PathBuf>,
}

/// Borrow of a loaded master for `expand_instance`.
pub struct MasterRef<'a> {
    pub def: &'a ComponentDef,
    pub scene: &'a Scene,
}

impl ComponentCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_master_loaded(
        &mut self,
        id: ComponentId,
        load: impl FnOnce() -> Result<Scene, SessionError>,
    ) -> Result<MasterRef<'_>, SessionError> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.master_scenes.entry(id) {
            entry.insert(load()?);
        }
        let def = self
            .defs
            .defs
            .get(&id)
            .ok_or(SessionError::MasterNotInScope)?;
        let scene = self
            .master_scenes
            .get(&id)
            .ok_or(SessionError::MasterNotInScope)?;
        Ok(MasterRef { def, scene })
    }

    pub fn library(&self) -> &ComponentLibrary {
        &self.defs
    }

    pub fn insert_def(&mut self, def: ComponentDef, design_dir: PathBuf) {
        let id = def.id;
        self.paths.insert(id, design_dir);
        self.defs.defs.insert(id, def);
    }
}
