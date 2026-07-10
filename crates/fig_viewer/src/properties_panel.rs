//! The Fanta properties panel: the inspector for the current canvas
//! selection, falling back to page properties when nothing is selected,
//! mirroring the original Fanta right inspector — its section stack, field
//! density, drag-to-scrub numeric fields, opacity slider, and anchored color
//! picker.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use editor::{Editor, EditorEvent, actions::SelectAll};
use fanta_canvas::{Axis, HAlign, VAlign, align_horizontal, align_vertical, distribute};
use fanta_doc::{
    AutoLayout, AxisSizing, BlendMode, BlurKind, Color as FantaColor, ComponentPropId,
    CounterAlign, Doc, Fill, Gradient, ImageFitMode, LayoutChild, LayoutMode, NodeData, NodeFlags,
    NodeId, Operation, PrimaryAlign, ShadowKind, Stroke, StrokeAlign, TextAlign,
    VAlign as TextVAlign, VarValue,
};
use fs::Fs;
use gpui::{
    App, AsyncWindowContext, Bounds, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle,
    Focusable, KeyDownEvent, Pixels, Point, ScrollHandle, Subscription, WeakEntity, Window,
    actions, px,
};
use settings::{Settings as _, update_settings_file};
use ui::prelude::*;
use ui::Divider;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::color_picker::{ColorPicker, ColorPickerEvent, GradientEditor, GradientEditorEvent};
use crate::document::{DocChange, FigItem};
use crate::inspector_widgets::{PanelDrag, TextDecorationGlyph, scrub_value, track_value};
use crate::panel_settings::FantaPropertiesPanelSettings;
use crate::view::FigView;
use crate::properties_ops::{
    ExportJob, add_fill, apply_preview_operation, blurs_operations,
    combine_as_variants_operations, convert_paint_kind, default_blur, default_shadow,
    detach_instance_operations, effects_operations, field_operations,
    finite_transform_operations, format_number, paint_slot_mut, parse_number, read_field_text,
    remove_fill,
    replace_data_operation, restore_snapshot, run_png_export, set_paint_gradient,
    stroke_list_mut, variant_cycle_operations,
};
use crate::properties_snapshot::{
    AlignCommand, HiddenPaintAlpha, InspectorBody, InspectorField, InspectorSnapshot, NodeKind,
    NodeSnapshot, PaintKey, PaintKind, clamp_field_value, field_node, master_roots, multi_section,
    next_text_resize, node_section, opaque_paint_alpha, page_section, paint_alpha,
    paint_alpha_is_visible, paired_field, set_paint_alpha, zeroed_paint_alpha,
};

actions!(
    fanta_properties_panel,
    [
        /// Toggle focus on the Fanta properties panel.
        ToggleFocus
    ]
);

const NO_DOCUMENT_MESSAGE: &str = "Open a Figma document to inspect properties";

/// The panel's draggable slider tracks. Their painted bounds are captured every
/// frame so a press maps straight to a fraction of the track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SliderTrack {
    Opacity,
    CornerSmoothing,
}

const SLIDER_TRACK_COUNT: usize = 2;

enum ScrubKind {
    /// Value follows the horizontal mouse delta (numeric field labels).
    Relative,
    /// Value maps the cursor to a fraction of a track (opacity slider).
    Track {
        bounds: Bounds<Pixels>,
        min: f64,
        max: f64,
    },
}

pub(crate) struct ScrubState {
    field: InspectorField,
    snapshot: NodeSnapshot,
    kind: ScrubKind,
    start_position: Point<Pixels>,
    start_value: f64,
    current_value: f64,
    moved: bool,
}

/// An open color-picker popover bound to one color field.
pub(crate) struct PickerSession {
    pub(crate) field: InspectorField,
    original: FantaColor,
    snapshot: NodeSnapshot,
    changed: bool,
    pub(crate) picker: Entity<ColorPicker>,
    _subscription: Subscription,
}

/// An open gradient-editor popover bound to one gradient paint. Mirrors
/// [`PickerSession`]: transient previews while open, one undoable op on close.
pub(crate) struct GradientSession {
    pub(crate) field: InspectorField,
    original: Gradient,
    snapshot: NodeSnapshot,
    changed: bool,
    pub(crate) editor: Entity<GradientEditor>,
    _subscription: Subscription,
}

pub struct FantaPropertiesPanel {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) fs: Arc<dyn Fs>,
    pub(crate) active_view: Option<WeakEntity<FigView>>,
    pub(crate) width: Option<Pixels>,
    pub(crate) field_editor: Entity<Editor>,
    pub(crate) editing_field: Option<InspectorField>,
    /// Scroll state of the section stack. Reset whenever the inspected
    /// subject changes: a shorter inspector would otherwise keep the previous
    /// subject's offset and show blank space past its content.
    pub(crate) content_scroll: ScrollHandle,
    /// User override for the per-corner radius expander. `None` follows the
    /// node's data (expanded only when it already carries distinct corners);
    /// reset on subject change so one node's expansion doesn't leak to the next.
    pub(crate) corner_radii_expanded: Option<bool>,
    /// An in-flight drag-to-scrub gesture (field label or a slider track).
    pub(crate) scrub: Option<ScrubState>,
    /// Each slider track's window bounds, captured during paint so a click/drag
    /// on the track maps to a 0–100% fraction. Indexed by [`SliderTrack`].
    pub(crate) slider_tracks: [Option<Bounds<Pixels>>; SLIDER_TRACK_COUNT],
    /// The alpha each hidden paint carried before its eye was toggled off, so
    /// showing it again restores the value instead of forcing full opacity.
    /// View-only state: cleared whenever the inspected subject changes.
    pub(crate) hidden_paint_alpha: HashMap<PaintKey, HiddenPaintAlpha>,
    /// The open color-picker popover, if any.
    pub(crate) picker: Option<PickerSession>,
    /// The open gradient-editor popover, if any.
    pub(crate) gradient_editor: Option<GradientSession>,
    /// Set on a swatch mouse-down when that swatch's popover is already open, so
    /// the click's release (which the popover's mouse-down-out handler has by
    /// then closed) does not immediately reopen it — letting a second click on a
    /// swatch toggle its popover shut. Only one popover is open at a time, so a
    /// single flag covers both the color and gradient swatches.
    pub(crate) swatch_press_dismissed: bool,
    pub(crate) _subscriptions: Vec<Subscription>,
    pub(crate) _active_view_subscription: Option<Subscription>,
}

impl FantaPropertiesPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            Self::new(workspace, window, cx)
        })
    }

    pub(crate) fn new_embedded(
        active_view: Entity<FigView>,
        fs: Arc<dyn Fs>,
        window: &mut Window,
        cx: &mut Context<FigView>,
    ) -> Entity<Self> {
        let panel = cx.new(|cx| Self::build(fs, None, window, cx, Vec::new()));
        cx.defer({
            let panel = panel.clone();
            move |cx| {
                let _ = panel.update(cx, |panel, cx| {
                    panel.set_active_view(Some(active_view), cx);
                });
            }
        });
        panel
    }

    fn new(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let fs = workspace.app_state().fs.clone();
        // The workspace entity is mid-update here, so the initial active view
        // must come from the `&mut Workspace` we were handed — reading the
        // entity would double-lease and panic.
        let initial_view = workspace
            .active_item(cx)
            .and_then(|item| item.downcast::<FigView>());
        let workspace_entity = cx.entity();
        cx.new(|cx| {
            let workspace_subscription = cx.subscribe_in(
                &workspace_entity,
                window,
                |this: &mut Self, workspace, event, window, cx| {
                    if matches!(event, workspace::Event::ActiveItemChanged) {
                        this.update_active_view(workspace, window, cx);
                    }
                },
            );
            Self::build(fs, initial_view, window, cx, vec![workspace_subscription])
        })
    }

    fn build(
        fs: Arc<dyn Fs>,
        initial_view: Option<Entity<FigView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
        mut subscriptions: Vec<Subscription>,
    ) -> Self {
        let field_editor = cx.new(|cx| Editor::single_line(window, cx));
        subscriptions.push(cx.subscribe_in(
            &field_editor,
            window,
            |this: &mut Self, _, event: &EditorEvent, _window, cx| {
                if matches!(event, EditorEvent::Blurred) && this.editing_field.is_some() {
                    this.stop_editing(cx);
                }
            },
        ));
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            fs,
            active_view: None,
            width: None,
            field_editor,
            editing_field: None,
            content_scroll: ScrollHandle::new(),
            corner_radii_expanded: None,
            scrub: None,
            slider_tracks: [None; SLIDER_TRACK_COUNT],
            hidden_paint_alpha: HashMap::new(),
            picker: None,
            gradient_editor: None,
            swatch_press_dismissed: false,
            _subscriptions: subscriptions,
            _active_view_subscription: None,
        };
        this.set_active_view(initial_view, cx);
        this
    }

    fn update_active_view(
        &mut self,
        workspace: &Entity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_view = workspace
            .read(cx)
            .active_item(cx)
            .and_then(|item| item.downcast::<FigView>());
        self.set_active_view(active_view, cx);
    }

    fn set_active_view(&mut self, active_view: Option<Entity<FigView>>, cx: &mut Context<Self>) {
        match active_view {
            Some(view) => {
                let is_same = self
                    .active_view
                    .as_ref()
                    .is_some_and(|previous| previous.entity_id() == view.entity_id());
                if !is_same {
                    // Subscribe to the item's event stream rather than
                    // observing the view: the view notifies on every pan and
                    // pointer-move frame, which re-rendered the inspector at
                    // input rate. Transient preview frames still refresh it —
                    // live X/Y during drags — but that render is O(selection),
                    // not O(document).
                    let item = view.read(cx).item().clone();
                    self._active_view_subscription = Some(cx.subscribe(
                        &item,
                        |this, _, event: &crate::document::FigItemEvent, cx| {
                            match event {
                                crate::document::FigItemEvent::SelectionChanged => {
                                    this.reset_for_new_subject(true, cx);
                                }
                                crate::document::FigItemEvent::StateChanged => {
                                    // The document may have been replaced from
                                    // disk; a stale snapshot restore would
                                    // resurrect old content, so abandon any
                                    // preview without restoring.
                                    this.reset_for_new_subject(false, cx);
                                }
                                _ => {}
                            }
                            cx.notify();
                        },
                    ));
                    self.active_view = Some(view.downgrade());
                    self.editing_field = None;
                    self.scrub = None;
                    self.picker = None;
                    self.gradient_editor = None;
                    self.content_scroll.set_offset(gpui::Point::default());
                    self.corner_radii_expanded = None;
                    self.hidden_paint_alpha.clear();
                }
            }
            None => {
                // Keep the last canvas bound while other items are active so
                // the inspector doesn't flicker away on focus changes.
            }
        }
        cx.notify();
    }

    /// Reset per-subject view state when the inspected subject changes.
    /// `restore_previews` restores any in-flight scrub / picker preview to its
    /// gesture-start state first (safe while the document is unchanged; not
    /// safe across a reload, where the snapshot would resurrect stale data).
    fn reset_for_new_subject(&mut self, restore_previews: bool, cx: &mut Context<Self>) {
        self.cancel_scrub(restore_previews, cx);
        self.abandon_color_picker(restore_previews, cx);
        self.abandon_gradient_editor(restore_previews, cx);
        self.content_scroll.set_offset(gpui::Point::default());
        self.corner_radii_expanded = None;
        self.hidden_paint_alpha.clear();
    }

    fn active_view(&self, _cx: &App) -> Option<Entity<FigView>> {
        self.active_view.as_ref().and_then(|view| view.upgrade())
    }

    fn active_item(&self, cx: &App) -> Option<Entity<FigItem>> {
        Some(self.active_view(cx)?.read(cx).item().clone())
    }

    // === Mutations ========================================================

    /// Build operations against the current document and apply them through
    /// the item so undo history and dirty tracking stay correct.
    ///
    /// A gesture that authors more than one operation (aligning a selection,
    /// replacing a color across it, detaching an instance) is wrapped in a
    /// single history transaction, so the whole gesture collapses to one undo
    /// step instead of one per node.
    fn apply_document_ops(
        &mut self,
        cx: &mut Context<Self>,
        build: impl FnOnce(&Doc) -> Vec<Operation>,
    ) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let operations = {
            let item_state = item.read(cx);
            if !item_state.is_editable() {
                return;
            }
            let Some(document) = item_state.document() else {
                return;
            };
            finite_transform_operations(build(&document.doc))
        };
        match operations.len() {
            0 => {}
            1 => {
                item.update(cx, |item, cx| {
                    for operation in operations {
                        if let Err(error) = item.apply(operation, cx) {
                            log::error!(
                                "Fanta properties panel failed to apply operation: {error:#}"
                            );
                        }
                    }
                });
            }
            _ => self.apply_document_transaction(&item, operations, cx),
        }
    }

    /// Apply several operations as one undoable transaction. `Doc::apply`
    /// appends to an open transaction instead of committing per-op, so the
    /// whole batch pushes a single undo entry.
    fn apply_document_transaction(
        &mut self,
        item: &Entity<FigItem>,
        operations: Vec<Operation>,
        cx: &mut Context<Self>,
    ) {
        let label = operations
            .first()
            .map(|operation| operation.label().to_string())
            .unwrap_or_else(|| "Edit".to_string());
        item.update(cx, |item, cx| {
            let applied = item.with_document(cx, |document| {
                let doc = &mut document.doc;
                doc.history.begin(label, &mut doc.scene);
                for operation in operations {
                    if let Err(error) = doc.apply(operation) {
                        log::error!("Fanta properties panel failed to apply operation: {error:#}");
                        break;
                    }
                }
                doc.history.commit(&mut doc.scene);
                ((), DocChange::Content)
            });
            if applied.is_none() {
                log::debug!("dropping inspector edit: the document is not ready");
            }
        });
    }

    pub(crate) fn apply_align(&mut self, command: AlignCommand, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, |doc| {
            let ids: Vec<NodeId> = doc.selection.iter().copied().collect();
            let scene = &doc.scene;
            match command {
                AlignCommand::Left => align_horizontal(scene, &ids, HAlign::Left),
                AlignCommand::CenterHorizontal => align_horizontal(scene, &ids, HAlign::Center),
                AlignCommand::Right => align_horizontal(scene, &ids, HAlign::Right),
                AlignCommand::Top => align_vertical(scene, &ids, VAlign::Top),
                AlignCommand::MiddleVertical => align_vertical(scene, &ids, VAlign::Middle),
                AlignCommand::Bottom => align_vertical(scene, &ids, VAlign::Bottom),
                AlignCommand::DistributeHorizontal => distribute(scene, &ids, Axis::X),
                AlignCommand::DistributeVertical => distribute(scene, &ids, Axis::Y),
            }
        });
    }

    fn update_node_data(
        &mut self,
        id: NodeId,
        mutate: impl FnOnce(&mut NodeData),
        cx: &mut Context<Self>,
    ) {
        self.apply_document_ops(cx, move |doc| replace_data_operation(doc, id, mutate));
    }

    pub(crate) fn toggle_flag(&mut self, id: NodeId, flag: NodeFlags, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            vec![Operation::SetFlags {
                id,
                old: node.flags,
                new: node.flags ^ flag,
            }]
        });
    }

    pub(crate) fn set_blend_mode(&mut self, id: NodeId, blend_mode: BlendMode, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            if node.blend_mode == blend_mode {
                return Vec::new();
            }
            vec![Operation::SetBlendMode {
                id,
                old: node.blend_mode,
                new: blend_mode,
            }]
        });
    }

    pub(crate) fn add_paint(&mut self, id: NodeId, is_stroke: bool, cx: &mut Context<Self>) {
        self.forget_hidden_paint_alpha(id, is_stroke);
        self.update_node_data(
            id,
            move |data| {
                if is_stroke {
                    if let Some(strokes) = stroke_list_mut(data) {
                        strokes.push(Stroke::solid(FantaColor::BLACK, 1.0));
                    }
                } else {
                    add_fill(data);
                }
            },
            cx,
        );
    }

    /// Drop the visibility-alpha memory for a paint list whose indices are about
    /// to shift. `hidden_paint_alpha` keys paints by list position, so after an
    /// add/remove a surviving key would resolve to a *different* paint; the
    /// memory is best-effort (show falls back to opaque when absent), so forget
    /// it rather than restore the wrong paint's alpha.
    fn forget_hidden_paint_alpha(&mut self, id: NodeId, is_stroke: bool) {
        self.hidden_paint_alpha
            .retain(|key, _| !(key.id == id && key.is_stroke == is_stroke));
    }

    pub(crate) fn remove_paint(&mut self, id: NodeId, index: usize, is_stroke: bool, cx: &mut Context<Self>) {
        self.forget_hidden_paint_alpha(id, is_stroke);
        self.update_node_data(
            id,
            move |data| {
                if is_stroke {
                    if let Some(strokes) = stroke_list_mut(data)
                        && index < strokes.len()
                    {
                        strokes.remove(index);
                    }
                } else {
                    remove_fill(data, index);
                }
            },
            cx,
        );
    }

    /// Toggle a paint's visibility by zeroing / restoring its alpha — the
    /// closest analog of the original's per-paint eye in a data model that
    /// carries no per-paint visible flag. Works for every paint kind (a solid's
    /// alpha, every gradient stop's alpha, an image fill's opacity) and, on
    /// show, restores the alpha the paint had when it was hidden rather than
    /// forcing it fully opaque.
    pub(crate) fn toggle_paint_visibility(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        cx: &mut Context<Self>,
    ) {
        let key = PaintKey {
            id,
            index,
            is_stroke,
        };
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let current = {
            let item_state = item.read(cx);
            let Some(document) = item_state.document() else {
                return;
            };
            let Some(node) = document.doc.scene.get(id) else {
                return;
            };
            let mut data = node.data.clone();
            paint_slot_mut(&mut data, index, is_stroke).map(|paint| paint_alpha(paint))
        };
        let Some(current) = current else {
            return;
        };
        let restore = if paint_alpha_is_visible(&current) {
            self.hidden_paint_alpha.insert(key, current);
            None
        } else {
            // Unknown prior alpha (a doc that loaded with a zeroed paint, or a
            // panel rebuilt since): fall back to fully opaque. A remembered
            // alpha whose kind no longer matches the live paint (the paint was
            // converted solid↔gradient↔image while hidden) is discarded too —
            // `set_paint_alpha` would silently no-op on the mismatch, leaving
            // the "show" click dead.
            Some(
                self.hidden_paint_alpha
                    .remove(&key)
                    .filter(|remembered| {
                        std::mem::discriminant(remembered) == std::mem::discriminant(&current)
                    })
                    .unwrap_or_else(|| opaque_paint_alpha(&current)),
            )
        };
        self.update_node_data(
            id,
            move |data| {
                if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
                    match &restore {
                        Some(alpha) => set_paint_alpha(paint, alpha),
                        None => set_paint_alpha(paint, &zeroed_paint_alpha(paint)),
                    }
                }
            },
            cx,
        );
    }

    pub(crate) fn set_font_weight(&mut self, id: NodeId, weight: u16, cx: &mut Context<Self>) {
        // During active text edit with selection, apply only to the selected
        // range (rich text support via the live TextBuffer in the session).
        if self.active_view.as_ref().and_then(|w| w.upgrade()).map_or(false, |v| {
            v.update(cx, |view, cx| view.with_text_selection_style(cx, |s| { s.weight = weight; }))
        }) {
            return;
        }
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    text.style.weight = weight;
                }
            },
            cx,
        );
    }

    /// Route a typography field edit to the live text session's sub-selection
    /// when one exists on that node, so font family / size / line height /
    /// letter spacing mix per-run inside one paragraph (like weight and color
    /// already do). Returns true when consumed; validation mirrors the
    /// whole-node `field_operations` arms.
    fn try_apply_text_selection_field(
        &mut self,
        field: &InspectorField,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = match field {
            InspectorField::FontFamily(id)
            | InspectorField::FontSize(id)
            | InspectorField::LineHeight(id)
            | InspectorField::LetterSpacing(id) => *id,
            _ => return false,
        };
        let Some(view) = self.active_view.as_ref().and_then(|weak| weak.upgrade()) else {
            return false;
        };
        if view.read(cx).text_selection_typography(id).is_none() {
            return false;
        }
        let field = field.clone();
        let text = text.trim().to_string();
        view.update(cx, |view, cx| {
            view.with_text_selection_style(cx, |style| match &field {
                InspectorField::FontFamily(_) => {
                    if !text.is_empty() {
                        style.font_family = text.clone();
                    }
                }
                InspectorField::FontSize(_) => {
                    if let Some(size) = parse_number(&text).filter(|size| *size > 0.0) {
                        style.size_px = size;
                    }
                }
                InspectorField::LineHeight(_) => {
                    if let Some(line_height) =
                        parse_number(&text).filter(|line_height| *line_height > 0.0)
                    {
                        style.line_height = line_height;
                    }
                }
                InspectorField::LetterSpacing(_) => {
                    if let Some(letter_spacing) = parse_number(&text) {
                        style.letter_spacing = letter_spacing;
                    }
                }
                _ => {}
            })
        })
    }

    pub(crate) fn toggle_comment_resolved(
        &mut self,
        page: NodeId,
        id: String,
        cx: &mut Context<Self>,
    ) {
        self.apply_document_ops(cx, move |doc| {
            crate::comments::toggle_resolved_op(doc, page, &id)
                .into_iter()
                .collect()
        });
    }

    pub(crate) fn delete_comment(&mut self, page: NodeId, id: String, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            crate::comments::remove_comment_op(doc, page, &id)
                .into_iter()
                .collect()
        });
    }

    fn set_text_color(&mut self, id: NodeId, color: FantaColor, cx: &mut Context<Self>) {
        // During active text edit with selection, apply only to the selected
        // range (rich text). Falls back to whole-node for normal selection.
        if self.active_view.as_ref().and_then(|w| w.upgrade()).map_or(false, |v| {
            v.update(cx, |view, cx| view.with_text_selection_style(cx, |s| { s.color = color; }))
        }) {
            return;
        }
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    text.style.color = color;
                }
            },
            cx,
        );
    }

    pub(crate) fn toggle_text_decoration(
        &mut self,
        id: NodeId,
        decoration: TextDecorationGlyph,
        cx: &mut Context<Self>,
    ) {
        // Apply only to selection if text editing with active selection.
        if self.active_view.as_ref().and_then(|w| w.upgrade()).map_or(false, |v| {
            v.update(cx, |view, cx| {
                view.with_text_selection_style(cx, |s| {
                    match decoration {
                        TextDecorationGlyph::Italic => s.italic = !s.italic,
                        TextDecorationGlyph::Underline => s.underline = !s.underline,
                        TextDecorationGlyph::Strikethrough => s.strikethrough = !s.strikethrough,
                    }
                })
            })
        }) {
            return;
        }
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    let flag = match decoration {
                        TextDecorationGlyph::Italic => &mut text.style.italic,
                        TextDecorationGlyph::Underline => &mut text.style.underline,
                        TextDecorationGlyph::Strikethrough => &mut text.style.strikethrough,
                    };
                    *flag = !*flag;
                }
            },
            cx,
        );
    }

    pub(crate) fn set_text_align(&mut self, id: NodeId, align: TextAlign, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    text.align = align;
                }
            },
            cx,
        );
    }

    pub(crate) fn set_text_vertical_align(&mut self, id: NodeId, align: TextVAlign, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Text(text) = data {
                    text.vertical_align = align;
                }
            },
            cx,
        );
    }

    pub(crate) fn cycle_text_resize(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            |data| {
                if let NodeData::Text(text) = data {
                    text.auto_resize = next_text_resize(text.auto_resize);
                }
            },
            cx,
        );
    }

    pub(crate) fn set_stroke_align(&mut self, id: NodeId, align: StrokeAlign, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let Some(strokes) = stroke_list_mut(data) {
                    for stroke in strokes.iter_mut() {
                        stroke.align = align;
                    }
                }
            },
            cx,
        );
    }

    pub(crate) fn add_effect(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            effects_operations(doc, id, |effects| effects.push(default_shadow()))
        });
    }

    pub(crate) fn remove_effect(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            effects_operations(doc, id, |effects| {
                if index < effects.len() {
                    effects.remove(index);
                }
            })
        });
    }

    pub(crate) fn toggle_effect_kind(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            effects_operations(doc, id, |effects| {
                if let Some(shadow) = effects.get_mut(index) {
                    shadow.kind = match shadow.kind {
                        ShadowKind::Drop => ShadowKind::Inner,
                        ShadowKind::Inner => ShadowKind::Drop,
                    };
                }
            })
        });
    }

    pub(crate) fn add_blur(&mut self, id: NodeId, kind: BlurKind, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            blurs_operations(doc, id, |blurs| blurs.push(default_blur(kind)))
        });
    }

    pub(crate) fn remove_blur(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            blurs_operations(doc, id, |blurs| {
                if index < blurs.len() {
                    blurs.remove(index);
                }
            })
        });
    }

    pub(crate) fn set_blur_kind(&mut self, id: NodeId, index: usize, kind: BlurKind, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            blurs_operations(doc, id, |blurs| {
                if let Some(blur) = blurs.get_mut(index) {
                    blur.kind = kind;
                }
            })
        });
    }

    /// Add or remove a frame/group's auto layout. `FigItem::apply` flips the
    /// document's cached `uses_auto_layout` gate on when an op introduces the
    /// first auto layout, so the frame is re-solved immediately.
    pub(crate) fn toggle_auto_layout(&mut self, id: NodeId, enable: bool, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            // The solver treats an auto-layout group as a frame that carries a
            // `clip_size` — imported frames always do, but a plain group has
            // none. Seed one from the current content bounds when enabling on a
            // clip-less group, or a later "Hug contents" sizing collapses the
            // frame to a zero-extent box and clips away every child. Mirrors
            // `toggle_clip_content`.
            let local_size = doc
                .scene
                .local_bounds(id)
                .map(|bounds| [bounds.width(), bounds.height()]);
            replace_data_operation(doc, id, move |data| {
                if let NodeData::Group(group) = data {
                    group.auto_layout = enable.then(AutoLayout::default);
                    if enable && group.clip_size.is_none() {
                        group.clip_size = local_size;
                    }
                }
            })
        });
    }

    fn update_auto_layout(
        &mut self,
        id: NodeId,
        mutate: impl FnOnce(&mut AutoLayout),
        cx: &mut Context<Self>,
    ) {
        self.update_node_data(
            id,
            |data| {
                if let NodeData::Group(group) = data
                    && let Some(layout) = group.auto_layout.as_mut()
                {
                    mutate(layout);
                }
            },
            cx,
        );
    }

    pub(crate) fn set_layout_direction(&mut self, id: NodeId, mode: LayoutMode, cx: &mut Context<Self>) {
        self.update_auto_layout(id, move |layout| layout.mode = mode, cx);
    }

    pub(crate) fn set_primary_align(&mut self, id: NodeId, align: PrimaryAlign, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.primary_align = align, cx);
    }

    pub(crate) fn set_counter_align(&mut self, id: NodeId, align: CounterAlign, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.counter_align = align, cx);
    }

    /// Set both auto-layout alignments from a clicked 3×3 grid cell. Which
    /// visual axis is "primary" depends on the flow direction, like the
    /// original's Figma-style grid.
    pub(crate) fn set_align_cell(&mut self, id: NodeId, col: u8, row: u8, cx: &mut Context<Self>) {
        self.update_auto_layout(
            id,
            move |layout| {
                let (primary_cell, counter_cell) = match layout.mode {
                    LayoutMode::Horizontal => (col, row),
                    LayoutMode::Vertical => (row, col),
                };
                layout.primary_align = match primary_cell {
                    0 => PrimaryAlign::Start,
                    1 => PrimaryAlign::Center,
                    _ => PrimaryAlign::End,
                };
                layout.counter_align = match counter_cell {
                    0 => CounterAlign::Start,
                    1 => CounterAlign::Center,
                    _ => CounterAlign::End,
                };
            },
            cx,
        );
    }

    pub(crate) fn set_primary_axis_sizing(&mut self, id: NodeId, sizing: AxisSizing, cx: &mut Context<Self>) {
        self.update_auto_layout(id, move |layout| layout.primary_sizing = sizing, cx);
    }

    pub(crate) fn set_counter_axis_sizing(&mut self, id: NodeId, sizing: AxisSizing, cx: &mut Context<Self>) {
        self.update_auto_layout(id, move |layout| layout.counter_sizing = sizing, cx);
    }

    pub(crate) fn toggle_layout_wrap(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.wrap = !layout.wrap, cx);
    }

    pub(crate) fn toggle_layout_stacking(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.update_auto_layout(id, |layout| layout.reverse_z = !layout.reverse_z, cx);
    }

    pub(crate) fn toggle_clip_content(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            let local_size = doc
                .scene
                .local_bounds(id)
                .map(|bounds| [bounds.width(), bounds.height()])
                .unwrap_or([0.0, 0.0]);
            replace_data_operation(doc, id, |data| {
                if let NodeData::Group(group) = data {
                    group.clip_size = match group.clip_size {
                        Some(_) => None,
                        None => Some(local_size),
                    };
                }
            })
        });
    }

    pub(crate) fn update_layout_child(
        &mut self,
        id: NodeId,
        mutate: impl FnOnce(&mut LayoutChild),
        cx: &mut Context<Self>,
    ) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            let old = node.layout_child;
            let mut child = old.unwrap_or(LayoutChild {
                grow: 0.0,
                absolute: false,
                align_self: None,
            });
            mutate(&mut child);
            let new = (!child.is_trivial()).then_some(child);
            if new == old {
                return Vec::new();
            }
            vec![Operation::SetLayoutChild { id, old, new }]
        });
    }

    pub(crate) fn set_image_fit(&mut self, id: NodeId, fit: ImageFitMode, cx: &mut Context<Self>) {
        self.update_node_data(
            id,
            move |data| {
                if let NodeData::Bitmap(bitmap) = data {
                    bitmap.fit = fit;
                }
            },
            cx,
        );
    }

    pub(crate) fn set_instance_prop(
        &mut self,
        id: NodeId,
        prop: ComponentPropId,
        new: Option<VarValue>,
        cx: &mut Context<Self>,
    ) {
        self.apply_document_ops(cx, move |doc| {
            let Some(node) = doc.scene.get(id) else {
                return Vec::new();
            };
            let NodeData::Instance(instance) = &node.data else {
                return Vec::new();
            };
            let old = instance.prop_values.get(&prop).cloned();
            if old == new {
                return Vec::new();
            }
            vec![Operation::SetInstanceProp { id, prop, old, new }]
        });
    }

    pub(crate) fn cycle_variant_axis(&mut self, id: NodeId, axis: SharedString, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| {
            variant_cycle_operations(doc, id, axis.as_ref())
        });
    }

    /// Replace the instance with editable copies of its master's subtree. One
    /// undoable transaction: the detach itself, plus the ops that fold the
    /// master root's own surface props onto the (now plain) frame, which the
    /// data swap alone would drop.
    pub(crate) fn detach_instance(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, move |doc| detach_instance_operations(doc, id));
    }

    /// Merge the selected component masters into one variant set, so their
    /// instances can switch between them along a single axis.
    pub(crate) fn combine_as_variants(&mut self, cx: &mut Context<Self>) {
        self.apply_document_ops(cx, combine_as_variants_operations);
    }

    /// Select and scroll to the master a component instance renders, switching
    /// pages when the master lives on the hidden Components page.
    pub(crate) fn focus_main_component(&mut self, target: NodeId, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().clone();
        let selected_page = view.read(cx).selected_page_index();
        let (page_index, current_index) = {
            let fig_item = item.read(cx);
            let Some(document) = fig_item.document() else {
                return;
            };
            (
                document.page_index_of_node(target),
                document.page_index(selected_page),
            )
        };
        view.update(cx, |view, cx| {
            if let Some(index) = page_index
                && Some(index) != current_index
            {
                view.select_page(index, cx);
            }
            view.focus_node(target, cx);
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(target);
                ((), DocChange::Selection)
            });
        });
    }

    /// Set one paint's per-paint blend mode (gradient / image paints only —
    /// solids carry no blend in this model).
    pub(crate) fn set_paint_blend(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        blend: BlendMode,
        cx: &mut Context<Self>,
    ) {
        self.update_node_data(
            id,
            move |data| {
                if let Some(paint) = paint_slot_mut(data, index, is_stroke) {
                    match paint {
                        Fill::Gradient { blend: slot, .. } | Fill::Image { blend: slot, .. } => {
                            *slot = blend;
                        }
                        Fill::Solid { .. } => {}
                    }
                }
            },
            cx,
        );
    }

    // === Export ===========================================================

    /// Render the selected node (or the whole page when nothing is selected)
    /// at 2x into `<project_root>/exports/<name>.png` on a background thread.
    pub(crate) fn export_png(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let (item, selected_page_index) = {
            let view = view.read(cx);
            (view.item().clone(), view.selected_page_index())
        };
        let job = {
            let item = item.read(cx);
            let Some(project_root) = item.project_root().map(PathBuf::from) else {
                log::error!("Fanta PNG export requires a Fanta project on disk");
                return;
            };
            let Some(document) = item.document() else {
                return;
            };
            let doc = &document.doc;
            let selected = doc
                .selection
                .iter()
                .copied()
                .find(|id| doc.scene.contains(*id));
            let (root, name, bounds) = match selected {
                Some(id) => {
                    let Some(bounds) = doc.scene.world_bounds(id).filter(|bounds| {
                        bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0
                    }) else {
                        log::error!("Fanta PNG export skipped: the selected node has no bounds");
                        return;
                    };
                    let name = doc
                        .scene
                        .get(id)
                        .map(|node| node.name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "Untitled".to_string());
                    (Some(id), name, bounds)
                }
                None => {
                    let page = document.page(selected_page_index);
                    let root = page.and_then(|page| page.root);
                    let name = page
                        .map(|page| page.name.to_string())
                        .unwrap_or_else(|| "Page".to_string());
                    (root, name, crate::document::page_bounds(doc, root))
                }
            };
            ExportJob {
                doc: doc.clone(),
                asset_resolver: document.asset_resolver.clone(),
                root,
                name,
                bounds,
                project_root,
            }
        };
        cx.background_spawn(async move {
            match run_png_export(&job) {
                Ok(path) => log::info!("Fanta PNG export written to {}", path.display()),
                Err(error) => log::error!("Fanta PNG export failed: {error:#}"),
            }
        })
        .detach();
    }

    // === Inline editing ===================================================

    pub(crate) fn start_editing(
        &mut self,
        field: InspectorField,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_field = Some(field);
        self.field_editor.update(cx, |editor, cx| {
            editor.set_text(initial, window, cx);
            editor.select_all(&SelectAll, window, cx);
        });
        self.field_editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.editing_field = None;
        cx.notify();
    }

    fn cancel_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_editing(cx);
        self.focus_handle.focus(window, cx);
    }

    fn commit_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.editing_field.take() else {
            return;
        };
        let text = self.field_editor.read(cx).text(cx);
        self.focus_handle.focus(window, cx);
        cx.notify();
        if self.try_apply_text_selection_field(&field, text.trim(), cx) {
            return;
        }
        self.apply_document_ops(cx, |doc| field_operations(doc, &field, text.trim()));
    }

    /// The committed display text a field would show right now, used to seed
    /// the editor when Tab hops to the paired field.
    fn field_display_text(&self, field: &InspectorField, cx: &App) -> Option<String> {
        let item = self.active_item(cx)?;
        let item = item.read(cx);
        let document = item.document()?;
        read_field_text(&document.doc, field)
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_field.is_some() {
            match event.keystroke.key.as_str() {
                "enter" => {
                    cx.stop_propagation();
                    self.commit_editing(window, cx);
                }
                "escape" => {
                    cx.stop_propagation();
                    self.cancel_editing(window, cx);
                }
                "tab" => {
                    cx.stop_propagation();
                    let field = self.editing_field.clone();
                    self.commit_editing(window, cx);
                    if let Some(field) = field
                        && let Some(next) = paired_field(&field)
                        && let Some(initial) = self.field_display_text(&next, cx)
                    {
                        self.start_editing(next, initial, window, cx);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.picker.is_some() {
            match event.keystroke.key.as_str() {
                "enter" => {
                    cx.stop_propagation();
                    self.close_color_picker(true, cx);
                }
                "escape" => {
                    cx.stop_propagation();
                    self.close_color_picker(false, cx);
                }
                _ => {}
            }
            return;
        }
        if self.gradient_editor.is_some() {
            match event.keystroke.key.as_str() {
                "enter" => {
                    cx.stop_propagation();
                    self.close_gradient_editor(true, cx);
                }
                "escape" => {
                    cx.stop_propagation();
                    self.close_gradient_editor(false, cx);
                }
                _ => {}
            }
        }
    }

    // === Drag-to-scrub ====================================================

    /// Snapshot the state a gesture on `field` will mutate, so the preview can
    /// be recomputed from gesture-start each frame and the commit records
    /// `old` = gesture-start. `None` when the document isn't editable.
    fn snapshot_for_field(&self, field: &InspectorField, cx: &App) -> Option<NodeSnapshot> {
        let item = self.active_item(cx)?;
        let item = item.read(cx);
        if !item.is_editable() {
            return None;
        }
        let document = item.document()?;
        let id = field_node(field)?;
        let node = document.doc.scene.get(id)?;
        Some(NodeSnapshot {
            id,
            transform: node.transform,
            opacity: node.opacity,
            data: Box::new(node.data.clone()),
            effects: node.effects.clone(),
            blurs: node.blurs.clone(),
        })
    }

    pub(crate) fn begin_field_scrub(
        &mut self,
        field: InspectorField,
        start_value: f64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.finish_scrub(cx);
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        self.scrub = Some(ScrubState {
            field,
            snapshot,
            kind: ScrubKind::Relative,
            start_position: position,
            start_value,
            current_value: start_value,
            moved: false,
        });
    }

    /// Begin a slider gesture: the press position maps straight to a 0–100%
    /// value (a click alone sets and commits it on release).
    pub(crate) fn begin_track_scrub(
        &mut self,
        track: SliderTrack,
        field: InspectorField,
        start_percent: f64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.finish_scrub(cx);
        let Some(bounds) = self.slider_tracks[track as usize] else {
            return;
        };
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        let value = clamp_field_value(
            &field,
            track_value(
                f64::from(position.x),
                f64::from(bounds.left()),
                f64::from(bounds.size.width),
                0.0,
                100.0,
            ),
        );
        self.scrub = Some(ScrubState {
            field: field.clone(),
            snapshot: snapshot.clone(),
            kind: ScrubKind::Track {
                bounds,
                min: 0.0,
                max: 100.0,
            },
            start_position: position,
            start_value: start_percent,
            current_value: value,
            moved: true,
        });
        self.preview_field_text(&field, &snapshot, &format_number(value), cx);
    }

    fn handle_scrub_move(
        &mut self,
        event: &DragMoveEvent<PanelDrag>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scrub) = self.scrub.as_mut() else {
            return;
        };
        let position = event.event.position;
        let modifiers = event.event.modifiers;
        let value = match &scrub.kind {
            ScrubKind::Relative => {
                let dx = f64::from(position.x - scrub.start_position.x);
                scrub_value(scrub.start_value, dx, modifiers.shift, modifiers.alt)
            }
            ScrubKind::Track { bounds, min, max } => track_value(
                f64::from(position.x),
                f64::from(bounds.left()),
                f64::from(bounds.size.width),
                *min,
                *max,
            ),
        };
        let value = clamp_field_value(&scrub.field, value);
        if scrub.moved && value == scrub.current_value {
            return;
        }
        scrub.moved = true;
        scrub.current_value = value;
        let field = scrub.field.clone();
        let snapshot = scrub.snapshot.clone();
        self.preview_field_text(&field, &snapshot, &format_number(value), cx);
    }

    /// Commit an in-flight scrub as ONE undoable operation: restore the
    /// gesture-start state, then author the op from there to the final value —
    /// the same transient-then-commit staging the canvas move tool uses.
    pub(crate) fn finish_scrub(&mut self, cx: &mut Context<Self>) {
        let Some(scrub) = self.scrub.take() else {
            return;
        };
        if !scrub.moved {
            return;
        }
        self.restore_snapshot_preview(&scrub.snapshot, cx);
        if scrub.current_value != scrub.start_value {
            let field = scrub.field.clone();
            let text = format_number(scrub.current_value);
            if !self.try_apply_text_selection_field(&field, &text, cx) {
                self.apply_document_ops(cx, |doc| field_operations(doc, &field, &text));
            }
        }
        cx.notify();
    }

    /// Drop an in-flight scrub. `restore` puts the document back to the
    /// gesture-start state (skip after a reload, where the snapshot is stale).
    fn cancel_scrub(&mut self, restore: bool, cx: &mut Context<Self>) {
        if let Some(scrub) = self.scrub.take()
            && scrub.moved
            && restore
        {
            self.restore_snapshot_preview(&scrub.snapshot, cx);
        }
    }

    /// Apply `text` to `field` as a TRANSIENT preview: restore the snapshot,
    /// author the same operations the commit path would, and write their `new`
    /// values straight into the scene (no history), reporting a
    /// `ContentPreview` so the canvas repaints without a layout re-solve.
    fn preview_field_text(
        &self,
        field: &InspectorField,
        snapshot: &NodeSnapshot,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let field = field.clone();
        let snapshot = snapshot.clone();
        let text = text.to_string();
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                return;
            }
            let applied = item.with_document(cx, |document| {
                restore_snapshot(&mut document.doc, &snapshot);
                let operations =
                    finite_transform_operations(field_operations(&document.doc, &field, &text));
                for operation in &operations {
                    apply_preview_operation(&mut document.doc, operation);
                }
                ((), DocChange::ContentPreview)
            });
            if applied.is_none() {
                log::debug!("dropping inspector preview: the document is not ready");
            }
        });
    }

    fn restore_snapshot_preview(&self, snapshot: &NodeSnapshot, cx: &mut Context<Self>) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let snapshot = snapshot.clone();
        item.update(cx, |item, cx| {
            let applied = item.with_document(cx, |document| {
                restore_snapshot(&mut document.doc, &snapshot);
                ((), DocChange::ContentPreview)
            });
            if applied.is_none() {
                log::debug!("dropping inspector preview restore: the document is not ready");
            }
        });
    }

    // === Color picker =====================================================

    /// Open (or toggle closed) the color-picker popover for `field`, seeded
    /// with the paint's current color. While open, every picker change
    /// previews transiently; closing commits one undoable operation.
    pub(crate) fn toggle_color_picker(
        &mut self,
        field: InspectorField,
        current: FantaColor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .picker
            .as_ref()
            .is_some_and(|session| session.field == field)
        {
            self.close_color_picker(true, cx);
            return;
        }
        self.close_color_picker(true, cx);
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        let picker = cx.new(|cx| ColorPicker::new(current, window, cx));
        let subscription = cx.subscribe(
            &picker,
            |this, _, event: &ColorPickerEvent, cx| match event {
                ColorPickerEvent::Changed(color) => this.preview_picker_color(*color, cx),
                ColorPickerEvent::Commit => this.close_color_picker(true, cx),
                ColorPickerEvent::Cancel => this.close_color_picker(false, cx),
            },
        );
        // Focusing the picker would blur the canvas, and the text session
        // commits on focus-out — killing the sub-selection the picked color
        // should apply to. Keep canvas focus while a session is live (mouse
        // picking still works; Figma likewise keeps you in text-edit mode
        // while driving the inspector).
        if !self.text_selection_targets_field(&field, cx) {
            picker.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.picker = Some(PickerSession {
            field,
            original: current,
            snapshot,
            changed: false,
            picker,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Whether `field` is the glyph color of a text node with a live edit
    /// session — the case where picker changes must route to the session's
    /// sub-selection rather than the whole node.
    fn text_selection_targets_field(&self, field: &InspectorField, cx: &App) -> bool {
        let InspectorField::TextColor(id) = field else {
            return false;
        };
        self.active_view
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .is_some_and(|view| view.read(cx).text_selection_typography(*id).is_some())
    }

    fn preview_picker_color(&mut self, color: FantaColor, cx: &mut Context<Self>) {
        let Some(session) = self.picker.as_mut() else {
            return;
        };
        session.changed = true;
        let field = session.field.clone();
        let snapshot = session.snapshot.clone();
        // A live text sub-selection recolors through the session so only the
        // selected characters preview (and the eventual commit matches what
        // the user watched). The whole-node preview below would repaint every
        // glyph.
        if self.text_selection_targets_field(&field, cx) {
            if let Some(view) = self.active_view.as_ref().and_then(|weak| weak.upgrade()) {
                view.update(cx, |view, cx| {
                    view.with_text_selection_style(cx, |style| style.color = color);
                });
                return;
            }
        }
        self.preview_field_text(&field, &snapshot, &color.to_hex(), cx);
    }

    /// Close the picker popover. `commit` keeps the picked color by restoring
    /// the pre-open state and authoring one undoable operation; otherwise the
    /// preview is rolled back.
    fn close_color_picker(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(session) = self.picker.take() else {
            return;
        };
        if session.changed {
            // A cancelled pick on a live text sub-selection re-applies the
            // pre-open color through the session (the node-snapshot restore
            // below can't reach the session's buffer).
            if !commit && self.text_selection_targets_field(&session.field, cx) {
                if let Some(view) = self.active_view.as_ref().and_then(|weak| weak.upgrade()) {
                    let original = session.original;
                    view.update(cx, |view, cx| {
                        view.with_text_selection_style(cx, |style| style.color = original);
                    });
                }
            }
            self.restore_snapshot_preview(&session.snapshot, cx);
            if commit {
                let final_color = session.picker.read(cx).color();
                if final_color != session.original {
                    let field = session.field.clone();
                    if let InspectorField::TextColor(id) = &field {
                        self.set_text_color(*id, final_color, cx);
                    } else {
                        let text = final_color.to_hex();
                        self.apply_document_ops(cx, |doc| field_operations(doc, &field, &text));
                    }
                }
            }
        }
        cx.notify();
    }

    /// Drop the picker popover without committing. `restore` rolls back any
    /// live preview (skip after a reload, where the snapshot is stale).
    fn abandon_color_picker(&mut self, restore: bool, cx: &mut Context<Self>) {
        if let Some(session) = self.picker.take()
            && session.changed
            && restore
        {
            self.restore_snapshot_preview(&session.snapshot, cx);
        }
    }

    // === Gradient editor ==================================================

    /// Open (or toggle closed) the gradient-editor popover for a gradient
    /// paint. `is_stroke` selects the paint list; `gradient` seeds the editor.
    /// While open, every change previews transiently; closing commits one op.
    pub(crate) fn toggle_gradient_editor(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        gradient: Gradient,
        cx: &mut Context<Self>,
    ) {
        let field = InspectorField::Gradient {
            id,
            index,
            is_stroke,
        };
        if self
            .gradient_editor
            .as_ref()
            .is_some_and(|session| session.field == field)
        {
            self.close_gradient_editor(true, cx);
            return;
        }
        self.close_color_picker(true, cx);
        self.close_gradient_editor(true, cx);
        let Some(snapshot) = self.snapshot_for_field(&field, cx) else {
            return;
        };
        let editor = cx.new(|cx| GradientEditor::new(gradient.clone(), cx));
        let subscription =
            cx.subscribe(
                &editor,
                |this, _, event: &GradientEditorEvent, cx| match event {
                    GradientEditorEvent::Changed(gradient) => {
                        this.preview_gradient(gradient.clone(), cx)
                    }
                    GradientEditorEvent::Commit => this.close_gradient_editor(true, cx),
                    GradientEditorEvent::Cancel => this.close_gradient_editor(false, cx),
                },
            );
        self.gradient_editor = Some(GradientSession {
            field,
            original: gradient,
            snapshot,
            changed: false,
            editor,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn preview_gradient(&mut self, gradient: Gradient, cx: &mut Context<Self>) {
        let Some(session) = self.gradient_editor.as_mut() else {
            return;
        };
        session.changed = true;
        let InspectorField::Gradient {
            id,
            index,
            is_stroke,
        } = session.field
        else {
            return;
        };
        let snapshot = session.snapshot.clone();
        self.preview_gradient_paint(id, index, is_stroke, &snapshot, gradient, cx);
    }

    /// Preview a gradient as a transient (no-history) edit: restore the
    /// snapshot, then write the gradient straight into the fill/stroke slot.
    fn preview_gradient_paint(
        &self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        snapshot: &NodeSnapshot,
        gradient: Gradient,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.active_item(cx) else {
            return;
        };
        let snapshot = snapshot.clone();
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                return;
            }
            let applied = item.with_document(cx, |document| {
                restore_snapshot(&mut document.doc, &snapshot);
                let operations = finite_transform_operations(replace_data_operation(
                    &document.doc,
                    id,
                    |data| set_paint_gradient(data, index, is_stroke, gradient.clone()),
                ));
                for operation in &operations {
                    apply_preview_operation(&mut document.doc, operation);
                }
                ((), DocChange::ContentPreview)
            });
            if applied.is_none() {
                log::debug!("dropping inspector gradient preview: the document is not ready");
            }
        });
    }

    /// Close the gradient editor. `commit` keeps the edited gradient by
    /// restoring the pre-open state and authoring one undoable op; otherwise
    /// the preview is rolled back.
    fn close_gradient_editor(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(session) = self.gradient_editor.take() else {
            return;
        };
        if session.changed {
            self.restore_snapshot_preview(&session.snapshot, cx);
            if commit {
                let final_gradient = session.editor.read(cx).gradient();
                if final_gradient != session.original
                    && let InspectorField::Gradient {
                        id,
                        index,
                        is_stroke,
                    } = session.field
                {
                    self.update_node_data(
                        id,
                        move |data| set_paint_gradient(data, index, is_stroke, final_gradient),
                        cx,
                    );
                }
            }
        }
        cx.notify();
    }

    /// Drop the gradient editor without committing. `restore` rolls back any
    /// live preview (skip after a reload, where the snapshot is stale).
    fn abandon_gradient_editor(&mut self, restore: bool, cx: &mut Context<Self>) {
        if let Some(session) = self.gradient_editor.take()
            && session.changed
            && restore
        {
            self.restore_snapshot_preview(&session.snapshot, cx);
        }
    }

    /// Change a paint's kind: Solid seeds a two-stop gradient (or flattens a
    /// gradient back to its representative solid color); the gradient kinds
    /// convert an existing gradient or seed one from the current solid. One
    /// undoable op.
    pub(crate) fn set_paint_kind(
        &mut self,
        id: NodeId,
        index: usize,
        is_stroke: bool,
        kind: PaintKind,
        cx: &mut Context<Self>,
    ) {
        self.close_gradient_editor(true, cx);
        self.close_color_picker(true, cx);
        self.update_node_data(
            id,
            move |data| convert_paint_kind(data, index, is_stroke, kind),
            cx,
        );
    }

    // === Snapshot =========================================================

    fn build_snapshot(&self, cx: &App) -> InspectorSnapshot {
        let Some(view) = self.active_view(cx) else {
            return InspectorSnapshot::Message(NO_DOCUMENT_MESSAGE.into());
        };
        let (item, selected_page_index) = {
            let view = view.read(cx);
            (view.item().clone(), view.selected_page_index())
        };
        let item = item.read(cx);
        if let Some(message) = item.document.loading_message() {
            return InspectorSnapshot::Message(message);
        }
        if let Some(error) = item.document.error() {
            return InspectorSnapshot::Message(
                format!("Could not open Figma file: {error:#}").into(),
            );
        }
        let Some(document) = item.document() else {
            return InspectorSnapshot::Message(NO_DOCUMENT_MESSAGE.into());
        };
        let editable = item.is_editable();
        let doc = &document.doc;
        let selection: Vec<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|id| doc.scene.contains(*id))
            .collect();
        // One O(library) scan per snapshot build — never per row. Both the
        // single-node "is this a component master" test and the multi-select
        // "how many masters are selected" count read it.
        let masters = master_roots(&doc.components);
        let body = match selection.as_slice() {
            [] => InspectorBody::Page(page_section(document, selected_page_index)),
            [id] => match node_section(doc, *id, &masters) {
                Some(mut node) => {
                    // While a text session is live, the panel reflects the
                    // sub-selection's style (and edits route to it through
                    // `with_text_selection_style`), not the node's base style.
                    if let (Some(typography), Some(selection_typography)) = (
                        node.typography.as_mut(),
                        view.read(cx).text_selection_typography(*id),
                    ) {
                        crate::properties_snapshot::overlay_selection_typography(
                            typography,
                            &selection_typography,
                        );
                    }
                    InspectorBody::Node(Box::new(node))
                }
                None => InspectorBody::Page(page_section(document, selected_page_index)),
            },
            ids => InspectorBody::Multi(multi_section(doc, ids, &masters)),
        };
        InspectorSnapshot::Ready {
            editable,
            selection_len: selection.len(),
            body,
        }
    }

}

impl Render for FantaPropertiesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_started = std::time::Instant::now();
        let snapshot = self.build_snapshot(cx);
        crate::report_slow("properties panel snapshot", render_started);

        // A scrub whose drag ended outside the panel gets no drop event; the
        // drag's end still forces a redraw, so commit it from here.
        if self.scrub.as_ref().is_some_and(|scrub| scrub.moved) && !cx.has_active_drag() {
            cx.defer_in(window, |this, _, cx| this.finish_scrub(cx));
        }

        let root = v_flex()
            .key_context("FantaPropertiesPanel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_drag_move(cx.listener(Self::handle_scrub_move))
            .on_drop(cx.listener(|this, _: &PanelDrag, _, cx| this.finish_scrub(cx)))
            .size_full()
            .bg(cx.theme().colors().panel_background);
        match snapshot {
            InspectorSnapshot::Message(message) => root.child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .child(Label::new(message).color(Color::Muted)),
            ),
            InspectorSnapshot::Ready {
                editable,
                selection_len,
                body,
            } => {
                let mut content = v_flex()
                    .id("fanta-properties-content")
                    .flex_1()
                    .min_h_0()
                    .track_scroll(&self.content_scroll)
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .pb_4();
                // Sections after the header are divider-separated; collect them
                // so the per-kind matrix below reads as a list of section rows
                // rather than an interleaved `.child(Divider)` chain.
                let mut sections: Vec<AnyElement> = Vec::new();
                match body {
                    InspectorBody::Page(page) => {
                        content = content.child(self.render_header(
                            "Page".into(),
                            page.name.clone(),
                            page.id.map(InspectorField::Name),
                            editable,
                            cx,
                        ));
                        sections.push(self.render_align_section(selection_len, editable, cx));
                        sections.push(self.render_page_properties(&page, editable, cx));
                        sections.push(self.render_export_section(editable, None, cx));
                    }
                    InspectorBody::Node(node) => {
                        use NodeKind::*;
                        content = content.child(self.render_header(
                            node.type_name.clone(),
                            node.name.clone(),
                            Some(InspectorField::Name(node.id)),
                            editable,
                            cx,
                        ));
                        let id = node.id;
                        let kind = node.kind;

                        // ---- The per-node-kind section matrix ----------------
                        // Section order is fixed. `Y` = shown, `-` = hidden.
                        //
                        //                    Frame Group Shape Text Image Inst Comp Other
                        //  Align               Y     Y     Y     Y    Y     Y    Y    Y
                        //  Position (X/Y/W/H)  Y     Y     Y     Y    Y     Y    Y    Y
                        //  Component master    -     -     -     -    -     -    Y    -
                        //  Instance info       -     -     -     -    -     Y    -    -
                        //  Typography          -     -     -     Y    -     -    -    -
                        //  Image (fit only)    -     -     -     -    Y     -    -    -
                        //  Layout              Y     Y     -     -    -     -    Y¹   -
                        //  Auto layout child  parent is an auto-layout frame    -    "
                        //  Appearance          Y     Y     Y     Y    Y     Y    Y    Y
                        //   + radius/smoothing Y     Y     Y     -    -     -    Y    -
                        //  Fill                Y(bg) Y(bg) Y     Y²   -     -³   Y(bg) -
                        //  Stroke              Y⁴    Y⁴    Y     -    -     -    Y⁴   -
                        //  Effects (+ blur)    Y     Y     Y     Y    Y     Y    Y    Y
                        //  Interactions        Y     Y     Y     Y    Y     Y    Y    Y
                        //  Export              Y     Y     Y     Y    Y     Y    Y    Y
                        //
                        // 1. Only when the master's root is a `Group`.
                        // 2. A single glyph-color row, not a paint stack.
                        // 3. Instance fills are reached through exposed Color
                        //    props, never a raw paint stack.
                        // 4. Broader than the original, which serves strokes to
                        //    vectors only — group/frame strokes are real here.
                        sections.push(self.render_align_section(selection_len, editable, cx));
                        sections.push(self.render_position_section(&node, editable, cx));

                        // Component master identity + variant set + schema.
                        if let Some(master) = &node.master {
                            sections.push(self.render_master_section(master));
                        }
                        // Instance info: master, variants, props, detach.
                        if let Some(instance) = &node.instance {
                            sections.push(self.render_instance_section(id, instance, editable, cx));
                        }
                        if kind == Text
                            && let Some(typography) = &node.typography
                        {
                            sections.push(
                                self.render_typography_section(
                                    id, typography, editable, window, cx,
                                ),
                            );
                        }
                        if kind == Image
                            && let Some(image_fit) = node.image_fit
                        {
                            sections.push(
                                self.render_image_section(id, image_fit, editable, window, cx),
                            );
                        }
                        // Layout serves the group-backed kinds — a component
                        // master rooted at a frame gets it too.
                        if matches!(kind, Frame | Group | Component)
                            && let Some(layout) = &node.layout
                        {
                            sections
                                .push(self.render_layout_section(id, layout, editable, window, cx));
                        }
                        // A master is never a child of an auto-layout frame in
                        // the scene sense the inspector cares about.
                        if kind != Component
                            && let Some(layout_child) = &node.layout_child
                        {
                            sections.push(self.render_layout_child_section(
                                id,
                                layout_child,
                                editable,
                                cx,
                            ));
                        }
                        sections.push(self.render_appearance_section(&node, editable, window, cx));
                        if let Some(fills) = &node.fills {
                            sections.push(self.render_paint_section(
                                id, "Fill", fills, None, false, editable, window, cx,
                            ));
                        } else if kind == Text
                            && let Some(typography) = &node.typography
                        {
                            // Text has no paint stack: its Fill is the glyph color.
                            sections.push(self.render_text_fill_section(
                                id,
                                typography.color,
                                typography.color_mixed,
                                editable,
                                cx,
                            ));
                        }
                        if let Some(strokes) = &node.strokes {
                            sections.push(self.render_paint_section(
                                id,
                                "Stroke",
                                strokes,
                                node.stroke_align,
                                true,
                                editable,
                                window,
                                cx,
                            ));
                        }
                        sections.push(self.render_effects_section(
                            id,
                            &node.effects,
                            &node.blurs,
                            editable,
                            cx,
                        ));
                        if !node.reactions.is_empty() {
                            sections.push(self.render_interactions_section(&node.reactions));
                        }
                        if !node.bindings.is_empty() {
                            sections.push(self.render_bindings_section(&node.bindings));
                        }
                        sections.push(self.render_export_section(
                            editable,
                            Some(node.type_icon),
                            cx,
                        ));
                    }
                    InspectorBody::Multi(multi) => {
                        content = content.child(self.render_header(
                            "Selection".into(),
                            format!("{} selected", multi.count),
                            None,
                            editable,
                            cx,
                        ));
                        sections.push(self.render_align_section(selection_len, editable, cx));
                        sections.push(self.render_multi_position_section(&multi, cx));
                        sections.push(self.render_selection_colors_section(
                            &multi.colors,
                            editable,
                            cx,
                        ));
                        if editable && multi.master_count >= 2 {
                            sections
                                .push(self.render_combine_variants_section(multi.master_count, cx));
                        }
                    }
                }
                for section in sections {
                    content = content.child(Divider::horizontal()).child(section);
                }
                root.child(content)
            }
        }
    }
}

impl EventEmitter<PanelEvent> for FantaPropertiesPanel {}

impl Focusable for FantaPropertiesPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for FantaPropertiesPanel {
    fn persistent_name() -> &'static str {
        "Fanta Properties Panel"
    }

    fn panel_key() -> &'static str {
        "FantaPropertiesPanel"
    }

    fn position(&self, _window: &Window, cx: &App) -> DockPosition {
        FantaPropertiesPanelSettings::get_global(cx).dock
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        position: DockPosition,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings.fanta_properties_panel.get_or_insert_default().dock = Some(position.into());
        });
    }

    fn default_size(&self, _window: &Window, cx: &App) -> Pixels {
        self.width
            .unwrap_or_else(|| FantaPropertiesPanelSettings::get_global(cx).default_width)
    }

    fn min_size(&self, _window: &Window, _cx: &App) -> Option<Pixels> {
        // Below this the 2-up field grid degenerates into unusable slivers.
        Some(px(260.))
    }

    fn icon(&self, _window: &Window, cx: &App) -> Option<IconName> {
        (FantaPropertiesPanelSettings::get_global(cx).button && self.active_view.is_some())
            .then_some(IconName::Sliders)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Fanta Properties Panel")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        9
    }

    fn enabled(&self, _cx: &App) -> bool {
        self.active_view.is_some()
    }
}

#[cfg(test)]
mod panel_integration_tests {
    //! End-to-end coverage for the gradient path through the *real*
    //! [`FantaPropertiesPanel`]: a Fanta project with a gradient-filled node is
    //! written to a real temp dir, opened through a `Project`/`FigView`, mounted
    //! in the panel, and drawn — then the gradient editor is opened, previewed,
    //! and committed. This reproduces the "crash on opening gradients" a plain
    //! render test of the gradient widgets alone cannot, because the panic lives
    //! in the panel ↔ item preview/commit wiring, not in the widgets.
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    use fanta_doc::{CanvasNode, GradientStop, GroupNode, VectorNode};

    use crate::properties_ops::fill_slot_mut;
    use gpui::{MouseButton, TestAppContext};
    use project::{Project, ProjectItem as _, ProjectPath};
    use workspace::ProjectItem as _;

    fn init_test(cx: &mut TestAppContext) {
        // The panel loads its document off a real temp dir through a real
        // `Project`, whose worktree scan and file watcher block on real IO.
        cx.executor().allow_parking();
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn linear_gradient_fill() -> Gradient {
        Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: FantaColor::BLACK,
                },
                GradientStop {
                    position: 1.0,
                    color: FantaColor::WHITE,
                },
            ],
        }
    }

    /// Write a Fanta project containing a page with a gradient-filled vector and
    /// a text node, returning their stable node ids.
    fn write_gradient_project(root: &Path, gradient: Gradient) -> (NodeId, NodeId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page One".into();
        let page_id = page.id;
        doc.scene.insert(page).unwrap();
        doc.add_page(page_id);

        let mut vector =
            VectorNode::rect_solid(0.0, 0.0, 120.0, 60.0, FantaColor::rgb(200, 200, 200));
        vector.fills = smallvec::smallvec![Fill::Gradient {
            gradient,
            blend: BlendMode::Normal,
        }];
        let mut node = CanvasNode::new(NodeData::Vector(vector));
        node.parent = Some(page_id);
        node.name = "Gradient Rect".into();
        let vector_id = node.id;
        doc.scene.insert(node).unwrap();

        // A text node so the typography / decorations / glyph-fill sections get
        // laid out by the draw tests too.
        let mut text = fanta_doc::TextNode::new("Hello", 100.0, 24.0);
        text.style.weight = 350; // an off-ladder weight must survive a render
        text.align = TextAlign::Justify;
        let mut text_node = CanvasNode::new(NodeData::Text(text));
        text_node.parent = Some(page_id);
        text_node.name = "Label".into();
        let text_id = text_node.id;
        doc.scene.insert(text_node).unwrap();

        doc.set_active_page(Some(page_id));

        fanta_format::write_project_tree(root, &doc, &BTreeMap::new())
            .expect("writing the fanta project tree");
        (vector_id, text_id)
    }

    struct Harness {
        panel: gpui::WindowHandle<FantaPropertiesPanel>,
        vector_id: NodeId,
        text_id: NodeId,
        _view: Entity<FigView>,
        _temp: tempfile::TempDir,
    }

    impl Harness {
        /// Replace the document selection, the way the canvas would.
        fn select(&self, ids: &[NodeId], cx: &mut TestAppContext) {
            let item = self._view.read_with(cx, |view, _| view.item().clone());
            let ids = ids.to_vec();
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.selection.replace_with(ids);
                    ((), DocChange::Selection)
                });
            });
            cx.run_until_parked();
        }
    }

    async fn open_panel_with_gradient(gradient: Gradient, cx: &mut TestAppContext) -> Harness {
        let temp = tempfile::tempdir().unwrap();
        let (vector_id, text_id) = write_gradient_project(temp.path(), gradient);

        let fs = std::sync::Arc::new(fs::RealFs::new(None, cx.executor()));
        let project = Project::test(fs.clone(), [temp.path()], cx).await;
        let worktree_id = project.update(cx, |project, cx| {
            project.worktrees(cx).next().unwrap().read(cx).id()
        });
        let path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path("fanta.json").into(),
        };

        let item = cx
            .update(|cx| FigItem::try_open(&project, &path, cx))
            .expect("fanta.json is openable as a FigItem")
            .await
            .expect("loading the FigItem");
        cx.run_until_parked();

        // The document loads on a background task; wait for it, then select the
        // gradient node so the inspector shows its fill section.
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(vector_id);
                ((), DocChange::Selection)
            });
        });
        cx.run_until_parked();

        // A throwaway window supplies the `&mut Window` FigView construction
        // needs; the FigView entity itself is not window-bound.
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| {
                    FigView::for_project_item(project.clone(), None, item.clone(), window, cx)
                })
            })
            .unwrap();

        let fs_dyn: std::sync::Arc<dyn fs::Fs> = fs;
        let panel = cx.add_window(|window, cx| FantaPropertiesPanel {
            focus_handle: cx.focus_handle(),
            fs: fs_dyn,
            active_view: None,
            width: None,
            field_editor: cx.new(|cx| Editor::single_line(window, cx)),
            editing_field: None,
            content_scroll: ScrollHandle::new(),
            corner_radii_expanded: None,
            scrub: None,
            slider_tracks: [None; SLIDER_TRACK_COUNT],
            hidden_paint_alpha: HashMap::new(),
            picker: None,
            gradient_editor: None,
            swatch_press_dismissed: false,
            _subscriptions: Vec::new(),
            _active_view_subscription: None,
        });
        panel
            .update(cx, |panel, _, cx| {
                panel.set_active_view(Some(view.clone()), cx);
            })
            .unwrap();
        cx.run_until_parked();

        Harness {
            panel,
            vector_id,
            text_id,
            _view: view,
            _temp: temp,
        }
    }

    fn draw(panel: gpui::WindowHandle<FantaPropertiesPanel>, cx: &mut TestAppContext) {
        cx.update_window(panel.into(), |_, window, cx| {
            window.draw(cx).clear();
        })
        .expect("drawing the properties panel");
    }

    #[gpui::test]
    async fn gradient_fill_row_draws_without_panicking(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn opening_the_gradient_editor_and_previewing_does_not_panic(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;
        draw(harness.panel, cx);

        // Open the gradient editor from the swatch, then draw the popover.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, linear_gradient_fill(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        // Drive a change through the editor exactly like a user edit: the editor
        // emits `Changed`, the panel's subscription previews it onto the item.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                editor.update(cx, |editor, cx| {
                    editor.set_kind(crate::color_picker::GradientKind::Radial, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        // Open the nested stop color picker, draw both popovers, then drive a
        // color change all the way through: stop picker → gradient editor →
        // panel preview → item. Finally commit the stop picker (which drops the
        // subscription that is mid-callback) and the whole editor.
        harness
            .panel
            .update(cx, |panel, window, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                editor.update(cx, |editor, cx| editor.open_stop_picker(0, window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        harness
            .panel
            .update(cx, |panel, window, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                let picker = editor.read(cx).stop_picker_for_test().unwrap();
                picker.update(cx, |picker, cx| {
                    picker.set_test_color(FantaColor::rgb(12, 240, 33), window, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        // Commit the edit (one undoable op) and redraw.
        harness
            .panel
            .update(cx, |panel, _, cx| panel.close_gradient_editor(true, cx))
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn converting_solid_to_gradient_via_paint_type_does_not_panic(cx: &mut TestAppContext) {
        init_test(cx);
        // Start from a gradient node, flatten to solid, then back to a gradient
        // through the same `set_paint_kind` path the paint-type dropdown drives.
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_paint_kind(vector_id, 0, false, PaintKind::Solid, cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_paint_kind(
                    vector_id,
                    0,
                    false,
                    PaintKind::Gradient(crate::color_picker::GradientKind::Angular),
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn real_clicks_open_and_dismiss_the_gradient_editor(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        // A tall panel so the fill section (near the bottom of the stack) is
        // laid out and hit-testable.
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let swatch = vcx
            .debug_bounds("fanta-fill-gradient-swatch-0")
            .expect("the gradient fill swatch is laid out");
        // Real click on the swatch dispatches through the hitbox / click
        // machinery — the exact path the app takes to open the editor.
        vcx.simulate_click(swatch.center(), gpui::Modifiers::default());
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        // Click far outside the popover: the editor's `on_mouse_down_out`
        // commits and dismisses it.
        vcx.simulate_click(gpui::Point::new(px(5.), px(5.)), gpui::Modifiers::default());
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();
    }

    fn node_x(harness: &Harness, cx: &mut TestAppContext) -> f64 {
        harness._view.read_with(cx, |view, cx| {
            let item = view.item().read(cx);
            let doc = &item.document().unwrap().doc;
            doc.scene
                .get(harness.vector_id)
                .unwrap()
                .transform
                .0
                .translation
                .x
        })
    }

    #[gpui::test]
    async fn real_drag_on_numeric_label_scrubs_the_value(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let start_x = node_x(&harness, cx);

        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let handle = vcx
            .debug_bounds("scrub-fanta-x-0")
            .expect("the X field scrub handle is laid out");
        let origin = handle.center();
        // Press, drag right by 40px past the gpui drag threshold, release —
        // exactly the gesture the app's scrub relies on.
        vcx.simulate_mouse_down(origin, MouseButton::Left, gpui::Modifiers::default());
        // Incremental moves, like a real drag: the first move past the threshold
        // only initiates gpui's drag; scrub deltas arrive on later moves.
        for step in 1..=8 {
            vcx.simulate_mouse_move(
                origin + gpui::point(px(step as f32 * 5.), px(0.)),
                MouseButton::Left,
                gpui::Modifiers::default(),
            );
        }
        vcx.simulate_mouse_up(
            origin + gpui::point(px(40.), px(0.)),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let end_x = node_x(&harness, cx);
        assert!(
            (end_x - start_x - 40.0).abs() < 1.5,
            "dragging the X label 40px right should scrub X by ~40 (from {start_x} to {end_x})"
        );
    }

    #[gpui::test]
    async fn plain_click_on_a_numeric_field_focuses_the_editor(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let field_x = InspectorField::X(harness.vector_id);

        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let handle = vcx
            .debug_bounds("scrub-fanta-x-0")
            .expect("X field is laid out");
        // A click with no drag must open the inline editor, not scrub.
        vcx.simulate_click(handle.center(), gpui::Modifiers::default());
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let editing = harness
            .panel
            .read_with(cx, |panel, _| panel.editing_field.clone())
            .unwrap();
        assert_eq!(
            editing,
            Some(field_x),
            "a plain click should focus the field editor"
        );
    }

    #[gpui::test]
    async fn real_drag_of_a_gradient_stop_does_not_panic(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;
        let mut vcx = gpui::VisualTestContext::from_window(harness.panel.into(), cx);
        vcx.simulate_resize(gpui::size(px(360.), px(1600.)));
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        // Open the editor so its preview bar and stop markers are laid out.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, linear_gradient_fill(), cx);
            })
            .unwrap();
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();

        let marker = vcx
            .debug_bounds("fanta-gradient-stop-marker-0")
            .expect("the first gradient stop marker is laid out");
        let origin = marker.center();
        vcx.simulate_mouse_down(origin, MouseButton::Left, gpui::Modifiers::default());
        for step in 1..=8 {
            vcx.simulate_mouse_move(
                origin + gpui::point(px(step as f32 * 4.), px(0.)),
                MouseButton::Left,
                gpui::Modifiers::default(),
            );
            vcx.update(|window, cx| {
                window.draw(cx).clear();
            });
        }
        vcx.simulate_mouse_up(
            origin + gpui::point(px(32.), px(0.)),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        vcx.update(|window, cx| {
            window.draw(cx).clear();
        });
        vcx.run_until_parked();
    }

    #[gpui::test]
    async fn selection_change_during_a_live_gradient_preview_does_not_panic(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;

        // Open the editor and drive a change so the session is marked `changed`
        // and a preview snapshot is staged onto the item.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, linear_gradient_fill(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        harness
            .panel
            .update(cx, |panel, _, cx| {
                let editor = panel.gradient_editor.as_ref().unwrap().editor.clone();
                editor.update(cx, |editor, cx| {
                    editor.set_kind(crate::color_picker::GradientKind::Radial, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();

        // Now change the selection on the item. This emits `SelectionChanged`,
        // which the panel handles by resetting per-subject state — restoring the
        // in-flight preview snapshot via a fresh `item.update` from inside the
        // item's own event dispatch. A re-entrant update would panic here.
        let view = harness._view.clone();
        let item = view.read_with(cx, |view, _| view.item().clone());
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.clear();
                ((), DocChange::Selection)
            });
        });
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn text_node_draws_typography_decorations_and_glyph_fill(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        // The text node carries an off-ladder weight (350) and a Justify align:
        // both were previously unrepresentable in the inspector.
        harness.select(&[harness.text_id], cx);
        draw(harness.panel, cx);

        let text_id = harness.text_id;
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_text_decoration(text_id, TextDecorationGlyph::Underline, cx);
                panel.set_font_weight(text_id, 800, cx);
                panel.set_text_align(text_id, TextAlign::Justify, cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);

        let (weight, underline, align) = harness._view.read_with(cx, |view, cx| {
            let item = view.item().read(cx);
            let doc = &item.document().unwrap().doc;
            let NodeData::Text(text) = &doc.scene.get(text_id).unwrap().data else {
                panic!("expected a text node");
            };
            (text.style.weight, text.style.underline, text.align)
        });
        assert_eq!(weight, 800);
        assert!(underline);
        assert_eq!(align, TextAlign::Justify);
    }

    #[gpui::test]
    async fn blur_rows_add_edit_and_remove_through_set_blurs(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;

        let blurs = |cx: &mut TestAppContext| {
            harness._view.read_with(cx, |view, cx| {
                let item = view.item().read(cx);
                let doc = &item.document().unwrap().doc;
                doc.scene.get(vector_id).unwrap().blurs.to_vec()
            })
        };

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.add_blur(vector_id, BlurKind::Layer, cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        assert_eq!(blurs(cx).len(), 1);
        assert_eq!(blurs(cx)[0].kind, BlurKind::Layer);

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_blur_kind(vector_id, 0, BlurKind::Background, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(blurs(cx)[0].kind, BlurKind::Background);

        harness
            .panel
            .update(cx, |panel, _, cx| panel.remove_blur(vector_id, 0, cx))
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        assert!(blurs(cx).is_empty());

        // The whole add → edit → remove sequence stays undoable.
        let item = harness._view.read_with(cx, |view, _| view.item().clone());
        item.update(cx, |item, cx| item.undo(cx).unwrap());
        cx.run_until_parked();
        assert_eq!(blurs(cx).len(), 1);
    }

    #[gpui::test]
    async fn hiding_then_showing_a_paint_preserves_partial_alpha(cx: &mut TestAppContext) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let vector_id = harness.vector_id;

        // Flatten the gradient to a partially transparent solid.
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.set_paint_kind(vector_id, 0, false, PaintKind::Solid, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let item = harness._view.read_with(cx, |view, _| view.item().clone());
        item.update(cx, |item, cx| {
            item.apply(
                {
                    let doc = &item.document().unwrap().doc;
                    replace_data_operation(doc, vector_id, |data| {
                        if let Some(Fill::Solid { color, .. }) = fill_slot_mut(data, 0) {
                            color.a = 128;
                        }
                    })
                    .pop()
                    .expect("an alpha edit")
                },
                cx,
            )
            .expect("applying the alpha edit");
        });
        cx.run_until_parked();

        let alpha = |cx: &mut TestAppContext| {
            harness._view.read_with(cx, |view, cx| {
                let item = view.item().read(cx);
                let doc = &item.document().unwrap().doc;
                let NodeData::Vector(vector) = &doc.scene.get(vector_id).unwrap().data else {
                    panic!("expected a vector");
                };
                match vector.fills.first() {
                    Some(Fill::Solid { color, .. }) => color.a,
                    _ => panic!("expected a solid fill"),
                }
            })
        };
        assert_eq!(alpha(cx), 128);

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_paint_visibility(vector_id, 0, false, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(alpha(cx), 0, "hiding zeroes the paint's alpha");

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_paint_visibility(vector_id, 0, false, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            alpha(cx),
            128,
            "showing restores the alpha, not full opacity"
        );
    }

    #[gpui::test]
    async fn multi_selection_draws_selection_colors_and_replaces_across_the_selection(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        // The text node's black glyph color is the only solid in the selection.
        harness.select(&[harness.vector_id, harness.text_id], cx);
        draw(harness.panel, cx);

        let text_id = harness.text_id;
        let glyph_color = |cx: &mut TestAppContext| {
            harness._view.read_with(cx, |view, cx| {
                let item = view.item().read(cx);
                let doc = &item.document().unwrap().doc;
                let NodeData::Text(text) = &doc.scene.get(text_id).unwrap().data else {
                    panic!("expected a text node");
                };
                text.style.color
            })
        };
        let original = glyph_color(cx);
        let replacement = FantaColor::rgb(0x22, 0x88, 0x44);

        harness
            .panel
            .update(cx, |panel, _, cx| {
                let field = InspectorField::SelectionColor { from: original };
                let text = replacement.to_hex();
                panel.apply_document_ops(cx, |doc| field_operations(doc, &field, &text));
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
        assert_eq!(glyph_color(cx), replacement);
    }

    #[gpui::test]
    async fn degenerate_gradient_fill_draws_without_panicking(cx: &mut TestAppContext) {
        init_test(cx);
        // A single-stop gradient — the kind a real imported `.fig` can carry.
        let gradient = Gradient::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
            handles: None,
            stops: vec![GradientStop {
                position: 0.0,
                color: FantaColor::rgb(30, 60, 90),
            }],
        };
        let harness = open_panel_with_gradient(gradient.clone(), cx).await;
        let vector_id = harness.vector_id;
        draw(harness.panel, cx);
        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_gradient_editor(vector_id, 0, false, gradient.clone(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        draw(harness.panel, cx);
    }

    #[gpui::test]
    async fn enabling_auto_layout_on_a_clip_less_group_seeds_its_clip_box(cx: &mut TestAppContext) {
        init_test(cx);
        // The page root is a plain Group (no clip_size, no background) with two
        // children — exactly the clip-less group the review flagged.
        let harness = open_panel_with_gradient(linear_gradient_fill(), cx).await;
        let page_id = harness._view.read_with(cx, |view, cx| {
            view.item()
                .read(cx)
                .document()
                .expect("ready")
                .doc
                .pages()
                .first()
                .copied()
                .expect("one page")
        });
        harness.select(&[page_id], cx);

        // Precondition: a clip-less group.
        let before = harness._view.read_with(cx, |view, cx| {
            match &view
                .item()
                .read(cx)
                .document()
                .unwrap()
                .doc
                .scene
                .get(page_id)
                .unwrap()
                .data
            {
                NodeData::Group(group) => (group.auto_layout.is_some(), group.clip_size),
                _ => panic!("the page root is a group"),
            }
        });
        assert_eq!(
            before,
            (false, None),
            "starts as a clip-less group with no auto layout"
        );

        harness
            .panel
            .update(cx, |panel, _, cx| {
                panel.toggle_auto_layout(page_id, true, cx)
            })
            .unwrap();
        cx.run_until_parked();

        // Enabling auto layout must seed a clip box from the content bounds, or
        // a later Hug sizing would collapse it to zero and hide every child.
        let (has_auto_layout, clip_size) = harness._view.read_with(cx, |view, cx| {
            match &view
                .item()
                .read(cx)
                .document()
                .unwrap()
                .doc
                .scene
                .get(page_id)
                .unwrap()
                .data
            {
                NodeData::Group(group) => (group.auto_layout.is_some(), group.clip_size),
                _ => unreachable!(),
            }
        });
        assert!(has_auto_layout, "auto layout is enabled");
        let clip = clip_size.expect("clip_size is seeded when auto layout is enabled");
        assert!(
            clip[0] > 0.0 && clip[1] > 0.0,
            "the seeded clip box wraps the content instead of collapsing to zero: {clip:?}"
        );
    }
}

