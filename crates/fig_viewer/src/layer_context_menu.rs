use super::*;
use crate::layer_context_ops as ops;
use anyhow::{Context as _, ensure};
use fanta_doc::{DocId, LayoutMode};
use fanta_gpui::atoms::ControlExt as _;
use fanta_gpui::layers::LayersPanelContextAction as Action;
use gpui::{
    ClipboardEntry, ClipboardItem, KeyDownEvent, MouseDownEvent, Point, ScrollHandle, anchored,
    deferred, px,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
enum Choice {
    Page(NodeId),
    Layout(Option<LayoutMode>),
    Crop(Option<f64>),
    CopySvg,
    CopyPng,
    CopyProperties,
    PasteProperties,
    CustomCrop,
    ApplyCrop,
    CropRectangle([f64; 4]),
    ToggleWrap,
}

pub(super) struct ContextPicker {
    document: DocId,
    node: NodeId,
    anchor: Point<Pixels>,
    choices: Vec<(String, Choice)>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    crop_inputs: Vec<Entity<gpui_component::input::InputState>>,
}

#[derive(Serialize, Deserialize)]
struct StyleClipboard {
    document: DocId,
    node: CanvasNode,
}

impl FantaDesignPanel {
    pub(super) fn apply_layer_command(
        &mut self,
        label: &str,
        build: impl FnOnce(&Doc) -> Result<Vec<Operation>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_layer_command_select(
            label,
            |doc| build(doc).map(|operations| (operations, None)),
            window,
            cx,
        );
    }

    pub(super) fn apply_layer_command_select(
        &mut self,
        label: &str,
        build: impl FnOnce(&Doc) -> Result<(Vec<Operation>, Option<Vec<NodeId>>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx)
        });
        let item = view.read(cx).item().clone();
        let result = item.update(cx, |item, cx| {
            if !item.is_editable() {
                return Some(Err(anyhow::anyhow!("This document is read-only")));
            }
            item.with_document(cx, |document| {
                let result = build(&document.doc).and_then(|(operations, selection)| {
                    let changed =
                        crate::clipboard::apply_transaction(&mut document.doc, label, operations)?;
                    if changed {
                        let selection = selection.unwrap_or_else(|| {
                            document
                                .doc
                                .selection
                                .iter()
                                .copied()
                                .filter(|id| document.doc.scene.contains(*id))
                                .collect()
                        });
                        document.doc.selection.replace_with(selection);
                    }
                    Ok(changed)
                });
                let change = if matches!(result, Ok(true)) {
                    DocChange::Content
                } else {
                    DocChange::None
                };
                (result, change)
            })
        });
        if let Some(Err(error)) = result {
            crate::view::show_canvas_notice(format!("{label}: {error:#}"), window, cx);
        }
    }

    pub(super) fn open_context_picker(
        &mut self,
        id: NodeId,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let item = view.read(cx).item().read(cx);
        let Some(document) = item.document() else {
            return;
        };
        let doc = &document.doc;
        let mut choices = Vec::new();
        match action {
            Action::MoveToPage => {
                for page in doc.pages() {
                    if *page == id || doc.scene.ancestors_of(id).any(|node| node.id == *page) {
                        continue;
                    }
                    if let Some(node) = doc.scene.get(*page) {
                        choices.push((node.name.clone(), Choice::Page(*page)));
                    }
                }
            }
            Action::MoreLayoutOptions => {
                choices.extend([
                    (
                        "Horizontal layout".into(),
                        Choice::Layout(Some(LayoutMode::Horizontal)),
                    ),
                    (
                        "Vertical layout".into(),
                        Choice::Layout(Some(LayoutMode::Vertical)),
                    ),
                ]);
                if matches!(doc.scene.get(id).map(|node| &node.data), Some(NodeData::Group(group)) if group.auto_layout.is_some())
                {
                    choices.extend([
                        ("Toggle wrap".into(), Choice::ToggleWrap),
                        ("Remove auto layout".into(), Choice::Layout(None)),
                    ]);
                }
            }
            Action::CropImage => choices.extend([
                ("Custom crop…".into(), Choice::CustomCrop),
                ("Square · 1:1".into(), Choice::Crop(Some(1.))),
                ("Landscape · 4:3".into(), Choice::Crop(Some(4. / 3.))),
                ("Widescreen · 16:9".into(), Choice::Crop(Some(16. / 9.))),
                ("Portrait · 3:4".into(), Choice::Crop(Some(3. / 4.))),
                ("Portrait · 9:16".into(), Choice::Crop(Some(9. / 16.))),
                ("Reset crop".into(), Choice::Crop(None)),
            ]),
            Action::CopyPasteAs => {
                choices.extend([
                    ("Copy as SVG".into(), Choice::CopySvg),
                    ("Copy as PNG".into(), Choice::CopyPng),
                    ("Copy properties".into(), Choice::CopyProperties),
                ]);
                if item.is_editable() && ops::editable(doc, id) {
                    choices.push(("Paste properties".into(), Choice::PasteProperties));
                }
            }
            _ => return,
        }
        if choices.is_empty() {
            crate::view::show_canvas_notice("There are no other pages".into(), window, cx);
            return;
        }
        let document_id = doc.id;
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.context_picker = Some(ContextPicker {
            document: document_id,
            node: id,
            anchor: window.mouse_position(),
            choices,
            focus,
            scroll: ScrollHandle::new(),
            crop_inputs: Vec::new(),
        });
        cx.notify();
    }

    pub(super) fn render_context_picker(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(picker) = &self.context_picker else {
            return div().into_any_element();
        };
        let mut menu = fanta_gpui::molecules::menu_panel("layer-command-options", px(240.), cx)
            .debug_selector(|| "layer-command-picker".to_owned())
            .occlude()
            .max_h((window.viewport_size().height - px(16.)).min(px(420.)))
            .overflow_y_scroll()
            .track_scroll(&picker.scroll)
            .track_focus(&picker.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.context_picker = None;
                    window.focus(&this.focus_handle, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.context_picker = None;
                cx.notify();
            }));
        for (label, input) in ["X %", "Y %", "Width %", "Height %"]
            .into_iter()
            .zip(&picker.crop_inputs)
        {
            menu = menu.child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .child(div().w(px(68.)).child(label))
                    .child(gpui_component::input::Input::new(input).w(px(120.))),
            );
        }
        for (index, (label, choice)) in picker.choices.iter().enumerate() {
            let choice = *choice;
            menu = menu.child(
                fanta_gpui::molecules::context_menu_item(
                    ("layer-command-choice", index),
                    px(28.),
                    true,
                    cx,
                )
                .child(label.clone())
                .on_activate(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    let choice = if matches!(choice, Choice::ApplyCrop) {
                        let Some(picker) = &this.context_picker else {
                            return;
                        };
                        let values: Result<Vec<f64>, _> = picker
                            .crop_inputs
                            .iter()
                            .map(|input| input.read(cx).value().parse::<f64>())
                            .collect();
                        let values = values
                            .ok()
                            .and_then(|values| <[f64; 4]>::try_from(values).ok());
                        let Some(values) = values else {
                            crate::view::show_canvas_notice(
                                "Enter a number in each crop field".into(),
                                window,
                                cx,
                            );
                            return;
                        };
                        Choice::CropRectangle(values)
                    } else {
                        choice
                    };
                    let Some(picker) = this.context_picker.take() else {
                        return;
                    };
                    let current = this.active_view(cx).and_then(|view| {
                        view.read(cx)
                            .item()
                            .read(cx)
                            .document()
                            .map(|document| document.doc.id)
                    });
                    if current == Some(picker.document) {
                        this.run_context_choice(picker.node, choice, window, cx);
                    }
                    cx.notify();
                })),
            );
        }
        deferred(anchored().position(picker.anchor).child(menu))
            .with_priority(5)
            .into_any_element()
    }

    fn run_context_choice(
        &mut self,
        id: NodeId,
        choice: Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match choice {
            Choice::CustomCrop => {
                let Some(view) = self.active_view(cx) else {
                    return;
                };
                let Some((document, crop)) =
                    view.read(cx)
                        .item()
                        .read(cx)
                        .document()
                        .and_then(|document| match &document.doc.scene.get(id)?.data {
                            NodeData::Bitmap(bitmap) => {
                                Some((document.doc.id, bitmap.crop.unwrap_or([0., 0., 1., 1.])))
                            }
                            _ => None,
                        })
                else {
                    return;
                };
                let crop_inputs = crop
                    .into_iter()
                    .map(|value| {
                        cx.new(|cx| {
                            let mut input = gpui_component::input::InputState::new(window, cx);
                            input.set_value(format!("{}", value * 100.), window, cx);
                            input
                        })
                    })
                    .collect();
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                self.context_picker = Some(ContextPicker {
                    document,
                    node: id,
                    anchor: window.mouse_position(),
                    choices: vec![("Apply crop".into(), Choice::ApplyCrop)],
                    focus,
                    scroll: ScrollHandle::new(),
                    crop_inputs,
                });
            }
            Choice::ApplyCrop => {}
            Choice::CropRectangle(crop) => self.apply_layer_command(
                "Crop image",
                |doc| ops::crop_rectangle(doc, id, crop),
                window,
                cx,
            ),
            Choice::ToggleWrap => self.apply_layer_command(
                "Toggle layout wrapping",
                |doc| {
                    ensure!(ops::editable(doc, id), "The layer is locked");
                    let node = doc.scene.get(id).context("Missing layer")?;
                    let NodeData::Group(group) = &node.data else {
                        anyhow::bail!("Choose an auto-layout frame");
                    };
                    let mut group = group.clone();
                    let layout = group.auto_layout.get_or_insert_with(Default::default);
                    layout.wrap = !layout.wrap;
                    Ok(vec![ops::replace_data(node, NodeData::Group(group))])
                },
                window,
                cx,
            ),
            Choice::Page(page) => self.apply_layer_command(
                "Move to page",
                |doc| ops::move_to_page(doc, id, page),
                window,
                cx,
            ),
            Choice::Layout(mode) => self.apply_layer_command(
                "Auto layout",
                |doc| ops::auto_layout(doc, id, mode),
                window,
                cx,
            ),
            Choice::Crop(aspect) => {
                self.apply_layer_command("Crop image", |doc| ops::crop(doc, id, aspect), window, cx)
            }
            Choice::CopySvg | Choice::CopyPng => {
                let result = self
                    .active_view(cx)
                    .context("The document is closed")
                    .and_then(|view| {
                        let item = view.read(cx).item().read(cx);
                        let document = item.document().context("The document is closed")?;
                        crate::export::render_layer(
                            &document.doc,
                            document.asset_resolver.clone(),
                            id,
                            if matches!(choice, Choice::CopySvg) {
                                crate::export::ExportFormat::Svg
                            } else {
                                crate::export::ExportFormat::Png
                            },
                        )
                    });
                match result {
                    Ok(bytes) => {
                        if matches!(choice, Choice::CopySvg) {
                            match String::from_utf8(bytes) {
                                Ok(svg) => cx.write_to_clipboard(ClipboardItem::new_string(svg)),
                                Err(error) => {
                                    crate::view::show_canvas_notice(error.to_string(), window, cx)
                                }
                            }
                        } else {
                            cx.write_to_clipboard(ClipboardItem::new_image(
                                &gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes),
                            ));
                        }
                    }
                    Err(error) => {
                        crate::view::show_canvas_notice(format!("Copy: {error:#}"), window, cx)
                    }
                }
            }
            Choice::CopyProperties => {
                let payload = self.active_view(cx).and_then(|view| {
                    let item = view.read(cx).item().read(cx);
                    let document = item.document()?;
                    Some(StyleClipboard {
                        document: document.doc.id,
                        node: document.doc.scene.get(id)?.clone(),
                    })
                });
                if let Some(payload) = payload {
                    cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
                        "Fanta layer properties".into(),
                        payload,
                    ));
                }
            }
            Choice::PasteProperties => {
                let payload = cx.read_from_clipboard().and_then(|clipboard| {
                    clipboard.entries.into_iter().find_map(|entry| match entry {
                        ClipboardEntry::String(text) => text.metadata_json::<StyleClipboard>(),
                        _ => None,
                    })
                });
                self.apply_layer_command("Paste properties", |doc| {
                    let payload = payload.context("Copy layer properties first")?;
                    ensure!(payload.document == doc.id, "Copy properties within this document so asset and variable references remain valid");
                    ops::paste_properties(doc, id, &payload.node)
                }, window, cx);
            }
        }
    }

    pub(super) fn paste_layer_to_replace(
        &mut self,
        id: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let payload = cx.read_from_clipboard().and_then(|clipboard| {
            clipboard.entries.into_iter().find_map(|entry| match entry {
                ClipboardEntry::String(text) => {
                    text.metadata_json::<crate::clipboard::CanvasClipboard>()
                }
                _ => None,
            })
        });
        self.apply_layer_command_select(
            "Paste to replace",
            |doc| {
                let operations = crate::clipboard::paste_to_replace_operations(
                    doc,
                    id,
                    &payload.context("Copy layers first")?,
                )?;
                let selected = ops::created_roots(&operations);
                Ok((operations, Some(selected)))
            },
            window,
            cx,
        );
    }

    pub(super) fn replace_layer_media(
        &mut self,
        id: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |view, cx| {
            view.finish_document_edits_for_external_change(cx)
        });
        let item = view.read(cx).item().clone();
        let Some((document_id, is_video)) = item.read(cx).document().and_then(|document| {
            Some((
                document.doc.id,
                matches!(document.doc.scene.get(id)?.data, NodeData::Video(_)),
            ))
        }) else {
            return;
        };
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if is_video {
                    "Replace video"
                } else {
                    "Replace image"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<()> = async {
                let Some(path) = paths.await??.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(());
                };
                let bytes = cx
                    .background_spawn(async move {
                        ensure!(
                            std::fs::metadata(&path)?.len() <= 256 * 1024 * 1024,
                            "Media must be smaller than 256 MB"
                        );
                        std::fs::read(&path).context("Could not read the selected media")
                    })
                    .await?;
                let (bytes, video) = if is_video {
                    (
                        Vec::new(),
                        Some(crate::generation_media::prepare_video(Arc::from(bytes)).await?),
                    )
                } else {
                    (bytes, None)
                };
                item.update(cx, |item, cx| {
                    ensure!(item.is_editable(), "This document is read-only");
                    item.with_document(cx, |document| {
                        let result = ops::replace_media(document, document_id, id, bytes, video);
                        let change = if result.is_ok() {
                            DocChange::Content
                        } else {
                            DocChange::None
                        };
                        (result, change)
                    })
                    .context("The document is closed")?
                })?;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                this.update_in(cx, |_, window, cx| {
                    crate::view::show_canvas_notice(format!("Replace media: {error:#}"), window, cx)
                })?;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    pub(super) fn set_layer_thumbnail(
        &mut self,
        id: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_layer_command(
            "Set project thumbnail",
            |doc| {
                ensure!(ops::editable(doc, id), "The layer is locked");
                let bounds = fanta_render::visual_world_bounds(&doc.scene, id, 0.)
                    .context("The layer has no visible bounds")?;
                ensure!(
                    bounds.width().is_finite()
                        && bounds.height().is_finite()
                        && bounds.width() > 0.
                        && bounds.height() > 0.,
                    "The layer has no visible bounds"
                );
                let mut operations = Vec::new();
                for node in doc
                    .scene
                    .roots()
                    .iter()
                    .flat_map(|id| doc.scene.descendants_of(*id))
                    .filter_map(|id| doc.scene.get(id))
                {
                    let selected = node.id == id;
                    if !selected && node.meta.get("fanta_project_thumbnail").is_none() {
                        continue;
                    }
                    let mut meta = node.meta.clone();
                    if !meta.is_object() {
                        meta = serde_json::json!({});
                    }
                    if let Some(object) = meta.as_object_mut() {
                        if selected {
                            object
                                .insert("fanta_project_thumbnail".into(), serde_json::json!(true));
                        } else {
                            object.remove("fanta_project_thumbnail");
                        }
                    }
                    operations.push(Operation::SetMeta {
                        id: node.id,
                        old: node.meta.clone(),
                        new: meta,
                    });
                }
                Ok(operations)
            },
            window,
            cx,
        );
    }
}
