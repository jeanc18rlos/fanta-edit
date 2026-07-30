use std::collections::{BTreeMap, BTreeSet};

use fanta_doc::{
    AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, BoundProp, Doc, Easing,
    Interpolation, Keyframe, KeyframeId, MotionProperty, MotionTarget, MotionTransform, NodeId,
    Operation, ResolvedVarValue, Transaction,
};
use fanta_ui::animation_panel::{AnimationDetailHeader, AnimationProperty, AnimationPropertyRow};
use fanta_ui::inspector::{InspectorEmptyState, InspectorFieldRow, InspectorSection};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, SharedString,
    Subscription, Window,
};
use ui::prelude::*;
use ui::{ContextMenu, ContextMenuEntry, DropdownMenu, DropdownStyle, IconPosition, Tooltip};
use util::ResultExt;

use crate::document::{DocChange, FigItem, FigItemEvent};

const DEFAULT_CLIP_DURATION_MS: u32 = 2_000;
const DEFAULT_ANIMATION_DURATION_MS: u32 = 500;
const DEFAULT_POSITION_DISTANCE: f64 = 200.0;
const POSITION_DISTANCES: [(u32, &str); 5] = [
    (50, "50"),
    (100, "100"),
    (200, "200"),
    (300, "300"),
    (400, "400"),
];
const SCALE_AMOUNTS: [(u32, &str); 5] = [
    (10, "10%"),
    (25, "25%"),
    (50, "50%"),
    (75, "75%"),
    (100, "100%"),
];
const ROTATION_ANGLES: [(u32, &str); 5] = [
    (15, "15°"),
    (30, "30°"),
    (45, "45°"),
    (90, "90°"),
    (180, "180°"),
];
const SIZE_AMOUNTS: [(u32, &str); 5] = [
    (10, "10%"),
    (25, "25%"),
    (50, "50%"),
    (75, "75%"),
    (100, "100%"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntranceDirection {
    Left,
    Right,
    Top,
    Bottom,
}

impl EntranceDirection {
    const ALL: [(Self, &'static str); 4] = [
        (Self::Left, "From left"),
        (Self::Right, "From right"),
        (Self::Top, "From top"),
        (Self::Bottom, "From bottom"),
    ];

    fn label(self) -> &'static str {
        Self::ALL
            .into_iter()
            .find_map(|(value, label)| (value == self).then_some(label))
            .unwrap_or("From left")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct AnimationSettings {
    direction: EntranceDirection,
    distance: f64,
    delay_ms: u32,
    duration_ms: u32,
    easing: Easing,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            direction: EntranceDirection::Left,
            distance: DEFAULT_POSITION_DISTANCE,
            delay_ms: 0,
            duration_ms: DEFAULT_ANIMATION_DURATION_MS,
            easing: Easing::EaseOut,
        }
    }
}

impl AnimationSettings {
    fn for_property(property: AnimationProperty) -> Self {
        Self {
            distance: match property {
                AnimationProperty::Position => DEFAULT_POSITION_DISTANCE,
                AnimationProperty::Scale => 25.0,
                AnimationProperty::Rotation => 90.0,
                AnimationProperty::Size => 50.0,
                AnimationProperty::Opacity | AnimationProperty::Path => 0.0,
            },
            ..Self::default()
        }
    }
}

#[derive(Clone)]
struct AnimationSummary {
    property: AnimationProperty,
    settings: Option<AnimationSettings>,
}

#[derive(Clone)]
struct MotionClipChoice {
    id: AnimationClipId,
    name: SharedString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionPanelEvent {
    SelectClip(AnimationClipId),
}

enum MotionPanelSnapshot {
    Message(SharedString),
    Selection {
        editable: bool,
        node: NodeId,
        node_name: SharedString,
        clip_name: Option<SharedString>,
        active_clip: Option<AnimationClipId>,
        clips: Vec<MotionClipChoice>,
        animations: Vec<AnimationSummary>,
        can_add_size: bool,
    },
}

pub struct FantaMotionPanel {
    item: Entity<FigItem>,
    focus_handle: FocusHandle,
    active_clip: Option<AnimationClipId>,
    selected_property: Option<AnimationProperty>,
    _item_subscription: Subscription,
}

impl FantaMotionPanel {
    pub fn new(item: Entity<FigItem>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&item, |this, _, event: &FigItemEvent, cx| {
            if matches!(event, FigItemEvent::StateChanged) {
                let selected_still_exists = this
                    .selected_property
                    .is_none_or(|property| this.snapshot(cx).has_property(property));
                if !selected_still_exists {
                    this.selected_property = None;
                }
            }
            cx.notify();
        });
        Self {
            item,
            focus_handle: cx.focus_handle(),
            active_clip: None,
            selected_property: None,
            _item_subscription: subscription,
        }
    }

    pub(crate) fn set_active_clip(
        &mut self,
        active_clip: Option<AnimationClipId>,
        cx: &mut Context<Self>,
    ) {
        if self.active_clip == active_clip {
            return;
        }
        self.active_clip = active_clip;
        self.selected_property = None;
        cx.notify();
    }

    fn snapshot(&self, cx: &App) -> MotionPanelSnapshot {
        let item = self.item.read(cx);
        if let Some(message) = item.document.loading_message() {
            return MotionPanelSnapshot::Message(message);
        }
        if let Some(error) = item.document.error() {
            return MotionPanelSnapshot::Message(error.to_string().into());
        }
        let Some(document) = item.document() else {
            return MotionPanelSnapshot::Message("The document is still loading".into());
        };
        let doc = &document.doc;
        let mut selection = doc.selection.iter().copied();
        let Some(node) = selection.next() else {
            return MotionPanelSnapshot::Message("Select a layer to animate".into());
        };
        if selection.next().is_some() {
            return MotionPanelSnapshot::Message("Select one layer to edit animations".into());
        }
        let Some(canvas_node) = doc.scene.get(node) else {
            return MotionPanelSnapshot::Message("The selected layer no longer exists".into());
        };
        let clip = resolved_clip(doc, self.active_clip);
        let animations = clip
            .map(|clip| animation_summaries(doc, clip, node))
            .unwrap_or_default();
        let clip_name = clip.map(|clip| clip.name.clone().into());
        let active_clip = clip.map(|clip| clip.id);
        let clips = clip_choices(doc);
        MotionPanelSnapshot::Selection {
            editable: item.is_editable(),
            node,
            node_name: if canvas_node.name.is_empty() {
                "Untitled layer".into()
            } else {
                canvas_node.name.clone().into()
            },
            clip_name,
            active_clip,
            clips,
            animations,
            can_add_size: bound_float(canvas_node, BoundProp::ClipWidth)
                .is_some_and(|width| width > f64::EPSILON)
                && bound_float(canvas_node, BoundProp::ClipHeight)
                    .is_some_and(|height| height > f64::EPSILON),
        }
    }

    fn add_animation(&mut self, node: NodeId, property: AnimationProperty, cx: &mut Context<Self>) {
        let active_clip = self.active_clip;
        let applied = self.apply_transaction(
            "Add Animation",
            |doc| {
                build_animation_transaction(
                    doc,
                    active_clip,
                    node,
                    property,
                    AnimationSettings::for_property(property),
                )
            },
            cx,
        );
        if applied {
            self.selected_property = Some(property);
            cx.notify();
        }
    }

    fn remove_animation(
        &mut self,
        node: NodeId,
        property: AnimationProperty,
        cx: &mut Context<Self>,
    ) {
        let active_clip = self.active_clip;
        let applied = self.apply_transaction(
            "Remove Animation",
            |doc| remove_animation_transaction(doc, active_clip, node, property),
            cx,
        );
        if applied && self.selected_property == Some(property) {
            self.selected_property = None;
            cx.notify();
        }
    }

    fn update_animation(
        &mut self,
        node: NodeId,
        property: AnimationProperty,
        update: impl FnOnce(&mut AnimationSettings),
        cx: &mut Context<Self>,
    ) {
        let active_clip = self.active_clip;
        self.apply_transaction(
            "Edit Animation",
            |doc| edit_animation_transaction(doc, active_clip, node, property, update),
            cx,
        );
    }

    fn apply_transaction(
        &mut self,
        label: &'static str,
        build: impl FnOnce(&Doc) -> Option<Transaction>,
        cx: &mut Context<Self>,
    ) -> bool {
        let transaction = {
            let item = self.item.read(cx);
            if !item.is_editable() {
                return false;
            }
            let Some(document) = item.document() else {
                return false;
            };
            build(&document.doc)
        };
        let Some(mut transaction) = transaction else {
            return false;
        };
        transaction.label = label.to_string();
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                if let Err(error) = document.doc.apply_transaction(transaction) {
                    log::error!("motion panel edit failed: {error:#}");
                    return (false, DocChange::None);
                }
                (true, DocChange::Content)
            })
            .unwrap_or(false)
        })
    }

    fn choice_dropdown<T: Copy + PartialEq + 'static>(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        current: T,
        options: &'static [(T, &'static str)],
        editable: bool,
        on_select: impl Fn(T, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.into();
        let label = label.into();
        if !editable {
            return Self::read_only_value(id, label);
        }
        let on_select = std::rc::Rc::new(on_select);
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (value, label) in options {
                let on_select = on_select.clone();
                let value = *value;
                menu.push_item(
                    ContextMenuEntry::new(*label)
                        .toggleable(IconPosition::End, current == value)
                        .handler(move |_, cx| on_select(value, cx)),
                );
            }
            menu
        });
        DropdownMenu::new(id, label, menu)
            .style(DropdownStyle::Outlined)
            .trigger_size(ButtonSize::Compact)
            .full_width(true)
            .into_any_element()
    }

    fn read_only_value(
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
    ) -> AnyElement {
        h_flex()
            .id(id)
            .h_7()
            .w_full()
            .px_2()
            .rounded_sm()
            .border_1()
            .border_color(gpui::transparent_black())
            .child(Label::new(label).size(LabelSize::Small))
            .into_any_element()
    }

    fn render_detail(
        &self,
        node: NodeId,
        summary: &AnimationSummary,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let property = summary.property;
        let close_panel = cx.weak_entity();
        let header = AnimationDetailHeader::new(property).on_close(move |_, cx| {
            close_panel
                .update(cx, |panel, cx| {
                    panel.selected_property = None;
                    cx.notify();
                })
                .log_err();
        });
        let Some(settings) = summary.settings else {
            return v_flex()
                .mt_2()
                .mx_2()
                .pb_3()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().surface_background)
                .child(header)
                .child(InspectorEmptyState::new(
                    "Custom keyframes",
                    "This animation has custom timing or values. Edit it in the timeline so no keyframes are replaced.",
                ))
                .into_any_element();
        };
        let type_label = match property {
            AnimationProperty::Position => "Slide in",
            AnimationProperty::Scale => "Scale in",
            AnimationProperty::Rotation => "Rotate in",
            AnimationProperty::Size => "Grow",
            AnimationProperty::Opacity => "Fade in",
            AnimationProperty::Path => "Path",
        };
        let mut detail = v_flex()
            .mt_2()
            .mx_2()
            .pb_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .child(header)
            .child(InspectorFieldRow::new(
                "Type",
                Self::read_only_value("fanta-animation-type", type_label),
            ));
        if property.supports_direction() {
            let panel = cx.weak_entity();
            detail = detail.child(InspectorFieldRow::new(
                "Direction",
                self.choice_dropdown(
                    "fanta-animation-direction",
                    settings.direction.label(),
                    settings.direction,
                    &EntranceDirection::ALL,
                    editable,
                    move |direction, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel.update_animation(
                                    node,
                                    property,
                                    |settings| settings.direction = direction,
                                    cx,
                                )
                            })
                            .log_err();
                    },
                    window,
                    cx,
                ),
            ));
        }
        if property.supports_distance() {
            let panel = cx.weak_entity();
            let distance = settings.distance.round().clamp(0.0, u32::MAX as f64) as u32;
            detail = detail.child(InspectorFieldRow::new(
                distance_field_label(property),
                self.choice_dropdown(
                    "fanta-animation-distance",
                    format_distance(property, settings.distance),
                    distance,
                    distance_choices(property),
                    editable,
                    move |distance, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel.update_animation(
                                    node,
                                    property,
                                    |settings| settings.distance = f64::from(distance),
                                    cx,
                                )
                            })
                            .log_err();
                    },
                    window,
                    cx,
                ),
            ));
        }
        const DELAYS: [(u32, &str); 6] = [
            (0, "0 ms"),
            (100, "100 ms"),
            (200, "200 ms"),
            (300, "300 ms"),
            (500, "500 ms"),
            (1_000, "1,000 ms"),
        ];
        const DURATIONS: [(u32, &str); 7] = [
            (100, "100 ms"),
            (200, "200 ms"),
            (300, "300 ms"),
            (500, "500 ms"),
            (800, "800 ms"),
            (1_000, "1,000 ms"),
            (2_000, "2,000 ms"),
        ];
        const EASINGS: [(Easing, &str); 4] = [
            (Easing::Linear, "Linear"),
            (Easing::EaseIn, "Ease in"),
            (Easing::EaseOut, "Ease out"),
            (Easing::EaseInOut, "Ease in and out"),
        ];
        let delay_panel = cx.weak_entity();
        detail = detail.child(div().h_2()).child(InspectorFieldRow::new(
            "Delay",
            self.choice_dropdown(
                "fanta-animation-delay",
                format!("{} ms", settings.delay_ms),
                settings.delay_ms,
                &DELAYS,
                editable,
                move |delay, cx| {
                    delay_panel
                        .update(cx, |panel, cx| {
                            panel.update_animation(
                                node,
                                property,
                                |settings| settings.delay_ms = delay,
                                cx,
                            )
                        })
                        .log_err();
                },
                window,
                cx,
            ),
        ));
        let duration_panel = cx.weak_entity();
        detail = detail.child(InspectorFieldRow::new(
            "Duration",
            self.choice_dropdown(
                "fanta-animation-duration",
                format!("{} ms", settings.duration_ms),
                settings.duration_ms,
                &DURATIONS,
                editable,
                move |duration, cx| {
                    duration_panel
                        .update(cx, |panel, cx| {
                            panel.update_animation(
                                node,
                                property,
                                |settings| settings.duration_ms = duration.max(1),
                                cx,
                            )
                        })
                        .log_err();
                },
                window,
                cx,
            ),
        ));
        let easing_panel = cx.weak_entity();
        detail = detail.child(InspectorFieldRow::new(
            "Easing",
            self.choice_dropdown(
                "fanta-animation-easing",
                easing_label(settings.easing),
                settings.easing,
                &EASINGS,
                editable,
                move |easing, cx| {
                    easing_panel
                        .update(cx, |panel, cx| {
                            panel.update_animation(
                                node,
                                property,
                                |settings| settings.easing = easing,
                                cx,
                            )
                        })
                        .log_err();
                },
                window,
                cx,
            ),
        ));
        detail.into_any_element()
    }
}

impl MotionPanelSnapshot {
    fn has_property(&self, property: AnimationProperty) -> bool {
        match self {
            Self::Selection { animations, .. } => animations
                .iter()
                .any(|animation| animation.property == property),
            Self::Message(_) => false,
        }
    }
}

impl Render for FantaMotionPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().panel_background);
        #[cfg(test)]
        let root = root.debug_selector(|| "fanta-motion-panel".to_owned());
        match self.snapshot(cx) {
            MotionPanelSnapshot::Message(message) => {
                root.child(InspectorEmptyState::new("Animations", message))
            }
            MotionPanelSnapshot::Selection {
                editable,
                node,
                node_name,
                clip_name,
                active_clip,
                clips,
                animations,
                can_add_size,
            } => {
                if self.selected_property.is_some_and(|property| {
                    !animations.iter().any(|item| item.property == property)
                }) {
                    self.selected_property = None;
                }
                let existing: BTreeSet<_> = animations
                    .iter()
                    .map(|animation| animation.property)
                    .collect();
                let add_button = editable.then(|| {
                    let panel = cx.weak_entity();
                    let add_menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
                        for property in AnimationProperty::ALL {
                            let panel = panel.clone();
                            let disabled = property == AnimationProperty::Path
                                || property == AnimationProperty::Size && !can_add_size
                                || existing.contains(&property);
                            let mut label = property.label().to_string();
                            if property == AnimationProperty::Path {
                                label.push_str(" · soon");
                            }
                            menu.push_item(
                                ContextMenuEntry::new(label)
                                    .icon(property.icon())
                                    .disabled(disabled)
                                    .handler(move |_, cx| {
                                        panel
                                            .update(cx, |panel, cx| {
                                                panel.add_animation(node, property, cx)
                                            })
                                            .log_err();
                                    }),
                            );
                        }
                        menu
                    });
                    DropdownMenu::new_with_element(
                        "fanta-motion-add-animation",
                        Icon::new(IconName::Plus)
                            .size(IconSize::Small)
                            .into_any_element(),
                        add_menu,
                    )
                    .no_chevron()
                    .style(DropdownStyle::Ghost)
                    .trigger_tooltip(Tooltip::text("Add animation"))
                });
                let mut animation_section = InspectorSection::new(
                    "fanta-motion-animations",
                    "Animations",
                )
                .children(animations.iter().map(|animation| {
                    let select_panel = cx.weak_entity();
                    let remove_panel = cx.weak_entity();
                    let property = animation.property;
                    AnimationPropertyRow::new(
                        format!("fanta-motion-animation-{}", property.label()),
                        property,
                    )
                    .subtitle(animation_summary(animation))
                    .selected(self.selected_property == Some(property))
                    .removable(editable)
                    .on_select(move |_, cx| {
                        select_panel
                            .update(cx, |panel, cx| {
                                panel.selected_property = Some(property);
                                cx.notify();
                            })
                            .log_err();
                    })
                    .on_remove(move |_, cx| {
                        remove_panel
                            .update(cx, |panel, cx| panel.remove_animation(node, property, cx))
                            .log_err();
                    })
                }));
                if let Some(add_button) = add_button {
                    animation_section = animation_section.action(add_button);
                }
                let clip_control = if clips.len() > 1 {
                    let panel = cx.weak_entity();
                    let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
                        for clip in clips {
                            let panel = panel.clone();
                            menu.push_item(
                                ContextMenuEntry::new(clip.name)
                                    .toggleable(IconPosition::End, active_clip == Some(clip.id))
                                    .handler(move |_, cx| {
                                        panel
                                            .update(cx, |_, cx| {
                                                cx.emit(MotionPanelEvent::SelectClip(clip.id));
                                            })
                                            .log_err();
                                    }),
                            );
                        }
                        menu
                    });
                    DropdownMenu::new(
                        "fanta-motion-clip-selector",
                        clip_name.unwrap_or_else(|| "Select animation clip".into()),
                        menu,
                    )
                    .style(DropdownStyle::Outlined)
                    .trigger_size(ButtonSize::Compact)
                    .full_width(true)
                    .into_any_element()
                } else {
                    Label::new(clip_name.unwrap_or_else(|| "No animation clip yet".into()))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .into_any_element()
                };
                let mut content = v_flex()
                    .id("fanta-motion-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .px_3()
                            .py_2()
                            .gap_1()
                            .child(Label::new(node_name).single_line())
                            .child(clip_control),
                    )
                    .child(animation_section);
                if animations.is_empty() {
                    content = content.child(InspectorEmptyState::new(
                        "Bring the selection to life",
                        "Use + to add Position, Scale, Rotation, Size, or Opacity. Each animation creates readable start and end keyframes.",
                    ));
                }
                if let Some(property) = self.selected_property
                    && let Some(summary) = animations.iter().find(|item| item.property == property)
                {
                    content =
                        content.child(self.render_detail(node, summary, editable, window, cx));
                }
                root.child(content)
            }
        }
    }
}

impl EventEmitter<MotionPanelEvent> for FantaMotionPanel {}

impl Focusable for FantaMotionPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn first_clip(doc: &Doc) -> Option<&AnimationClip> {
    doc.motion.clips.values().next()
}

fn clip_choices(doc: &Doc) -> Vec<MotionClipChoice> {
    doc.motion
        .clips
        .values()
        .map(|clip| MotionClipChoice {
            id: clip.id,
            name: clip.name.clone().into(),
        })
        .collect()
}

fn resolved_clip(doc: &Doc, active_clip: Option<AnimationClipId>) -> Option<&AnimationClip> {
    active_clip
        .and_then(|clip| doc.motion.clips.get(&clip))
        .or_else(|| first_clip(doc))
}

fn animation_summaries(doc: &Doc, clip: &AnimationClip, node: NodeId) -> Vec<AnimationSummary> {
    AnimationProperty::ALL
        .into_iter()
        .filter(|property| *property != AnimationProperty::Path)
        .filter(|property| {
            clip.tracks.values().any(|track| {
                track.target.node == node
                    && property_for_target(track.target.property) == Some(*property)
            })
        })
        .map(|property| AnimationSummary {
            property,
            settings: infer_settings(doc, clip, node, property),
        })
        .collect()
}

fn infer_settings(
    doc: &Doc,
    clip: &AnimationClip,
    node: NodeId,
    property: AnimationProperty,
) -> Option<AnimationSettings> {
    let tracks: Vec<_> = clip
        .tracks
        .values()
        .filter(|track| {
            track.target.node == node
                && property_for_target(track.target.property) == Some(property)
        })
        .collect();
    if tracks.is_empty() {
        return None;
    }
    if tracks.iter().any(|track| track.keyframes.len() != 2) {
        return None;
    }
    let primary = *tracks.first()?;
    let (first, last) = track_endpoints(primary)?;
    if first.time_ms >= last.time_ms
        || first.interpolation != Interpolation::Linear
        || last.interpolation != Interpolation::Linear
    {
        return None;
    }
    for track in tracks.iter().skip(1) {
        let (candidate_first, candidate_last) = track_endpoints(track)?;
        if candidate_first.time_ms != first.time_ms
            || candidate_last.time_ms != last.time_ms
            || candidate_first.easing != first.easing
            || candidate_last.easing != last.easing
            || candidate_first.interpolation != Interpolation::Linear
            || candidate_last.interpolation != Interpolation::Linear
        {
            return None;
        }
    }
    let mut settings = AnimationSettings {
        delay_ms: first.time_ms,
        duration_ms: last.time_ms.saturating_sub(first.time_ms),
        easing: first.easing,
        ..AnimationSettings::for_property(property)
    };
    let (Some(start), Some(end)) = (float_value(&first.value), float_value(&last.value)) else {
        return None;
    };
    let delta = start - end;
    match primary.target.property {
        MotionProperty::PositionX => {
            settings.direction = if delta <= 0.0 {
                EntranceDirection::Left
            } else {
                EntranceDirection::Right
            };
            settings.distance = delta.abs();
        }
        MotionProperty::PositionY => {
            settings.direction = if delta <= 0.0 {
                EntranceDirection::Top
            } else {
                EntranceDirection::Bottom
            };
            settings.distance = delta.abs();
        }
        MotionProperty::Rotation => settings.distance = snap_near_integer(delta.abs().to_degrees()),
        MotionProperty::ScaleX | MotionProperty::ScaleY => {
            if end.abs() <= f64::EPSILON {
                return None;
            }
            settings.distance = snap_near_integer((end - start).abs() / end.abs() * 100.0)
        }
        MotionProperty::Bound {
            prop: BoundProp::ClipWidth | BoundProp::ClipHeight,
        } => {
            if end.abs() <= f64::EPSILON {
                return None;
            }
            settings.distance = snap_near_integer((end - start).abs() / end.abs() * 100.0)
        }
        MotionProperty::Bound { .. } => {}
    }
    let expected = desired_values(doc, node, property, settings)?;
    if expected.len() != tracks.len() {
        return None;
    }
    for track in tracks {
        let (first, last) = track_endpoints(track)?;
        let (actual_start, actual_end) = (float_value(&first.value)?, float_value(&last.value)?);
        let (expected_start, expected_end) = expected.get(&track.target)?;
        if !approximately_equal(actual_start, *expected_start)
            || !approximately_equal(actual_end, *expected_end)
        {
            return None;
        }
    }
    Some(settings)
}

fn approximately_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= scale * 1e-9
}

fn snap_near_integer(value: f64) -> f64 {
    let rounded = value.round();
    if approximately_equal(value, rounded) {
        rounded
    } else {
        value
    }
}

fn edit_animation_transaction(
    doc: &Doc,
    active_clip: Option<AnimationClipId>,
    node: NodeId,
    property: AnimationProperty,
    update: impl FnOnce(&mut AnimationSettings),
) -> Option<Transaction> {
    let clip = resolved_clip(doc, active_clip)?;
    let mut settings = infer_settings(doc, clip, node, property)?;
    update(&mut settings);
    build_animation_transaction(doc, Some(clip.id), node, property, settings)
}

fn track_endpoints(track: &AnimationTrack) -> Option<(&Keyframe, &Keyframe)> {
    let mut keyframes: Vec<_> = track.keyframes.values().collect();
    keyframes.sort_by_key(|keyframe| (keyframe.time_ms, keyframe.id));
    Some((*keyframes.first()?, *keyframes.last()?))
}

fn property_for_target(property: MotionProperty) -> Option<AnimationProperty> {
    match property {
        MotionProperty::PositionX | MotionProperty::PositionY => Some(AnimationProperty::Position),
        MotionProperty::ScaleX | MotionProperty::ScaleY => Some(AnimationProperty::Scale),
        MotionProperty::Rotation => Some(AnimationProperty::Rotation),
        MotionProperty::Bound {
            prop: BoundProp::ClipWidth | BoundProp::ClipHeight,
        } => Some(AnimationProperty::Size),
        MotionProperty::Bound {
            prop: BoundProp::Opacity,
        } => Some(AnimationProperty::Opacity),
        MotionProperty::Bound { .. } => None,
    }
}

fn desired_values(
    doc: &Doc,
    node: NodeId,
    property: AnimationProperty,
    settings: AnimationSettings,
) -> Option<BTreeMap<MotionTarget, (f64, f64)>> {
    let canvas_node = doc.scene.get(node)?;
    let transform = MotionTransform::decompose(canvas_node.transform)?;
    let mut values = BTreeMap::new();
    match property {
        AnimationProperty::Position => {
            if !settings.distance.is_finite() || settings.distance < 0.0 {
                return None;
            }
            let (property, start, end) = match settings.direction {
                EntranceDirection::Left => (
                    MotionProperty::PositionX,
                    transform.position[0] - settings.distance,
                    transform.position[0],
                ),
                EntranceDirection::Right => (
                    MotionProperty::PositionX,
                    transform.position[0] + settings.distance,
                    transform.position[0],
                ),
                EntranceDirection::Top => (
                    MotionProperty::PositionY,
                    transform.position[1] - settings.distance,
                    transform.position[1],
                ),
                EntranceDirection::Bottom => (
                    MotionProperty::PositionY,
                    transform.position[1] + settings.distance,
                    transform.position[1],
                ),
            };
            values.insert(MotionTarget::new(node, property), (start, end));
        }
        AnimationProperty::Scale => {
            let amount = percentage_amount(settings.distance)?;
            if transform.scale[0].abs() <= f64::EPSILON || transform.scale[1].abs() <= f64::EPSILON
            {
                return None;
            }
            values.insert(
                MotionTarget::new(node, MotionProperty::ScaleX),
                (transform.scale[0] * (1.0 - amount), transform.scale[0]),
            );
            values.insert(
                MotionTarget::new(node, MotionProperty::ScaleY),
                (transform.scale[1] * (1.0 - amount), transform.scale[1]),
            );
        }
        AnimationProperty::Rotation => {
            if !settings.distance.is_finite() || settings.distance < 0.0 {
                return None;
            }
            values.insert(
                MotionTarget::new(node, MotionProperty::Rotation),
                (
                    transform.rotation_radians - settings.distance.to_radians(),
                    transform.rotation_radians,
                ),
            );
        }
        AnimationProperty::Size => {
            let width = bound_float(canvas_node, BoundProp::ClipWidth)?;
            let height = bound_float(canvas_node, BoundProp::ClipHeight)?;
            if width <= f64::EPSILON || height <= f64::EPSILON {
                return None;
            }
            let amount = percentage_amount(settings.distance)?;
            values.insert(
                MotionTarget::new(node, MotionProperty::bound(BoundProp::ClipWidth)),
                (width * (1.0 - amount), width),
            );
            values.insert(
                MotionTarget::new(node, MotionProperty::bound(BoundProp::ClipHeight)),
                (height * (1.0 - amount), height),
            );
        }
        AnimationProperty::Opacity => {
            let opacity = bound_float(canvas_node, BoundProp::Opacity)?;
            values.insert(
                MotionTarget::new(node, MotionProperty::bound(BoundProp::Opacity)),
                (0.0, opacity),
            );
        }
        AnimationProperty::Path => return None,
    }
    Some(values)
}

fn percentage_amount(value: f64) -> Option<f64> {
    (value.is_finite() && (0.0..=100.0).contains(&value)).then_some(value / 100.0)
}

fn build_animation_transaction(
    doc: &Doc,
    active_clip: Option<AnimationClipId>,
    node: NodeId,
    property: AnimationProperty,
    settings: AnimationSettings,
) -> Option<Transaction> {
    let values = desired_values(doc, node, property, settings)?;
    let clip = resolved_clip(doc, active_clip);
    let clip_id = clip
        .map(|clip| clip.id)
        .unwrap_or_else(AnimationClipId::new);
    let mut transaction = Transaction::new("Edit Animation");
    if clip.is_none() {
        transaction.push(Operation::CreateAnimationClip {
            clip: Box::new(AnimationClip::new(
                clip_id,
                "Animation 1",
                DEFAULT_CLIP_DURATION_MS,
            )),
        });
    }
    let end_ms = settings
        .delay_ms
        .saturating_add(settings.duration_ms.max(1));
    if let Some(clip) = clip
        && end_ms > clip.duration_ms
    {
        transaction.push(Operation::SetAnimationClipDuration {
            id: clip.id,
            old: clip.duration_ms,
            new: end_ms,
        });
    }
    let desired_targets: BTreeSet<_> = values.keys().copied().collect();
    if let Some(clip) = clip {
        for track in clip.tracks.values().filter(|track| {
            track.target.node == node
                && property_for_target(track.target.property) == Some(property)
                && !desired_targets.contains(&track.target)
        }) {
            transaction.push(Operation::SetAnimationTrack {
                clip: clip_id,
                track: track.id,
                old: Some(Box::new(track.clone())),
                new: None,
            });
        }
    }
    for (target, (start, end)) in values {
        let existing = clip.and_then(|clip| clip.track_for_target(target));
        let track_id = existing
            .map(|track| track.id)
            .unwrap_or_else(AnimationTrackId::new);
        let mut keyframe_ids = existing
            .map(|track| {
                let mut keyframes: Vec<_> = track.keyframes.values().collect();
                keyframes.sort_by_key(|keyframe| (keyframe.time_ms, keyframe.id));
                (
                    keyframes.first().map(|keyframe| keyframe.id),
                    keyframes.last().map(|keyframe| keyframe.id),
                )
            })
            .unwrap_or((None, None));
        if keyframe_ids.1 == keyframe_ids.0 {
            keyframe_ids.1 = None;
        }
        let start_id = keyframe_ids.0.unwrap_or_else(KeyframeId::new);
        let end_id = keyframe_ids.1.unwrap_or_else(KeyframeId::new);
        let mut track = AnimationTrack::new(track_id, target);
        track.keyframes.insert(
            start_id,
            Keyframe {
                id: start_id,
                time_ms: settings.delay_ms,
                value: ResolvedVarValue::Float { value: start },
                interpolation: Interpolation::Linear,
                easing: settings.easing,
            },
        );
        track.keyframes.insert(
            end_id,
            Keyframe {
                id: end_id,
                time_ms: end_ms,
                value: ResolvedVarValue::Float { value: end },
                interpolation: Interpolation::Linear,
                easing: settings.easing,
            },
        );
        transaction.push(Operation::SetAnimationTrack {
            clip: clip_id,
            track: track_id,
            old: existing.cloned().map(Box::new),
            new: Some(Box::new(track)),
        });
    }
    (!transaction.is_empty()).then_some(transaction)
}

fn remove_animation_transaction(
    doc: &Doc,
    active_clip: Option<AnimationClipId>,
    node: NodeId,
    property: AnimationProperty,
) -> Option<Transaction> {
    let clip = resolved_clip(doc, active_clip)?;
    let tracks: Vec<_> = clip
        .tracks
        .values()
        .filter(|track| {
            track.target.node == node
                && property_for_target(track.target.property) == Some(property)
        })
        .cloned()
        .collect();
    if tracks.is_empty() {
        return None;
    }
    let mut transaction = Transaction::new("Remove Animation");
    for track in tracks {
        transaction.push(Operation::SetAnimationTrack {
            clip: clip.id,
            track: track.id,
            old: Some(Box::new(track)),
            new: None,
        });
    }
    Some(transaction)
}

fn bound_float(node: &fanta_doc::CanvasNode, property: BoundProp) -> Option<f64> {
    float_value(&property.read_resolved(node)?)
}

fn float_value(value: &ResolvedVarValue) -> Option<f64> {
    match value {
        ResolvedVarValue::Float { value } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn animation_summary(animation: &AnimationSummary) -> SharedString {
    let Some(settings) = animation.settings else {
        return "Custom · edit in timeline".into();
    };
    if animation.property == AnimationProperty::Position {
        return format!(
            "Slide in · {} · {} ms",
            settings.direction.label(),
            settings.duration_ms
        )
        .into();
    }
    format!(
        "{} · {} ms",
        match animation.property {
            AnimationProperty::Position => "Slide in",
            AnimationProperty::Scale => "Scale in",
            AnimationProperty::Rotation => "Rotate in",
            AnimationProperty::Size => "Grow",
            AnimationProperty::Opacity => "Fade in",
            AnimationProperty::Path => "Path",
        },
        settings.duration_ms
    )
    .into()
}

fn format_distance(property: AnimationProperty, distance: f64) -> SharedString {
    if matches!(property, AnimationProperty::Scale | AnimationProperty::Size) {
        format!("{distance:.0}%").into()
    } else if matches!(property, AnimationProperty::Rotation) {
        format!("{distance:.0}°").into()
    } else {
        format!("{distance:.0}").into()
    }
}

fn distance_field_label(property: AnimationProperty) -> &'static str {
    match property {
        AnimationProperty::Position => "Distance",
        AnimationProperty::Scale | AnimationProperty::Size => "Amount",
        AnimationProperty::Rotation => "Angle",
        AnimationProperty::Opacity | AnimationProperty::Path => "Distance",
    }
}

fn distance_choices(property: AnimationProperty) -> &'static [(u32, &'static str)] {
    match property {
        AnimationProperty::Position => &POSITION_DISTANCES,
        AnimationProperty::Scale => &SCALE_AMOUNTS,
        AnimationProperty::Rotation => &ROTATION_ANGLES,
        AnimationProperty::Size => &SIZE_AMOUNTS,
        AnimationProperty::Opacity | AnimationProperty::Path => &[],
    }
}

fn easing_label(easing: Easing) -> &'static str {
    match easing {
        Easing::Linear => "Linear",
        Easing::EaseIn => "Ease in",
        Easing::EaseOut => "Ease out",
        Easing::EaseInOut => "Ease in and out",
        Easing::CubicBezier { .. } => "Custom curve",
        Easing::Spring { .. } => "Spring",
    }
}

#[cfg(test)]
mod tests {
    use fanta_doc::{CanvasNode, GroupNode, NodeData, Operation, Transform2D};

    use super::*;

    fn document_with_selected_node() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        let mut animated_group = GroupNode::default();
        animated_group.clip_size = Some([80.0, 40.0]);
        let mut node = CanvasNode::new(NodeData::Group(animated_group));
        node.parent = Some(page_id);
        node.transform = Transform2D::translation(300.0, 120.0);
        let node_id = node.id;
        doc.apply(Operation::create_node(node)).unwrap();
        doc.selection.select_only(node_id);
        doc.history = Default::default();
        (doc, node_id)
    }

    fn create_clip(doc: &mut Doc, name: &str) -> AnimationClipId {
        let clip = AnimationClip::new(AnimationClipId::new(), name, DEFAULT_CLIP_DURATION_MS);
        let id = clip.id;
        doc.apply(Operation::CreateAnimationClip {
            clip: Box::new(clip),
        })
        .expect("create animation clip");
        id
    }

    fn assert_preset_round_trip(
        property: AnimationProperty,
        direction: EntranceDirection,
        distance: u32,
    ) {
        let (mut doc, node) = document_with_selected_node();
        let settings = AnimationSettings {
            direction,
            distance: f64::from(distance),
            delay_ms: 123,
            duration_ms: 789,
            easing: Easing::EaseInOut,
        };
        let transaction = build_animation_transaction(&doc, None, node, property, settings)
            .unwrap_or_else(|| panic!("{property:?} {distance} should author a preset"));
        doc.apply_transaction(transaction)
            .expect("apply animation preset");
        assert_eq!(
            infer_settings(
                &doc,
                first_clip(&doc).expect("created clip"),
                node,
                property,
            ),
            Some(settings),
            "{property:?} {distance} must round-trip without clamping or normalization",
        );
    }

    #[test]
    fn every_offered_property_amount_round_trips_exactly() {
        for (distance, _) in distance_choices(AnimationProperty::Position) {
            for (direction, _) in EntranceDirection::ALL {
                assert_preset_round_trip(AnimationProperty::Position, direction, *distance);
            }
        }
        for property in [
            AnimationProperty::Scale,
            AnimationProperty::Rotation,
            AnimationProperty::Size,
        ] {
            for (distance, _) in distance_choices(property) {
                assert_preset_round_trip(property, EntranceDirection::Left, *distance);
            }
        }
    }

    #[test]
    fn scale_rejects_amounts_that_would_collapse_to_the_same_keyframes() {
        let (doc, node) = document_with_selected_node();
        for distance in [101.0, 200.0, 300.0, 400.0] {
            let settings = AnimationSettings {
                distance,
                ..AnimationSettings::for_property(AnimationProperty::Scale)
            };
            assert!(
                build_animation_transaction(&doc, None, node, AnimationProperty::Scale, settings,)
                    .is_none(),
                "scale must reject {distance}% instead of clamping it to 100%",
            );
        }
        assert_eq!(
            distance_choices(AnimationProperty::Scale),
            &[
                (10, "10%"),
                (25, "25%"),
                (50, "50%"),
                (75, "75%"),
                (100, "100%")
            ],
        );
    }

    #[test]
    fn active_clip_resolution_is_exact_with_a_deterministic_fallback() {
        let (mut doc, _) = document_with_selected_node();
        let first = create_clip(&mut doc, "First");
        let second = create_clip(&mut doc, "Second");

        assert_eq!(
            clip_choices(&doc)
                .into_iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            doc.motion.clips.keys().copied().collect::<Vec<_>>(),
            "the selector must use the document's stable clip order",
        );

        assert_eq!(
            resolved_clip(&doc, Some(first)).map(|clip| clip.id),
            Some(first)
        );
        assert_eq!(
            resolved_clip(&doc, Some(second)).map(|clip| clip.id),
            Some(second)
        );
        assert_eq!(
            resolved_clip(&doc, Some(AnimationClipId::new())).map(|clip| clip.id),
            first_clip(&doc).map(|clip| clip.id),
        );
    }

    #[test]
    fn animation_edits_and_removal_only_touch_the_active_clip() {
        let (mut doc, node) = document_with_selected_node();
        let first = create_clip(&mut doc, "First");
        let second = create_clip(&mut doc, "Second");
        let first_settings = AnimationSettings {
            distance: 50.0,
            ..AnimationSettings::default()
        };
        let second_settings = AnimationSettings {
            distance: 300.0,
            ..AnimationSettings::default()
        };
        doc.apply_transaction(
            build_animation_transaction(
                &doc,
                Some(first),
                node,
                AnimationProperty::Position,
                first_settings,
            )
            .expect("first clip animation"),
        )
        .expect("apply first clip animation");
        doc.apply_transaction(
            build_animation_transaction(
                &doc,
                Some(second),
                node,
                AnimationProperty::Position,
                second_settings,
            )
            .expect("second clip animation"),
        )
        .expect("apply second clip animation");
        let first_before = doc.motion.clips[&first].clone();

        doc.apply_transaction(
            edit_animation_transaction(
                &doc,
                Some(second),
                node,
                AnimationProperty::Position,
                |settings| settings.duration_ms = 800,
            )
            .expect("edit second clip animation"),
        )
        .expect("apply second clip edit");
        assert_eq!(doc.motion.clips[&first], first_before);
        assert_eq!(
            infer_settings(
                &doc,
                &doc.motion.clips[&second],
                node,
                AnimationProperty::Position,
            )
            .expect("second preset")
            .duration_ms,
            800,
        );

        doc.apply_transaction(
            remove_animation_transaction(&doc, Some(second), node, AnimationProperty::Position)
                .expect("remove from second clip"),
        )
        .expect("apply second clip removal");
        assert_eq!(doc.motion.clips[&first], first_before);
        assert!(doc.motion.clips[&second].tracks.is_empty());
    }

    #[test]
    fn adding_position_authors_a_readable_two_keyframe_entrance() {
        let (mut doc, node) = document_with_selected_node();
        let settings = AnimationSettings::default();
        let transaction =
            build_animation_transaction(&doc, None, node, AnimationProperty::Position, settings)
                .expect("position animation");
        doc.apply_transaction(transaction).unwrap();
        let clip = first_clip(&doc).unwrap();
        assert_eq!(clip.duration_ms, DEFAULT_CLIP_DURATION_MS);
        let track = clip
            .track_for_target(MotionTarget::new(node, MotionProperty::PositionX))
            .expect("x track");
        let (first, last) = track_endpoints(track).unwrap();
        assert_eq!(first.time_ms, 0);
        assert_eq!(last.time_ms, DEFAULT_ANIMATION_DURATION_MS);
        assert_eq!(float_value(&first.value), Some(100.0));
        assert_eq!(float_value(&last.value), Some(300.0));
        assert_eq!(
            infer_settings(&doc, clip, node, AnimationProperty::Position),
            Some(settings)
        );
    }

    #[test]
    fn editing_direction_reuses_track_and_endpoint_ids_when_axis_is_stable() {
        let (mut doc, node) = document_with_selected_node();
        let settings = AnimationSettings::default();
        doc.apply_transaction(
            build_animation_transaction(&doc, None, node, AnimationProperty::Position, settings)
                .unwrap(),
        )
        .unwrap();
        let clip = first_clip(&doc).unwrap();
        let original = clip
            .track_for_target(MotionTarget::new(node, MotionProperty::PositionX))
            .unwrap()
            .clone();
        let mut next = settings;
        next.direction = EntranceDirection::Right;
        next.delay_ms = 200;
        doc.apply_transaction(
            build_animation_transaction(&doc, None, node, AnimationProperty::Position, next)
                .unwrap(),
        )
        .unwrap();
        let edited = first_clip(&doc)
            .unwrap()
            .track_for_target(MotionTarget::new(node, MotionProperty::PositionX))
            .unwrap();
        assert_eq!(edited.id, original.id);
        assert_eq!(
            edited.keyframes.keys().collect::<Vec<_>>(),
            original.keyframes.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            infer_settings(
                &doc,
                first_clip(&doc).unwrap(),
                node,
                AnimationProperty::Position,
            ),
            Some(next)
        );
    }

    #[test]
    fn custom_multi_keyframe_track_is_visible_but_never_rebuilt_as_a_preset() {
        let (mut doc, node) = document_with_selected_node();
        doc.apply_transaction(
            build_animation_transaction(
                &doc,
                None,
                node,
                AnimationProperty::Position,
                AnimationSettings::default(),
            )
            .expect("position animation"),
        )
        .expect("apply position animation");
        let clip_id = first_clip(&doc).expect("clip").id;
        let target = MotionTarget::new(node, MotionProperty::PositionX);
        let track_id = doc.motion.clips[&clip_id]
            .track_for_target(target)
            .expect("position track")
            .id;
        let custom_keyframe = KeyframeId::new();
        doc.motion
            .clips
            .get_mut(&clip_id)
            .and_then(|clip| clip.tracks.get_mut(&track_id))
            .expect("position track remains")
            .keyframes
            .insert(
                custom_keyframe,
                Keyframe {
                    id: custom_keyframe,
                    time_ms: 250,
                    value: ResolvedVarValue::Float { value: 175.0 },
                    interpolation: Interpolation::Linear,
                    easing: Easing::EaseInOut,
                },
            );
        let before = doc.motion.clips[&clip_id].tracks[&track_id].clone();

        let summaries = animation_summaries(&doc, &doc.motion.clips[&clip_id], node);
        let position = summaries
            .iter()
            .find(|summary| summary.property == AnimationProperty::Position)
            .expect("custom position remains listed");
        assert!(position.settings.is_none());
        assert_eq!(animation_summary(position), "Custom · edit in timeline");
        assert!(
            edit_animation_transaction(&doc, None, node, AnimationProperty::Position, |settings| {
                settings.duration_ms = 800
            },)
            .is_none(),
            "the preset inspector must not synthesize a replacement transaction"
        );
        assert_eq!(doc.motion.clips[&clip_id].tracks[&track_id], before);
    }

    #[test]
    fn removing_grouped_scale_tracks_is_one_undoable_transaction() {
        let (mut doc, node) = document_with_selected_node();
        let settings = AnimationSettings {
            distance: 20.0,
            ..Default::default()
        };
        doc.apply_transaction(
            build_animation_transaction(&doc, None, node, AnimationProperty::Scale, settings)
                .unwrap(),
        )
        .unwrap();
        doc.history = Default::default();
        let remove =
            remove_animation_transaction(&doc, None, node, AnimationProperty::Scale).unwrap();
        assert_eq!(remove.ops.len(), 2);
        doc.apply_transaction(remove).unwrap();
        assert!(animation_summaries(&doc, first_clip(&doc).unwrap(), node).is_empty());
        assert!(doc.undo().unwrap());
        assert_eq!(
            animation_summaries(&doc, first_clip(&doc).unwrap(), node)
                .first()
                .expect("restored scale animation")
                .property,
            AnimationProperty::Scale
        );
    }
}
