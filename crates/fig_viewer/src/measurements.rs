use anyhow::{Context as _, Result, bail, ensure};
use fanta_doc::{Bounds, Doc, DocId, NodeData, NodeId, Operation, Transform2D, Viewport};
use glam::DVec2;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

const META_KEY: &str = "measurements";
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Measurement {
    pub(crate) version: u32,
    pub(crate) id: String,
    pub(crate) start: [f64; 2],
    pub(crate) end: [f64; 2],
    pub(crate) author: String,
    pub(crate) created: u64,
}

impl Measurement {
    pub(crate) fn new(
        start: [f64; 2],
        end: [f64; 2],
        author: String,
        created: u64,
    ) -> Result<Self> {
        distance_px(start, end)?;
        Ok(Self {
            version: VERSION,
            id: NodeId::new().to_string(),
            start,
            end,
            author,
            created,
        })
    }

    pub(crate) fn distance_px(&self) -> Result<f64> {
        distance_px(self.start, self.end)
    }

    pub(crate) fn label(&self) -> Result<String> {
        Ok(distance_label(self.distance_px()?))
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == VERSION,
            "This measurement version is not supported."
        );
        ensure!(
            !self.id.trim().is_empty(),
            "The measurement has no stable identity."
        );
        self.distance_px()?;
        Ok(())
    }
}

fn distance_label(distance: f64) -> String {
    if distance > 0.0 && distance < 0.01 {
        return "<0.01 px".into();
    }
    let formatted = format!("{distance:.2}");
    format!(
        "{} px",
        formatted.trim_end_matches('0').trim_end_matches('.')
    )
}

fn distance_px(start: [f64; 2], end: [f64; 2]) -> Result<f64> {
    ensure!(
        start.into_iter().chain(end).all(f64::is_finite),
        "Measurement endpoints must be finite page coordinates."
    );
    let distance = (end[0] - start[0]).hypot(end[1] - start[1]);
    ensure!(
        distance.is_finite(),
        "The measurement distance is too large."
    );
    ensure!(distance > 0.0, "Measurement endpoints must be different.");
    Ok(distance)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasurementRecord {
    page: NodeId,
    scene_instance: u64,
    measurement: Measurement,
    raw: Value,
}

impl MeasurementRecord {
    pub(crate) fn measurement(&self) -> &Measurement {
        &self.measurement
    }

    pub(crate) fn page(&self) -> NodeId {
        self.page
    }
}

pub(crate) fn read_measurements(doc: &Doc, page: NodeId) -> Result<Vec<MeasurementRecord>> {
    let metadata = page_metadata(doc, page)?;
    let entries = measurement_entries(metadata)?;
    let mut identity_counts = HashMap::new();
    for entry in entries {
        if let Some(id) = entry.get("id").and_then(Value::as_str) {
            *identity_counts.entry(id).or_insert(0_usize) += 1;
        }
    }
    let mut records = Vec::new();
    for entry in entries {
        let Ok(measurement) = serde_json::from_value::<Measurement>(entry.clone()) else {
            continue;
        };
        if measurement.validate().is_err()
            || identity_counts.get(measurement.id.as_str()) != Some(&1)
        {
            continue;
        }
        records.push(MeasurementRecord {
            page,
            scene_instance: doc.scene.instance_id(),
            measurement,
            raw: entry.clone(),
        });
    }
    Ok(records)
}

// SetMeta does not compare its `old` value during application. Call these
// builders and apply their result within the same guarded document update.
pub(crate) fn create_measurement_op(
    doc: &Doc,
    page: NodeId,
    measurement: &Measurement,
) -> Result<Operation> {
    measurement.validate()?;
    let metadata = page_metadata(doc, page)?;
    let entries = measurement_entries(metadata)?;
    ensure!(
        !entries
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(measurement.id.as_str())),
        "A record with this measurement identity already exists."
    );
    let mut updated = entries.to_vec();
    updated.push(serde_json::to_value(measurement).context("Could not encode the measurement.")?);
    metadata_operation(page, metadata, updated)
}

pub(crate) fn update_measurement_op(
    doc: &Doc,
    expected: &MeasurementRecord,
    start: [f64; 2],
    end: [f64; 2],
) -> Result<Option<Operation>> {
    let (metadata, entries, index) = current_record(doc, expected)?;
    distance_px(start, end)?;
    if expected.measurement.start == start && expected.measurement.end == end {
        return Ok(None);
    }
    let mut updated = entries.to_vec();
    let record = updated
        .get_mut(index)
        .and_then(Value::as_object_mut)
        .context("The measurement record is no longer available.")?;
    // Copying only geometry preserves fields written by newer clients, as well
    // as the original attribution and the representation of untouched fields.
    if expected.measurement.start != start {
        record.insert("start".into(), serde_json::to_value(start)?);
    }
    if expected.measurement.end != end {
        record.insert("end".into(), serde_json::to_value(end)?);
    }
    metadata_operation(expected.page, metadata, updated).map(Some)
}

pub(crate) fn delete_measurement_op(doc: &Doc, expected: &MeasurementRecord) -> Result<Operation> {
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
        "Measurements belong to a document page."
    );
    let node = doc
        .scene
        .get(page)
        .context("The measurement page no longer exists.")?;
    ensure!(
        matches!(&node.data, NodeData::Group(_)),
        "The measurement page is not a page container."
    );
    ensure!(
        node.meta.is_null() || node.meta.is_object(),
        "The page metadata uses an unsupported format; it was left unchanged."
    );
    Ok(&node.meta)
}

fn measurement_entries(metadata: &Value) -> Result<&[Value]> {
    match metadata.get(META_KEY) {
        None => Ok(&[]),
        Some(Value::Array(entries)) => Ok(entries),
        Some(_) => bail!("The page measurement data is not a list; it was left unchanged."),
    }
}

fn current_record<'document>(
    doc: &'document Doc,
    expected: &MeasurementRecord,
) -> Result<(&'document Value, &'document [Value], usize)> {
    ensure!(
        doc.scene.instance_id() == expected.scene_instance,
        "The document was replaced while this measurement was being edited."
    );
    let metadata = page_metadata(doc, expected.page)?;
    let entries = measurement_entries(metadata)?;
    let mut matching = entries.iter().enumerate().filter(|(_, entry)| {
        entry.get("id").and_then(Value::as_str) == Some(expected.measurement.id.as_str())
    });
    let (index, current) = matching
        .next()
        .context("The measurement was deleted while it was being edited.")?;
    ensure!(
        matching.next().is_none(),
        "The measurement identity is ambiguous; no record was changed."
    );
    ensure!(
        current == &expected.raw,
        "The measurement changed while it was being edited. Review the current record or cancel the edit."
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

pub(crate) const MEASUREMENT_DRAG_THRESHOLD_PX: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeasurementGeometry {
    pub(crate) start: [f64; 2],
    pub(crate) end: [f64; 2],
}

impl MeasurementGeometry {
    pub(crate) fn label(&self) -> Result<String> {
        let delta = finite_point(self.end)? - finite_point(self.start)?;
        Ok(distance_label(vector_length(delta)?))
    }
}

impl From<&Measurement> for MeasurementGeometry {
    fn from(measurement: &Measurement) -> Self {
        Self {
            start: measurement.start,
            end: measurement.end,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeasurementProjection {
    page_to_world: Transform2D,
    world_to_page: Transform2D,
    viewport: Viewport,
    screen_size: [f64; 2],
}

impl MeasurementProjection {
    pub(crate) fn new(
        page_to_world: Transform2D,
        viewport: Viewport,
        screen_size: [f64; 2],
    ) -> Result<Self> {
        ensure!(
            page_to_world.is_finite(),
            "The page transform is not finite."
        );
        let determinant = page_to_world.0.matrix2.determinant();
        ensure!(
            determinant.is_finite() && determinant != 0.0,
            "The page transform cannot be inverted."
        );
        let world_to_page = page_to_world.inverse();
        ensure!(
            world_to_page.is_finite(),
            "The inverse page transform is not finite."
        );
        ensure!(
            viewport.zoom.is_finite()
                && viewport.zoom >= f64::EPSILON
                && viewport.center.into_iter().all(f64::is_finite)
                && screen_size
                    .into_iter()
                    .all(|dimension| dimension.is_finite() && dimension > 0.0),
            "The measurement viewport is not valid."
        );
        Ok(Self {
            page_to_world,
            world_to_page,
            viewport,
            screen_size,
        })
    }

    pub(crate) fn page_to_screen(&self, point: [f64; 2]) -> Result<[f64; 2]> {
        let point = finite_point(point)?;
        let world = self.page_to_world.transform_point(point);
        let screen =
            fanta_canvas::world_to_screen(world, &self.viewport, DVec2::from(self.screen_size));
        Ok(finite_point(screen.to_array())?.to_array())
    }

    pub(crate) fn screen_to_page(&self, point: [f64; 2]) -> Result<[f64; 2]> {
        let screen = finite_point(point)?;
        let world =
            fanta_canvas::screen_to_world(screen, &self.viewport, DVec2::from(self.screen_size));
        let point = self.world_to_page.transform_point(world);
        Ok(finite_point(point.to_array())?.to_array())
    }

    pub(crate) fn project(&self, geometry: MeasurementGeometry) -> Result<ScreenMeasurement> {
        let start = self.page_to_screen(geometry.start)?;
        let end = self.page_to_screen(geometry.end)?;
        let midpoint = DVec2::from(start) * 0.5 + DVec2::from(end) * 0.5;
        Ok(ScreenMeasurement {
            start,
            end,
            label_anchor: finite_point(midpoint.to_array())?.to_array(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScreenMeasurement {
    pub(crate) start: [f64; 2],
    pub(crate) end: [f64; 2],
    pub(crate) label_anchor: [f64; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasurementOverlay {
    pub(crate) id: Option<String>,
    pub(crate) screen: ScreenMeasurement,
    pub(crate) label: String,
    pub(crate) selected: bool,
    pub(crate) show_handles: bool,
    pub(crate) preview: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MeasurementHit {
    StartEndpoint,
    EndEndpoint,
    Label,
    Segment,
}

impl ScreenMeasurement {
    pub(crate) fn hit_test(
        &self,
        screen: [f64; 2],
        tolerance_px: f64,
        endpoint_handles: bool,
        label_bounds: Option<Bounds>,
    ) -> Result<Option<MeasurementHit>> {
        ensure!(
            tolerance_px.is_finite() && tolerance_px >= 0.0,
            "The measurement hit tolerance is not valid."
        );
        let screen = finite_point(screen)?;
        let start = finite_point(self.start)?;
        let end = finite_point(self.end)?;
        let start_distance = vector_length(screen - start)?;
        let end_distance = vector_length(screen - end)?;
        if endpoint_handles && start_distance.min(end_distance) <= tolerance_px {
            return Ok(Some(if start_distance <= end_distance {
                MeasurementHit::StartEndpoint
            } else {
                MeasurementHit::EndEndpoint
            }));
        }
        if let Some(bounds) = label_bounds {
            ensure!(
                bounds.is_finite()
                    && bounds.width().is_finite()
                    && bounds.height().is_finite()
                    && bounds.width() >= 0.0
                    && bounds.height() >= 0.0,
                "The measurement label bounds are not valid."
            );
            if bounds.contains_point(screen) {
                return Ok(Some(MeasurementHit::Label));
            }
        }
        let delta = end - start;
        let length = vector_length(delta)?;
        let distance = if length == 0.0 {
            start_distance
        } else {
            let direction = delta / length;
            let along = (screen - start).dot(direction);
            ensure!(
                along.is_finite(),
                "The measurement hit position is too large."
            );
            vector_length(screen - (start + direction * along.clamp(0.0, length)))?
        };
        Ok((distance <= tolerance_px).then_some(MeasurementHit::Segment))
    }
}

fn finite_point(point: [f64; 2]) -> Result<DVec2> {
    ensure!(
        point.into_iter().all(f64::is_finite),
        "The measurement position is not finite."
    );
    Ok(DVec2::from(point))
}

fn vector_length(delta: DVec2) -> Result<f64> {
    let length = delta.x.hypot(delta.y);
    ensure!(length.is_finite(), "The measurement movement is too large.");
    Ok(length)
}

pub(crate) fn constrain_eight_directions(anchor: [f64; 2], position: [f64; 2]) -> Result<[f64; 2]> {
    let anchor = finite_point(anchor)?;
    let delta = finite_point(position)? - anchor;
    let length = vector_length(delta)?;
    if length == 0.0 {
        return Ok(position);
    }
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    let directions = [
        [1.0, 0.0],
        [diagonal, diagonal],
        [0.0, 1.0],
        [-diagonal, diagonal],
        [-1.0, 0.0],
        [-diagonal, -diagonal],
        [0.0, -1.0],
        [diagonal, -diagonal],
    ];
    let octant =
        ((delta.y.atan2(delta.x) / std::f64::consts::FRAC_PI_4).round() as i32).rem_euclid(8);
    let direction = directions
        .get(octant as usize)
        .context("The measurement angle is not valid.")?;
    Ok(finite_point((anchor + DVec2::from(*direction) * length).to_array())?.to_array())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MeasurementOrigin {
    document: DocId,
    scene_instance: u64,
    page: NodeId,
    page_to_world: Transform2D,
}

impl MeasurementOrigin {
    fn capture(doc: &Doc, page: NodeId) -> Result<Self> {
        page_metadata(doc, page)?;
        ensure!(
            doc.active_page() == Some(page),
            "Return to the measurement's page before editing it."
        );
        let page_to_world = doc
            .scene
            .world_transform(page)
            .context("The measurement page transform is unavailable.")?;
        Ok(Self {
            document: doc.id,
            scene_instance: doc.scene.instance_id(),
            page,
            page_to_world,
        })
    }

    fn validate(&self, doc: &Doc) -> Result<()> {
        ensure!(
            doc.id == self.document && doc.scene.instance_id() == self.scene_instance,
            "The measurement draft belongs to a different document instance."
        );
        let current = Self::capture(doc, self.page)?;
        ensure!(
            current.page_to_world == self.page_to_world,
            "The page transform changed during the measurement drag. Cancel the drag and start again."
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MeasurementDragKind {
    StartEndpoint,
    EndEndpoint,
    Move,
}

#[derive(Debug, Clone, PartialEq)]
enum MeasurementDraftKind {
    Create,
    Edit {
        expected: MeasurementRecord,
        kind: MeasurementDragKind,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasurementDraft {
    generation: u64,
    origin: MeasurementOrigin,
    projection: MeasurementProjection,
    kind: MeasurementDraftKind,
    press_screen: [f64; 2],
    press_page: [f64; 2],
    original: MeasurementGeometry,
    preview: MeasurementGeometry,
    dragged: bool,
    released: bool,
    release_valid: bool,
}

impl MeasurementDraft {
    pub(crate) fn preview(&self) -> Option<MeasurementGeometry> {
        self.dragged.then_some(self.preview)
    }

    pub(crate) fn edited_id(&self) -> Option<&str> {
        match &self.kind {
            MeasurementDraftKind::Create => None,
            MeasurementDraftKind::Edit { expected, .. } => Some(&expected.measurement.id),
        }
    }

    pub(crate) fn belongs_to(&self, doc: &Doc) -> bool {
        self.origin.validate(doc).is_ok()
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct MeasurementController {
    draft: Option<MeasurementDraft>,
    generation: u64,
}

impl MeasurementController {
    pub(crate) fn draft(&self) -> Option<&MeasurementDraft> {
        self.draft.as_ref()
    }

    pub(crate) fn has_pending_authoring(&self) -> bool {
        self.draft.is_some()
    }

    pub(crate) fn is_dragging(&self) -> bool {
        self.draft.as_ref().is_some_and(|draft| !draft.released)
    }

    pub(crate) fn cancel(&mut self) -> bool {
        self.draft.take().is_some()
    }

    pub(crate) fn begin_create(
        &mut self,
        doc: &Doc,
        page: NodeId,
        projection: MeasurementProjection,
        press_screen: [f64; 2],
    ) -> Result<()> {
        let press_page = projection.screen_to_page(press_screen)?;
        self.begin(
            doc,
            page,
            projection,
            press_screen,
            MeasurementDraftKind::Create,
            MeasurementGeometry {
                start: press_page,
                end: press_page,
            },
        )
    }

    pub(crate) fn begin_edit(
        &mut self,
        doc: &Doc,
        expected: &MeasurementRecord,
        kind: MeasurementDragKind,
        projection: MeasurementProjection,
        press_screen: [f64; 2],
    ) -> Result<()> {
        current_record(doc, expected)?;
        self.begin(
            doc,
            expected.page,
            projection,
            press_screen,
            MeasurementDraftKind::Edit {
                expected: expected.clone(),
                kind,
            },
            MeasurementGeometry::from(expected.measurement()),
        )
    }

    fn begin(
        &mut self,
        doc: &Doc,
        page: NodeId,
        projection: MeasurementProjection,
        press_screen: [f64; 2],
        kind: MeasurementDraftKind,
        original: MeasurementGeometry,
    ) -> Result<()> {
        ensure!(
            self.draft.is_none(),
            "Finish or cancel the existing measurement drag first."
        );
        let origin = MeasurementOrigin::capture(doc, page)?;
        ensure!(
            projection.page_to_world == origin.page_to_world,
            "The measurement projection does not match its page."
        );
        let press_page = projection.screen_to_page(press_screen)?;
        let generation = self
            .generation
            .checked_add(1)
            .context("The measurement controller needs to be reopened.")?;
        self.generation = generation;
        self.draft = Some(MeasurementDraft {
            generation,
            origin,
            projection,
            kind,
            press_screen,
            press_page,
            original,
            preview: original,
            dragged: false,
            released: false,
            release_valid: false,
        });
        Ok(())
    }

    pub(crate) fn update_pointer(
        &mut self,
        doc: &Doc,
        projection: MeasurementProjection,
        screen: [f64; 2],
        shift: bool,
    ) -> Result<()> {
        let draft = self
            .draft
            .as_ref()
            .context("There is no active measurement drag.")?;
        ensure!(
            !draft.released,
            "This measurement drag has ended. Commit or cancel its preview first."
        );
        draft.origin.validate(doc)?;
        ensure!(
            projection == draft.projection,
            "The viewport changed during the measurement drag. Cancel the drag and start again."
        );
        let pointer_distance =
            vector_length(finite_point(screen)? - DVec2::from(draft.press_screen))?;
        let dragged = draft.dragged || pointer_distance >= MEASUREMENT_DRAG_THRESHOLD_PX;
        if !dragged {
            return Ok(());
        }
        let page_point = projection.screen_to_page(screen)?;
        let delta = finite_point(page_point)? - DVec2::from(draft.press_page);
        let mut preview = draft.original;
        if delta != DVec2::ZERO {
            match &draft.kind {
                MeasurementDraftKind::Create => {
                    preview.end = if shift {
                        constrain_eight_directions(preview.start, page_point)?
                    } else {
                        page_point
                    };
                }
                MeasurementDraftKind::Edit {
                    kind: MeasurementDragKind::StartEndpoint,
                    ..
                } => {
                    let moved =
                        finite_point((DVec2::from(preview.start) + delta).to_array())?.to_array();
                    preview.start = if shift {
                        constrain_eight_directions(preview.end, moved)?
                    } else {
                        moved
                    };
                }
                MeasurementDraftKind::Edit {
                    kind: MeasurementDragKind::EndEndpoint,
                    ..
                } => {
                    let moved =
                        finite_point((DVec2::from(preview.end) + delta).to_array())?.to_array();
                    preview.end = if shift {
                        constrain_eight_directions(preview.start, moved)?
                    } else {
                        moved
                    };
                }
                MeasurementDraftKind::Edit {
                    kind: MeasurementDragKind::Move,
                    ..
                } => {
                    let delta = if shift {
                        DVec2::from(constrain_eight_directions([0.0, 0.0], delta.to_array())?)
                    } else {
                        delta
                    };
                    preview.start =
                        finite_point((DVec2::from(preview.start) + delta).to_array())?.to_array();
                    preview.end =
                        finite_point((DVec2::from(preview.end) + delta).to_array())?.to_array();
                }
            }
        }
        let draft = self
            .draft
            .as_mut()
            .context("There is no active measurement drag.")?;
        draft.preview = preview;
        draft.dragged = true;
        Ok(())
    }

    pub(crate) fn release_pointer(
        &mut self,
        doc: &Doc,
        projection: MeasurementProjection,
        screen: [f64; 2],
        shift: bool,
    ) -> Result<Option<MeasurementCommit>> {
        let result = self.update_pointer(doc, projection, screen, shift);
        if let Some(draft) = self.draft.as_mut() {
            draft.released = true;
            draft.release_valid = result.is_ok();
        }
        result?;
        self.commit_intent(doc)
    }

    pub(crate) fn freeze_after_release_error(&mut self) {
        if let Some(draft) = self.draft.as_mut() {
            draft.released = true;
            draft.release_valid = false;
        }
    }

    // A failed operation must leave the preview available for retry/cancel.
    // The host clears the controller only after application succeeds.
    pub(crate) fn commit_intent(&self, doc: &Doc) -> Result<Option<MeasurementCommit>> {
        let draft = self
            .draft
            .as_ref()
            .context("There is no active measurement drag.")?;
        ensure!(
            draft.released,
            "Release the pointer before committing the measurement."
        );
        ensure!(
            draft.release_valid,
            "The final pointer position could not be accepted. Cancel this preview and start again."
        );
        draft.origin.validate(doc)?;
        if let MeasurementDraftKind::Edit { expected, .. } = &draft.kind {
            current_record(doc, expected)?;
        }
        if !draft.dragged || draft.preview == draft.original {
            return Ok(None);
        }
        distance_px(draft.preview.start, draft.preview.end)?;
        Ok(Some(MeasurementCommit {
            generation: draft.generation,
            origin: draft.origin,
            kind: draft.kind.clone(),
            geometry: draft.preview,
        }))
    }

    pub(crate) fn build_operation(
        &self,
        doc: &Doc,
        intent: &MeasurementCommit,
        author: String,
        created: u64,
    ) -> Result<Option<(String, Operation)>> {
        ensure!(
            self.commit_intent(doc)?.as_ref() == Some(intent),
            "The measurement draft changed or was cancelled before commit."
        );
        intent.build_operation(doc, author, created)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasurementCommit {
    generation: u64,
    origin: MeasurementOrigin,
    kind: MeasurementDraftKind,
    geometry: MeasurementGeometry,
}

impl MeasurementCommit {
    fn build_operation(
        &self,
        doc: &Doc,
        author: String,
        created: u64,
    ) -> Result<Option<(String, Operation)>> {
        self.origin.validate(doc)?;
        match &self.kind {
            MeasurementDraftKind::Create => {
                let measurement =
                    Measurement::new(self.geometry.start, self.geometry.end, author, created)?;
                let operation = create_measurement_op(doc, self.origin.page, &measurement)?;
                Ok(Some((measurement.id, operation)))
            }
            MeasurementDraftKind::Edit { expected, .. } => {
                update_measurement_op(doc, expected, self.geometry.start, self.geometry.end).map(
                    |operation| {
                        operation.map(|operation| (expected.measurement.id.clone(), operation))
                    },
                )
            }
        }
    }
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
        let page = doc.scene.insert(page).expect("insert page");
        doc.add_page(page);
        doc.set_active_page(Some(page));
        (doc, page)
    }

    fn measurement(id: &str) -> Measurement {
        Measurement {
            version: VERSION,
            id: id.into(),
            start: [0.0, 0.0],
            end: [3.0, 4.0],
            author: "Designer".into(),
            created: 42,
        }
    }

    fn metadata(doc: &Doc, page: NodeId) -> &Value {
        &doc.scene.get(page).expect("page").meta
    }

    fn only_record(doc: &Doc, page: NodeId) -> MeasurementRecord {
        let mut records = read_measurements(doc, page).expect("measurement records");
        assert_eq!(records.len(), 1);
        records.pop().expect("one measurement")
    }

    fn apply_metadata(doc: &mut Doc, page: NodeId, new: Value) {
        doc.apply(Operation::SetMeta {
            id: page,
            old: metadata(doc, page).clone(),
            new,
        })
        .expect("apply unrelated metadata edit");
    }

    #[test]
    fn finite_fixed_geometry_has_a_derived_label_and_stable_identity() {
        let measurement = Measurement::new([12.5, 8.0], [15.5, 12.0], "Designer".into(), 42)
            .expect("finite segment");
        assert_eq!(measurement.distance_px().expect("distance"), 5.0);
        assert_eq!(measurement.label().expect("label"), "5 px");
        let encoded = serde_json::to_value(&measurement).expect("encode");
        assert!(encoded.get("distance").is_none());
        assert!(encoded.get("label").is_none());
        let decoded: Measurement = serde_json::from_value(encoded).expect("decode");
        assert_eq!(decoded, measurement);
        assert_eq!(decoded.id, measurement.id);
        let reversed = Measurement::new(measurement.end, measurement.start, "Designer".into(), 42)
            .expect("reversed segment");
        assert_eq!(reversed.label().expect("label"), "5 px");
        let diagonal =
            Measurement::new([0.0, 0.0], [1.0, 1.0], String::new(), 0).expect("diagonal");
        assert_eq!(diagonal.label().expect("label"), "1.41 px");
        let tiny =
            Measurement::new([0.0, 0.0], [0.0001, 0.0], String::new(), 0).expect("short segment");
        assert_eq!(tiny.label().expect("label"), "<0.01 px");
    }

    #[test]
    fn invalid_or_degenerate_geometry_never_produces_an_operation() {
        let (doc, page) = fixture(Value::Null);
        for (start, end) in [
            ([0.0, 0.0], [0.0, 0.0]),
            ([f64::NAN, 0.0], [3.0, 4.0]),
            ([0.0, 0.0], [f64::INFINITY, 1.0]),
            ([-f64::MAX, 0.0], [f64::MAX, 0.0]),
        ] {
            assert!(Measurement::new(start, end, String::new(), 0).is_err());
            let mut invalid = measurement("invalid");
            invalid.start = start;
            invalid.end = end;
            assert!(create_measurement_op(&doc, page, &invalid).is_err());
        }
        assert_eq!(metadata(&doc, page), &Value::Null);
        assert_eq!(doc.history.undo_depth(), 0);
    }

    #[test]
    fn reads_valid_records_and_every_write_preserves_unknown_and_malformed_values() {
        let raw_valid = json!({
            "version": 1, "id": "known", "start": [0, 0], "end": [3, 4],
            "author": "Designer", "created": 42, "future_style": {"color": "violet"}
        });
        let unsupported = json!({"version": 9, "id": "future", "attached": {"layer": "abc"}});
        let malformed = json!({"version": 1, "id": "broken", "start": "opaque"});
        let original = json!({
            "comments": [{"text": "Keep this comment"}], "custom": [1, {"nested": true}],
            "measurements": [unsupported, raw_valid, malformed, null, 17]
        });
        let (mut doc, page) = fixture(original.clone());
        let expected = only_record(&doc, page);
        assert_eq!(expected.measurement().id, "known");
        let update = update_measurement_op(&doc, &expected, [0.0, 0.0], [6.0, 8.0])
            .expect("update")
            .expect("geometry changed");
        doc.apply(update).expect("apply update");
        let after_update = metadata(&doc, page);
        assert_eq!(after_update.get("comments"), original.get("comments"));
        assert_eq!(after_update.get("custom"), original.get("custom"));
        let entries = after_update
            .get(META_KEY)
            .and_then(Value::as_array)
            .expect("raw entries");
        let original_entries = original
            .get(META_KEY)
            .and_then(Value::as_array)
            .expect("original entries");
        for index in [0, 2, 3, 4] {
            assert_eq!(entries.get(index), original_entries.get(index));
        }
        let changed = entries.get(1).expect("edited record");
        assert_eq!(changed.get("future_style"), raw_valid.get("future_style"));
        assert_eq!(changed.get("start"), raw_valid.get("start"));
        assert_eq!(changed.get("author"), raw_valid.get("author"));
        let new_measurement = measurement("added");
        doc.apply(create_measurement_op(&doc, page, &new_measurement).expect("create"))
            .expect("apply create");
        let known = read_measurements(&doc, page)
            .expect("read")
            .into_iter()
            .find(|record| record.measurement().id == "known")
            .expect("known record");
        doc.apply(delete_measurement_op(&doc, &known).expect("delete"))
            .expect("apply delete");
        let entries = metadata(&doc, page)
            .get(META_KEY)
            .and_then(Value::as_array)
            .expect("raw entries");
        assert_eq!(entries.first(), Some(&unsupported));
        assert_eq!(entries.get(1), Some(&malformed));
        assert_eq!(entries.get(2), Some(&Value::Null));
        assert_eq!(entries.get(3), Some(&json!(17)));
        assert_eq!(
            entries.get(4),
            Some(&serde_json::to_value(new_measurement).expect("new record"))
        );
    }

    #[test]
    fn expected_record_guard_merges_unrelated_metadata_and_other_measurement_edits() {
        let (mut doc, page) =
            fixture(json!({"measurements": [measurement("first"), measurement("second")]}));
        let expected = read_measurements(&doc, page)
            .expect("read")
            .into_iter()
            .find(|record| record.measurement().id == "first")
            .expect("first record");
        let mut current = metadata(&doc, page).clone();
        current["comments"] = json!([{"text": "Added from another view"}]);
        current["other"] = json!({"revision": 9});
        current["measurements"][1]["end"] = json!([30, 40]);
        apply_metadata(&mut doc, page, current.clone());
        let operation = update_measurement_op(&doc, &expected, [1.0, 1.0], [4.0, 5.0])
            .expect("unrelated edits merge")
            .expect("changed geometry");
        let Operation::SetMeta { old, .. } = &operation else {
            panic!("metadata operation");
        };
        assert_eq!(old, &current, "history captures current metadata at commit");
        doc.apply(operation).expect("apply update");
        let updated = metadata(&doc, page);
        assert_eq!(updated.get("comments"), current.get("comments"));
        assert_eq!(updated.get("other"), current.get("other"));
        assert_eq!(updated["measurements"][1], current["measurements"][1]);
        assert!(doc.undo().expect("undo measurement edit"));
        assert_eq!(metadata(&doc, page), &current);
    }

    #[test]
    fn changes_to_the_target_record_reject_update_delete_and_stale_noops() {
        let (mut doc, page) = fixture(json!({"measurements": [measurement("target")]}));
        let expected = only_record(&doc, page);
        let mut changed = metadata(&doc, page).clone();
        changed["measurements"][0]["future_field"] = json!("Changed by another client");
        apply_metadata(&mut doc, page, changed.clone());
        let depth = doc.history.undo_depth();
        assert!(update_measurement_op(&doc, &expected, [1.0, 1.0], [4.0, 5.0]).is_err());
        assert!(
            update_measurement_op(
                &doc,
                &expected,
                expected.measurement().start,
                expected.measurement().end
            )
            .is_err()
        );
        assert!(delete_measurement_op(&doc, &expected).is_err());
        assert_eq!(metadata(&doc, page), &changed);
        assert_eq!(doc.history.undo_depth(), depth);
    }

    #[test]
    fn unchanged_geometry_is_a_noop_without_reserializing_raw_coordinates() {
        let original = json!({"measurements": [{
            "version": 1, "id": "target", "start": [0, 0], "end": [3, 4],
            "author": "Designer", "created": 42, "unknown": ["retain"]
        }]});
        let (doc, page) = fixture(original.clone());
        let expected = only_record(&doc, page);
        assert!(
            update_measurement_op(&doc, &expected, [0.0, 0.0], [3.0, 4.0])
                .expect("valid no-op")
                .is_none()
        );
        assert!(update_measurement_op(&doc, &expected, [0.0, 0.0], [0.0, 0.0]).is_err());
        assert_eq!(metadata(&doc, page), &original);
        assert_eq!(doc.history.undo_depth(), 0);
    }

    #[test]
    fn incompatible_containers_and_unsupported_record_versions_are_never_overwritten() {
        for original in [
            json!(17),
            json!("opaque"),
            json!([]),
            json!({"measurements": null}),
            json!({"measurements": {"future": true}}),
        ] {
            let (doc, page) = fixture(original.clone());
            assert!(read_measurements(&doc, page).is_err());
            assert!(create_measurement_op(&doc, page, &measurement("new")).is_err());
            assert_eq!(metadata(&doc, page), &original);
        }
        let (doc, page) = fixture(Value::Null);
        let mut future = measurement("future");
        future.version = 2;
        assert!(create_measurement_op(&doc, page, &future).is_err());
        assert_eq!(metadata(&doc, page), &Value::Null);
    }

    #[test]
    fn duplicate_or_deleted_identities_never_retarget_an_expected_record() {
        let (mut doc, page) = fixture(json!({"measurements": [measurement("target")]}));
        let expected = only_record(&doc, page);
        assert!(create_measurement_op(&doc, page, expected.measurement()).is_err());
        let duplicated =
            json!({"measurements": [measurement("target"), {"id": "target", "version": 9}]});
        apply_metadata(&mut doc, page, duplicated.clone());
        assert!(
            read_measurements(&doc, page)
                .expect("read duplicate entries")
                .is_empty()
        );
        assert!(delete_measurement_op(&doc, &expected).is_err());
        assert_eq!(metadata(&doc, page), &duplicated);
        apply_metadata(&mut doc, page, json!({"comments": ["retain"]}));
        assert!(update_measurement_op(&doc, &expected, [1.0, 1.0], [4.0, 5.0]).is_err());
        assert!(delete_measurement_op(&doc, &expected).is_err());
    }

    #[test]
    fn component_roots_and_replaced_scenes_are_rejected() {
        let (doc, page) = fixture(json!({"measurements": [measurement("target")]}));
        let expected = only_record(&doc, page);
        let mut replaced = Doc::new();
        replaced
            .scene
            .insert(doc.scene.get(page).expect("page").clone())
            .expect("same page in replacement scene");
        replaced.add_page(page);
        assert!(delete_measurement_op(&replaced, &expected).is_err());
        let mut component = CanvasNode::new(NodeData::Group(GroupNode::default()));
        component.name = "Component master".into();
        let component = replaced.scene.insert(component).expect("component root");
        replaced.set_active_page(Some(component));
        assert!(
            create_measurement_op(&replaced, component, &measurement("component measurement"))
                .is_err()
        );
        assert!(read_measurements(&replaced, component).is_err());
    }

    #[test]
    fn create_update_delete_each_have_one_undo_step_and_restore_exact_metadata() {
        let original = json!({"comments": [{"text": "Keep"}], "future": {"value": [1, 2]}});
        let (mut doc, page) = fixture(original.clone());
        let record = measurement("stable-id");
        doc.apply(create_measurement_op(&doc, page, &record).expect("create"))
            .expect("apply create");
        let created = metadata(&doc, page).clone();
        assert_eq!(doc.history.undo_depth(), 1);
        let expected = only_record(&doc, page);
        doc.apply(
            update_measurement_op(&doc, &expected, [2.0, 2.0], [5.0, 6.0])
                .expect("update")
                .expect("changed geometry"),
        )
        .expect("apply update");
        let updated = metadata(&doc, page).clone();
        assert_eq!(doc.history.undo_depth(), 2);
        let expected = only_record(&doc, page);
        doc.apply(delete_measurement_op(&doc, &expected).expect("delete"))
            .expect("apply delete");
        assert_eq!(metadata(&doc, page), &original);
        assert_eq!(doc.history.undo_depth(), 3);
        for expected in [&updated, &created, &original] {
            assert!(doc.undo().expect("undo"));
            assert_eq!(metadata(&doc, page), expected);
        }
        assert_eq!(doc.history.undo_depth(), 0);
        assert_eq!(doc.history.redo_depth(), 3);
        for expected in [&created, &updated, &original] {
            assert!(doc.redo().expect("redo"));
            assert_eq!(metadata(&doc, page), expected);
        }
        assert_eq!(doc.history.redo_depth(), 0);
    }

    fn projection(doc: &Doc, page: NodeId, zoom: f64) -> MeasurementProjection {
        MeasurementProjection::new(
            doc.scene.world_transform(page).expect("page transform"),
            Viewport {
                center: [0.0, 0.0],
                zoom,
            },
            [800.0, 600.0],
        )
        .expect("measurement projection")
    }

    fn close(actual: [f64; 2], expected: [f64; 2]) {
        assert!(
            (DVec2::from(actual) - DVec2::from(expected)).length() < 1e-9,
            "{actual:?} != {expected:?}"
        );
    }

    fn preview(controller: &MeasurementController) -> MeasurementGeometry {
        controller
            .draft()
            .expect("draft")
            .preview()
            .expect("dragged preview")
    }

    #[test]
    fn projection_keeps_page_distance_and_screen_hits_at_every_zoom() {
        let transform = Transform2D::scale_xy(2.0, 0.5)
            .then(&Transform2D::rotation(std::f64::consts::FRAC_PI_2))
            .then(&Transform2D::translation(100.0, -20.0));
        let geometry = MeasurementGeometry {
            start: [10.0, 20.0],
            end: [16.0, 28.0],
        };
        assert_eq!(
            distance_px(geometry.start, geometry.end).expect("page distance"),
            10.0
        );
        for zoom in [0.5, 1.0, 2.0] {
            let projection = MeasurementProjection::new(
                transform,
                Viewport {
                    center: [0.0, 0.0],
                    zoom,
                },
                [800.0, 600.0],
            )
            .expect("projection");
            let screen = projection.project(geometry).expect("project geometry");
            close(screen.start, [400.0 + 90.0 * zoom, 300.0]);
            close(screen.end, [400.0 + 86.0 * zoom, 300.0 + 12.0 * zoom]);
            close(
                projection
                    .screen_to_page(screen.start)
                    .expect("inverse start"),
                geometry.start,
            );
            close(
                projection.screen_to_page(screen.end).expect("inverse end"),
                geometry.end,
            );
            let direction = DVec2::from(screen.end) - DVec2::from(screen.start);
            let normal = DVec2::new(-direction.y, direction.x).normalize();
            let midpoint = DVec2::from(screen.label_anchor);
            assert_eq!(
                screen
                    .hit_test((midpoint + normal * 5.9).to_array(), 6.0, false, None)
                    .expect("hit"),
                Some(MeasurementHit::Segment)
            );
            assert_eq!(
                screen
                    .hit_test((midpoint + normal * 6.1).to_array(), 6.0, false, None)
                    .expect("miss"),
                None
            );
        }
    }

    #[test]
    fn screen_hits_prioritize_selected_endpoints_then_label_then_segment() {
        let screen = ScreenMeasurement {
            start: [10.0, 10.0],
            end: [30.0, 10.0],
            label_anchor: [20.0, 10.0],
        };
        let label = Some(Bounds::from_xywh(8.0, 8.0, 24.0, 18.0));
        assert_eq!(
            screen
                .hit_test([10.0, 10.0], 6.0, true, label)
                .expect("start"),
            Some(MeasurementHit::StartEndpoint)
        );
        assert_eq!(
            screen
                .hit_test([29.0, 10.0], 6.0, true, label)
                .expect("end"),
            Some(MeasurementHit::EndEndpoint)
        );
        assert_eq!(
            screen
                .hit_test([20.0, 24.0], 6.0, true, label)
                .expect("label"),
            Some(MeasurementHit::Label)
        );
        assert_eq!(
            screen
                .hit_test([20.0, 10.0], 6.0, false, None)
                .expect("segment"),
            Some(MeasurementHit::Segment)
        );
        assert_eq!(
            screen
                .hit_test([37.0, 10.0], 6.0, false, None)
                .expect("outside finite segment"),
            None
        );
        assert_eq!(
            screen
                .hit_test([10.0, 10.0], 6.0, false, label)
                .expect("no editable handle"),
            Some(MeasurementHit::Label)
        );
    }

    #[test]
    fn shift_constrains_all_eight_page_directions_without_changing_length() {
        let anchor = [12.0, -7.0];
        for octant in 0..8 {
            let angle = f64::from(octant) * std::f64::consts::FRAC_PI_4;
            let input_angle = angle + 12.0_f64.to_radians();
            let position = [
                anchor[0] + 13.0 * input_angle.cos(),
                anchor[1] + 13.0 * input_angle.sin(),
            ];
            let snapped = constrain_eight_directions(anchor, position).expect("constrained point");
            close(
                snapped,
                [
                    anchor[0] + 13.0 * angle.cos(),
                    anchor[1] + 13.0 * angle.sin(),
                ],
            );
            assert!((distance_px(anchor, snapped).expect("length") - 13.0).abs() < 1e-9);
        }
        assert_eq!(
            constrain_eight_directions(anchor, anchor).expect("zero movement"),
            anchor
        );
        assert!(constrain_eight_directions(anchor, [f64::NAN, 0.0]).is_err());
    }

    #[test]
    fn drag_threshold_is_in_screen_pixels_and_preview_never_writes_metadata() {
        for zoom in [0.5, 1.0, 2.0] {
            let (mut doc, page) = fixture(json!({"comments": ["retain"]}));
            let projection = projection(&doc, page, zoom);
            let before = metadata(&doc, page).clone();
            let mut controller = MeasurementController::default();
            controller
                .begin_create(&doc, page, projection, [400.0, 300.0])
                .expect("press");
            controller
                .update_pointer(&doc, projection, [402.9, 300.0], false)
                .expect("small movement");
            assert!(controller.draft().expect("draft").preview().is_none());
            assert!(
                controller.commit_intent(&doc).is_err(),
                "cannot commit before release"
            );
            let intent = controller
                .release_pointer(&doc, projection, [403.1, 300.0], false)
                .expect("release")
                .expect("drag intent");
            close(preview(&controller).end, [3.1 / zoom, 0.0]);
            assert_eq!(metadata(&doc, page), &before);
            assert_eq!(doc.history.undo_depth(), 0);
            assert!(!controller.is_dragging());
            assert!(controller.has_pending_authoring());
            let (id, operation) = controller
                .build_operation(&doc, &intent, "Designer".into(), 42)
                .expect("build against current doc")
                .expect("create operation");
            doc.apply(operation).expect("apply one commit");
            assert_eq!(doc.history.undo_depth(), 1);
            assert_eq!(only_record(&doc, page).measurement().id, id);
            assert!(controller.cancel());
            assert!(!controller.cancel());
            assert!(doc.undo().expect("undo"));
            assert_eq!(metadata(&doc, page), &before);
            assert!(doc.redo().expect("redo"));
            assert_eq!(only_record(&doc, page).measurement().id, id);
        }
    }

    #[test]
    fn transformed_page_creation_uses_release_position_and_current_shift() {
        let (mut doc, page) = fixture(Value::Null);
        let transform = Transform2D::scale_xy(2.0, 0.5)
            .then(&Transform2D::rotation(std::f64::consts::FRAC_PI_2))
            .then(&Transform2D::translation(100.0, -20.0));
        doc.scene.get_mut(page).expect("page").transform = transform;
        let projection = projection(&doc, page, 2.0);
        let start = [10.0, 20.0];
        let end = [22.0, 24.0];
        let mut controller = MeasurementController::default();
        controller
            .begin_create(
                &doc,
                page,
                projection,
                projection.page_to_screen(start).expect("press"),
            )
            .expect("begin");
        let intent = controller
            .release_pointer(
                &doc,
                projection,
                projection.page_to_screen(end).expect("release"),
                true,
            )
            .expect("release without intermediate move")
            .expect("intent");
        let (id, operation) = controller
            .build_operation(&doc, &intent, "Designer".into(), 42)
            .expect("operation")
            .expect("changed");
        doc.apply(operation).expect("apply");
        let stored = only_record(&doc, page);
        assert_eq!(stored.measurement().id, id);
        close(stored.measurement().start, start);
        close(
            stored.measurement().end,
            [start[0] + 160.0_f64.sqrt(), start[1]],
        );
        assert!(
            (stored.measurement().distance_px().expect("distance") - 160.0_f64.sqrt()).abs() < 1e-9
        );
    }

    #[test]
    fn endpoint_edits_keep_press_offset_and_line_move_keeps_length() {
        for (kind, start, end) in [
            (MeasurementDragKind::StartEndpoint, [10.0, 20.0], [3.0, 4.0]),
            (MeasurementDragKind::EndEndpoint, [0.0, 0.0], [13.0, 24.0]),
            (MeasurementDragKind::Move, [10.0, 20.0], [13.0, 24.0]),
        ] {
            let (mut doc, page) = fixture(json!({"measurements": [measurement("target")]}));
            let expected = only_record(&doc, page);
            let projection = projection(&doc, page, 1.0);
            let mut controller = MeasurementController::default();
            controller
                .begin_edit(&doc, &expected, kind, projection, [405.0, 303.0])
                .expect("press near endpoint or line");
            let intent = controller
                .release_pointer(&doc, projection, [415.0, 323.0], false)
                .expect("release")
                .expect("edit");
            assert_eq!(preview(&controller), MeasurementGeometry { start, end });
            let (_, operation) = controller
                .build_operation(&doc, &intent, "Other editor".into(), 99)
                .expect("build")
                .expect("operation");
            doc.apply(operation).expect("apply");
            let actual = only_record(&doc, page);
            assert_eq!(actual.measurement().start, start);
            assert_eq!(actual.measurement().end, end);
            assert_eq!(actual.measurement().author, "Designer");
            assert_eq!(actual.measurement().created, 42);
            assert_eq!(doc.history.undo_depth(), 1);
            assert!(doc.undo().expect("undo"));
            assert_eq!(
                only_record(&doc, page).measurement(),
                expected.measurement()
            );
        }
    }

    #[test]
    fn original_return_click_only_and_escape_are_exact_noops() {
        let (doc, page) = fixture(json!({"measurements": [measurement("target")]}));
        let original = metadata(&doc, page).clone();
        let projection = projection(&doc, page, 1.0);
        let expected = only_record(&doc, page);
        for kind in [
            MeasurementDragKind::StartEndpoint,
            MeasurementDragKind::EndEndpoint,
            MeasurementDragKind::Move,
        ] {
            let mut controller = MeasurementController::default();
            controller
                .begin_edit(&doc, &expected, kind, projection, [400.0, 300.0])
                .expect("press");
            controller
                .update_pointer(&doc, projection, [420.0, 320.0], true)
                .expect("move");
            assert!(
                controller
                    .release_pointer(&doc, projection, [400.0, 300.0], true)
                    .expect("original return")
                    .is_none()
            );
            assert_eq!(
                preview(&controller),
                MeasurementGeometry::from(expected.measurement())
            );
            assert!(controller.cancel());
        }
        for release in [[400.0, 300.0], [402.9, 300.0]] {
            let mut controller = MeasurementController::default();
            controller
                .begin_create(&doc, page, projection, [400.0, 300.0])
                .expect("press");
            assert!(
                controller
                    .release_pointer(&doc, projection, release, false)
                    .expect("click")
                    .is_none()
            );
            assert!(controller.cancel());
        }
        let mut controller = MeasurementController::default();
        controller
            .begin_create(&doc, page, projection, [400.0, 300.0])
            .expect("press");
        controller
            .update_pointer(&doc, projection, [420.0, 320.0], false)
            .expect("move");
        assert!(controller.cancel());
        assert!(!controller.cancel());
        assert!(
            controller
                .release_pointer(&doc, projection, [420.0, 320.0], false)
                .is_err()
        );
        assert_eq!(metadata(&doc, page), &original);
        assert_eq!(doc.history.undo_depth(), 0);
    }

    #[test]
    fn cancelled_intent_cannot_commit_into_an_identical_replacement_draft() {
        let (doc, page) = fixture(Value::Null);
        let projection = projection(&doc, page, 1.0);
        let mut controller = MeasurementController::default();
        controller
            .begin_create(&doc, page, projection, [400.0, 300.0])
            .expect("press");
        let stale = controller
            .release_pointer(&doc, projection, [410.0, 300.0], false)
            .expect("release")
            .expect("intent");
        assert!(controller.cancel());
        assert!(
            controller
                .build_operation(&doc, &stale, String::new(), 0)
                .is_err()
        );
        controller
            .begin_create(&doc, page, projection, [400.0, 300.0])
            .expect("second press");
        let current = controller
            .release_pointer(&doc, projection, [410.0, 300.0], false)
            .expect("second release")
            .expect("second intent");
        assert!(
            controller
                .build_operation(&doc, &stale, String::new(), 0)
                .is_err()
        );
        assert!(
            controller
                .build_operation(&doc, &current, String::new(), 0)
                .expect("current intent")
                .is_some()
        );
        assert_eq!(metadata(&doc, page), &Value::Null);
        assert_eq!(doc.history.undo_depth(), 0);
    }

    #[test]
    fn failed_commit_freezes_preview_and_preserves_unrelated_metadata_for_retry() {
        let (mut doc, page) = fixture(json!({"measurements": [measurement("target")]}));
        let projection = projection(&doc, page, 1.0);
        let expected = only_record(&doc, page);
        let mut controller = MeasurementController::default();
        controller
            .begin_edit(
                &doc,
                &expected,
                MeasurementDragKind::Move,
                projection,
                [400.0, 300.0],
            )
            .expect("press");
        let intent = controller
            .release_pointer(&doc, projection, [420.0, 320.0], false)
            .expect("release")
            .expect("intent");
        let saved = controller.clone();
        assert!(
            controller
                .update_pointer(&doc, projection, [440.0, 340.0], false)
                .is_err()
        );
        assert_eq!(controller, saved);
        let mut current = metadata(&doc, page).clone();
        current["comments"] = json!(["new unrelated comment"]);
        apply_metadata(&mut doc, page, current.clone());
        let (_, operation) = controller
            .build_operation(&doc, &intent, String::new(), 0)
            .expect("retry against current metadata")
            .expect("operation");
        doc.apply(operation).expect("apply");
        assert_eq!(
            metadata(&doc, page).get("comments"),
            current.get("comments")
        );
        assert!(
            controller
                .build_operation(&doc, &intent, String::new(), 0)
                .is_err(),
            "same-record change blocks repeat application"
        );
        assert_eq!(controller, saved);
        assert!(doc.undo().expect("undo mark edit"));
        assert_eq!(metadata(&doc, page), &current);
    }

    #[test]
    fn collapsed_endpoint_preview_is_retained_but_cannot_be_committed() {
        let (doc, page) = fixture(json!({"measurements": [measurement("target")]}));
        let projection = projection(&doc, page, 1.0);
        let expected = only_record(&doc, page);
        let mut controller = MeasurementController::default();
        controller
            .begin_edit(
                &doc,
                &expected,
                MeasurementDragKind::EndEndpoint,
                projection,
                [403.0, 304.0],
            )
            .expect("press");
        assert!(
            controller
                .release_pointer(&doc, projection, [400.0, 300.0], false)
                .is_err()
        );
        assert_eq!(
            preview(&controller),
            MeasurementGeometry {
                start: [0.0, 0.0],
                end: [0.0, 0.0]
            }
        );
        assert!(!controller.is_dragging());
        assert!(controller.has_pending_authoring());
        assert_eq!(doc.history.undo_depth(), 0);
        assert_eq!(only_record(&doc, page), expected);
        assert!(controller.cancel());
    }

    #[test]
    fn invalid_projection_input_and_changed_origin_preserve_the_draft() {
        let (mut doc, page) = fixture(Value::Null);
        let projection = projection(&doc, page, 1.0);
        let mut controller = MeasurementController::default();
        controller
            .begin_create(&doc, page, projection, [400.0, 300.0])
            .expect("press");
        controller
            .update_pointer(&doc, projection, [410.0, 310.0], false)
            .expect("move");
        let saved = controller.clone();
        assert!(
            controller
                .update_pointer(&doc, projection, [f64::NAN, 300.0], false)
                .is_err()
        );
        assert_eq!(controller, saved);
        let changed_viewport = MeasurementProjection::new(
            Transform2D::IDENTITY,
            Viewport {
                center: [0.0, 0.0],
                zoom: 2.0,
            },
            [800.0, 600.0],
        )
        .expect("changed viewport");
        assert!(
            controller
                .update_pointer(&doc, changed_viewport, [410.0, 310.0], false)
                .is_err()
        );
        assert_eq!(controller, saved);
        let mut replaced = Doc::new();
        replaced.id = doc.id;
        replaced
            .scene
            .insert(doc.scene.get(page).expect("page").clone())
            .expect("replacement page");
        replaced.add_page(page);
        replaced.set_active_page(Some(page));
        assert!(
            controller
                .update_pointer(&replaced, projection, [410.0, 310.0], false)
                .is_err()
        );
        assert_eq!(controller, saved);
        doc.set_active_page(None);
        assert!(
            controller
                .update_pointer(&doc, projection, [410.0, 310.0], false)
                .is_err()
        );
        assert_eq!(controller, saved);
        doc.set_active_page(Some(page));
        doc.apply(Operation::SetTransform {
            id: page,
            old: Transform2D::IDENTITY,
            new: Transform2D::translation(5.0, 0.0),
        })
        .expect("external page movement");
        assert!(
            controller
                .update_pointer(&doc, projection, [410.0, 310.0], false)
                .is_err()
        );
        assert_eq!(controller, saved);
        assert_eq!(metadata(&doc, page), &Value::Null);
        assert_eq!(
            doc.history.undo_depth(),
            1,
            "only external page movement was recorded"
        );
    }

    #[test]
    fn noninvertible_or_nonfinite_projection_and_hit_inputs_are_rejected() {
        for transform in [
            Transform2D::scale_xy(0.0, 1.0),
            Transform2D::translation(f64::NAN, 0.0),
        ] {
            assert!(
                MeasurementProjection::new(transform, Viewport::default(), [800.0, 600.0]).is_err()
            );
        }
        for zoom in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            assert!(
                MeasurementProjection::new(
                    Transform2D::IDENTITY,
                    Viewport {
                        center: [0.0, 0.0],
                        zoom
                    },
                    [800.0, 600.0]
                )
                .is_err()
            );
        }
        for screen_size in [[0.0, 600.0], [800.0, -1.0], [f64::INFINITY, 600.0]] {
            assert!(
                MeasurementProjection::new(Transform2D::IDENTITY, Viewport::default(), screen_size)
                    .is_err()
            );
        }
        let screen = ScreenMeasurement {
            start: [0.0, 0.0],
            end: [10.0, 0.0],
            label_anchor: [5.0, 0.0],
        };
        assert!(screen.hit_test([f64::NAN, 0.0], 6.0, false, None).is_err());
        assert!(screen.hit_test([5.0, 0.0], -1.0, false, None).is_err());
        assert!(
            screen
                .hit_test(
                    [5.0, 0.0],
                    6.0,
                    false,
                    Some(Bounds {
                        min_x: -f64::MAX,
                        min_y: 0.0,
                        max_x: f64::MAX,
                        max_y: 1.0
                    })
                )
                .is_err()
        );
    }

    #[test]
    fn rejected_final_pointer_cannot_commit_the_previous_preview_on_retry() {
        let (doc, page) = fixture(Value::Null);
        let original_projection = projection(&doc, page, 1.0);
        for (release_projection, release_position) in [
            (projection(&doc, page, 2.0), [420.0, 320.0]),
            (original_projection, [f64::NAN, 320.0]),
        ] {
            let mut controller = MeasurementController::default();
            controller
                .begin_create(&doc, page, original_projection, [400.0, 300.0])
                .expect("press");
            controller
                .update_pointer(&doc, original_projection, [410.0, 310.0], false)
                .expect("accepted preview");
            let accepted = preview(&controller);
            assert!(
                controller
                    .release_pointer(&doc, release_projection, release_position, false)
                    .is_err()
            );
            assert_eq!(preview(&controller), accepted);
            assert!(!controller.is_dragging());
            assert!(controller.commit_intent(&doc).is_err());
            assert!(
                controller
                    .update_pointer(&doc, original_projection, [420.0, 320.0], false)
                    .is_err()
            );
            assert!(controller.commit_intent(&doc).is_err());
            assert_eq!(doc.history.undo_depth(), 0);
            assert!(controller.cancel());
        }
    }

    fn project_file_snapshot(
        root: &std::path::Path,
    ) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        let mut directories = vec![root.to_path_buf()];
        let mut files = std::collections::BTreeMap::new();
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(&directory).expect("read project directory") {
                let entry = entry.expect("project directory entry");
                let path = entry.path();
                let kind = entry.file_type().expect("entry type");
                if kind.is_dir() {
                    directories.push(path);
                } else if kind.is_file() {
                    let relative = path
                        .strip_prefix(root)
                        .expect("project-relative file")
                        .to_path_buf();
                    files.insert(relative, std::fs::read(path).expect("read authored bytes"));
                }
            }
        }
        files
    }

    #[test]
    fn measurements_and_mixed_metadata_survive_fnx_reopen_and_byte_identical_second_save() {
        let unknown = json!({"version": 9, "id": "future-measurement", "attached": {"layer": "future-layer"}, "style": [0.1 + 0.2, null]});
        let malformed = json!({"version": 1, "id": "incomplete", "start": "opaque"});
        let (mut doc, first_page) = fixture(json!({
            "comments": [{"future_comment": {"replies": [null, "preserve"]}}],
            "measurements": [unknown, malformed, null, 17],
            "future_page_settings": {"nested": [true, {"value": "retain"}]}
        }));
        doc.scene.get_mut(first_page).expect("first page").name = "Page 1".into();
        let (_, comment) = crate::comments::add_comment_op(
            &doc,
            first_page,
            [11.0, 12.0],
            "Keep this discussion — 測定",
        )
        .expect("real comment operation");
        doc.apply(comment).expect("apply comment");
        let mut second_page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        second_page_node.name = "Page 2".into();
        second_page_node.index = doc.scene.next_child_index(None);
        second_page_node.transform =
            Transform2D::scale_xy(2.0, 0.5).then(&Transform2D::translation(123.0, -45.0));
        second_page_node.meta = json!({"comments": [{"future": "opaque comment"}], "measurements": ["unknown record"], "other": [1, 2, 3]});
        let second_page = doc.scene.insert(second_page_node).expect("second page");
        doc.add_page(second_page);

        let first = Measurement::new(
            [f64::from(21.762165_f32), 0.1 + 0.2],
            [f64::from(935.65722_f32), -18.25],
            "First editor".into(),
            41,
        )
        .expect("first measurement");
        let second = Measurement::new([-12.5, 9.25], [-9.5, 13.25], "Second editor".into(), 42)
            .expect("second measurement");
        doc.apply(create_measurement_op(&doc, first_page, &first).expect("create on first page"))
            .expect("apply first mark");
        doc.apply(
            create_measurement_op(&doc, second_page, &second).expect("create on second page"),
        )
        .expect("apply second mark");
        let first_metadata = metadata(&doc, first_page).clone();
        let second_metadata = metadata(&doc, second_page).clone();
        let comments = crate::comments::read_comments(&doc, first_page);
        assert_eq!(comments.len(), 1);
        doc.set_active_page(Some(second_page));
        doc.selection.select_only(first_page);
        let directory = tempfile::tempdir().expect("measurement project directory");
        fanta_format::write_project_tree(
            directory.path(),
            &doc,
            &std::collections::BTreeMap::new(),
        )
        .expect("save actual FNX project");
        let before = project_file_snapshot(directory.path());
        assert_eq!(
            before
                .keys()
                .filter(|path| path.extension().is_some_and(|extension| extension == "fnx"))
                .count(),
            2
        );
        for (page, record) in [(first_page, &first), (second_page, &second)] {
            let source =
                fanta_format::locate_page_source(directory.path(), page).expect("page FNX source");
            let source = std::fs::read_to_string(source).expect("read saved FNX");
            assert!(source.contains("measurements"));
            assert!(source.contains(&record.id));
            assert!(source.contains("comments"));
        }

        let (reopened, assets) =
            fanta_format::read_project_tree(directory.path()).expect("reopen from page FNX");
        assert_eq!(reopened.id, doc.id);
        assert_eq!(reopened.pages(), &[first_page, second_page]);
        assert_eq!(only_record(&reopened, first_page).measurement(), &first);
        assert_eq!(only_record(&reopened, second_page).measurement(), &second);
        assert_eq!(
            only_record(&reopened, second_page)
                .measurement()
                .label()
                .expect("derived label"),
            "5 px"
        );
        assert_eq!(metadata(&reopened, first_page), &first_metadata);
        assert_eq!(metadata(&reopened, second_page), &second_metadata);
        assert_eq!(
            crate::comments::read_comments(&reopened, first_page),
            comments
        );
        assert_eq!(
            reopened
                .scene
                .get(second_page)
                .expect("restored page")
                .transform,
            doc.scene.get(second_page).expect("original page").transform
        );
        assert!(
            reopened.selection.is_empty(),
            "selection remains session state"
        );
        assert_eq!(
            reopened.history.undo_depth(),
            0,
            "history is not promised across reopen"
        );
        let report = fanta_format::write_project_tree(directory.path(), &reopened, &assets)
            .expect("second save after reopen");
        assert!(
            report.written.is_empty(),
            "unchanged Save rewrote {:?}",
            report.written
        );
        assert!(report.removed.is_empty());
        assert_eq!(
            project_file_snapshot(directory.path()),
            before,
            "every authored file and filename stays byte-identical"
        );
    }

    #[test]
    fn moving_and_deleting_underlying_art_never_changes_page_fixed_measurements() {
        let (mut doc, page) = fixture(json!({"comments": ["keep"]}));
        let mut art = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
            0.0,
            0.0,
            100.0,
            50.0,
            fanta_doc::Color::BLACK,
        )));
        art.parent = Some(page);
        art.transform = Transform2D::translation(10.0, 20.0);
        let art_id = doc.scene.insert(art).expect("underlying art");
        let measurement = Measurement::new([10.0, 20.0], [110.0, 70.0], "Designer".into(), 42)
            .expect("page-fixed mark");
        doc.apply(create_measurement_op(&doc, page, &measurement).expect("create mark"))
            .expect("apply mark");
        let before = metadata(&doc, page).clone();
        doc.apply(Operation::SetTransform {
            id: art_id,
            old: Transform2D::translation(10.0, 20.0),
            new: Transform2D::scale(3.0).then(&Transform2D::translation(900.0, -800.0)),
        })
        .expect("move and scale underlying art");
        assert_eq!(only_record(&doc, page).measurement(), &measurement);
        assert_eq!(metadata(&doc, page), &before);
        let moved_art = doc.scene.get(art_id).expect("moved art").clone();
        doc.apply(Operation::DeleteSubtree {
            snapshot: vec![moved_art.clone()],
        })
        .expect("delete underlying art");
        assert!(!doc.scene.contains(art_id));
        assert_eq!(only_record(&doc, page).measurement(), &measurement);
        assert_eq!(metadata(&doc, page), &before);
        assert!(doc.undo().expect("undo art deletion"));
        assert_eq!(doc.scene.get(art_id), Some(&moved_art));
        assert_eq!(only_record(&doc, page).measurement(), &measurement);
        assert!(doc.redo().expect("redo art deletion"));
        assert_eq!(only_record(&doc, page).measurement(), &measurement);
    }
}
