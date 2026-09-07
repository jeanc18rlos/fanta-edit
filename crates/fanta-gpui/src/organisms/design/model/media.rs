use std::path::PathBuf;

use gpui::SharedString;

use super::{
    DesignPaintTransform, DesignPaintType, DesignPanelCollection, DesignPanelEditPhase,
    paint_transform_is_finite,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPaintSource {
    pub id: SharedString,
    pub name: SharedString,
    pub mime_type: Option<SharedString>,
    /// Opaque host payload, such as an asset reference or content hash.
    pub reference: Option<SharedString>,
}

impl DesignPaintSource {
    pub fn new(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            mime_type: None,
            reference: None,
        }
    }
}

impl Default for DesignPaintSource {
    fn default() -> Self {
        Self::new("", "No source")
    }
}

/// The media payload currently occupying one Image or Video paint row.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignMediaKind {
    Image,
    Video,
}

impl DesignMediaKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Video => "Video",
        }
    }
}

/// One external file format accepted by Figma's Image/Video fill workflow.
///
/// JPEG represents both `.jpg` and `.jpeg`; TIFF represents both `.tif` and
/// `.tiff`. TIFF is intentionally absent from
/// [`DesignMediaFileKinds::STANDARD`] so a host must opt into it explicitly.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignMediaFileKind {
    Jpeg,
    Png,
    Heic,
    Webp,
    Gif,
    Tiff,
    Mp4,
    Mov,
    Webm,
}

impl DesignMediaFileKind {
    pub const ALL: [Self; 9] = [
        Self::Jpeg,
        Self::Png,
        Self::Heic,
        Self::Webp,
        Self::Gif,
        Self::Tiff,
        Self::Mp4,
        Self::Mov,
        Self::Webm,
    ];

    pub const fn media_kind(self) -> DesignMediaKind {
        match self {
            Self::Jpeg | Self::Png | Self::Heic | Self::Webp | Self::Gif | Self::Tiff => {
                DesignMediaKind::Image
            }
            Self::Mp4 | Self::Mov | Self::Webm => DesignMediaKind::Video,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Heic => "HEIC",
            Self::Webp => "WebP",
            Self::Gif => "GIF",
            Self::Tiff => "TIFF",
            Self::Mp4 => "MP4",
            Self::Mov => "MOV",
            Self::Webm => "WebM",
        }
    }

    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Heic => "image/heic",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Tiff => "image/tiff",
            Self::Mp4 => "video/mp4",
            Self::Mov => "video/quicktime",
            Self::Webm => "video/webm",
        }
    }

    /// Classifies a path by its final extension, case-insensitively.
    ///
    /// SVG, PDF, extensionless paths, and unknown extensions are deliberately
    /// rejected rather than coerced into an Image paint. This is a syntactic
    /// classification only; the receiving host still validates that the path
    /// is a readable file whose content matches the declared format.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "heic" => Some(Self::Heic),
            "webp" => Some(Self::Webp),
            "gif" => Some(Self::Gif),
            "tif" | "tiff" => Some(Self::Tiff),
            "mp4" => Some(Self::Mp4),
            "mov" => Some(Self::Mov),
            "webm" => Some(Self::Webm),
            _ => None,
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// Copyable capability bitset for external Image/Video fill drops.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct DesignMediaFileKinds(u16);

impl DesignMediaFileKinds {
    pub const NONE: Self = Self(0);
    pub const JPEG: Self = Self(DesignMediaFileKind::Jpeg.bit());
    pub const PNG: Self = Self(DesignMediaFileKind::Png.bit());
    pub const HEIC: Self = Self(DesignMediaFileKind::Heic.bit());
    pub const WEBP: Self = Self(DesignMediaFileKind::Webp.bit());
    pub const GIF: Self = Self(DesignMediaFileKind::Gif.bit());
    pub const TIFF: Self = Self(DesignMediaFileKind::Tiff.bit());
    pub const MP4: Self = Self(DesignMediaFileKind::Mp4.bit());
    pub const MOV: Self = Self(DesignMediaFileKind::Mov.bit());
    pub const WEBM: Self = Self(DesignMediaFileKind::Webm.bit());
    pub const STANDARD_IMAGES: Self =
        Self(Self::JPEG.0 | Self::PNG.0 | Self::HEIC.0 | Self::WEBP.0 | Self::GIF.0);
    pub const STANDARD_VIDEOS: Self = Self(Self::MP4.0 | Self::MOV.0 | Self::WEBM.0);
    pub const STANDARD: Self = Self(Self::STANDARD_IMAGES.0 | Self::STANDARD_VIDEOS.0);
    pub const ALL: Self = Self(Self::STANDARD.0 | Self::TIFF.0);

    pub const fn from_kind(kind: DesignMediaFileKind) -> Self {
        Self(kind.bit())
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, kind: DesignMediaFileKind) -> bool {
        self.0 & kind.bit() != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn for_media_kind(kind: DesignMediaKind) -> Self {
        match kind {
            DesignMediaKind::Image => Self::STANDARD_IMAGES,
            DesignMediaKind::Video => Self::STANDARD_VIDEOS,
        }
    }
}

impl std::ops::BitOr for DesignMediaFileKinds {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for DesignMediaFileKinds {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// The one external file carried by a media-source drop intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignMediaDroppedFile {
    pub path: PathBuf,
    pub kind: DesignMediaFileKind,
}

impl DesignMediaDroppedFile {
    pub fn new(path: PathBuf, kind: DesignMediaFileKind) -> Self {
        Self { path, kind }
    }

    /// Accepts exactly one classified path and rejects every other drop.
    pub fn from_paths(paths: &[PathBuf], accepted: DesignMediaFileKinds) -> Option<Self> {
        let [path] = paths else {
            return None;
        };
        let kind = DesignMediaFileKind::from_path(path)?;
        accepted
            .contains(kind)
            .then(|| Self::new(path.clone(), kind))
    }

    pub const fn media_kind(&self) -> DesignMediaKind {
        self.kind.media_kind()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignMediaPaintScaleMode {
    Fill,
    Fit,
    Crop,
    Tile,
}

impl DesignMediaPaintScaleMode {
    pub const ALL: [Self; 4] = [Self::Fill, Self::Fit, Self::Crop, Self::Tile];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Fit => "Fit",
            Self::Crop => "Crop",
            Self::Tile => "Tile",
        }
    }
}

/// The quarter-turn rotation supported by Figma image/video paints outside
/// crop mode.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignMediaQuarterTurn {
    #[default]
    None,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}

impl DesignMediaQuarterTurn {
    pub const ALL: [Self; 4] = [
        Self::None,
        Self::Clockwise90,
        Self::Clockwise180,
        Self::Clockwise270,
    ];

    pub const fn degrees(self) -> u16 {
        match self {
            Self::None => 0,
            Self::Clockwise90 => 90,
            Self::Clockwise180 => 180,
            Self::Clockwise270 => 270,
        }
    }

    pub const fn rotated_clockwise(self) -> Self {
        match self {
            Self::None => Self::Clockwise90,
            Self::Clockwise90 => Self::Clockwise180,
            Self::Clockwise180 => Self::Clockwise270,
            Self::Clockwise270 => Self::None,
        }
    }
}

/// A mode-discriminated image/video placement matching Figma's Paint API.
///
/// Crop owns the only arbitrary affine transform. Fill/Fit/Tile own only a
/// quarter-turn rotation, and Tile alone owns a scaling factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignMediaPaintPlacement {
    Fill {
        rotation: DesignMediaQuarterTurn,
    },
    Fit {
        rotation: DesignMediaQuarterTurn,
    },
    Crop {
        transform: DesignPaintTransform,
    },
    Tile {
        scaling_factor: f32,
        rotation: DesignMediaQuarterTurn,
    },
}

impl DesignMediaPaintPlacement {
    pub const fn mode(self) -> DesignMediaPaintScaleMode {
        match self {
            Self::Fill { .. } => DesignMediaPaintScaleMode::Fill,
            Self::Fit { .. } => DesignMediaPaintScaleMode::Fit,
            Self::Crop { .. } => DesignMediaPaintScaleMode::Crop,
            Self::Tile { .. } => DesignMediaPaintScaleMode::Tile,
        }
    }

    pub const fn rotation(self) -> Option<DesignMediaQuarterTurn> {
        match self {
            Self::Fill { rotation } | Self::Fit { rotation } | Self::Tile { rotation, .. } => {
                Some(rotation)
            }
            Self::Crop { .. } => None,
        }
    }

    pub const fn crop_transform(self) -> Option<DesignPaintTransform> {
        match self {
            Self::Crop { transform } => Some(transform),
            _ => None,
        }
    }

    pub const fn tile_scaling_factor(self) -> Option<f32> {
        match self {
            Self::Tile { scaling_factor, .. } => Some(scaling_factor),
            _ => None,
        }
    }

    pub fn with_mode(self, mode: DesignMediaPaintScaleMode) -> Self {
        let rotation = self.rotation().unwrap_or_default();
        match mode {
            DesignMediaPaintScaleMode::Fill => Self::Fill { rotation },
            DesignMediaPaintScaleMode::Fit => Self::Fit { rotation },
            DesignMediaPaintScaleMode::Crop => Self::Crop {
                transform: self.crop_transform().unwrap_or_default(),
            },
            DesignMediaPaintScaleMode::Tile => Self::Tile {
                scaling_factor: self.tile_scaling_factor().unwrap_or(1.),
                rotation,
            },
        }
    }

    pub fn is_valid(self) -> bool {
        match self {
            Self::Fill { .. } | Self::Fit { .. } => true,
            Self::Crop { transform } => paint_transform_is_finite(transform),
            Self::Tile { scaling_factor, .. } => scaling_factor.is_finite() && scaling_factor > 0.,
        }
    }
}

impl Default for DesignMediaPaintPlacement {
    fn default() -> Self {
        Self::Fill {
            rotation: DesignMediaQuarterTurn::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignImageFilter {
    Exposure,
    Contrast,
    Saturation,
    Temperature,
    Tint,
    Highlights,
    Shadows,
}

impl DesignImageFilter {
    pub const ALL: [Self; 7] = [
        Self::Exposure,
        Self::Contrast,
        Self::Saturation,
        Self::Temperature,
        Self::Tint,
        Self::Highlights,
        Self::Shadows,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Exposure => "Exposure",
            Self::Contrast => "Contrast",
            Self::Saturation => "Saturation",
            Self::Temperature => "Temperature",
            Self::Tint => "Tint",
            Self::Highlights => "Highlights",
            Self::Shadows => "Shadows",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignImageFilters {
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
    pub tint: f32,
    pub highlights: f32,
    pub shadows: f32,
}

impl DesignImageFilters {
    pub const fn value(self, filter: DesignImageFilter) -> f32 {
        match filter {
            DesignImageFilter::Exposure => self.exposure,
            DesignImageFilter::Contrast => self.contrast,
            DesignImageFilter::Saturation => self.saturation,
            DesignImageFilter::Temperature => self.temperature,
            DesignImageFilter::Tint => self.tint,
            DesignImageFilter::Highlights => self.highlights,
            DesignImageFilter::Shadows => self.shadows,
        }
    }

    /// Applies one Figma image-filter value. Non-finite values are rejected;
    /// finite values are bounded to the API's inclusive `-1..=1` range.
    pub fn set_value(&mut self, filter: DesignImageFilter, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        let value = value.clamp(-1., 1.);
        match filter {
            DesignImageFilter::Exposure => self.exposure = value,
            DesignImageFilter::Contrast => self.contrast = value,
            DesignImageFilter::Saturation => self.saturation = value,
            DesignImageFilter::Temperature => self.temperature = value,
            DesignImageFilter::Tint => self.tint = value,
            DesignImageFilter::Highlights => self.highlights = value,
            DesignImageFilter::Shadows => self.shadows = value,
        }
        true
    }

    pub fn normalized(mut self) -> Option<Self> {
        for filter in DesignImageFilter::ALL {
            let value = self.value(filter);
            if !self.set_value(filter, value) {
                return None;
            }
        }
        Some(self)
    }
}

impl Default for DesignImageFilters {
    fn default() -> Self {
        Self {
            exposure: 0.,
            contrast: 0.,
            saturation: 0.,
            temperature: 0.,
            tint: 0.,
            highlights: 0.,
            shadows: 0.,
        }
    }
}

/// Host-owned source flow requested from an image or video paint.
///
/// Each variant opens a host surface or asynchronous workflow. The reusable
/// panel never chooses a file, runs generation/editing, or predicts the
/// resulting [`DesignPaintSource`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignMediaSourceAction {
    Upload,
    MakeImage,
    EditImage,
}

impl DesignMediaSourceAction {
    pub const ALL: [Self; 3] = [Self::Upload, Self::MakeImage, Self::EditImage];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Upload => "Upload from computer",
            Self::MakeImage => "Make an image",
            Self::EditImage => "Edit image",
        }
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::MakeImage => "make-image",
            Self::EditImage => "edit-image",
        }
    }

    pub const fn is_applicable_to(self, paint_type: DesignPaintType) -> bool {
        match self {
            Self::Upload => matches!(paint_type, DesignPaintType::Image | DesignPaintType::Video),
            Self::MakeImage | Self::EditImage => matches!(paint_type, DesignPaintType::Image),
        }
    }
}

/// Host permissions specific to one media paint.
///
/// Source workflows are intentionally independent from property editing and
/// from one another: for example, a host may allow filter edits and uploads
/// while withholding its AI image-generation and image-editing surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignMediaPaintCapabilities {
    pub can_edit_properties: bool,
    pub can_upload_source: bool,
    pub can_make_image: bool,
    pub can_edit_image: bool,
    /// Exact external file formats accepted for direct source replacement.
    ///
    /// This is independent from the file chooser itself but remains gated by
    /// `can_upload_source`. The standard preset intentionally excludes TIFF.
    pub accepted_drop_file_kinds: DesignMediaFileKinds,
}

impl DesignMediaPaintCapabilities {
    pub const fn editor() -> Self {
        Self {
            can_edit_properties: true,
            can_upload_source: true,
            can_make_image: true,
            can_edit_image: true,
            accepted_drop_file_kinds: DesignMediaFileKinds::STANDARD,
        }
    }

    pub const fn property_editor_only() -> Self {
        Self {
            can_edit_properties: true,
            can_upload_source: false,
            can_make_image: false,
            can_edit_image: false,
            accepted_drop_file_kinds: DesignMediaFileKinds::NONE,
        }
    }

    pub const fn viewer() -> Self {
        Self {
            can_edit_properties: false,
            can_upload_source: false,
            can_make_image: false,
            can_edit_image: false,
            accepted_drop_file_kinds: DesignMediaFileKinds::NONE,
        }
    }

    pub const fn with_source_actions(
        mut self,
        upload: bool,
        make_image: bool,
        edit_image: bool,
    ) -> Self {
        self.can_upload_source = upload;
        self.can_make_image = make_image;
        self.can_edit_image = edit_image;
        self
    }

    pub const fn with_accepted_drop_file_kinds(
        mut self,
        accepted_drop_file_kinds: DesignMediaFileKinds,
    ) -> Self {
        self.accepted_drop_file_kinds = accepted_drop_file_kinds;
        self
    }

    pub const fn allows_source_action(self, action: DesignMediaSourceAction) -> bool {
        match action {
            DesignMediaSourceAction::Upload => self.can_upload_source,
            DesignMediaSourceAction::MakeImage => self.can_make_image,
            DesignMediaSourceAction::EditImage => self.can_edit_image,
        }
    }

    pub const fn allows_file_drop(self, kind: DesignMediaFileKind) -> bool {
        self.can_upload_source && self.accepted_drop_file_kinds.contains(kind)
    }
}

impl Default for DesignMediaPaintCapabilities {
    fn default() -> Self {
        Self::editor()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignMediaCropAspectRatio {
    #[default]
    Free,
    Original,
    Square,
    FourByThree,
    SixteenByNine,
}

impl DesignMediaCropAspectRatio {
    pub const ALL: [Self; 5] = [
        Self::Free,
        Self::Original,
        Self::Square,
        Self::FourByThree,
        Self::SixteenByNine,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Original => "Original",
            Self::Square => "1:1",
            Self::FourByThree => "4:3",
            Self::SixteenByNine => "16:9",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Free => Self::Original,
            Self::Original => Self::Square,
            Self::Square => Self::FourByThree,
            Self::FourByThree => Self::SixteenByNine,
            Self::SixteenByNine => Self::Free,
        }
    }
}

/// Host-controlled state for the crop tool. The paint's committed affine
/// transform remains in [`DesignMediaPaintPlacement::Crop`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignMediaCropToolState {
    pub active: bool,
    pub transform: DesignPaintTransform,
    pub zoom: f32,
    pub aspect_ratio: DesignMediaCropAspectRatio,
}

impl Default for DesignMediaCropToolState {
    fn default() -> Self {
        Self {
            active: false,
            transform: DesignPaintTransform::IDENTITY,
            zoom: 1.,
            aspect_ratio: DesignMediaCropAspectRatio::Free,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignVideoPreviewStatus {
    Idle,
    Loading,
    Ready,
    Error { message: SharedString },
}

/// Host-controlled, preview-only video state. None of these values are paint
/// document properties.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignVideoPreviewState {
    pub duration_seconds: f32,
    pub current_seconds: f32,
    pub playing: bool,
    pub status: DesignVideoPreviewStatus,
}

impl Default for DesignVideoPreviewState {
    fn default() -> Self {
        Self {
            duration_seconds: 0.,
            current_seconds: 0.,
            playing: false,
            status: DesignVideoPreviewStatus::Idle,
        }
    }
}

impl DesignVideoPreviewState {
    pub fn loading() -> Self {
        Self {
            status: DesignVideoPreviewStatus::Loading,
            ..Self::default()
        }
    }

    pub fn ready(duration_seconds: f32, current_seconds: f32, playing: bool) -> Self {
        let duration_seconds = if duration_seconds.is_finite() {
            duration_seconds.max(0.)
        } else {
            0.
        };
        let current_seconds = if current_seconds.is_finite() {
            current_seconds.clamp(0., duration_seconds)
        } else {
            0.
        };
        Self {
            duration_seconds,
            current_seconds,
            playing,
            status: DesignVideoPreviewStatus::Ready,
        }
    }

    pub fn error(message: impl Into<SharedString>) -> Self {
        Self {
            status: DesignVideoPreviewStatus::Error {
                message: message.into(),
            },
            ..Self::default()
        }
    }
}

/// View-only host state for one stable media paint occurrence.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignMediaPaintView {
    pub collection: DesignPanelCollection,
    pub paint_id: SharedString,
    pub index: usize,
    pub capabilities: DesignMediaPaintCapabilities,
    pub crop_tool: DesignMediaCropToolState,
    pub video_preview: Option<DesignVideoPreviewState>,
}

impl DesignMediaPaintView {
    pub fn new(
        collection: DesignPanelCollection,
        paint_id: impl Into<SharedString>,
        index: usize,
    ) -> Self {
        Self {
            collection,
            paint_id: paint_id.into(),
            index,
            capabilities: DesignMediaPaintCapabilities::default(),
            crop_tool: DesignMediaCropToolState::default(),
            video_preview: None,
        }
    }

    pub const fn with_capabilities(mut self, capabilities: DesignMediaPaintCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub const fn with_crop_tool(mut self, crop_tool: DesignMediaCropToolState) -> Self {
        self.crop_tool = crop_tool;
        self
    }

    pub fn with_video_preview(mut self, preview: DesignVideoPreviewState) -> Self {
        self.video_preview = Some(preview);
        self
    }

    pub fn matches(
        &self,
        collection: DesignPanelCollection,
        paint_id: &SharedString,
        index: usize,
    ) -> bool {
        self.collection == collection
            && if self.paint_id.is_empty() || paint_id.is_empty() {
                self.index == index
            } else {
                self.paint_id == *paint_id
            }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignMediaPaintViewData {
    pub paints: Vec<DesignMediaPaintView>,
}

impl DesignMediaPaintViewData {
    pub fn new(paints: impl IntoIterator<Item = DesignMediaPaintView>) -> Self {
        Self {
            paints: paints.into_iter().collect(),
        }
    }

    pub fn paint(
        &self,
        collection: DesignPanelCollection,
        paint_id: &SharedString,
        index: usize,
    ) -> Option<&DesignMediaPaintView> {
        self.paints
            .iter()
            .find(|paint| paint.matches(collection, paint_id, index))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignMediaCropAction {
    Begin,
    Preview {
        transform: DesignPaintTransform,
        zoom: f32,
        aspect_ratio: DesignMediaCropAspectRatio,
    },
    Commit {
        transform: DesignPaintTransform,
        zoom: f32,
        aspect_ratio: DesignMediaCropAspectRatio,
    },
    Cancel,
    ResizeToFit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignVideoPreviewAction {
    Play,
    Pause,
    Seek {
        seconds: f32,
    },
    Scrub {
        seconds: f32,
        phase: DesignPanelEditPhase,
    },
}

/// Legacy node-level media projection retained for source compatibility.
///
/// New hosts should model image/video through
/// [`DesignPaintPayload::Image`] and [`DesignPaintPayload::Video`]. The Design
/// panel no longer renders this duplicate snapshot or emits
/// `ReplaceMediaRequested`; media source and filter edits are paint intents.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignMedia {
    pub crop_mode: SharedString,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
    pub tint: f32,
    pub highlights: f32,
    pub shadows: f32,
}

impl Default for DesignMedia {
    fn default() -> Self {
        Self {
            crop_mode: "Fill".into(),
            exposure: 0.,
            contrast: 0.,
            saturation: 0.,
            temperature: 0.,
            tint: 0.,
            highlights: 0.,
            shadows: 0.,
        }
    }
}

impl DesignMedia {
    /// Adapts the legacy crop label to the canonical media-paint value.
    pub fn paint_scale_mode(&self) -> DesignMediaPaintScaleMode {
        match self.crop_mode.as_ref().to_ascii_lowercase().as_str() {
            "fit" => DesignMediaPaintScaleMode::Fit,
            "crop" => DesignMediaPaintScaleMode::Crop,
            "tile" => DesignMediaPaintScaleMode::Tile,
            _ => DesignMediaPaintScaleMode::Fill,
        }
    }

    /// Adapts legacy adjustment fields without inventing source/crop data.
    pub const fn image_filters(&self) -> DesignImageFilters {
        DesignImageFilters {
            exposure: self.exposure,
            contrast: self.contrast,
            saturation: self.saturation,
            temperature: self.temperature,
            tint: self.tint,
            highlights: self.highlights,
            shadows: self.shadows,
        }
    }
}
