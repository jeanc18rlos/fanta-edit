#![cfg(all(target_os = "macos", feature = "metal"))]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fanta_doc::{
    AssetId, BitmapNode, CanvasNode, Color, Doc, Fill, GroupNode, NodeData, NodeId, Transform2D,
    VectorNode, Viewport,
};
use fanta_render::{
    AssetResolver, DecodedImage, InMemoryAssetResolver, RasterRenderer, RenderInputs,
    RenderMetrics, RetainedError, RetainedGpuTarget, RetainedTranslationSession,
};
use metal::foreign_types::ForeignType;
use serde_json::json;
use skia_safe::gpu::{self, SyncCpu, backend_render_targets, direct_contexts, mtl};
use skia_safe::{
    AlphaType, ColorType, Image, ImageInfo, PixelGeometry, SurfaceProps, SurfacePropsFlags,
};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const SIZE: (u32, u32) = (640, 400);
const OFFSETS: [f64; 8] = [-16.0, -8.5, -1.0, 0.0, 1.25, 8.0, 16.0, 32.0];
const PARITY_THRESHOLD: u8 = 2;

struct MetalOwner {
    context: gpu::DirectContext,
    _queue: metal::CommandQueue,
    device: metal::Device,
}

impl MetalOwner {
    fn new() -> TestResult<Self> {
        let device = metal::Device::system_default().ok_or("actual Metal device is required")?;
        let queue = device.new_command_queue();
        // The device and queue stay owned until after the DirectContext drops.
        let backend = unsafe {
            mtl::BackendContext::new(
                device.as_ptr() as mtl::Handle,
                queue.as_ptr() as mtl::Handle,
            )
        };
        let context = direct_contexts::make_metal(&backend, None)
            .ok_or("creating actual Skia Metal context failed")?;
        Ok(Self {
            context,
            _queue: queue,
            device,
        })
    }

    fn target(
        &mut self,
        size: (u32, u32),
        color: ColorType,
        properties: Option<&SurfaceProps>,
    ) -> TestResult<OwnedTarget> {
        let format = match color {
            ColorType::BGRA8888 => metal::MTLPixelFormat::BGRA8Unorm,
            ColorType::RGBA8888 => metal::MTLPixelFormat::RGBA8Unorm,
            _ => {
                return Err(
                    "test target must use an actual supported four-byte Metal texture".into(),
                );
            }
        };
        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_texture_type(metal::MTLTextureType::D2);
        descriptor.set_pixel_format(format);
        descriptor.set_width(u64::from(size.0));
        descriptor.set_height(u64::from(size.1));
        descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        let texture = self.device.new_texture(&descriptor);
        // TextureInfo retains this real texture; OwnedTarget also keeps its owner
        // until both the target Surface and BackendRenderTarget have dropped.
        let texture_info = unsafe { mtl::TextureInfo::new(texture.as_ptr() as mtl::Handle) };
        let backend = backend_render_targets::make_mtl(
            (i32::try_from(size.0)?, i32::try_from(size.1)?),
            &texture_info,
        );
        let target =
            RetainedGpuTarget::wrap_top_left(&mut self.context, &backend, color, properties)?;
        Ok(OwnedTarget {
            target,
            backend,
            _texture: texture,
        })
    }
}

struct OwnedTarget {
    target: RetainedGpuTarget,
    backend: gpu::BackendRenderTarget,
    _texture: metal::Texture,
}

impl OwnedTarget {
    fn pixels(&mut self, owner: &mut MetalOwner) -> TestResult<Vec<u8>> {
        self.target.flush_and_submit(SyncCpu::Yes)?;
        let image = self.target.image_snapshot()?;
        image_pixels(&image, &mut owner.context)
    }

    fn sentinel(&mut self) -> TestResult<()> {
        self.target
            .canvas()?
            .clear(skia_safe::Color::from_argb(255, 12, 93, 181));
        self.target.flush_and_submit(SyncCpu::Yes)?;
        Ok(())
    }

    fn compose(&mut self, image: &Image) -> TestResult<()> {
        let mut paint = skia_safe::Paint::default();
        paint.set_blend_mode(skia_safe::BlendMode::Src);
        self.target
            .canvas()?
            .draw_image(image, (0, 0), Some(&paint));
        self.target.flush_and_submit(SyncCpu::Yes)?;
        Ok(())
    }
}

fn image_pixels(image: &Image, context: &mut gpu::DirectContext) -> TestResult<Vec<u8>> {
    let size = image.dimensions();
    let info = ImageInfo::new(size, ColorType::RGBA8888, AlphaType::Premul, None);
    let row = usize::try_from(size.width)?
        .checked_mul(4)
        .ok_or("readback row overflow")?;
    let length = row
        .checked_mul(usize::try_from(size.height)?)
        .ok_or("readback size overflow")?;
    let mut pixels = vec![0; length];
    if !image.read_pixels_with_context(
        Some(context),
        &info,
        &mut pixels,
        row,
        (0, 0),
        skia_safe::image::CachingHint::Disallow,
    ) {
        return Err("actual Metal image readback failed".into());
    }
    Ok(pixels)
}

struct ProbeCase {
    doc: Doc,
    page: NodeId,
    moving: NodeId,
    resolver: Arc<dyn AssetResolver>,
    viewport: Viewport,
    scale: f32,
    label: String,
    transparent: bool,
}

fn insert(
    doc: &mut Doc,
    parent: Option<NodeId>,
    data: NodeData,
    x: f64,
    y: f64,
) -> TestResult<NodeId> {
    let mut node = CanvasNode::new(data);
    node.parent = parent;
    node.index = doc.scene.next_child_index(parent);
    node.transform = Transform2D::translation(x, y);
    let id = node.id;
    doc.scene.insert(node)?;
    Ok(id)
}

fn fixture(instance: bool, scale: f32, transparent: bool) -> TestResult<ProbeCase> {
    let mut doc = Doc::new();
    let page = insert(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            background: Some(Fill::solid(if transparent {
                Color::TRANSPARENT
            } else {
                Color::rgb(38, 27, 51)
            })),
            ..Default::default()
        }),
        0.0,
        0.0,
    )?;
    let parent = insert(
        &mut doc,
        Some(page),
        NodeData::Group(GroupNode {
            clip_size: Some([200.0, 130.0]),
            corner_radius: Some(13.0),
            background: Some(Fill::solid(Color::rgb(170, 30, 40))),
            ..Default::default()
        }),
        -100.0,
        -65.0,
    )?;
    doc.scene
        .get_mut(parent)
        .ok_or("fixture parent missing")?
        .meta = json!({"clip_content":false});
    let asset = AssetId::new();
    let mut resolver = InMemoryAssetResolver::new();
    resolver.insert(
        asset,
        DecodedImage::new(
            Arc::new(vec![
                255, 80, 0, 255, 0, 210, 80, 180, 0, 70, 255, 80, 255, 255, 255, 255,
            ]),
            2,
            2,
        ),
    );
    let data = NodeData::Bitmap(BitmapNode {
        asset,
        natural_size: [2, 2],
        local_size: [54.0, 42.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Fill,
        tint: None,
    });
    let moving = if instance {
        let master = insert(
            &mut doc,
            None,
            NodeData::Group(GroupNode {
                clip_size: Some([80.0, 60.0]),
                corner_radius: Some(7.0),
                ..Default::default()
            }),
            1000.0,
            1000.0,
        )?;
        insert(&mut doc, Some(master), data, 4.0, 2.0)?;
        let mut text = fanta_doc::TextNode::new("Aa", 60.0, 24.0);
        text.style.size_px = 17.0;
        text.style.color = Color::WHITE;
        insert(&mut doc, Some(master), NodeData::Text(text), 7.0, 32.0)?;
        let component = fanta_doc::ComponentId::new();
        doc.components.defs.insert(
            component,
            fanta_doc::ComponentDef::new(component, master, "Probe"),
        );
        insert(
            &mut doc,
            Some(parent),
            NodeData::Instance(fanta_doc::InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [80.0, 60.0],
            }),
            20.0,
            25.0,
        )?
    } else {
        insert(&mut doc, Some(parent), data, 20.0, 25.0)?
    };
    let mask = insert(
        &mut doc,
        Some(parent),
        NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 115.0, 82.0, Color::WHITE)),
        55.0,
        20.0,
    )?;
    doc.scene
        .get_mut(mask)
        .ok_or("fixture mask missing")?
        .is_mask = true;
    let above = insert(
        &mut doc,
        Some(parent),
        NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            160.0,
            100.0,
            Color::rgb(30, 80, 190),
        )),
        30.0,
        10.0,
    )?;
    doc.scene
        .get_mut(above)
        .ok_or("fixture Screen node missing")?
        .blend_mode = fanta_doc::BlendMode::Screen;
    Ok(ProbeCase {
        doc,
        page,
        moving,
        resolver: Arc::new(resolver),
        viewport: Viewport {
            center: [0.375, -1.125],
            zoom: 0.73,
        },
        scale,
        transparent,
        label: format!(
            "{}-dpi{scale}-transparent{transparent}",
            if instance { "instance-text" } else { "bitmap" }
        ),
    })
}

fn viewport(case: &ProbeCase) -> Viewport {
    Viewport {
        center: case.viewport.center,
        zoom: case.viewport.zoom * f64::from(case.scale),
    }
}

fn renderer(case: &ProbeCase) -> TestResult<RasterRenderer> {
    let mut renderer = RasterRenderer::new(SIZE.0, SIZE.1)?;
    renderer.background = if case.transparent {
        Color::TRANSPARENT
    } else {
        Color::rgb(245, 245, 245)
    };
    renderer.set_pixel_snap_pan(true);
    renderer.set_asset_resolver(Arc::clone(&case.resolver));
    Ok(renderer)
}

fn complete(metrics: &RenderMetrics) -> TestResult<()> {
    if metrics.incomplete_artwork || metrics.effect_failed || metrics.non_artwork_content {
        return Err(
            "rendered fixture contains incomplete artwork, failed effects or placeholders".into(),
        );
    }
    Ok(())
}

fn prepare(
    renderer: &mut RasterRenderer,
    target: &mut OwnedTarget,
    case: &ProbeCase,
) -> Result<RetainedTranslationSession, RetainedError> {
    RetainedTranslationSession::prepare_for_target(
        renderer,
        &mut target.target,
        &case.doc.scene,
        case.page,
        case.moving,
        &viewport(case),
        &RenderInputs::for_doc(&case.doc),
        Some(case.resolver.as_ref()),
        0,
    )
}

fn render(
    session: &mut RetainedTranslationSession,
    renderer: &mut RasterRenderer,
    target: &mut OwnedTarget,
    case: &ProbeCase,
) -> Result<fanta_render::RetainedFrame, RetainedError> {
    session.render_for_target(
        renderer,
        &mut target.target,
        &case.doc.scene,
        &viewport(case),
        &RenderInputs::for_doc(&case.doc),
        Some(case.resolver.as_ref()),
        0,
    )
}

fn move_to(case: &mut ProbeCase, original: Transform2D, offset: f64) -> TestResult<()> {
    case.doc.scene.set_transform(
        case.moving,
        original.then(&Transform2D::translation(offset, offset * 0.375)),
    )?;
    case.doc
        .components
        .bump_preview_for_node(&case.doc.scene, case.moving);
    Ok(())
}

fn full_frame(
    renderer: &mut RasterRenderer,
    target: &mut OwnedTarget,
    owner: &mut MetalOwner,
    case: &ProbeCase,
) -> TestResult<Vec<u8>> {
    let metrics = renderer.render_to_canvas(
        target.target.canvas()?,
        SIZE.0,
        SIZE.1,
        &case.doc.scene,
        &viewport(case),
        Some(case.page),
        &RenderInputs::for_doc(&case.doc),
    );
    complete(&metrics)?;
    target.pixels(owner)
}

fn assert_parity(expected: &[u8], actual: &[u8], label: &str) -> TestResult<u8> {
    assert_eq!(expected.len(), actual.len(), "{label}: output length");
    let maximum = expected
        .iter()
        .zip(actual)
        .map(|(left, right)| left.abs_diff(*right))
        .max()
        .ok_or("empty parity output")?;
    assert!(
        maximum <= PARITY_THRESHOLD,
        "{label}: maximum channel error {maximum}, threshold {PARITY_THRESHOLD}"
    );
    Ok(maximum)
}

fn matrix_case(mut case: ProbeCase) -> TestResult<()> {
    let before = case.doc.to_json_pretty()?;
    let original = case
        .doc
        .scene
        .get(case.moving)
        .ok_or("moving node missing")?
        .transform;
    let mut normal_owner = MetalOwner::new()?;
    let mut retained_owner = MetalOwner::new()?;
    let mut normal_target = normal_owner.target(SIZE, ColorType::BGRA8888, None)?;
    let mut retained_target = retained_owner.target(SIZE, ColorType::BGRA8888, None)?;
    let mut normal_renderer = renderer(&case)?;
    let mut retained_renderer = renderer(&case)?;
    retained_target.sentinel()?;
    let sentinel = retained_target.pixels(&mut retained_owner)?;
    let mut session = prepare(&mut retained_renderer, &mut retained_target, &case)?;
    assert_eq!(
        retained_target.pixels(&mut retained_owner)?,
        sentinel,
        "prepare changed caller target"
    );
    complete(&session.build_metrics().below)?;
    complete(&session.build_metrics().above)?;
    let mut frames = 0;
    let mut maximum_error = 0;
    for offset in OFFSETS {
        move_to(&mut case, original, offset)?;
        let expected = full_frame(
            &mut normal_renderer,
            &mut normal_target,
            &mut normal_owner,
            &case,
        )?;
        let frame = render(
            &mut session,
            &mut retained_renderer,
            &mut retained_target,
            &case,
        )?;
        complete(&frame.metrics.middle)?;
        assert_eq!(
            retained_target.pixels(&mut retained_owner)?,
            sentinel,
            "render changed caller target before composition"
        );
        retained_target.compose(&frame.image)?;
        let actual = retained_target.pixels(&mut retained_owner)?;
        let background = expected.get(..4).ok_or("empty full-frame output")?;
        if case.transparent {
            assert_eq!(background, [0, 0, 0, 0], "transparent fixture backdrop");
        }
        assert!(
            expected
                .chunks_exact(4)
                .filter(|pixel| *pixel != background)
                .count()
                > 100,
            "fixture has no meaningful artwork"
        );
        maximum_error = maximum_error.max(assert_parity(
            &expected,
            &actual,
            &format!("{} offset {offset}", case.label),
        )?);
        retained_target.sentinel()?;
        frames += 1;
    }
    assert_eq!(frames, 8);
    println!(
        "{}: {frames} real GPU frames, maximum channel error {maximum_error}",
        case.label
    );
    case.doc.scene.set_transform(case.moving, original)?;
    assert_eq!(
        case.doc.to_json_pretty()?,
        before,
        "matrix changed authored document"
    );
    Ok(())
}

#[test]
#[ignore = "requires real macOS Metal; run with --features metal --test retained_gpu -- --ignored --test-threads=1"]
fn retained_gpu_matches_full_metal_for_all_forty_fixture_frames() -> TestResult<()> {
    metal::objc::rc::autoreleasepool(|| {
        for (instance, scale, transparent) in [
            (false, 1.0, false),
            (false, 2.0, false),
            (true, 1.0, false),
            (true, 2.0, false),
            (false, 1.0, true),
        ] {
            matrix_case(fixture(instance, scale, transparent)?)?;
        }
        Ok(())
    })
}

#[test]
#[ignore = "requires real macOS Metal and explicit GPU test execution"]
fn retained_gpu_preserves_caller_state_and_prior_images_across_frames_and_teardown()
-> TestResult<()> {
    metal::objc::rc::autoreleasepool(|| {
        let mut case = fixture(true, 2.0, false)?;
        let mut owner = MetalOwner::new()?;
        let mut target = owner.target(SIZE, ColorType::BGRA8888, None)?;
        let mut replacement = owner.target(SIZE, ColorType::BGRA8888, None)?;
        target.sentinel()?;
        let untouched = target.pixels(&mut owner)?;
        let canvas = target.target.canvas()?;
        canvas.save();
        canvas.translate((9.25, -3.5));
        canvas.clip_rect(skia_safe::Rect::from_xywh(12., 14., 80., 60.), None, true);
        let expected_matrix = canvas.local_to_device();
        let expected_clip = canvas.device_clip_bounds();
        let expected_save_count = canvas.save_count();
        let mut renderer = renderer(&case)?;
        let mut session = prepare(&mut renderer, &mut target, &case)?;
        let first = render(&mut session, &mut renderer, &mut target, &case)?;
        let first_pixels = image_pixels(&first.image, &mut owner.context)?;
        assert_eq!(target.pixels(&mut owner)?, untouched);
        let canvas = target.target.canvas()?;
        assert_eq!(canvas.local_to_device(), expected_matrix);
        assert_eq!(canvas.device_clip_bounds(), expected_clip);
        assert_eq!(canvas.save_count(), expected_save_count);
        let original = case
            .doc
            .scene
            .get(case.moving)
            .ok_or("moving node")?
            .transform;
        move_to(&mut case, original, 32.)?;
        replacement.sentinel()?;
        let replacement_before = replacement.pixels(&mut owner)?;
        let second = render(&mut session, &mut renderer, &mut replacement, &case)?;
        let second_pixels = image_pixels(&second.image, &mut owner.context)?;
        assert_ne!(
            first_pixels, second_pixels,
            "the moved fixture must change pixels"
        );
        assert_eq!(replacement.pixels(&mut owner)?, replacement_before);
        assert_eq!(
            image_pixels(&first.image, &mut owner.context)?,
            first_pixels,
            "later frame mutated a retained snapshot"
        );
        case.doc
            .scene
            .get_mut(case.moving)
            .ok_or("moving node")?
            .name = "invalid non-translation edit".into();
        let rejected = render(&mut session, &mut renderer, &mut replacement, &case);
        assert!(matches!(rejected, Err(RetainedError::ChangedScene)));
        assert_eq!(
            image_pixels(&first.image, &mut owner.context)?,
            first_pixels
        );
        assert_eq!(
            image_pixels(&second.image, &mut owner.context)?,
            second_pixels
        );
        assert_eq!(replacement.pixels(&mut owner)?, replacement_before);
        assert!(matches!(
            render(&mut session, &mut renderer, &mut replacement, &case),
            Err(RetainedError::Disabled)
        ));
        drop(session);
        drop(target);
        assert_eq!(
            image_pixels(&first.image, &mut owner.context)?,
            first_pixels,
            "snapshot must retain its storage after session teardown"
        );
        assert_eq!(
            image_pixels(&second.image, &mut owner.context)?,
            second_pixels
        );
        Ok(())
    })
}

#[derive(Clone, Copy, Debug)]
enum Mismatch {
    Context,
    Size,
    Format,
    Properties,
}

#[test]
#[ignore = "requires real macOS Metal and explicit GPU test execution"]
fn retained_gpu_rejects_incompatible_targets_before_any_output_mutation() -> TestResult<()> {
    metal::objc::rc::autoreleasepool(|| {
        for mismatch in [
            Mismatch::Context,
            Mismatch::Size,
            Mismatch::Format,
            Mismatch::Properties,
        ] {
            let mut case = fixture(false, 1., false)?;
            let mut owner = MetalOwner::new()?;
            let mut other_owner = MetalOwner::new()?;
            let mut target = owner.target(SIZE, ColorType::BGRA8888, None)?;
            target.sentinel()?;
            let target_before = target.pixels(&mut owner)?;
            let mut renderer = renderer(&case)?;
            let mut session = prepare(&mut renderer, &mut target, &case)?;
            let accepted = render(&mut session, &mut renderer, &mut target, &case)?;
            let accepted_pixels = image_pixels(&accepted.image, &mut owner.context)?;
            let properties = SurfaceProps::new(
                SurfacePropsFlags::USE_DEVICE_INDEPENDENT_FONTS,
                PixelGeometry::RGBH,
            );
            let mut incompatible = match mismatch {
                Mismatch::Context => other_owner.target(SIZE, ColorType::BGRA8888, None)?,
                Mismatch::Size => owner.target((SIZE.0 + 1, SIZE.1), ColorType::BGRA8888, None)?,
                Mismatch::Format => owner.target(SIZE, ColorType::RGBA8888, None)?,
                Mismatch::Properties => {
                    owner.target(SIZE, ColorType::BGRA8888, Some(&properties))?
                }
            };
            incompatible.sentinel()?;
            let incompatible_before = match mismatch {
                Mismatch::Context => incompatible.pixels(&mut other_owner)?,
                _ => incompatible.pixels(&mut owner)?,
            };
            let original = case
                .doc
                .scene
                .get(case.moving)
                .ok_or("moving node")?
                .transform;
            move_to(&mut case, original, 8.0)?;
            let rejected = render(&mut session, &mut renderer, &mut incompatible, &case);
            assert!(
                matches!(rejected, Err(RetainedError::ChangedBackend)),
                "{mismatch:?}"
            );
            let incompatible_after = match mismatch {
                Mismatch::Context => incompatible.pixels(&mut other_owner)?,
                _ => incompatible.pixels(&mut owner)?,
            };
            assert_eq!(
                incompatible_after, incompatible_before,
                "{mismatch:?} changed target"
            );
            assert_eq!(target.pixels(&mut owner)?, target_before);
            assert_eq!(
                image_pixels(&accepted.image, &mut owner.context)?,
                accepted_pixels
            );
            assert!(matches!(
                render(&mut session, &mut renderer, &mut target, &case),
                Err(RetainedError::Disabled)
            ));
        }
        Ok(())
    })
}

#[test]
#[ignore = "requires real macOS Metal and explicit GPU test execution"]
fn retained_gpu_rejects_lost_context_at_every_target_and_session_boundary() -> TestResult<()> {
    metal::objc::rc::autoreleasepool(|| {
        let case = fixture(false, 1., false)?;
        let mut owner = MetalOwner::new()?;
        let mut target = owner.target(SIZE, ColorType::BGRA8888, None)?;
        let mut renderer = renderer(&case)?;
        let mut session = prepare(&mut renderer, &mut target, &case)?;
        let frame = render(&mut session, &mut renderer, &mut target, &case)?;
        let _pixels_before_loss = image_pixels(&frame.image, &mut owner.context)?;
        owner.context.abandon();
        assert!(matches!(
            target.target.canvas(),
            Err(RetainedError::BackendLost)
        ));
        assert!(matches!(
            target.target.image_snapshot(),
            Err(RetainedError::BackendLost)
        ));
        assert!(matches!(
            target.target.flush_and_submit(SyncCpu::Yes),
            Err(RetainedError::BackendLost)
        ));
        assert!(matches!(
            prepare(&mut renderer, &mut target, &case),
            Err(RetainedError::BackendLost)
        ));
        assert!(matches!(
            render(&mut session, &mut renderer, &mut target, &case),
            Err(RetainedError::BackendLost)
        ));
        assert!(matches!(
            RetainedGpuTarget::wrap_top_left(
                &mut owner.context,
                &target.backend,
                ColorType::BGRA8888,
                None
            ),
            Err(RetainedError::BackendLost)
        ));
        assert!(matches!(
            render(&mut session, &mut renderer, &mut target, &case),
            Err(RetainedError::Disabled)
        ));
        // Images cannot be read through a lost context. Keeping them alive until
        // after session/target teardown exercises release ownership, not parity.
        drop(session);
        drop(target);
        drop(frame);
        Ok(())
    })
}

#[test]
#[ignore = "requires real macOS Metal and explicit GPU test execution"]
fn retained_gpu_refuses_invalid_wrap_and_renderer_size_without_touching_target() -> TestResult<()> {
    metal::objc::rc::autoreleasepool(|| {
        let case = fixture(false, 1., false)?;
        let mut owner = MetalOwner::new()?;
        let mut target = owner.target(SIZE, ColorType::BGRA8888, None)?;
        target.sentinel()?;
        let before = target.pixels(&mut owner)?;
        assert!(matches!(
            RetainedGpuTarget::wrap_top_left(
                &mut owner.context,
                &target.backend,
                ColorType::Alpha8,
                None
            ),
            Err(RetainedError::ChangedBackend)
        ));
        let invalid = backend_render_targets::make_mtl((0, 0), &mtl::TextureInfo::default());
        assert!(matches!(
            RetainedGpuTarget::wrap_top_left(
                &mut owner.context,
                &invalid,
                ColorType::BGRA8888,
                None
            ),
            Err(RetainedError::ChangedBackend)
        ));
        let mut wrong_size = RasterRenderer::new(SIZE.0 - 1, SIZE.1)?;
        assert!(matches!(
            prepare(&mut wrong_size, &mut target, &case),
            Err(RetainedError::ChangedBackend)
        ));
        assert_eq!(target.pixels(&mut owner)?, before);
        Ok(())
    })
}

#[test]
#[ignore = "requires real macOS Metal and explicit GPU test execution"]
fn retained_gpu_and_cpu_sessions_reject_cross_backend_use() -> TestResult<()> {
    metal::objc::rc::autoreleasepool(|| {
        let case = fixture(false, 1., false)?;
        let mut owner = MetalOwner::new()?;
        let mut target = owner.target(SIZE, ColorType::BGRA8888, None)?;
        target.sentinel()?;
        let before = target.pixels(&mut owner)?;
        let mut renderer = renderer(&case)?;
        let mut gpu_session = prepare(&mut renderer, &mut target, &case)?;
        assert!(matches!(
            gpu_session.render(
                &mut renderer,
                &case.doc.scene,
                &viewport(&case),
                &RenderInputs::for_doc(&case.doc),
                Some(case.resolver.as_ref()),
                0
            ),
            Err(RetainedError::ChangedBackend)
        ));
        let mut cpu_session = RetainedTranslationSession::prepare(
            &mut renderer,
            &case.doc.scene,
            case.page,
            case.moving,
            &viewport(&case),
            &RenderInputs::for_doc(&case.doc),
            Some(case.resolver.as_ref()),
            0,
        )?;
        assert!(matches!(
            render(&mut cpu_session, &mut renderer, &mut target, &case),
            Err(RetainedError::ChangedBackend)
        ));
        assert_eq!(target.pixels(&mut owner)?, before);
        Ok(())
    })
}

thread_local! {
    static RESOLVER_CONTEXT: RefCell<Option<gpu::DirectContext>> = const { RefCell::new(None) };
}

struct ResolverContext;

impl ResolverContext {
    fn install(context: &gpu::DirectContext) -> TestResult<Self> {
        RESOLVER_CONTEXT.with_borrow_mut(|current| {
            if current.is_some() {
                return Err("resolver context already installed on this thread".into());
            }
            *current = Some(context.clone());
            Ok(Self)
        })
    }
}

impl Drop for ResolverContext {
    fn drop(&mut self) {
        RESOLVER_CONTEXT.with_borrow_mut(|current| *current = None);
    }
}

struct AbandoningResolver {
    source: Arc<dyn AssetResolver>,
    calls: AtomicUsize,
}

impl AssetResolver for AbandoningResolver {
    fn resolve(&self, asset: AssetId) -> Option<DecodedImage> {
        let image = self.source.resolve(asset);
        RESOLVER_CONTEXT.with_borrow_mut(|context| {
            if let Some(context) = context {
                context.abandon();
                self.calls.fetch_add(1, Ordering::SeqCst);
            }
        });
        image
    }
}

#[test]
#[ignore = "requires real macOS Metal and explicit GPU test execution"]
fn retained_gpu_rechecks_context_after_caller_asset_resolution() -> TestResult<()> {
    metal::objc::rc::autoreleasepool(|| {
        let mut case = fixture(false, 1., false)?;
        let mut owner = MetalOwner::new()?;
        let mut target = owner.target(SIZE, ColorType::BGRA8888, None)?;
        target.sentinel()?;
        let mut renderer = renderer(&case)?;
        let mut session = prepare(&mut renderer, &mut target, &case)?;
        let accepted = render(&mut session, &mut renderer, &mut target, &case)?;
        let _accepted_pixels = image_pixels(&accepted.image, &mut owner.context)?;
        let resolver = Arc::new(AbandoningResolver {
            source: Arc::clone(&case.resolver),
            calls: AtomicUsize::new(0),
        });
        case.resolver = resolver.clone();
        let original = case
            .doc
            .scene
            .get(case.moving)
            .ok_or("moving node")?
            .transform;
        move_to(&mut case, original, 8.)?;
        // The Send+Sync resolver accesses a clone only on this test's render
        // thread. The guard removes it before the owning Metal context drops.
        let resolver_context = ResolverContext::install(&owner.context)?;
        assert!(matches!(
            render(&mut session, &mut renderer, &mut target, &case),
            Err(RetainedError::BackendLost)
        ));
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            target.target.canvas(),
            Err(RetainedError::BackendLost)
        ));
        assert!(matches!(
            render(&mut session, &mut renderer, &mut target, &case),
            Err(RetainedError::Disabled)
        ));
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        drop(resolver_context);
        // A lost context cannot support pixel readback; no post-loss pixel
        // preservation claim is inferred from this error-path check.
        drop(accepted);
        Ok(())
    })
}
