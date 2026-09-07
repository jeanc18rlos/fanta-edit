//! [`RefTable`] construction — the one place the doc model's component /
//! variable naming is distilled into the model-agnostic name↔id table
//! `fanta-fnx` resolves and emits references with.
//!
//! Two builders, one contract:
//!
//! - [`build_ref_table`] works on the typed [`ComponentLibrary`] /
//!   [`VariableRegistry`] the session layer already holds.
//! - [`build_ref_table_json`] works in raw JSON space for
//!   `read_project_tree`, which assembles the doc from per-file JSON BEFORE
//!   the schema-migration ladder runs — deserializing an older tree's
//!   `variables.json` into today's typed registry could fail, and name
//!   extraction must not. It reads only the fields whose shape has been
//!   stable since variables existed (`name`, `collection`) and simply skips
//!   anything unreadable, so a partially understood registry degrades to
//!   fewer resolvable names, never to a load error.
//!
//! Both spell a variable's canonical path as
//! `"{collection_name}/{variable_name}"` and register ids in their bare-ULID
//! serde form (the exact string that appears inside node JSON), so the two
//! builders and the codec always agree on the vocabulary.

use fanta_doc::{ComponentLibrary, VariableRegistry};
use fanta_fnx::RefTable;
use serde_json::{Map, Value};

/// Distill the typed component library + variable registry into a resolving
/// [`RefTable`]. `emit_names` opts the table into printing names (project
/// layout v4+); resolution on parse is unconditional for a built table.
pub(crate) fn build_ref_table(
    components: &ComponentLibrary,
    variables: &VariableRegistry,
    emit_names: bool,
) -> RefTable {
    let mut table = RefTable::new(emit_names);
    for (id, def) in &components.defs {
        table.insert_component(&def.name, &id.0.to_string());
    }
    for variable in variables.variables.values() {
        // A variable whose collection dangles has no canonical path; it stays
        // addressable by id only (same dangling-ref tolerance as the doc).
        let Some(collection) = variables.collections.get(&variable.collection) else {
            continue;
        };
        table.insert_variable(
            &format!("{}/{}", collection.name, variable.name),
            &variable.id.0.to_string(),
        );
    }
    table
}

/// JSON-space twin of [`build_ref_table`] for the project-tree reader:
/// `defs` is the `components/*/def.json` map keyed by bare component id,
/// `variables` the raw `doc/variables.json` value.
pub(crate) fn build_ref_table_json(
    defs: &Map<String, Value>,
    variables: &Value,
    emit_names: bool,
) -> RefTable {
    let mut table = RefTable::new(emit_names);
    for (id, def) in defs {
        if let Some(name) = def.get("name").and_then(Value::as_str) {
            table.insert_component(name, id);
        }
    }
    let collections = variables
        .get("collections")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(registry) = variables.get("variables").and_then(Value::as_object) {
        for (id, variable) in registry {
            let Some(name) = variable.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(collection_name) = variable
                .get("collection")
                .and_then(Value::as_str)
                .and_then(|collection| collections.get(collection))
                .and_then(|collection| collection.get("name"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            table.insert_variable(&format!("{collection_name}/{name}"), id);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_and_typed_builders_agree_on_a_real_registry() {
        use fanta_doc::{
            ComponentDef, ComponentId, Mode, ModeId, NodeId, Variable, VariableCollection,
            VariableCollectionId, VariableId, VariableType,
        };

        let mut components = ComponentLibrary::new();
        let component = ComponentId::new();
        components.defs.insert(
            component,
            ComponentDef::new(component, NodeId::new(), "Button"),
        );

        let mut variables = VariableRegistry::new();
        let collection = VariableCollectionId::new();
        let mode = ModeId::new();
        variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Theme".into(),
                modes: vec![Mode {
                    id: mode,
                    name: "Light".into(),
                }],
                default_mode: mode,
                variable_order: Vec::new(),
            },
        );
        let variable = VariableId::new();
        variables.variables.insert(
            variable,
            Variable {
                id: variable,
                collection,
                name: "bg".into(),
                ty: VariableType::Color,
                values_by_mode: Default::default(),
                scopes: Vec::new(),
            },
        );

        let typed = build_ref_table(&components, &variables, true);

        // The exact JSON the writer puts on disk, rebuilt by hand.
        let mut defs = serde_json::Map::new();
        defs.insert(
            component.0.to_string(),
            serde_json::to_value(&components.defs[&component]).unwrap(),
        );
        let variables_json = serde_json::to_value(&variables).unwrap();
        let json = build_ref_table_json(&defs, &variables_json, true);

        assert_eq!(typed, json, "the two builders must produce one vocabulary");
    }

    #[test]
    fn json_builder_skips_unreadable_entries_instead_of_failing() {
        let mut defs = serde_json::Map::new();
        defs.insert("01AAAAAAAAAAAAAAAAAAAAAAAA".into(), json!({}));
        let variables = json!({
            "collections": {"01CCCCCCCCCCCCCCCCCCCCCCCC": {"name": "Theme"}},
            "variables": {
                "01DDDDDDDDDDDDDDDDDDDDDDDD": {"name": "bg", "collection": "01CCCCCCCCCCCCCCCCCCCCCCCC"},
                "01EEEEEEEEEEEEEEEEEEEEEEEE": {"name": "orphan", "collection": "01MISSINGMISSINGMISSING000"},
            },
        });
        let table = build_ref_table_json(&defs, &variables, false);
        let mut expected = RefTable::new(false);
        expected.insert_variable("Theme/bg", "01DDDDDDDDDDDDDDDDDDDDDDDD");
        assert_eq!(table, expected);
    }
}
