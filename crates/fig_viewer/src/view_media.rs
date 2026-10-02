use super::*;

impl FigView {
    pub(super) fn local_media_origin(&self, cx: &Context<Self>) -> Result<LocalMediaOrigin> {
        let item = self.item.read(cx);
        anyhow::ensure!(
            self.is_editable(cx),
            "This document is currently read-only."
        );
        anyhow::ensure!(
            item.can_preview_for_owner(cx.entity_id()),
            "Finish saving or editing this document before placing media."
        );
        anyhow::ensure!(
            self.editor_workspace(cx) == EditorWorkspace::Canvas && self.prototype_player.is_none(),
            "Return to the canvas editor before placing media."
        );
        anyhow::ensure!(
            !self.has_pending_authoring(cx),
            "Finish or cancel the current edit before placing media."
        );
        let document = item
            .document()
            .context("The document is no longer available.")?;
        let page = document
            .doc
            .active_page()
            .context("Choose a page before placing media.")?;
        anyhow::ensure!(
            document.doc.pages().contains(&page),
            "Choose a page before placing media."
        );
        Ok(LocalMediaOrigin {
            item: self.item.entity_id(),
            scene: document.doc.scene.instance_id(),
            page,
            scope: self.scope,
            mode: self.editor_mode(cx),
            path: item.abs_path().to_path_buf(),
            generation: self.media_import_generation.get(),
        })
    }

    pub(super) fn validate_local_media_origin(
        &self,
        origin: &LocalMediaOrigin,
        cx: &Context<Self>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.local_media_origin(cx)? == *origin,
            "The document or page changed while choosing media. Choose the file again."
        );
        Ok(())
    }

    pub(crate) fn choose_local_media(&mut self, cx: &mut Context<Self>) {
        self.choose_local_media_with(
            |path| async move { crate::generation_media::prepare_local_media(&path).await },
            cx,
        );
    }

    pub(crate) fn choose_local_media_with<Preparation>(
        &mut self,
        prepare: impl FnOnce(std::path::PathBuf) -> Preparation + 'static,
        cx: &mut Context<Self>,
    ) where
        Preparation: std::future::Future<Output = Result<crate::generation_media::PreparedLocalMedia>>
            + Send
            + 'static,
    {
        if self.media_import_task.is_some() {
            show_canvas_notice_deferred("A media import is already in progress.".into(), cx);
            return;
        }
        let origin = match self.local_media_origin(cx) {
            Ok(origin) => origin,
            Err(error) => {
                show_canvas_notice_deferred(format!("Could not place media: {error:#}"), cx);
                return;
            }
        };
        let viewport = self.viewport.unwrap_or(Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        });
        let visible = self
            .container_bounds
            .map(bounds_size)
            .map(|(width, height)| [width / viewport.zoom, height / viewport.zoom])
            .unwrap_or([1024.0, 768.0]);
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Place image or MP4 video".into()),
        });
        self.media_import_task = Some(cx.spawn(async move |this, cx| {
            let result: Result<Option<crate::generation_media::PreparedLocalMedia>> = async {
                let Some(paths) = paths
                    .await
                    .context("The file picker closed unexpectedly.")??
                else {
                    return Ok(None);
                };
                anyhow::ensure!(paths.len() <= 1, "Choose one image or MP4 video at a time.");
                let Some(path) = paths.into_iter().next() else {
                    return Ok(None);
                };
                this.update(cx, |this, cx| {
                    this.validate_local_media_origin(&origin, cx)?;
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    show_canvas_notice_deferred(format!("Preparing {name}…"), cx);
                    Ok::<_, anyhow::Error>(())
                })??;
                let work = cx.background_spawn(prepare(path));
                let timer = cx
                    .background_executor()
                    .timer(std::time::Duration::from_secs(30));
                let prepared = match futures::future::select(work, timer).await {
                    futures::future::Either::Left((result, _)) => result?,
                    futures::future::Either::Right(_) => {
                        anyhow::bail!("Preparing this media took too long. Try a smaller file.")
                    }
                };
                Ok(Some(prepared))
            }
            .await;
            if let Err(error) = this.update(cx, |this, cx| {
                this.media_import_task = None;
                let result = result.and_then(|prepared| {
                    let Some(prepared) = prepared else {
                        return Ok(false);
                    };
                    this.validate_local_media_origin(&origin, cx)?;
                    this.item.update(cx, |item, cx| {
                        item.with_document(cx, |document| {
                            crate::generation_media::place_local_media(
                                document,
                                prepared,
                                viewport.center,
                                visible,
                            )
                        })
                        .context("The media document is no longer available.")?
                    })?;
                    Ok(true)
                });
                match result {
                    Ok(true) => {
                        this.tools.activate_without_context(ToolKind::Select);
                        this.remember_tool_face(ToolKind::Select);
                        this.invalidate_canvas_cache();
                        show_canvas_notice_deferred("Media placed on the canvas.".into(), cx);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        log::warn!("placing local media failed: {error:#}");
                        show_canvas_notice_deferred(
                            format!("Could not place media: {error:#}"),
                            cx,
                        );
                    }
                }
                cx.notify();
            }) {
                log::debug!("The media import owner was closed: {error}");
            }
        }));
        cx.notify();
    }
}
