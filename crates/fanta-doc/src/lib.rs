//! Polymorphic scene graph for the Fanta engine.
//!
//! See `ARCHITECTURE.md` for the design — this crate implements §3 (doc model),
//! enforces §7's CRDT-class invariants through its type shape, and produces
//! the canonical `.fant.json` projection described in §6 and §8.
//!
//! The crate has **no rendering dependencies on purpose**. `fanta-render`
//! consumes types from here; the dependency direction never reverses, which is
//! what makes a future renderer swap (Skia → Vello, or anything else) a
//! mechanical change.
//!
//! ## Quick tour
//!
//! ```
//! use fanta_doc::{Doc, CanvasNode, NodeData, VectorNode, Operation, Color};
//!
//! let mut doc = Doc::new();
//! let rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
//!     0.0, 0.0, 100.0, 50.0, Color::rgb(0xFF, 0x66, 0x00),
//! )));
//! let rect_id = rect.id;
//! doc.apply(Operation::create_node(rect)).unwrap();
//! assert!(doc.scene.contains(rect_id));
//! doc.undo().unwrap();
//! assert!(!doc.scene.contains(rect_id));
//! ```

#![forbid(unsafe_code)]

pub mod binding;
pub mod color;
pub mod component;
pub mod doc;
pub mod history;
pub mod id;
pub mod index;
pub mod journal;
pub mod layout;
pub mod motion;
pub mod node;
pub mod op;
pub mod path;
pub mod replay;
pub mod resolve;
pub mod scene;
pub mod selection;
pub(crate) mod serde_util;
pub mod snapshot;
pub mod spatial;
pub mod style;
pub mod transform;
pub mod value;
pub mod variables;

// Flat re-exports for ergonomic call sites.
pub use binding::BoundProp;
pub use color::{Color, Gradient, GradientStop};
pub use component::{
    ComponentDef, ComponentLibrary, ComponentPropDef, ComponentPropFormatter, ComponentPropKind,
    ComponentSet, ComponentSetMembership, PropBindingTarget, VariantAxis,
};
pub use doc::{
    Doc, DocLoadError, DocMetadata, Flow, PresentationConfig, SCHEMA_VERSION, Viewport,
    migrate_doc_json,
};
pub use history::{History, Transaction};
pub use id::{
    AnimationClipId, AnimationTrackId, AssetId, ComponentId, ComponentPropId, DocId, IdBuildHasher,
    IdHashMap, IdHasher, IdParseError, KeyframeId, LinkId, ModeId, NodeId, ReactionId,
    VariableCollectionId, VariableId, WorkflowNodeId,
};
pub use index::IndexKey;
pub use journal::{
    JournalEvent, JournalSink, JournalViewport, Provenance, RecordedInput, SessionJournal,
    SessionStep,
};
pub use layout::{ExpandedTree, LayoutTree, Measure, solve_auto_layout, solve_expanded};
pub use motion::{
    AnimationClip, AnimationTrack, Interpolation, Keyframe, MotionEvaluation, MotionLibrary,
    MotionProperty, MotionTarget, MotionTransform,
};
pub use node::fields::known_fields;
pub use node::{
    Action, AiArtifactNode, AudioNode, AutoLayout, AxisSizing, BitmapNode, BooleanNode, BooleanOp,
    Camera3d, CanvasNode, ConstraintH, ConstraintV, Constraints, CounterAlign, DerivedOverride,
    Direction, Easing, EmbedNode, FontVariation, GenerationStatus, GroupNode, InstanceNode,
    LayoutChild, LayoutMode, Link, MaskType, Model3dNode, NodeData, NodeFlags, NodeGraph,
    NodeGraphNode, OverlayPosition, OverlaySettings, Override, OverridePath, OverrideValue,
    ParametricShape, PrimaryAlign, PrototypeAnimation, Reaction, ScrollBehavior, ScrollDirection,
    TextAlign, TextAutoResize, TextNode, TextStyle, TextStyleRun, Transition, TransitionStyle,
    Trigger, VAlign, VectorNode, VideoNode, WorkflowNode, spring_progress,
};
pub use op::{ModeScope, OpCtx, Operation};
pub use path::{FillRule, PathData, PathSegment, SvgPathError};
pub use replay::{Divergence, ReplayResult, diff_snapshots, replay_ops};
pub use resolve::{
    ExpandedNode, InstanceExpansionContext, ResolvedComponentDefRef, backfill_vector_viewports,
    def_local_path, expand_instance, expand_instance_with_context, resolve_bound_value,
    resolve_effective_mode, resolved_component, resolved_component_rev,
    resolved_component_rev_with_context, resolved_component_root,
    resolved_component_root_with_context, resolved_component_with_context,
    strip_redundant_instance_overrides,
};
pub use scene::{Ancestors, Descendants, Scene, SceneError};
pub use selection::Selection;
pub use snapshot::{
    NodeSnapshot, ResolvedInstanceHop, ResolvedNodeTrace, ResolvedSceneTrace, SceneSnapshot,
    TraceCoverage,
};
pub use spatial::SpatialIndex;
pub use style::{
    BlendMode, Blur, BlurKind, Fill, ImageAdjust, ImageFitMode, Shadow, ShadowKind, Stroke,
    StrokeAlign, StrokeCap, StrokeJoin, UnitInterval,
};
pub use transform::{Bounds, Transform2D};
pub use value::{ResolvedVarValue, VarValue, VariableType};
pub use variables::{Mode, Variable, VariableCollection, VariableRegistry, VariableScope};
