use fanta_doc::{Doc, NodeData, NodeFlags, NodeId, Operation, Transaction};
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) fn transaction_is_dev_mark_edit(
    doc: &Doc,
    active_page: NodeId,
    transaction: &Transaction,
) -> bool {
    if transaction.is_empty()
        || doc.active_page() != Some(active_page)
        || !doc.pages().contains(&active_page)
        || doc.is_component_root(active_page)
    {
        return false;
    }
    let Some(page) = doc.scene.get(active_page) else {
        return false;
    };
    if !matches!(page.data, NodeData::Group(_)) || page.flags.contains(NodeFlags::HIDDEN) {
        return false;
    }
    let Some(current_unrelated) = unrelated_metadata(&page.meta) else {
        return false;
    };
    transaction.ops.iter().all(|operation| {
        let Operation::SetMeta { id, old, new } = operation else {
            return false;
        };
        if *id != active_page
            || (old.get("measurements") == new.get("measurements")
                && old.get("annotations") == new.get("annotations"))
        {
            return false;
        }
        match (unrelated_metadata(old), unrelated_metadata(new)) {
            // SetMeta replaces the full blob, including unrelated fields that
            // may have been changed outside this document's history.
            (Some(old), Some(new)) => old == new && old == current_unrelated,
            _ => false,
        }
    })
}

fn unrelated_metadata(metadata: &Value) -> Option<BTreeMap<&str, &Value>> {
    match metadata {
        Value::Null => Some(BTreeMap::new()),
        Value::Object(metadata) => Some(
            metadata
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "measurements" | "annotations"))
                .map(|(key, value)| (key.as_str(), value))
                .collect(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::{
        DeveloperAnnotation, create_annotation_op, delete_annotation_op, read_annotations,
        update_annotation_op,
    };
    use crate::measurements::{
        Measurement, create_measurement_op, delete_measurement_op, read_measurements,
        update_measurement_op,
    };
    use fanta_doc::{
        CanvasNode, Color, ComponentDef, ComponentId, GroupNode, Transform2D, VectorNode,
    };
    use serde_json::json;

    fn fixture(metadata: Value) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.meta = metadata;
        let page = doc.scene.insert(page).expect("insert page");
        doc.add_page(page);
        (doc, page)
    }

    fn transaction(operations: Vec<Operation>) -> Transaction {
        Transaction {
            label: "Untrusted history label".into(),
            ops: operations,
        }
    }

    fn metadata_op(page: NodeId, old: Value, new: Value) -> Operation {
        Operation::SetMeta { id: page, old, new }
    }

    fn apply_and_check_history(doc: &mut Doc, page: NodeId, operation: Operation) {
        let before = doc.scene.get(page).expect("page").meta.clone();
        doc.apply(operation).expect("apply mark");
        let after = doc.scene.get(page).expect("page").meta.clone();
        assert_ne!(before, after);
        assert!(transaction_is_dev_mark_edit(
            doc,
            page,
            doc.history.next_undo_transaction().expect("undo mark")
        ));
        assert!(doc.undo().expect("undo mark"));
        assert_eq!(doc.scene.get(page).expect("page").meta, before);
        assert!(transaction_is_dev_mark_edit(
            doc,
            page,
            doc.history.next_redo_transaction().expect("redo mark")
        ));
        assert!(doc.redo().expect("redo mark"));
        assert_eq!(doc.scene.get(page).expect("page").meta, after);
    }

    #[test]
    fn annotation_create_edit_delete_are_allowed_in_both_history_directions() {
        let (mut doc, page) = fixture(json!({
            "comments": [{"future": [null, 17]}],
            "measurements": [{"version": 99}],
            "unknown": {"preserve": [1, 2.0, "exact"]}
        }));
        let annotation = DeveloperAnnotation::new(
            [3.0, 4.0],
            "Implementation note".into(),
            "Designer".into(),
            42,
        )
        .expect("annotation");
        let operation = create_annotation_op(&doc, page, &annotation).expect("create annotation");
        apply_and_check_history(&mut doc, page, operation);
        let record = read_annotations(&doc, page)
            .expect("annotations")
            .pop()
            .expect("annotation record");
        let operation = update_annotation_op(&doc, &record, [8.0, 9.0], "Updated note")
            .expect("edit annotation")
            .expect("changed annotation");
        apply_and_check_history(&mut doc, page, operation);
        let record = read_annotations(&doc, page)
            .expect("annotations")
            .pop()
            .expect("updated record");
        let operation = delete_annotation_op(&doc, &record).expect("delete annotation");
        apply_and_check_history(&mut doc, page, operation);
    }

    #[test]
    fn measurement_create_edit_delete_are_allowed_from_null_metadata() {
        let (mut doc, page) = fixture(Value::Null);
        let measurement =
            Measurement::new([0.0, 0.0], [3.0, 4.0], "Designer".into(), 42).expect("measurement");
        let operation =
            create_measurement_op(&doc, page, &measurement).expect("create measurement");
        apply_and_check_history(&mut doc, page, operation);
        let record = read_measurements(&doc, page)
            .expect("measurements")
            .pop()
            .expect("measurement record");
        let operation = update_measurement_op(&doc, &record, [5.0, 6.0], [8.0, 10.0])
            .expect("edit measurement")
            .expect("changed measurement");
        apply_and_check_history(&mut doc, page, operation);
        let record = read_measurements(&doc, page)
            .expect("measurements")
            .pop()
            .expect("updated record");
        let operation = delete_measurement_op(&doc, &record).expect("delete measurement");
        apply_and_check_history(&mut doc, page, operation);
        assert_eq!(doc.scene.get(page).expect("page").meta, json!({}));
    }

    #[test]
    fn grouped_mark_edits_allow_every_operation_without_trusting_labels() {
        let (mut doc, page) = fixture(Value::Null);
        let annotation = DeveloperAnnotation::new([3.0, 4.0], "Note".into(), "Designer".into(), 42)
            .expect("annotation");
        let measurement =
            Measurement::new([0.0, 0.0], [3.0, 4.0], "Designer".into(), 42).expect("measurement");
        doc.history.begin("Transform", &mut doc.scene);
        let annotation = create_annotation_op(&doc, page, &annotation).expect("create annotation");
        doc.apply(annotation).expect("apply annotation");
        let measurement =
            create_measurement_op(&doc, page, &measurement).expect("create measurement");
        doc.apply(measurement).expect("apply measurement");
        doc.history.commit(&mut doc.scene);
        let transaction = doc.history.next_undo_transaction().expect("grouped marks");
        assert_eq!(transaction.ops.len(), 2);
        assert!(transaction_is_dev_mark_edit(&doc, page, transaction));
        assert!(doc.undo().expect("undo grouped marks"));
        assert_eq!(doc.scene.get(page).expect("page").meta, Value::Null);
        assert!(transaction_is_dev_mark_edit(
            &doc,
            page,
            doc.history
                .next_redo_transaction()
                .expect("redo grouped marks")
        ));
    }

    #[test]
    fn mixed_artwork_and_page_transactions_are_refused_without_changing_history() {
        let (doc, page) = fixture(Value::Null);
        let mark = metadata_op(
            page,
            Value::Null,
            json!({"annotations": [{"text": "Note"}]}),
        );
        for unrelated in [
            Operation::SetTransform {
                id: page,
                old: Transform2D::IDENTITY,
                new: Transform2D::translation(10.0, 20.0),
            },
            Operation::create_node(CanvasNode::new(NodeData::Group(GroupNode::default()))),
            Operation::SetPageRegistry {
                old_pages: vec![page],
                new_pages: Vec::new(),
                old_active_page: Some(page),
                new_active_page: None,
            },
            metadata_op(
                page,
                Value::Null,
                json!({"comments": [{"text": "Feedback"}]}),
            ),
        ] {
            for operations in [
                vec![mark.clone(), unrelated.clone()],
                vec![unrelated.clone(), mark.clone()],
            ] {
                let transaction = transaction(operations);
                let before = serde_json::to_value(&doc).expect("document snapshot");
                assert!(!transaction_is_dev_mark_edit(&doc, page, &transaction));
                assert_eq!(
                    serde_json::to_value(&doc).expect("unchanged document"),
                    before
                );
            }
        }
    }

    #[test]
    fn refusing_mixed_undo_and_redo_leaves_the_document_and_both_stacks_unchanged() {
        let (mut doc, page) = fixture(Value::Null);
        doc.apply_transaction(transaction(vec![
            metadata_op(page, Value::Null, json!({"measurements": []})),
            Operation::SetTransform {
                id: page,
                old: Transform2D::IDENTITY,
                new: Transform2D::translation(10.0, 20.0),
            },
        ]))
        .expect("mixed transaction");
        let before = serde_json::to_value(&doc).expect("document snapshot");
        assert!(!transaction_is_dev_mark_edit(
            &doc,
            page,
            doc.history.next_undo_transaction().expect("mixed undo")
        ));
        assert_eq!(
            serde_json::to_value(&doc).expect("unchanged document"),
            before
        );

        assert!(doc.undo().expect("prepare redo stack outside Dev mode"));
        let before = serde_json::to_value(&doc).expect("document snapshot");
        assert!(!transaction_is_dev_mark_edit(
            &doc,
            page,
            doc.history.next_redo_transaction().expect("mixed redo")
        ));
        assert_eq!(
            serde_json::to_value(&doc).expect("unchanged document"),
            before
        );
    }

    #[test]
    fn unrelated_metadata_is_exact_and_cannot_be_overwritten_from_stale_history() {
        let original = json!({"unknown": {"nested": [null, 1, 2.0]}, "comments": []});
        let (mut doc, page) = fixture(original.clone());
        let mut changed = original.clone();
        changed["measurements"] = json!([{"version": 99}]);
        let allowed = transaction(vec![metadata_op(page, original.clone(), changed.clone())]);
        assert!(transaction_is_dev_mark_edit(&doc, page, &allowed));
        for (key, value) in [
            ("unknown", json!({"nested": [null, 1.0, 2.0]})),
            ("comments", json!([{"text": "Changed"}])),
            ("new_extension", json!(true)),
        ] {
            let mut unrelated_change = changed.clone();
            unrelated_change[key] = value;
            assert!(!transaction_is_dev_mark_edit(
                &doc,
                page,
                &transaction(vec![metadata_op(page, original.clone(), unrelated_change)])
            ));
        }
        doc.scene.get_mut(page).expect("page").meta["new_extension"] = json!("External field");
        assert!(!transaction_is_dev_mark_edit(&doc, page, &allowed));
    }

    #[test]
    fn null_empty_and_malformed_metadata_are_classified_conservatively() {
        let (doc, page) = fixture(Value::Null);
        let marks = json!({"annotations": [{"text": "Note"}]});
        for empty in [Value::Null, json!({})] {
            for (old, new) in [(empty.clone(), marks.clone()), (marks.clone(), empty)] {
                assert!(transaction_is_dev_mark_edit(
                    &doc,
                    page,
                    &transaction(vec![metadata_op(page, old, new)])
                ));
            }
        }
        assert!(!transaction_is_dev_mark_edit(
            &doc,
            page,
            &transaction(Vec::new())
        ));
        for (old, new) in [
            (Value::Null, json!({})),
            (marks.clone(), marks.clone()),
            (json!(17), marks.clone()),
            (marks.clone(), json!([])),
            (json!("old"), marks),
        ] {
            assert!(!transaction_is_dev_mark_edit(
                &doc,
                page,
                &transaction(vec![metadata_op(page, old, new)])
            ));
        }
    }

    #[test]
    fn marks_must_belong_to_the_current_existing_ordinary_visible_page() {
        let (mut doc, page) = fixture(Value::Null);
        let second_page = doc
            .scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .expect("second page");
        doc.add_page(second_page);
        let mark = transaction(vec![metadata_op(
            page,
            Value::Null,
            json!({"annotations": []}),
        )]);
        assert!(transaction_is_dev_mark_edit(&doc, page, &mark));
        let mut cross_page = mark.clone();
        cross_page.ops.push(metadata_op(
            second_page,
            Value::Null,
            json!({"measurements": []}),
        ));
        assert!(!transaction_is_dev_mark_edit(&doc, page, &cross_page));
        assert!(!transaction_is_dev_mark_edit(&doc, second_page, &mark));
        doc.set_active_page(Some(second_page));
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
        assert!(!transaction_is_dev_mark_edit(&doc, second_page, &mark));
        doc.set_active_page(None);
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
        doc.set_active_page(Some(page));
        doc.scene
            .get_mut(page)
            .expect("page")
            .flags
            .insert(NodeFlags::HIDDEN);
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
        doc.scene
            .get_mut(page)
            .expect("page")
            .flags
            .remove(NodeFlags::HIDDEN);
        doc.pages.retain(|candidate| *candidate != page);
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
        doc.pages.push(page);
        doc.scene.get_mut(page).expect("page").meta = json!([]);
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
        doc.scene.get_mut(page).expect("page").meta = Value::Null;
        let component = ComponentDef {
            id: ComponentId::new(),
            root: page,
            name: "Component master".into(),
            variant_of: None,
            props: Vec::new(),
            rev: 0,
            preview_rev: 0,
        };
        doc.components.defs.insert(component.id, component);
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
        doc.components.defs.clear();
        doc.scene.get_mut(page).expect("page").data =
            NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, Color::BLACK));
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
        doc.scene.remove(page).expect("remove page");
        assert!(!transaction_is_dev_mark_edit(&doc, page, &mark));
    }
}
