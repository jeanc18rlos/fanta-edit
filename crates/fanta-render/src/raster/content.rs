//! [`paint_node_content`] — paint a single node's *own* content (no child
//! recursion), shared by the live-scene walk and the transient instance walk
//! so a component instance renders pixel-identically to the same subtree
//! placed directly in the scene.
use super::{
    Bounds, Canvas, CanvasNode, Color, Fill, NodeData, NodeId, Rect, RenderCtx, bounds_to_f32,
    draw_image_cached, draw_placeholder, draw_text_node, draw_vector, fill_to_paint,
    rounded_rect_path, stroke_box_path,
};

/// This node's live playback position (0..=1) from the app-playback→render seam,
/// or `None` when it isn't the node currently playing (or for a transient clone
/// with no scene id). Drives the audio playhead + the video progress bar.
fn media_progress(ctx: &RenderCtx, scene_id: Option<NodeId>) -> Option<f32> {
    let id = scene_id?;
    ctx.inputs.playback?.get(&id).map(|p| p.progress)
}

#[derive(Default)]
pub(crate) struct ContentPaintState {
    pub(crate) restore_child_clip: bool,
}

/// Paint a single node's *own* content — no child recursion. Shared by the
/// live-scene walk ([`render_node`]) and the transient instance-subtree walk
/// ([`render_expanded`]) so a component instance renders pixel-identically to
/// the same subtree placed directly in the scene.
///
/// `scene_id` is `Some` for a live-scene node (lets a frame background fall
/// back to scene-computed content bounds) and `None` for a transient clone
/// (whose descendants are not in the scene; an unclipped group with a
/// background then paints nothing, which is correct — there is no box to fill).
pub(crate) fn paint_node_content(
    canvas: &Canvas,
    node: &CanvasNode,
    scene_id: Option<NodeId>,
    ctx: &mut RenderCtx,
) -> ContentPaintState {
    let mut state = ContentPaintState::default();
    match &node.data {
        NodeData::Instance(i) => {
            // The instance's own content is nothing — its pixels come from the
            // expanded subtree, drawn by the caller. We only set up the frame
            // clip here so the master's content is confined to the instance
            // box (Figma instances clip like a frame). `local_size` is the
            // instance's box, independent of the master's size.
            let [w, h] = i.local_size;
            canvas.save();
            canvas.clip_rect(Rect::from_xywh(0.0, 0.0, w as f32, h as f32), None, true);
            state.restore_child_clip = true;
        }
        NodeData::Group(g) => {
            // A Figma frame is a group carrying a background fill (and usually a
            // clip size); a page (CANVAS) is a group with a background too.
            // Painting that background is what separates a frame — and the page
            // — from the dark canvas behind it. Without it every frame is
            // transparent and its content (notably black text on a white frame)
            // floats on the void, which reads as one undifferentiated jumble. A
            // clipped frame fills its `[0,0,w,h]` box; an unclipped group with a
            // background falls back to its content bounds.
            // The frame's box: its `clip_size` (a Figma FRAME/SECTION), else the
            // content bounds for an unclipped group that still carries a
            // background or a border. This box is what the background fills and
            // the border strokes — both rounded to the frame's corner radius.
            let box_bounds = match g.clip_size {
                Some([w, h]) => Some(Bounds::from_xywh(0.0, 0.0, w, h)),
                None if g.background.is_some()
                    || !g.background_fills.is_empty()
                    || !g.strokes.is_empty() =>
                {
                    scene_id.and_then(|id| ctx.scene.local_bounds(id))
                }
                None => None,
            };
            // The rounded-rect path of the frame box (square when no radius),
            // shared by the background fill and the border stroke so they round
            // identically. Built once and reused.
            let box_path = box_bounds.map(|b| {
                rounded_rect_path(
                    bounds_to_f32(&b),
                    g.corner_radius,
                    g.corner_radii,
                    g.corner_smoothing,
                )
            });

            // Painting the background is what separates a frame — and the page —
            // from the dark canvas behind it. Without it every frame is
            // transparent and its content (notably black text on a white frame)
            // floats on the void, which reads as one undifferentiated jumble.
            // Drawn as the rounded box path so a rounded card's fill matches its
            // border (no square fill peeking past rounded corners).
            if let (Some(b), Some(path)) = (box_bounds, &box_path) {
                let mut paint_box_fill = |fill: &Fill| {
                    if let Fill::Image {
                        asset,
                        mode,
                        opacity,
                        crop,
                    } = fill
                    {
                        // Hoisted so the resolve closure captures a local, not
                        // `ctx`, which `ctx.cache` borrows mutably.
                        let resolver = ctx.resolver;
                        canvas.save();
                        canvas.clip_path(path, None, true);
                        canvas.translate((b.min_x as f32, b.min_y as f32));
                        let crop_rect = crop.as_deref().copied();
                        let drawn = draw_image_cached(
                            canvas,
                            ctx.cache,
                            *asset,
                            || resolver.and_then(|r| r.resolve(*asset)),
                            [b.width(), b.height()],
                            crop_rect,
                            *mode,
                            None,
                            *opacity,
                        );
                        canvas.restore();
                        if !drawn {
                            // Unresolved asset or malformed buffer: the same
                            // placeholder paint as before, outside the clip.
                            let paint = fill_to_paint(fill, bounds_to_f32(&b));
                            canvas.draw_path(path, &paint);
                        }
                        ctx.metrics.nodes_drawn += 1;
                    } else {
                        let paint = fill_to_paint(fill, bounds_to_f32(&b));
                        canvas.draw_path(path, &paint);
                        ctx.metrics.nodes_drawn += 1;
                    }
                };
                if let Some(bg) = &g.background {
                    paint_box_fill(bg);
                }
                for bg in &g.background_fills {
                    paint_box_fill(bg);
                }
            }

            // Frame clipping: a Figma FRAME usually confines its content to its
            // box, whereas a plain group/page lets content overflow. The box is
            // carried by `clip_size`; `meta.clip_content=false` preserves the
            // box/background/border while disabling the descendant crop
            // (`frameMaskDisabled=true` and SECTION).
            //
            // A rounded frame clips to its ROUNDED-rect path so children respect
            // the corner radius; a square frame clips to the plain rect (cheaper,
            // and identical to the old behaviour). The clip is in node-LOCAL
            // space because `node.transform` is already concatenated by the
            // caller (so a translated/rotated frame clips to its own oriented
            // box). It is applied within this node's `save()`/`restore()` pair —
            // and BEFORE the child recursion that follows in the caller — so it
            // scopes exactly this frame's subtree and is lifted off the stack
            // before any sibling is drawn, never leaking. The background is
            // painted before this child slot; the foreground stroke is painted
            // after the child slot is restored.
            //
            // Anti-aliased (`true`) to match the soft edges the rest of the
            // renderer draws, so a clipped child's boundary doesn't show a hard
            // aliased step against the frame edge.
            let clips_content = node
                .meta
                .get("clip_content")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if clips_content && let Some([w, h]) = g.clip_size {
                canvas.save();
                if g.corner_radius.is_some() || g.corner_radii.is_some() {
                    let path = rounded_rect_path(
                        [0.0, 0.0, w as f32, h as f32],
                        g.corner_radius,
                        g.corner_radii,
                        g.corner_smoothing,
                    );
                    canvas.clip_path(&path, None, true);
                } else {
                    canvas.clip_rect(Rect::from_xywh(0.0, 0.0, w as f32, h as f32), None, true);
                }
                state.restore_child_clip = true;
            }
        }
        NodeData::Vector(v) => {
            draw_vector(
                canvas,
                &v.path,
                &v.fills,
                &v.strokes,
                v.corner_radius,
                v.corner_radii,
                ctx,
            );
        }
        NodeData::Text(t) => {
            // Real glyph rendering: shape `t.content` through `fanta-text`'s
            // Skia `LayoutEngine` at `t.style` / `t.align`, wrapping to the
            // box's width, and paint at the node-local origin. The canvas
            // already carries the full parent→viewport→DPI transform (the
            // caller concatenated `node.transform`), so we draw in LOCAL
            // coordinates — no zoom/scale re-application here.
            draw_text_node(canvas, t);
            ctx.metrics.nodes_drawn += 1;
        }
        NodeData::Bitmap(b) => {
            // Cache-hit → fit-draw the uploaded SkImage; miss → resolve/decode,
            // build/upload (cached by asset id), then draw. Anything that fails
            // (no resolver, unknown asset, still-decoding, malformed buffer)
            // falls through to the placeholder so a missing image is *visible*
            // and never panics — the deferred-output contract of
            // `AssetResolver`.
            let resolver = ctx.resolver;
            let drawn = draw_image_cached(
                canvas,
                ctx.cache,
                b.asset,
                || resolver.and_then(|r| r.resolve(b.asset)),
                b.local_size,
                b.crop,
                b.fit,
                b.tint,
                1.0,
            );
            if !drawn {
                draw_placeholder(canvas, b.local_size, Color::rgba(120, 200, 255, 100));
            }
            ctx.metrics.nodes_drawn += 1;
        }
        NodeData::Video(v) => {
            // While playing, the app-playback→render seam supplies the current
            // filmstrip FRAME for this node; otherwise we show the poster. Either
            // is clipped to the shared card radius and sits on the same shadow as
            // its siblings; the film-strip card is the fallback when neither a
            // frame nor a poster is available (e.g. ffmpeg unavailable).
            let resolver = ctx.resolver;
            let pb = scene_id.and_then(|id| ctx.inputs.playback.and_then(|m| m.get(&id)));
            let frame = pb.and_then(|p| p.frame);
            super::media::draw_media_shadow(canvas, v.local_size);
            // Try the live frame first, then the poster — so an unresolvable
            // frame id degrades to the poster, never the blank card.
            let mut drawn = false;
            let mut showing_frame = false;
            for (is_frame, asset) in frame
                .map(|a| (true, a))
                .into_iter()
                .chain(v.poster.map(|a| (false, a)))
            {
                canvas.save();
                super::media::clip_media(canvas, v.local_size);
                let ok = draw_image_cached(
                    canvas,
                    ctx.cache,
                    asset,
                    || resolver.and_then(|r| r.resolve(asset)),
                    v.local_size,
                    None,
                    v.fit,
                    None,
                    1.0,
                );
                canvas.restore();
                if ok {
                    drawn = true;
                    showing_frame = is_frame;
                    break;
                }
            }
            if drawn {
                // The play overlay is the resting affordance — shown on the
                // poster, hidden while real frames animate.
                if !showing_frame {
                    super::media::draw_play_overlay(canvas, v.local_size);
                }
                super::media::draw_media_border(canvas, v.local_size, true);
            } else {
                super::media::draw_video_card(canvas, v.local_size);
                super::media::draw_media_border(canvas, v.local_size, true);
            }
            // A bottom progress bar while the clip is playing.
            if let Some(p) = pb.map(|p| p.progress) {
                super::media::draw_video_progress(canvas, v.local_size, p);
            }
            ctx.metrics.nodes_drawn += 1;
        }
        NodeData::Audio(a) => {
            // Real waveform from the source PCM when the bytes are reachable;
            // otherwise the placeholder (the deferred-output contract). The shared
            // media-card shadow + hairline make it a sibling of the video/3D nodes.
            super::media::draw_media_shadow(canvas, a.local_size);
            // Live playhead position from the app-playback→render seam, if this
            // node is the one currently playing.
            let progress = media_progress(ctx, scene_id);
            // The node's waveform color counts only when it was explicitly set —
            // the default is opaque black, which vanished on the card; `None`
            // then lets the renderer pick a theme accent that actually reads.
            let accent = (a.waveform_color != Color::BLACK).then_some(a.waveform_color);
            let dark = ctx.inputs.dark_ui;
            let drawn = ctx
                .resolver
                .and_then(|r| r.resolve_bytes(a.asset))
                .is_some_and(|bytes| {
                    super::media::draw_audio_waveform(
                        canvas,
                        a.local_size,
                        &bytes,
                        accent,
                        progress,
                        dark,
                    )
                });
            if drawn {
                super::media::draw_media_border(canvas, a.local_size, dark);
            } else {
                draw_placeholder(canvas, a.local_size, Color::rgba(120, 255, 200, 100));
            }
            ctx.metrics.nodes_drawn += 1;
        }
        NodeData::NodeGraph(n) => {
            draw_placeholder(canvas, n.local_size, Color::rgba(255, 200, 120, 100));
            ctx.metrics.nodes_drawn += 1;
        }
        NodeData::Model3d(m) => {
            super::media::draw_media_shadow(canvas, m.local_size);
            super::media::draw_studio_fill(canvas, m.local_size);
            canvas.save();
            super::media::clip_media(canvas, m.local_size);
            super::media::draw_contact_shadow(canvas, m.local_size);
            super::media::draw_cube(canvas, m.local_size);
            canvas.restore();
            super::media::draw_media_border(canvas, m.local_size, true);
            ctx.metrics.nodes_drawn += 1;
        }
        NodeData::AiArtifact(a) => {
            draw_placeholder(canvas, a.local_size, Color::rgba(255, 120, 200, 100));
            ctx.metrics.nodes_drawn += 1;
        }
        NodeData::Embed(e) => {
            draw_placeholder(canvas, e.local_size, Color::rgba(180, 180, 180, 80));
            ctx.metrics.nodes_drawn += 1;
        }
    }
    state
}

pub(crate) fn paint_node_foreground(
    canvas: &Canvas,
    node: &CanvasNode,
    scene_id: Option<NodeId>,
    ctx: &mut RenderCtx,
) {
    let NodeData::Group(g) = &node.data else {
        return;
    };
    if g.strokes.is_empty() || is_figma_section(node) {
        return;
    }

    let box_bounds = match g.clip_size {
        Some([w, h]) => Some(Bounds::from_xywh(0.0, 0.0, w, h)),
        None if g.background.is_some()
            || !g.background_fills.is_empty()
            || !g.strokes.is_empty() =>
        {
            scene_id.and_then(|id| ctx.scene.local_bounds(id))
        }
        None => None,
    };
    if let Some(bounds) = box_bounds {
        stroke_box_path(
            canvas,
            bounds_to_f32(&bounds),
            g.corner_radius,
            g.corner_radii,
            g.corner_smoothing,
            &g.strokes,
            ctx,
        );
    }
}

fn is_figma_section(node: &CanvasNode) -> bool {
    node.meta.get("figma_type").and_then(|value| value.as_str()) == Some("SECTION")
}
