use super::{Bounds, Canvas, ImageFillMods, RenderCtx, draw_image_cached};
use fanta_doc::{BlendMode, VideoFill};

pub(crate) fn draw_video_fill(
    canvas: &Canvas,
    path: &skia_safe::Path,
    bounds: Bounds,
    video: &VideoFill,
    opacity: f32,
    blend: BlendMode,
    ctx: &mut RenderCtx,
) -> bool {
    let opacity = (opacity * ctx.paint_alpha).clamp(0.0, 1.0);
    let mods = ImageFillMods {
        scale: video.scale,
        rotation: video.rotation,
        blend,
        adjust: video.adjust,
    };
    let local_size = [bounds.width(), bounds.height()];
    let crop = video.crop.as_deref().copied();
    let frame = ctx
        .inputs
        .video_fill_frames
        .and_then(|frames| frames.get(&video.asset));
    if frame.is_some() {
        ctx.layer_volatile = true;
    }
    let resolver = ctx.resolver;

    canvas.save();
    canvas.clip_path(path, None, true);
    canvas.translate((bounds.min_x as f32, bounds.min_y as f32));
    let drawn = if let Some(frame) = frame {
        crate::image::blit_sk_image(
            canvas,
            frame,
            frame.width() as u32,
            frame.height() as u32,
            local_size,
            crop,
            video.mode,
            None,
            opacity,
            mods,
        )
    } else if let Some(poster) = video.poster {
        draw_image_cached(
            canvas,
            ctx.cache,
            poster,
            || resolver.and_then(|resolver| resolver.resolve(poster)),
            local_size,
            crop,
            video.mode,
            None,
            opacity,
            mods,
        )
    } else {
        false
    };
    canvas.restore();
    if !drawn {
        ctx.layer_volatile = true;
    }
    drawn
}
