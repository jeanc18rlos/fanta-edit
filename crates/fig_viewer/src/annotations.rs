use anyhow::{Context as _, Result, bail, ensure};
use fanta_doc::{Doc, DocId, NodeData, NodeId, Operation, Transform2D, Viewport};
use glam::DVec2;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::measurements::MeasurementProjection;

const META_KEY: &str = "annotations";
const VERSION: u32 = 1;
const DRAG_THRESHOLD_PX: f64 = 3.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DeveloperAnnotation {
    pub(crate) version: u32,
    pub(crate) id: String,
    pub(crate) anchor: [f64; 2],
    pub(crate) text: String,
    pub(crate) author: String,
    pub(crate) created: u64,
}

impl DeveloperAnnotation {
    #[cfg(test)]
    pub(crate) fn new(
        anchor: [f64; 2],
        text: String,
        author: String,
        created: u64,
    ) -> Result<Self> {
        let annotation = Self {
            version: VERSION,
            id: NodeId::new().to_string(),
            anchor,
            text,
            author,
            created,
        };
        annotation.validate()?;
        Ok(annotation)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == VERSION,
            "This annotation version is not supported."
        );
        ensure!(
            !self.id.trim().is_empty(),
            "The annotation has no stable identity."
        );
        validate_content(self.anchor, &self.text)
    }
}

fn finite_point(point: [f64; 2]) -> Result<DVec2> {
    ensure!(
        point.into_iter().all(f64::is_finite),
        "The annotation anchor must be a finite page coordinate."
    );
    Ok(DVec2::from(point))
}

fn validate_content(anchor: [f64; 2], text: &str) -> Result<()> {
    finite_point(anchor)?;
    ensure!(
        !text.trim().is_empty(),
        "Enter annotation text before adding or saving the note."
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnnotationRecord {
    document: DocId,
    scene_instance: u64,
    page: NodeId,
    annotation: DeveloperAnnotation,
    raw: Value,
}

impl AnnotationRecord {
    pub(crate) fn annotation(&self) -> &DeveloperAnnotation {
        &self.annotation
    }
    pub(crate) fn page(&self) -> NodeId {
        self.page
    }
}

pub(crate) fn read_annotations(doc: &Doc, page: NodeId) -> Result<Vec<AnnotationRecord>> {
    let entries = annotation_entries(page_metadata(doc, page)?)?;
    let mut identity_counts = HashMap::new();
    for entry in entries {
        if let Some(id) = entry.get("id").and_then(Value::as_str) {
            *identity_counts.entry(id).or_insert(0_usize) += 1;
        }
    }
    let mut records = Vec::new();
    for entry in entries {
        let Ok(annotation) = serde_json::from_value::<DeveloperAnnotation>(entry.clone()) else {
            continue;
        };
        if annotation.validate().is_err() || identity_counts.get(annotation.id.as_str()) != Some(&1)
        {
            continue;
        }
        records.push(AnnotationRecord {
            document: doc.id,
            scene_instance: doc.scene.instance_id(),
            page,
            annotation,
            raw: entry.clone(),
        });
    }
    Ok(records)
}

// SetMeta does not compare `old` when applied. Build and apply these operations
// against the current Doc within one guarded foreground item update.
pub(crate) fn create_annotation_op(
    doc: &Doc,
    page: NodeId,
    annotation: &DeveloperAnnotation,
) -> Result<Operation> {
    annotation.validate()?;
    let metadata = page_metadata(doc, page)?;
    let entries = annotation_entries(metadata)?;
    ensure!(
        !entries
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(annotation.id.as_str())),
        "A record with this annotation identity already exists."
    );
    let mut updated = entries.to_vec();
    updated.push(serde_json::to_value(annotation).context("Could not encode the annotation.")?);
    metadata_operation(page, metadata, updated)
}

pub(crate) fn update_annotation_op(
    doc: &Doc,
    expected: &AnnotationRecord,
    anchor: [f64; 2],
    text: &str,
) -> Result<Option<Operation>> {
    let (metadata, entries, index) = current_record(doc, expected)?;
    validate_content(anchor, text)?;
    if expected.annotation.anchor == anchor && expected.annotation.text == text {
        return Ok(None);
    }
    let mut updated = entries.to_vec();
    let record = updated
        .get_mut(index)
        .and_then(Value::as_object_mut)
        .context("The annotation is no longer available.")?;
    // Touch only changed fields so attribution, future fields and untouched
    // numeric JSON spellings survive an edit by this client.
    if expected.annotation.anchor != anchor {
        record.insert("anchor".into(), serde_json::to_value(anchor)?);
    }
    if expected.annotation.text != text {
        record.insert("text".into(), Value::String(text.into()));
    }
    metadata_operation(expected.page, metadata, updated).map(Some)
}

pub(crate) fn delete_annotation_op(doc: &Doc, expected: &AnnotationRecord) -> Result<Operation> {
    let (metadata, entries, index) = current_record(doc, expected)?;
    let updated = entries
        .iter()
        .enumerate()
        .filter(|(entry_index, _)| *entry_index != index)
        .map(|(_, entry)| entry.clone())
        .collect();
    metadata_operation(expected.page, metadata, updated)
}

fn page_metadata(doc: &Doc, page: NodeId) -> Result<&Value> {
    ensure!(
        doc.pages().contains(&page),
        "Annotations belong to an ordinary document page."
    );
    let node = doc
        .scene
        .get(page)
        .context("The annotation's page no longer exists.")?;
    ensure!(
        matches!(&node.data, NodeData::Group(_)),
        "The annotation page is not a page container."
    );
    ensure!(
        node.meta.is_null() || node.meta.is_object(),
        "The page metadata uses an unsupported format; it was left unchanged."
    );
    Ok(&node.meta)
}

fn annotation_entries(metadata: &Value) -> Result<&[Value]> {
    match metadata.get(META_KEY) {
        None => Ok(&[]),
        Some(Value::Array(entries)) => Ok(entries),
        Some(_) => bail!("The page annotation data is not a list; it was left unchanged."),
    }
}

fn current_record<'document>(
    doc: &'document Doc,
    expected: &AnnotationRecord,
) -> Result<(&'document Value, &'document [Value], usize)> {
    ensure!(
        doc.id == expected.document && doc.scene.instance_id() == expected.scene_instance,
        "The document was replaced while the annotation was being edited."
    );
    let metadata = page_metadata(doc, expected.page)?;
    let entries = annotation_entries(metadata)?;
    let mut matching = entries.iter().enumerate().filter(|(_, entry)| {
        entry.get("id").and_then(Value::as_str) == Some(expected.annotation.id.as_str())
    });
    let (index, current) = matching
        .next()
        .context("The annotation was deleted while it was being edited.")?;
    ensure!(
        matching.next().is_none(),
        "The annotation identity is ambiguous; nothing was changed."
    );
    ensure!(
        current == &expected.raw,
        "The annotation changed while it was being edited. Copy your draft or cancel it before starting a new edit."
    );
    Ok((metadata, entries, index))
}

fn metadata_operation(page: NodeId, old: &Value, entries: Vec<Value>) -> Result<Operation> {
    let mut metadata = match old {
        Value::Null => Map::new(),
        Value::Object(metadata) => metadata.clone(),
        _ => bail!("The page metadata uses an unsupported format; it was left unchanged."),
    };
    if entries.is_empty() {
        metadata.remove(META_KEY);
    } else {
        metadata.insert(META_KEY.into(), Value::Array(entries));
    }
    Ok(Operation::SetMeta {
        id: page,
        old: old.clone(),
        new: Value::Object(metadata),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnnotationOrigin {
    document: DocId,
    scene_instance: u64,
    page: NodeId,
}

impl AnnotationOrigin {
    fn capture(doc: &Doc, page: NodeId) -> Result<Self> {
        annotation_entries(page_metadata(doc, page)?)?;
        ensure!(
            doc.active_page() == Some(page),
            "Return to the annotation's page before editing it."
        );
        Ok(Self {
            document: doc.id,
            scene_instance: doc.scene.instance_id(),
            page,
        })
    }

    fn validate(&self, doc: &Doc) -> Result<()> {
        ensure!(
            *self == Self::capture(doc, self.page)?,
            "The annotation draft belongs to a different document instance. Its text was kept."
        );
        Ok(())
    }
}

fn projection_for_page(
    doc: &Doc,
    page: NodeId,
    viewport: Viewport,
    screen_size: [f64; 2],
) -> Result<MeasurementProjection> {
    let transform = doc
        .scene
        .world_transform(page)
        .context("The annotation page transform is unavailable.")?;
    MeasurementProjection::new(transform, viewport, screen_size)
}

pub(crate) fn annotation_pin_hit_test(
    projection: MeasurementProjection,
    anchor: [f64; 2],
    screen: [f64; 2],
    radius_px: f64,
) -> Result<bool> {
    ensure!(
        radius_px.is_finite() && radius_px >= 0.0,
        "The annotation hit radius is not valid."
    );
    let delta = finite_point(screen)? - finite_point(projection.page_to_screen(anchor)?)?;
    let distance = delta.x.hypot(delta.y);
    ensure!(
        distance.is_finite(),
        "The annotation hit position is too large."
    );
    Ok(distance <= radius_px)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MovePhase {
    Dragging,
    Released,
    Rejected,
}

#[derive(Debug, Clone, PartialEq)]
struct AnnotationMove {
    projection: MeasurementProjection,
    page_to_world: Transform2D,
    press_screen: [f64; 2],
    press_page: [f64; 2],
    original_anchor: [f64; 2],
    dragged: bool,
    phase: MovePhase,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnnotationDraft {
    generation: u64,
    origin: AnnotationOrigin,
    expected: Option<AnnotationRecord>,
    annotation: DeveloperAnnotation,
    movement: Option<AnnotationMove>,
}

impl AnnotationDraft {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn annotation(&self) -> &DeveloperAnnotation {
        &self.annotation
    }
    pub(crate) fn page(&self) -> NodeId {
        self.origin.page
    }
    pub(crate) fn is_new(&self) -> bool {
        self.expected.is_none()
    }
    pub(crate) fn is_move(&self) -> bool {
        self.movement.is_some()
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct AnnotationController {
    generation: u64,
    draft: Option<AnnotationDraft>,
}

impl AnnotationController {
    pub(crate) fn draft(&self) -> Option<&AnnotationDraft> {
        self.draft.as_ref()
    }
    pub(crate) fn has_pending_authoring(&self) -> bool {
        self.draft.is_some()
    }
    pub(crate) fn is_moving(&self) -> bool {
        self.draft
            .as_ref()
            .and_then(|draft| draft.movement.as_ref())
            .is_some_and(|movement| movement.phase == MovePhase::Dragging)
    }
    pub(crate) fn cancel(&mut self) -> bool {
        self.draft.take().is_some()
    }
    pub(crate) fn cancel_draft(&mut self, generation: u64) -> bool {
        if self
            .draft
            .as_ref()
            .is_none_or(|draft| draft.generation != generation)
        {
            return false;
        }
        self.cancel()
    }

    pub(crate) fn begin_create(
        &mut self,
        doc: &Doc,
        page: NodeId,
        viewport: Viewport,
        screen_size: [f64; 2],
        screen: [f64; 2],
        author: String,
        created: u64,
    ) -> Result<()> {
        let origin = AnnotationOrigin::capture(doc, page)?;
        let anchor =
            projection_for_page(doc, page, viewport, screen_size)?.screen_to_page(screen)?;
        let annotation = DeveloperAnnotation {
            version: VERSION,
            id: NodeId::new().to_string(),
            anchor,
            text: String::new(),
            author,
            created,
        };
        self.begin(origin, None, annotation, None)
    }

    pub(crate) fn begin_edit(&mut self, doc: &Doc, expected: &AnnotationRecord) -> Result<()> {
        current_record(doc, expected)?;
        self.begin(
            AnnotationOrigin::capture(doc, expected.page)?,
            Some(expected.clone()),
            expected.annotation.clone(),
            None,
        )
    }

    pub(crate) fn begin_move(
        &mut self,
        doc: &Doc,
        expected: &AnnotationRecord,
        viewport: Viewport,
        screen_size: [f64; 2],
        press_screen: [f64; 2],
    ) -> Result<()> {
        current_record(doc, expected)?;
        let origin = AnnotationOrigin::capture(doc, expected.page)?;
        let projection = projection_for_page(doc, expected.page, viewport, screen_size)?;
        let page_to_world = doc
            .scene
            .world_transform(expected.page)
            .context("The annotation page transform is unavailable.")?;
        let press_page = projection.screen_to_page(press_screen)?;
        let movement = AnnotationMove {
            projection,
            page_to_world,
            press_screen,
            press_page,
            original_anchor: expected.annotation.anchor,
            dragged: false,
            phase: MovePhase::Dragging,
        };
        self.begin(
            origin,
            Some(expected.clone()),
            expected.annotation.clone(),
            Some(movement),
        )
    }

    fn begin(
        &mut self,
        origin: AnnotationOrigin,
        expected: Option<AnnotationRecord>,
        annotation: DeveloperAnnotation,
        movement: Option<AnnotationMove>,
    ) -> Result<()> {
        ensure!(
            self.draft.is_none(),
            "Add, save or cancel the existing annotation draft first."
        );
        let generation = self
            .generation
            .checked_add(1)
            .context("The annotation controller needs to be reopened.")?;
        self.generation = generation;
        self.draft = Some(AnnotationDraft {
            generation,
            origin,
            expected,
            annotation,
            movement,
        });
        Ok(())
    }

    pub(crate) fn set_text(&mut self, generation: u64, text: String) -> Result<()> {
        let draft = self
            .draft
            .as_mut()
            .context("There is no annotation draft.")?;
        ensure!(
            draft.generation == generation,
            "This text editor belongs to an earlier annotation draft."
        );
        ensure!(
            draft.movement.is_none(),
            "Finish or cancel the annotation movement before editing its text."
        );
        // Text remains editable/copyable after an origin failure. Only an
        // explicit Add/Save may attempt to persist it to the original page.
        draft.annotation.text = text;
        Ok(())
    }

    pub(crate) fn update_move(
        &mut self,
        doc: &Doc,
        viewport: Viewport,
        screen_size: [f64; 2],
        screen: [f64; 2],
    ) -> Result<()> {
        let result = self.update_move_inner(doc, viewport, screen_size, screen);
        if result.is_err() {
            self.freeze_move();
        }
        result
    }

    fn update_move_inner(
        &mut self,
        doc: &Doc,
        viewport: Viewport,
        screen_size: [f64; 2],
        screen: [f64; 2],
    ) -> Result<()> {
        let draft = self
            .draft
            .as_ref()
            .context("There is no annotation draft.")?;
        draft.origin.validate(doc)?;
        let movement = draft
            .movement
            .as_ref()
            .context("This annotation draft is not a movement.")?;
        ensure!(
            movement.phase == MovePhase::Dragging,
            "The annotation movement ended. Commit or cancel its preview first."
        );
        let projection = projection_for_page(doc, draft.origin.page, viewport, screen_size)?;
        ensure!(
            projection == movement.projection,
            "The page transform or viewport changed during the annotation movement. Cancel the movement and start again."
        );
        let delta = finite_point(screen)? - finite_point(movement.press_screen)?;
        let distance = delta.x.hypot(delta.y);
        ensure!(
            distance.is_finite(),
            "The annotation movement is too large."
        );
        let dragged = movement.dragged || distance >= DRAG_THRESHOLD_PX;
        if !dragged {
            return Ok(());
        }
        let page_delta =
            finite_point(projection.screen_to_page(screen)?)? - finite_point(movement.press_page)?;
        let anchor = if page_delta == DVec2::ZERO {
            movement.original_anchor
        } else {
            finite_point((finite_point(movement.original_anchor)? + page_delta).to_array())?
                .to_array()
        };
        let draft = self
            .draft
            .as_mut()
            .context("There is no annotation draft.")?;
        draft.annotation.anchor = anchor;
        let movement = draft
            .movement
            .as_mut()
            .context("This annotation draft is not a movement.")?;
        movement.dragged = true;
        Ok(())
    }

    pub(crate) fn release_move(
        &mut self,
        doc: &Doc,
        viewport: Viewport,
        screen_size: [f64; 2],
        screen: [f64; 2],
    ) -> Result<Option<AnnotationCommit>> {
        let result = self.update_move(doc, viewport, screen_size, screen);
        if result.is_ok()
            && let Some(movement) = self
                .draft
                .as_mut()
                .and_then(|draft| draft.movement.as_mut())
        {
            movement.phase = MovePhase::Released;
        }
        result?;
        self.prepare_move(doc)
    }

    pub(crate) fn freeze_move(&mut self) {
        if let Some(movement) = self
            .draft
            .as_mut()
            .and_then(|draft| draft.movement.as_mut())
            && movement.phase == MovePhase::Dragging
        {
            movement.phase = MovePhase::Rejected;
        }
    }

    pub(crate) fn prepare_add(&self, doc: &Doc, generation: u64) -> Result<AnnotationCommit> {
        let draft = self
            .draft
            .as_ref()
            .context("There is no annotation draft.")?;
        ensure!(
            draft.generation == generation,
            "This Add action belongs to an earlier annotation draft."
        );
        ensure!(
            draft.expected.is_none() && draft.movement.is_none(),
            "Use Save for an existing annotation."
        );
        self.prepare(doc)?
            .context("The new annotation has no changes.")
    }

    pub(crate) fn prepare_save(
        &self,
        doc: &Doc,
        generation: u64,
    ) -> Result<Option<AnnotationCommit>> {
        let draft = self
            .draft
            .as_ref()
            .context("There is no annotation draft.")?;
        ensure!(
            draft.generation == generation,
            "This Save action belongs to an earlier annotation draft."
        );
        ensure!(
            draft.expected.is_some() && draft.movement.is_none(),
            "Use Add for a new annotation or release its move gesture."
        );
        self.prepare(doc)
    }

    pub(crate) fn prepare_move(&self, doc: &Doc) -> Result<Option<AnnotationCommit>> {
        let draft = self
            .draft
            .as_ref()
            .context("There is no annotation draft.")?;
        let movement = draft
            .movement
            .as_ref()
            .context("This annotation draft is not a movement.")?;
        ensure!(
            movement.phase == MovePhase::Released,
            "The final annotation pointer position was not accepted. Release a valid gesture or cancel this preview."
        );
        self.prepare(doc)
    }

    fn prepare(&self, doc: &Doc) -> Result<Option<AnnotationCommit>> {
        let draft = self
            .draft
            .as_ref()
            .context("There is no annotation draft.")?;
        draft.origin.validate(doc)?;
        if let Some(movement) = &draft.movement {
            ensure!(
                doc.scene.world_transform(draft.origin.page) == Some(movement.page_to_world),
                "The page transform changed during the annotation movement. Its preview was kept."
            );
        }
        if let Some(expected) = &draft.expected {
            current_record(doc, expected)?;
        }
        draft.annotation.validate()?;
        if let Some(expected) = &draft.expected
            && expected.annotation.anchor == draft.annotation.anchor
            && expected.annotation.text == draft.annotation.text
        {
            return Ok(None);
        }
        Ok(Some(AnnotationCommit {
            generation: draft.generation,
            origin: draft.origin,
            expected: draft.expected.clone(),
            annotation: draft.annotation.clone(),
        }))
    }

    pub(crate) fn build_operation(
        &self,
        doc: &Doc,
        intent: &AnnotationCommit,
    ) -> Result<(String, Operation)> {
        let draft = self
            .draft
            .as_ref()
            .context("The annotation draft was cancelled.")?;
        if let Some(movement) = &draft.movement {
            ensure!(
                movement.phase == MovePhase::Released,
                "The final annotation pointer position was not accepted."
            );
        }
        ensure!(
            self.prepare(doc)?.as_ref() == Some(intent),
            "The annotation draft changed or was cancelled before commit."
        );
        let operation = match &intent.expected {
            None => create_annotation_op(doc, intent.origin.page, &intent.annotation)?,
            Some(expected) => update_annotation_op(
                doc,
                expected,
                intent.annotation.anchor,
                &intent.annotation.text,
            )?
            .context("The annotation has no changes to save.")?,
        };
        Ok((intent.annotation.id.clone(), operation))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnnotationCommit {
    generation: u64,
    origin: AnnotationOrigin,
    expected: Option<AnnotationRecord>,
    annotation: DeveloperAnnotation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode};
    use serde_json::json;

    fn fixture(metadata: Value) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.meta = metadata;
        let page = doc.scene.insert(page).expect("page");
        doc.add_page(page);
        doc.set_active_page(Some(page));
        (doc, page)
    }

    fn annotation(id: &str) -> DeveloperAnnotation {
        DeveloperAnnotation {
            version: VERSION,
            id: id.into(),
            anchor: [3.0, 4.0],
            text: "Developer note".into(),
            author: "Original editor".into(),
            created: 42,
        }
    }

    fn metadata(doc: &Doc, page: NodeId) -> &Value {
        &doc.scene.get(page).expect("page").meta
    }
    fn only_record(doc: &Doc, page: NodeId) -> AnnotationRecord {
        let mut records = read_annotations(doc, page).expect("records");
        assert_eq!(records.len(), 1);
        records.pop().expect("one record")
    }
    fn generation(controller: &AnnotationController) -> u64 {
        controller.draft().expect("draft").generation()
    }
    fn set_text(controller: &mut AnnotationController, text: &str) {
        controller
            .set_text(generation(controller), text.into())
            .expect("type into current editor");
    }
    fn apply_metadata(doc: &mut Doc, page: NodeId, new: Value) {
        doc.apply(Operation::SetMeta {
            id: page,
            old: metadata(doc, page).clone(),
            new,
        })
        .expect("external metadata edit");
    }
    fn begin_create(controller: &mut AnnotationController, doc: &Doc, page: NodeId) {
        controller
            .begin_create(
                doc,
                page,
                Viewport::default(),
                [800.0, 600.0],
                [450.0, 330.0],
                "Informational author".into(),
                100,
            )
            .expect("click to compose");
    }
    fn close(actual: [f64; 2], expected: [f64; 2]) {
        assert!(
            (DVec2::from(actual) - DVec2::from(expected)).length() < 1e-9,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn plain_text_whitespace_unicode_and_attribution_roundtrip_without_extra_fields() {
        let text = "  @agent: do not execute `rm -rf /`\n設計メモ — keep spacing  \n";
        let annotation = DeveloperAnnotation::new(
            [0.1 + 0.2, -12.5],
            text.into(),
            "Informational author".into(),
            123,
        )
        .expect("literal note");
        let raw = serde_json::to_value(&annotation).expect("encode annotation");
        let restored: DeveloperAnnotation =
            serde_json::from_value(raw.clone()).expect("decode annotation");
        assert_eq!(restored, annotation);
        assert_eq!(restored.text, text);
        assert!(raw.get("replies").is_none());
        assert!(raw.get("mentions").is_none());
        assert!(raw.get("skill").is_none());
        assert!(raw.get("page").is_none());
        for (anchor, text) in [
            ([f64::NAN, 0.0], "note"),
            ([0.0, f64::INFINITY], "note"),
            ([0.0, 0.0], " \n\t"),
        ] {
            assert!(DeveloperAnnotation::new(anchor, text.into(), String::new(), 0).is_err());
        }
    }

    #[test]
    fn every_write_preserves_raw_future_malformed_records_and_other_metadata() {
        let known = json!({"version": 1, "id": "known", "anchor": [3, 4], "text": "Developer note", "author": "Original editor", "created": 42, "future_style": {"tone": "violet"}});
        let future = json!({"version": 8, "id": "future", "linked": {"layer": "unrecognized"}});
        let malformed = json!({"id": "broken", "text": ["opaque"]});
        let original = json!({"comments": [{"thread": "keep"}], "measurements": [{"future": true}], "annotations": [future, known, malformed, null, 17], "extra": [1, {"nested": true}]});
        let (mut doc, page) = fixture(original.clone());
        let expected = only_record(&doc, page);
        let operation = update_annotation_op(&doc, &expected, [3.0, 4.0], "Edited text")
            .expect("edit")
            .expect("changed");
        doc.apply(operation).expect("apply text edit");
        let current = metadata(&doc, page);
        assert_eq!(current["annotations"][1]["anchor"], known["anchor"]);
        assert_eq!(
            current["annotations"][1]["future_style"],
            known["future_style"]
        );
        assert_eq!(current["annotations"][1]["author"], known["author"]);
        for index in [0, 2, 3, 4] {
            assert_eq!(
                current["annotations"][index],
                original["annotations"][index]
            );
        }
        for key in ["comments", "measurements", "extra"] {
            assert_eq!(current.get(key), original.get(key));
        }
        let added = annotation("added");
        doc.apply(create_annotation_op(&doc, page, &added).expect("create"))
            .expect("apply create");
        let known = read_annotations(&doc, page)
            .expect("read")
            .into_iter()
            .find(|record| record.annotation.id == "known")
            .expect("known");
        doc.apply(delete_annotation_op(&doc, &known).expect("delete"))
            .expect("apply delete");
        assert_eq!(
            metadata(&doc, page)["annotations"],
            json!([future, malformed, null, 17, added])
        );
        for key in ["comments", "measurements", "extra"] {
            assert_eq!(metadata(&doc, page).get(key), original.get(key));
        }
    }

    #[test]
    fn current_record_guard_merges_unrelated_edits_but_rejects_changed_targets_and_noops() {
        let (mut doc, page) =
            fixture(json!({"annotations": [annotation("target"), annotation("other")]}));
        let expected = read_annotations(&doc, page)
            .expect("records")
            .into_iter()
            .find(|record| record.annotation.id == "target")
            .expect("target");
        let mut current = metadata(&doc, page).clone();
        current["comments"] = json!(["new comment"]);
        current["annotations"][1]["text"] = json!("Unrelated annotation edit");
        apply_metadata(&mut doc, page, current.clone());
        let operation = update_annotation_op(&doc, &expected, [5.0, 6.0], "Target edited")
            .expect("merge")
            .expect("changed");
        let Operation::SetMeta { old, .. } = &operation else {
            panic!("metadata operation")
        };
        assert_eq!(old, &current);
        doc.apply(operation).expect("apply");
        assert_eq!(metadata(&doc, page)["comments"], current["comments"]);
        assert_eq!(
            metadata(&doc, page)["annotations"][1],
            current["annotations"][1]
        );
        assert!(doc.undo().expect("undo target edit"));
        assert_eq!(metadata(&doc, page), &current);
        current["annotations"][0]["future_field"] = json!("same-record external change");
        apply_metadata(&mut doc, page, current.clone());
        let depth = doc.history.undo_depth();
        assert!(
            update_annotation_op(
                &doc,
                &expected,
                expected.annotation.anchor,
                &expected.annotation.text
            )
            .is_err()
        );
        assert!(delete_annotation_op(&doc, &expected).is_err());
        assert_eq!(metadata(&doc, page), &current);
        assert_eq!(doc.history.undo_depth(), depth);
    }

    #[test]
    fn ambiguous_deleted_or_unsupported_records_never_get_retargeted() {
        for raw in [
            json!([]),
            json!(17),
            json!({"annotations": null}),
            json!({"annotations": {"future": true}}),
        ] {
            let (doc, page) = fixture(raw.clone());
            assert!(read_annotations(&doc, page).is_err());
            assert!(create_annotation_op(&doc, page, &annotation("new")).is_err());
            assert_eq!(metadata(&doc, page), &raw);
        }
        let (mut doc, page) = fixture(json!({"annotations": [annotation("target")]}));
        let expected = only_record(&doc, page);
        apply_metadata(
            &mut doc,
            page,
            json!({"annotations": [annotation("target"), {"version": 9, "id": "target"}]}),
        );
        assert!(
            read_annotations(&doc, page)
                .expect("read ambiguous IDs")
                .is_empty()
        );
        assert!(create_annotation_op(&doc, page, &annotation("target")).is_err());
        assert!(delete_annotation_op(&doc, &expected).is_err());
        apply_metadata(&mut doc, page, json!({"comments": ["keep"]}));
        assert!(update_annotation_op(&doc, &expected, [3.0, 4.0], "new text").is_err());
        let mut unsupported = annotation("future");
        unsupported.version = 2;
        assert!(create_annotation_op(&doc, page, &unsupported).is_err());
        let mut component = CanvasNode::new(NodeData::Group(GroupNode::default()));
        component.name = "Component master".into();
        let component = doc.scene.insert(component).expect("component");
        doc.set_active_page(Some(component));
        assert!(
            AnnotationController::default()
                .begin_create(
                    &doc,
                    component,
                    Viewport::default(),
                    [800.0, 600.0],
                    [400.0, 300.0],
                    String::new(),
                    0
                )
                .is_err()
        );
    }

    #[test]
    fn click_and_typing_are_local_until_explicit_add_and_empty_add_keeps_draft() {
        let (mut doc, page) = fixture(Value::Null);
        let mut controller = AnnotationController::default();
        begin_create(&mut controller, &doc, page);
        let token = generation(&controller);
        let id = controller.draft().expect("draft").annotation.id.clone();
        assert_eq!(
            controller.draft().expect("draft").annotation.anchor,
            [50.0, 30.0]
        );
        assert!(controller.prepare_add(&doc, token).is_err());
        assert!(controller.prepare_save(&doc, token).is_err());
        set_text(&mut controller, "  Unicode note\n検査  ");
        assert_eq!(metadata(&doc, page), &Value::Null);
        assert_eq!(doc.history.undo_depth(), 0);
        let intent = controller.prepare_add(&doc, token).expect("explicit Add");
        let (committed_id, operation) = controller
            .build_operation(&doc, &intent)
            .expect("build current operation");
        assert_eq!(committed_id, id);
        doc.apply(operation).expect("commit annotation");
        let saved = only_record(&doc, page);
        assert_eq!(saved.annotation.text, "  Unicode note\n検査  ");
        assert_eq!(saved.annotation.author, "Informational author");
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(
            controller.build_operation(&doc, &intent).is_err(),
            "a repeated Add cannot duplicate the stable ID"
        );
        assert!(controller.cancel_draft(token));
        assert!(!controller.cancel());
    }

    #[test]
    fn body_save_preserves_attribution_and_empty_text_cannot_delete_a_note() {
        let (mut doc, page) = fixture(json!({"annotations": [annotation("target")]}));
        let expected = only_record(&doc, page);
        let mut controller = AnnotationController::default();
        controller.begin_edit(&doc, &expected).expect("edit note");
        let token = generation(&controller);
        assert!(controller.prepare_add(&doc, token).is_err());
        assert!(
            controller
                .prepare_save(&doc, token)
                .expect("unchanged Save")
                .is_none()
        );
        set_text(&mut controller, " \n\t");
        assert!(controller.prepare_save(&doc, token).is_err());
        assert_eq!(
            controller.draft().expect("kept draft").annotation.text,
            " \n\t"
        );
        assert_eq!(only_record(&doc, page), expected);
        set_text(&mut controller, "New body\nwith a second line");
        let intent = controller
            .prepare_save(&doc, token)
            .expect("Save")
            .expect("body changed");
        let mut current = metadata(&doc, page).clone();
        current["measurements"] = json!([{"future": "preserve"}]);
        apply_metadata(&mut doc, page, current.clone());
        let (_, operation) = controller
            .build_operation(&doc, &intent)
            .expect("current metadata merge");
        doc.apply(operation).expect("apply body Save");
        let saved = only_record(&doc, page);
        assert_eq!(saved.annotation.anchor, expected.annotation.anchor);
        assert_eq!(saved.annotation.author, expected.annotation.author);
        assert_eq!(saved.annotation.created, expected.annotation.created);
        assert_eq!(
            metadata(&doc, page)["measurements"],
            current["measurements"]
        );
        assert!(doc.undo().expect("undo body Save"));
        assert_eq!(metadata(&doc, page), &current);
    }

    #[test]
    fn invalid_origin_preserves_copyable_text_and_returning_to_original_page_can_retry() {
        let (mut doc, page) = fixture(Value::Null);
        let mut controller = AnnotationController::default();
        begin_create(&mut controller, &doc, page);
        let token = generation(&controller);
        set_text(&mut controller, "Unsaved developer note");
        let original = controller.clone();
        doc.set_active_page(None);
        assert!(controller.prepare_add(&doc, token).is_err());
        assert_eq!(controller, original);
        set_text(
            &mut controller,
            "Unsaved developer note\nkeep typing after refusal",
        );
        let retained = controller.clone();
        doc.set_active_page(Some(page));
        let replaced = doc.clone();
        assert_eq!(replaced.id, doc.id);
        assert!(controller.prepare_add(&replaced, token).is_err());
        assert_eq!(controller, retained);
        let original_id = doc.id;
        doc.id = Doc::new().id;
        assert!(controller.prepare_add(&doc, token).is_err());
        assert_eq!(controller, retained);
        doc.id = original_id;
        assert!(controller.prepare_add(&doc, token).is_ok());
        assert_eq!(metadata(&doc, page), &Value::Null);
        assert_eq!(doc.history.undo_depth(), 0);
        assert!(controller.cancel_draft(token));
    }

    #[test]
    fn stale_editor_change_add_save_and_cancel_callbacks_cannot_touch_new_draft() {
        let (doc, page) = fixture(Value::Null);
        let mut controller = AnnotationController::default();
        begin_create(&mut controller, &doc, page);
        set_text(&mut controller, "Earlier note");
        let stale_token = generation(&controller);
        let stale = controller
            .prepare_add(&doc, stale_token)
            .expect("earlier Add");
        assert!(controller.cancel());
        begin_create(&mut controller, &doc, page);
        set_text(&mut controller, "Current note");
        let current = controller.clone();
        assert!(
            controller
                .set_text(stale_token, "stale editor event".into())
                .is_err()
        );
        assert!(controller.prepare_add(&doc, stale_token).is_err());
        assert!(controller.prepare_save(&doc, stale_token).is_err());
        assert!(!controller.cancel_draft(stale_token));
        assert!(controller.build_operation(&doc, &stale).is_err());
        assert_eq!(controller, current);
        let current_token = generation(&controller);
        let intent = controller
            .prepare_add(&doc, current_token)
            .expect("current Add");
        set_text(&mut controller, "A newer edit in the same draft");
        assert!(controller.build_operation(&doc, &intent).is_err());
        assert_eq!(metadata(&doc, page), &Value::Null);
    }

    #[test]
    fn busy_or_conflicting_editor_keeps_exact_text_until_explicit_cancel() {
        let (mut doc, page) = fixture(json!({"annotations": [annotation("target")]}));
        let expected = only_record(&doc, page);
        let mut controller = AnnotationController::default();
        controller.begin_edit(&doc, &expected).expect("edit");
        set_text(&mut controller, "  Keep this draft\n\tαβγ  ");
        let retained = controller.clone();
        let token = generation(&controller);
        assert!(controller.begin_edit(&doc, &expected).is_err());
        assert!(
            controller
                .begin_create(
                    &doc,
                    page,
                    Viewport::default(),
                    [800.0, 600.0],
                    [450.0, 330.0],
                    String::new(),
                    0
                )
                .is_err()
        );
        assert!(
            controller
                .begin_move(
                    &doc,
                    &expected,
                    Viewport::default(),
                    [800.0, 600.0],
                    [403.0, 304.0]
                )
                .is_err()
        );
        assert_eq!(controller, retained);
        let intent = controller
            .prepare_save(&doc, token)
            .expect("prepare Save")
            .expect("text changed");
        let mut current = metadata(&doc, page).clone();
        current["annotations"][0]["text"] = json!("Changed in another view");
        apply_metadata(&mut doc, page, current.clone());
        let depth = doc.history.undo_depth();
        assert!(controller.prepare_save(&doc, token).is_err());
        assert!(controller.build_operation(&doc, &intent).is_err());
        assert_eq!(controller, retained);
        assert_eq!(metadata(&doc, page), &current);
        assert_eq!(doc.history.undo_depth(), depth);
        assert!(controller.cancel_draft(token));
        assert_eq!(metadata(&doc, page), &current);
        assert_eq!(doc.history.undo_depth(), depth);
    }

    #[test]
    fn invalid_projection_or_click_creates_no_draft_and_no_history() {
        let (mut doc, page) = fixture(Value::Null);
        let mut controller = AnnotationController::default();
        for (transform, viewport, size, point) in [
            (
                Transform2D::scale_xy(0.0, 1.0),
                Viewport::default(),
                [800.0, 600.0],
                [450.0, 330.0],
            ),
            (
                Transform2D::IDENTITY,
                Viewport {
                    center: [0.0, 0.0],
                    zoom: 0.0,
                },
                [800.0, 600.0],
                [450.0, 330.0],
            ),
            (
                Transform2D::IDENTITY,
                Viewport::default(),
                [0.0, 600.0],
                [450.0, 330.0],
            ),
            (
                Transform2D::IDENTITY,
                Viewport::default(),
                [800.0, 600.0],
                [f64::INFINITY, 330.0],
            ),
        ] {
            doc.scene.get_mut(page).expect("page").transform = transform;
            assert!(
                controller
                    .begin_create(&doc, page, viewport, size, point, String::new(), 0)
                    .is_err()
            );
            assert_eq!(controller, AnnotationController::default());
            assert_eq!(metadata(&doc, page), &Value::Null);
            assert_eq!(doc.history.undo_depth(), 0);
        }
    }

    #[test]
    fn move_uses_page_projection_press_offset_and_screen_pixel_threshold() {
        for zoom in [0.5, 1.0, 2.0] {
            let (mut doc, page) = fixture(json!({"annotations": [annotation("target")]}));
            doc.scene.get_mut(page).expect("page").transform = Transform2D::scale_xy(2.0, 0.5)
                .then(&Transform2D::rotation(std::f64::consts::FRAC_PI_2))
                .then(&Transform2D::translation(100.0, -20.0));
            let viewport = Viewport {
                center: [0.0, 0.0],
                zoom,
            };
            let projection =
                projection_for_page(&doc, page, viewport, [800.0, 600.0]).expect("projection");
            let expected = only_record(&doc, page);
            let anchor_screen = projection
                .page_to_screen(expected.annotation.anchor)
                .expect("projected anchor");
            assert!(
                annotation_pin_hit_test(
                    projection,
                    expected.annotation.anchor,
                    [anchor_screen[0] + 5.9, anchor_screen[1]],
                    6.0
                )
                .expect("screen hit")
            );
            assert!(
                !annotation_pin_hit_test(
                    projection,
                    expected.annotation.anchor,
                    [anchor_screen[0] + 6.1, anchor_screen[1]],
                    6.0
                )
                .expect("screen miss")
            );
            let press = [anchor_screen[0] + 2.0, anchor_screen[1] - 1.0];
            let mut controller = AnnotationController::default();
            controller
                .begin_move(&doc, &expected, viewport, [800.0, 600.0], press)
                .expect("press near pin");
            controller
                .update_move(&doc, viewport, [800.0, 600.0], [press[0] + 2.9, press[1]])
                .expect("subthreshold movement");
            assert_eq!(
                controller.draft().expect("draft").annotation.anchor,
                expected.annotation.anchor
            );
            assert!(
                controller.prepare_move(&doc).is_err(),
                "release is required"
            );
            let intent = controller
                .release_move(&doc, viewport, [800.0, 600.0], [press[0] + 3.1, press[1]])
                .expect("release")
                .expect("moved");
            close(
                controller.draft().expect("preview").annotation.anchor,
                [3.0, 4.0 - 6.2 / zoom],
            );
            assert_eq!(
                only_record(&doc, page),
                expected,
                "preview has no metadata writes"
            );
            let (_, operation) = controller
                .build_operation(&doc, &intent)
                .expect("move operation");
            doc.apply(operation).expect("apply move");
            assert_eq!(doc.history.undo_depth(), 1);
            assert_eq!(
                only_record(&doc, page).annotation.text,
                expected.annotation.text
            );
            assert!(doc.undo().expect("undo move"));
            assert_eq!(only_record(&doc, page), expected);
        }
    }

    #[test]
    fn move_original_return_click_only_and_focus_loss_never_create_an_operation() {
        let (doc, page) = fixture(json!({"annotations": [annotation("target")]}));
        let expected = only_record(&doc, page);
        let mut controller = AnnotationController::default();
        for moved in [false, true] {
            controller
                .begin_move(
                    &doc,
                    &expected,
                    Viewport::default(),
                    [800.0, 600.0],
                    [403.0, 304.0],
                )
                .expect("press");
            let token = generation(&controller);
            assert!(
                controller
                    .set_text(token, "must not change during a move".into())
                    .is_err()
            );
            if moved {
                controller
                    .update_move(&doc, Viewport::default(), [800.0, 600.0], [450.0, 330.0])
                    .expect("move");
            }
            assert!(
                controller
                    .release_move(&doc, Viewport::default(), [800.0, 600.0], [403.0, 304.0])
                    .expect("original return")
                    .is_none()
            );
            assert_eq!(
                controller.draft().expect("draft").annotation,
                expected.annotation
            );
            assert!(controller.cancel());
        }
        controller
            .begin_move(
                &doc,
                &expected,
                Viewport::default(),
                [800.0, 600.0],
                [403.0, 304.0],
            )
            .expect("press");
        controller
            .update_move(&doc, Viewport::default(), [800.0, 600.0], [450.0, 330.0])
            .expect("move");
        let anchor = controller.draft().expect("draft").annotation.anchor;
        controller.freeze_move();
        assert!(!controller.is_moving());
        assert!(
            controller
                .update_move(&doc, Viewport::default(), [800.0, 600.0], [460.0, 340.0])
                .is_err()
        );
        assert!(controller.prepare_move(&doc).is_err());
        assert_eq!(
            controller.draft().expect("frozen draft").annotation.anchor,
            anchor
        );
        assert!(controller.has_pending_authoring());
        assert_eq!(only_record(&doc, page), expected);
        assert_eq!(doc.history.undo_depth(), 0);
    }

    #[test]
    fn rejected_final_move_and_page_transform_changes_cannot_commit_prior_preview() {
        let (mut doc, page) = fixture(json!({"annotations": [annotation("target")]}));
        let expected = only_record(&doc, page);
        for (viewport, position) in [
            (
                Viewport {
                    center: [0.0, 0.0],
                    zoom: 2.0,
                },
                [450.0, 330.0],
            ),
            (Viewport::default(), [f64::NAN, 330.0]),
        ] {
            let mut controller = AnnotationController::default();
            controller
                .begin_move(
                    &doc,
                    &expected,
                    Viewport::default(),
                    [800.0, 600.0],
                    [403.0, 304.0],
                )
                .expect("press");
            controller
                .update_move(&doc, Viewport::default(), [800.0, 600.0], [450.0, 330.0])
                .expect("accepted move");
            let anchor = controller.draft().expect("draft").annotation.anchor;
            assert!(
                controller
                    .release_move(&doc, viewport, [800.0, 600.0], position)
                    .is_err()
            );
            assert!(controller.prepare_move(&doc).is_err());
            assert_eq!(
                controller
                    .draft()
                    .expect("retained preview")
                    .annotation
                    .anchor,
                anchor
            );
            assert_eq!(only_record(&doc, page), expected);
        }
        let mut controller = AnnotationController::default();
        controller
            .begin_move(
                &doc,
                &expected,
                Viewport::default(),
                [800.0, 600.0],
                [403.0, 304.0],
            )
            .expect("press");
        let intent = controller
            .release_move(&doc, Viewport::default(), [800.0, 600.0], [450.0, 330.0])
            .expect("release")
            .expect("move intent");
        let draft = controller.clone();
        doc.apply(Operation::SetTransform {
            id: page,
            old: Transform2D::IDENTITY,
            new: Transform2D::translation(10.0, 20.0),
        })
        .expect("external page transform");
        assert!(controller.build_operation(&doc, &intent).is_err());
        assert_eq!(controller, draft);
        assert_eq!(only_record(&doc, page), expected);
    }

    #[test]
    fn rejected_intermediate_move_stays_frozen_but_valid_release_remains_retryable() {
        let (doc, page) = fixture(json!({"annotations": [annotation("target")]}));
        let expected = only_record(&doc, page);
        let mut controller = AnnotationController::default();
        controller
            .begin_move(
                &doc,
                &expected,
                Viewport::default(),
                [800.0, 600.0],
                [403.0, 304.0],
            )
            .expect("press");
        controller
            .update_move(&doc, Viewport::default(), [800.0, 600.0], [450.0, 330.0])
            .expect("accepted preview");
        let preview = controller.draft().expect("preview").annotation.clone();
        assert!(
            controller
                .update_move(
                    &doc,
                    Viewport {
                        center: [1.0, 0.0],
                        zoom: 1.0
                    },
                    [800.0, 600.0],
                    [460.0, 340.0]
                )
                .is_err()
        );
        assert!(!controller.is_moving());
        assert!(
            controller
                .release_move(&doc, Viewport::default(), [800.0, 600.0], [450.0, 330.0])
                .is_err()
        );
        assert!(controller.prepare_move(&doc).is_err());
        assert_eq!(
            controller.draft().expect("retained preview").annotation,
            preview
        );
        controller.cancel();
        controller
            .begin_move(
                &doc,
                &expected,
                Viewport::default(),
                [800.0, 600.0],
                [403.0, 304.0],
            )
            .expect("new press");
        let intent = controller
            .release_move(&doc, Viewport::default(), [800.0, 600.0], [450.0, 330.0])
            .expect("valid release")
            .expect("changed");
        controller.freeze_move();
        assert!(
            controller
                .release_move(&doc, Viewport::default(), [800.0, 600.0], [460.0, 340.0])
                .is_err(),
            "a repeated mouse-up cannot change the accepted final position"
        );
        assert_eq!(
            controller.prepare_move(&doc).expect("retry"),
            Some(intent.clone())
        );
        assert!(controller.build_operation(&doc, &intent).is_ok());
        assert_eq!(only_record(&doc, page), expected);
        assert_eq!(doc.history.undo_depth(), 0);
    }

    #[test]
    fn add_body_save_move_and_delete_each_restore_exact_metadata_with_undo_redo() {
        let original = json!({"comments": ["keep"], "annotations": [{"version": 9, "id": "future", "value": null}], "measurements": [17]});
        let (mut doc, page) = fixture(original.clone());
        let note = annotation("stable");
        doc.apply(create_annotation_op(&doc, page, &note).expect("Add"))
            .expect("apply Add");
        let added = metadata(&doc, page).clone();
        let expected = only_record(&doc, page);
        doc.apply(
            update_annotation_op(&doc, &expected, note.anchor, "Saved body")
                .expect("Save")
                .expect("changed"),
        )
        .expect("apply Save");
        let saved = metadata(&doc, page).clone();
        let expected = only_record(&doc, page);
        doc.apply(
            update_annotation_op(&doc, &expected, [30.0, 40.0], "Saved body")
                .expect("Move")
                .expect("changed"),
        )
        .expect("apply Move");
        let moved = metadata(&doc, page).clone();
        let expected = only_record(&doc, page);
        doc.apply(delete_annotation_op(&doc, &expected).expect("Delete"))
            .expect("apply Delete");
        assert_eq!(doc.history.undo_depth(), 4);
        assert_eq!(metadata(&doc, page), &original);
        for expected in [&moved, &saved, &added, &original] {
            assert!(doc.undo().expect("one-step Undo"));
            assert_eq!(metadata(&doc, page), expected);
        }
        for expected in [&added, &saved, &moved, &original] {
            assert!(doc.redo().expect("one-step Redo"));
            assert_eq!(metadata(&doc, page), expected);
        }
    }
}
