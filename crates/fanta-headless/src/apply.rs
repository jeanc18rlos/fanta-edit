//! The design-op applier: turns agent [`DesignOp`]s into document
//! operations, one undoable batch at a time. The editor's agent surface and
//! the headless engine both apply ops through it, so an op lands the same way
//! in the app and in the cloud.

use std::collections::HashSet;

use anyhow::{Context as _, Result, anyhow, bail};
use fanta_design_ops::{
    AlignContent, AlignEdge, CellAlignment, CrossAxisAlignment, DesignComponentPropertyKind,
    DesignEffect, DesignImageFit, DesignNodeType, DesignOp, DesignPaint, DesignShadow, DesignShape,
    DesignStrokeCap, DesignStrokeJoin, DesignVariableType, DistributeAxis, GridTrackSpec,
    HorizontalConstraint, LayerPosition, LayoutDirection, LayoutSizing, MainAxisAlignment,
    NamedLayerPosition, StrokeAlignment, TextAlignment, TextSizing, VariableBindingProperty,
    VerticalConstraint,
};
use fanta_doc::{
    AssetId, AutoLayout, AxisSizing, BitmapNode, BoundProp, CanvasNode, Color, ComponentId,
    CounterAlign, Doc, Fill, GridAlign, GridLayout, GridTrack, GroupNode, ImageFitMode, IndexKey,
    InstanceNode, LayoutMode, Mode, ModeId, ModeScope, NodeData, NodeFlags, NodeId, Operation,
    PathData, PrimaryAlign, Shadow, ShadowKind, Stroke, StrokeAlign, StrokeCap, StrokeJoin,
    TextAlign, TextAutoResize, TextNode, Transform2D, UnitInterval, VarValue, Variable,
    VariableCollection, VariableCollectionId, VariableId, VariableType, VectorNode,
};
use serde_json::{Value, json};

use crate::assets::{ImageStore, decode_image_source};
use crate::clipboard::{create_operations, duplicate_operations};
use crate::edit_operations::{
    DEFAULT_FILL_COLOR, blurs_operations, create_component_operations, default_shadow,
    effects_operations, parse_color, replace_data_operation, resize_operations,
    rotation_operations, set_corner_radius, stroke_list_mut,
};
use crate::scene::{find_collection, union_bounds};
use crate::structure::{frame_selection_operations, group_operations, ungroup_operations};

/// Parse an agent's node id (the `n_…` form).
pub fn parse_node_id(raw: &str) -> Result<NodeId> {
    raw.parse::<NodeId>()
        .map_err(|_| anyhow!("`{raw}` is not a valid node id"))
}

fn required_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 512 {
        bail!("names must contain 1–512 bytes");
    }
    Ok(name)
}

fn find_variable<'a>(doc: &'a Doc, key: &str) -> Result<&'a Variable> {
    if let Ok(id) = key.parse::<VariableId>() {
        return doc
            .variables
            .variables
            .get(&id)
            .context("variable id does not exist");
    }
    let mut matches = doc
        .variables
        .variables
        .values()
        .filter(|variable| variable.name == key);
    let variable = matches.next().context("variable name does not exist")?;
    if matches.next().is_some() {
        bail!("ambiguous variable name; use its exact id");
    }
    Ok(variable)
}

fn find_mode(collection: &VariableCollection, key: &str) -> Result<ModeId> {
    if let Ok(id) = key.parse::<ModeId>() {
        if collection.has_mode(id) {
            return Ok(id);
        }
        bail!("mode does not belong to this collection");
    }
    let mut matches = collection.modes.iter().filter(|mode| mode.name == key);
    let mode = matches
        .next()
        .context("mode name does not exist in this collection")?;
    if matches.next().is_some() {
        bail!("ambiguous mode name; use its exact id");
    }
    Ok(mode.id)
}

fn find_component_property<'a>(
    definition: &'a fanta_doc::ComponentDef,
    key: &str,
) -> Result<&'a fanta_doc::ComponentPropDef> {
    if let Ok(id) = key.parse::<fanta_doc::ComponentPropId>() {
        return definition
            .props
            .iter()
            .find(|property| property.id == id)
            .context("property id does not belong to this component");
    }
    let mut matches = definition
        .props
        .iter()
        .filter(|property| property.name == key);
    let property = matches
        .next()
        .context("component property name does not exist")?;
    if matches.next().is_some() {
        bail!("ambiguous component property name; use its exact id");
    }
    Ok(property)
}

fn variable_type(kind: DesignVariableType) -> VariableType {
    match kind {
        DesignVariableType::Color => VariableType::Color,
        DesignVariableType::Float => VariableType::Float,
        DesignVariableType::String => VariableType::String,
        DesignVariableType::Boolean => VariableType::Boolean,
        DesignVariableType::Typography => VariableType::Typography,
        DesignVariableType::Enum => VariableType::Enum,
        DesignVariableType::Image => VariableType::Image,
    }
}

/// Trim and validate an enum's option list: non-empty, no blank or repeated names.
fn enum_options(options: &[String]) -> Result<Vec<String>> {
    let options: Vec<String> = options
        .iter()
        .map(|option| option.trim().to_owned())
        .collect();
    if options.is_empty() {
        bail!("an enum variable needs at least one option");
    }
    if options.iter().any(String::is_empty) {
        bail!("enum options cannot be blank");
    }
    let mut seen = HashSet::new();
    if let Some(repeated) = options.iter().find(|option| !seen.insert(option.as_str())) {
        bail!("enum option {repeated:?} is listed twice");
    }
    Ok(options)
}

/// `options` are the enum's choices when `kind` is an enum (empty = unchecked).
fn parse_variable_value(
    doc: &Doc,
    kind: VariableType,
    options: &[String],
    value: &Value,
) -> Result<VarValue> {
    if let Some(alias) = value.get("alias").and_then(Value::as_str) {
        let target = find_variable(doc, alias)?;
        if target.ty != kind {
            bail!("alias target has a different variable type");
        }
        return Ok(VarValue::Alias {
            variable: target.id,
        });
    }
    Ok(match kind {
        VariableType::Color => VarValue::Color {
            value: parse_fill_color(value.as_str().context("color requires a hex string")?)?,
        },
        VariableType::Float => {
            let value = value
                .as_f64()
                .filter(|value| value.is_finite())
                .context("float requires a finite number")?;
            VarValue::Float { value }
        }
        VariableType::String => VarValue::String {
            value: value.as_str().context("string requires text")?.to_owned(),
        },
        VariableType::Boolean => VarValue::Boolean {
            value: value.as_bool().context("boolean requires true or false")?,
        },
        VariableType::Typography => {
            let style: fanta_doc::TextStyle = serde_json::from_value(value.clone())
                .context("typography requires a TextStyle object")?;
            if style.font_family.trim().is_empty()
                || !(style.size_px.is_finite() && style.size_px > 0.0)
                || !(style.line_height.is_finite() && style.line_height > 0.0)
                || !style.letter_spacing.is_finite()
            {
                bail!("typography requires a font family and positive finite size/line height");
            }
            VarValue::TextStyle { value: style }
        }
        VariableType::Enum => {
            let option = value
                .as_str()
                .map(str::trim)
                .filter(|option| !option.is_empty())
                .context("enum requires one of the variable's option names")?;
            if !options.is_empty() && !options.iter().any(|candidate| candidate == option) {
                bail!("enum value {option:?} is not one of the options {options:?}");
            }
            VarValue::Enum {
                value: option.to_owned(),
            }
        }
        VariableType::Image => {
            let asset = match value {
                Value::Null => None,
                Value::String(id) => Some(
                    id.parse::<AssetId>()
                        .context("image requires a project asset id such as a_…, or null")?,
                ),
                _ => bail!("image requires a project asset id such as a_…, or null"),
            };
            VarValue::Image { value: asset }
        }
    })
}

fn binding_property(property: VariableBindingProperty) -> BoundProp {
    match property {
        VariableBindingProperty::FillColor { index } => BoundProp::FillColor { index },
        VariableBindingProperty::StrokeColor { index } => BoundProp::StrokeColor { index },
        VariableBindingProperty::StrokeWidth { index } => BoundProp::StrokeWidth { index },
        VariableBindingProperty::CornerRadius => BoundProp::CornerRadius,
        VariableBindingProperty::Opacity => BoundProp::Opacity,
        VariableBindingProperty::Visible => BoundProp::Visible,
        VariableBindingProperty::TextContent => BoundProp::TextContent,
        VariableBindingProperty::TextStyle => BoundProp::TextStyle,
        VariableBindingProperty::ClipWidth => BoundProp::ClipWidth,
        VariableBindingProperty::ClipHeight => BoundProp::ClipHeight,
        VariableBindingProperty::ImageFill { index } => BoundProp::ImageFill { index },
    }
}

fn layout_sizing(sizing: LayoutSizing) -> AxisSizing {
    match sizing {
        LayoutSizing::Fixed => AxisSizing::Fixed,
        LayoutSizing::Hug => AxisSizing::Hug,
    }
}

/// A `set_grid_layout` track: px, `"<n>fr"` (or `"fr"`), `"<n>px"`, or
/// `"auto"`/`"hug"`.
fn grid_track(spec: &GridTrackSpec) -> Result<GridTrack> {
    let fixed = |size: f64| {
        if size.is_finite() && size >= 0.0 {
            Ok(GridTrack::Fixed { size })
        } else {
            Err(anyhow!("a px track must be finite and non-negative"))
        }
    };
    match spec {
        GridTrackSpec::Px(size) => fixed(*size),
        GridTrackSpec::Keyword(word) => {
            let word = word.trim().to_ascii_lowercase();
            if matches!(word.as_str(), "auto" | "hug") {
                return Ok(GridTrack::Hug);
            }
            if let Some(fr) = word.strip_suffix("fr") {
                let fr = match fr.trim() {
                    "" => 1.0,
                    fr => fr
                        .parse::<f64>()
                        .map_err(|_| anyhow!("bad fr track {word:?}"))?,
                };
                if !(fr.is_finite() && fr > 0.0) {
                    bail!("an fr track must be positive");
                }
                return Ok(GridTrack::Flex { fr });
            }
            match word
                .strip_suffix("px")
                .unwrap_or(&word)
                .trim()
                .parse::<f64>()
            {
                Ok(size) => fixed(size),
                Err(_) => {
                    bail!("unknown grid track {word:?}: use a px number, \"<n>fr\" or \"auto\"")
                }
            }
        }
    }
}

fn grid_alignment(alignment: CellAlignment) -> GridAlign {
    match alignment {
        CellAlignment::Start => GridAlign::Start,
        CellAlignment::Center => GridAlign::Center,
        CellAlignment::End => GridAlign::End,
    }
}

fn counter_alignment(alignment: CrossAxisAlignment) -> CounterAlign {
    match alignment {
        CrossAxisAlignment::Start => CounterAlign::Start,
        CrossAxisAlignment::Center => CounterAlign::Center,
        CrossAxisAlignment::End => CounterAlign::End,
        CrossAxisAlignment::Stretch => CounterAlign::Stretch,
        CrossAxisAlignment::Baseline => CounterAlign::Baseline,
    }
}

/// What a batch changed, for the editor to report to its panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchEffect {
    None,
    Selection,
    Content,
}

/// A batch's result: the `batch_design` JSON and what the batch changed.
pub struct BatchOutcome {
    pub value: Value,
    pub effect: BatchEffect,
    /// The image assets a committed batch added to the project.
    pub ingested_assets: Vec<AssetId>,
}

/// What one applied op affected, for dirty/selection tracking and reporting.
enum Applied {
    Content {
        created: Option<String>,
        /// Extra op-specific facts merged into the op's status entry (e.g.
        /// the children an `ungroup` freed).
        detail: Option<Value>,
    },
    Selection,
    Viewport,
    Nothing,
}

impl Applied {
    fn created(id: NodeId) -> Self {
        Self::Content {
            created: Some(id.to_string()),
            detail: None,
        }
    }

    fn changed() -> Self {
        Self::Content {
            created: None,
            detail: None,
        }
    }
}

/// Apply the batch inside one history transaction: all content ops commit as
/// a single undo step, and any failure rolls the content back (selection and
/// viewport changes are not transactional). Image assets ingested by
/// `create_image` ops are removed again when the batch rolls back, and nodes
/// the batch removed leave the selection only once it has committed, so a
/// rolled-back batch leaves the selection exactly as it found it. Ops without
/// a parent place their nodes in `default_container` (the editor's active
/// page).
pub fn apply_batch(
    doc: &mut Doc,
    assets: &mut dyn ImageStore,
    ops: &[DesignOp],
    label: &str,
    default_container: Option<NodeId>,
) -> BatchOutcome {
    let mut progress = BatchProgress::new(default_container);
    if !ops.iter().all(design_operation_is_presence) {
        doc.history.begin(label, &mut doc.scene);
        progress.mark_history_started();
    }
    for (index, operation) in ops.iter().enumerate() {
        progress.apply(doc, assets, index, operation);
        if progress.failed() {
            break;
        }
    }
    progress.finish(doc, assets, ops.len())
}

/// A batch applied one op at a time, so the editor can paint each step of an
/// agent's edit inside the same undo transaction.
#[derive(Default)]
pub struct BatchProgress {
    default_container: Option<NodeId>,
    created: Vec<String>,
    statuses: Vec<Value>,
    ingested_assets: Vec<AssetId>,
    removed_nodes: HashSet<NodeId>,
    content_changed: bool,
    selection_changed: bool,
    failure: Option<(usize, String)>,
    history_started: bool,
}

impl BatchProgress {
    pub fn new(default_container: Option<NodeId>) -> Self {
        Self {
            default_container,
            ..Self::default()
        }
    }

    /// Record that the caller opened the batch's history transaction, so
    /// [`finish`](Self::finish) commits it or rolls it back.
    pub fn mark_history_started(&mut self) {
        self.history_started = true;
    }

    pub fn failed(&self) -> bool {
        self.failure.is_some()
    }

    /// The node the last applied op created, if it created one.
    pub fn last_created(&self) -> Option<NodeId> {
        self.statuses
            .last()
            .and_then(|status| status.get("created"))
            .and_then(Value::as_str)
            .and_then(|id| id.parse().ok())
    }

    /// The assets ingested so far, for a caller that rolls the batch back
    /// itself.
    pub fn take_ingested_assets(&mut self) -> Vec<AssetId> {
        std::mem::take(&mut self.ingested_assets)
    }

    pub fn apply(
        &mut self,
        doc: &mut Doc,
        assets: &mut dyn ImageStore,
        index: usize,
        operation: &DesignOp,
    ) {
        match apply_one(
            doc,
            assets,
            &mut self.ingested_assets,
            &mut self.removed_nodes,
            operation,
            self.default_container,
        ) {
            Ok(Applied::Content { created, detail }) => {
                self.content_changed = true;
                let mut status = json!({ "index": index, "status": "ok" });
                if let Some(id) = created {
                    status["created"] = json!(id);
                    self.created.push(id);
                }
                if let (Some(Value::Object(detail)), Some(status)) =
                    (detail, status.as_object_mut())
                {
                    status.extend(detail);
                }
                self.statuses.push(status);
            }
            Ok(Applied::Selection) => {
                self.selection_changed = true;
                self.statuses
                    .push(json!({ "index": index, "status": "ok" }));
            }
            Ok(Applied::Viewport | Applied::Nothing) => {
                self.statuses
                    .push(json!({ "index": index, "status": "ok" }));
            }
            Err(error) => self.failure = Some((index, format!("{error:#}"))),
        }
    }

    pub fn finish(
        &mut self,
        doc: &mut Doc,
        assets: &mut dyn ImageStore,
        operation_count: usize,
    ) -> BatchOutcome {
        match self.failure.take() {
            None => {
                if self.history_started {
                    doc.history.commit(&mut doc.scene);
                }
                if doc
                    .selection
                    .iter()
                    .any(|selected| self.removed_nodes.contains(selected))
                {
                    let surviving: Vec<NodeId> = doc
                        .selection
                        .iter()
                        .copied()
                        .filter(|selected| !self.removed_nodes.contains(selected))
                        .collect();
                    doc.selection.replace_with(surviving);
                }
                BatchOutcome {
                    value: json!({ "applied": true, "created": self.created, "ops": self.statuses }),
                    effect: if self.content_changed {
                        BatchEffect::Content
                    } else if self.selection_changed {
                        BatchEffect::Selection
                    } else {
                        BatchEffect::None
                    },
                    ingested_assets: std::mem::take(&mut self.ingested_assets),
                }
            }
            Some((index, mut message)) => {
                if self.history_started
                    && let Err(abort_error) = doc.abort_transaction()
                {
                    message = format!("{message}; rolling back also failed: {abort_error}");
                }
                for asset in self.ingested_assets.drain(..) {
                    assets.remove(asset);
                }
                self.statuses
                    .push(json!({ "index": index, "status": "failed", "error": message }));
                for skipped in index + 1..operation_count {
                    self.statuses
                        .push(json!({ "index": skipped, "status": "skipped" }));
                }
                BatchOutcome {
                    value: json!({ "applied": false,
                        "error": format!("op {index} failed; the batch was rolled back"), "ops": self.statuses }),
                    effect: if self.selection_changed {
                        BatchEffect::Selection
                    } else {
                        BatchEffect::None
                    },
                    ingested_assets: Vec::new(),
                }
            }
        }
    }

    /// Roll the batch back whether or not it failed: a dry run that checks
    /// the ops without keeping them. A batch that would apply reports
    /// `applied: true` with the ids it created, which no longer exist.
    /// Selection and viewport changes are the caller's to restore.
    pub fn roll_back(
        &mut self,
        doc: &mut Doc,
        assets: &mut dyn ImageStore,
        operation_count: usize,
    ) -> BatchOutcome {
        if self.failure.is_some() {
            return self.finish(doc, assets, operation_count);
        }
        let mut value = json!({ "applied": true, "created": self.created, "ops": self.statuses });
        if self.history_started
            && let Err(error) = doc.abort_transaction()
        {
            value = json!({ "applied": false,
                "error": format!("rolling back the checked batch failed: {error}"), "ops": self.statuses });
        }
        for asset in self.ingested_assets.drain(..) {
            assets.remove(asset);
        }
        BatchOutcome {
            value,
            effect: BatchEffect::None,
            ingested_assets: Vec::new(),
        }
    }
}

/// Apply one op into the caller's open history transaction, without a
/// batch's commit or rollback: the building block for callers that gather
/// several edits into one undo step. Returns the node the op created, if any.
/// When the op fails, the images it ingested are removed again, but the
/// operations it applied before failing stay in the open transaction for the
/// caller to roll back with [`Doc::rollback_open_transaction`].
pub fn apply_design_op(
    doc: &mut Doc,
    assets: &mut dyn ImageStore,
    operation: &DesignOp,
    default_container: Option<NodeId>,
) -> Result<Option<NodeId>> {
    let mut ingested_assets = Vec::new();
    let mut removed_nodes = HashSet::new();
    let applied = match apply_one(
        doc,
        assets,
        &mut ingested_assets,
        &mut removed_nodes,
        operation,
        default_container,
    ) {
        Ok(applied) => applied,
        Err(error) => {
            for asset in ingested_assets {
                assets.remove(asset);
            }
            return Err(error);
        }
    };
    if doc
        .selection
        .iter()
        .any(|selected| removed_nodes.contains(selected))
    {
        let surviving: Vec<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|selected| !removed_nodes.contains(selected))
            .collect();
        doc.selection.replace_with(surviving);
    }
    match applied {
        Applied::Content {
            created: Some(id), ..
        } => parse_node_id(&id).map(Some),
        _ => Ok(None),
    }
}

pub fn design_operation_is_presence(operation: &DesignOp) -> bool {
    matches!(
        operation,
        DesignOp::SetViewport { .. } | DesignOp::Select { .. }
    )
}

pub fn design_operation_is_variable(operation: &DesignOp) -> bool {
    matches!(
        operation,
        DesignOp::CreateVariableCollection { .. }
            | DesignOp::AddVariableMode { .. }
            | DesignOp::CreateVariable { .. }
            | DesignOp::SetVariableValue { .. }
            | DesignOp::SetVariableOptions { .. }
            | DesignOp::SetVariableMode { .. }
    )
}

pub fn design_operation_node(doc: &Doc, operation: &DesignOp) -> Option<NodeId> {
    let id = match operation {
        DesignOp::SetProps { id, .. }
        | DesignOp::SetStroke { id, .. }
        | DesignOp::SetShadow { id, .. }
        | DesignOp::SetTextStyle { id, .. }
        | DesignOp::SetAutoLayout { id, .. }
        | DesignOp::SetLayoutChild { id, .. }
        | DesignOp::BindVariable { id, .. }
        | DesignOp::UnbindVariable { id, .. }
        | DesignOp::SetIndex { id, .. }
        | DesignOp::Rotate { id, .. }
        | DesignOp::Ungroup { id }
        | DesignOp::Duplicate { id, .. }
        | DesignOp::CreateComponent { id }
        | DesignOp::BindComponentProperty { id, .. }
        | DesignOp::SetInstanceProperty { id, .. }
        | DesignOp::Reparent { id, .. }
        | DesignOp::Delete { id } => Some(id),
        DesignOp::Align { ids, .. }
        | DesignOp::Distribute { ids, .. }
        | DesignOp::Group { ids, .. }
        | DesignOp::FrameSelection { ids, .. }
        | DesignOp::CombineVariants { ids, .. }
        | DesignOp::Componentize { ids }
        | DesignOp::Select { ids } => ids.first(),
        DesignOp::CreateComponentProperty { component, .. } => {
            return resolve_component(doc, component)
                .ok()
                .and_then(|component| doc.components.def(component))
                .map(|definition| definition.root);
        }
        _ => None,
    };
    id.and_then(|id| id.parse::<NodeId>().ok())
        .filter(|id| doc.scene.get(*id).is_some())
}

/// Apply one op. Assets it ingests go into `ingested_assets` and nodes it
/// removes from the scene into `removed_nodes`, both for [`apply_batch`] to
/// settle once the whole batch has committed or rolled back.
fn apply_one(
    doc: &mut Doc,
    assets: &mut dyn ImageStore,
    ingested_assets: &mut Vec<AssetId>,
    removed_nodes: &mut HashSet<NodeId>,
    op: &DesignOp,
    default_container: Option<NodeId>,
) -> Result<Applied> {
    match op {
        DesignOp::CreateImage {
            source,
            parent,
            name,
            x,
            y,
            width,
            height,
            meta,
        } => {
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let bytes = decode_image_source(source)?;
            let (asset, natural_size, inserted) = assets.add_image_tracked(bytes)?;
            // Track before any fallible step so a later failure in this batch
            // rolls the asset back out of the stores too.
            if inserted {
                ingested_assets.push(asset);
            }

            let natural_width = f64::from(natural_size[0].max(1));
            let natural_height = f64::from(natural_size[1].max(1));
            let (width, height) = match (width, height) {
                (Some(width), Some(height)) => (*width, *height),
                (Some(width), None) => (*width, width * natural_height / natural_width),
                (None, Some(height)) => (height * natural_width / natural_height, *height),
                (None, None) => (natural_width, natural_height),
            };
            if !(width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0) {
                bail!("width and height must be positive");
            }

            let mut node = CanvasNode::new(NodeData::Bitmap(BitmapNode {
                asset,
                natural_size,
                local_size: [width, height],
                crop: None,
                fit: ImageFitMode::Fill,
                tint: None,
            }));
            if let Some(name) = name {
                node.name = name.clone();
            }
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform = Transform2D::translation(*x, *y).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::create_node(node))
                .context("creating the image node")?;
            if let Some(meta) = meta
                && !meta.is_null()
            {
                doc.apply(Operation::SetMeta {
                    id,
                    old: Value::Null,
                    new: meta.clone(),
                })
                .context("recording the image node's metadata")?;
            }
            Ok(Applied::created(id))
        }
        DesignOp::CreateNode {
            node_type,
            parent,
            name,
            x,
            y,
            width,
            height,
            fill,
            text,
            font_size,
        } => {
            if !(width.is_finite() && *width > 0.0 && height.is_finite() && *height > 0.0) {
                bail!("width and height must be positive");
            }
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let fill_color = fill.as_deref().map(parse_fill_color).transpose()?;
            let (data, origin) = match node_type {
                DesignNodeType::Frame => (
                    NodeData::Group(GroupNode {
                        clip_size: Some([*width, *height]),
                        background: Some(Fill::solid(fill_color.unwrap_or(Color::WHITE))),
                        ..GroupNode::default()
                    }),
                    (*x, *y),
                ),
                DesignNodeType::Rectangle => (
                    NodeData::Vector(VectorNode::rect_solid(
                        0.0,
                        0.0,
                        *width,
                        *height,
                        fill_color.unwrap_or(DEFAULT_FILL_COLOR),
                    )),
                    (*x, *y),
                ),
                DesignNodeType::Ellipse => {
                    let mut vector = VectorNode {
                        path: PathData::ellipse(0.0, 0.0, width * 0.5, height * 0.5),
                        ..VectorNode::default()
                    };
                    vector
                        .fills
                        .push(Fill::solid(fill_color.unwrap_or(DEFAULT_FILL_COLOR)));
                    // The path is centered on the local origin, so the node
                    // transform points at the ellipse's center.
                    (
                        NodeData::Vector(vector),
                        (x + width * 0.5, y + height * 0.5),
                    )
                }
                DesignNodeType::Text => {
                    let mut text_node = TextNode::new(
                        text.clone().unwrap_or_else(|| "Text".to_string()),
                        *width,
                        *height,
                    );
                    if let Some(size) = font_size {
                        if !(size.is_finite() && *size > 0.0) {
                            bail!("font_size must be positive");
                        }
                        text_node.style.size_px = f64::from(*size);
                    }
                    if let Some(color) = fill_color {
                        text_node.style.color = color;
                    }
                    (NodeData::Text(text_node), (*x, *y))
                }
            };
            let mut node = CanvasNode::new(data);
            if let Some(name) = name {
                node.name = name.clone();
            }
            // Rebase the desired world position into the parent's space, the
            // same way the canvas creation tools place new nodes.
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform =
                Transform2D::translation(origin.0, origin.1).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::create_node(node))
                .context("creating the node")?;
            Ok(Applied::created(id))
        }
        DesignOp::SetProps {
            id,
            name,
            x,
            y,
            width,
            height,
            opacity,
            fill,
            corner_radius,
            text,
            hidden,
            locked,
        } => {
            let id = parse_node_id(id)?;
            let mut applied_any = false;
            let node = |doc: &Doc| {
                doc.scene
                    .get(id)
                    .with_context(|| format!("node {id} does not exist"))
                    .cloned()
            };
            node(doc)?;

            if let Some(name) = name {
                let old = node(doc)?.name;
                if *name != old {
                    doc.apply(Operation::SetName {
                        id,
                        old,
                        new: name.clone(),
                    })?;
                    applied_any = true;
                }
            }
            if let Some(text) = text {
                let NodeData::Text(_) = node(doc)?.data else {
                    bail!("node {id} is not a text node");
                };
                for operation in replace_data_operation(doc, id, |data| {
                    if let NodeData::Text(text_node) = data {
                        text_node.content = text.clone();
                    }
                }) {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(fill) = fill {
                let color = parse_fill_color(fill)?;
                if !matches!(
                    node(doc)?.data,
                    NodeData::Vector(_)
                        | NodeData::Group(_)
                        | NodeData::Boolean(_)
                        | NodeData::Text(_)
                ) {
                    bail!("cannot set a fill on a {} node", node(doc)?.data.kind_tag());
                }
                for operation in replace_data_operation(doc, id, |data| set_solid_fill(data, color))
                {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(radius) = corner_radius {
                if !(radius.is_finite() && *radius >= 0.0) {
                    bail!("corner_radius must be non-negative");
                }
                if !matches!(node(doc)?.data, NodeData::Vector(_) | NodeData::Group(_)) {
                    bail!(
                        "corner_radius only applies to rectangles and frames, not a {} node",
                        node(doc)?.data.kind_tag()
                    );
                }
                for operation in
                    replace_data_operation(doc, id, |data| set_corner_radius(data, *radius))
                {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(opacity) = opacity {
                let old = node(doc)?.opacity;
                let new = UnitInterval::new(*opacity);
                if new != old {
                    doc.apply(Operation::SetOpacity { id, old, new })?;
                    applied_any = true;
                }
            }
            if hidden.is_some() || locked.is_some() {
                let old = node(doc)?.flags;
                let mut new = old;
                if let Some(hidden) = hidden {
                    new.set(NodeFlags::HIDDEN, *hidden);
                }
                if let Some(locked) = locked {
                    new.set(NodeFlags::LOCKED, *locked);
                }
                if new != old {
                    doc.apply(Operation::SetFlags { id, old, new })?;
                    applied_any = true;
                }
            }
            if let Some(width) = width {
                if !(width.is_finite() && *width > 0.0) {
                    bail!("width must be positive");
                }
                let operations = resize_operations(doc, id, *width, true);
                if operations.is_empty() {
                    bail!("node {id} cannot be resized");
                }
                for operation in operations {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(height) = height {
                if !(height.is_finite() && *height > 0.0) {
                    bail!("height must be positive");
                }
                let operations = resize_operations(doc, id, *height, false);
                if operations.is_empty() {
                    bail!("node {id} cannot be resized");
                }
                for operation in operations {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if x.is_some() || y.is_some() {
                // `x`/`y` address the node's world bounds origin (matching
                // `create_node`), so shift the world transform by the delta.
                let bounds = doc
                    .scene
                    .world_bounds(id)
                    .with_context(|| format!("node {id} has no bounds to position"))?;
                let world = doc
                    .scene
                    .world_transform(id)
                    .with_context(|| format!("node {id} has no world transform"))?;
                let delta_x = x.map(|x| x - bounds.min_x).unwrap_or(0.0);
                let delta_y = y.map(|y| y - bounds.min_y).unwrap_or(0.0);
                if !(delta_x.is_finite() && delta_y.is_finite()) {
                    bail!("x and y must be finite");
                }
                if delta_x != 0.0 || delta_y != 0.0 {
                    let current = node(doc)?;
                    let parent_world = current
                        .parent
                        .and_then(|parent| doc.scene.world_transform(parent))
                        .unwrap_or(Transform2D::IDENTITY);
                    let new_local = world
                        .then(&Transform2D::translation(delta_x, delta_y))
                        .then(&parent_world.inverse());
                    doc.apply(Operation::SetTransform {
                        id,
                        old: current.transform,
                        new: new_local,
                    })?;
                    applied_any = true;
                }
            }

            if applied_any {
                Ok(Applied::changed())
            } else {
                Ok(Applied::Nothing)
            }
        }
        DesignOp::CreateInstance {
            component,
            x,
            y,
            parent,
            name,
        } => {
            if !(x.is_finite() && y.is_finite()) {
                bail!("x and y must be finite");
            }
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let component_id = resolve_component(doc, component)?;
            let def = doc
                .components
                .def(component_id)
                .or_else(|| {
                    doc.components
                        .sets
                        .get(&component_id)
                        .and_then(|set| doc.components.def(set.default_variant))
                })
                .with_context(|| format!("component {component} does not exist"))?;
            let masters = doc
                .components
                .sets
                .get(&component_id)
                .map(|set| {
                    set.members
                        .iter()
                        .filter_map(|id| doc.components.def(*id))
                        .map(|definition| definition.root)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![def.root]);
            if parent.is_some_and(|parent| {
                masters.iter().any(|root| {
                    *root == parent
                        || doc
                            .scene
                            .ancestors_of(parent)
                            .any(|ancestor| ancestor.id == *root)
                })
            }) {
                bail!("cannot place an instance of a component inside its own master");
            }
            let local_size = master_size(doc, def.root).unwrap_or([100.0, 100.0]);
            let mut node = CanvasNode::new(NodeData::Instance(InstanceNode {
                component: component_id,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size,
            }));
            node.name = name.clone().unwrap_or_else(|| def.name.clone());
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform = Transform2D::translation(*x, *y).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::CreateInstance {
                node: Box::new(node),
            })
            .context("creating the instance")?;
            Ok(Applied::created(id))
        }
        DesignOp::SetStroke {
            id,
            color,
            width,
            align,
            sides,
            dash,
            cap,
            join,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Vector(_) | NodeData::Group(_)) {
                bail!(
                    "strokes apply to shapes and frames, not a {} node",
                    node.data.kind_tag()
                );
            }
            if let Some(width) = width
                && !(width.is_finite() && *width >= 0.0)
            {
                bail!("the stroke width must be non-negative");
            }
            let color = color.as_deref().map(parse_fill_color).transpose()?;
            let align = align.map(|align| match align {
                StrokeAlignment::Inside => StrokeAlign::Inside,
                StrokeAlignment::Center => StrokeAlign::Center,
                StrokeAlignment::Outside => StrokeAlign::Outside,
            });
            if let Some(Some(sides)) = sides
                && sides.iter().any(|side| !(side.is_finite() && *side >= 0.0))
            {
                bail!("per-side stroke widths must be non-negative");
            }
            if let Some(dash) = dash
                && dash
                    .iter()
                    .any(|length| !(length.is_finite() && *length >= 0.0))
            {
                bail!("dash lengths must be non-negative");
            }
            let cap = cap.map(|cap| match cap {
                DesignStrokeCap::Butt => StrokeCap::Butt,
                DesignStrokeCap::Round => StrokeCap::Round,
                DesignStrokeCap::Square => StrokeCap::Square,
            });
            let join = join.map(|join| match join {
                DesignStrokeJoin::Miter => StrokeJoin::Miter,
                DesignStrokeJoin::Round => StrokeJoin::Round,
                DesignStrokeJoin::Bevel => StrokeJoin::Bevel,
            });
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let Some(strokes) = stroke_list_mut(data) else {
                        return;
                    };
                    if *width == Some(0.0) {
                        strokes.clear();
                        return;
                    }
                    if strokes.is_empty() {
                        strokes.push(Stroke::solid(
                            color.unwrap_or(Color::BLACK),
                            width.unwrap_or(1.0),
                        ));
                    }
                    let Some(stroke) = strokes.first_mut() else {
                        return;
                    };
                    if let Some(color) = color {
                        stroke.paint.set_solid_color(color);
                    }
                    if let Some(width) = width {
                        stroke.width = *width;
                    }
                    if let Some(align) = align {
                        stroke.align = align;
                    }
                    if let Some(sides) = sides {
                        stroke.per_side = *sides;
                    }
                    if let Some(dash) = dash {
                        stroke.dash = dash.clone();
                    }
                    if let Some(cap) = cap {
                        stroke.cap = cap;
                    }
                    if let Some(join) = join {
                        stroke.join = join;
                    }
                }),
            )
        }
        DesignOp::SetEffects { id, effects } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let mut shadows = Vec::new();
            let mut blurs = Vec::new();
            for effect in effects {
                match effect {
                    DesignEffect::DropShadow(shadow) => {
                        shadows.push(design_shadow(shadow, ShadowKind::Drop)?)
                    }
                    DesignEffect::InnerShadow(shadow) => {
                        shadows.push(design_shadow(shadow, ShadowKind::Inner)?)
                    }
                    DesignEffect::LayerBlur { radius }
                    | DesignEffect::BackgroundBlur { radius } => {
                        if !(radius.is_finite() && *radius >= 0.0) {
                            bail!("the blur radius must be non-negative");
                        }
                        let kind = if matches!(effect, DesignEffect::LayerBlur { .. }) {
                            fanta_doc::BlurKind::Layer
                        } else {
                            fanta_doc::BlurKind::Background
                        };
                        blurs.push(fanta_doc::Blur {
                            kind,
                            radius: *radius,
                        });
                    }
                }
            }
            let mut operations = effects_operations(doc, id, |effects| {
                *effects = shadows.into_iter().collect();
            });
            operations.extend(blurs_operations(doc, id, |current| {
                *current = blurs.into_iter().collect();
            }));
            apply_all(doc, operations)
        }
        DesignOp::SetFill { id, paints } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(
                node.data,
                NodeData::Vector(_) | NodeData::Boolean(_) | NodeData::Group(_)
            ) {
                bail!(
                    "fills apply to frames and shapes, not a {} node (text color is set_text_style)",
                    node.data.kind_tag()
                );
            }
            let mut fills = Vec::with_capacity(paints.len());
            for paint in paints {
                fills.push(design_fill(paint, assets, ingested_assets)?);
            }
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| match data {
                    NodeData::Vector(vector) => vector.fills = fills.into_iter().collect(),
                    NodeData::Boolean(boolean) => boolean.fills = fills.into_iter().collect(),
                    NodeData::Group(group) => {
                        let mut fills = fills.into_iter();
                        group.background = fills.next();
                        group.background_fills = fills.collect();
                    }
                    _ => {}
                }),
            )
        }
        DesignOp::SetConstraints {
            id,
            horizontal,
            vertical,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            let old = node.constraints;
            let mut new = old.unwrap_or_default();
            if let Some(horizontal) = horizontal {
                new.horizontal = match horizontal {
                    HorizontalConstraint::Left => fanta_doc::ConstraintH::Left,
                    HorizontalConstraint::Right => fanta_doc::ConstraintH::Right,
                    HorizontalConstraint::LeftRight => fanta_doc::ConstraintH::LeftRight,
                    HorizontalConstraint::Center => fanta_doc::ConstraintH::Center,
                    HorizontalConstraint::Scale => fanta_doc::ConstraintH::Scale,
                };
            }
            if let Some(vertical) = vertical {
                new.vertical = match vertical {
                    VerticalConstraint::Top => fanta_doc::ConstraintV::Top,
                    VerticalConstraint::Bottom => fanta_doc::ConstraintV::Bottom,
                    VerticalConstraint::TopBottom => fanta_doc::ConstraintV::TopBottom,
                    VerticalConstraint::Center => fanta_doc::ConstraintV::Center,
                    VerticalConstraint::Scale => fanta_doc::ConstraintV::Scale,
                };
            }
            apply_all(
                doc,
                vec![Operation::SetConstraints {
                    id,
                    old,
                    new: Some(new),
                }],
            )
        }
        DesignOp::CreateShape {
            shape,
            parent,
            name,
            x,
            y,
            width,
            height,
            points,
            inner_ratio,
            path,
            fill,
            stroke,
            stroke_width,
        } => {
            if ![*x, *y, *width, *height]
                .iter()
                .all(|value| value.is_finite())
                || *width < 0.0
                || *height < 0.0
            {
                bail!("x, y, width and height must be finite and the size non-negative");
            }
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let (path_data, open) = match shape {
                DesignShape::Line => {
                    let mut line = PathData::new();
                    line.move_to(0.0, 0.0);
                    line.line_to(*width, *height);
                    (line, true)
                }
                DesignShape::Polygon => (
                    PathData::polygon(*width, *height, points.unwrap_or(3)),
                    false,
                ),
                DesignShape::Star => (
                    PathData::star(
                        *width,
                        *height,
                        points.unwrap_or(5),
                        inner_ratio.unwrap_or(0.382),
                    ),
                    false,
                ),
                DesignShape::Path => {
                    let d = path
                        .as_deref()
                        .context("shape \"path\" needs SVG path data in `path`")?;
                    let data = PathData::from_svg_d(d)
                        .map_err(|error| anyhow!("invalid SVG path data: {error}"))?;
                    let open = !d.trim_end().to_ascii_lowercase().ends_with('z');
                    (data, open)
                }
            };
            let mut vector = VectorNode {
                path: path_data,
                ..VectorNode::default()
            };
            let fill = fill.as_deref().map(parse_fill_color).transpose()?;
            if let Some(color) = fill.or((!open).then_some(DEFAULT_FILL_COLOR)) {
                vector.fills.push(Fill::solid(color));
            }
            let stroke_color = stroke.as_deref().map(parse_fill_color).transpose()?;
            if let Some(width) = stroke_width
                && !(width.is_finite() && *width >= 0.0)
            {
                bail!("the stroke width must be non-negative");
            }
            if let Some(color) = stroke_color.or(open.then_some(Color::BLACK)) {
                vector
                    .strokes
                    .push(Stroke::solid(color, stroke_width.unwrap_or(1.0)));
            }
            let mut node = CanvasNode::new(NodeData::Vector(vector));
            node.name = name.clone().unwrap_or_else(|| {
                match shape {
                    DesignShape::Line => "Line",
                    DesignShape::Polygon => "Polygon",
                    DesignShape::Star => "Star",
                    DesignShape::Path => "Vector",
                }
                .to_owned()
            });
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform = Transform2D::translation(*x, *y).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::create_node(node))
                .context("creating the shape")?;
            Ok(Applied::created(id))
        }
        DesignOp::SetShadow {
            id,
            color,
            x,
            y,
            blur,
            spread,
            remove,
        } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let color = color.as_deref().map(parse_fill_color).transpose()?;
            for (label, value) in [("blur", blur), ("spread", spread), ("x", x), ("y", y)] {
                if let Some(value) = value
                    && !value.is_finite()
                {
                    bail!("the shadow {label} must be finite");
                }
            }
            if let Some(blur) = blur
                && *blur < 0.0
            {
                bail!("the shadow blur must be non-negative");
            }
            apply_all(
                doc,
                effects_operations(doc, id, |effects| {
                    if *remove == Some(true) {
                        effects.retain(|shadow| shadow.kind != ShadowKind::Drop);
                        return;
                    }
                    if !effects.iter().any(|shadow| shadow.kind == ShadowKind::Drop) {
                        effects.push(default_shadow());
                    }
                    let Some(shadow) = effects
                        .iter_mut()
                        .find(|shadow| shadow.kind == ShadowKind::Drop)
                    else {
                        return;
                    };
                    if let Some(color) = color {
                        shadow.color = color;
                    }
                    if let Some(x) = x {
                        shadow.offset[0] = *x;
                    }
                    if let Some(y) = y {
                        shadow.offset[1] = *y;
                    }
                    if let Some(blur) = blur {
                        shadow.blur = *blur;
                    }
                    if let Some(spread) = spread {
                        shadow.spread = *spread;
                    }
                }),
            )
        }
        DesignOp::SetTextStyle {
            id,
            font_family,
            font_weight,
            font_size,
            line_height,
            letter_spacing,
            align,
            color,
            sizing,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Text(_)) {
                bail!("node {id} is not a text node");
            }
            if let Some(size) = font_size
                && !(size.is_finite() && *size > 0.0)
            {
                bail!("font_size must be positive");
            }
            if let Some(weight) = font_weight
                && !(100..=900).contains(weight)
            {
                bail!("font_weight must be between 100 and 900");
            }
            if let Some(line_height) = line_height
                && !(line_height.is_finite() && *line_height > 0.0)
            {
                bail!("line_height must be a positive multiple of the font size");
            }
            if let Some(spacing) = letter_spacing
                && !spacing.is_finite()
            {
                bail!("letter_spacing must be finite");
            }
            let color = color.as_deref().map(parse_fill_color).transpose()?;
            let align = align.map(|align| match align {
                TextAlignment::Left => TextAlign::Left,
                TextAlignment::Center => TextAlign::Center,
                TextAlignment::Right => TextAlign::Right,
                TextAlignment::Justify => TextAlign::Justify,
            });
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let NodeData::Text(text) = data else {
                        return;
                    };
                    // Rich-text runs override the base style, so a whole-node
                    // change has to land on every run too or it would show on
                    // none of the styled characters.
                    let mut styles = vec![&mut text.style];
                    styles.extend(text.style_runs.iter_mut().map(|run| &mut run.style));
                    for style in styles {
                        if let Some(family) = font_family {
                            style.font_family = family.clone();
                        }
                        if let Some(weight) = font_weight {
                            style.weight = *weight;
                        }
                        if let Some(size) = font_size {
                            style.size_px = *size;
                        }
                        if let Some(line_height) = line_height {
                            style.line_height = *line_height;
                            style.line_height_auto_percent = None;
                        }
                        if let Some(spacing) = letter_spacing {
                            style.letter_spacing = *spacing;
                        }
                        if let Some(color) = color {
                            style.color = color;
                        }
                    }
                    if let Some(align) = align {
                        text.align = align;
                    }
                    if let Some(sizing) = sizing {
                        text.auto_resize = match sizing {
                            TextSizing::Fixed => TextAutoResize::None,
                            TextSizing::AutoHeight => TextAutoResize::Height,
                            TextSizing::AutoWidth => TextAutoResize::WidthAndHeight,
                        };
                    }
                }),
            )
        }
        DesignOp::SetAutoLayout {
            id,
            direction,
            gap,
            padding,
            align_items,
            justify,
            primary_sizing,
            counter_sizing,
            min_size,
            max_size,
            wrap,
            counter_gap,
            align_content,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Group(_)) {
                bail!(
                    "auto layout applies to frames and groups, not a {} node",
                    node.data.kind_tag()
                );
            }
            let child_count = doc.scene.children_of(Some(id)).len();
            if let Some(gap) = gap
                && !(gap.is_finite() && *gap >= 0.0)
            {
                bail!("gap must be non-negative");
            }
            let padding = padding.as_deref().map(parse_padding).transpose()?;
            if counter_gap.is_some_and(|gap| !gap.is_finite() || gap < 0.0) {
                bail!("counter_gap must be finite and non-negative");
            }
            for size in [min_size, max_size].into_iter().flatten() {
                if size
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite() || *value < 0.0)
                {
                    bail!("min_size and max_size must be finite and non-negative");
                }
            }
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let NodeData::Group(group) = data else {
                        return;
                    };
                    let mode = match direction {
                        LayoutDirection::Horizontal => LayoutMode::Horizontal,
                        LayoutDirection::Vertical => LayoutMode::Vertical,
                        LayoutDirection::Grid => LayoutMode::Grid,
                        LayoutDirection::None => {
                            group.auto_layout = None;
                            group.grid = None;
                            return;
                        }
                    };
                    let layout = group.auto_layout.get_or_insert_with(AutoLayout::default);
                    layout.mode = mode;
                    if mode == LayoutMode::Grid {
                        // A grid's gaps live on its tracks: `gap` sets both,
                        // `counter_gap` the row gap.
                        let grid = group.grid.get_or_insert_with(|| {
                            GridLayout::for_children(child_count, layout.spacing)
                        });
                        if let Some(gap) = gap {
                            grid.column_gap = *gap;
                            grid.row_gap = *gap;
                        }
                        if let Some(gap) = counter_gap {
                            grid.row_gap = *gap;
                        }
                    } else {
                        group.grid = None;
                    }
                    if let Some(align_content) = align_content {
                        layout.counter_auto_spacing = *align_content == AlignContent::SpaceBetween;
                    }
                    if let Some(sizing) = primary_sizing {
                        layout.primary_sizing = layout_sizing(*sizing);
                    }
                    if let Some(sizing) = counter_sizing {
                        layout.counter_sizing = layout_sizing(*sizing);
                    }
                    if let Some(size) = min_size {
                        layout.min_size = *size;
                    }
                    if let Some(size) = max_size {
                        layout.max_size = *size;
                    }
                    if let Some(wrap) = wrap {
                        layout.wrap = *wrap;
                    }
                    if let Some(gap) = counter_gap {
                        layout.counter_spacing = *gap;
                    }
                    if let Some(gap) = gap {
                        layout.spacing = *gap;
                    }
                    if let Some(padding) = padding {
                        layout.padding = padding;
                    }
                    if let Some(align_items) = align_items {
                        layout.counter_align = match align_items {
                            CrossAxisAlignment::Start => CounterAlign::Start,
                            CrossAxisAlignment::Center => CounterAlign::Center,
                            CrossAxisAlignment::End => CounterAlign::End,
                            CrossAxisAlignment::Stretch => CounterAlign::Stretch,
                            CrossAxisAlignment::Baseline => CounterAlign::Baseline,
                        };
                    }
                    if let Some(justify) = justify {
                        layout.primary_align = match justify {
                            MainAxisAlignment::Start => PrimaryAlign::Start,
                            MainAxisAlignment::Center => PrimaryAlign::Center,
                            MainAxisAlignment::End => PrimaryAlign::End,
                            MainAxisAlignment::SpaceBetween => PrimaryAlign::SpaceBetween,
                            MainAxisAlignment::SpaceEvenly => PrimaryAlign::SpaceEvenly,
                        };
                    }
                }),
            )
        }
        DesignOp::SetGridLayout {
            id,
            columns,
            rows,
            gap,
            row_gap,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Group(_)) {
                bail!(
                    "grid layout applies to frames and groups, not a {} node",
                    node.data.kind_tag()
                );
            }
            if columns.is_empty() {
                bail!("columns needs at least one track");
            }
            let columns = columns.iter().map(grid_track).collect::<Result<Vec<_>>>()?;
            let rows = rows
                .as_ref()
                .map(|rows| rows.iter().map(grid_track).collect::<Result<Vec<_>>>())
                .transpose()?;
            if [gap, row_gap]
                .into_iter()
                .flatten()
                .any(|gap| !gap.is_finite() || *gap < 0.0)
            {
                bail!("gap and row_gap must be finite and non-negative");
            }
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let NodeData::Group(group) = data else {
                        return;
                    };
                    group
                        .auto_layout
                        .get_or_insert_with(AutoLayout::default)
                        .mode = LayoutMode::Grid;
                    let grid = group.grid.get_or_insert_with(GridLayout::default);
                    grid.columns = columns;
                    if let Some(rows) = rows {
                        grid.rows = rows;
                    }
                    if let Some(gap) = gap {
                        grid.column_gap = *gap;
                        grid.row_gap = *gap;
                    }
                    if let Some(gap) = row_gap {
                        grid.row_gap = *gap;
                    }
                }),
            )
        }
        DesignOp::SetLayoutChild {
            id,
            grow,
            align_self,
            absolute,
            column,
            row,
            column_span,
            row_span,
            cell_horizontal,
            cell_vertical,
            auto_place,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if grow.is_some_and(|grow| !grow.is_finite() || grow < 0.0) {
                bail!("grow must be finite and non-negative");
            }
            let pins_cell = column.is_some()
                || row.is_some()
                || column_span.is_some()
                || row_span.is_some()
                || cell_horizontal.is_some()
                || cell_vertical.is_some();
            if pins_cell || auto_place.is_some() {
                let parent_is_grid = node
                    .parent
                    .and_then(|parent| doc.scene.get(parent))
                    .is_some_and(|parent| {
                        matches!(&parent.data, NodeData::Group(group)
                            if group.auto_layout.is_some_and(|layout| layout.mode == LayoutMode::Grid))
                    });
                if !parent_is_grid {
                    bail!(
                        "column, row, spans, cell alignment and auto_place apply to children of a grid frame"
                    );
                }
            }
            if pins_cell && *auto_place == Some(true) {
                bail!(
                    "auto_place unpins the cell; send it without column, row, spans or cell alignment"
                );
            }
            if [column_span, row_span]
                .into_iter()
                .flatten()
                .any(|span| *span == 0)
            {
                bail!("column_span and row_span must be at least 1");
            }
            let old = node.layout_child;
            let mut new = old.unwrap_or_default();
            if *auto_place == Some(true) {
                new.grid = None;
            }
            if pins_cell {
                let mut cell = new.grid.unwrap_or_default();
                if let Some(column) = column {
                    cell.column = *column;
                }
                if let Some(row) = row {
                    cell.row = *row;
                }
                if let Some(span) = column_span {
                    cell.column_span = *span;
                }
                if let Some(span) = row_span {
                    cell.row_span = *span;
                }
                if let Some(align) = cell_horizontal {
                    cell.horizontal = grid_alignment(*align);
                }
                if let Some(align) = cell_vertical {
                    cell.vertical = grid_alignment(*align);
                }
                new.grid = Some(cell);
            }
            if let Some(grow) = grow {
                new.grow = *grow;
            }
            if let Some(align) = align_self {
                new.align_self = Some(counter_alignment(*align));
            }
            if let Some(absolute) = absolute {
                new.absolute = *absolute;
            }
            apply_all(
                doc,
                vec![Operation::SetLayoutChild {
                    id,
                    old,
                    new: Some(new),
                }],
            )
        }
        DesignOp::CreateVariableCollection { name, modes } => {
            let name = required_name(name)?;
            if doc
                .variables
                .collections
                .values()
                .any(|collection| collection.name == name)
            {
                bail!("a variable collection named {name:?} already exists");
            }
            let names = if modes.is_empty() {
                vec!["Default".to_owned()]
            } else {
                modes.clone()
            };
            let mut unique = HashSet::new();
            let modes = names
                .iter()
                .map(|name| {
                    let name = required_name(name)?;
                    if !unique.insert(name.to_owned()) {
                        bail!("duplicate mode name {name:?}");
                    }
                    Ok(Mode {
                        id: ModeId::new(),
                        name: name.to_owned(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let default_mode = modes.first().context("a collection requires a mode")?.id;
            let collection = VariableCollection {
                id: VariableCollectionId::new(),
                name: name.to_owned(),
                modes,
                default_mode,
                variable_order: Vec::new(),
            };
            let detail =
                json!({"collection": collection.id.to_string(), "modes": collection.modes});
            doc.apply(Operation::CreateVariableCollection {
                collection: Box::new(collection),
            })?;
            Ok(Applied::Content {
                created: None,
                detail: Some(detail),
            })
        }
        DesignOp::AddVariableMode { collection, name } => {
            let collection = find_collection(doc, collection)?;
            let name = required_name(name)?;
            if collection.modes.iter().any(|mode| mode.name == name) {
                bail!("mode {name:?} already exists");
            }
            let mode = Mode {
                id: ModeId::new(),
                name: name.to_owned(),
            };
            let detail = json!({"mode": mode.id.to_string()});
            let transaction = doc.add_mode_transaction(collection.id, mode)?;
            for operation in transaction.ops {
                doc.apply(operation)?;
            }
            Ok(Applied::Content {
                created: None,
                detail: Some(detail),
            })
        }
        DesignOp::CreateVariable {
            collection,
            name,
            kind,
            value,
            options,
        } => {
            let collection = find_collection(doc, collection)?;
            let name = required_name(name)?;
            if doc
                .variables
                .variables
                .values()
                .any(|variable| variable.collection == collection.id && variable.name == name)
            {
                bail!("variable {name:?} already exists in this collection");
            }
            if collection.modes.is_empty() {
                bail!("add a mode before creating variables");
            }
            let ty = variable_type(*kind);
            let options = if ty == VariableType::Enum {
                enum_options(options)?
            } else if options.is_empty() {
                Vec::new()
            } else {
                bail!("only enum variables take options");
            };
            let value = parse_variable_value(doc, ty, &options, value)?;
            let variable = Variable {
                id: VariableId::new(),
                collection: collection.id,
                name: name.to_owned(),
                ty,
                values_by_mode: collection
                    .modes
                    .iter()
                    .map(|mode| (mode.id, value.clone()))
                    .collect(),
                scopes: Vec::new(),
                options,
            };
            let detail = json!({"variable": variable.id.to_string()});
            doc.apply(Operation::CreateVariable {
                variable: Box::new(variable),
            })?;
            Ok(Applied::Content {
                created: None,
                detail: Some(detail),
            })
        }
        DesignOp::SetVariableValue {
            variable,
            mode,
            value,
        } => {
            let variable = find_variable(doc, variable)?;
            let collection = doc
                .variables
                .collection_of(variable.id)
                .context("variable collection is missing")?;
            let mode = find_mode(collection, mode)?;
            let value = parse_variable_value(doc, variable.ty, &variable.options, value)?;
            let operation =
                crate::edit_operations::set_variable_value_operation(doc, variable.id, mode, value)
                    .map_err(|error| anyhow!(error))?;
            apply_all(doc, operation.into_iter().collect())
        }
        DesignOp::SetVariableOptions { variable, options } => {
            let variable = find_variable(doc, variable)?;
            if variable.ty != VariableType::Enum {
                bail!("only enum variables take options");
            }
            let id = variable.id;
            let old = variable.options.clone();
            let new = enum_options(options)?;
            if old == new {
                return Ok(Applied::Content {
                    created: None,
                    detail: Some(json!({"variable": id.to_string()})),
                });
            }
            doc.apply(Operation::SetVariableOptions { id, old, new })?;
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({"variable": id.to_string()})),
            })
        }
        DesignOp::SetVariableMode {
            collection,
            mode,
            frame,
        } => {
            let collection = find_collection(doc, collection)?;
            let new = mode
                .as_deref()
                .map(|mode| find_mode(collection, mode))
                .transpose()?;
            let (scope, old) = if let Some(frame) = frame {
                let id = parse_node_id(frame)?;
                let NodeData::Group(group) = &existing_node(doc, id)?.data else {
                    bail!("mode pins require a frame or group");
                };
                (
                    ModeScope::Frame { node: id },
                    group.explicit_modes.get(&collection.id).copied(),
                )
            } else {
                (
                    ModeScope::Doc,
                    doc.active_modes.get(&collection.id).copied(),
                )
            };
            apply_all(
                doc,
                vec![Operation::SetActiveMode {
                    scope,
                    collection: collection.id,
                    old,
                    new,
                }],
            )
        }
        DesignOp::BindVariable {
            id,
            property,
            variable,
        } => {
            let variable = find_variable(doc, variable)?.id;
            let operation = crate::edit_operations::variable_binding_operation(
                doc,
                parse_node_id(id)?,
                binding_property(*property),
                Some(variable),
            )
            .map_err(|error| anyhow!(error))?;
            apply_all(doc, operation.into_iter().collect())
        }
        DesignOp::UnbindVariable { id, property } => {
            let operation = crate::edit_operations::variable_binding_operation(
                doc,
                parse_node_id(id)?,
                binding_property(*property),
                None,
            )
            .map_err(|error| anyhow!(error))?;
            apply_all(doc, operation.into_iter().collect())
        }
        DesignOp::SetIndex { id, position } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if doc.pages().contains(&id) {
                bail!("page roots cannot be reordered with set_index");
            }
            let Some(new) = layer_position_index(doc, node.parent, id, node.index, *position)?
            else {
                return Ok(Applied::Nothing);
            };
            doc.apply(Operation::SetIndex {
                id,
                old: node.index,
                new,
            })?;
            Ok(Applied::changed())
        }
        DesignOp::Rotate { id, degrees } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            if !degrees.is_finite() {
                bail!("degrees must be finite");
            }
            if doc.pages().contains(&id) {
                bail!("page roots cannot be rotated");
            }
            apply_all(doc, rotation_operations(doc, id, *degrees))
        }
        DesignOp::Align { ids, edge } => {
            let ids = parse_content_ids(doc, ids)?;
            let target = match ids.as_slice() {
                [] => bail!("align needs at least one node id"),
                [single] => {
                    let parent = existing_node(doc, *single)?.parent.with_context(|| {
                        format!("node {single} has no parent frame to align to")
                    })?;
                    if doc.pages().contains(&parent) {
                        bail!(
                            "aligning a single node needs a parent frame, or two or more ids to align to each other"
                        );
                    }
                    doc.scene
                        .world_bounds(parent)
                        .with_context(|| format!("parent {parent} has no bounds"))?
                }
                many => union_bounds(doc, many).context("the nodes have no bounds to align")?,
            };
            let operations = match edge {
                AlignEdge::Left => fanta_canvas::align_to_bounds_h(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::HAlign::Left,
                ),
                AlignEdge::CenterX => fanta_canvas::align_to_bounds_h(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::HAlign::Center,
                ),
                AlignEdge::Right => fanta_canvas::align_to_bounds_h(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::HAlign::Right,
                ),
                AlignEdge::Top => fanta_canvas::align_to_bounds_v(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::VAlign::Top,
                ),
                AlignEdge::CenterY => fanta_canvas::align_to_bounds_v(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::VAlign::Middle,
                ),
                AlignEdge::Bottom => fanta_canvas::align_to_bounds_v(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::VAlign::Bottom,
                ),
            };
            apply_all(doc, operations)
        }
        DesignOp::Distribute { ids, axis } => {
            let ids = parse_content_ids(doc, ids)?;
            if ids.len() < 3 {
                bail!("distribute needs at least three node ids");
            }
            let axis = match axis {
                DistributeAxis::Horizontal => fanta_canvas::Axis::X,
                DistributeAxis::Vertical => fanta_canvas::Axis::Y,
            };
            apply_all(doc, fanta_canvas::distribute(&doc.scene, &ids, axis))
        }
        DesignOp::Group { ids, name } => {
            let ids = parse_content_ids(doc, ids)?;
            let grouped = group_operations(doc, &ids, name.as_deref())?;
            for operation in grouped.operations {
                doc.apply(operation)?;
            }
            Ok(Applied::created(grouped.group))
        }
        DesignOp::FrameSelection { ids, name } => {
            let ids = parse_content_ids(doc, ids)?;
            let framed = frame_selection_operations(doc, &ids, name.as_deref())?;
            for operation in framed.operations {
                doc.apply(operation)?;
            }
            Ok(Applied::created(framed.group))
        }
        DesignOp::Ungroup { id } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let ungrouped = ungroup_operations(doc, id)?;
            for operation in ungrouped.operations {
                doc.apply(operation)?;
            }
            removed_nodes.insert(id);
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({
                    "children": ungrouped
                        .children
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                })),
            })
        }
        DesignOp::Duplicate { id, dx, dy } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let pasted = duplicate_operations(doc, &[id], (dx.unwrap_or(0.0), dy.unwrap_or(0.0)))?;
            let copy = pasted
                .roots
                .first()
                .copied()
                .context("duplicating produced no copy")?;
            for operation in create_operations(&pasted) {
                doc.apply(operation)?;
            }
            Ok(Applied::created(copy))
        }
        DesignOp::CreateComponent { id } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if doc.pages().contains(&id) {
                bail!("a page cannot become a component");
            }
            let operations = create_component_operations(doc, id);
            let Some(Operation::DefineComponent { def }) = operations.first() else {
                if !matches!(node.data, NodeData::Group(_)) {
                    bail!(
                        "only frames and groups can become components, not a {} node",
                        node.data.kind_tag()
                    );
                }
                bail!("node {id} is already a component master");
            };
            let component = def.id;
            for operation in operations {
                doc.apply(operation)?;
            }
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({ "component": component.to_string() })),
            })
        }
        DesignOp::Componentize { ids } => {
            let ids = parse_content_ids(doc, ids)?;
            let Some((&master, copies)) =
                ids.split_first().filter(|(_, copies)| !copies.is_empty())
            else {
                bail!("componentize needs the master's id and at least one copy");
            };
            let related = |a: NodeId, b: NodeId| {
                doc.scene.ancestors_of(a).any(|ancestor| ancestor.id == b)
                    || doc.scene.ancestors_of(b).any(|ancestor| ancestor.id == a)
            };
            for (index, &copy) in copies.iter().enumerate() {
                if related(master, copy)
                    || copies[..index].iter().any(|&other| related(other, copy))
                {
                    bail!(
                        "node {copy} is inside another of the ids; componentize layers side by side"
                    );
                }
                if doc.components.defs.values().any(|def| def.root == copy) {
                    bail!("node {copy} is already a component master");
                }
            }
            // Check every copy before changing anything.
            let plans = copies
                .iter()
                .map(|&copy| {
                    Ok((
                        copy,
                        crate::componentize::overrides_for_copy(doc, master, copy)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;

            let component = match doc.components.defs.values().find(|def| def.root == master) {
                Some(def) if def.variant_of.is_some() => {
                    bail!("node {master} is a variant; componentize a standalone layer")
                }
                Some(def) => def.id,
                None => {
                    let operations = create_component_operations(doc, master);
                    let Some(Operation::DefineComponent { def }) = operations.first() else {
                        bail!(
                            "only frames and groups can become components, not a {} node",
                            existing_node(doc, master)?.data.kind_tag()
                        );
                    };
                    let component = def.id;
                    for operation in operations {
                        doc.apply(operation)?;
                    }
                    component
                }
            };

            let mut instances = Vec::with_capacity(plans.len());
            for (copy, overrides) in plans {
                let original = existing_node(doc, copy)?.clone();
                let size = master_size(doc, copy).unwrap_or([100.0, 100.0]);
                let snapshot: Vec<CanvasNode> = doc
                    .scene
                    .descendants_of(copy)
                    .filter_map(|descendant| doc.scene.get(descendant).cloned())
                    .collect();
                removed_nodes.extend(snapshot.iter().map(|node| node.id));
                doc.apply(Operation::DeleteSubtree { snapshot })?;

                let mut node = CanvasNode::new(NodeData::Instance(InstanceNode {
                    component,
                    overrides,
                    prop_values: Default::default(),
                    derived: Vec::new(),
                    local_size: size,
                }));
                node.name = original.name;
                node.parent = original.parent;
                node.index = original.index;
                node.transform = original.transform;
                node.layout_child = original.layout_child;
                node.constraints = original.constraints;
                node.flags = original.flags;
                instances.push(node.id.to_string());
                doc.apply(Operation::CreateInstance {
                    node: Box::new(node),
                })
                .context("replacing a copy with an instance")?;
            }
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({
                    "component": component.to_string(),
                    "instances": instances,
                })),
            })
        }
        DesignOp::CombineVariants { ids, name } => {
            let ids = parse_content_ids(doc, ids)?;
            let name = name.as_deref().map(required_name).transpose()?;
            let edit = crate::variant_sets::combine_variants(doc, &ids, name)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::AddVariants { set, ids } => {
            let set = resolve_variant_set(doc, set)?;
            let ids = parse_content_ids(doc, ids)?;
            let edit = crate::variant_sets::add_variants(doc, set, &ids)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::RemoveVariant { id } => {
            let component = match parse_node_id(id) {
                Ok(node) => doc
                    .components
                    .defs
                    .values()
                    .find(|def| def.root == node)
                    .map(|def| def.id)
                    .with_context(|| format!("node {node} is not a component master"))?,
                Err(_) => resolve_component(doc, id)?,
            };
            let edit = crate::variant_sets::remove_variant(doc, component)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::ArrangeVariants { set } => {
            let set = resolve_variant_set(doc, set)?;
            let edit = crate::variant_sets::arrange_variants(doc, set)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::RenameComponent { component, name } => {
            let component = resolve_component(doc, component)?;
            let operations = crate::variant_sets::rename_component(doc, component, name)?;
            apply_all(doc, operations)
        }
        DesignOp::CreateComponentProperty {
            component,
            kind,
            name,
        } => {
            use crate::component_properties::CreateComponentPropertyKind as Kind;
            let component = resolve_component(doc, component)?;
            let kind = match kind {
                DesignComponentPropertyKind::Text => Kind::Text,
                DesignComponentPropertyKind::Boolean => Kind::Boolean,
                DesignComponentPropertyKind::Number => Kind::Number,
                DesignComponentPropertyKind::Color => Kind::Color,
                DesignComponentPropertyKind::InstanceSwap => Kind::InstanceSwap,
                DesignComponentPropertyKind::Variant => Kind::Variant,
            };
            let mut operations = crate::component_properties::create_component_property_operations(
                doc, component, kind,
            );
            let Some(Operation::SetComponentProps { new, .. }) = operations.first() else {
                bail!("component has no available property/variant axis to expose");
            };
            let property = new.last().context("component property was not created")?.id;
            if let Some(name) = name {
                let name = required_name(name)?;
                for operation in &mut operations {
                    if let Operation::SetComponentProps { new, .. } = operation {
                        if new
                            .iter()
                            .any(|current| current.id != property && current.name == name)
                        {
                            bail!("component property name already exists");
                        }
                        for current in new.iter_mut().filter(|current| current.id == property) {
                            current.name = name.to_owned();
                        }
                    }
                }
            }
            apply_all(doc, operations)?;
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({"property": property.to_string()})),
            })
        }
        DesignOp::BindComponentProperty {
            component,
            id,
            target,
            property,
        } => {
            let component = resolve_component(doc, component)?;
            let id = parse_node_id(id)?;
            let target = binding_property(*target);
            let definition = doc
                .components
                .def(component)
                .context("component master is missing")?;
            let node = existing_node(doc, id)?;
            if id == definition.root
                || !doc
                    .scene
                    .ancestors_of(id)
                    .any(|ancestor| ancestor.id == definition.root)
            {
                bail!("target must be a descendant of the component master");
            }
            if !target.applies_to(&node) {
                bail!("target property does not apply to this node");
            }
            let property = property
                .as_deref()
                .map(|property| find_component_property(definition, property))
                .transpose()?;
            if property.is_some_and(|property| {
                !crate::component_properties::component_property_accepts_binding(
                    &property.kind,
                    target,
                )
            }) {
                bail!("component property has an incompatible type");
            }
            let operations = crate::component_properties::set_component_property_binding_operations(
                doc,
                component,
                id,
                target,
                property.map(|property| property.id),
            );
            apply_all(doc, operations)
        }
        DesignOp::SetInstanceProperty {
            id,
            property,
            value,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            let NodeData::Instance(instance) = &node.data else {
                bail!("node is not a component instance");
            };
            let definition = doc
                .components
                .def(instance.component)
                .or_else(|| {
                    doc.components
                        .sets
                        .get(&instance.component)
                        .and_then(|set| doc.components.def(set.default_variant))
                })
                .context("instance component is missing")?;
            let property = find_component_property(definition, property)?;
            let kind = property
                .kind
                .default_value()
                .variable_type()
                .context("component property has no concrete type")?;
            let mut new = parse_variable_value(doc, kind, &[], value)?;
            if let fanta_doc::ComponentPropKind::Variant { axis } = &property.kind {
                let set = definition
                    .variant_of
                    .as_ref()
                    .and_then(|member| doc.components.sets.get(&member.set))
                    .context("variant set is missing")?;
                let allowed = set
                    .axes
                    .iter()
                    .find(|current| current.name == *axis)
                    .context("variant axis is missing")?;
                let Some(selected) = value.as_str() else {
                    bail!("variant requires an exact axis value");
                };
                if !allowed.values.iter().any(|current| current == selected) {
                    bail!("unknown variant value");
                }
            } else if property.kind == fanta_doc::ComponentPropKind::InstanceSwap {
                let component = resolve_component(
                    doc,
                    value
                        .as_str()
                        .context("instance swap requires a component id or unique name")?,
                )?;
                new = VarValue::String {
                    value: component.to_string(),
                };
            }
            let operation = Operation::SetInstanceProp {
                id,
                prop: property.id,
                old: instance.prop_values.get(&property.id).cloned(),
                new: Some(new),
            };
            apply_all(doc, vec![operation])
        }
        DesignOp::Reparent { id, parent, index } => {
            let id = parse_node_id(id)?;
            let node = doc
                .scene
                .get(id)
                .with_context(|| format!("node {id} does not exist"))?
                .clone();
            let new_parent = resolve_container(doc, parent.as_deref(), default_container)?;
            if new_parent == Some(id)
                || new_parent.is_some_and(|parent| {
                    doc.scene
                        .ancestors_of(parent)
                        .any(|ancestor| ancestor.id == id)
                })
            {
                bail!("cannot reparent {id} into its own subtree");
            }
            let world = doc
                .scene
                .world_transform(id)
                .with_context(|| format!("node {id} has no world transform"))?;
            let new_index = sibling_index(doc, new_parent, id, *index)?;
            doc.apply(Operation::Reparent {
                id,
                old_parent: node.parent,
                old_index: node.index,
                new_parent,
                new_index,
            })?;
            // Keep the node visually in place under its new parent.
            let parent_world = new_parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            let new_local = world.then(&parent_world.inverse());
            let current = doc
                .scene
                .get(id)
                .with_context(|| format!("node {id} disappeared during reparent"))?;
            if current.transform != new_local {
                doc.apply(Operation::SetTransform {
                    id,
                    old: current.transform,
                    new: new_local,
                })?;
            }
            Ok(Applied::changed())
        }
        DesignOp::Delete { id } => {
            let id = parse_node_id(id)?;
            if !doc.scene.contains(id) {
                bail!("node {id} does not exist");
            }
            if doc.pages().contains(&id) {
                bail!("refusing to delete a page root");
            }
            let snapshot: Vec<CanvasNode> = doc
                .scene
                .descendants_of(id)
                .filter_map(|descendant| doc.scene.get(descendant).cloned())
                .collect();
            removed_nodes.extend(snapshot.iter().map(|node| node.id));
            doc.apply(Operation::DeleteSubtree { snapshot })?;
            Ok(Applied::changed())
        }
        DesignOp::Select { ids } => {
            let mut parsed = Vec::with_capacity(ids.len());
            for raw in ids {
                let id = parse_node_id(raw)?;
                if !doc.scene.contains(id) {
                    bail!("node {raw} does not exist");
                }
                parsed.push(id);
            }
            doc.selection.replace_with(parsed);
            Ok(Applied::Selection)
        }
        DesignOp::SetViewport { center, zoom } => {
            if let Some(center) = center {
                if !(center[0].is_finite() && center[1].is_finite()) {
                    bail!("the viewport center must be finite");
                }
                doc.viewport.center = *center;
            }
            if let Some(zoom) = zoom {
                if !(zoom.is_finite() && *zoom > 0.0) {
                    bail!("the viewport zoom must be positive");
                }
                doc.viewport.zoom = zoom.clamp(0.01, 64.0);
            }
            Ok(Applied::Viewport)
        }
    }
}

fn resolve_container(
    doc: &Doc,
    parent: Option<&str>,
    default_container: Option<NodeId>,
) -> Result<Option<NodeId>> {
    // Follow can change the viewed page between streamed operations; viewing
    // must not change the destination captured for this batch.
    let Some(id) = parent.map(parse_node_id).transpose()?.or(default_container) else {
        return Ok(None);
    };
    let node = doc
        .scene
        .get(id)
        .with_context(|| format!("parent {id} does not exist"))?;
    if !node.can_have_children() {
        bail!(
            "parent {id} is a {} node and cannot have children",
            node.data.kind_tag()
        );
    }
    Ok(Some(id))
}

/// The fractional index for inserting at `position` among `parent`'s children
/// (excluding `moving`, which is being repositioned). `None` appends on top.
fn sibling_index(
    doc: &Doc,
    parent: Option<NodeId>,
    moving: NodeId,
    position: Option<usize>,
) -> Result<IndexKey> {
    let Some(position) = position else {
        return Ok(doc.scene.next_child_index(parent));
    };
    let mut keys: Vec<IndexKey> = doc
        .scene
        .children_of(parent)
        .iter()
        .filter(|child| **child != moving)
        .filter_map(|child| doc.scene.get(*child))
        .map(|child| child.index)
        .collect();
    keys.sort_by(|left, right| left.raw().total_cmp(&right.raw()));
    if keys.is_empty() {
        return Ok(doc.scene.next_child_index(parent));
    }
    if position == 0 {
        return Ok(IndexKey::before(keys[0]));
    }
    if position >= keys.len() {
        return Ok(IndexKey::after(keys[keys.len() - 1]));
    }
    let (left, right) = (keys[position - 1], keys[position]);
    if IndexKey::near_precision_limit(left, right) {
        bail!("the insertion gap at position {position} is exhausted; reorder the siblings first");
    }
    Ok(IndexKey::between(left, right))
}

/// The node for `id`, cloned so the borrow does not outlive later mutation.
fn existing_node(doc: &Doc, id: NodeId) -> Result<CanvasNode> {
    doc.scene
        .get(id)
        .cloned()
        .with_context(|| format!("node {id} does not exist"))
}

/// Apply a helper's operation list inside the running transaction; an empty
/// list means the request changed nothing.
fn apply_all(doc: &mut Doc, operations: Vec<Operation>) -> Result<Applied> {
    if operations.is_empty() {
        return Ok(Applied::Nothing);
    }
    for operation in operations {
        doc.apply(operation)?;
    }
    Ok(Applied::changed())
}

/// Parse a list of ids that must all name existing, non-page content nodes.
fn parse_content_ids(doc: &Doc, raw_ids: &[String]) -> Result<Vec<NodeId>> {
    let mut ids = Vec::with_capacity(raw_ids.len());
    for raw in raw_ids {
        let id = parse_node_id(raw)?;
        if !doc.scene.contains(id) {
            bail!("node {raw} does not exist");
        }
        if doc.pages().contains(&id) {
            bail!("{raw} is a page root, not a content node");
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// `[all]`, `[vertical, horizontal]` or `[top, right, bottom, left]` into the
/// layout's `[top, right, bottom, left]`.
fn parse_padding(values: &[f64]) -> Result<[f64; 4]> {
    if values
        .iter()
        .any(|value| !(value.is_finite() && *value >= 0.0))
    {
        bail!("padding values must be non-negative");
    }
    match values {
        [all] => Ok([*all; 4]),
        [vertical, horizontal] => Ok([*vertical, *horizontal, *vertical, *horizontal]),
        [top, right, bottom, left] => Ok([*top, *right, *bottom, *left]),
        _ => bail!(
            "padding takes 1, 2 or 4 values ([all], [vertical, horizontal] or [top, right, bottom, left]), not {}",
            values.len()
        ),
    }
}

/// A component by id, or by name when exactly one component carries it.
fn resolve_component(doc: &Doc, reference: &str) -> Result<ComponentId> {
    if let Ok(id) = reference.parse::<ComponentId>()
        && (doc.components.def(id).is_some() || doc.components.sets.contains_key(&id))
    {
        return Ok(id);
    }
    let mut matches = doc
        .components
        .defs
        .values()
        .filter(|def| def.name == reference)
        .map(|def| def.id)
        .chain(
            doc.components
                .sets
                .values()
                .filter(|set| set.name == reference)
                .map(|set| set.id),
        );
    let first = matches
        .next()
        .with_context(|| format!("no component is named or identified by `{reference}`"))?;
    if matches.next().is_some() {
        bail!("several components are named `{reference}`; use the component id");
    }
    Ok(first)
}

/// Apply a variant-set edit and report the set and its frame.
fn apply_variant_edit(doc: &mut Doc, edit: crate::variant_sets::SetEdit) -> Result<Applied> {
    for operation in edit.operations {
        doc.apply(operation)?;
    }
    Ok(Applied::Content {
        created: None,
        detail: Some(json!({
            "component_set": edit.set.to_string(),
            "frame": edit.frame.map(|frame| frame.to_string()),
        })),
    })
}

fn resolve_variant_set(doc: &Doc, reference: &str) -> Result<ComponentId> {
    let id = resolve_component(doc, reference)?;
    if !doc.components.sets.contains_key(&id) {
        bail!("`{reference}` is a component, not a variant set");
    }
    Ok(id)
}

/// The size a new instance takes: the master's frame box, or its content
/// bounds for a plain group.
fn master_size(doc: &Doc, root: NodeId) -> Option<[f64; 2]> {
    let node = doc.scene.get(root)?;
    if let NodeData::Group(group) = &node.data
        && let Some(size) = group.clip_size.or(group.local_size)
    {
        return Some(size);
    }
    let bounds = doc.scene.local_bounds(root)?;
    Some([bounds.width(), bounds.height()])
}

/// The new z-slot for `set_index`, or `None` when the node is already there.
fn layer_position_index(
    doc: &Doc,
    parent: Option<NodeId>,
    id: NodeId,
    current: IndexKey,
    position: LayerPosition,
) -> Result<Option<IndexKey>> {
    let siblings: Vec<IndexKey> = doc
        .scene
        .children_of(parent)
        .iter()
        .filter(|sibling| **sibling != id)
        .filter_map(|sibling| doc.scene.get(*sibling))
        .map(|sibling| sibling.index)
        .collect();
    let above = siblings.iter().copied().filter(|index| *index > current);
    let below = siblings.iter().copied().filter(|index| *index < current);
    let new = match position {
        LayerPosition::Named(NamedLayerPosition::Front) => match siblings.last() {
            Some(top) if *top > current => IndexKey::after(*top),
            _ => return Ok(None),
        },
        LayerPosition::Named(NamedLayerPosition::Back) => match siblings.first() {
            Some(bottom) if *bottom < current => IndexKey::before(*bottom),
            _ => return Ok(None),
        },
        LayerPosition::Named(NamedLayerPosition::Forward) => {
            let mut above = above;
            let Some(next) = above.next() else {
                return Ok(None);
            };
            match above.next() {
                Some(after_next) => {
                    if IndexKey::near_precision_limit(next, after_next) {
                        bail!(
                            "the z-order gap above {id} is exhausted; reorder the siblings first"
                        );
                    }
                    IndexKey::between(next, after_next)
                }
                None => IndexKey::after(next),
            }
        }
        LayerPosition::Named(NamedLayerPosition::Backward) => {
            let mut below: Vec<IndexKey> = below.collect();
            let Some(previous) = below.pop() else {
                return Ok(None);
            };
            match below.pop() {
                Some(before_previous) => {
                    if IndexKey::near_precision_limit(before_previous, previous) {
                        bail!(
                            "the z-order gap below {id} is exhausted; reorder the siblings first"
                        );
                    }
                    IndexKey::between(before_previous, previous)
                }
                None => IndexKey::before(previous),
            }
        }
        LayerPosition::Absolute { index } => {
            let current_position = siblings
                .iter()
                .filter(|sibling| **sibling < current)
                .count();
            let already_on_top = index >= siblings.len() && current_position == siblings.len();
            if index == current_position || already_on_top {
                return Ok(None);
            }
            sibling_index(doc, parent, id, Some(index))?
        }
    };
    Ok(Some(new))
}

fn parse_fill_color(raw: &str) -> Result<Color> {
    parse_color(raw)
        .with_context(|| format!("`{raw}` is not a valid hex color (use #RRGGBB or #RRGGBBAA)"))
}

/// Replace the node's primary paint with a solid color: a vector/boolean's
/// first fill, a frame's background, a text node's glyph color.
fn design_shadow(shadow: &DesignShadow, kind: ShadowKind) -> Result<Shadow> {
    let defaults = default_shadow();
    for (label, value) in [
        ("x", shadow.x),
        ("y", shadow.y),
        ("blur", shadow.blur),
        ("spread", shadow.spread),
    ] {
        if value.is_some_and(|value| !value.is_finite()) {
            bail!("the shadow {label} must be finite");
        }
    }
    if shadow.blur.is_some_and(|blur| blur < 0.0) {
        bail!("the shadow blur must be non-negative");
    }
    Ok(Shadow {
        kind,
        color: match &shadow.color {
            Some(color) => parse_fill_color(color)?,
            None => defaults.color,
        },
        blur: shadow.blur.unwrap_or(defaults.blur),
        spread: shadow.spread.unwrap_or(defaults.spread),
        offset: [
            shadow.x.unwrap_or(defaults.offset[0]),
            shadow.y.unwrap_or(defaults.offset[1]),
        ],
        ..defaults
    })
}

fn design_fill(
    paint: &DesignPaint,
    assets: &mut dyn ImageStore,
    ingested_assets: &mut Vec<AssetId>,
) -> Result<Fill> {
    let stops =
        |stops: &[fanta_design_ops::DesignGradientStop]| -> Result<Vec<fanta_doc::GradientStop>> {
            if stops.len() < 2 {
                bail!("a gradient needs at least two stops");
            }
            stops
                .iter()
                .map(|stop| {
                    if !(stop.position.is_finite() && (0.0..=1.0).contains(&stop.position)) {
                        bail!("gradient stop positions must be between 0 and 1");
                    }
                    Ok(fanta_doc::GradientStop {
                        position: stop.position,
                        color: parse_fill_color(&stop.color)?,
                    })
                })
                .collect()
        };
    let point = |value: Option<[f32; 2]>, default: [f32; 2]| -> Result<[f32; 2]> {
        let point = value.unwrap_or(default);
        if !point.iter().all(|coordinate| coordinate.is_finite()) {
            bail!("gradient points must be finite");
        }
        Ok(point)
    };
    Ok(match paint {
        DesignPaint::Solid { color } => Fill::solid(parse_fill_color(color)?),
        DesignPaint::Linear {
            from,
            to,
            stops: list,
        } => Fill::Gradient {
            gradient: fanta_doc::Gradient::Linear {
                start: point(*from, [0.5, 0.0])?,
                end: point(*to, [0.5, 1.0])?,
                stops: stops(list)?,
            },
            blend: fanta_doc::BlendMode::Normal,
        },
        DesignPaint::Radial {
            center,
            radius,
            stops: list,
        } => {
            let radius = radius.unwrap_or(0.5);
            if !(radius.is_finite() && radius > 0.0) {
                bail!("the gradient radius must be positive");
            }
            Fill::Gradient {
                gradient: fanta_doc::Gradient::Radial {
                    center: point(*center, [0.5, 0.5])?,
                    radius,
                    handles: None,
                    stops: stops(list)?,
                },
                blend: fanta_doc::BlendMode::Normal,
            }
        }
        DesignPaint::Image { source, fit } => {
            let bytes = decode_image_source(source)?;
            let (asset, _natural_size, inserted) = assets.add_image_tracked(bytes)?;
            if inserted {
                ingested_assets.push(asset);
            }
            Fill::Image {
                asset,
                mode: match fit.unwrap_or(DesignImageFit::Fill) {
                    DesignImageFit::Fill => ImageFitMode::Fill,
                    DesignImageFit::Fit => ImageFitMode::Fit,
                    DesignImageFit::Stretch => ImageFitMode::Stretch,
                    DesignImageFit::Tile => ImageFitMode::Tile,
                },
                opacity: 1.0,
                crop: None,
                adjust: Default::default(),
                scale: None,
                rotation: None,
                blend: fanta_doc::BlendMode::Normal,
            }
        }
    })
}

fn set_solid_fill(data: &mut NodeData, color: Color) {
    match data {
        NodeData::Vector(vector) => {
            if let Some(fill) = vector.fills.first_mut() {
                *fill = Fill::solid(color);
            } else {
                vector.fills.push(Fill::solid(color));
            }
        }
        NodeData::Boolean(boolean) => {
            if let Some(fill) = boolean.fills.first_mut() {
                *fill = Fill::solid(color);
            } else {
                boolean.fills.push(Fill::solid(color));
            }
        }
        NodeData::Group(group) => group.background = Some(Fill::solid(color)),
        NodeData::Text(text) => text.set_glyph_color(color),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use base64::Engine as _;
    use fanta_render::AssetResolver;

    use fanta_design_ops::DesignSystemQuery;
    use fanta_doc::Bounds;

    use super::*;
    use crate::assets::{AssetStores, OverlayAssetResolver};
    use crate::scene::{
        COMPONENT_OVERVIEW_LIMIT, EMPTY_SPACE_MARGIN, PAGE_LISTING_NODE_BUDGET, SUMMARY_TEXT_CHARS,
        check_page_listing_budget, components_json, design_system_json, empty_space, node_summary,
        page_area_is_empty, paginated_node_summary,
    };

    /// Owned backing for [`AssetStores`], for tests that place images.
    #[derive(Default)]
    struct TestImageStore {
        raw_assets: Arc<BTreeMap<AssetId, Vec<u8>>>,
        asset_resolver: Option<Arc<dyn AssetResolver>>,
        overlay: Option<Arc<OverlayAssetResolver>>,
    }

    impl TestImageStore {
        fn stores(&mut self) -> AssetStores<'_> {
            AssetStores::new(
                &mut self.raw_assets,
                &mut self.asset_resolver,
                &mut self.overlay,
            )
        }

        fn raw_assets(&self) -> &BTreeMap<AssetId, Vec<u8>> {
            &self.raw_assets
        }

        fn resolver(&self) -> Option<&Arc<dyn AssetResolver>> {
            self.asset_resolver.as_ref()
        }
    }

    /// Run a batch the way the editor does: parentless ops land on the active
    /// page.
    fn run_batch_with(
        doc: &mut Doc,
        assets: &mut dyn ImageStore,
        ops: &[DesignOp],
        label: &str,
    ) -> BatchOutcome {
        let default_container = doc.active_page();
        apply_batch(doc, assets, ops, label, default_container)
    }

    /// Run a batch against throwaway asset stores (for tests that don't
    /// place images).
    fn run_batch(doc: &mut Doc, ops: &[DesignOp], label: &str) -> BatchOutcome {
        let mut stores = TestImageStore::default();
        run_batch_with(doc, &mut stores.stores(), ops, label)
    }

    fn ops(value: Value) -> Vec<DesignOp> {
        serde_json::from_value(value).expect("ops JSON matches the DesignOp schema")
    }

    fn doc_with_page() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        doc.history = Default::default();
        (doc, page_id)
    }

    fn created_id(outcome: &BatchOutcome, index: usize) -> NodeId {
        outcome.value["created"][index]
            .as_str()
            .expect("created id present")
            .parse()
            .expect("created id parses")
    }

    /// A tiny valid PNG (2x1, opaque) as base64.
    fn tiny_png_base64() -> String {
        let mut png = Vec::new();
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encoding the fixture PNG");
        base64::engine::general_purpose::STANDARD.encode(&png)
    }

    fn assert_applied(outcome: &BatchOutcome) {
        assert_eq!(outcome.value["applied"], json!(true), "{}", outcome.value);
    }

    #[test]
    fn presence_batches_preserve_an_open_content_transaction() -> Result<()> {
        let (mut document, page) = doc_with_page();
        let original_name = document.scene.get(page).context("page")?.name.clone();
        document.history.begin("Original edit", &mut document.scene);
        document.apply(Operation::SetName {
            id: page,
            old: original_name.clone(),
            new: "Pending name".into(),
        })?;
        for operations in [
            ops(json!([
                {"op":"set_viewport","center":[100.0,200.0],"zoom":0.5},
                {"op":"select","ids":[]}
            ])),
            ops(json!([{"op":"set_viewport","zoom":-1.0}])),
        ] {
            run_batch(&mut document, &operations, "Navigate");
            assert_eq!(document.history.undo_depth(), 0);
            assert_eq!(
                document.scene.get(page).context("page")?.name,
                "Pending name"
            );
        }
        document.abort_transaction()?;
        assert_eq!(
            document.scene.get(page).context("page")?.name,
            original_name
        );
        assert_eq!(document.viewport.center, [100.0, 200.0]);
        assert_eq!(document.viewport.zoom, 0.5);
        Ok(())
    }

    #[test]
    fn agent_design_system_creates_modes_bindings_and_persists_sources() -> Result<()> {
        let (mut doc, _) = doc_with_page();
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable_collection","name":"Theme","modes":["Light","Dark"]},
                {"op":"create_variable","collection":"Theme","name":"surface/accent","kind":"color","value":"#2255EE"},
                {"op":"create_node","node_type":"rectangle","x":0,"y":0,"width":128,"height":48,"fill":"#FFFFFF"}
            ])),
            "Create foundations",
        );
        assert_eq!(result.value["applied"], true);
        let node = created_id(&result, 0);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_variable_value","variable":"surface/accent","mode":"Dark","value":"#88AAFF"},
                {"op":"bind_variable","id":node.to_string(),"property":{"prop":"fill_color"},"variable":"surface/accent"},
                {"op":"set_variable_mode","collection":"Theme","mode":"Dark"},
                {"op":"add_variable_mode","collection":"Theme","name":"Contrast"}
            ])),
            "Bind and theme",
        );
        assert_eq!(result.value["applied"], true);
        let variable = find_variable(&doc, "surface/accent")?;
        let collection = find_collection(&doc, "Theme")?;
        assert_eq!(variable.values_by_mode.len(), 3);
        assert_eq!(
            variable
                .values_by_mode
                .get(&find_mode(collection, "Contrast")?),
            variable
                .values_by_mode
                .get(&find_mode(collection, "Light")?)
        );
        let variable_id = variable.id;
        let resolved = fanta_doc::resolve_bound_value(
            &doc.variables,
            &doc.scene,
            node,
            &doc.active_modes,
            variable_id,
        )
        .context("resolved token")?;
        let mut painted = existing_node(&doc, node)?;
        BoundProp::FillColor { index: 0 }.apply_resolved(&mut painted, resolved);
        let NodeData::Vector(vector) = &painted.data else {
            bail!("expected a vector");
        };
        assert_eq!(
            vector.fills.first(),
            Some(&Fill::solid(parse_fill_color("#88AAFF")?))
        );
        doc.selection.replace_with([node]);
        let state = design_system_json(
            &doc,
            None,
            DesignSystemQuery {
                include_bindings: true,
                ..Default::default()
            },
            |_| None,
        )?;
        assert_eq!(state["variable_count"], 1);
        assert_eq!(
            state["selection_bindings"][0]["bindings"][0]["variable"],
            variable_id.to_string()
        );
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        let (saved, _) = fanta_format::read_project_tree(directory.path())?;
        assert_eq!(saved.variables, doc.variables);
        assert_eq!(saved.active_modes, doc.active_modes);
        assert_eq!(
            saved.scene.get(node).context("saved node")?.bindings,
            doc.scene.get(node).context("node")?.bindings
        );
        doc.undo()?;
        assert!(doc.active_modes.is_empty());
        assert!(doc.scene.get(node).context("node")?.bindings.is_empty());
        assert_eq!(find_collection(&doc, "Theme")?.modes.len(), 2);
        Ok(())
    }

    #[test]
    fn agent_enum_and_image_variables_carry_options_and_bind() -> Result<()> {
        let (mut doc, _) = doc_with_page();
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable_collection","name":"Theme","modes":["Light","Dark"]},
                {"op":"create_variable","collection":"Theme","name":"state","kind":"enum","value":"idle","options":["idle","hover"]},
                {"op":"create_variable","collection":"Theme","name":"hero","kind":"image","value":null},
                {"op":"create_node","node_type":"rectangle","x":0,"y":0,"width":128,"height":48,"fill":"#FFFFFF"}
            ])),
            "Create enum and image tokens",
        );
        assert_eq!(result.value["applied"], true);
        let node = created_id(&result, 0);
        let state = find_variable(&doc, "state")?;
        assert_eq!(state.ty, VariableType::Enum);
        assert_eq!(state.options, vec!["idle", "hover"]);
        assert_eq!(find_variable(&doc, "hero")?.ty, VariableType::Image);

        // A value outside the option list is refused, so the batch rolls back.
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_variable_value","variable":"state","mode":"Dark","value":"pressed"}
            ])),
            "Unknown option",
        );
        assert_eq!(result.value["applied"], false);

        let asset = fanta_doc::AssetId::new().to_string();
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_variable_options","variable":"state","options":["idle","hover","pressed"]},
                {"op":"set_variable_value","variable":"state","mode":"Dark","value":"pressed"},
                {"op":"set_variable_value","variable":"hero","mode":"Dark","value":asset},
                {"op":"bind_variable","id":node.to_string(),"property":{"prop":"image_fill","index":0},"variable":"hero"}
            ])),
            "Options, values and an image binding",
        );
        assert_eq!(result.value["applied"], true);
        let dark = find_mode(find_collection(&doc, "Theme")?, "Dark")?;
        let state = find_variable(&doc, "state")?;
        assert_eq!(state.options, vec!["idle", "hover", "pressed"]);
        assert_eq!(
            state.values_by_mode.get(&dark),
            Some(&VarValue::Enum {
                value: "pressed".into()
            })
        );
        let hero = find_variable(&doc, "hero")?;
        assert_eq!(
            hero.values_by_mode.get(&dark),
            Some(&VarValue::Image {
                value: Some(asset.parse()?)
            })
        );
        assert_eq!(
            existing_node(&doc, node)?
                .bindings
                .get(&BoundProp::ImageFill { index: 0 }),
            Some(&hero.id)
        );

        // Options belong to enums only, and an enum needs at least one.
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable","collection":"Theme","name":"size","kind":"float","value":8,"options":["a"]}
            ])),
            "Options on a float",
        );
        assert_eq!(result.value["applied"], false);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable","collection":"Theme","name":"empty","kind":"enum","value":"x"}
            ])),
            "Enum without options",
        );
        assert_eq!(result.value["applied"], false);
        Ok(())
    }

    #[test]
    fn agent_design_system_rejects_invalid_aliases_and_rolls_back_variables() -> Result<()> {
        let (mut doc, _) = doc_with_page();
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable_collection","name":"Theme"},
                {"op":"create_variable","collection":"Theme","name":"radius","kind":"float","value":8},
                {"op":"create_variable","collection":"Theme","name":"accent","kind":"color","value":{"alias":"radius"}}
            ])),
            "Invalid system",
        );
        assert_eq!(result.value["applied"], false);
        assert!(doc.variables.is_empty());
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable_collection","name":"Theme"},
                {"op":"create_variable","collection":"Theme","name":"a","kind":"float","value":8},
                {"op":"create_variable","collection":"Theme","name":"b","kind":"float","value":{"alias":"a"}},
                {"op":"set_variable_value","variable":"a","mode":"Default","value":{"alias":"b"}}
            ])),
            "Alias cycle",
        );
        assert_eq!(result.value["applied"], false);
        assert!(doc.variables.is_empty());
        Ok(())
    }

    #[test]
    fn agent_design_system_pages_large_component_libraries_before_resolving_sources() -> Result<()>
    {
        use fanta_doc::{
            ComponentDef, ComponentPropDef, ComponentPropId, ComponentPropKind, ComponentSet,
        };

        let (mut doc, page) = doc_with_page();
        for index in 0..9_012 {
            let id = ComponentId::from_u128(index + 1);
            let mut definition = ComponentDef::new(id, page, format!("Component {index}"));
            if index == 120 {
                definition.props.push(ComponentPropDef {
                    id: ComponentPropId::new(),
                    name: "Label".into(),
                    kind: ComponentPropKind::Text,
                    formatter: Default::default(),
                    default: VarValue::String {
                        value: "Export".into(),
                    },
                    bindings: Vec::new(),
                });
            }
            doc.components.defs.insert(id, definition);
            if index < 250 {
                let set = ComponentId::from_u128(10_000 + index);
                doc.components.sets.insert(
                    set,
                    ComponentSet {
                        id: set,
                        name: format!("Set {index}"),
                        axes: Vec::new(),
                        members: vec![id],
                        default_variant: id,
                        root: None,
                    },
                );
            }
        }
        let source_calls = std::cell::Cell::new(0);
        let overview = components_json(&doc, None, 0, COMPONENT_OVERVIEW_LIMIT, |_| {
            source_calls.set(source_calls.get() + 1);
            None
        });
        assert_eq!(overview.len(), COMPONENT_OVERVIEW_LIMIT);
        assert_eq!(source_calls.get(), COMPONENT_OVERVIEW_LIMIT);

        source_calls.set(0);
        let window = design_system_json(
            &doc,
            None,
            DesignSystemQuery {
                offset: Some(120),
                limit: Some(23),
                ..Default::default()
            },
            |_| {
                source_calls.set(source_calls.get() + 1);
                None
            },
        )?;
        assert_eq!(source_calls.get(), 23);
        assert_eq!(window["component_count"], 9_012);
        assert_eq!(window["component_set_count"], 250);
        assert_eq!(window["components_offset"], 120);
        assert_eq!(window["components_limit"], 23);
        assert_eq!(
            window["components"].as_array().context("components")?.len(),
            23
        );
        assert_eq!(
            window["component_sets"].as_array().context("sets")?.len(),
            23
        );
        assert_eq!(window["components"][0]["name"], "Component 120");
        assert_eq!(window["component_sets"][0]["name"], "Set 120");
        assert_eq!(window["components"][0]["properties"][0]["name"], "Label");
        assert_eq!(window["more_components"], true);
        assert_eq!(window["more_component_sets"], true);
        assert_eq!(window["more_variables"], false);

        let tail = design_system_json(
            &doc,
            None,
            DesignSystemQuery {
                offset: Some(9_010),
                limit: Some(usize::MAX),
                ..Default::default()
            },
            |_| None,
        )?;
        assert_eq!(tail["components"].as_array().context("tail")?.len(), 2);
        assert_eq!(tail["components_limit"], 200);
        assert_eq!(tail["more_components"], false);
        assert_eq!(tail["more_component_sets"], false);

        let past_end = design_system_json(
            &doc,
            None,
            DesignSystemQuery {
                offset: Some(usize::MAX),
                ..Default::default()
            },
            |_| None,
        )?;
        assert!(
            past_end["components"]
                .as_array()
                .context("past end")?
                .is_empty()
        );
        assert_eq!(past_end["more_components"], false);
        Ok(())
    }

    #[test]
    fn agent_component_properties_and_variant_sets_drive_real_instances() -> Result<()> {
        let (mut doc, page) = doc_with_page();
        let mut masters = Vec::new();
        let mut labels = Vec::new();
        for name in ["Default", "Hover"] {
            let mut master = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([128.0, 48.0]),
                ..Default::default()
            }));
            master.name = name.into();
            master.parent = Some(page);
            let root = master.id;
            doc.apply(Operation::create_node(master))?;
            let mut label = CanvasNode::new(NodeData::Text(TextNode::new(name, 100.0, 20.0)));
            label.parent = Some(root);
            labels.push(label.id);
            doc.apply(Operation::create_node(label))?;
            masters.push(root);
        }
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_component","id":masters[0].to_string()},
                {"op":"create_component","id":masters[1].to_string()},
                {"op":"combine_variants","ids":masters.iter().map(ToString::to_string).collect::<Vec<_>>(),"name":"Button"},
                {"op":"create_component_property","component":"Default","kind":"text","name":"Label"},
                {"op":"bind_component_property","component":"Default","id":labels[0].to_string(),"target":{"prop":"text_content"},"property":"Label"},
                {"op":"create_component_property","component":"Default","kind":"variant","name":"State"},
                {"op":"create_instance","component":"Button","x":200,"y":100}
            ])),
            "Create reusable button",
        );
        assert_eq!(result.value["applied"], true, "{}", result.value);
        let instance = created_id(&result, 0);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_instance_property","id":instance.to_string(),"property":"Label","value":"Continue"}
            ])),
            "Customize label",
        );
        assert_eq!(result.value["applied"], true);
        let expanded_text = |doc: &Doc| -> Result<Vec<String>> {
            let NodeData::Instance(instance) = &doc.scene.get(instance).context("instance")?.data
            else {
                bail!("expected instance");
            };
            Ok(
                fanta_doc::expand_instance(&doc.scene, &doc.components, instance)
                    .into_iter()
                    .filter_map(|expanded| match expanded.node.data {
                        NodeData::Text(text) => Some(text.content),
                        _ => None,
                    })
                    .collect(),
            )
        };
        assert_eq!(expanded_text(&doc)?, vec!["Continue"]);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_instance_property","id":instance.to_string(),"property":"State","value":"Hover"}
            ])),
            "Use hover variant",
        );
        assert_eq!(result.value["applied"], true);
        assert_eq!(expanded_text(&doc)?, vec!["Hover"]);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_instance_property","id":instance.to_string(),"property":"State","value":"Unknown"}
            ])),
            "Reject unknown variant",
        );
        assert_eq!(result.value["applied"], false);
        assert_eq!(expanded_text(&doc)?, vec!["Hover"]);
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        let (saved, _) = fanta_format::read_project_tree(directory.path())?;
        assert_eq!(expanded_text(&saved)?, vec!["Hover"]);
        Ok(())
    }

    #[test]
    fn create_image_ingests_the_asset_and_places_a_bitmap_node() {
        let (mut doc, page_id) = doc_with_page();
        let mut stores = TestImageStore::default();
        let outcome = run_batch_with(
            &mut doc,
            &mut stores.stores(),
            &ops(json!([
                {"op": "create_image", "source": tiny_png_base64(), "name": "Hero",
                 "x": 5.0, "y": 7.0, "width": 200.0,
                 "meta": {"generation": {"prompt": "a hero", "model": "m1", "generation_id": "g1"}}},
            ])),
            "Place image",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        let id = created_id(&outcome, 0);
        let node = doc.scene.get(id).unwrap();
        assert_eq!(node.parent, Some(page_id));
        assert_eq!(node.name, "Hero");
        let NodeData::Bitmap(bitmap) = &node.data else {
            panic!("expected a bitmap node");
        };
        assert_eq!(bitmap.natural_size, [2, 1]);
        // One dimension given: the other follows the 2:1 natural aspect.
        assert_eq!(bitmap.local_size, [200.0, 100.0]);
        assert_eq!(
            node.meta["generation"]["prompt"],
            json!("a hero"),
            "provenance lands in node meta"
        );

        // The asset is in the raw store (for save) and resolvable (for render).
        assert_eq!(stores.raw_assets().len(), 1);
        assert!(stores.raw_assets().contains_key(&bitmap.asset));
        let resolved = stores
            .resolver()
            .expect("resolver present after ingest")
            .resolve(bitmap.asset)
            .expect("the new asset resolves");
        assert_eq!((resolved.width, resolved.height), (2, 1));
    }

    #[test]
    fn failing_batch_rolls_back_ingested_assets() {
        let (mut doc, _) = doc_with_page();
        let mut stores = TestImageStore::default();
        let outcome = run_batch_with(
            &mut doc,
            &mut stores.stores(),
            &ops(json!([
                {"op": "create_image", "source": tiny_png_base64(),
                 "x": 0.0, "y": 0.0},
                {"op": "delete", "id": "not-a-node"},
            ])),
            "Broken image batch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert_eq!(
            stores.raw_assets().len(),
            0,
            "the ingested asset was rolled back"
        );
    }

    #[test]
    fn create_image_refuses_urls_and_junk() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_image", "source": "https://example.com/cat.png",
                 "x": 0.0, "y": 0.0},
            ])),
            "URL image",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        let error = outcome.value["ops"][0]["error"].as_str().unwrap();
        assert!(
            error.contains("place_generation"),
            "error steers to the tool: {error}"
        );

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_image", "source": "bm90IGFuIGltYWdl", // "not an image"
                 "x": 0.0, "y": 0.0},
            ])),
            "Junk image",
        );
        assert_eq!(outcome.value["applied"], json!(false));
    }

    #[test]
    fn create_batch_places_nodes_on_the_active_page_as_one_undo_step() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 10.0, "y": 20.0, "width": 200.0, "height": 100.0, "fill": "#FFFFFF"},
                {"op": "create_node", "node_type": "text", "text": "Hello",
                 "x": 26.0, "y": 36.0, "width": 120.0, "height": 24.0, "font_size": 14.0},
            ])),
            "Create card",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        assert_eq!(outcome.effect, BatchEffect::Content);

        let frame = created_id(&outcome, 0);
        let text = created_id(&outcome, 1);
        assert_eq!(doc.scene.get(frame).unwrap().parent, Some(page_id));
        assert_eq!(doc.scene.get(frame).unwrap().name, "Card");
        let bounds = doc.scene.world_bounds(frame).unwrap();
        assert_eq!(
            (bounds.min_x, bounds.min_y, bounds.width(), bounds.height()),
            (10.0, 20.0, 200.0, 100.0)
        );
        let NodeData::Text(text_node) = &doc.scene.get(text).unwrap().data else {
            panic!("expected a text node");
        };
        assert_eq!(text_node.content, "Hello");
        assert_eq!(text_node.style.size_px, 14.0);

        // The whole batch is one undo step.
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.undo().unwrap());
        assert!(!doc.scene.contains(frame));
        assert!(!doc.scene.contains(text));
    }

    #[test]
    fn set_props_addresses_world_bounds_and_restyles_in_place() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 40.0, "height": 40.0},
            ])),
            "Create",
        );
        let id = created_id(&outcome, 0);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_props", "id": id.to_string(), "x": 50.0, "y": 60.0,
                 "fill": "#FF0000", "corner_radius": 8.0, "opacity": 0.5},
            ])),
            "Restyle",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        let bounds = doc.scene.world_bounds(id).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (50.0, 60.0));
        let node = doc.scene.get(id).unwrap();
        assert_eq!(node.opacity, UnitInterval::new(0.5));
        let NodeData::Vector(vector) = &node.data else {
            panic!("expected a vector node");
        };
        assert_eq!(vector.corner_radius, Some(8.0));
        assert_eq!(
            vector.fills.first().and_then(Fill::solid_color),
            parse_color("#FF0000")
        );
    }

    #[test]
    fn reparent_preserves_world_position() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 100.0, "y": 100.0, "width": 300.0, "height": 200.0},
                {"op": "create_node", "node_type": "rectangle",
                 "x": 120.0, "y": 130.0, "width": 40.0, "height": 40.0},
            ])),
            "Create",
        );
        let frame = created_id(&outcome, 0);
        let rect = created_id(&outcome, 1);
        assert_eq!(doc.scene.get(rect).unwrap().parent, Some(page_id));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "reparent", "id": rect.to_string(), "parent": frame.to_string()},
            ])),
            "Nest",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        assert_eq!(doc.scene.get(rect).unwrap().parent, Some(frame));
        let bounds = doc.scene.world_bounds(rect).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (120.0, 130.0));
    }

    #[test]
    fn failing_op_rolls_back_the_whole_batch() {
        let (mut doc, _) = doc_with_page();
        let nodes_before = doc.scene.len();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 40.0, "height": 40.0},
                {"op": "delete", "id": "not-a-node"},
            ])),
            "Broken batch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert_eq!(outcome.effect, BatchEffect::None);
        assert_eq!(outcome.value["ops"][1]["status"], json!("failed"));
        assert_eq!(outcome.value["ops"][0]["status"], json!("ok"));
        assert_eq!(
            doc.scene.len(),
            nodes_before,
            "the created node was rolled back"
        );
        assert_eq!(
            doc.history.undo_depth(),
            0,
            "no undo step for a failed batch"
        );
    }

    #[test]
    fn delete_removes_the_subtree_and_prunes_selection() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0},
            ])),
            "Create",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "ellipse", "parent": frame.to_string(),
                 "x": 10.0, "y": 10.0, "width": 20.0, "height": 20.0},
            ])),
            "Fill in",
        );
        let ellipse = created_id(&outcome, 0);
        doc.selection.select_only(ellipse);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "delete", "id": frame.to_string()}])),
            "Delete",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        assert!(!doc.scene.contains(frame));
        assert!(!doc.scene.contains(ellipse));
        assert!(doc.selection.iter().next().is_none());
    }

    /// Delete and ungroup prune the selection only once the batch commits:
    /// a batch that rolls back restores the nodes, so it must hand back the
    /// selection it started with too, or the caller would lose the user's
    /// selection to an edit that never happened.
    #[test]
    fn a_failed_batch_leaves_the_selection_untouched() {
        let (mut doc, _) = doc_with_page();
        let first = create_rect(&mut doc, 0.0, 0.0, 20.0, 20.0);
        let second = create_rect(&mut doc, 40.0, 0.0, 20.0, 20.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "group", "ids": [second.to_string()]}])),
            "Group",
        );
        assert_applied(&outcome);
        let group = created_id(&outcome, 0);
        doc.selection.replace_with([first, group]);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "delete", "id": first.to_string()},
                {"op": "ungroup", "id": group.to_string()},
                {"op": "delete", "id": "not-a-node"},
            ])),
            "Broken batch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert!(doc.scene.contains(first));
        assert!(doc.scene.contains(group));
        assert_eq!(
            doc.selection.iter().copied().collect::<Vec<_>>(),
            vec![first, group]
        );

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "delete", "id": first.to_string()},
                {"op": "ungroup", "id": group.to_string()},
            ])),
            "Working batch",
        );
        assert_applied(&outcome);
        assert!(doc.selection.is_empty());
    }

    fn create_rect(doc: &mut Doc, x: f64, y: f64, width: f64, height: f64) -> NodeId {
        let outcome = run_batch(
            doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": x, "y": y, "width": width, "height": height},
            ])),
            "Create",
        );
        assert_eq!(outcome.value["applied"], json!(true), "{}", outcome.value);
        created_id(&outcome, 0)
    }

    #[test]
    fn empty_space_is_the_origin_on_an_empty_page_and_beside_content_otherwise() {
        let (mut doc, page_id) = doc_with_page();
        assert_eq!(empty_space(&doc, page_id, 100.0, 50.0), glam::DVec2::ZERO);

        create_rect(&mut doc, 10.0, 20.0, 200.0, 100.0);
        let spot = empty_space(&doc, page_id, 100.0, 50.0);
        assert_eq!((spot.x, spot.y), (210.0 + EMPTY_SPACE_MARGIN, 20.0));
        assert!(page_area_is_empty(
            &doc,
            page_id,
            Bounds::from_xywh(spot.x, spot.y, 100.0, 50.0)
        ));
        assert!(!page_area_is_empty(
            &doc,
            page_id,
            Bounds::from_xywh(0.0, 0.0, 50.0, 50.0)
        ));
    }

    #[test]
    fn empty_space_never_overlaps_existing_top_level_content() {
        let (mut doc, page_id) = doc_with_page();
        create_rect(&mut doc, 0.0, 0.0, 100.0, 100.0);
        // An empty frame is invisible to the leaf index but still occupies
        // the page; the spot must clear it too.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 100.0 + EMPTY_SPACE_MARGIN, "y": 0.0, "width": 300.0, "height": 100.0},
            ])),
            "Frame",
        );
        assert_applied(&outcome);
        let spot = empty_space(&doc, page_id, 50.0, 50.0);
        let rect = Bounds::from_xywh(spot.x, spot.y, 50.0, 50.0);
        assert!(page_area_is_empty(&doc, page_id, rect));
        for child in doc.scene.children_of(Some(page_id)) {
            let bounds = doc.scene.world_bounds(*child).unwrap();
            assert!(!bounds.intersects(&rect), "{rect:?} overlaps {bounds:?}");
        }
    }

    #[test]
    fn state_style_ops_write_stroke_shadow_and_text_style() {
        let (mut doc, _) = doc_with_page();
        let rect = create_rect(&mut doc, 0.0, 0.0, 40.0, 40.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": "Hi",
                 "x": 0.0, "y": 0.0, "width": 80.0, "height": 20.0},
            ])),
            "Text",
        );
        let text = created_id(&outcome, 0);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_stroke", "id": rect.to_string(), "color": "#112233", "width": 3.0,
                 "align": "inside"},
                {"op": "set_shadow", "id": rect.to_string(), "color": "#00000080", "y": 6.0,
                 "blur": 12.0},
                {"op": "set_text_style", "id": text.to_string(), "font_family": "Inter",
                 "font_weight": 600, "font_size": 24.0, "line_height": 1.2,
                 "letter_spacing": -0.5, "align": "center", "color": "#FF0000"},
            ])),
            "Style",
        );
        assert_applied(&outcome);
        let NodeData::Vector(vector) = &doc.scene.get(rect).unwrap().data else {
            panic!("expected a vector");
        };
        assert_eq!(vector.strokes.len(), 1);
        assert_eq!(vector.strokes[0].width, 3.0);
        assert_eq!(vector.strokes[0].align, StrokeAlign::Inside);
        assert_eq!(
            vector.strokes[0].paint.solid_color(),
            parse_color("#112233")
        );
        let effects = &doc.scene.get(rect).unwrap().effects;
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].offset, [0.0, 6.0]);
        assert_eq!(effects[0].blur, 12.0);
        let NodeData::Text(text_node) = &doc.scene.get(text).unwrap().data else {
            panic!("expected text");
        };
        assert_eq!(text_node.style.font_family, "Inter");
        assert_eq!(text_node.style.weight, 600);
        assert_eq!(text_node.style.size_px, 24.0);
        assert_eq!(text_node.style.line_height, 1.2);
        assert_eq!(text_node.style.letter_spacing, -0.5);
        assert_eq!(text_node.align, TextAlign::Center);
        assert_eq!(text_node.style.color, parse_color("#FF0000").unwrap());

        // Width 0 removes the stroke; remove drops the shadow.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_stroke", "id": rect.to_string(), "width": 0.0},
                {"op": "set_shadow", "id": rect.to_string(), "remove": true},
            ])),
            "Clear",
        );
        assert_applied(&outcome);
        let NodeData::Vector(vector) = &doc.scene.get(rect).unwrap().data else {
            panic!("expected a vector");
        };
        assert!(vector.strokes.is_empty());
        assert!(doc.scene.get(rect).unwrap().effects.is_empty());

        // Strokes on text are refused with the op index named by the batch.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "set_stroke", "id": text.to_string(), "width": 1.0}])),
            "Bad stroke",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert_eq!(outcome.value["ops"][0]["status"], json!("failed"));
    }

    #[test]
    fn set_auto_layout_configures_and_clears_the_frame_layout() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 300.0, "height": 100.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "vertical",
                 "gap": 8.0, "padding": [16.0, 24.0], "align_items": "stretch",
                 "justify": "space_between"},
            ])),
            "Layout",
        );
        assert_applied(&outcome);
        let NodeData::Group(group) = &doc.scene.get(frame).unwrap().data else {
            panic!("expected a frame");
        };
        let layout = group.auto_layout.expect("auto layout on");
        assert_eq!(layout.mode, LayoutMode::Vertical);
        assert_eq!(layout.spacing, 8.0);
        assert_eq!(layout.padding, [16.0, 24.0, 16.0, 24.0]);
        assert_eq!(layout.counter_align, CounterAlign::Stretch);
        assert_eq!(layout.primary_align, PrimaryAlign::SpaceBetween);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "none"},
            ])),
            "Layout off",
        );
        assert_applied(&outcome);
        let NodeData::Group(group) = &doc.scene.get(frame).unwrap().data else {
            panic!("expected a frame");
        };
        assert!(group.auto_layout.is_none());

        assert!(parse_padding(&[1.0, 2.0, 3.0]).is_err());
        assert_eq!(parse_padding(&[4.0]).unwrap(), [4.0; 4]);
        assert_eq!(
            parse_padding(&[1.0, 2.0, 3.0, 4.0]).unwrap(),
            [1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn set_grid_layout_builds_tracks_and_pins_cells() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 300.0, "height": 200.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle", "parent": frame.to_string(),
                 "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0},
                {"op": "create_node", "node_type": "rectangle", "parent": frame.to_string(),
                 "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0},
            ])),
            "Cells",
        );
        let second = created_id(&outcome, 1);

        // `direction: grid` on a frame with two children starts with
        // ⌈√2⌉ = 2 equal columns.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "grid", "gap": 12.0},
            ])),
            "Grid",
        );
        assert_applied(&outcome);
        let grid = |doc: &Doc| match &doc.scene.get(frame).unwrap().data {
            NodeData::Group(group) => group.grid.clone().expect("grid tracks"),
            _ => panic!("expected a frame"),
        };
        assert_eq!(grid(&doc).columns, vec![GridTrack::Flex { fr: 1.0 }; 2]);
        assert_eq!((grid(&doc).column_gap, grid(&doc).row_gap), (12.0, 12.0));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_grid_layout", "id": frame.to_string(),
                 "columns": [120, "2fr", "auto"], "rows": ["1fr"], "gap": 8.0, "row_gap": 4.0},
                {"op": "set_layout_child", "id": second.to_string(), "column": 1, "row": 0,
                 "column_span": 2, "cell_horizontal": "center"},
            ])),
            "Tracks",
        );
        assert_applied(&outcome);
        let tracks = grid(&doc);
        assert_eq!(
            tracks.columns,
            vec![
                GridTrack::Fixed { size: 120.0 },
                GridTrack::Flex { fr: 2.0 },
                GridTrack::Hug,
            ]
        );
        assert_eq!(tracks.rows, vec![GridTrack::Flex { fr: 1.0 }]);
        assert_eq!((tracks.column_gap, tracks.row_gap), (8.0, 4.0));
        let cell = doc
            .scene
            .get(second)
            .unwrap()
            .layout_child
            .unwrap()
            .grid
            .unwrap();
        assert_eq!((cell.column, cell.row, cell.column_span), (1, 0, 2));
        assert_eq!(cell.horizontal, GridAlign::Center);

        let style = crate::style::node_style(&doc, frame);
        assert_eq!(style["auto_layout"]["direction"], json!("grid"));
        assert_eq!(style["grid"]["columns"], json!([120.0, "2fr", "auto"]));
        let child = crate::style::node_style(&doc, second);
        assert_eq!(child["layout_child"]["column_span"], json!(2));

        // Cells only exist inside a grid, and `auto_place` unpins one.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_layout_child", "id": frame.to_string(), "column": 0},
            ])),
            "Not a grid child",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_layout_child", "id": second.to_string(), "auto_place": true},
            ])),
            "Unpin",
        );
        assert_applied(&outcome);
        assert!(
            doc.scene
                .get(second)
                .unwrap()
                .layout_child
                .is_none_or(|child| child.grid.is_none())
        );

        for bad in [json!(["wide"]), json!([]), json!(["0fr"]), json!([-4])] {
            let outcome = run_batch(
                &mut doc,
                &ops(json!([
                    {"op": "set_grid_layout", "id": frame.to_string(), "columns": bad},
                ])),
                "Bad tracks",
            );
            assert_eq!(outcome.value["applied"], json!(false), "{bad}");
        }
    }

    #[test]
    fn set_index_moves_a_node_through_its_siblings() {
        let (mut doc, page_id) = doc_with_page();
        let bottom = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let middle = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let top = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let order = |doc: &Doc| doc.scene.children_of(Some(page_id)).to_vec();
        assert_eq!(order(&doc), vec![bottom, middle, top]);

        let run = |doc: &mut Doc, id: NodeId, position: Value| {
            let outcome = run_batch(
                doc,
                &ops(json!([{"op": "set_index", "id": id.to_string(), "position": position}])),
                "Reorder",
            );
            assert_applied(&outcome);
        };
        run(&mut doc, bottom, json!("front"));
        assert_eq!(order(&doc), vec![middle, top, bottom]);
        run(&mut doc, bottom, json!("backward"));
        assert_eq!(order(&doc), vec![middle, bottom, top]);
        run(&mut doc, middle, json!("forward"));
        assert_eq!(order(&doc), vec![bottom, middle, top]);
        run(&mut doc, top, json!("back"));
        assert_eq!(order(&doc), vec![top, bottom, middle]);
        run(&mut doc, top, json!({"index": 1}));
        assert_eq!(order(&doc), vec![bottom, top, middle]);
        // Already at the front: no change, still a successful op.
        run(&mut doc, middle, json!("front"));
        assert_eq!(order(&doc), vec![bottom, top, middle]);
    }

    #[test]
    fn align_and_distribute_move_nodes_in_world_space() {
        let (mut doc, _) = doc_with_page();
        let first = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let second = create_rect(&mut doc, 100.0, 50.0, 10.0, 10.0);
        let third = create_rect(&mut doc, 130.0, 90.0, 10.0, 10.0);
        let ids = [first, second, third]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "align", "ids": ids, "edge": "top"},
                {"op": "distribute", "ids": ids, "axis": "horizontal"},
            ])),
            "Tidy",
        );
        assert_applied(&outcome);
        for id in [first, second, third] {
            assert_eq!(doc.scene.world_bounds(id).unwrap().min_y, 0.0);
        }
        assert_eq!(doc.scene.world_bounds(second).unwrap().min_x, 65.0);

        // A single node aligns to its parent frame.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 200.0, "y": 200.0, "width": 100.0, "height": 100.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "reparent", "id": first.to_string(), "parent": frame.to_string()},
                {"op": "align", "ids": [first.to_string()], "edge": "center_x"},
                {"op": "align", "ids": [first.to_string()], "edge": "bottom"},
            ])),
            "Center in frame",
        );
        assert_applied(&outcome);
        let bounds = doc.scene.world_bounds(first).unwrap();
        assert_eq!((bounds.min_x, bounds.max_y), (245.0, 300.0));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "distribute", "ids": [second.to_string()], "axis": "vertical"}])),
            "Too few",
        );
        assert_eq!(outcome.value["applied"], json!(false));
    }

    #[test]
    fn rotate_sets_an_absolute_angle_about_the_centre() {
        let (mut doc, _) = doc_with_page();
        let rect = create_rect(&mut doc, 0.0, 0.0, 40.0, 20.0);
        let before = doc.scene.world_bounds(rect).unwrap().center();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "rotate", "id": rect.to_string(), "degrees": 90.0}])),
            "Rotate",
        );
        assert_applied(&outcome);
        let after = doc.scene.world_bounds(rect).unwrap();
        assert!((after.center() - before).length() < 1e-9);
        assert!((after.width() - 20.0).abs() < 1e-9 && (after.height() - 40.0).abs() < 1e-9);
        let angle = fanta_canvas::transform_angle(&doc.scene.world_transform(rect).unwrap());
        assert!((angle - 90.0_f64.to_radians()).abs() < 1e-9);
    }

    #[test]
    fn duplicate_returns_the_copy_and_offsets_it() {
        let (mut doc, page_id) = doc_with_page();
        let rect = create_rect(&mut doc, 10.0, 10.0, 40.0, 20.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "duplicate", "id": rect.to_string(), "dx": 50.0, "dy": 0.0}])),
            "Duplicate",
        );
        assert_applied(&outcome);
        let copy = created_id(&outcome, 0);
        assert_ne!(copy, rect);
        let bounds = doc.scene.world_bounds(copy).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (60.0, 10.0));
        assert_eq!(doc.scene.children_of(Some(page_id)), &[rect, copy]);
        assert_eq!(doc.history.undo_depth(), 2);
    }

    #[test]
    fn componentize_replaces_copies_with_instances_that_keep_their_differences() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 0.0, "y": 0.0, "width": 100.0, "height": 60.0},
            ])),
            "Frame",
        );
        let card = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": "Title", "parent": card.to_string(),
                 "x": 8.0, "y": 8.0, "width": 80.0, "height": 20.0},
                {"op": "create_node", "node_type": "rectangle", "parent": card.to_string(), "fill": "#00ff00",
                 "x": 8.0, "y": 32.0, "width": 20.0, "height": 20.0},
            ])),
            "Contents",
        );
        assert_applied(&outcome);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "duplicate", "id": card.to_string(), "dx": 150.0, "dy": 0.0}])),
            "Copy",
        );
        let copy = created_id(&outcome, 0);
        let copy_children = doc.scene.children_of(Some(copy)).to_vec();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_props", "id": copy_children[0].to_string(), "text": "Other"},
                {"op": "set_props", "id": copy_children[1].to_string(), "fill": "#ff0000"},
                {"op": "set_props", "id": copy.to_string(), "opacity": 0.5},
            ])),
            "Edit the copy",
        );
        assert_applied(&outcome);
        let copy_node = doc.scene.get(copy).unwrap().clone();
        let copy_index = doc
            .scene
            .children_of(Some(page_id))
            .iter()
            .position(|id| *id == copy);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "componentize", "ids": [card.to_string(), copy.to_string()]}])),
            "Componentize",
        );
        assert_applied(&outcome);
        let detail = &outcome.value["ops"][0];
        let instance: NodeId = detail["instances"][0].as_str().unwrap().parse().unwrap();
        assert!(!doc.scene.contains(copy));
        assert!(
            doc.components
                .defs
                .values()
                .any(|def| def.root == card && def.id.to_string() == detail["component"])
        );
        let node = doc
            .scene
            .get(instance)
            .expect("the instance replaced the copy");
        assert_eq!(node.parent, copy_node.parent);
        assert_eq!(node.transform, copy_node.transform);
        assert_eq!(node.name, copy_node.name);
        assert_eq!(
            doc.scene
                .children_of(Some(page_id))
                .iter()
                .position(|id| *id == instance),
            copy_index,
            "the instance keeps the copy's z-order"
        );

        let NodeData::Instance(data) = &node.data else {
            panic!("expected an instance");
        };
        // Only what the copy changed: its text, its rectangle's fill, its
        // root's opacity.
        let overridden: Vec<_> = data
            .overrides
            .iter()
            .map(|o| match &o.value {
                fanta_doc::OverrideValue::Field { value } => format!("field {value}"),
                fanta_doc::OverrideValue::Text { .. } => "text".to_owned(),
                fanta_doc::OverrideValue::Fills { .. } => "fills".to_owned(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            overridden,
            vec!["field {\"opacity\":0.5}", "text", "fills"],
            "{:?}",
            data.overrides
        );
        let expanded = fanta_doc::expand_instance(&doc.scene, &doc.components, data);
        let text = expanded
            .iter()
            .find_map(|clone| match &clone.node.data {
                NodeData::Text(text) => Some(text.content.clone()),
                _ => None,
            })
            .expect("text clone");
        assert_eq!(text, "Other");
        let fill = expanded
            .iter()
            .find_map(|clone| match &clone.node.data {
                NodeData::Vector(vector) => vector.fills.first().and_then(Fill::solid_color),
                _ => None,
            })
            .expect("rectangle clone");
        assert_eq!(fill.to_hex(), "#FF0000");
        let root = expanded
            .iter()
            .find(|clone| clone.def_path.is_empty())
            .unwrap();
        assert_eq!(root.node.opacity.get(), 0.5);

        // A copy whose structure differs is refused, and nothing changes.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Odd",
                 "x": 0.0, "y": 200.0, "width": 100.0, "height": 60.0},
            ])),
            "Odd frame",
        );
        let odd = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "componentize", "ids": [card.to_string(), odd.to_string()]}])),
            "Mismatch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert!(doc.scene.contains(odd));
    }

    #[test]
    fn a_failed_batch_rolls_back_a_component_definition() {
        let (mut doc, _page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Button",
                 "x": 0.0, "y": 0.0, "width": 120.0, "height": 40.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let defs_before = doc.components.defs.len();

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_component", "id": frame.to_string()},
                {"op": "delete", "id": NodeId::new().to_string()},
            ])),
            "Componentize then fail",
        );
        assert_eq!(outcome.value["applied"], Value::Bool(false));
        assert_eq!(
            doc.components.defs.len(),
            defs_before,
            "the definition applied before the failure must roll back with the batch"
        );
        assert!(!doc.is_component_root(frame));
    }

    #[test]
    fn create_component_then_create_instance_by_name_and_id() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Button",
                 "x": 0.0, "y": 0.0, "width": 120.0, "height": 40.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "create_component", "id": frame.to_string()}])),
            "Componentize",
        );
        assert_applied(&outcome);
        let component = outcome.value["ops"][0]["component"]
            .as_str()
            .expect("the component id is reported")
            .to_string();
        assert!(doc.is_component_root(frame));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_instance", "component": "Button", "x": 200.0, "y": 300.0},
                {"op": "create_instance", "component": component, "x": 400.0, "y": 300.0,
                 "name": "Second"},
            ])),
            "Instances",
        );
        assert_applied(&outcome);
        let instance = created_id(&outcome, 0);
        let node = doc.scene.get(instance).unwrap();
        assert_eq!(node.parent, Some(page_id));
        assert_eq!(node.name, "Button");
        let NodeData::Instance(instance_node) = &node.data else {
            panic!("expected an instance");
        };
        assert_eq!(instance_node.local_size, [120.0, 40.0]);
        let bounds = doc.scene.world_bounds(instance).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (200.0, 300.0));
        assert_eq!(
            doc.scene.get(created_id(&outcome, 1)).unwrap().name,
            "Second"
        );

        // Instances cannot nest inside their own master.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_instance", "component": "Button", "x": 0.0, "y": 0.0,
                 "parent": frame.to_string()},
            ])),
            "Recursive",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        // A second master named the same makes the name ambiguous.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Button",
                 "x": 0.0, "y": 500.0, "width": 10.0, "height": 10.0},
            ])),
            "Frame",
        );
        let other = created_id(&outcome, 0);
        run_batch(
            &mut doc,
            &ops(json!([{"op": "create_component", "id": other.to_string()}])),
            "Componentize",
        );
        assert!(resolve_component(&doc, "Button").is_err());
    }

    #[test]
    fn group_ungroup_and_frame_selection_report_their_structure() {
        let (mut doc, page_id) = doc_with_page();
        let first = create_rect(&mut doc, 10.0, 10.0, 20.0, 20.0);
        let second = create_rect(&mut doc, 50.0, 30.0, 20.0, 20.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "group", "ids": [first.to_string(), second.to_string()], "name": "Pair"},
            ])),
            "Group",
        );
        assert_applied(&outcome);
        let group = created_id(&outcome, 0);
        assert_eq!(doc.scene.get(group).unwrap().name, "Pair");
        assert_eq!(doc.scene.get(first).unwrap().parent, Some(group));
        let bounds = doc.scene.world_bounds(first).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (10.0, 10.0));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "ungroup", "id": group.to_string()}])),
            "Ungroup",
        );
        assert_applied(&outcome);
        let children = outcome.value["ops"][0]["children"]
            .as_array()
            .expect("freed children are reported");
        assert_eq!(children.len(), 2);
        assert!(!doc.scene.contains(group));
        assert_eq!(doc.scene.get(second).unwrap().parent, Some(page_id));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "frame_selection", "ids": [first.to_string(), second.to_string()]},
            ])),
            "Frame",
        );
        assert_applied(&outcome);
        let frame = created_id(&outcome, 0);
        let NodeData::Group(group_node) = &doc.scene.get(frame).unwrap().data else {
            panic!("expected a frame");
        };
        assert_eq!(group_node.clip_size, Some([60.0, 40.0]));
        assert!(group_node.background.is_none());
    }

    #[test]
    fn node_summary_carries_kind_specific_facts() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 0.0, "y": 0.0, "width": 200.0, "height": 100.0, "fill": "#FAFAFA"},
                {"op": "create_node", "node_type": "text", "text": "Title",
                 "x": 8.0, "y": 8.0, "width": 100.0, "height": 20.0},
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "fill": "#123456"},
            ])),
            "Create",
        );
        let frame = created_id(&outcome, 0);
        run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "horizontal"},
            ])),
            "Layout",
        );
        let summary = node_summary(&doc, page_id, None, false);
        let children = summary["children"].as_array().unwrap();
        assert_eq!(children[0]["size"], json!([200.0, 100.0]));
        assert_eq!(children[0]["auto_layout"], json!("horizontal"));
        assert_eq!(children[0]["fill"], json!("#FAFAFA"));
        assert_eq!(children[1]["text"], json!("Title"));
        assert!(children[1].get("text_length").is_none());
        assert_eq!(children[1]["font"]["size"], json!(16.0));
        assert_eq!(children[2]["fill"], json!("#123456"));
    }

    /// A page listing must stay bounded per node, so long text is cut in the
    /// summary and its full length reported; the whole content is still
    /// there when the node is fetched by id.
    #[test]
    fn node_summary_truncates_long_text_and_reports_its_length() {
        let (mut doc, page_id) = doc_with_page();
        let long_text = "é".repeat(SUMMARY_TEXT_CHARS + 30);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": long_text,
                 "x": 0.0, "y": 0.0, "width": 100.0, "height": 20.0},
            ])),
            "Create",
        );
        assert_applied(&outcome);
        let text = created_id(&outcome, 0);

        let summary = node_summary(&doc, page_id, None, false);
        let listed = &summary["children"][0];
        let preview = listed["text"].as_str().expect("text is a string");
        assert_eq!(preview.chars().count(), SUMMARY_TEXT_CHARS + 1);
        assert!(preview.ends_with('…'));
        assert_eq!(listed["text_length"], json!(SUMMARY_TEXT_CHARS + 30));

        let NodeData::Text(node) = &doc.scene.get(text).unwrap().data else {
            panic!("expected a text node");
        };
        assert_eq!(node.content, long_text);
    }

    /// A page with `count` sibling rectangles, and their ids in z-order.
    fn page_with_children(count: usize) -> (Doc, NodeId, Vec<String>) {
        let (mut doc, page_id) = doc_with_page();
        let requests: Vec<Value> = (0..count)
            .map(|index| {
                json!({"op": "create_node", "node_type": "rectangle", "name": format!("Row {index}"),
                       "x": 0.0, "y": index as f64 * 20.0, "width": 10.0, "height": 10.0})
            })
            .collect();
        let outcome = run_batch(&mut doc, &ops(json!(requests)), "Create");
        assert_applied(&outcome);
        let ids = (0..count)
            .map(|index| created_id(&outcome, index).to_string())
            .collect();
        (doc, page_id, ids)
    }

    fn listed_child_ids(summary: &Value) -> Vec<String> {
        summary["children"]
            .as_array()
            .expect("a listing carries a children array")
            .iter()
            .map(|child| child["id"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// Listing is the only way to discover node ids, so a page too wide for one
    /// response must still be enumerable: each call returns a window plus the
    /// facts needed to ask for the next one.
    #[test]
    fn a_page_listing_windows_its_children_and_reports_how_to_continue() {
        let (doc, page_id, all_children) = page_with_children(7);

        let first = paginated_node_summary(&doc, page_id, Some(1), false, 0, 3);
        assert_eq!(first["child_count"], json!(7));
        assert_eq!(first["children_offset"], json!(0));
        assert_eq!(first["children_limit"], json!(3));
        assert_eq!(first["more_children"], json!(true));
        let first_window = listed_child_ids(&first);
        assert_eq!(first_window.len(), 3);

        let second = paginated_node_summary(&doc, page_id, Some(1), false, 3, 3);
        assert_eq!(second["children_offset"], json!(3));
        assert_eq!(second["more_children"], json!(true));
        let third = paginated_node_summary(&doc, page_id, Some(1), false, 6, 3);
        assert_eq!(third["more_children"], json!(false));

        let walked: Vec<String> = [
            first_window,
            listed_child_ids(&second),
            listed_child_ids(&third),
        ]
        .concat();
        assert_eq!(walked, all_children, "the windows must tile the child list");
    }

    #[test]
    fn an_offset_past_the_end_lists_nothing_rather_than_failing() {
        let (doc, page_id, _) = page_with_children(3);
        let summary = paginated_node_summary(&doc, page_id, Some(1), false, 99, 200);
        assert_eq!(summary["child_count"], json!(3));
        assert_eq!(summary["children_offset"], json!(99));
        assert_eq!(summary["more_children"], json!(false));
        assert!(listed_child_ids(&summary).is_empty());
    }

    /// Only the listed node's own children are windowed; what hangs below a
    /// listed child is governed by `depth` alone, so a windowed listing never
    /// silently drops part of a subtree it did return.
    #[test]
    fn pagination_applies_to_the_top_level_only() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 0.0, "y": 0.0, "width": 200.0, "height": 100.0},
            ])),
            "Frame",
        );
        assert_applied(&outcome);
        let frame = created_id(&outcome, 0);
        let requests: Vec<Value> = (0..5)
            .map(|index| {
                json!({"op": "create_node", "node_type": "rectangle", "parent": frame.to_string(),
                       "x": index as f64, "y": 0.0, "width": 4.0, "height": 4.0})
            })
            .collect();
        assert_applied(&run_batch(&mut doc, &ops(json!(requests)), "Rows"));

        let summary = paginated_node_summary(&doc, page_id, Some(2), false, 0, 1);
        let listed = &summary["children"][0];
        assert_eq!(listed["children"].as_array().map(Vec::len), Some(5));
        assert!(listed.get("more_children").is_none());
        assert!(listed.get("children_limit").is_none());
    }

    /// `depth: 0` asks for counts, not children, so it keeps its old shape.
    #[test]
    fn a_depth_zero_listing_still_reports_only_a_child_count() {
        let (doc, page_id, _) = page_with_children(4);
        let summary = paginated_node_summary(&doc, page_id, Some(0), false, 0, 2);
        assert_eq!(summary["child_count"], json!(4));
        assert!(summary.get("children").is_none());
        assert!(summary.get("more_children").is_none());
    }

    #[test]
    fn page_listing_budget_limits_nested_breadth_and_allows_narrower_queries() -> Result<()> {
        let (mut doc, page) = doc_with_page();
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
        frame.parent = Some(page);
        let frame_id = frame.id;
        doc.scene.insert(frame)?;
        for _ in 0..PAGE_LISTING_NODE_BUDGET {
            let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
            child.parent = Some(frame_id);
            child.index = doc.scene.next_child_index(Some(frame_id));
            doc.scene.insert(child)?;
        }
        let mut sibling = CanvasNode::new(NodeData::Group(GroupNode::default()));
        sibling.parent = Some(page);
        sibling.index = doc.scene.next_child_index(Some(page));
        doc.scene.insert(sibling)?;

        let error = check_page_listing_budget(&doc, page, Some(2), 0, 1)
            .expect_err("one broad frame exceeds the total-node budget");
        let message = error.to_string();
        assert!(message.contains("depth"));
        assert!(message.contains("limit"));
        assert!(message.contains("ids"));
        assert!(check_page_listing_budget(&doc, page, None, 0, 1).is_err());
        check_page_listing_budget(&doc, page, Some(1), 0, 1)?;
        check_page_listing_budget(&doc, page, Some(0), 0, usize::MAX)?;
        check_page_listing_budget(&doc, page, None, 1, 1)?;
        check_page_listing_budget(&doc, page, Some(u32::MAX), 99, usize::MAX)?;
        Ok(())
    }

    #[test]
    fn set_text_style_rejects_weights_outside_the_opentype_range() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": "Hi",
                 "x": 0.0, "y": 0.0, "width": 80.0, "height": 20.0},
            ])),
            "Text",
        );
        let text = created_id(&outcome, 0);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_text_style", "id": text.to_string(), "font_weight": 1000},
            ])),
            "Weight",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        let error = outcome.value["ops"][0]["error"]
            .as_str()
            .expect("the failed op carries its error");
        assert!(error.contains("between 100 and 900"), "{error}");

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_text_style", "id": text.to_string(), "font_weight": 900},
            ])),
            "Weight",
        );
        assert_applied(&outcome);
    }

    /// The `style` projection speaks the ops' vocabulary: a value read from it
    /// can be sent back through its op unchanged.
    #[test]
    fn style_values_round_trip_through_their_ops() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 300.0, "height": 200.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0).to_string();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame, "direction": "vertical", "gap": 8.0,
                 "padding": [16.0, 24.0], "align_items": "stretch", "justify": "space_between",
                 "wrap": false, "counter_gap": 4.0},
                {"op": "set_effects", "id": frame, "effects": [
                    {"kind": "drop_shadow", "color": "#0000001A", "y": 1.0, "blur": 2.0},
                    {"kind": "drop_shadow", "color": "#00000014", "y": 8.0, "blur": 24.0, "spread": -4.0},
                    {"kind": "background_blur", "radius": 12.0},
                ]},
                {"op": "set_fill", "id": frame, "paints": [
                    {"kind": "linear", "stops": [
                        {"position": 0.0, "color": "#FFFFFF"},
                        {"position": 1.0, "color": "#F4F4F5"},
                    ]},
                    {"kind": "solid", "color": "#FFFFFF80"},
                ]},
                {"op": "set_stroke", "id": frame, "color": "#E4E4E7", "width": 1.0,
                 "sides": [0.0, 0.0, 1.0, 0.0], "dash": [4.0, 2.0], "cap": "round"},
                {"op": "set_constraints", "id": frame, "horizontal": "left_right",
                 "vertical": "top"},
            ])),
            "Style",
        );
        assert_applied(&outcome);
        let id: NodeId = frame.parse().unwrap();
        let style = crate::style::node_style(&doc, id);
        assert_eq!(style["auto_layout"]["direction"], "vertical");
        assert_eq!(
            style["auto_layout"]["padding"],
            json!([16.0, 24.0, 16.0, 24.0])
        );
        assert_eq!(style["effects"].as_array().unwrap().len(), 3);
        assert_eq!(style["fills"][0]["kind"], "linear");
        assert_eq!(style["strokes"][0]["sides"], json!([0.0, 0.0, 1.0, 0.0]));
        assert_eq!(style["constraints"]["horizontal"], "left_right");

        // Write every read value back through its op.
        let mut auto_layout = style["auto_layout"].clone();
        auto_layout["op"] = json!("set_auto_layout");
        auto_layout["id"] = json!(frame);
        let mut stroke = style["strokes"][0].clone();
        stroke["op"] = json!("set_stroke");
        stroke["id"] = json!(frame);
        let mut constraints = style["constraints"].clone();
        constraints["op"] = json!("set_constraints");
        constraints["id"] = json!(frame);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                auto_layout,
                {"op": "set_effects", "id": frame, "effects": style["effects"]},
                {"op": "set_fill", "id": frame, "paints": style["fills"]},
                stroke,
                constraints,
            ])),
            "Round trip",
        );
        assert_applied(&outcome);
        assert_eq!(crate::style::node_style(&doc, id), style);
    }

    #[test]
    fn a_style_listing_is_one_flat_node_per_entry() {
        let (mut doc, page) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 200.0, "height": 100.0, "fill": "#FFFFFF"},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0).to_string();
        assert_applied(&run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame, "direction": "horizontal", "gap": 12.0},
                {"op": "set_stroke", "id": frame, "width": 1.0},
            ])),
            "Style",
        ));
        let mut listing = paginated_node_summary(&doc, page, Some(1), true, 0, 10);
        crate::style::attach_styles(&doc, &mut listing);
        let entry = listing["children"]
            .as_array()
            .unwrap()
            .iter()
            .find(|child| child["id"] == json!(frame))
            .expect("the frame is listed");
        assert_eq!(entry["fill"], "#FFFFFF", "the listing's own keys stay");
        assert_eq!(
            entry["auto_layout"]["gap"], 12.0,
            "style replaces the mode string"
        );
        assert_eq!(entry["strokes"][0]["width"], 1.0);
        assert!(entry.get("style").is_none());
    }

    #[test]
    fn create_shape_builds_lines_polygons_stars_and_paths() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_shape", "shape": "line", "x": 0.0, "y": 0.0,
                 "width": 120.0, "height": 0.0},
                {"op": "create_shape", "shape": "polygon", "x": 0.0, "y": 0.0,
                 "width": 60.0, "height": 60.0, "points": 6, "fill": "#22C55E"},
                {"op": "create_shape", "shape": "star", "x": 0.0, "y": 0.0,
                 "width": 60.0, "height": 60.0},
                {"op": "create_shape", "shape": "path", "x": 10.0, "y": 20.0,
                 "width": 24.0, "height": 24.0, "path": "M0 0 L24 12 L0 24 Z"},
            ])),
            "Shapes",
        );
        assert_applied(&outcome);
        let vector = |index| {
            let NodeData::Vector(vector) = doc
                .scene
                .get(created_id(&outcome, index))
                .unwrap()
                .data
                .clone()
            else {
                panic!("expected a vector");
            };
            vector
        };
        let line = vector(0);
        assert!(line.fills.is_empty(), "a line is not filled");
        assert_eq!(line.strokes.len(), 1, "a line is stroked by default");
        let polygon = vector(1);
        assert_eq!(polygon.fills.len(), 1);
        assert!(polygon.strokes.is_empty());
        assert_eq!(
            vector(2).fills.len(),
            1,
            "a closed star gets the default fill"
        );
        assert_eq!(
            vector(3).fills.len(),
            1,
            "a closed path gets the default fill"
        );

        let bad = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_shape", "shape": "path", "x": 0.0, "y": 0.0,
                 "width": 1.0, "height": 1.0, "path": "not svg"},
            ])),
            "Bad path",
        );
        assert!(bad.value["error"].is_string(), "{:?}", bad.value);
    }

    #[test]
    fn new_ops_validate_and_undo() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 40.0, "height": 40.0},
            ])),
            "Rect",
        );
        let rect = created_id(&outcome, 0);
        let id = rect.to_string();
        for (label, op) in [
            (
                "one stop",
                json!({"op": "set_fill", "id": id, "paints": [
                {"kind": "linear", "stops": [{"position": 0.0, "color": "#000000"}]}]}),
            ),
            (
                "negative blur",
                json!({"op": "set_effects", "id": id, "effects": [
                {"kind": "layer_blur", "radius": -1.0}]}),
            ),
            (
                "negative side",
                json!({"op": "set_stroke", "id": id, "sides": [1.0, -1.0, 0.0, 0.0]}),
            ),
        ] {
            let outcome = run_batch(&mut doc, &ops(json!([op])), label);
            assert!(
                outcome.value["error"].is_string(),
                "{label}: {:?}",
                outcome.value
            );
        }

        assert!(doc.scene.get(rect).unwrap().constraints.is_none());
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_constraints", "id": id, "horizontal": "scale"},
            ])),
            "Constraints",
        );
        assert_applied(&outcome);
        assert_eq!(
            doc.scene.get(rect).unwrap().constraints.unwrap().horizontal,
            fanta_doc::ConstraintH::Scale
        );
        assert!(doc.undo().unwrap());
        assert!(doc.scene.get(rect).unwrap().constraints.is_none());
    }
}
