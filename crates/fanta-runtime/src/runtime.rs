#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use fanta_doc::{
    AnimationClipId, AssetId, BoundProp, MotionProperty, NodeData, NodeId, ResolvedVarValue,
    TextStyle, TextStyleRun, VarValue,
};
use fanta_format::runtime::{RuntimeProgram, SourceBundle};
use fanta_present::PlaybackSession;
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
    pub fn compile_source(&mut self, source: &SourceBundle) -> Result<u32, RuntimeError> {
        let size = source
            .files
            .values()
            .map(|bytes| bytes.len() as u64)
            .chain(source.fonts.iter().map(|font| font.bytes.len() as u64))
            .try_fold(0_u64, u64::checked_add)
            .ok_or_else(|| RuntimeError("Source bundle size overflow".into()))?;
        if size > MAX_PROGRAM_INPUT_BYTES {
            return Err(RuntimeError(
                "Source bundle exceeds the 64 MiB expanded input budget".into(),
            ));
        }
        let program = RuntimeProgram::compile(source).map_err(RuntimeError::from_error)?;
        self.insert_program(program)
    }

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

    pub fn to_bundle(&self, handle: u32) -> Result<Vec<u8>, RuntimeError> {
        self.programs
            .get(&handle)
            .ok_or_else(|| RuntimeError("Program was disposed or does not exist".into()))?
            .compiled
            .to_bundle()
            .map_err(RuntimeError::from_error)
    }

    pub fn create_player(
        &mut self,
        program_handle: u32,
        width: f64,
        height: f64,
        start: Option<NodeId>,
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
        let session = program
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
        let handle = self.allocate_handle()?;
        let mut player = Player {
            session,
            renderer: None,
            text: program.text.clone(),
            program,
            paused: false,
            validated_text: BTreeSet::new(),
        };
        player.validate_effective_text()?;
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
            "fit" => response = self.session.fit_view(),
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
                let mode = arguments.get("mode").cloned().unwrap_or(json!("once"));
                self.session
                    .play_clip(
                        clip,
                        scope,
                        serde_json::from_value(mode).map_err(RuntimeError::from_error)?,
                    )
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
        let camera = matches!(command, "pan" | "zoom" | "fit");
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
            json!({"response":response,"state":self.session.playback_state(),"schedule":self.session.schedule(),"paused":self.paused,"openUrl":url,"events":self.session.take_playback_events(),"occurrences": if command == "state" {Some(self.session.occurrences())} else {None}, "inspection":inspection,"viewport":self.session.viewport()}),
        )
    }
}
