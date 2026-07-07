//! VECTOR-family geometry: decode real paths from `fillGeometry` command
//! blobs (and the `vectorNetworkBlob` fallback), else a bbox-rect placeholder.

use super::{
    CanvasNode, Fill, HashMap, IndexKey, KiwiValue, NodeBuild, NodeData, PathData, Transform2D,
    VectorNode, apply_node_visual_props, build_stroke, first_paint_fill, read_mask,
};

/// Build a [`NodeData::Vector`] for a VECTOR-family node, decoding its **real**
/// path geometry from the file's command blobs (STEP 2) when possible.
///
/// The vector geometry lives in `fillGeometry: Path[]`; each `Path`'s
/// `commandsBlob` is a `uint` index into the document's `blobs` table, and the
/// referenced blob is a little-endian path-command stream (verb byte + f32
/// coords — see [`crate::geometry`]). We decode every path of the node into one
/// combined [`PathData`] (subpaths concatenated), set the fill rule from the
/// first path's `windingRule`, decode `strokeGeometry` into the node's stroke
/// (with the node's `strokeWeight`/`strokePaints`), preserve the node's solid
/// fills, and tag `meta.geometry = "decoded"`.
///
/// If no geometry decodes (empty/absent `fillGeometry`, a missing/garbage blob),
/// we keep the STEP-1 **bbox fallback**: a rectangle sized to the node, tagged
/// `meta.geometry = "bbox_fallback"`. A node thus never regresses — it gets its
/// real shape or a footprint placeholder, never a torn half-path.
pub(crate) fn build_vector(
    type_name: &str,
    change: &KiwiValue,
    size: (f64, f64),
    fills: &[Fill],
    transform: Transform2D,
    blobs: &[Vec<u8>],
) -> NodeBuild {
    // STEP 2: prefer the pre-flattened `fillGeometry` command blobs. STEP 2b: when
    // a vector node carries no `fillGeometry` (the ~0.2% of fixture vectors that
    // only ship their editable `vectorData.vectorNetworkBlob`), decode that
    // VectorNetwork into the same `PathData` instead of regressing to a bbox rect.
    // Mirrors op2 `figma-vector-decoder.ts` (fillGeometry → vectorNetworkBlob
    // fallback) but with op1's correct wire format.
    let mut from_vector_network = false;
    let decoded_fill = decode_geometry(change, "fillGeometry", blobs).or_else(|| {
        let vn = decode_vector_network(change, size, blobs);
        from_vector_network = vn.is_some();
        vn
    });

    // Gap A — stroke-only icon outline. Lucide/Feather-style icons are stroke-only
    // vectors: their `fillGeometry` is the ALREADY-EXPANDED stroke outline (a
    // closed region, not a centerline). Decoding it into the path AND then adding
    // our own width-based stroke double-strokes it (blobby/doubled glyphs). When a
    // node has NO visible fills but DOES have visible strokes, we instead paint the
    // decoded outline as a FILL (the stroke paint becomes the fill) and emit NO
    // width stroke. Mirrors op2 `path-converter.ts` `isStrokeOnlyOutline` /
    // `figma-stroke-mapper.ts`. (Only meaningful when geometry actually decoded —
    // without the outline there's nothing to fill, so the bbox fallback keeps its
    // normal stroke.)
    // The Gap-A trick only applies to `fillGeometry`, which Figma ships as the
    // already-EXPANDED stroke outline. A `vectorNetworkBlob` path is the editable
    // CENTERLINE, so a stroke-only network must still be stroked with width (not
    // painted as a fill) — otherwise it would render as a thin filled hairline.
    let stroke_only_outline = decoded_fill.is_some()
        && !from_vector_network
        && !has_visible_fills(change)
        && has_visible_strokes(change);

    let (vector, decoded, stroke_only) = match decoded_fill {
        Some(path) if stroke_only_outline => {
            // Render the expanded stroke outline AS a fill (the stroke's paint),
            // and do NOT add a width stroke on top.
            let outline_fill = first_paint_fill(change.get("strokePaints"));
            (
                VectorNode {
                    path,
                    fills: outline_fill.into_iter().collect(),
                    strokes: Default::default(),
                    corner_radius: None,
                    corner_radii: None,
                },
                true,
                true,
            )
        }
        Some(path) => (
            VectorNode {
                path,
                fills: fills.iter().cloned().collect(),
                strokes: build_stroke(change).into_iter().collect(),
                // A rounded-rect-ish corner radius is meaningless on a real
                // decoded path (the curvature is already baked into the
                // commands), so leave it unset.
                corner_radius: None,
                corner_radii: None,
            },
            true,
            false,
        ),
        // No decodable geometry: keep the bbox fallback (STEP 1), but still carry
        // the node's fills + strokes onto the placeholder rect.
        None => (
            VectorNode {
                path: PathData::rect(0.0, 0.0, size.0, size.1),
                fills: fills.iter().cloned().collect(),
                strokes: build_stroke(change).into_iter().collect(),
                corner_radius: None,
                corner_radii: None,
            },
            false,
            false,
        ),
    };

    let mut node = CanvasNode::new(NodeData::Vector(vector));
    node.transform = transform;
    if let Some(name) = change.get("name").and_then(KiwiValue::as_str) {
        if !name.is_empty() {
            node.name = name.to_owned();
        }
    }
    apply_node_visual_props(&mut node, change);
    let geom_tag = if from_vector_network {
        "vector_network"
    } else if decoded {
        "decoded"
    } else {
        "bbox_fallback"
    };
    node.meta = serde_json::json!({
        "figma_type": type_name,
        "geometry": geom_tag,
        "stroke_only_outline": stroke_only,
    });
    // Masks: a VECTOR flagged `mask` masks its following siblings (the Spectrum
    // file's icon/shape masks are VECTOR/OUTLINE). The generic builder reads this
    // too, but VECTOR-family nodes take this early-returning path, so it must be
    // read here as well.
    read_mask(change, &mut node);
    node.index = IndexKey::FIRST;
    NodeBuild::Node {
        node: Box::new(node),
        is_page: false,
        geometry_decoded: decoded,
    }
}

/// Whether the node has at least one *visible* fill paint (`visible != false`),
/// reading the raw `fillPaints`. Used by Gap A's stroke-only-outline detection,
/// which must distinguish "the node draws a fill" from "we happened to decode a
/// fill path". Mirrors op2 `path-converter.ts` `hasVisibleFills`.
pub(crate) fn has_visible_fills(change: &KiwiValue) -> bool {
    has_any_visible_paint(change.get("fillPaints"))
}

/// Whether the node has at least one *visible* stroke paint (`visible != false`).
/// Mirrors op2 `path-converter.ts` `hasVisibleStrokes`.
pub(crate) fn has_visible_strokes(change: &KiwiValue) -> bool {
    has_any_visible_paint(change.get("strokePaints"))
}

/// Whether a paint array carries any paint that is not explicitly hidden. A paint
/// is visible unless it carries `visible: false` (matching Figma + op2's
/// `visible !== false` predicate). We deliberately do NOT require the paint to be
/// a *recognized* type here — a node painting an unsupported paint kind still
/// "has a visible fill/stroke" for the purpose of the stroke-only decision.
pub(crate) fn has_any_visible_paint(paints: Option<&KiwiValue>) -> bool {
    let Some(arr) = paints.and_then(KiwiValue::as_array) else {
        return false;
    };
    arr.iter()
        .any(|p| !matches!(p.get("visible"), Some(KiwiValue::Bool(false))))
}

/// Decode a node's `fillGeometry`/`strokeGeometry` (named by `field`) into one
/// combined [`PathData`]. Returns `None` if the field is absent/empty or no path
/// decoded cleanly — the caller then keeps the bbox fallback. The fill rule is
/// taken from the first path's `windingRule`.
pub(crate) fn decode_geometry(
    change: &KiwiValue,
    field: &str,
    blobs: &[Vec<u8>],
) -> Option<PathData> {
    let paths = change.get(field).and_then(KiwiValue::as_array)?;
    if paths.is_empty() {
        return None;
    }
    let mut out = PathData::new();
    let mut any = false;
    let mut fill_rule_set = false;
    for path in paths {
        let Some(idx) = path.get("commandsBlob").and_then(KiwiValue::as_f64) else {
            continue;
        };
        // A negative or NaN index can't be a valid table slot.
        if !(idx.is_finite() && idx >= 0.0) {
            continue;
        }
        let Some(blob) = blobs.get(idx as usize) else {
            continue;
        };
        if crate::geometry::append_blob_path(&mut out, blob) {
            any = true;
            // The whole node's fill rule comes from its first decoded path; Figma
            // stores it per-path but a node's paths share one rule in practice,
            // and our model carries a single rule per `PathData`.
            if !fill_rule_set {
                out.fill_rule = crate::geometry::winding_rule(
                    path.get("windingRule").and_then(KiwiValue::as_str),
                );
                fill_rule_set = true;
            }
        }
    }
    any.then_some(out)
}

/// Decode a VECTOR node's `vectorData.vectorNetworkBlob` into a [`PathData`] —
/// the **fallback** for the handful of nodes whose `fillGeometry`/`strokeGeometry`
/// command-blob arrays are empty/absent (in the Adobe Spectrum fixture, ~12 of
/// ~7200 vector nodes). Without this they fall back to a bare bbox rectangle.
///
/// ## The `vectorNetworkBlob` wire format — confirmed against the real fixture
///
/// `vectorData.vectorNetworkBlob` is a `uint` index into the document's `blobs`
/// table (same table as `commandsBlob`). The referenced blob is a little-endian
/// `VectorNetwork` — Figma's *editable* curve representation (vertices + segments
/// with bezier tangents + regions), distinct from the pre-flattened command
/// stream that [`crate::geometry::append_blob_path`] decodes. Layout (verified by
/// decoding the fixture's empty-`fillGeometry` vector nodes, and matching
/// OpenPencil `op1 packages/core/src/vector/index.ts::decodeVectorNetworkBlob`):
///
/// | section | bytes | fields                                                   |
/// |---------|-------|----------------------------------------------------------|
/// | header  | 12    | `numVertices:u32, numSegments:u32, numRegions:u32`       |
/// | vertex  | 12    | `styleOverrideIdx:u32, x:f32, y:f32`  × numVertices       |
/// | segment | 28    | `styleIdx:u32, start:u32, tsX:f32, tsY:f32, end:u32, teX:f32, teY:f32` × numSegments |
/// | region  | var   | `windingRule:u32, numLoops:u32, {numSegs:u32, segIdx:u32 …} …` × numRegions |
///
/// A segment's tangents are **relative offsets** from its endpoint vertices: a
/// segment with all-zero tangents is a straight `lineTo`; otherwise it's a cubic
/// with control points `start + tangentStart` and `end + tangentEnd`. The region
/// winding `0 → even-odd`, anything else → non-zero (note: this is the
/// VectorNetwork region convention, the **inverse** of the per-path `WindingRule`
/// enum that [`crate::geometry::winding_rule`] reads).
///
/// Coordinates are in the node's `normalizedSize` space, so we scale by
/// `nodeSize / normalizedSize` (per op1's `resolveVectorNetwork`) to land in the
/// node's local box. Returns `None` on any malformed/truncated blob, an absent
/// index, or a network that produced no segments — the caller then keeps the
/// bbox fallback (never a torn half-path).
pub(crate) fn decode_vector_network(
    change: &KiwiValue,
    size: (f64, f64),
    blobs: &[Vec<u8>],
) -> Option<PathData> {
    let vd = change.get("vectorData")?;
    let idx = vd.get("vectorNetworkBlob").and_then(KiwiValue::as_f64)?;
    if !(idx.is_finite() && idx >= 0.0) {
        return None;
    }
    let blob = blobs.get(idx as usize)?;

    // Scale from normalizedSize → node size (op1 `resolveVectorNetwork`). When the
    // node has no usable size or normalizedSize, fall back to 1:1 (the coordinates
    // are then taken as-is, which is still better than a bbox rect).
    let norm = vd.get("normalizedSize");
    let nw = norm.and_then(|n| n.get("x")).and_then(KiwiValue::as_f64);
    let nh = norm.and_then(|n| n.get("y")).and_then(KiwiValue::as_f64);
    let sx = match (nw, size.0 > 0.0) {
        (Some(w), true) if w > 1e-3 => size.0 / w,
        _ => 1.0,
    };
    let sy = match (nh, size.1 > 0.0) {
        (Some(h), true) if h > 1e-3 => size.1 / h,
        _ => 1.0,
    };

    let net = parse_vector_network(blob)?;
    net.to_path(sx, sy)
}

/// A decoded `vectorNetworkBlob`: vertices, segments (with relative bezier
/// tangents), and regions (filled loops). Kept minimal — only what's needed to
/// emit a [`PathData`]. See [`decode_vector_network`] for the wire format.
pub(crate) struct VectorNetwork {
    vertices: Vec<[f32; 2]>,
    segments: Vec<VnSegment>,
    /// Each region is `(even_odd, loops)`; each loop is a list of segment indices.
    regions: Vec<(bool, Vec<Vec<usize>>)>,
}

/// One `VectorNetwork` segment: endpoint vertex indices plus the cubic tangents
/// (relative offsets from the endpoints; all-zero ⇒ straight line).
pub(crate) struct VnSegment {
    start: usize,
    end: usize,
    ts: [f32; 2],
    te: [f32; 2],
}

/// Bounded little-endian cursor over a `vectorNetworkBlob`. Every read is
/// length-checked, so a truncated blob yields `None` rather than panicking.
struct VnCursor<'a> {
    b: &'a [u8],
    o: usize,
}

impl VnCursor<'_> {
    fn u32(&mut self) -> Option<u32> {
        let e = self.o.checked_add(4)?;
        if e > self.b.len() {
            return None;
        }
        let v = u32::from_le_bytes([
            self.b[self.o],
            self.b[self.o + 1],
            self.b[self.o + 2],
            self.b[self.o + 3],
        ]);
        self.o = e;
        Some(v)
    }
    fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits)
    }
}

/// Parse the `vectorNetworkBlob` bytes into a [`VectorNetwork`]. Returns `None`
/// on any truncation / out-of-range index, so a malformed blob is tolerated
/// (caller keeps the bbox fallback) and never panics.
pub(crate) fn parse_vector_network(blob: &[u8]) -> Option<VectorNetwork> {
    let mut c = VnCursor { b: blob, o: 0 };
    let n_v = c.u32()?;
    let n_s = c.u32()?;
    let n_r = c.u32()?;
    if n_v > MAX_VN_COUNT || n_s > MAX_VN_COUNT || n_r > MAX_VN_COUNT {
        return None;
    }

    let vertices = parse_vn_vertices(&mut c, n_v)?;
    let segments = parse_vn_segments(&mut c, n_s, vertices.len())?;
    let regions = parse_vn_regions(&mut c, n_r, segments.len())?;

    Some(VectorNetwork {
        vertices,
        segments,
        regions,
    })
}

/// Cap on any `vectorNetworkBlob` section count so a corrupt header can't trigger
/// a huge allocation. The largest real network in the fixture has a few hundred
/// vertices; 1M is a generous ceiling that still bounds memory.
const MAX_VN_COUNT: u32 = 1_000_000;

/// Read `n_v` `(styleOverrideIdx, x, y)` vertex records (the style index is a
/// handle-mirroring hint, unused here). `None` on truncation.
fn parse_vn_vertices(c: &mut VnCursor<'_>, n_v: u32) -> Option<Vec<[f32; 2]>> {
    let mut vertices = Vec::with_capacity(n_v as usize);
    for _ in 0..n_v {
        let _style = c.u32()?;
        let x = c.f32()?;
        let y = c.f32()?;
        vertices.push([x, y]);
    }
    Some(vertices)
}

/// Read `n_s` segment records, validating each endpoint index against
/// `n_vertices`. `None` on truncation or an out-of-range endpoint.
fn parse_vn_segments(c: &mut VnCursor<'_>, n_s: u32, n_vertices: usize) -> Option<Vec<VnSegment>> {
    let mut segments = Vec::with_capacity(n_s as usize);
    for _ in 0..n_s {
        let _style = c.u32()?; // segment styleOverrideIdx — unused
        let start = c.u32()? as usize;
        let ts_x = c.f32()?;
        let ts_y = c.f32()?;
        let end = c.u32()? as usize;
        let te_x = c.f32()?;
        let te_y = c.f32()?;
        // Endpoint indices must reference real vertices.
        if start >= n_vertices || end >= n_vertices {
            return None;
        }
        segments.push(VnSegment {
            start,
            end,
            ts: [ts_x, ts_y],
            te: [te_x, te_y],
        });
    }
    Some(segments)
}

/// Read `n_r` region records (`windingRule`, loops of segment indices),
/// validating each segment index against `n_segments` and bounding loop/seg
/// counts by [`MAX_VN_COUNT`]. Region winding: 0 ⇒ even-odd, else ⇒ non-zero
/// (op1). `None` on truncation or an out-of-range segment.
fn parse_vn_regions(
    c: &mut VnCursor<'_>,
    n_r: u32,
    n_segments: usize,
) -> Option<Vec<(bool, Vec<Vec<usize>>)>> {
    let mut regions = Vec::with_capacity(n_r as usize);
    for _ in 0..n_r {
        let winding = c.u32()?;
        let n_loops = c.u32()?;
        if n_loops > MAX_VN_COUNT {
            return None;
        }
        let mut loops = Vec::with_capacity(n_loops as usize);
        for _ in 0..n_loops {
            loops.push(parse_vn_loop(c, n_segments)?);
        }
        regions.push((winding == 0, loops));
    }
    Some(regions)
}

/// Read one region loop: an `n_segs`-long list of segment indices, each
/// validated against `n_segments`. `None` on truncation or an out-of-range index.
fn parse_vn_loop(c: &mut VnCursor<'_>, n_segments: usize) -> Option<Vec<usize>> {
    let n_segs = c.u32()?;
    if n_segs > MAX_VN_COUNT {
        return None;
    }
    let mut loop_segs = Vec::with_capacity(n_segs as usize);
    for _ in 0..n_segs {
        let seg_idx = c.u32()? as usize;
        if seg_idx >= n_segments {
            return None;
        }
        loop_segs.push(seg_idx);
    }
    Some(loop_segs)
}

impl VectorNetwork {
    /// Convert this network into a single [`PathData`], scaling vertices and
    /// tangents by `(sx, sy)`. Closed regions become closed sub-paths (and set the
    /// path's fill rule from the first region); a network with no regions falls
    /// back to walking connected open chains. Returns `None` if nothing emitted.
    pub(crate) fn to_path(&self, sx: f64, sy: f64) -> Option<PathData> {
        let mut out = PathData::new();
        if !self.regions.is_empty() {
            let mut fill_rule_set = false;
            for (even_odd, loops) in &self.regions {
                if !fill_rule_set {
                    out.fill_rule = if *even_odd {
                        fanta_doc::path::FillRule::EvenOdd
                    } else {
                        fanta_doc::path::FillRule::NonZero
                    };
                    fill_rule_set = true;
                }
                for lp in loops {
                    self.append_loop(&mut out, lp, sx, sy);
                }
            }
        } else {
            self.append_open_chains(&mut out, sx, sy);
        }
        (!out.segments.is_empty()).then_some(out)
    }

    /// Scaled vertex position.
    fn pt(&self, i: usize, sx: f64, sy: f64) -> (f64, f64) {
        let v = self.vertices[i];
        (v[0] as f64 * sx, v[1] as f64 * sy)
    }

    /// Emit one segment, walking from `current` (the vertex we're standing on).
    /// `forward` is whether the segment runs start→end from `current`. A segment
    /// with all-zero tangents is a straight line; otherwise a cubic with the
    /// tangents as relative control-point offsets (mirrors op1 `addSegmentDirected`
    /// / op2 `emitSegment`). Returns the vertex index we end on.
    fn emit_segment(
        &self,
        out: &mut PathData,
        seg: &VnSegment,
        current: usize,
        sx: f64,
        sy: f64,
    ) -> usize {
        let forward = seg.start == current;
        let (a, b) = if forward {
            (seg.start, seg.end)
        } else {
            (seg.end, seg.start)
        };
        let (p0x, p0y) = self.pt(a, sx, sy);
        let (p3x, p3y) = self.pt(b, sx, sy);
        let is_line = seg.ts == [0.0, 0.0] && seg.te == [0.0, 0.0];
        if is_line {
            out.line_to(p3x, p3y);
        } else {
            // Control 1 sits on the `current` vertex's tangent; control 2 on the
            // far vertex's. When traversing the segment backwards we swap which
            // tangent applies to which end.
            let (t_start, t_end) = if forward {
                (seg.ts, seg.te)
            } else {
                (seg.te, seg.ts)
            };
            let c1x = p0x + t_start[0] as f64 * sx;
            let c1y = p0y + t_start[1] as f64 * sy;
            let c2x = p3x + t_end[0] as f64 * sx;
            let c2y = p3y + t_end[1] as f64 * sy;
            out.cubic_to(c1x, c1y, c2x, c2y, p3x, p3y);
        }
        b
    }

    /// Append a region loop as a closed sub-path. The loop is an ordered list of
    /// segment indices; we pick a start vertex consistent with the second segment
    /// (op1 `addLoopToPath`) so the walk direction is unambiguous.
    fn append_loop(&self, out: &mut PathData, lp: &[usize], sx: f64, sy: f64) {
        if lp.is_empty() {
            return;
        }
        let first = &self.segments[lp[0]];
        let mut current = if lp.len() == 1 {
            first.start
        } else {
            let second = &self.segments[lp[1]];
            if first.end == second.start || first.end == second.end {
                first.start
            } else {
                first.end
            }
        };
        let (mx, my) = self.pt(current, sx, sy);
        out.move_to(mx, my);
        for &seg_idx in lp {
            current = self.emit_segment(out, &self.segments[seg_idx], current, sx, sy);
        }
        out.close();
    }

    /// Append open (non-region) geometry by walking connected segment chains, then
    /// emitting any leftover segments individually. Mirrors op1
    /// `addOpenSegmentsToPath` / `buildChains`.
    fn append_open_chains(&self, out: &mut PathData, sx: f64, sy: f64) {
        if self.segments.is_empty() {
            return;
        }
        // Adjacency: vertex → segment indices touching it.
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, s) in self.segments.iter().enumerate() {
            adj.entry(s.start).or_default().push(i);
            adj.entry(s.end).or_default().push(i);
        }
        let mut visited = vec![false; self.segments.len()];

        // Prefer starting from degree-1 vertices (open-chain endpoints) so a chain
        // is walked end-to-end; if there are none (all closed), start anywhere.
        let mut starts: Vec<usize> = adj
            .iter()
            .filter(|(_, segs)| segs.len() == 1)
            .map(|(v, _)| *v)
            .collect();
        starts.sort_unstable();
        if starts.is_empty() {
            starts.push(self.segments[0].start);
        }

        for start_vertex in starts {
            // Collect the chain reachable from this start, then emit it.
            let mut current = start_vertex;
            let mut chain: Vec<usize> = Vec::new();
            while let Some(segs) = adj.get(&current) {
                let Some(&next) = segs.iter().find(|&&s| !visited[s]) else {
                    break;
                };
                visited[next] = true;
                chain.push(next);
                let seg = &self.segments[next];
                current = if seg.start == current {
                    seg.end
                } else {
                    seg.start
                };
            }
            self.emit_chain(out, &chain, sx, sy);
        }

        // Any segment not reached by a chain walk (e.g. an isolated closed cycle
        // with no degree-1 entry) is emitted on its own.
        for (i, seg) in self.segments.iter().enumerate() {
            if visited[i] {
                continue;
            }
            visited[i] = true;
            let (mx, my) = self.pt(seg.start, sx, sy);
            out.move_to(mx, my);
            self.emit_segment(out, seg, seg.start, sx, sy);
        }
    }

    /// Emit a single chain (already in connection order) as one open sub-path,
    /// starting from the vertex that makes the first segment connect to the
    /// second (op1 `findChainStart`).
    fn emit_chain(&self, out: &mut PathData, chain: &[usize], sx: f64, sy: f64) {
        if chain.is_empty() {
            return;
        }
        let first = &self.segments[chain[0]];
        let mut current = if chain.len() < 2 {
            first.start
        } else {
            let second = &self.segments[chain[1]];
            if first.start == second.start || first.start == second.end {
                first.end
            } else {
                first.start
            }
        };
        let (mx, my) = self.pt(current, sx, sy);
        out.move_to(mx, my);
        for &seg_idx in chain {
            current = self.emit_segment(out, &self.segments[seg_idx], current, sx, sy);
        }
    }
}
