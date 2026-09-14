use std::collections::BTreeMap;
use std::sync::Arc;

use fanta_doc::{AssetId, Color, Doc, DocId, NodeData, NodeFlags, NodeId, Viewport};

use crate::{AssetResolver, DecodedImage, RasterRenderer, RenderError, RenderInputs};

pub const MAX_SAMPLE_DIMENSION: u32 = 8192;
pub const MAX_SAMPLE_RGBA_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SNAPSHOT_IMAGE_BYTES: usize = 256 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16384;
const MAX_SAMPLING_NODES: usize = 100_000;
const MAX_SAMPLING_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum SamplingError {
    #[error("The canvas geometry is not finite or has no usable pixels")]
    InvalidGeometry,
    #[error("The canvas is too large to sample at its current device scale")]
    ViewportTooLarge,
    #[error("The decoded image snapshot is too large to sample")]
    AssetsTooLarge,
    #[error("The artwork is too deeply nested or too complex to sample")]
    TooComplex,
    #[error("Component expansion is too large to sample")]
    ExpansionTooLarge,
    #[error("Image {0} has invalid dimensions or RGBA data")]
    InvalidAsset(AssetId),
    #[error("Only an existing visible ordinary page can be sampled")]
    IneligiblePage,
    #[error("The document scene is invalid: {0}")]
    InvalidScene(#[source] fanta_doc::SceneError),
    #[error("Could not allocate the sampling pixel buffer")]
    Allocation,
    #[error("Could not read the rendered artwork pixels")]
    Readback,
    #[error("Could not render an artwork effect completely")]
    EffectFailed,
    #[error("Artwork contains an unavailable image or unresolved component")]
    IncompleteArtwork,
    #[error("Static artwork sampling does not support media cards or placeholder content")]
    UnsupportedContent,
    #[error("The sample is no longer current")]
    StaleRequest,
    #[error("The pointer is outside the captured canvas pixels")]
    OutsideCanvas,
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// Captures the visible canvas and the symmetric logical margin of its full
/// rendered frame. The viewport is centered in that frame, as in the live
/// canvas. Physical dimensions follow host f32 rounding; no downsampling is used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingGeometry {
    pub canvas_origin: [f64; 2],
    pub canvas_size: [f64; 2],
    /// Capture the actual frame policy (currently 160 on macOS, zero elsewhere).
    /// Reprojected/stale display frames are not accepted artwork snapshots.
    pub render_margin: [f64; 2],
    pub device_scale: f64,
    pub viewport: Viewport,
    /// Capture the displayed renderer's policy: the native GPU canvas snaps
    /// its pan to device pixels, while the CPU fallback currently does not.
    pub pixel_snap_pan: bool,
}

impl SamplingGeometry {
    fn dimensions(self) -> Result<[u32; 2], SamplingError> {
        if !self.canvas_origin.into_iter().all(f64::is_finite)
            || !self
                .canvas_size
                .into_iter()
                .all(|value| value.is_finite() && value > 0.)
            || !self
                .render_margin
                .into_iter()
                .all(|value| value.is_finite() && value >= 0.)
            || !self
                .viewport
                .center
                .into_iter()
                .all(|value| (value as f32).is_finite())
            || !self.viewport.zoom.is_finite()
            || self.viewport.zoom <= 0.
            || !self.device_scale.is_finite()
            || self.device_scale <= 0.
        {
            return Err(SamplingError::InvalidGeometry);
        }
        let effective_scale = (self.viewport.zoom * self.device_scale) as f32;
        if !effective_scale.is_finite()
            || effective_scale <= 0.
            || !(self.device_scale as f32).is_finite()
            || self.device_scale as f32 <= 0.
        {
            return Err(SamplingError::InvalidGeometry);
        }
        let mut dimensions = [0; 2];
        for axis in 0..2 {
            let end = self.canvas_origin[axis] + self.canvas_size[axis];
            if !end.is_finite()
                || end <= self.canvas_origin[axis]
                || !((self.viewport.center[axis] as f32) * effective_scale).is_finite()
            {
                return Err(SamplingError::InvalidGeometry);
            }
            let frame_origin = self.canvas_origin[axis] - self.render_margin[axis];
            let frame_size = self.canvas_size[axis] + 2. * self.render_margin[axis];
            if !frame_origin.is_finite()
                || !frame_size.is_finite()
                || !(frame_origin + frame_size).is_finite()
                || frame_origin + frame_size <= frame_origin
            {
                return Err(SamplingError::InvalidGeometry);
            }
            let physical = ((frame_size as f32) * (self.device_scale as f32))
                .round()
                .max(1.);
            if !physical.is_finite() || physical > MAX_SAMPLE_DIMENSION as f32 {
                return Err(SamplingError::ViewportTooLarge);
            }
            dimensions[axis] = physical as u32;
        }
        rgba_length(dimensions, MAX_SAMPLE_RGBA_BYTES).ok_or(SamplingError::ViewportTooLarge)?;
        Ok(dimensions)
    }

    fn pixel(self, dimensions: [u32; 2], pointer: [f64; 2]) -> Result<[u32; 2], SamplingError> {
        let mut pixel = [0; 2];
        for axis in 0..2 {
            let local = pointer[axis] - self.canvas_origin[axis];
            if !pointer[axis].is_finite()
                || !local.is_finite()
                || local < 0.
                || local >= self.canvas_size[axis]
            {
                return Err(SamplingError::OutsideCanvas);
            }
            let physical = ((local + self.render_margin[axis]) * self.device_scale).floor();
            if !physical.is_finite() || physical < 0. || physical >= f64::from(dimensions[axis]) {
                return Err(SamplingError::OutsideCanvas);
            }
            pixel[axis] = physical as u32;
        }
        Ok(pixel)
    }
}

fn rgba_length(dimensions: [u32; 2], budget: usize) -> Option<usize> {
    if dimensions.contains(&0) {
        return None;
    }
    usize::try_from(dimensions[0])
        .ok()?
        .checked_mul(usize::try_from(dimensions[1]).ok()?)?
        .checked_mul(4)
        .filter(|length| *length <= budget)
}

struct SnapshotAssets(BTreeMap<AssetId, DecodedImage>);

impl SnapshotAssets {
    fn new(images: BTreeMap<AssetId, DecodedImage>) -> Result<Self, SamplingError> {
        let mut bytes = 0usize;
        for (id, image) in &images {
            let dimensions = [image.width, image.height];
            let expected = rgba_length(dimensions, MAX_SNAPSHOT_IMAGE_BYTES)
                .filter(|_| {
                    dimensions
                        .into_iter()
                        .all(|value| value <= MAX_IMAGE_DIMENSION)
                })
                .ok_or(SamplingError::InvalidAsset(*id))?;
            if expected != image.pixels_rgba.len() {
                return Err(SamplingError::InvalidAsset(*id));
            }
            bytes = bytes
                .checked_add(expected)
                .filter(|value| *value <= MAX_SNAPSHOT_IMAGE_BYTES)
                .ok_or(SamplingError::AssetsTooLarge)?;
        }
        Ok(Self(images))
    }
}

impl AssetResolver for SnapshotAssets {
    fn resolve(&self, id: AssetId) -> Option<DecodedImage> {
        self.0.get(&id).cloned()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SamplingWorkBudget {
    visited: usize,
    expanded: usize,
    depth: usize,
    pub(crate) exhausted: bool,
    pub(crate) expansion_exhausted: bool,
}

impl SamplingWorkBudget {
    pub(crate) fn reserve_expanded_node(&mut self) -> bool {
        if self.exhausted {
            return false;
        }
        if self.expanded >= MAX_SAMPLING_NODES {
            self.exhausted = true;
            self.expansion_exhausted = true;
            return false;
        }
        self.expanded += 1;
        true
    }

    pub(crate) fn enter(&mut self) -> bool {
        if self.exhausted || self.visited >= MAX_SAMPLING_NODES || self.depth >= MAX_SAMPLING_DEPTH
        {
            self.exhausted = true;
            return false;
        }
        self.visited += 1;
        self.depth += 1;
        true
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }
}

fn validate_scene(doc: &Doc) -> Result<(), SamplingError> {
    if doc.scene.len() > MAX_SAMPLING_NODES {
        return Err(SamplingError::TooComplex);
    }
    let mut stack: Vec<_> = doc
        .scene
        .roots()
        .iter()
        .map(|id| (*id, None, 1usize))
        .collect();
    let mut visited = 0usize;
    while let Some((id, expected_parent, depth)) = stack.pop() {
        visited += 1;
        let node = doc.scene.get(id).ok_or(SamplingError::InvalidScene(
            fanta_doc::SceneError::NotFound(id),
        ))?;
        // Scene::validate walks actual parent chains. Verify the child index
        // first so the depth bound here also bounds that later validation.
        if node.parent != expected_parent {
            return Err(SamplingError::InvalidScene(
                fanta_doc::SceneError::InvariantViolated(
                    "sampling scene parent differs from child index".into(),
                ),
            ));
        }
        if !node.transform.is_finite() {
            return Err(SamplingError::InvalidGeometry);
        }
        if visited > MAX_SAMPLING_NODES || depth > MAX_SAMPLING_DEPTH {
            return Err(SamplingError::TooComplex);
        }
        stack.extend(
            doc.scene
                .children_of(Some(id))
                .iter()
                .map(|child| (*child, Some(id), depth + 1)),
        );
    }
    if visited != doc.scene.len() {
        return Err(SamplingError::InvalidScene(
            fanta_doc::SceneError::InvariantViolated("unreachable sampling scene nodes".into()),
        ));
    }
    doc.scene.validate().map_err(SamplingError::InvalidScene)
}

#[derive(Debug)]
struct RequestIdentity {
    document: DocId,
    scene_instance: u64,
    scene_revision: u64,
    page: NodeId,
    geometry: SamplingGeometry,
    dimensions: [u32; 2],
}

/// A process-local request identity, not a persistent content fingerprint or
/// permission token. Hosts must replace/drop their current key on tool/context,
/// source, preview, viewport, scale, page, account or item changes and new clicks.
#[derive(Debug, Clone)]
pub struct ArtworkSampleKey(Arc<RequestIdentity>);

impl ArtworkSampleKey {
    pub fn document(&self) -> DocId {
        self.0.document
    }
    pub fn scene_instance(&self) -> u64 {
        self.0.scene_instance
    }
    pub fn scene_revision(&self) -> u64 {
        self.0.scene_revision
    }
    pub fn page(&self) -> NodeId {
        self.0.page
    }
    pub fn geometry(&self) -> SamplingGeometry {
        self.0.geometry
    }
    pub fn dimensions(&self) -> [u32; 2] {
        self.0.dimensions
    }

    pub fn is_same_request(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Owns one accepted document and decoded asset snapshot, so rendering never
/// borrows a live editor or a mutable/lazy resolver. Pass a committed
/// `Doc::clone_for_persist()` after host permission/draft checks, capture `key()`,
/// then move this request to a worker. Rendering consumes it and retains only
/// the pixels/key. A new request always has a new identity, even for equal IDs.
pub struct ArtworkSamplingRequest {
    doc: Doc,
    assets: Arc<SnapshotAssets>,
    key: ArtworkSampleKey,
}

impl ArtworkSamplingRequest {
    pub fn new(
        doc: Doc,
        decoded_images: BTreeMap<AssetId, DecodedImage>,
        page: NodeId,
        geometry: SamplingGeometry,
    ) -> Result<Self, SamplingError> {
        let dimensions = geometry.dimensions()?;
        let root = doc.scene.get(page).ok_or(SamplingError::IneligiblePage)?;
        if !doc.pages.contains(&page)
            || root.parent.is_some()
            || !matches!(root.data, NodeData::Group(_))
            || root.flags.contains(NodeFlags::HIDDEN)
            || root
                .meta
                .get("hidden_page")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            || doc
                .components
                .defs
                .values()
                .any(|definition| definition.root == page)
        {
            return Err(SamplingError::IneligiblePage);
        }
        validate_scene(&doc)?;
        let assets = Arc::new(SnapshotAssets::new(decoded_images)?);
        let key = ArtworkSampleKey(Arc::new(RequestIdentity {
            document: doc.id,
            scene_instance: doc.scene.instance_id(),
            scene_revision: doc.scene.revision(),
            page,
            geometry,
            dimensions,
        }));
        Ok(Self { doc, assets, key })
    }

    pub fn key(&self) -> ArtworkSampleKey {
        self.key.clone()
    }

    pub fn render(self) -> Result<ArtworkSampleFrame, SamplingError> {
        let [width, height] = self.key.dimensions();
        let length = rgba_length([width, height], MAX_SAMPLE_RGBA_BYTES)
            .ok_or(SamplingError::ViewportTooLarge)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(length)
            .map_err(|_| SamplingError::Allocation)?;
        pixels.resize(length, 0);
        let mut renderer = RasterRenderer::new(width, height)?;
        renderer.set_asset_resolver(self.assets);
        // A one-shot snapshot must observe incomplete-content flags on this
        // walk; no cached effect layer may hide a missing dependency.
        renderer.set_layer_cache_enabled(false);
        let geometry = self.key.geometry();
        renderer.set_pixel_snap_pan(geometry.pixel_snap_pan);
        let viewport = Viewport {
            center: geometry.viewport.center,
            zoom: geometry.viewport.zoom * geometry.device_scale,
        };
        renderer.sample_page_rgba(
            &self.doc.scene,
            &viewport,
            self.key.page(),
            &RenderInputs::for_doc(&self.doc),
            &mut pixels,
        )?;
        Ok(ArtworkSampleFrame {
            key: self.key,
            pixels,
        })
    }
}

/// Straight-alpha sRGB document artwork, without editor overlays/background.
/// Pixel access requires the host's still-current request key, including before
/// an explicit Copy. A rejected release must never be clamped to the canvas.
pub struct ArtworkSampleFrame {
    key: ArtworkSampleKey,
    pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtworkColorSample {
    pub color: Color,
    pub device_pixel: [u32; 2],
}

impl ArtworkSampleFrame {
    pub fn sample(
        &self,
        current: &ArtworkSampleKey,
        pointer: [f64; 2],
    ) -> Result<ArtworkColorSample, SamplingError> {
        if !self.key.is_same_request(current) {
            return Err(SamplingError::StaleRequest);
        }
        let dimensions = self.key.dimensions();
        let pixel = self.key.geometry().pixel(dimensions, pointer)?;
        let offset = (pixel[1] as usize)
            .checked_mul(dimensions[0] as usize)
            .and_then(|row| row.checked_add(pixel[0] as usize))
            .and_then(|index| index.checked_mul(4))
            .ok_or(SamplingError::Readback)?;
        let channels = self
            .pixels
            .get(offset..offset.checked_add(4).ok_or(SamplingError::Readback)?)
            .ok_or(SamplingError::Readback)?;
        let [red, green, blue, alpha] =
            <[u8; 4]>::try_from(channels).map_err(|_| SamplingError::Readback)?;
        Ok(ArtworkColorSample {
            color: Color::rgba(red, green, blue, alpha),
            device_pixel: pixel,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        BitmapNode, BlendMode, CanvasNode, ComponentDef, ComponentId, Fill, Gradient, GradientStop,
        GroupNode, ImageFitMode, InstanceNode, NodeGraphNode, Transform2D, VectorNode,
    };

    fn test_geometry() -> SamplingGeometry {
        SamplingGeometry {
            canvas_origin: [10., 20.],
            canvas_size: [32., 32.],
            render_margin: [0., 0.],
            device_scale: 1.,
            viewport: Viewport::default(),
            pixel_snap_pan: true,
        }
    }

    fn document() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let root = doc
            .scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .expect("page");
        doc.add_page(root);
        (doc, root)
    }

    fn insert(doc: &mut Doc, parent: NodeId, data: NodeData) -> NodeId {
        let mut node = CanvasNode::new(data);
        node.parent = Some(parent);
        node.index = doc.scene.next_child_index(Some(parent));
        doc.scene.insert(node).expect("node")
    }

    fn rect(
        doc: &mut Doc,
        parent: NodeId,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: Color,
    ) -> NodeId {
        insert(
            doc,
            parent,
            NodeData::Vector(VectorNode::rect_solid(x, y, width, height, color)),
        )
    }

    fn request(doc: &Doc, page: NodeId, geometry: SamplingGeometry) -> ArtworkSamplingRequest {
        ArtworkSamplingRequest::new(doc.clone_for_persist(), BTreeMap::new(), page, geometry)
            .expect("request")
    }

    fn render(request: ArtworkSamplingRequest) -> (ArtworkSampleKey, ArtworkSampleFrame) {
        let key = request.key();
        (key, request.render().expect("render"))
    }

    fn color(frame: &ArtworkSampleFrame, key: &ArtworkSampleKey, pointer: [f64; 2]) -> Color {
        frame.sample(key, pointer).expect("sample").color
    }

    #[test]
    fn sampling_is_send_without_sharing_the_non_sync_live_scene() {
        fn assert_send<T: Send>() {}
        assert_send::<ArtworkSamplingRequest>();
        assert_send::<ArtworkSampleFrame>();
        assert_send::<ArtworkSampleKey>();
    }

    #[test]
    fn sampling_returns_rgba_red_and_blue_with_explicit_transparency() {
        let (mut doc, page) = document();
        rect(&mut doc, page, -12., -8., 8., 16., Color::rgb(255, 0, 0));
        rect(&mut doc, page, 4., -8., 8., 16., Color::rgb(0, 0, 255));
        let (key, frame) = render(request(&doc, page, test_geometry()));
        assert_eq!(color(&frame, &key, [18., 36.]), Color::rgb(255, 0, 0));
        assert_eq!(color(&frame, &key, [34., 36.]).to_hex(), "#0000FF");
        assert_eq!(color(&frame, &key, [26., 36.]).to_hex(), "#00000000");
    }

    #[test]
    fn sampling_unpremultiplies_and_composites_the_authored_page_background() {
        let (mut doc, page) = document();
        rect(
            &mut doc,
            page,
            -8.,
            -8.,
            16.,
            16.,
            Color::rgba(255, 0, 0, 128),
        );
        let (key, transparent_frame) = render(request(&doc, page, test_geometry()));
        assert_eq!(
            color(&transparent_frame, &key, [26., 36.]),
            Color::rgba(255, 0, 0, 128)
        );
        if let NodeData::Group(group) = &mut doc.scene.get_mut(page).expect("page").data {
            group.background = Some(Fill::solid(Color::rgb(0, 0, 255)));
        }
        let (key, blue_frame) = render(request(&doc, page, test_geometry()));
        assert_eq!(color(&blue_frame, &key, [11., 21.]), Color::rgb(0, 0, 255));
        let mixed = color(&blue_frame, &key, [26., 36.]);
        assert!((127..=129).contains(&mixed.r), "{mixed:?}");
        assert_eq!(mixed.g, 0);
        assert!((126..=128).contains(&mixed.b), "{mixed:?}");
        assert_eq!(mixed.a, 255);
    }

    #[test]
    fn sampling_metadata_and_selection_never_paint_or_mutate_artwork() {
        let (mut doc, page) = document();
        let shape = rect(&mut doc, page, -8., -8., 16., 16., Color::rgb(0, 255, 0));
        let (before_key, before) = render(request(&doc, page, test_geometry()));
        doc.scene.get_mut(page).expect("page").meta = serde_json::json!({
            "measurements": [{"start": [-200., -200.], "end": [200., 200.]}],
            "annotations": [{"anchor": [0., 0.], "text": "must not paint"}],
            "comments": [{"world": [0., 0.], "text": "must not paint"}]
        });
        doc.selection.select_only(shape);
        let baseline = serde_json::to_value(&doc).expect("document JSON");
        let (key, after) = render(request(&doc, page, test_geometry()));
        assert_eq!(before.pixels, after.pixels);
        assert_eq!(color(&after, &key, [26., 36.]), Color::rgb(0, 255, 0));
        assert!(matches!(
            after.sample(&before_key, [26., 36.]),
            Err(SamplingError::StaleRequest)
        ));
        assert_eq!(serde_json::to_value(&doc).expect("document JSON"), baseline);
    }

    #[test]
    fn sampling_rotated_translated_page_pan_zoom_and_retina_use_world_geometry() {
        let (mut doc, page) = document();
        doc.scene.get_mut(page).expect("page").transform =
            Transform2D::rotation(std::f64::consts::FRAC_PI_2)
                .then(&Transform2D::translation(120., -80.));
        rect(&mut doc, page, 0., 0., 8., 4., Color::rgb(0, 0, 255));
        let mut geometry = test_geometry();
        geometry.device_scale = 2.;
        geometry.viewport = Viewport {
            center: [118., -76.],
            zoom: 2.,
        };
        let (key, frame) = render(request(&doc, page, geometry));
        assert_eq!(key.dimensions(), [64, 64]);
        let sample = frame.sample(&key, [26., 36.]).expect("center");
        assert_eq!(sample.device_pixel, [32, 32]);
        assert_eq!(sample.color, Color::rgb(0, 0, 255));
        assert_eq!(color(&frame, &key, [11., 21.]), Color::TRANSPARENT);
        geometry.viewport.center = [114., -76.];
        let (panned_key, panned) = render(request(&doc, page, geometry));
        assert_eq!(
            panned
                .sample(&panned_key, [34., 36.])
                .expect("panned point")
                .device_pixel,
            [48, 32]
        );
        assert_eq!(
            color(&panned, &panned_key, [34., 36.]),
            Color::rgb(0, 0, 255)
        );
        assert!(matches!(
            frame.sample(&panned_key, [26., 36.]),
            Err(SamplingError::StaleRequest)
        ));
    }

    #[test]
    fn sampling_uses_containing_device_pixel_and_rejects_outside_or_rounded_away_edges() {
        let (doc, page) = document();
        let mut geometry = test_geometry();
        geometry.canvas_size = [3.1, 2.];
        let (key, frame) = render(request(&doc, page, geometry));
        assert_eq!(key.dimensions(), [3, 2]);
        assert_eq!(
            frame.sample(&key, [10., 20.]).expect("first").device_pixel,
            [0, 0]
        );
        assert_eq!(
            frame
                .sample(&key, [12.99, 21.99])
                .expect("last")
                .device_pixel,
            [2, 1]
        );
        for pointer in [
            [9.99, 20.],
            [13.1, 20.],
            [10., 22.],
            [13.05, 20.],
            [f64::NAN, 20.],
            [10., f64::INFINITY],
        ] {
            assert!(
                matches!(
                    frame.sample(&key, pointer),
                    Err(SamplingError::OutsideCanvas)
                ),
                "{pointer:?}"
            );
        }
        geometry.device_scale = 2.;
        let (key, frame) = render(request(&doc, page, geometry));
        assert_eq!(
            frame
                .sample(&key, [10.75, 20.25])
                .expect("retina pixel")
                .device_pixel,
            [1, 0]
        );
    }

    #[test]
    fn sampling_rejects_nonfinite_degenerate_and_over_budget_geometry_before_render() {
        for value in [0., -1., f64::NAN, f64::INFINITY] {
            let mut geometry = test_geometry();
            geometry.device_scale = value;
            assert!(matches!(
                geometry.dimensions(),
                Err(SamplingError::InvalidGeometry)
            ));
            let mut geometry = test_geometry();
            geometry.viewport.zoom = value;
            assert!(matches!(
                geometry.dimensions(),
                Err(SamplingError::InvalidGeometry)
            ));
        }
        let mut geometry = test_geometry();
        geometry.canvas_origin = [f64::MAX, 0.];
        assert!(matches!(
            geometry.dimensions(),
            Err(SamplingError::InvalidGeometry)
        ));
        geometry = SamplingGeometry {
            canvas_size: [8193., 1.],
            ..test_geometry()
        };
        assert!(matches!(
            geometry.dimensions(),
            Err(SamplingError::ViewportTooLarge)
        ));
        geometry.canvas_size = [8192., 8192.];
        assert!(matches!(
            geometry.dimensions(),
            Err(SamplingError::ViewportTooLarge)
        ));
        assert_eq!(
            rgba_length([u32::MAX, u32::MAX], MAX_SAMPLE_RGBA_BYTES),
            None
        );
    }

    fn bitmap(asset: AssetId) -> NodeData {
        NodeData::Bitmap(BitmapNode {
            asset,
            natural_size: [1, 1],
            local_size: [16., 16.],
            crop: None,
            fit: ImageFitMode::Fill,
            tint: None,
        })
    }

    fn image(color: [u8; 4]) -> DecodedImage {
        DecodedImage::new(Arc::new(color.to_vec()), 1, 1)
    }

    #[test]
    fn sampling_freezes_asset_pixels_and_source_and_rejects_previous_request_identity() {
        let (mut doc, page) = document();
        let asset = AssetId::new();
        let shape = insert(&mut doc, page, bitmap(asset));
        let mut pixels = image([255, 0, 0, 255]);
        let first = ArtworkSamplingRequest::new(
            doc.clone_for_persist(),
            BTreeMap::from([(asset, pixels.clone())]),
            page,
            test_geometry(),
        )
        .expect("first");
        Arc::make_mut(&mut pixels.pixels_rgba).copy_from_slice(&[0, 0, 255, 255]);
        let second = ArtworkSamplingRequest::new(
            doc.clone_for_persist(),
            BTreeMap::from([(asset, pixels)]),
            page,
            test_geometry(),
        )
        .expect("second");
        doc.scene
            .get_mut(shape)
            .expect("shape")
            .flags
            .insert(NodeFlags::HIDDEN);
        let (first_key, first_frame) = render(first);
        let (second_key, second_frame) = render(second);
        assert_eq!(first_key.document(), second_key.document());
        assert_eq!(first_key.page(), second_key.page());
        assert_eq!(
            color(&first_frame, &first_key, [30., 40.]),
            Color::rgb(255, 0, 0)
        );
        assert_eq!(
            color(&second_frame, &second_key, [30., 40.]),
            Color::rgb(0, 0, 255)
        );
        assert!(matches!(
            first_frame.sample(&second_key, [30., 40.]),
            Err(SamplingError::StaleRequest)
        ));
        assert!(
            doc.scene
                .get(shape)
                .expect("shape")
                .flags
                .contains(NodeFlags::HIDDEN)
        );
    }

    #[test]
    fn sampling_missing_image_fails_instead_of_returning_placeholder_color() {
        let (mut doc, page) = document();
        insert(&mut doc, page, bitmap(AssetId::new()));
        assert!(matches!(
            request(&doc, page, test_geometry()).render(),
            Err(SamplingError::IncompleteArtwork)
        ));
    }

    #[test]
    fn sampling_invalid_and_over_budget_asset_snapshots_are_explicit_errors() {
        let id = AssetId::new();
        for malformed in [
            DecodedImage::new(Arc::new(vec![0; 3]), 1, 1),
            DecodedImage::new(Arc::new(Vec::new()), 0, 0),
            DecodedImage::new(Arc::new(Vec::new()), u32::MAX, 1),
        ] {
            assert!(
                matches!(SnapshotAssets::new(BTreeMap::from([(id, malformed)])), Err(SamplingError::InvalidAsset(found)) if found == id)
            );
        }
        let shared = DecodedImage::new(Arc::new(vec![0; 1024 * 1024]), 512, 512);
        let images = (0..257)
            .map(|index| (AssetId::from_u128(index), shared.clone()))
            .collect();
        assert!(matches!(
            SnapshotAssets::new(images),
            Err(SamplingError::AssetsTooLarge)
        ));
    }

    #[test]
    fn sampling_gradient_and_clip_are_composited_by_the_artwork_renderer() {
        let (mut doc, page) = document();
        let clip = insert(
            &mut doc,
            page,
            NodeData::Group(GroupNode {
                clip_size: Some([8., 8.]),
                ..Default::default()
            }),
        );
        let mut vector = VectorNode::rect_solid(0., 0., 16., 8., Color::BLACK);
        vector.fills = [Fill::Gradient {
            gradient: Gradient::Linear {
                start: [0., 0.],
                end: [1., 0.],
                stops: vec![
                    GradientStop {
                        position: 0.,
                        color: Color::rgb(255, 0, 0),
                    },
                    GradientStop {
                        position: 1.,
                        color: Color::rgb(0, 0, 255),
                    },
                ],
            },
            blend: BlendMode::Normal,
        }]
        .into_iter()
        .collect();
        insert(&mut doc, clip, NodeData::Vector(vector));
        let (key, frame) = render(request(&doc, page, test_geometry()));
        let middle = color(&frame, &key, [30., 40.]);
        assert!(
            middle.r > middle.b && middle.b > 50 && middle.r < 220,
            "{middle:?}"
        );
        assert_eq!(middle.a, 255);
        assert_eq!(color(&frame, &key, [36., 40.]), Color::TRANSPARENT);
    }

    #[test]
    fn sampling_component_expands_at_instance_position_and_missing_master_fails() {
        let (mut doc, page) = document();
        let master = rect(&mut doc, page, 0., 0., 16., 16., Color::rgb(0, 255, 0));
        doc.scene.get_mut(master).expect("master").transform =
            Transform2D::translation(1000., 1000.);
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Green"));
        insert(
            &mut doc,
            page,
            NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [16., 16.],
            }),
        );
        let (key, frame) = render(request(&doc, page, test_geometry()));
        assert_eq!(color(&frame, &key, [30., 40.]), Color::rgb(0, 255, 0));
        doc.components.defs.clear();
        assert!(matches!(
            request(&doc, page, test_geometry()).render(),
            Err(SamplingError::IncompleteArtwork)
        ));
    }

    #[test]
    fn sampling_placeholder_content_and_hidden_or_non_page_roots_are_unavailable() {
        let (mut doc, page) = document();
        let graph = insert(
            &mut doc,
            page,
            NodeData::NodeGraph(NodeGraphNode {
                local_size: [16., 16.],
                graph: Default::default(),
                preview: None,
            }),
        );
        assert!(matches!(
            request(&doc, page, test_geometry()).render(),
            Err(SamplingError::UnsupportedContent)
        ));
        assert!(matches!(
            ArtworkSamplingRequest::new(
                doc.clone_for_persist(),
                BTreeMap::new(),
                graph,
                test_geometry()
            ),
            Err(SamplingError::IneligiblePage)
        ));
        doc.scene.get_mut(page).expect("page").meta = serde_json::json!({"hidden_page": true});
        assert!(matches!(
            ArtworkSamplingRequest::new(
                doc.clone_for_persist(),
                BTreeMap::new(),
                page,
                test_geometry()
            ),
            Err(SamplingError::IneligiblePage)
        ));
        doc.scene.get_mut(page).expect("page").meta = serde_json::Value::Null;
        doc.scene
            .get_mut(page)
            .expect("page")
            .flags
            .insert(NodeFlags::HIDDEN);
        assert!(matches!(
            ArtworkSamplingRequest::new(doc, BTreeMap::new(), page, test_geometry()),
            Err(SamplingError::IneligiblePage)
        ));
    }

    #[test]
    fn sampling_fallible_readback_rejects_short_buffer_and_surface_creation_error() {
        let (doc, page) = document();
        let mut renderer = RasterRenderer::new(32, 32).expect("renderer");
        assert!(matches!(
            renderer.sample_page_rgba(
                &doc.scene,
                &Viewport::default(),
                page,
                &RenderInputs::for_doc(&doc),
                &mut [0; 4]
            ),
            Err(SamplingError::Readback)
        ));
        assert!(matches!(
            RasterRenderer::new(0, 32),
            Err(RenderError::SurfaceCreate { .. })
        ));
    }

    #[test]
    fn sampling_variable_mode_is_frozen_and_resolved_without_changing_literal_paint() {
        use fanta_doc::{
            BoundProp, Mode, ModeId, VarValue, Variable, VariableCollection, VariableCollectionId,
            VariableId, VariableType,
        };
        let (mut doc, page) = document();
        let shape = rect(&mut doc, page, -8., -8., 16., 16., Color::rgb(255, 0, 0));
        let collection = VariableCollectionId::new();
        let light = ModeId::new();
        let dark = ModeId::new();
        let variable = VariableId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Theme".into(),
                modes: vec![
                    Mode {
                        id: light,
                        name: "Light".into(),
                    },
                    Mode {
                        id: dark,
                        name: "Dark".into(),
                    },
                ],
                default_mode: light,
                variable_order: vec![variable],
            },
        );
        doc.variables.variables.insert(
            variable,
            Variable {
                id: variable,
                collection,
                name: "surface".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([
                    (
                        light,
                        VarValue::Color {
                            value: Color::WHITE,
                        },
                    ),
                    (
                        dark,
                        VarValue::Color {
                            value: Color::rgb(0, 0, 255),
                        },
                    ),
                ]),
                scopes: Vec::new(),
            },
        );
        doc.scene
            .get_mut(shape)
            .expect("shape")
            .bindings
            .insert(BoundProp::FillColor { index: 0 }, variable);
        let light_request = request(&doc, page, test_geometry());
        doc.active_modes.insert(collection, dark);
        let dark_request = request(&doc, page, test_geometry());
        let baseline = serde_json::to_value(&doc).expect("source");
        let (light_key, light_frame) = render(light_request);
        let (dark_key, dark_frame) = render(dark_request);
        assert_eq!(color(&light_frame, &light_key, [26., 36.]), Color::WHITE);
        assert_eq!(
            color(&dark_frame, &dark_key, [26., 36.]),
            Color::rgb(0, 0, 255)
        );
        assert_eq!(serde_json::to_value(&doc).expect("source"), baseline);
        assert!(matches!(
            light_frame.sample(&dark_key, [26., 36.]),
            Err(SamplingError::StaleRequest)
        ));
    }

    #[test]
    fn sampling_missing_image_paints_on_vectors_frames_and_strokes_are_errors() {
        let missing = Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 1.,
            crop: None,
            scale: None,
            rotation: None,
            blend: BlendMode::Normal,
            adjust: Default::default(),
        };
        for kind in 0..3 {
            let (mut doc, page) = document();
            if kind == 0 {
                insert(
                    &mut doc,
                    page,
                    NodeData::Group(GroupNode {
                        local_size: Some([16., 16.]),
                        background: Some(missing.clone()),
                        ..Default::default()
                    }),
                );
            } else {
                let mut vector = VectorNode::rect_solid(0., 0., 16., 16., Color::WHITE);
                if kind == 1 {
                    vector.fills = [missing.clone()].into_iter().collect();
                } else {
                    let mut stroke = fanta_doc::Stroke::solid(Color::BLACK, 2.);
                    stroke.paint = missing.clone();
                    vector.strokes.push(stroke);
                }
                insert(&mut doc, page, NodeData::Vector(vector));
            }
            assert!(
                matches!(
                    request(&doc, page, test_geometry()).render(),
                    Err(SamplingError::IncompleteArtwork)
                ),
                "kind {kind}"
            );
        }
    }

    #[test]
    fn sampling_refuses_deep_scene_and_bounds_recursive_component_expansion() {
        let (mut doc, page) = document();
        let mut parent = page;
        for _ in 0..MAX_SAMPLING_DEPTH {
            parent = insert(&mut doc, parent, NodeData::Group(GroupNode::default()));
        }
        assert!(matches!(
            ArtworkSamplingRequest::new(doc, BTreeMap::new(), page, test_geometry()),
            Err(SamplingError::TooComplex)
        ));

        let (mut doc, page) = document();
        let master = insert(
            &mut doc,
            page,
            NodeData::Group(GroupNode {
                local_size: Some([16., 16.]),
                ..Default::default()
            }),
        );
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Recursive"));
        insert(
            &mut doc,
            master,
            NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [16., 16.],
            }),
        );
        assert!(matches!(
            request(&doc, page, test_geometry()).render(),
            Err(SamplingError::TooComplex)
        ));
    }

    #[test]
    fn sampling_rejects_nonfinite_page_transform_before_it_can_look_transparent() {
        let (mut doc, page) = document();
        doc.scene.get_mut(page).expect("page").transform = Transform2D::translation(f64::NAN, 0.);
        assert!(matches!(
            ArtworkSamplingRequest::new(doc, BTreeMap::new(), page, test_geometry()),
            Err(SamplingError::InvalidGeometry)
        ));
    }

    #[test]
    fn sampling_fractional_pan_matches_captured_gpu_and_cpu_snap_policies() {
        let (mut doc, page) = document();
        rect(&mut doc, page, 0., -8., 8., 16., Color::rgb(255, 0, 0));
        let mut frames = Vec::new();
        for pixel_snap_pan in [false, true] {
            let geometry = SamplingGeometry {
                device_scale: 1.5,
                viewport: Viewport {
                    center: [0.2, 0.35],
                    zoom: 1.3,
                },
                pixel_snap_pan,
                ..test_geometry()
            };
            let (key, frame) = render(request(&doc, page, geometry));
            let mut reference = RasterRenderer::new(48, 48).expect("full viewport");
            reference.set_pixel_snap_pan(pixel_snap_pan);
            reference.render_page_with(
                &doc.scene,
                &Viewport {
                    center: [0.2, 0.35],
                    zoom: 1.3 * 1.5,
                },
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            assert_eq!(frame.pixels, reference.copy_rgba());
            let edge = frame.sample(&key, [25.5, 36.]).expect("edge pixel");
            assert_eq!(edge.device_pixel, [23, 24]);
            if pixel_snap_pan {
                assert_eq!(edge.color, Color::TRANSPARENT);
            } else {
                assert_eq!(edge.color.r, 255);
                assert!(edge.color.a > 0 && edge.color.a < 255, "{edge:?}");
            }
            frames.push(frame.pixels);
        }
        assert_ne!(frames[0], frames[1]);
    }

    #[test]
    fn sampling_full_frame_margin_preserves_edge_effects_but_is_not_clickable() {
        let (mut doc, page) = document();
        rect(&mut doc, page, -40., -30., 24., 60., Color::rgb(0, 0, 255));
        rect(&mut doc, page, -16., -30., 56., 60., Color::rgb(255, 0, 0));
        let frost = rect(&mut doc, page, -20., -10., 12., 20., Color::TRANSPARENT);
        doc.scene
            .get_mut(frost)
            .expect("frost")
            .blurs
            .push(fanta_doc::Blur::background(12.));
        let translucent = rect(&mut doc, page, -14., -4., 8., 8., Color::WHITE);
        doc.scene.get_mut(translucent).expect("translucent").opacity =
            fanta_doc::UnitInterval::new(0.25);
        for pixel_snap_pan in [false, true] {
            let geometry = SamplingGeometry {
                render_margin: [160., 160.],
                device_scale: f64::from(1.3333334_f32),
                viewport: Viewport {
                    center: [0.2, 0.35],
                    zoom: 1.,
                },
                pixel_snap_pan,
                ..test_geometry()
            };
            let (key, frame) = render(request(&doc, page, geometry));
            assert_eq!(key.dimensions(), [469, 469]);
            let mut reference = RasterRenderer::new(469, 469).expect("margin frame");
            reference.set_pixel_snap_pan(pixel_snap_pan);
            reference.render_page_with(
                &doc.scene,
                &Viewport {
                    center: [0.2, 0.35],
                    zoom: geometry.device_scale,
                },
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            let reference_pixels = reference.copy_rgba();
            assert_eq!(frame.pixels, reference_pixels);
            let edge = frame.sample(&key, [10.25, 36.]).expect("visible edge");
            assert_eq!(edge.device_pixel, [213, 234]);
            let offset = (234 * 469 + 213) * 4;
            assert_eq!(
                &reference_pixels[offset..offset + 4],
                &[edge.color.r, edge.color.g, edge.color.b, edge.color.a]
            );
            assert!(
                edge.color.r > 0 && edge.color.b > 0,
                "blurred edge: {edge:?}"
            );
            for pointer in [[9.99, 36.], [42., 36.], [26., 19.99], [26., 52.]] {
                assert!(matches!(
                    frame.sample(&key, pointer),
                    Err(SamplingError::OutsideCanvas)
                ));
            }
            let (_, cropped) = render(request(
                &doc,
                page,
                SamplingGeometry {
                    render_margin: [0., 0.],
                    ..geometry
                },
            ));
            let cropped_edge = ((16. * geometry.device_scale).floor() as usize * 43) * 4;
            assert_ne!(
                &cropped.pixels[cropped_edge..cropped_edge + 4],
                &[edge.color.r, edge.color.g, edge.color.b, edge.color.a],
                "omitting the margin loses the blue backdrop at the visible edge"
            );
        }
        for render_margin in [[-1., 0.], [f64::NAN, 0.], [f64::INFINITY, 0.]] {
            assert!(matches!(
                SamplingGeometry {
                    render_margin,
                    ..test_geometry()
                }
                .dimensions(),
                Err(SamplingError::InvalidGeometry)
            ));
        }
        assert!(matches!(
            SamplingGeometry {
                render_margin: [4096., 4096.],
                ..test_geometry()
            }
            .dimensions(),
            Err(SamplingError::ViewportTooLarge)
        ));
    }

    #[test]
    fn sampling_alpha_mask_and_overlapping_opacity_have_known_pixels() {
        let (mut doc, page) = document();
        let group = insert(&mut doc, page, NodeData::Group(GroupNode::default()));
        let mask = rect(&mut doc, group, -8., -8., 8., 16., Color::WHITE);
        doc.scene.get_mut(mask).expect("mask").is_mask = true;
        doc.scene.get_mut(mask).expect("mask").mask_type = fanta_doc::MaskType::Alpha;
        rect(&mut doc, group, -8., -8., 16., 16., Color::rgb(255, 0, 0));
        let blue = rect(&mut doc, group, -4., -8., 12., 16., Color::rgb(0, 0, 255));
        doc.scene.get_mut(blue).expect("blue").opacity = fanta_doc::UnitInterval::new(0.5);
        let (key, frame) = render(request(&doc, page, test_geometry()));
        assert_eq!(color(&frame, &key, [20., 36.]), Color::rgb(255, 0, 0));
        let overlap = color(&frame, &key, [24., 36.]);
        assert!(
            (126..=129).contains(&overlap.r) && (126..=129).contains(&overlap.b),
            "{overlap:?}"
        );
        assert_eq!(overlap.g, 0);
        assert_eq!(overlap.a, 255);
        assert_eq!(color(&frame, &key, [30., 36.]), Color::TRANSPARENT);
    }

    #[test]
    fn sampling_transient_component_boolean_is_explicitly_unsupported() {
        let (mut doc, page) = document();
        let master = insert(
            &mut doc,
            page,
            NodeData::Group(GroupNode {
                local_size: Some([16., 16.]),
                ..Default::default()
            }),
        );
        doc.scene.get_mut(master).expect("master").transform =
            Transform2D::translation(1000., 1000.);
        let boolean = insert(
            &mut doc,
            master,
            NodeData::Boolean(fanta_doc::BooleanNode::default()),
        );
        rect(&mut doc, boolean, 0., 0., 16., 16., Color::WHITE);
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Boolean"));
        insert(
            &mut doc,
            page,
            NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [16., 16.],
            }),
        );
        assert!(matches!(
            request(&doc, page, test_geometry()).render(),
            Err(SamplingError::UnsupportedContent)
        ));
    }

    #[test]
    fn sampling_reserves_wide_recursive_master_before_cloning_or_layout() {
        let (mut doc, page) = document();
        let master = insert(
            &mut doc,
            page,
            NodeData::Group(GroupNode {
                local_size: Some([16., 16.]),
                ..Default::default()
            }),
        );
        let component = ComponentId::new();
        doc.components.defs.insert(
            component,
            ComponentDef::new(component, master, "Wide recursive"),
        );
        insert(
            &mut doc,
            master,
            NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [16., 16.],
            }),
        );
        for _ in 0..5000 {
            insert(&mut doc, master, NodeData::Group(GroupNode::default()));
        }
        // A depth-only traversal guard reaches this recursive first child long
        // before the remaining 5,000 cloned siblings, retaining a wide clone at
        // each level. The expansion quota must be the first refusal instead.
        assert!(matches!(
            request(&doc, page, test_geometry()).render(),
            Err(SamplingError::ExpansionTooLarge)
        ));
    }

    #[test]
    fn sampling_rejects_parent_chains_that_disagree_with_the_traversal_index() {
        for cyclic in [false, true] {
            let (mut doc, page) = document();
            let children: Vec<_> = (0..2000)
                .map(|_| insert(&mut doc, page, NodeData::Group(GroupNode::default())))
                .collect();
            for pair in children.windows(2) {
                doc.scene.get_mut(pair[1]).expect("child").parent = Some(pair[0]);
            }
            if cyclic {
                doc.scene.get_mut(children[0]).expect("first").parent = children.last().copied();
            }
            assert!(matches!(
                ArtworkSamplingRequest::new(doc, BTreeMap::new(), page, test_geometry()),
                Err(SamplingError::InvalidScene(_))
            ));
        }
    }
}
