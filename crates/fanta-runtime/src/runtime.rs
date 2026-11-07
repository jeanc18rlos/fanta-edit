#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use fanta_bundle::RuntimeProgram;
use fanta_canvas::{Camera2D, HitPrecision, camera_screen_to_world, camera_world_to_screen};
use fanta_doc::{
    AnimationClipId, AssetId, BoundProp, MotionProperty, NodeData, NodeId, ResolvedVarValue, Scene,
    TextStyle, TextStyleRun, VarValue,
};
use fanta_present::{FitMode, PlaybackSession};
use fanta_render::{LazyAssetResolver, RasterRenderer, TextRenderContext, decode_image_bytes};
use fanta_text::{FontRegistration, LayoutEngine};
use glam::DVec2;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct RuntimeError(pub String);

impl RuntimeError {
    fn from_error(error: impl std::fmt::Display) -> Self {
        Self(error.to_string())
    }
}

struct Program {
    compiled: RuntimeProgram,
    text: TextRenderContext,
    assets: Arc<LazyAssetResolver>,
    text_clips: BTreeSet<AnimationClipId>,
}

pub struct Player {
    pub session: PlaybackSession,
    pub renderer: Option<RasterRenderer>,
    pub text: TextRenderContext,
    program: Rc<Program>,
    paused: bool,
    validated_text: BTreeSet<(String, u16, bool, String)>,
}

const MAX_PROGRAM_INPUT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub struct Runtime {
    programs: BTreeMap<u32, Rc<Program>>,
    players: BTreeMap<u32, Player>,
    next_handle: u32,
}

impl Runtime {
    pub fn load_bundle(&mut self, bytes: &[u8]) -> Result<u32, RuntimeError> {
        let program = RuntimeProgram::load_bundle_with_limit(bytes, MAX_PROGRAM_INPUT_BYTES)
            .map_err(RuntimeError::from_error)?;
        self.insert_program(program)
    }

    fn insert_program(&mut self, compiled: RuntimeProgram) -> Result<u32, RuntimeError> {
        let registrations: Vec<_> = compiled
            .fonts()
            .iter()
            .map(|font| FontRegistration {
                family: &font.family,
                weight: font.weight,
                italic: font.italic,
                face_index: font.face_index,
                bytes: &font.bytes,
            })
            .collect();
        let engine = LayoutEngine::from_fonts(registrations, compiled.font_substitutions())
            .map_err(RuntimeError::from_error)?;
        let text_context = TextRenderContext::new(engine);
        let scene = &compiled.document().scene;
        for node in scene
            .roots()
            .iter()
            .flat_map(|root| scene.descendants_of(*root))
            .filter_map(|identifier| scene.get(identifier))
        {
            match &node.data {
                NodeData::Text(text) => {
                    validate_text(&text_context, &text.content, &text.style, &text.style_runs)
                }
                NodeData::TextPath(text) => {
                    validate_text(&text_context, &text.content, &text.style, &text.style_runs)
                }
                _ => Ok(()),
            }
            .map_err(|error| RuntimeError(format!("Text {:?}: {}", node.name, error.0)))?;
        }
        for variable in compiled.document().variables.variables.values() {
            for value in variable.values_by_mode.values() {
                if let VarValue::TextStyle { value: style } = value {
                    validate_text(&text_context, "", style, &[])?;
                }
            }
        }
        let mut text_clips = BTreeSet::new();
        for clip in compiled.document().motion.clips.values() {
            for track in clip.tracks.values() {
                if matches!(
                    track.target.property,
                    MotionProperty::Bound {
                        prop: BoundProp::TextStyle | BoundProp::TextContent
                    }
                ) {
                    text_clips.insert(clip.id);
                }
                for keyframe in track.keyframes.values() {
                    if let ResolvedVarValue::TextStyle { value: style } = &keyframe.value {
                        validate_text(&text_context, "", style, &[])?;
                    }
                }
            }
        }
        for encoded in compiled.resources().values() {
            if let Some([width, height]) = fanta_render::encoded_image_dimensions(encoded) {
                let decoded_bytes = u64::from(width) * u64::from(height) * 4;
                if decoded_bytes > 64 * 1024 * 1024 {
                    return Err(RuntimeError(
                        "Image exceeds the 64 MiB decoded resource budget".into(),
                    ));
                }
            }
        }
        let mut image_resources = BTreeSet::new();
        collect_image_resources(
            &serde_json::to_value(compiled.document()).map_err(RuntimeError::from_error)?,
            &mut image_resources,
        )?;
        for identifier in image_resources {
            let encoded = compiled
                .resources()
                .get(&identifier)
                .ok_or_else(|| RuntimeError(format!("Missing image resource {identifier}")))?;
            if decode_image_bytes(encoded).is_none() {
                return Err(RuntimeError(format!(
                    "Invalid or unsupported image resource {identifier}"
                )));
            }
        }
        let assets = Arc::new(LazyAssetResolver::with_budget(
            Arc::new(compiled.resources().clone()),
            Arc::new(|_, bytes| decode_image_bytes(bytes)),
            64 * 1024 * 1024,
        ));
        let handle = self.allocate_handle()?;
        self.programs.insert(
            handle,
            Rc::new(Program {
                compiled,
                text: text_context,
                assets,
                text_clips,
            }),
        );
        Ok(handle)
    }

    fn allocate_handle(&mut self) -> Result<u32, RuntimeError> {
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .ok_or_else(|| RuntimeError("Runtime handle limit reached".into()))?;
        Ok(self.next_handle)
    }

    pub fn program_info(&self, handle: u32) -> Result<Value, RuntimeError> {
        let program = self
            .programs
            .get(&handle)
            .ok_or_else(|| RuntimeError("Program was disposed or does not exist".into()))?;
        let document = program.compiled.document();
        Ok(json!({
            "handle": handle,
            "documentId": document.id,
            "flows": document.flows,
            "clips": document.motion.clips.values().map(|clip| json!({"id":clip.id,"name":clip.name,"durationMs":clip.duration_ms})).collect::<Vec<_>>(),
            "diagnostics": program.compiled.diagnostics(),
            "capabilities": {"media":"posters", "liveVideo":false, "liveAudio":false},
        }))
    }

    pub fn inspect_program(&self, handle: u32) -> Result<Value, RuntimeError> {
        let program = self
            .programs
            .get(&handle)
            .ok_or_else(|| RuntimeError("Program was disposed or does not exist".into()))?;
        serde_json::to_value(program.compiled.document().clone_for_persist())
            .map_err(RuntimeError::from_error)
    }

    pub fn create_player(
        &mut self,
        program_handle: u32,
        width: f64,
        height: f64,
        start: Option<NodeId>,
    ) -> Result<u32, RuntimeError> {
        self.create_player_with_fit(program_handle, width, height, start, FitMode::Contain)
    }

    pub fn create_player_with_fit(
        &mut self,
        program_handle: u32,
        width: f64,
        height: f64,
        start: Option<NodeId>,
        fit: FitMode,
    ) -> Result<u32, RuntimeError> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(RuntimeError(
                "Player dimensions must be finite and positive".into(),
            ));
        }
        let program = self
            .programs
            .get(&program_handle)
            .cloned()
            .ok_or_else(|| RuntimeError("Program was disposed or does not exist".into()))?;
        let document = program.compiled.document();
        let start = start.or_else(|| document.flow_start()).or_else(|| {
            let page = document
                .active_page()
                .or_else(|| document.pages().first().copied());
            let candidates = page.map_or_else(
                || document.scene.roots(),
                |page| document.scene.children_of(Some(page)),
            );
            candidates.iter().copied().find(|identifier| {
                document.scene.get(*identifier).is_some_and(|node| {
                    !node.flags.contains(fanta_doc::NodeFlags::HIDDEN)
                        && matches!(&node.data,NodeData::Group(group) if group.is_frame_surface())
                })
            })
        });
        let mut session = program
            .text
            .with(|| {
                PlaybackSession::new_with_compiled_motion(
                    program.compiled.document(),
                    start,
                    DVec2::new(width, height),
                    program.text.clone(),
                    program.compiled.compiled_motion().clone(),
                )
            })
            .map_err(RuntimeError::from_error)?;
        session.set_fit_mode(fit);
        session.fit_view();
        let mut player = Player {
            session,
            renderer: None,
            text: program.text.clone(),
            program,
            paused: false,
            validated_text: BTreeSet::new(),
        };
        player.validate_effective_text()?;
        let handle = self.allocate_handle()?;
        self.players.insert(handle, player);
        Ok(handle)
    }

    pub fn player_mut(&mut self, handle: u32) -> Result<&mut Player, RuntimeError> {
        self.players
            .get_mut(&handle)
            .ok_or_else(|| RuntimeError("Player was disposed or does not exist".into()))
    }

    pub fn take_player(&mut self, handle: u32) -> Result<Player, RuntimeError> {
        self.players
            .remove(&handle)
            .ok_or_else(|| RuntimeError("Player was disposed or does not exist".into()))
    }

    pub fn dispose_program(&mut self, handle: u32) -> bool {
        self.programs.remove(&handle).is_some()
    }
    pub fn dispose_player(&mut self, handle: u32) -> bool {
        self.players.remove(&handle).is_some()
    }

    pub fn command(
        &mut self,
        handle: u32,
        command: &str,
        arguments: Value,
    ) -> Result<Value, RuntimeError> {
        let player = self.player_mut(handle)?;
        let context = player.text.clone();
        context.with(|| player.command(command, arguments))
    }
}

fn collect_image_resources(
    value: &Value,
    images: &mut BTreeSet<AssetId>,
) -> Result<(), RuntimeError> {
    match value {
        Value::Object(object) => {
            let image = object.get("type").and_then(Value::as_str) == Some("bitmap")
                || object.get("kind").and_then(Value::as_str) == Some("image");
            let resource = if image {
                object.get("asset")
            } else if object.get("type").and_then(Value::as_str) == Some("video") {
                object.get("poster")
            } else if object.get("kind").and_then(Value::as_str) == Some("video") {
                object.get("video").and_then(|video| video.get("poster"))
            } else {
                None
            };
            if let Some(resource) = resource.filter(|value| !value.is_null()) {
                images.insert(
                    serde_json::from_value(resource.clone()).map_err(RuntimeError::from_error)?,
                );
            }
            for child in object.values() {
                collect_image_resources(child, images)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_image_resources(child, images)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_text(
    engine: &TextRenderContext,
    content: &str,
    base: &TextStyle,
    runs: &[TextStyleRun],
) -> Result<(), RuntimeError> {
    let mut boundaries = vec![0, content.len()];
    for run in runs {
        if content.get(run.start..run.end).is_none() {
            return Err(RuntimeError(
                "Text style range is not a valid UTF-8 range".into(),
            ));
        }
        boundaries.extend([run.start, run.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    for style in std::iter::once(base).chain(runs.iter().map(|run| &run.style)) {
        if !engine.supports_family(&style.font_family) {
            return Err(RuntimeError(format!(
                "Missing font {:?}; supply its bytes or an explicit substitution",
                style.font_family
            )));
        }
        engine
            .validate_text(&style.font_family, style.weight, style.italic, "")
            .map_err(RuntimeError::from_error)?;
    }
    for interval in boundaries.windows(2) {
        let [start, end] = interval else {
            continue;
        };
        let style = runs
            .iter()
            .rev()
            .find(|run| run.start <= *start && run.end >= *end)
            .map_or(base, |run| &run.style);
        let text = content
            .get(*start..*end)
            .ok_or_else(|| RuntimeError("Invalid text range".into()))?;
        engine
            .validate_text(&style.font_family, style.weight, style.italic, text)
            .map_err(RuntimeError::from_error)?;
    }
    Ok(())
}

fn argument<T: for<'de> Deserialize<'de>>(arguments: &Value, key: &str) -> Result<T, RuntimeError> {
    let value = arguments
        .get(key)
        .ok_or_else(|| RuntimeError(format!("Missing argument {key}")))?;
    serde_json::from_value(value.clone()).map_err(RuntimeError::from_error)
}

impl Player {
    pub fn asset_resolver(&self) -> Arc<LazyAssetResolver> {
        self.program.assets.clone()
    }

    pub fn prepare_renderer(&mut self) -> Result<(), RuntimeError> {
        let (width, height) = self.session.pixel_size();
        if let Some(renderer) = self.renderer.as_mut() {
            renderer
                .resize(width, height)
                .map_err(RuntimeError::from_error)?;
        } else {
            let mut renderer =
                RasterRenderer::new(width, height).map_err(RuntimeError::from_error)?;
            renderer.set_asset_resolver(self.program.assets.clone());
            renderer.set_text_context(self.text.clone());
            if let Some(color) = self
                .program
                .compiled
                .document()
                .presentation
                .as_ref()
                .and_then(|presentation| presentation.frame_color)
            {
                renderer.background = color;
            }
            renderer.set_image_cache_budget_bytes(Some(64 * 1024 * 1024));
            renderer.set_layer_cache_budget_bytes(64 * 1024 * 1024);
            self.renderer = Some(renderer);
        }
        Ok(())
    }

    fn validate_effective_text(&mut self) -> Result<(), RuntimeError> {
        let mut failure = None;
        let validated = &mut self.validated_text;
        let text_context = &self.text;
        if validated.len() > 16384 {
            validated.clear();
        }
        self.session.visit_effective_nodes(|node| {
            let (content, style, runs) = match &node.data {
                NodeData::Text(text) => (&text.content, &text.style, &text.style_runs),
                NodeData::TextPath(text) => (&text.content, &text.style, &text.style_runs),
                _ => return,
            };
            let key = (
                style.font_family.clone(),
                style.weight,
                style.italic,
                content.clone(),
            );
            if !runs.is_empty() || !validated.contains(&key) {
                match validate_text(text_context, content, style, runs) {
                    Ok(()) => {
                        validated.insert(key);
                    }
                    Err(error) => {
                        if failure.is_none() {
                            failure = Some(error);
                        }
                    }
                }
            }
        });
        failure.map_or(Ok(()), Err)
    }

    fn command(&mut self, command: &str, arguments: Value) -> Result<Value, RuntimeError> {
        use fanta_present::{KeyEvent, PointerEvent};
        match command {
            "hitTest" => return self.hit_test(&arguments),
            "nodeBounds" => return self.node_bounds(&arguments),
            _ => {}
        }
        let mut response = fanta_present::PresentResponse::default();
        let previous_layer = self.session.playback_state().current_layer;
        let previous_generation = self.session.resolution_generation();
        let timer_due = command == "advanceBy"
            && self
                .session
                .schedule()
                .next_deadline_ms
                .is_some_and(|deadline| {
                    arguments
                        .get("deltaMs")
                        .and_then(Value::as_f64)
                        .is_some_and(|elapsed| elapsed >= deadline)
                });
        match command {
            "advanceBy" => {
                let milliseconds: f64 = argument(&arguments, "deltaMs")?;
                if !milliseconds.is_finite() || milliseconds < 0.0 {
                    return Err(RuntimeError(
                        "deltaMs must be finite and nonnegative".into(),
                    ));
                }
                if !self.paused {
                    response = self
                        .session
                        .advance_by(milliseconds / 1000.0)
                        .map_err(RuntimeError::from_error)?;
                }
            }
            "pointer" => {
                let coordinates: [f64; 2] = argument(&arguments, "point")?;
                if !coordinates.into_iter().all(f64::is_finite) {
                    return Err(RuntimeError("Pointer coordinates must be finite".into()));
                }
                let event = match argument::<String>(&arguments, "type")?.as_str() {
                    "move" => PointerEvent::Move,
                    "click" => PointerEvent::Click,
                    "dragStart" => PointerEvent::DragStart,
                    "down" => PointerEvent::Down,
                    "up" => PointerEvent::Up,
                    "leave" => PointerEvent::Leave,
                    _ => return Err(RuntimeError("Unknown pointer event".into())),
                };
                response = self
                    .session
                    .handle_pointer(DVec2::from_array(coordinates), event);
            }
            "cancelPointer" => response = self.session.cancel_pointer_input(),
            "key" => {
                response = self.session.handle_key(
                    &argument::<String>(&arguments, "key")?,
                    if arguments.get("type").and_then(Value::as_str) == Some("up") {
                        KeyEvent::Up
                    } else {
                        KeyEvent::Down
                    },
                );
            }
            "scroll" => {
                let point: [f64; 2] = argument(&arguments, "point")?;
                let delta: [f64; 2] = argument(&arguments, "delta")?;
                if !point.into_iter().chain(delta).all(f64::is_finite) {
                    return Err(RuntimeError("Scroll coordinates must be finite".into()));
                }
                response.needs_redraw = self
                    .session
                    .scroll_by_screen(DVec2::from_array(point), delta)
                    .is_some();
            }
            "pan" => {
                let delta: [f64; 2] = argument(&arguments, "delta")?;
                response = self
                    .session
                    .pan_view(DVec2::from_array(delta))
                    .map_err(RuntimeError::from_error)?;
            }
            "zoom" => {
                let point: [f64; 2] = argument(&arguments, "point")?;
                response = self
                    .session
                    .zoom_view(DVec2::from_array(point), argument(&arguments, "factor")?)
                    .map_err(RuntimeError::from_error)?;
            }
            "fit" => {
                let mode = arguments
                    .get("mode")
                    .map(|mode| {
                        if !mode.is_string() {
                            return Err(RuntimeError(
                                "fit mode must be contain, cover, or fill".into(),
                            ));
                        }
                        serde_json::from_value::<FitMode>(mode.clone())
                            .map_err(RuntimeError::from_error)
                    })
                    .transpose()?;
                if let Some(mode) = mode {
                    self.session.set_fit_mode(mode);
                }
                response = self.session.fit_view();
            }
            "startFlow" => {
                if !self.session.start_flow(argument(&arguments, "index")?) {
                    return Err(RuntimeError("Unknown flow".into()));
                }
                response.needs_redraw = true;
                response.navigated = true;
            }
            "restart" => {
                let start = self.program.compiled.document().flow_start().or_else(|| {
                    self.program
                        .compiled
                        .document()
                        .flows
                        .first()
                        .map(|flow| flow.start)
                });
                let start = arguments
                    .get("start")
                    .filter(|value| !value.is_null())
                    .map(|value| {
                        serde_json::from_value(value.clone()).map_err(RuntimeError::from_error)
                    })
                    .transpose()?
                    .or(start)
                    .unwrap_or(self.session.current_frame());
                if !self.session.restart_at(start) {
                    return Err(RuntimeError("Unknown start frame".into()));
                }
                self.paused = false;
                response.needs_redraw = true;
                response.navigated = true;
            }
            "navigate" => {
                let frame = argument(&arguments, "frame")?;
                if !self.session.document().scene.contains(frame) {
                    return Err(RuntimeError("Unknown frame".into()));
                }
                response = self.session.navigate(frame);
            }
            "back" => {
                response = self.session.back();
            }
            "setVariable" => {
                response = self
                    .session
                    .set_runtime_variable(
                        argument(&arguments, "variable")?,
                        argument(&arguments, "value")?,
                    )
                    .map_err(RuntimeError::from_error)?;
            }
            "setMode" => {
                response = self
                    .session
                    .set_runtime_mode(
                        argument(&arguments, "collection")?,
                        argument(&arguments, "mode")?,
                    )
                    .map_err(RuntimeError::from_error)?;
            }
            "playClip" => {
                let clip: AnimationClipId = argument(&arguments, "clip")?;
                let scope = arguments
                    .get("scope")
                    .filter(|value| !value.is_null())
                    .map(|value| {
                        serde_json::from_value(value.clone()).map_err(RuntimeError::from_error)
                    })
                    .transpose()?;
                // The authored loop mode is the default; the host may override it.
                let mode = match arguments.get("mode").filter(|value| !value.is_null()) {
                    Some(mode) => {
                        serde_json::from_value(mode.clone()).map_err(RuntimeError::from_error)?
                    }
                    None => self
                        .session
                        .document()
                        .motion
                        .clip(clip)
                        .map_or(fanta_present::ClipPlaybackMode::Once, |clip| {
                            clip.settings.loop_mode.into()
                        }),
                };
                self.session
                    .play_clip(clip, scope, mode)
                    .map_err(RuntimeError::from_error)?;
                response.needs_redraw = true;
            }
            "pauseClip" => self.session.pause_clip(),
            "resumeClip" => self.session.resume_clip(),
            "seekClip" => {
                self.session
                    .seek_clip(argument(&arguments, "timeMs")?)
                    .map_err(RuntimeError::from_error)?;
                response.needs_redraw = true;
            }
            "stopClip" => {
                self.session.stop_clip();
                response.needs_redraw = true;
            }
            "pausePlayback" => {
                self.paused = true;
                self.session.pause();
            }
            "resumePlayback" => {
                self.paused = false;
                self.session.resume();
            }
            "resize" => {
                let width: f64 = argument(&arguments, "width")?;
                let height: f64 = argument(&arguments, "height")?;
                let scale: f64 = argument(&arguments, "scale")?;
                if ![width, height, scale]
                    .into_iter()
                    .all(|value| value.is_finite() && value > 0.0)
                {
                    return Err(RuntimeError(
                        "Resize dimensions and scale must be finite and positive".into(),
                    ));
                }
                self.session
                    .resize_with_scale(DVec2::new(width, height), scale)
                    .map_err(RuntimeError::from_error)?;
                response.needs_redraw = true;
            }
            "setViewport" => {
                let camera: Camera2D =
                    serde_json::from_value(arguments).map_err(RuntimeError::from_error)?;
                response = self
                    .session
                    .set_camera(camera)
                    .map_err(RuntimeError::from_error)?;
            }
            "state" | "inspect" => {}
            _ => return Err(RuntimeError(format!("Unknown player command {command}"))),
        }
        let state = self.session.playback_state();
        let text_motion = state
            .clip
            .as_ref()
            .is_some_and(|clip| self.program.text_clips.contains(&clip.clip));
        // Camera moves redraw without changing which text is effective, and a
        // pan gesture would otherwise revalidate every text node per event.
        let camera = matches!(command, "pan" | "zoom" | "fit" | "setViewport");
        if self.session.resolution_generation() != previous_generation
            || state.current_layer != previous_layer
            || timer_due
            || (text_motion && command != "state" && command != "inspect")
            || (response.needs_redraw && command != "advanceBy" && !camera)
        {
            if let Err(error) = self.validate_effective_text() {
                self.paused = true;
                self.session.pause();
                return Err(error);
            }
        }
        let inspection = if command == "inspect" {
            let mut nodes = Vec::new();
            self.session.visit_effective_occurrences(|occurrence, node| nodes.push(json!({"occurrence": occurrence, "name":node.name,"transform":node.transform,"opacity":node.opacity,"flags":node.flags,"data":node.data,"styleAssignment":node.style_assignment})));
            Some(
                json!({"nodes":nodes,"variables":self.session.runtime_variables(),"activeModes":self.session.runtime_active_modes()}),
            )
        } else {
            None
        };
        let url = self.session.take_open_url();
        Ok(
            json!({"response":response,"state":self.session.playback_state(),"schedule":self.session.schedule(),"paused":self.paused,"openUrl":url,"events":self.session.take_playback_events(),"occurrences": if command == "state" {Some(self.session.occurrences())} else {None}, "inspection":inspection,"viewport":self.session.viewport(),"camera":self.session.camera(),"fitMode":self.session.fit_mode()}),
        )
    }

    /// The layer under a screen point. It is looked up in the design document
    /// rather than the playback layer because hosts name layers by the ids the
    /// design tools take, and to those an instance is one layer. A viewer that
    /// runs no reactions draws the document's geometry.
    fn hit_test(&self, arguments: &Value) -> Result<Value, RuntimeError> {
        let point: [f64; 2] = argument(arguments, "point")?;
        if !point.into_iter().all(f64::is_finite) {
            return Err(RuntimeError("Hit test coordinates must be finite".into()));
        }
        let deep = match arguments.get("deep") {
            None => false,
            Some(deep) => deep
                .as_bool()
                .ok_or_else(|| RuntimeError("deep must be a boolean".into()))?,
        };
        let scene = &self.session.document().scene;
        let world = camera_screen_to_world(
            DVec2::from_array(point),
            &self.session.camera(),
            self.session.screen_size(),
        );
        let Some(leaf) = fanta_canvas::hit_test(
            scene,
            world,
            HitPrecision::Path,
            Some(self.session.current_frame()),
        ) else {
            return Ok(Value::Null);
        };
        // From the page's top-level layer down to the hit, like a click and the
        // double-clicks that drill in from it in the editor.
        let mut path: Vec<NodeId> = std::iter::once(leaf)
            .chain(scene.ancestors_of(leaf).map(|ancestor| ancestor.id))
            .filter(|identifier| {
                scene
                    .get(*identifier)
                    .is_some_and(|node| node.parent.is_some())
            })
            .collect();
        path.reverse();
        let selected = if deep {
            leaf
        } else {
            path.first().copied().unwrap_or(leaf)
        };
        Ok(json!({
            "node": selected,
            "id": selected.to_string(),
            "name": scene.get(selected).map(|node| node.name.as_str()),
            "path": path,
        }))
    }

    /// Screen boxes of layers, keyed by their serde ids; null for a layer the
    /// shown page doesn't have.
    fn node_bounds(&self, arguments: &Value) -> Result<Value, RuntimeError> {
        let nodes: Vec<NodeId> = argument(arguments, "nodes")?;
        let scene = &self.session.document().scene;
        let page = self.session.current_frame();
        let camera = self.session.camera();
        let mut bounds = serde_json::Map::new();
        for node in nodes {
            let Value::String(key) =
                serde_json::to_value(node).map_err(RuntimeError::from_error)?
            else {
                return Err(RuntimeError("Node ids serialize as strings".into()));
            };
            let screen = fanta_canvas::hit_test::on_active_page(scene, node, page)
                .then(|| screen_box(scene, node, &camera, self.session.screen_size()))
                .flatten();
            bounds.insert(key, screen.unwrap_or(Value::Null));
        }
        Ok(Value::Object(bounds))
    }
}

/// A layer's own box on screen: `x`/`y` is its top-left corner and `rotation`
/// (degrees, clockwise) turns the box around it, so a rotated layer's outline
/// is exact.
fn screen_box(scene: &Scene, node: NodeId, camera: &Camera2D, screen_size: DVec2) -> Option<Value> {
    let local = scene.local_bounds(node)?;
    let transform = scene.world_transform(node)?;
    let corner = |x: f64, y: f64| {
        camera_world_to_screen(
            transform.transform_point(DVec2::new(x, y)),
            camera,
            screen_size,
        )
    };
    // A mirrored layer's corners run the other way round on screen, so its box
    // starts from the bottom-left corner to stay a rotated rectangle.
    let (origin, across, down) = if transform.0.matrix2.determinant() < 0.0 {
        (
            corner(local.min_x, local.max_y),
            corner(local.max_x, local.max_y),
            corner(local.min_x, local.min_y),
        )
    } else {
        (
            corner(local.min_x, local.min_y),
            corner(local.max_x, local.min_y),
            corner(local.min_x, local.max_y),
        )
    };
    let width = across - origin;
    Some(json!({
        "x": origin.x,
        "y": origin.y,
        "width": width.length(),
        "height": (down - origin).length(),
        "rotation": width.y.atan2(width.x).to_degrees(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        CanvasNode, Color, ComponentDef, ComponentId, Doc, DocId, Fill, GroupNode, IndexKey,
        InstanceNode, NodeFlags, Transform2D, VectorNode,
    };
    use fanta_format::runtime::SourceBundle;

    fn fit_bundle() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(fit_source().compile()?.to_bundle()?)
    }

    fn fit_source() -> SourceBundle {
        SourceBundle {
            files: BTreeMap::from([
                (
                    "fanta.json".into(),
                    br#"{"format":"fanta-project","version":6,"schema_version":4,"project_id":"d_00000000000000000000000030"}"#.to_vec(),
                ),
                (
                    "doc/metadata.json".into(),
                    br#"{"title":"Fit geometry","created_at":0,"modified_at":0}"#.to_vec(),
                ),
                (
                    "doc/flow_start.json".into(),
                    br#""00000000000000000000000002""#.to_vec(),
                ),
                (
                    "pages/fit/page.json".into(),
                    br#"{"id":"00000000000000000000000001","order":0}"#.to_vec(),
                ),
                (
                    "pages/fit/page.fnx".into(),
                    br#"<Frame id="00000000000000000000000001" name="Fit">
  <Frame id="00000000000000000000000002" name="Wide" x={400} y={60} width={200} height={100}>
    <Frame id="00000000000000000000000003" name="Hit" x={80} y={20} width={20} height={20} reactions={[{"id":"00000000000000000000000020","trigger":{"kind":"click"},"action":{"kind":"open_link","url":"https://example.invalid/fit"}}]} />
  </Frame>
</Frame>"#.to_vec(),
                ),
            ]),
            ..SourceBundle::default()
        }
    }

    #[test]
    fn player_fit_modes_preserve_legacy_creation_and_independent_program_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::default();
        let program = runtime.load_bundle(&fit_bundle()?)?;
        let legacy = runtime.create_player(program, 300.0, 300.0, None)?;
        assert_eq!(
            runtime.command(legacy, "state", json!({}))?["camera"],
            json!({"center":[500.0,110.0],"scale":[1.5,1.5]})
        );
        for (mode, name, scale, hit) in [
            (FitMode::Contain, "contain", [1.5_f64, 1.5], [130.0, 125.0]),
            (FitMode::Cover, "cover", [3.0, 3.0], [105.0, 80.0]),
            (FitMode::Fill, "fill", [1.5, 3.0], [130.0, 80.0]),
        ] {
            let player = runtime.create_player_with_fit(program, 300.0, 300.0, None, mode)?;
            let state = runtime.command(player, "state", json!({}))?;
            assert_eq!(state["fitMode"], name);
            assert_eq!(
                state["camera"],
                json!({"center":[500.0,110.0],"scale":scale})
            );
            assert_eq!(state["viewport"]["zoom"], scale[0].min(scale[1]));
            let clicked =
                runtime.command(player, "pointer", json!({"type":"click","point":hit}))?;
            assert_eq!(clicked["openUrl"], "https://example.invalid/fit");
            runtime.command(player, "fit", json!({"mode":"fill"}))?;
            assert_eq!(
                runtime.command(legacy, "state", json!({}))?["fitMode"],
                "contain"
            );
        }
        Ok(())
    }

    #[test]
    fn fit_requests_validate_atomically_and_refit_both_axes_without_advancing_playback()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::default();
        let program = runtime.load_bundle(&fit_bundle()?)?;
        let player = runtime.create_player_with_fit(program, 300.0, 300.0, None, FitMode::Fill)?;
        runtime.command(player, "advanceBy", json!({"deltaMs":125.0}))?;
        runtime.command(player, "pausePlayback", json!({}))?;
        let zoomed = runtime.command(player, "zoom", json!({"point":[60.0,90.0],"factor":2.0}))?;
        assert_eq!(
            zoomed["camera"],
            json!({"center":[470.0,100.0],"scale":[3.0,6.0]})
        );
        let panned = runtime.command(player, "pan", json!({"delta":[30.0,-12.0]}))?;
        assert_eq!(
            panned["camera"],
            json!({"center":[460.0,102.0],"scale":[3.0,6.0]})
        );
        let before = runtime.command(player, "state", json!({}))?;
        for invalid in [
            json!(null),
            json!(false),
            json!(1),
            json!([]),
            json!({}),
            json!({"contain":null}),
            json!("Fill"),
            json!("stretch"),
        ] {
            assert!(
                runtime
                    .command(player, "fit", json!({"mode":invalid}))
                    .is_err()
            );
            assert_eq!(runtime.command(player, "state", json!({}))?, before);
        }
        let fitted = runtime.command(player, "fit", json!({}))?;
        assert_eq!(
            fitted["camera"],
            json!({"center":[500.0,110.0],"scale":[1.5,3.0]})
        );
        assert_eq!(fitted["fitMode"], "fill");
        assert_eq!(fitted["state"]["clockMs"], 125.0);
        assert_eq!(fitted["schedule"], before["schedule"]);
        let cover = runtime.command(player, "fit", json!({"mode":"cover"}))?;
        assert_eq!(
            cover["camera"],
            json!({"center":[500.0,110.0],"scale":[3.0,3.0]})
        );
        assert_eq!(cover["fitMode"], "cover");
        assert!(runtime.player_mut(player)?.renderer.is_none());
        Ok(())
    }

    fn layer(
        document: &mut Doc,
        identity: u128,
        parent: Option<NodeId>,
        name: &str,
        transform: Transform2D,
        data: NodeData,
    ) -> Result<NodeId, Box<dyn std::error::Error>> {
        let mut node = CanvasNode::new(data);
        node.id = NodeId::from_u128(identity);
        node.parent = parent;
        node.index = IndexKey::from_raw(identity as f64);
        node.name = name.into();
        node.transform = transform;
        let identifier = node.id;
        document.scene.insert(node)?;
        Ok(identifier)
    }

    fn frame(width: f64, height: f64) -> NodeData {
        NodeData::Group(GroupNode {
            clip_size: Some([width, height]),
            background: Some(Fill::solid(Color::WHITE)),
            ..GroupNode::default()
        })
    }

    fn rectangle(width: f64, height: f64) -> NodeData {
        NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            width,
            height,
            Color::BLACK,
        ))
    }

    fn translated(x: f64, y: f64) -> Transform2D {
        Transform2D::translation(x, y)
    }

    /// Page 1 holds the card below; page 2 holds a frame at the same place,
    /// painted above it, which a hit on page 1 must never reach.
    ///
    /// Card (0, 0, 200, 200)
    ///   Inner (20, 20, 100, 100) > Dot (30, 30, 20, 20)
    ///   Chip, an instance (20, 140, 80, 40)
    ///   Shown (130, 20, 60, 60) under Hidden, at the same place
    ///   Under (130, 100, 60, 30) under Locked, at the same place
    ///   Turned, 40 × 20 turned 90° at (190, 140): covers (170, 140, 20, 40)
    ///   Mirrored, 20 × 10 flipped across x at (120, 190): covers (100, 190, 20, 10)
    fn viewer_bundle() -> Result<(Vec<u8>, NodeId), Box<dyn std::error::Error>> {
        let mut document = Doc::new();
        document.id = DocId::from_u128(40);
        document.metadata.created_at = 0;
        document.metadata.modified_at = 0;
        document.metadata.title = "Viewer geometry".into();
        let identity = Transform2D::IDENTITY;
        let page = layer(
            &mut document,
            1000,
            None,
            "Page",
            identity,
            NodeData::Group(GroupNode::default()),
        )?;
        document.add_page(page);
        let card = layer(
            &mut document,
            1001,
            Some(page),
            "Card",
            identity,
            frame(200.0, 200.0),
        )?;
        let inner = layer(
            &mut document,
            1002,
            Some(card),
            "Inner",
            translated(20.0, 20.0),
            frame(100.0, 100.0),
        )?;
        layer(
            &mut document,
            1003,
            Some(inner),
            "Dot",
            translated(10.0, 10.0),
            rectangle(20.0, 20.0),
        )?;
        let master = layer(
            &mut document,
            1020,
            None,
            "Chip master",
            identity,
            frame(80.0, 40.0),
        )?;
        layer(
            &mut document,
            1021,
            Some(master),
            "Chip fill",
            identity,
            rectangle(80.0, 40.0),
        )?;
        let component = ComponentId::from_u128(6000);
        document
            .components
            .defs
            .insert(component, ComponentDef::new(component, master, "Chip"));
        layer(
            &mut document,
            1004,
            Some(card),
            "Chip",
            translated(20.0, 140.0),
            NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [80.0, 40.0],
            }),
        )?;
        layer(
            &mut document,
            1005,
            Some(card),
            "Shown",
            translated(130.0, 20.0),
            rectangle(60.0, 60.0),
        )?;
        let hidden = layer(
            &mut document,
            1006,
            Some(card),
            "Hidden",
            translated(130.0, 20.0),
            rectangle(60.0, 60.0),
        )?;
        layer(
            &mut document,
            1007,
            Some(card),
            "Under",
            translated(130.0, 100.0),
            rectangle(60.0, 30.0),
        )?;
        let locked = layer(
            &mut document,
            1008,
            Some(card),
            "Locked",
            translated(130.0, 100.0),
            rectangle(60.0, 30.0),
        )?;
        for (node, flag) in [(hidden, NodeFlags::HIDDEN), (locked, NodeFlags::LOCKED)] {
            document.scene.get_mut(node).ok_or("fixture layer")?.flags |= flag;
        }
        layer(
            &mut document,
            1009,
            Some(card),
            "Turned",
            Transform2D(glam::DAffine2::from_angle_translation(
                std::f64::consts::FRAC_PI_2,
                DVec2::new(190.0, 140.0),
            )),
            rectangle(40.0, 20.0),
        )?;
        layer(
            &mut document,
            1010,
            Some(card),
            "Mirrored",
            Transform2D(glam::DAffine2::from_scale_angle_translation(
                DVec2::new(-1.0, 1.0),
                0.0,
                DVec2::new(120.0, 190.0),
            )),
            rectangle(20.0, 10.0),
        )?;
        let other_page = layer(
            &mut document,
            1100,
            None,
            "Other page",
            identity,
            NodeData::Group(GroupNode::default()),
        )?;
        document.add_page(other_page);
        layer(
            &mut document,
            1101,
            Some(other_page),
            "Elsewhere",
            identity,
            frame(200.0, 200.0),
        )?;

        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &document, &BTreeMap::new())?;
        let mut source = SourceBundle::default();
        let mut pending = vec![directory.path().to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in std::fs::read_dir(folder)? {
                let path = entry?.path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let relative = path.strip_prefix(directory.path())?;
                    source.files.insert(
                        relative.to_string_lossy().replace('\\', "/"),
                        std::fs::read(&path)?,
                    );
                }
            }
        }
        Ok((source.compile()?.to_bundle()?, page))
    }

    /// Where a world point of the document lands on the player's screen.
    fn on_screen(state: &Value, world: [f64; 2]) -> [f64; 2] {
        let camera: Camera2D = serde_json::from_value(state["camera"].clone()).unwrap_or_default();
        let screen_size = DVec2::new(400.0, 400.0);
        camera_world_to_screen(DVec2::from_array(world), &camera, screen_size).to_array()
    }

    fn key(identity: u128) -> Value {
        json!(NodeId::from_u128(identity))
    }

    #[test]
    fn hits_name_the_drawn_layer_on_the_shown_page_in_both_id_forms()
    -> Result<(), Box<dyn std::error::Error>> {
        let (bundle, page) = viewer_bundle()?;
        let mut runtime = Runtime::default();
        let program = runtime.load_bundle(&bundle)?;
        let player = runtime.create_player(program, 400.0, 400.0, Some(page))?;
        let state = runtime.command(player, "state", json!({}))?;

        let mut hit = |world: [f64; 2], deep: bool| {
            runtime.command(
                player,
                "hitTest",
                json!({"point": on_screen(&state, world), "deep": deep}),
            )
        };
        let dot = hit([40.0, 40.0], true)?;
        assert_eq!(dot["node"], key(1003));
        assert_eq!(dot["id"], NodeId::from_u128(1003).to_string());
        assert!(dot["id"].as_str().is_some_and(|id| id.starts_with("n_")));
        assert!(
            dot["node"]
                .as_str()
                .is_some_and(|node| !node.starts_with("n_"))
        );
        assert_eq!(dot["name"], "Dot");
        assert_eq!(dot["path"], json!([key(1001), key(1002), key(1003)]));
        // A plain click selects the page's top-level layer, as in the editor.
        let card = hit([40.0, 40.0], false)?;
        assert_eq!(card["node"], key(1001));
        assert_eq!(card["path"], dot["path"]);

        for (world, expected, path) in [
            // A frame's own body.
            ([25.0, 25.0], 1002, vec![1001, 1002]),
            // An instance is one layer, as it is to the design tools.
            ([60.0, 160.0], 1004, vec![1001, 1004]),
            // Hidden and locked layers let the click through to what they cover.
            ([160.0, 50.0], 1005, vec![1001, 1005]),
            ([160.0, 115.0], 1007, vec![1001, 1007]),
            ([180.0, 160.0], 1009, vec![1001, 1009]),
            ([110.0, 195.0], 1010, vec![1001, 1010]),
            // Page 2's frame sits at the same place, painted above.
            ([5.0, 5.0], 1001, vec![1001]),
        ] {
            let found = hit(world, true)?;
            assert_eq!(found["node"], key(expected), "at {world:?}");
            assert_eq!(
                found["path"],
                json!(path.into_iter().map(key).collect::<Vec<_>>()),
                "at {world:?}"
            );
        }
        assert_eq!(hit([-50.0, -50.0], true)?, Value::Null);
        assert_eq!(hit([40.0, 40.0], false)?["node"], key(1001));

        for invalid in [
            json!({}),
            json!({"point": [1.0]}),
            json!({"point": [0.0, 0.0], "deep": "yes"}),
        ] {
            assert!(runtime.command(player, "hitTest", invalid).is_err());
        }
        Ok(())
    }

    #[test]
    fn node_bounds_outline_rotated_and_mirrored_layers_and_follow_the_camera()
    -> Result<(), Box<dyn std::error::Error>> {
        let (bundle, page) = viewer_bundle()?;
        let mut runtime = Runtime::default();
        let program = runtime.load_bundle(&bundle)?;
        let player = runtime.create_player(program, 400.0, 400.0, Some(page))?;
        let fitted = runtime.command(player, "state", json!({}))?;
        let scale = fitted["camera"]["scale"][0]
            .as_f64()
            .ok_or("camera scale")?;

        let nodes = [1003, 1009, 1010, 1101, 4242].map(key);
        let bounds = runtime.command(player, "nodeBounds", json!({"nodes": nodes}))?;
        let close = |found: &Value, expected: [f64; 5]| {
            let found = ["x", "y", "width", "height", "rotation"]
                .map(|field| found[field].as_f64().unwrap_or(f64::NAN));
            found
                .iter()
                .zip(expected)
                .all(|(found, expected)| (found - expected).abs() < 1e-9)
        };
        let [dot_x, dot_y] = on_screen(&fitted, [30.0, 30.0]);
        assert!(
            close(
                &bounds[nodes[0].as_str().unwrap_or_default()],
                [dot_x, dot_y, 20.0 * scale, 20.0 * scale, 0.0]
            ),
            "{bounds}"
        );
        let [turned_x, turned_y] = on_screen(&fitted, [190.0, 140.0]);
        assert!(
            close(
                &bounds[nodes[1].as_str().unwrap_or_default()],
                [turned_x, turned_y, 40.0 * scale, 20.0 * scale, 90.0]
            ),
            "{bounds}"
        );
        let [mirrored_x, mirrored_y] = on_screen(&fitted, [120.0, 200.0]);
        assert!(
            close(
                &bounds[nodes[2].as_str().unwrap_or_default()],
                [mirrored_x, mirrored_y, 20.0 * scale, 10.0 * scale, 180.0]
            ),
            "{bounds}"
        );
        // On another page, or not in the design at all.
        assert_eq!(bounds[nodes[3].as_str().unwrap_or_default()], Value::Null);
        assert_eq!(bounds[nodes[4].as_str().unwrap_or_default()], Value::Null);
        // Hosts name layers to the runtime by their serde ids only.
        let display = NodeId::from_u128(1003).to_string();
        assert!(
            runtime
                .command(player, "nodeBounds", json!({"nodes": [display]}))
                .is_err()
        );

        runtime.command(
            player,
            "zoom",
            json!({"point": [200.0, 200.0], "factor": 2.0}),
        )?;
        let zoomed = runtime.command(player, "state", json!({}))?;
        let bounds = runtime.command(player, "nodeBounds", json!({"nodes": [key(1003)]}))?;
        let [dot_x, dot_y] = on_screen(&zoomed, [30.0, 30.0]);
        assert!(
            close(
                &bounds[nodes[0].as_str().unwrap_or_default()],
                [dot_x, dot_y, 40.0 * scale, 40.0 * scale, 0.0]
            ),
            "{bounds}"
        );
        Ok(())
    }

    #[test]
    fn set_viewport_restores_a_saved_camera_and_refuses_invalid_ones()
    -> Result<(), Box<dyn std::error::Error>> {
        let (bundle, page) = viewer_bundle()?;
        let mut runtime = Runtime::default();
        let program = runtime.load_bundle(&bundle)?;
        let player = runtime.create_player(program, 400.0, 400.0, Some(page))?;
        runtime.command(
            player,
            "zoom",
            json!({"point": [120.0, 80.0], "factor": 3.0}),
        )?;
        runtime.command(player, "pan", json!({"delta": [-40.0, 25.0]}))?;
        let saved = runtime.command(player, "state", json!({}))?["camera"].clone();

        // A reload creates a new player, fitted again; the host puts the camera back.
        let reloaded = runtime.create_player(program, 400.0, 400.0, Some(page))?;
        assert_ne!(
            runtime.command(reloaded, "state", json!({}))?["camera"],
            saved
        );
        let restored = runtime.command(reloaded, "setViewport", saved.clone())?;
        assert_eq!(restored["camera"], saved);
        assert_eq!(restored["response"]["needsRedraw"], true);
        let point = on_screen(&restored, [40.0, 40.0]);
        assert_eq!(
            runtime.command(reloaded, "hitTest", json!({"point": point, "deep": true}))?["node"],
            key(1003)
        );

        for invalid in [
            json!({}),
            json!({"center": [0.0, 0.0], "zoom": 2.0}),
            json!({"center": [0.0, 0.0], "scale": [0.0, 1.0]}),
            json!({"center": [0.0, 0.0], "scale": [-1.0, 1.0]}),
            json!({"center": [0.0], "scale": [1.0, 1.0]}),
        ] {
            assert!(runtime.command(reloaded, "setViewport", invalid).is_err());
            assert_eq!(
                runtime.command(reloaded, "state", json!({}))?["camera"],
                saved
            );
        }
        Ok(())
    }
}
