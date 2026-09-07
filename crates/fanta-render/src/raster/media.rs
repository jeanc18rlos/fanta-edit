//! Visualizer renders for the media node kinds — audio waveform, 3D cube, and
//! video card. Pure Skia / CPU (no GPU, no codecs) so `fanta-render` stays
//! decode-free, yet a placed audio/3D/video node "comes alive" on the canvas
//! instead of a flat placeholder. Audio is a *real* render of the source PCM.

use fanta_doc::Color;
use skia_safe::{
    BlurStyle, Canvas, MaskFilter, Paint, Path, Point, RRect, Rect, TileMode, gradient_shader,
    paint::Style,
};

use crate::color::to_sk_color;

/// Shared corner radius for every media-node card (3D / video / audio), so a
/// model, a clip, and a track read as ONE family on the canvas instead of three
/// unrelated shapes. The single source of truth for the media-card look.
pub(crate) const MEDIA_RADIUS: f32 = 12.0;

fn fill(color: Color) -> Paint {
    let mut p = Paint::default();
    p.set_anti_alias(true);
    p.set_color(to_sk_color(color));
    p
}

fn panel_bg(canvas: &Canvas, w: f32, h: f32, color: Color) {
    canvas.draw_round_rect(
        Rect::from_xywh(0.0, 0.0, w, h),
        MEDIA_RADIUS,
        MEDIA_RADIUS,
        &fill(color),
    );
}

/// The card's rounded rect at the shared radius.
fn media_rrect(w: f32, h: f32) -> RRect {
    RRect::new_rect_xy(Rect::from_xywh(0.0, 0.0, w, h), MEDIA_RADIUS, MEDIA_RADIUS)
}

/// Soft drop shadow under a media card. The arm draws this BEFORE the content so
/// every visualizer sits on the canvas with the same gentle elevation — the
/// first cue that turns three pasted-in shapes into a coherent media family.
pub(crate) fn draw_media_shadow(canvas: &Canvas, size: [f64; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    if w < 8.0 || h < 8.0 {
        return;
    }
    let mut p = Paint::default();
    p.set_anti_alias(true);
    p.set_color(to_sk_color(Color::rgba(15, 18, 25, 64)));
    p.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 7.0, false));
    canvas.draw_round_rect(
        Rect::from_xywh(0.0, 5.0, w, h),
        MEDIA_RADIUS,
        MEDIA_RADIUS,
        &p,
    );
}

/// Clip subsequent drawing to the card's rounded rect. The caller wraps this in
/// `save()` / `restore()`, so edge-to-edge content (a poster, a mesh) inherits
/// the shared corner radius instead of bleeding to a hard rectangle.
pub(crate) fn clip_media(canvas: &Canvas, size: [f64; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    canvas.clip_rrect(media_rrect(w, h), None, Some(true));
}

/// A 1px inner hairline around the card. `dark_content` chooses a light vs dark
/// stroke so it reads on a dark poster/waveform AND on a light 3D viewport.
pub(crate) fn draw_media_border(canvas: &Canvas, size: [f64; 2], dark_content: bool) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let mut p = Paint::default();
    p.set_anti_alias(true);
    p.set_style(Style::Stroke);
    p.set_stroke_width(1.0);
    let c = if dark_content {
        Color::rgba(255, 255, 255, 38)
    } else {
        Color::rgba(0, 0, 0, 30)
    };
    p.set_color(to_sk_color(c));
    canvas.draw_round_rect(
        Rect::from_xywh(0.5, 0.5, w - 1.0, h - 1.0),
        MEDIA_RADIUS - 0.5,
        MEDIA_RADIUS - 0.5,
        &p,
    );
}

/// A light "studio" viewport fill for the 3D card — a soft vertical gradient the
/// transparent mesh composites onto, so the model sits IN a viewport instead of
/// floating as a bare cutout on the canvas.
pub(crate) fn draw_studio_fill(canvas: &Canvas, size: [f64; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let mut p = Paint::default();
    p.set_anti_alias(true);
    if let Some(shader) = gradient_shader::linear(
        (Point::new(0.0, 0.0), Point::new(0.0, h)),
        &[
            to_sk_color(Color::rgb(250, 250, 252)),
            to_sk_color(Color::rgb(231, 232, 238)),
        ][..],
        None,
        TileMode::Clamp,
        None,
        None,
    ) {
        p.set_shader(shader);
    } else {
        p.set_color(to_sk_color(Color::rgb(242, 242, 246)));
    }
    canvas.draw_round_rect(
        Rect::from_xywh(0.0, 0.0, w, h),
        MEDIA_RADIUS,
        MEDIA_RADIUS,
        &p,
    );
}

/// A soft contact shadow ellipse that grounds the 3D model in its viewport.
/// Drawn inside the card clip, before the mesh — the single biggest cue that
/// reads "object sitting in a scene" rather than "sticker pasted on".
pub(crate) fn draw_contact_shadow(canvas: &Canvas, size: [f64; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let mut p = Paint::default();
    p.set_anti_alias(true);
    p.set_color(to_sk_color(Color::rgba(18, 22, 32, 66)));
    p.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 9.0, false));
    let (ew, eh) = (w * 0.52, h * 0.11);
    let (cx, cy) = (w * 0.5, h * 0.82);
    canvas.draw_oval(Rect::from_xywh(cx - ew * 0.5, cy - eh * 0.5, ew, eh), &p);
}

/// Draw a real waveform from raw WAV/PCM bytes into the node box, as a themed
/// audio player: a card, a left play/pause button, and the waveform. Returns
/// `false` (→ caller draws the placeholder) when the bytes aren't 16-bit PCM WAV.
///
/// - `accent` is the node's explicit waveform color, or `None` to use a
///   theme-derived accent that reads on the card (the old default was black,
///   which vanished on the dark pill).
/// - `progress` (0..=1) splits the waveform like a real player: bars left of the
///   playhead are the accent, bars to the right are dimmed, a playhead marks the
///   boundary, and the button shows a pause glyph. `None` = resting waveform.
/// - `dark` picks the card fill + contrast for the active app theme.
pub(crate) fn draw_audio_waveform(
    canvas: &Canvas,
    size: [f64; 2],
    bytes: &[u8],
    accent: Option<Color>,
    progress: Option<f32>,
    dark: bool,
) -> bool {
    let (w, h) = (size[0] as f32, size[1] as f32);
    if w < 8.0 || h < 8.0 {
        return false;
    }

    // Theme palette: card fill, default accent, dim (remaining), centerline.
    let (card, base_accent, line) = if dark {
        (
            Color::rgba(24, 28, 36, 255),
            Color::rgb(96, 165, 250),
            Color::rgba(255, 255, 255, 22),
        )
    } else {
        (
            Color::rgba(236, 238, 244, 255),
            Color::rgb(43, 104, 198),
            Color::rgba(18, 22, 31, 24),
        )
    };
    let acc = accent.unwrap_or(base_accent);
    // Remaining bars: the accent at a low-but-visible alpha (the old 40%-of-black
    // disappeared). Bright enough to read on either card.
    let dim_a = if dark { 150 } else { 96 };
    let dim = Color::rgba(acc.r, acc.g, acc.b, dim_a);

    panel_bg(canvas, w, h, card);

    // Left play/pause button.
    let btn_r = (h * 0.26).clamp(11.0, 26.0);
    let btn_cx = 12.0 + btn_r;
    let btn_cy = h / 2.0;
    draw_audio_button(canvas, btn_cx, btn_cy, btn_r, acc, progress.is_some());

    // Waveform region: everything to the right of the button.
    let x0 = btn_cx + btn_r + 12.0;
    let x1 = w - 12.0;
    let region = (x1 - x0).max(1.0);
    let cols = ((region / 2.4) as usize).clamp(16, 600);
    let Some(peaks) = wav_peaks(bytes, cols) else {
        return false;
    };

    let mid = h / 2.0;
    let mut center = Paint::default();
    center.set_anti_alias(true);
    center.set_color(to_sk_color(line));
    canvas.draw_line((x0, mid), (x1, mid), &center);

    let played = fill(acc);
    let remaining = fill(dim);
    let n = peaks.len().max(1) as f32;
    let bw = region / n;
    let play_x = progress.map(|p| x0 + p.clamp(0.0, 1.0) * region);
    for (i, p) in peaks.iter().enumerate() {
        let x = x0 + i as f32 * bw;
        let bh = (p * (h * 0.40)).max(1.0);
        let paint = match play_x {
            Some(px) if x + bw * 0.5 > px => &remaining,
            _ => &played,
        };
        canvas.draw_round_rect(
            Rect::from_xywh(x + bw * 0.14, mid - bh, (bw * 0.72).max(1.0), bh * 2.0),
            1.5,
            1.5,
            paint,
        );
    }
    if let Some(px) = play_x {
        let mut head = Paint::default();
        head.set_anti_alias(true);
        head.set_color(to_sk_color(acc));
        canvas.draw_round_rect(
            Rect::from_xywh(px - 1.0, 6.0, 2.0, h - 12.0),
            1.0,
            1.0,
            &head,
        );
    }
    true
}

/// The circular play/pause control at the left of an audio card. Accent disc +
/// a white glyph (a play triangle, or two pause bars while playing).
fn draw_audio_button(canvas: &Canvas, cx: f32, cy: f32, r: f32, accent: Color, playing: bool) {
    canvas.draw_circle((cx, cy), r, &fill(accent));
    let glyph = Color::rgba(255, 255, 255, 240);
    if playing {
        let (bw, bh) = (r * 0.24, r * 0.78);
        canvas.draw_round_rect(
            Rect::from_xywh(cx - bw * 1.5, cy - bh * 0.5, bw, bh),
            1.5,
            1.5,
            &fill(glyph),
        );
        canvas.draw_round_rect(
            Rect::from_xywh(cx + bw * 0.5, cy - bh * 0.5, bw, bh),
            1.5,
            1.5,
            &fill(glyph),
        );
    } else {
        let t = r * 0.5;
        let mut tri = Path::new();
        tri.move_to((cx - t * 0.4, cy - t));
        tri.line_to((cx - t * 0.4, cy + t));
        tri.line_to((cx + t * 0.82, cy));
        tri.close();
        canvas.draw_path(&tri, &fill(glyph));
    }
}

/// Peak amplitude (0..=1) per column from a 16-bit PCM WAV; `None` if the bytes
/// are not a 16-bit-PCM RIFF/WAVE file.
fn wav_peaks(bytes: &[u8], cols: usize) -> Option<Vec<f32>> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut i = 12usize;
    let mut channels = 1u16;
    let mut bits = 16u16;
    let mut data: Option<(usize, usize)> = None;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let sz =
            u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        let start = i + 8;
        let end = start.saturating_add(sz).min(bytes.len());
        if id == b"fmt " && end - start >= 16 {
            channels = u16::from_le_bytes([bytes[start + 2], bytes[start + 3]]).max(1);
            bits = u16::from_le_bytes([bytes[start + 14], bytes[start + 15]]);
        } else if id == b"data" {
            data = Some((start, end));
            break;
        }
        i = start + sz + (sz & 1); // chunks are word-aligned
    }
    let (ds, de) = data?;
    if bits != 16 {
        return None;
    }
    let frame = 2 * channels as usize;
    if frame == 0 || cols == 0 {
        return None;
    }
    let frames = (de - ds) / frame;
    if frames == 0 {
        return None;
    }
    let data = &bytes[ds..de];
    let step = (frames / (cols * 64)).max(1); // subsample for speed on long clips
    let mut peaks = vec![0f32; cols];
    for (c, peak) in peaks.iter_mut().enumerate() {
        let s = c * frames / cols;
        let e = ((c + 1) * frames / cols).min(frames);
        let mut k = s;
        let mut m = 0f32;
        while k < e {
            let off = k * frame;
            if off + 2 <= data.len() {
                let v = i16::from_le_bytes([data[off], data[off + 1]]) as f32 / 32768.0;
                m = m.max(v.abs());
            }
            k += step;
        }
        *peak = m;
    }
    Some(peaks)
}

// Unit-cube corners, bit-indexed: bit0=x, bit1=y, bit2=z, each in {-1, +1}.
fn cube_vertex(i: usize) -> (f32, f32, f32) {
    let pick = |bit: usize| if (i >> bit) & 1 == 1 { 1.0 } else { -1.0 };
    (pick(0), pick(1), pick(2))
}

const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7), // x edges
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7), // y edges
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7), // z edges
];
// The three faces visible from a top-front-left iso view.
const CUBE_FACES: [([usize; 4], u8); 3] = [
    ([2, 3, 7, 6], 150), // top    (y = +1)
    ([4, 5, 7, 6], 120), // front  (z = +1)
    ([0, 2, 6, 4], 92),  // left   (x = -1)
];

/// Draw a shaded isometric cube — a genuine 3D projection of the unit cube's
/// vertices — as the 3D-node preview. Pure Skia, no GPU.
pub(crate) fn draw_cube(canvas: &Canvas, size: [f64; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    panel_bg(canvas, w, h, Color::rgba(26, 28, 38, 255));

    let (cx, cy) = (w / 2.0, h / 2.0);
    let s = w.min(h) * 0.28;
    let proj = |idx: usize| -> (f32, f32) {
        let (x, y, z) = cube_vertex(idx);
        (
            cx + (x - z) * s * 0.866,
            cy - y * s * 0.9 + (x + z) * s * 0.30,
        )
    };

    for (face, shade) in CUBE_FACES {
        let mut path = Path::new();
        let (ax, ay) = proj(face[0]);
        path.move_to((ax, ay));
        for &v in &face[1..] {
            let (x, y) = proj(v);
            path.line_to((x, y));
        }
        path.close();
        canvas.draw_path(
            &path,
            &fill(Color::rgb(shade, shade.saturating_add(16), 235)),
        );
    }

    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(Style::Stroke);
    edge.set_stroke_width(1.5);
    edge.set_color(to_sk_color(Color::rgba(255, 255, 255, 130)));
    for (a, b) in CUBE_EDGES {
        let (ax, ay) = proj(a);
        let (bx, by) = proj(b);
        canvas.draw_line((ax, ay), (bx, by), &edge);
    }
}

/// Draw a play button centered in the box — overlaid on a real poster frame so a
/// placed video reads as playable, not a static image. A soft radial scrim sits
/// behind the disc so the control survives a bright poster (e.g. a blown-out sky)
/// as well as a dark one; the disc is floored at a 44px tap target.
pub(crate) fn draw_play_overlay(canvas: &Canvas, size: [f64; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = (w.min(h) * 0.15).clamp(18.0, 44.0);

    // Radial scrim: a blurred dark halo that darkens any background so the disc
    // and glyph keep contrast regardless of the poster underneath.
    let mut scrim = Paint::default();
    scrim.set_anti_alias(true);
    scrim.set_color(to_sk_color(Color::rgba(0, 0, 0, 82)));
    scrim.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, r * 0.55, false));
    canvas.draw_circle((cx, cy), r * 1.25, &scrim);

    canvas.draw_circle((cx, cy), r, &fill(Color::rgba(0, 0, 0, 150)));
    canvas.draw_circle((cx, cy), r, &{
        let mut p = Paint::default();
        p.set_anti_alias(true);
        p.set_style(Style::Stroke);
        p.set_stroke_width(1.5);
        p.set_color(to_sk_color(Color::rgba(255, 255, 255, 235)));
        p
    });
    let t = r * 0.46;
    let mut tri = Path::new();
    tri.move_to((cx - t * 0.45, cy - t));
    tri.line_to((cx - t * 0.45, cy + t));
    tri.line_to((cx + t, cy));
    tri.close();
    canvas.draw_path(&tri, &fill(Color::rgba(255, 255, 255, 245)));
}

/// A thin progress bar pinned to the bottom of a playing video card — the video
/// counterpart of the audio playhead, fed by the same playback seam.
pub(crate) fn draw_video_progress(canvas: &Canvas, size: [f64; 2], progress: f32) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let y = h - 7.0;
    let track_w = (w - 16.0).max(0.0);
    canvas.draw_round_rect(
        Rect::from_xywh(8.0, y, track_w, 3.0),
        1.5,
        1.5,
        &fill(Color::rgba(255, 255, 255, 64)),
    );
    let pw = (track_w * progress.clamp(0.0, 1.0)).max(0.0);
    canvas.draw_round_rect(
        Rect::from_xywh(8.0, y, pw, 3.0),
        1.5,
        1.5,
        &fill(Color::rgba(255, 255, 255, 235)),
    );
}

/// Draw a video card — dark frame, film-strip perforations, and a play glyph.
pub(crate) fn draw_video_card(canvas: &Canvas, size: [f64; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    panel_bg(canvas, w, h, Color::rgba(16, 18, 24, 255));

    let perf = fill(Color::rgba(255, 255, 255, 30));
    let ph = (h * 0.09).clamp(4.0, 12.0);
    let pw = ph * 0.7;
    let gap = ph * 1.7;
    let mut x = gap * 0.5;
    while x + pw < w {
        canvas.draw_round_rect(Rect::from_xywh(x, ph * 0.4, pw, ph), 2.0, 2.0, &perf);
        canvas.draw_round_rect(Rect::from_xywh(x, h - ph * 1.4, pw, ph), 2.0, 2.0, &perf);
        x += gap;
    }

    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = (w.min(h) * 0.16).clamp(10.0, 40.0);
    canvas.draw_circle((cx, cy), r, &fill(Color::rgba(255, 255, 255, 28)));
    let t = r * 0.5;
    let mut tri = Path::new();
    tri.move_to((cx - t * 0.5, cy - t));
    tri.line_to((cx - t * 0.5, cy + t));
    tri.line_to((cx + t, cy));
    tri.close();
    canvas.draw_path(&tri, &fill(Color::rgba(255, 255, 255, 235)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_peaks_reads_a_pcm16_sine() {
        // 200 mono i16 frames of a half-amplitude tone.
        let mut data = Vec::new();
        for i in 0..200i32 {
            let v = ((i as f32 * 0.3).sin() * 16000.0) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44_100u32.to_le_bytes());
        wav.extend_from_slice(&88_200u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);

        let peaks = wav_peaks(&wav, 16).expect("pcm16 wav parses");
        assert_eq!(peaks.len(), 16);
        assert!(peaks.iter().any(|&p| p > 0.1), "should detect signal");
        assert!(peaks.iter().all(|&p| p <= 1.0));
    }

    #[test]
    fn wav_peaks_rejects_non_wav() {
        assert!(wav_peaks(b"not a wav file at all............", 16).is_none());
    }

    /// Visual harness for the unified media-card chrome. Set
    /// `FANTA_MEDIA_PREVIEW=/path/out.png` to composite the 3D / video / audio
    /// cards side-by-side on the canvas gray and save a PNG for inspection.
    /// Skipped (no-op) without the env var so CI stays headless-clean.
    #[test]
    fn media_cards_preview() {
        let Ok(out) = std::env::var("FANTA_MEDIA_PREVIEW") else {
            return;
        };
        use skia_safe::{EncodedImageFormat, surfaces};
        let mut surface = surfaces::raster_n32_premul((1060, 480)).expect("surface");
        let canvas = surface.canvas();
        canvas.clear(to_sk_color(Color::rgb(227, 227, 227)));

        // A synth WAV so the audio card shows a REAL waveform + the new split.
        let mut data = Vec::new();
        for i in 0..6000i32 {
            let env = 1.0 - (i as f32 / 6000.0);
            let v = ((i as f32 * 0.25).sin() * 18000.0 * env) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44_100u32.to_le_bytes());
        wav.extend_from_slice(&88_200u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);

        let card = |canvas: &Canvas, x: f32, y: f32, f: &dyn Fn(&Canvas)| {
            canvas.save();
            canvas.translate((x, y));
            f(canvas);
            canvas.restore();
        };

        // 3D card: shadow + studio viewport + contact shadow + a stand-in yellow
        // "duck" blob + light border.
        card(canvas, 40.0, 100.0, &|c| {
            let sz = [300.0, 260.0];
            draw_media_shadow(c, sz);
            draw_studio_fill(c, sz);
            c.save();
            clip_media(c, sz);
            draw_contact_shadow(c, sz);
            c.draw_oval(
                Rect::from_xywh(95.0, 120.0, 150.0, 95.0),
                &fill(Color::rgb(214, 188, 64)),
            );
            c.draw_circle((110.0, 120.0), 46.0, &fill(Color::rgb(224, 198, 72)));
            c.draw_circle((96.0, 110.0), 7.0, &fill(Color::rgb(20, 20, 20)));
            c.restore();
            draw_media_border(c, sz, false);
        });

        // Video card: shadow + a clipped "poster" gradient + scrim play disc.
        card(canvas, 380.0, 110.0, &|c| {
            let sz = [320.0, 240.0];
            draw_media_shadow(c, sz);
            c.save();
            clip_media(c, sz);
            let mut p = Paint::default();
            p.set_anti_alias(true);
            if let Some(sh) = gradient_shader::linear(
                (Point::new(0.0, 0.0), Point::new(0.0, 240.0)),
                &[
                    to_sk_color(Color::rgb(196, 222, 244)),
                    to_sk_color(Color::rgb(58, 120, 70)),
                ][..],
                None,
                TileMode::Clamp,
                None,
                None,
            ) {
                p.set_shader(sh);
            }
            c.draw_rect(Rect::from_xywh(0.0, 0.0, 320.0, 240.0), &p);
            c.restore();
            draw_play_overlay(c, sz);
            draw_media_border(c, sz, true);
        });

        // Audio card (dark theme): shadow + play button + waveform with the
        // played/remaining split + playhead at 42%.
        card(canvas, 720.0, 60.0, &|c| {
            let sz = [300.0, 96.0];
            draw_media_shadow(c, sz);
            draw_audio_waveform(c, sz, &wav, None, Some(0.42), true);
            draw_media_border(c, sz, true);
        });
        // Audio card (light theme) — proves theme awareness + contrast.
        card(canvas, 720.0, 240.0, &|c| {
            let sz = [300.0, 96.0];
            draw_media_shadow(c, sz);
            draw_audio_waveform(c, sz, &wav, None, None, false);
            draw_media_border(c, sz, false);
        });

        let img = surface.image_snapshot();
        let png = img
            .encode(None, EncodedImageFormat::PNG, 100)
            .expect("encode png");
        std::fs::write(&out, png.as_bytes()).expect("write png");
        eprintln!("media_cards_preview wrote {out}");
    }
}
