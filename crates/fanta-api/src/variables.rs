//! Text variables: the values `{{name}}` tokens in text show. A key is a
//! variable's bare name when no other variable shares it, or its
//! `Collection/Name` path. Setting a new key creates a String variable in the
//! collection the path names, or in the `Text` collection.

use anyhow::anyhow;
use fanta_doc::{
    Mode, ModeId, NodeId, Operation, VarValue, Variable, VariableCollection, VariableCollectionId,
    VariableId, VariableType,
};

use crate::{ApiError, DesignSession, Result};

/// Where text variables set under a new bare name are created.
const TEXT_COLLECTION: &str = "Text";

impl DesignSession {
    /// The keys of the String variables, sorted: bare names where they are
    /// unique, `Collection/Name` paths otherwise.
    pub fn text_variables(&self) -> Vec<String> {
        let variables = &self.doc().variables;
        let mut keys: Vec<String> = variables
            .variables
            .values()
            .filter(|variable| variable.ty == VariableType::String)
            .filter_map(|variable| {
                if fanta_doc::text_token_variable(variables, &variable.name) == Some(variable.id) {
                    return Some(variable.name.clone());
                }
                variables
                    .collections
                    .get(&variable.collection)
                    .map(|collection| format!("{}/{}", collection.name, variable.name))
            })
            .collect();
        keys.sort();
        keys
    }

    /// The text a `{{key}}` token shows outside any frame that pins a mode:
    /// its variable's value in the document's active mode. `None` when the key
    /// names no variable, or one that is not text-like.
    pub fn text_variable(&self, key: &str) -> Option<String> {
        let doc = self.doc();
        // An anchor outside the scene inherits no frame's pinned mode, so the
        // value is the one the document's active modes select.
        fanta_doc::resolve_text_token(
            &doc.variables,
            &doc.scene,
            NodeId::new(),
            &doc.active_modes,
            key.trim(),
        )
    }

    /// Set the String variable `key` names to `value`, in its collection's
    /// active mode. A new key creates the variable with `value` in every mode.
    pub fn set_text_variable(&mut self, key: &str, value: &str) -> Result<()> {
        let key = key.trim();
        if key.is_empty() || key.len() > 512 || key.contains(['{', '}', '\n']) {
            return Err(ApiError::InvalidArgument(
                "a text variable needs a 1–512 byte name without braces or line breaks".into(),
            ));
        }
        let Some(variable) = fanta_doc::text_token_variable(&self.doc().variables, key) else {
            let operations = self.create_text_variable_operations(key, value)?;
            return self.edit(operations);
        };
        let doc = self.doc();
        let record = doc
            .variables
            .variables
            .get(&variable)
            .ok_or_else(|| anyhow!("variable {variable} is missing"))?;
        if record.ty != VariableType::String {
            return Err(ApiError::InvalidArgument(format!(
                "`{key}` is not a text variable"
            )));
        }
        let collection = doc
            .variables
            .collections
            .get(&record.collection)
            .ok_or_else(|| anyhow!("the collection of `{key}` is missing"))?;
        let mode = doc
            .active_modes
            .get(&collection.id)
            .copied()
            .filter(|mode| collection.has_mode(*mode))
            .unwrap_or(collection.default_mode);
        let operation = fanta_headless::edit_operations::set_variable_value_operation(
            doc,
            variable,
            mode,
            VarValue::String {
                value: value.to_owned(),
            },
        )
        .map_err(|reason| ApiError::InvalidArgument(reason.into()))?;
        self.edit(operation.into_iter().collect())
    }

    /// Delete the variable `key` names. Tokens naming it then show as typed.
    /// Returns whether there was one.
    pub fn remove_text_variable(&mut self, key: &str) -> Result<bool> {
        let Some(variable) = fanta_doc::text_token_variable(&self.doc().variables, key.trim())
        else {
            return Ok(false);
        };
        let transaction = self
            .doc()
            .delete_variable_transaction(variable)
            .map_err(anyhow::Error::new)?;
        self.edit(transaction.ops)?;
        Ok(true)
    }

    /// The operations creating the String variable a new `key` names, in the
    /// collection its `Collection/` prefix names, else in the Text collection,
    /// which they create when it is missing.
    fn create_text_variable_operations(&self, key: &str, value: &str) -> Result<Vec<Operation>> {
        let variables = &self.doc().variables;
        let (collection, name) = variables
            .collections
            .values()
            .find_map(|collection| {
                let name = key
                    .strip_prefix(collection.name.as_str())?
                    .strip_prefix('/')?;
                (!name.is_empty()).then_some((Some(collection), name))
            })
            .unwrap_or_else(|| {
                let text = variables
                    .collections
                    .values()
                    .find(|collection| collection.name == TEXT_COLLECTION);
                let name = key
                    .strip_prefix(TEXT_COLLECTION)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .filter(|rest| !rest.is_empty())
                    .unwrap_or(key);
                (text, name)
            });
        // The key did not resolve, so a variable that already has this name
        // or path shares it with another one.
        if variables.variables.values().any(|variable| {
            variable.name == key
                || collection.is_some_and(|collection| {
                    variable.collection == collection.id && variable.name == name
                })
        }) {
            return Err(ApiError::InvalidArgument(format!(
                "`{key}` names more than one variable; use its Collection/Name path"
            )));
        }
        let mut operations = Vec::new();
        let (collection, modes) = match collection {
            Some(collection) => (
                collection.id,
                collection.modes.iter().map(|mode| mode.id).collect(),
            ),
            None => {
                let mode = ModeId::new();
                let collection = VariableCollection {
                    id: VariableCollectionId::new(),
                    name: TEXT_COLLECTION.into(),
                    modes: vec![Mode {
                        id: mode,
                        name: "Default".into(),
                    }],
                    default_mode: mode,
                    variable_order: Vec::new(),
                };
                let id = collection.id;
                operations.push(Operation::CreateVariableCollection {
                    collection: Box::new(collection),
                });
                (id, vec![mode])
            }
        };
        if modes.is_empty() {
            return Err(ApiError::InvalidArgument(format!(
                "the collection `{key}` names has no modes"
            )));
        }
        let text = VarValue::String {
            value: value.to_owned(),
        };
        operations.push(Operation::CreateVariable {
            variable: Box::new(Variable {
                id: VariableId::new(),
                collection,
                name: name.to_owned(),
                ty: VariableType::String,
                values_by_mode: modes.into_iter().map(|mode| (mode, text.clone())).collect(),
                scopes: Vec::new(),
                options: Vec::new(),
            }),
        });
        Ok(operations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeEvent, NodeEventKind, NodeFilter, NodeKind};
    use fanta_design_ops::{DesignOp, DesignVariableType};
    use fanta_doc::NodeData;

    fn shown_text(session: &DesignSession, node: NodeId) -> String {
        let doc = session.doc();
        match fanta_doc::displayed_text_node(&doc.scene, &doc.variables, &doc.active_modes, node)
            .map(|node| node.data)
        {
            Some(NodeData::Text(text)) => text.content,
            _ => panic!("a text node"),
        }
    }

    #[test]
    fn setting_a_text_variable_creates_the_text_collection_once() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let greeting = session
            .create_node(page, NodeKind::Text("{{name}}, welcome".into()))
            .expect("text");
        assert_eq!(shown_text(&session, greeting), "{{name}}, welcome");
        session.set_text_variable("Text/name", "Ada").expect("set");
        session
            .set_text_variable("plan", "Pro")
            .expect("set another");
        assert_eq!(session.doc().variables.collections.len(), 1);
        assert_eq!(session.text_variables(), vec!["name", "plan"]);
        assert_eq!(session.text_variable("name"), Some("Ada".into()));
        assert_eq!(session.text_variable("Text/plan"), Some("Pro".into()));
        assert_eq!(shown_text(&session, greeting), "Ada, welcome");
        assert!(session.set_text_variable("{bad}", "x").is_err());
        assert!(session.set_text_variable("  ", "x").is_err());
    }

    #[test]
    fn text_variables_list_bare_and_qualified_keys() {
        let mut session = DesignSession::new().expect("session");
        session.set_text_variable("headline", "Hello").expect("set");
        session
            .edit_with_design_op(&DesignOp::CreateVariableCollection {
                name: "Copy".into(),
                modes: vec!["Default".into()],
            })
            .expect("collection");
        session
            .edit_with_design_op(&DesignOp::CreateVariable {
                collection: "Copy".into(),
                name: "count".into(),
                kind: DesignVariableType::Float,
                value: serde_json::json!(3),
                options: Vec::new(),
            })
            .expect("number");
        session
            .set_text_variable("Copy/headline", "Hola")
            .expect("set in Copy");
        session
            .set_text_variable("Copy/footer", "Bye")
            .expect("set footer");
        assert_eq!(
            session.text_variables(),
            vec!["Copy/headline", "Text/headline", "footer"]
        );
        assert_eq!(session.text_variable("headline"), None, "ambiguous");
        assert_eq!(session.text_variable("Copy/headline"), Some("Hola".into()));
        assert_eq!(session.text_variable("count"), Some("3".into()));
        assert!(
            session.set_text_variable("headline", "x").is_err(),
            "an ambiguous key is refused rather than guessed"
        );
        assert!(
            session.set_text_variable("count", "x").is_err(),
            "a number variable is not text"
        );
    }

    #[test]
    fn removing_a_text_variable_leaves_tokens_literal() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let label = session
            .create_node(page, NodeKind::Text("Plan: {{plan}}".into()))
            .expect("text");
        session.set_text_variable("plan", "Team").expect("set");
        assert_eq!(shown_text(&session, label), "Plan: Team");
        assert!(session.remove_text_variable("plan").expect("remove"));
        assert!(!session.remove_text_variable("plan").expect("already gone"));
        assert_eq!(shown_text(&session, label), "Plan: {{plan}}");
    }

    #[test]
    fn text_variable_changes_undo_and_update_their_text() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let label = session
            .create_node(page, NodeKind::Text("{{city}}".into()))
            .expect("text");
        session.set_text_variable("city", "Lima").expect("set");
        session.add_undo_step("City").expect("commit");
        let subscription = session.subscribe(NodeFilter::All);
        session.set_text_variable("city", "Quito").expect("change");
        assert_eq!(shown_text(&session, label), "Quito");
        let updated = NodeEvent {
            node: label,
            kind: NodeEventKind::Updated,
        };
        assert!(
            session
                .poll_events(subscription)
                .expect("poll")
                .contains(&updated)
        );
        assert!(session.undo().expect("undo"));
        assert_eq!(shown_text(&session, label), "Lima");
        assert!(
            session
                .poll_events(subscription)
                .expect("poll")
                .contains(&updated)
        );
    }
}
