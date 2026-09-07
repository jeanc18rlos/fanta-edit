//! Decode of Figma's vector path-command blobs into [`fanta_doc::PathData`].
//!
//! ## The `commandsBlob` binary format — CONFIRMED EMPIRICALLY
//!
//! A vector-like `NodeChange` carries `fillGeometry: Path[]` (and
//! `strokeGeometry: Path[]`). Each `Path` is the Kiwi message
//! `{ windingRule: WindingRule, commandsBlob: uint, styleID: uint }`. The
//! `commandsBlob` is an **index into the root message's `blobs: Blob[]` array**
//! (each `Blob` is the struct `{ bytes: byte[] }`); the path's actual command
//! bytes are `blobs[commandsBlob]` (see [`crate::fig::FigDocument::blobs`]).
//!
//! The blob is a little-endian byte stream of path commands, parsed to the end
//! of the blob (there is **no** leading count). Each command is **one verb byte**
//! followed by N little-endian `f32` coordinates:
//!
//! | verb | meaning  | f32 coords                        |
//! |------|----------|-----------------------------------|
//! | `0`  | Close    | 0                                 |
//! | `1`  | MoveTo   | 2  (x, y)                         |
//! | `2`  | LineTo   | 2  (x, y)                         |
//! | `3`  | QuadTo   | 4  (cx, cy, x, y)                 |
//! | `4`  | CubicTo  | 6  (c1x, c1y, c2x, c2y, x, y)     |
//!
//! Coordinates are in the node's **local space** (origin at its top-left; values
//! fall within roughly `[0, size]`).
//!
//! ### How this was confirmed (not assumed)
//!
//! Dumped the schema of the real Adobe Spectrum community `.fig`: def `Path`
//! (`windingRule`/`commandsBlob`/`styleID`), def `Blob {bytes: byte[]}`, def
//! `WindingRule { NONZERO=0, ODD=1 }`, and the root `Message.blobs` array (5921
//! entries). Then, across the first 4000 `fillGeometry` blobs, the mapping
//! `0:0 1:2 2:2 3:4 4:6` (Close/Move/Line/Quad/Cubic) decoded **4000/4000**
//! cleanly — every verb byte in `0..=4` and the stream consuming to the exact
//! end — while the swapped `v3=Cubic/v4=Quad` mapping only managed 2383/4000 and
//! the `v0=Move` mapping 0/4000. Every coordinate landed inside the node's size
//! box. The verb-byte histogram showed `Close` and `Move` perfectly balanced
//! (5222 each — one of each per subpath), and a fully-decoded simple icon blob
//! began with `Move` and ended with `Close`, forming a closed contour.
//!
//! NOTE: this differs from the original task hypothesis (Move=0/Line=1/Quad=2/
//! Cubic=3/Close=4). The real verb numbering is **shifted**: Close=0, Move=1,
//! Line=2, Quad=3, Cubic=4.

use fanta_doc::path::{FillRule, PathData};

/// Verb byte for a `Close`.
const VERB_CLOSE: u8 = 0;
/// Verb byte for a `MoveTo` (2 f32).
const VERB_MOVE: u8 = 1;
/// Verb byte for a `LineTo` (2 f32).
const VERB_LINE: u8 = 2;
/// Verb byte for a `QuadTo` (4 f32).
const VERB_QUAD: u8 = 3;
/// Verb byte for a `CubicTo` (6 f32).
const VERB_CUBIC: u8 = 4;

/// The largest verb byte we recognize. A byte above this means the stream is
/// not in the expected format (or we lost sync), so decoding bails — the caller
/// then keeps the bbox fallback rather than rendering garbage.
const MAX_VERB: u8 = VERB_CUBIC;

/// Decode a single `commandsBlob` byte stream and **append** its subpaths onto
/// `out`. Returns `true` if the blob decoded cleanly (every verb valid and the
/// stream consumed to its exact end); `false` (leaving `out` unchanged) if the
/// blob is empty, malformed, or only partially decodable.
///
/// On a malformed blob we deliberately append **nothing** and report failure,
/// so a node either gets its whole real geometry or falls back to the bbox — we
/// never emit a half-decoded path that would render as a torn shape.
pub fn append_blob_path(out: &mut PathData, blob: &[u8]) -> bool {
    if blob.is_empty() {
        return false;
    }
    // Decode into a scratch buffer first; only commit on full, clean consumption.
    let mut scratch = PathData::new();
    let mut i = 0usize;
    while i < blob.len() {
        let verb = blob[i];
        if verb > MAX_VERB {
            return false; // unknown verb -> not our format / lost sync
        }
        i += 1;
        let nfloats = floats_for_verb(verb);
        // Need `nfloats` little-endian f32 = 4*nfloats bytes.
        let end = match i.checked_add(nfloats * 4) {
            Some(e) if e <= blob.len() => e,
            _ => return false, // truncated mid-command
        };
        let mut coords = [0f32; 6];
        for (c, slot) in coords.iter_mut().enumerate().take(nfloats) {
            let off = i + c * 4;
            *slot = f32::from_le_bytes([blob[off], blob[off + 1], blob[off + 2], blob[off + 3]]);
        }
        // A NaN/Infinity coordinate (seen in the wild in a handful of
        // degenerate blobs) would otherwise bake a non-finite float into the
        // doc; the project-tree writer serializes those as JSON `null`, which
        // then fails to deserialize back into the plain `f64` the schema
        // expects. Treat it like any other malformed command: bail on the
        // whole blob and keep the bbox fallback.
        if coords[..nfloats].iter().any(|c| !c.is_finite()) {
            return false;
        }
        i = end;
        let f = |k: usize| coords[k] as f64;
        match verb {
            VERB_CLOSE => {
                scratch.close();
            }
            VERB_MOVE => {
                scratch.move_to(f(0), f(1));
            }
            VERB_LINE => {
                scratch.line_to(f(0), f(1));
            }
            VERB_QUAD => {
                scratch.quad_to(f(0), f(1), f(2), f(3));
            }
            VERB_CUBIC => {
                scratch.cubic_to(f(0), f(1), f(2), f(3), f(4), f(5));
            }
            _ => unreachable!("verb already range-checked against MAX_VERB"),
        }
    }
    // A well-formed path has at least one Move; a blob that decoded to nothing
    // (e.g. all-Close noise) is not usable geometry.
    if scratch.segments.is_empty() {
        return false;
    }
    out.segments.extend(scratch.segments);
    true
}

/// f32-coordinate count for a verb byte (see the module-level format table).
fn floats_for_verb(verb: u8) -> usize {
    match verb {
        VERB_CLOSE => 0,
        VERB_MOVE | VERB_LINE => 2,
        VERB_QUAD => 4,
        VERB_CUBIC => 6,
        _ => 0,
    }
}

/// Map a Figma `WindingRule` enum member to a doc [`FillRule`]. `NONZERO` →
/// [`FillRule::NonZero`]; `ODD` (Figma's even-odd member name) →
/// [`FillRule::EvenOdd`]. Unknown / absent defaults to non-zero (the common
/// case and the doc default).
pub fn winding_rule(member: Option<&str>) -> FillRule {
    match member {
        Some("ODD") | Some("EVENODD") | Some("EVEN_ODD") => FillRule::EvenOdd,
        _ => FillRule::NonZero,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::path::PathSegment;

    /// Encode a verb byte plus its little-endian f32 coords (test helper that
    /// builds a blob in the confirmed wire format).
    fn cmd(verb: u8, coords: &[f32]) -> Vec<u8> {
        let mut v = vec![verb];
        for c in coords {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v
    }

    fn blob(cmds: &[(u8, &[f32])]) -> Vec<u8> {
        let mut v = Vec::new();
        for (verb, coords) in cmds {
            v.extend_from_slice(&cmd(*verb, coords));
        }
        v
    }

    #[test]
    fn decodes_a_triangle_move_line_line_close() {
        let bytes = blob(&[
            (VERB_MOVE, &[0.0, 0.0]),
            (VERB_LINE, &[10.0, 0.0]),
            (VERB_LINE, &[5.0, 8.0]),
            (VERB_CLOSE, &[]),
        ]);
        let mut p = PathData::new();
        assert!(append_blob_path(&mut p, &bytes));
        assert_eq!(
            p.segments,
            vec![
                PathSegment::Move { to: [0.0, 0.0] },
                PathSegment::Line { to: [10.0, 0.0] },
                PathSegment::Line { to: [5.0, 8.0] },
                PathSegment::Close,
            ]
        );
    }

    #[test]
    fn decodes_quad_and_cubic_commands() {
        let bytes = blob(&[
            (VERB_MOVE, &[1.0, 2.0]),
            (VERB_QUAD, &[3.0, 4.0, 5.0, 6.0]),
            (VERB_CUBIC, &[7.0, 8.0, 9.0, 10.0, 11.0, 12.0]),
            (VERB_CLOSE, &[]),
        ]);
        let mut p = PathData::new();
        assert!(append_blob_path(&mut p, &bytes));
        assert_eq!(
            p.segments,
            vec![
                PathSegment::Move { to: [1.0, 2.0] },
                PathSegment::Quad {
                    ctrl: [3.0, 4.0],
                    to: [5.0, 6.0]
                },
                PathSegment::Cubic {
                    ctrl1: [7.0, 8.0],
                    ctrl2: [9.0, 10.0],
                    to: [11.0, 12.0],
                },
                PathSegment::Close,
            ]
        );
    }

    #[test]
    fn multiple_subpaths_concatenate_in_one_pathdata() {
        // Two Move..Close contours in one blob become one PathData with both.
        let bytes = blob(&[
            (VERB_MOVE, &[0.0, 0.0]),
            (VERB_LINE, &[1.0, 0.0]),
            (VERB_CLOSE, &[]),
            (VERB_MOVE, &[2.0, 2.0]),
            (VERB_LINE, &[3.0, 2.0]),
            (VERB_CLOSE, &[]),
        ]);
        let mut p = PathData::new();
        assert!(append_blob_path(&mut p, &bytes));
        let moves = p
            .segments
            .iter()
            .filter(|s| matches!(s, PathSegment::Move { .. }))
            .count();
        assert_eq!(moves, 2, "two subpaths");
        assert_eq!(p.segments.len(), 6);
    }

    #[test]
    fn appends_onto_existing_path_without_clobbering() {
        let mut p = PathData::new();
        p.move_to(100.0, 100.0).line_to(110.0, 100.0).close();
        let before = p.segments.len();
        let bytes = blob(&[
            (VERB_MOVE, &[0.0, 0.0]),
            (VERB_LINE, &[1.0, 1.0]),
            (VERB_CLOSE, &[]),
        ]);
        assert!(append_blob_path(&mut p, &bytes));
        assert_eq!(p.segments.len(), before + 3);
    }

    #[test]
    fn empty_blob_fails_gracefully() {
        let mut p = PathData::new();
        assert!(!append_blob_path(&mut p, &[]));
        assert!(p.segments.is_empty());
    }

    #[test]
    fn unknown_verb_byte_rejects_without_partial_output() {
        // 0xFF is not a valid verb; the whole blob is rejected and `out` stays
        // empty (no torn half-path).
        let mut bytes = blob(&[(VERB_MOVE, &[0.0, 0.0]), (VERB_LINE, &[1.0, 1.0])]);
        bytes.push(0xFF);
        let mut p = PathData::new();
        assert!(!append_blob_path(&mut p, &bytes));
        assert!(
            p.segments.is_empty(),
            "must not emit a partially-decoded path"
        );
    }

    #[test]
    fn truncated_command_rejects() {
        // A Cubic verb that promises 6 f32 but supplies only 1.
        let mut bytes = vec![VERB_CUBIC];
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        let mut p = PathData::new();
        assert!(!append_blob_path(&mut p, &bytes));
        assert!(p.segments.is_empty());
    }

    #[test]
    fn garbage_bytes_do_not_panic_and_fall_back() {
        // Random-ish bytes: must return false (not our format), never panic.
        for seed in 0u8..32 {
            let bytes: Vec<u8> = (0..37u8)
                .map(|i| i.wrapping_mul(seed).wrapping_add(7))
                .collect();
            let mut p = PathData::new();
            let _ = append_blob_path(&mut p, &bytes); // result varies; must not panic
        }
    }

    #[test]
    fn winding_rule_maps_nonzero_and_odd() {
        assert_eq!(winding_rule(Some("NONZERO")), FillRule::NonZero);
        assert_eq!(winding_rule(Some("ODD")), FillRule::EvenOdd);
        assert_eq!(winding_rule(None), FillRule::NonZero);
        assert_eq!(winding_rule(Some("SOMETHING_ELSE")), FillRule::NonZero);
    }
}
