use std::cell::RefCell;
use std::collections::BTreeMap;

use fanta_format::runtime::{RuntimeFont, SourceBundle};
use fanta_present::FitMode;
use fanta_runtime::{Runtime, RuntimeError};
use serde_json::{Value, json};
use skia_safe::{Surface, gpu};

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::default());
    static BUFFERS: RefCell<BTreeMap<u32, Vec<u8>>> = const {RefCell::new(BTreeMap::new())};
    static GRAPHICS: RefCell<BTreeMap<u32, Graphics>> = const {RefCell::new(BTreeMap::new())};
}

struct Graphics {
    context: gpu::DirectContext,
    surface: Surface,
    width: u32,
    height: u32,
}

fn error(message: impl Into<String>) -> RuntimeError {
    RuntimeError(message.into())
}

fn buffer(bytes: Vec<u8>) -> Result<u32, RuntimeError> {
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(error("Bridge buffer exceeds 128 MiB"));
    }
    let mut bytes = bytes;
    if bytes.capacity() == 0 {
        bytes
            .try_reserve_exact(1)
            .map_err(|_| error("Bridge buffer allocation failed"))?;
    }
    let pointer = bytes.as_mut_ptr() as usize as u32;
    BUFFERS.with(|buffers| {
        let mut buffers = buffers
            .try_borrow_mut()
            .map_err(|_| error("Reentrant buffer allocation"))?;
        let held: usize = buffers.values().map(Vec::len).sum();
        if held.saturating_add(bytes.len()) > 256 * 1024 * 1024 {
            return Err(error("Bridge buffers exceed 256 MiB"));
        }
        buffers.insert(pointer, bytes);
        Ok(pointer)
    })
}

fn read_buffer(pointer: u32) -> Result<Vec<u8>, RuntimeError> {
    BUFFERS.with(|buffers| {
        buffers
            .try_borrow()
            .map_err(|_| error("Reentrant buffer read"))?
            .get(&pointer)
            .cloned()
            .ok_or_else(|| error("Unknown bridge buffer"))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn fanta_alloc(length: u32) -> u32 {
    if length > 128 * 1024 * 1024 {
        return 0;
    }
    let permitted = BUFFERS.with(|buffers| {
        buffers
            .borrow()
            .values()
            .map(Vec::len)
            .sum::<usize>()
            .saturating_add(length as usize)
            <= 256 * 1024 * 1024
    });
    if !permitted {
        return 0;
    }
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact((length as usize).max(1)).is_err() {
        return 0;
    }
    bytes.resize(length as usize, 0);
    buffer(bytes).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn fanta_buffer_len(pointer: u32) -> u32 {
    BUFFERS.with(|buffers| {
        buffers
            .borrow()
            .get(&pointer)
            .map_or(0, |bytes| bytes.len() as u32)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn fanta_free(pointer: u32) {
    BUFFERS.with(|buffers| {
        buffers.borrow_mut().remove(&pointer);
    });
}

fn number(arguments: &Value, name: &str) -> Result<u32, RuntimeError> {
    arguments
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| error(format!("Invalid argument {name}")))
}

fn positive(arguments: &Value, name: &str) -> Result<f64, RuntimeError> {
    arguments
        .get(name)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| error(format!("{name} must be finite and positive")))
}

fn optional<T: serde::de::DeserializeOwned>(
    arguments: &Value,
    name: &str,
    default: T,
) -> Result<T, RuntimeError> {
    match arguments.get(name) {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| error(format!("Invalid argument {name}"))),
        None => Ok(default),
    }
}

fn source(arguments: &Value) -> Result<SourceBundle, RuntimeError> {
    let mut files = BTreeMap::new();
    for (path, value) in arguments
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| error("Missing source files"))?
    {
        let pointer = value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| error("Invalid source buffer"))?;
        files.insert(path.clone(), read_buffer(pointer)?);
    }
    let mut fonts = Vec::new();
    if let Some(font_values) = arguments.get("fonts") {
        let registrations = font_values
            .as_array()
            .ok_or_else(|| error("fonts must be an array"))?;
        for registration in registrations {
            let bytes = read_buffer(number(registration, "bytes")?)?;
            let family = registration
                .get("family")
                .and_then(Value::as_str)
                .ok_or_else(|| error("Missing font family"))?
                .to_owned();
            fonts.push(RuntimeFont {
                family,
                weight: optional(registration, "weight", 400_u16)?,
                italic: optional(registration, "italic", false)?,
                face_index: optional(registration, "faceIndex", 0_u32)?,
                bytes,
            });
        }
    }
    let font_substitutions = serde_json::from_value(
        arguments
            .get("fontSubstitutions")
            .cloned()
            .unwrap_or(json!({})),
    )
    .map_err(|failure| error(failure.to_string()))?;
    Ok(SourceBundle {
        files,
        fonts,
        font_substitutions,
    })
}

impl Graphics {
    fn new(width: u32, height: u32) -> Result<Self, RuntimeError> {
        let interface = gpu::gl::Interface::new_native()
            .ok_or_else(|| error("Unable to initialize WebGL2 Skia interface"))?;
        let mut context = gpu::direct_contexts::make_gl(interface, None)
            .ok_or_else(|| error("Unable to initialize Skia GPU context"))?;
        context.set_resource_cache_limit(128 * 1024 * 1024);
        let surface = Self::make_surface(&mut context, width, height)?;
        Ok(Self {
            context,
            surface,
            width,
            height,
        })
    }

    fn make_surface(
        context: &mut gpu::DirectContext,
        width: u32,
        height: u32,
    ) -> Result<Surface, RuntimeError> {
        let size = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| error("Presentation surface dimensions overflow"))?;
        if size > 128 * 1024 * 1024 {
            return Err(error("Presentation surface exceeds 128 MiB"));
        }
        let dimensions = (
            i32::try_from(width).map_err(|_| error("Surface width is too large"))?,
            i32::try_from(height).map_err(|_| error("Surface height is too large"))?,
        );
        let info = gpu::gl::FramebufferInfo {
            fboid: 0,
            format: gpu::gl::Format::RGBA8.into(),
            protected: gpu::Protected::No,
        };
        let target = gpu::backend_render_targets::make_gl(dimensions, 0, 8, info);
        gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            gpu::SurfaceOrigin::BottomLeft,
            skia_safe::ColorType::RGBA8888,
            None,
            None,
        )
        .ok_or_else(|| error("Unable to allocate WebGL presentation surface"))
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), RuntimeError> {
        if self.width != width || self.height != height {
            self.surface = Self::make_surface(&mut self.context, width, height)?;
            self.width = width;
            self.height = height;
        }
        Ok(())
    }
}

fn dispatch(arguments: Value) -> Result<Value, RuntimeError> {
    let operation = arguments
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| error("Missing bridge operation"))?;
    match operation {
        "compileSource" => {
            let source = source(&arguments)?;
            RUNTIME.with(|runtime| {
                let mut runtime = runtime
                    .try_borrow_mut()
                    .map_err(|_| error("Reentrant runtime call"))?;
                let handle = runtime.compile_source(&source)?;
                runtime.program_info(handle)
            })
        }
        "loadBundle" => {
            let bytes = read_buffer(number(&arguments, "bytes")?)?;
            RUNTIME.with(|runtime| {
                let mut runtime = runtime
                    .try_borrow_mut()
                    .map_err(|_| error("Reentrant runtime call"))?;
                let handle = runtime.load_bundle(&bytes)?;
                runtime.program_info(handle)
            })
        }
        "toBundle" => {
            let bytes = RUNTIME.with(|runtime| {
                runtime
                    .try_borrow()
                    .map_err(|_| error("Reentrant runtime call"))?
                    .to_bundle(number(&arguments, "program")?)
            })?;
            Ok(json!({"buffer":buffer(bytes)?}))
        }
        "inspectProgram" => RUNTIME.with(|runtime| {
            runtime
                .try_borrow()
                .map_err(|_| error("Reentrant runtime call"))?
                .inspect_program(number(&arguments, "program")?)
        }),
        "createPlayer" => {
            if arguments.get("fit").is_some_and(|fit| !fit.is_string()) {
                return Err(error("Invalid argument fit"));
            }
            let fit = optional(&arguments, "fit", FitMode::Contain)?;
            let start = arguments
                .get("start")
                .filter(|value| !value.is_null())
                .map(|value| {
                    serde_json::from_value(value.clone())
                        .map_err(|failure| error(failure.to_string()))
                })
                .transpose()?;
            let handle = RUNTIME.with(|runtime| {
                runtime
                    .try_borrow_mut()
                    .map_err(|_| error("Reentrant runtime call"))?
                    .create_player_with_fit(
                        number(&arguments, "program")?,
                        positive(&arguments, "width")?,
                        positive(&arguments, "height")?,
                        start,
                        fit,
                    )
            })?;
            Ok(json!({"handle":handle}))
        }
        "command" => RUNTIME.with(|runtime| {
            runtime
                .try_borrow_mut()
                .map_err(|_| error("Reentrant runtime call"))?
                .command(
                    number(&arguments, "player")?,
                    arguments
                        .get("command")
                        .and_then(Value::as_str)
                        .ok_or_else(|| error("Missing player command"))?,
                    arguments.get("arguments").cloned().unwrap_or(json!({})),
                )
        }),
        "render" => {
            let handle = number(&arguments, "player")?;
            let width = number(&arguments, "width")?;
            let height = number(&arguments, "height")?;
            let scale = positive(&arguments, "scale")?;
            GRAPHICS.with(|graphics| {
                let mut graphics = graphics
                    .try_borrow_mut()
                    .map_err(|_| error("Reentrant render call"))?;
                if let std::collections::btree_map::Entry::Vacant(entry) = graphics.entry(handle) {
                    entry.insert(Graphics::new(width, height)?);
                }
                let graphics = graphics
                    .get_mut(&handle)
                    .ok_or_else(|| error("Unknown GPU player"))?;
                graphics.resize(width, height)?;
                RUNTIME.with(|runtime| {
                    let mut runtime = runtime
                        .try_borrow_mut()
                        .map_err(|_| error("Reentrant runtime call"))?;
                    let player = runtime.player_mut(handle)?;
                    player.prepare_renderer()?;
                    let text = player.text.clone();
                    let metrics=text.with(|| {
                        let renderer = player.renderer.as_mut().ok_or_else(|| error("Missing presentation renderer"))?;
                        fanta_present::gpu::GpuPresenter::default().render_playback(
                            &mut player.session,
                            renderer,
                            graphics.surface.canvas(),
                            width,
                            height,
                            scale,
                        ).map_err(|failure| error(failure.to_string()))
                    })
                    .map_err(|failure| error(failure.to_string()))?;
                    graphics
                        .context
                        .flush_and_submit_surface(&mut graphics.surface, None);
                    Ok(json!({"wholeFrameReadbacks":metrics.whole_frame_readbacks,"frameMicros":metrics.render.frame_micros,"renderedLayers":metrics.rendered_layers,"peakSurfaceBytes":metrics.peak_surface_bytes}))
                })
            })
        }
        "releaseGraphics" => {
            let handle = number(&arguments, "player")?;
            GRAPHICS.with(|graphics| {
                if let Some(released) = graphics.borrow_mut().get_mut(&handle) {
                    released.context.abandon();
                }
            });
            RUNTIME.with(|runtime| {
                runtime.borrow_mut().player_mut(handle)?.renderer = None;
                Ok(())
            })?;
            GRAPHICS.with(|graphics| {
                graphics.borrow_mut().remove(&handle);
            });
            Ok(json!(null))
        }
        "disposePlayer" => {
            let handle = number(&arguments, "player")?;
            let disposed = RUNTIME.with(|runtime| runtime.borrow_mut().dispose_player(handle));
            GRAPHICS.with(|graphics| {
                graphics.borrow_mut().remove(&handle);
            });
            Ok(json!(disposed))
        }
        "disposeProgram" => RUNTIME.with(|runtime| {
            Ok(json!(
                runtime
                    .borrow_mut()
                    .dispose_program(number(&arguments, "program")?)
            ))
        }),
        _ => Err(error(format!("Unknown bridge operation {operation}"))),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn fanta_request(pointer: u32) -> u32 {
    let result = read_buffer(pointer)
        .and_then(|bytes| {
            serde_json::from_slice(&bytes).map_err(|failure| error(failure.to_string()))
        })
        .and_then(dispatch);
    let response = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(failure) => {
            json!({"ok":false,"error":{"code":"RuntimeError","message":failure.to_string()}})
        }
    };
    match serde_json::to_vec(&response)
        .map_err(|failure| error(failure.to_string()))
        .and_then(buffer)
    {
        Ok(pointer) => pointer,
        Err(_) => 0,
    }
}
