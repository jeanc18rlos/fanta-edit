//! Where components live in a project (layout v5).
//!
//! A component is one folder under `components/`. A standalone component is
//! `components/<slug>/` (`def.json`, `master.fnx`, `master.ids.json`). A
//! component with variants (a variant set) is `components/<set-slug>/`
//! holding its `set.json` and one folder per variant, named after the
//! variant's values:
//!
//! ```text
//! components/button/set.json
//! components/button/primary-default/{def.json, master.fnx, master.ids.json}
//! components/button/primary-hover/…
//! ```
//!
//! Layout v4 kept every variant in its own top-level `components/<slug>/`
//! and all sets in `components/sets.json`; [`scan_component_dirs`] reads
//! both, and the next save writes v5 and removes the v4 files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fanta_doc::{ComponentDef, ComponentId, ComponentSet, Doc};
use serde_json::{Map, Value};

use super::layout::{COMPONENTS_DIR, DEF_JSON, SET_JSON, SETS_JSON};
use super::write::design_slugs;
use crate::error::{FormatError, Result};

/// The projected directory of every component and variant set, relative to
/// the project root.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ComponentDirs {
    pub(crate) components: BTreeMap<ComponentId, PathBuf>,
    pub(crate) sets: BTreeMap<ComponentId, PathBuf>,
}

/// The set a def is a variant of, when that set exists.
fn member_set<'a>(doc: &'a Doc, def: &ComponentDef) -> Option<&'a ComponentSet> {
    def.variant_of
        .as_ref()
        .and_then(|membership| doc.components.sets.get(&membership.set))
}

/// A variant folder's name source: its values in the set's axis order
/// ("Primary Default"), or the def's name when it has none.
fn variant_label(def: &ComponentDef, set: &ComponentSet) -> String {
    let values = def
        .variant_of
        .as_ref()
        .map(|membership| {
            set.axes
                .iter()
                .filter_map(|axis| membership.axis_values.get(&axis.name))
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    if values.trim().is_empty() {
        def.name.clone()
    } else {
        values
    }
}

/// Every component's and set's directory for `doc`. Sets and standalone
/// components share the `components/` namespace; variants are named within
/// their set. Collisions get `-2`, `-3`, … in id order, so the layout is a
/// pure function of the doc.
pub(crate) fn component_dirs(doc: &Doc) -> ComponentDirs {
    let standalone = doc
        .components
        .defs
        .values()
        .filter(|def| member_set(doc, def).is_none())
        .map(|def| (def.id, def.name.clone()));
    let sets = doc
        .components
        .sets
        .values()
        .map(|set| (set.id, set.name.clone()));
    let top = design_slugs(standalone.chain(sets), "component");
    let root = PathBuf::from(COMPONENTS_DIR);
    let mut dirs = ComponentDirs::default();
    for (id, slug) in top {
        if doc.components.sets.contains_key(&id) {
            dirs.sets.insert(id, root.join(slug));
        } else {
            dirs.components.insert(id, root.join(slug));
        }
    }
    for set in doc.components.sets.values() {
        let Some(set_dir) = dirs.sets.get(&set.id).cloned() else {
            continue;
        };
        let variants = doc
            .components
            .defs
            .values()
            .filter(|def| member_set(doc, def).is_some_and(|member_of| member_of.id == set.id))
            .map(|def| (def.id, variant_label(def, set)));
        for (id, slug) in design_slugs(variants, "variant") {
            dirs.components.insert(id, set_dir.join(slug));
        }
    }
    dirs
}

/// The component folders and set files under `components/`, in either layout.
#[derive(Debug, Default)]
pub(crate) struct ScannedComponents {
    /// Absolute folders holding a `def.json`, standalone or variant.
    pub(crate) component_dirs: Vec<PathBuf>,
    /// Absolute set files: v4's `components/sets.json` (a map of sets), then
    /// each v5 `components/<set>/set.json` (one set).
    pub(crate) set_files: Vec<PathBuf>,
}

fn sorted_dirs(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// Walk `components/` under `project_root`: each folder with a `def.json` is
/// a component; each with a `set.json` is a variant set whose sub-folders
/// with a `def.json` are its variants.
pub(crate) fn scan_component_dirs(project_root: &Path) -> Result<ScannedComponents> {
    let components = project_root.join(COMPONENTS_DIR);
    let mut scan = ScannedComponents::default();
    if !components.is_dir() {
        return Ok(scan);
    }
    let legacy = components.join(SETS_JSON);
    if legacy.is_file() {
        scan.set_files.push(legacy);
    }
    for dir in sorted_dirs(&components)? {
        if dir.join(DEF_JSON).is_file() {
            scan.component_dirs.push(dir);
        } else if dir.join(SET_JSON).is_file() {
            scan.set_files.push(dir.join(SET_JSON));
            for variant in sorted_dirs(&dir)? {
                if variant.join(DEF_JSON).is_file() {
                    scan.component_dirs.push(variant);
                }
            }
        }
    }
    Ok(scan)
}

/// Every set in `files` as one `{ "<id>": set }` object, the shape of
/// `ComponentLibrary::sets`. A v4 `sets.json` contributes its whole map; a
/// v5 `set.json` contributes the one set it holds.
pub(crate) fn merge_set_files(files: &[(PathBuf, Value)]) -> Result<Value> {
    let mut sets = Map::new();
    for (path, value) in files {
        if path.file_name().is_some_and(|name| name == SETS_JSON) {
            let Value::Object(map) = value else {
                return Err(FormatError::InvalidProjectTree(format!(
                    "{}: component sets must be an object",
                    path.display()
                )));
            };
            sets.extend(map.clone());
        } else {
            let id = value.get("id").and_then(Value::as_str).ok_or_else(|| {
                FormatError::InvalidProjectTree(format!("{}: the set has no id", path.display()))
            })?;
            sets.insert(id.to_owned(), value.clone());
        }
    }
    Ok(Value::Object(sets))
}

/// The design directory (relative to the project root) of the component a
/// path under `components/` belongs to: `components/<slug>` for a standalone
/// component, `components/<set>/<variant>` for a variant. `None` for a set's
/// own `set.json`, or a path outside `components/`.
pub(crate) fn component_design_dir_of(project_root: &Path, relative: &Path) -> Option<PathBuf> {
    let parts: Vec<&str> = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<_>>()?;
    let [COMPONENTS_DIR, first, rest @ ..] = parts.as_slice() else {
        return None;
    };
    let top = PathBuf::from(COMPONENTS_DIR).join(first);
    let is_set = project_root.join(&top).join(SET_JSON).is_file()
        || (!project_root.join(&top).join(DEF_JSON).is_file()
            && rest.first().is_some_and(|second| {
                project_root
                    .join(&top)
                    .join(second)
                    .join(DEF_JSON)
                    .is_file()
            }));
    match rest {
        [] => (!is_set).then_some(top),
        [file] if is_set && *file == SET_JSON => None,
        [second, ..] if is_set => Some(top.join(second)),
        _ => Some(top),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, ComponentSetMembership, GroupNode, NodeData, VariantAxis};

    fn def(doc: &mut Doc, name: &str) -> ComponentId {
        let root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        doc.scene.insert(root).unwrap();
        let id = ComponentId::new();
        doc.components
            .defs
            .insert(id, ComponentDef::new(id, root_id, name));
        id
    }

    #[test]
    fn variants_live_in_their_sets_folder_named_after_their_values() {
        let mut doc = Doc::new();
        let avatar = def(&mut doc, "Avatar");
        let primary = def(&mut doc, "Variant=Primary, State=Default");
        let hover = def(&mut doc, "Variant=Primary, State=Hover");
        let set_id = ComponentId::new();
        doc.components.sets.insert(
            set_id,
            ComponentSet {
                id: set_id,
                name: "Button".into(),
                axes: vec![
                    VariantAxis {
                        name: "Variant".into(),
                        values: vec!["Primary".into()],
                    },
                    VariantAxis {
                        name: "State".into(),
                        values: vec!["Default".into(), "Hover".into()],
                    },
                ],
                members: vec![primary, hover],
                default_variant: primary,
                root: None,
            },
        );
        for (member, state) in [(primary, "Default"), (hover, "Hover")] {
            doc.components.defs.get_mut(&member).unwrap().variant_of =
                Some(ComponentSetMembership {
                    set: set_id,
                    axis_values: BTreeMap::from([
                        ("Variant".into(), "Primary".into()),
                        ("State".into(), state.into()),
                    ]),
                });
        }
        let dirs = component_dirs(&doc);
        assert_eq!(dirs.sets[&set_id], Path::new("components/button"));
        assert_eq!(
            dirs.components[&primary],
            Path::new("components/button/primary-default")
        );
        assert_eq!(
            dirs.components[&hover],
            Path::new("components/button/primary-hover")
        );
        assert_eq!(dirs.components[&avatar], Path::new("components/avatar"));
    }

    #[test]
    fn paths_under_components_map_to_their_design() {
        let root = tempfile::tempdir().unwrap();
        let path = |relative: &str| root.path().join(relative);
        for file in [
            "components/avatar/def.json",
            "components/button/set.json",
            "components/button/primary/def.json",
        ] {
            std::fs::create_dir_all(path(file).parent().unwrap()).unwrap();
            std::fs::write(path(file), "{}").unwrap();
        }
        let of = |relative: &str| component_design_dir_of(root.path(), Path::new(relative));
        assert_eq!(
            of("components/avatar/master.fnx"),
            Some(PathBuf::from("components/avatar"))
        );
        assert_eq!(
            of("components/button/primary/master.fnx"),
            Some(PathBuf::from("components/button/primary"))
        );
        assert_eq!(of("components/button/set.json"), None);
        assert_eq!(of("pages/home/page.fnx"), None);
        let scan = scan_component_dirs(root.path()).unwrap();
        assert_eq!(
            scan.component_dirs,
            [path("components/avatar"), path("components/button/primary")]
        );
        assert_eq!(scan.set_files, [path("components/button/set.json")]);
    }
}
