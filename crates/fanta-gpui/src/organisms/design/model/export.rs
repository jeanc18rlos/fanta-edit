use gpui::SharedString;

use super::DesignPanelTarget;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignExportFormat {
    Png,
    Jpg,
    Svg,
    Pdf,
}

impl DesignExportFormat {
    pub const ALL: [Self; 4] = [Self::Png, Self::Jpg, Self::Svg, Self::Pdf];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpg => "JPG",
            Self::Svg => "SVG",
            Self::Pdf => "PDF",
        }
    }

    /// SVG and PDF export at their intrinsic size in Figma's static-export
    /// panel. Raster formats additionally accept width- and height-based
    /// sizing.
    pub const fn supports_custom_sizing(self) -> bool {
        matches!(self, Self::Png | Self::Jpg)
    }
}

/// Compatibility record used by early hosts of the compact export row.
///
/// New integrations should supply [`DesignExportViewData`]. The panel adapts
/// this legacy scale-only record into a canonical configuration when no
/// explicit export view data is present.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignExportSetting {
    pub scale: f32,
    pub suffix: SharedString,
    pub format: DesignExportFormat,
}

/// Canonical static-export sizing entered as `2x`, `320w`, or `240h`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignExportSizing {
    Scale(f32),
    Width(f32),
    Height(f32),
}

impl DesignExportSizing {
    pub const fn value(self) -> f32 {
        match self {
            Self::Scale(value) | Self::Width(value) | Self::Height(value) => value,
        }
    }

    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Scale(_) => "x",
            Self::Width(_) => "w",
            Self::Height(_) => "h",
        }
    }

    pub fn normalized(self) -> Self {
        let value = self.value();
        if value.is_finite() && value > 0. {
            self
        } else {
            Self::Scale(1.)
        }
    }

    pub fn parse(value: &str) -> Result<Self, DesignExportSizingParseError> {
        let value = value.trim();
        let suffix = value.chars().last().ok_or(DesignExportSizingParseError)?;
        let number = &value[..value.len() - suffix.len_utf8()];
        let number = number
            .trim()
            .parse::<f32>()
            .map_err(|_| DesignExportSizingParseError)?;
        if !number.is_finite() || number <= 0. {
            return Err(DesignExportSizingParseError);
        }
        match suffix.to_ascii_lowercase() {
            'x' => Ok(Self::Scale(number)),
            'w' => Ok(Self::Width(number)),
            'h' => Ok(Self::Height(number)),
            _ => Err(DesignExportSizingParseError),
        }
    }
}

impl std::fmt::Display for DesignExportSizing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = self.value();
        if value.fract() == 0. {
            write!(formatter, "{value:.0}{}", self.suffix())
        } else {
            let mut number = format!("{value:.3}");
            while number.ends_with('0') {
                number.pop();
            }
            if number.ends_with('.') {
                number.pop();
            }
            write!(formatter, "{number}{}", self.suffix())
        }
    }
}

impl std::str::FromStr for DesignExportSizing {
    type Err = DesignExportSizingParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignExportSizingParseError;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignExportColorProfile {
    SameAsFile,
    Srgb,
    DisplayP3,
}

impl DesignExportColorProfile {
    pub const ALL: [Self; 3] = [Self::SameAsFile, Self::Srgb, Self::DisplayP3];

    pub const fn label(self) -> &'static str {
        match self {
            Self::SameAsFile => "Same as file",
            Self::Srgb => "sRGB",
            Self::DisplayP3 => "Display P3",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignExportImageResampling {
    Detailed,
    Basic,
}

impl DesignExportImageResampling {
    pub const ALL: [Self; 2] = [Self::Detailed, Self::Basic];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Detailed => "Detailed",
            Self::Basic => "Basic",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignExportImageQuality {
    Low,
    Medium,
    High,
}

impl DesignExportImageQuality {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignExportPngSettings {
    pub ignore_overlapping_layers: bool,
    pub include_text_bounding_box: bool,
    pub resampling: DesignExportImageResampling,
}

impl Default for DesignExportPngSettings {
    fn default() -> Self {
        Self {
            ignore_overlapping_layers: true,
            include_text_bounding_box: false,
            resampling: DesignExportImageResampling::Detailed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignExportJpgSettings {
    pub ignore_overlapping_layers: bool,
    pub include_text_bounding_box: bool,
    pub resampling: DesignExportImageResampling,
    pub quality: DesignExportImageQuality,
}

impl Default for DesignExportJpgSettings {
    fn default() -> Self {
        Self {
            ignore_overlapping_layers: true,
            include_text_bounding_box: false,
            resampling: DesignExportImageResampling::Detailed,
            quality: DesignExportImageQuality::High,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignExportSvgSettings {
    pub ignore_overlapping_layers: bool,
    pub include_bounds: bool,
    pub include_id_attribute: bool,
    pub outline_text: bool,
    pub simplify_stroke: bool,
}

impl Default for DesignExportSvgSettings {
    fn default() -> Self {
        Self {
            ignore_overlapping_layers: true,
            include_bounds: false,
            include_id_attribute: false,
            outline_text: true,
            simplify_stroke: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignExportPdfSettings {
    pub resampling: DesignExportImageResampling,
    pub quality: DesignExportImageQuality,
}

impl Default for DesignExportPdfSettings {
    fn default() -> Self {
        Self {
            resampling: DesignExportImageResampling::Detailed,
            quality: DesignExportImageQuality::Medium,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesignExportFormatSettings {
    Png(DesignExportPngSettings),
    Jpg(DesignExportJpgSettings),
    Svg(DesignExportSvgSettings),
    Pdf(DesignExportPdfSettings),
}

impl DesignExportFormatSettings {
    pub fn for_format(format: DesignExportFormat) -> Self {
        match format {
            DesignExportFormat::Png => Self::Png(DesignExportPngSettings::default()),
            DesignExportFormat::Jpg => Self::Jpg(DesignExportJpgSettings::default()),
            DesignExportFormat::Svg => Self::Svg(DesignExportSvgSettings::default()),
            DesignExportFormat::Pdf => Self::Pdf(DesignExportPdfSettings::default()),
        }
    }

    pub const fn format(self) -> DesignExportFormat {
        match self {
            Self::Png(_) => DesignExportFormat::Png,
            Self::Jpg(_) => DesignExportFormat::Jpg,
            Self::Svg(_) => DesignExportFormat::Svg,
            Self::Pdf(_) => DesignExportFormat::Pdf,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignExportCommonSettings {
    pub suffix: SharedString,
    pub color_profile: DesignExportColorProfile,
}

impl Default for DesignExportCommonSettings {
    fn default() -> Self {
        Self {
            suffix: SharedString::default(),
            color_profile: DesignExportColorProfile::SameAsFile,
        }
    }
}

/// A host-owned export row with an ID that remains stable across reordering.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignExportConfiguration {
    pub id: SharedString,
    pub sizing: DesignExportSizing,
    pub common: DesignExportCommonSettings,
    pub format_settings: DesignExportFormatSettings,
}

impl DesignExportConfiguration {
    pub fn new(id: impl Into<SharedString>, format: DesignExportFormat) -> Self {
        Self {
            id: id.into(),
            sizing: DesignExportSizing::Scale(1.),
            common: DesignExportCommonSettings::default(),
            format_settings: DesignExportFormatSettings::for_format(format),
        }
    }

    pub fn new_for_capabilities(
        id: impl Into<SharedString>,
        format: DesignExportFormat,
        capabilities: DesignStaticExportCapabilities,
    ) -> Self {
        let mut configuration = Self::new(id, format);
        configuration.apply_target_defaults(capabilities);
        configuration
    }

    pub const fn format(&self) -> DesignExportFormat {
        self.format_settings.format()
    }

    /// Applies defaults that Figma derives from the current export target.
    /// Hosts should call this when creating a row or changing its format, then
    /// echo the resulting controlled configuration back to the panel.
    pub fn apply_target_defaults(&mut self, capabilities: DesignStaticExportCapabilities) {
        match &mut self.format_settings {
            DesignExportFormatSettings::Png(settings) => {
                if !capabilities.can_include_text_bounding_box {
                    settings.include_text_bounding_box = false;
                }
            }
            DesignExportFormatSettings::Jpg(settings) => {
                if !capabilities.can_include_text_bounding_box {
                    settings.include_text_bounding_box = false;
                }
            }
            DesignExportFormatSettings::Svg(settings) => {
                if !capabilities.can_include_svg_bounds {
                    settings.include_bounds = false;
                }
                settings.outline_text = capabilities.svg_outline_text_default;
                settings.simplify_stroke = capabilities.svg_simplify_stroke_default;
            }
            DesignExportFormatSettings::Pdf(_) => {}
        }
    }

    pub fn normalized(mut self) -> Self {
        self.sizing = if self.format().supports_custom_sizing() {
            self.sizing.normalized()
        } else {
            DesignExportSizing::Scale(1.)
        };
        self
    }

    pub fn apply_change(&mut self, change: DesignExportConfigurationChange) {
        match change {
            DesignExportConfigurationChange::Sizing(sizing) => self.sizing = sizing,
            DesignExportConfigurationChange::Suffix(suffix) => self.common.suffix = suffix,
            DesignExportConfigurationChange::Format(format) => {
                self.format_settings = DesignExportFormatSettings::for_format(format);
            }
            DesignExportConfigurationChange::ColorProfile(profile) => {
                self.common.color_profile = profile;
            }
            DesignExportConfigurationChange::IgnoreOverlappingLayers(value) => {
                match &mut self.format_settings {
                    DesignExportFormatSettings::Png(settings) => {
                        settings.ignore_overlapping_layers = value;
                    }
                    DesignExportFormatSettings::Jpg(settings) => {
                        settings.ignore_overlapping_layers = value;
                    }
                    DesignExportFormatSettings::Svg(settings) => {
                        settings.ignore_overlapping_layers = value;
                    }
                    DesignExportFormatSettings::Pdf(_) => {}
                }
            }
            DesignExportConfigurationChange::IncludeTextBoundingBox(value) => {
                match &mut self.format_settings {
                    DesignExportFormatSettings::Png(settings) => {
                        settings.include_text_bounding_box = value;
                    }
                    DesignExportFormatSettings::Jpg(settings) => {
                        settings.include_text_bounding_box = value;
                    }
                    DesignExportFormatSettings::Svg(_) | DesignExportFormatSettings::Pdf(_) => {}
                }
            }
            DesignExportConfigurationChange::Resampling(value) => match &mut self.format_settings {
                DesignExportFormatSettings::Png(settings) => settings.resampling = value,
                DesignExportFormatSettings::Jpg(settings) => settings.resampling = value,
                DesignExportFormatSettings::Pdf(settings) => settings.resampling = value,
                DesignExportFormatSettings::Svg(_) => {}
            },
            DesignExportConfigurationChange::Quality(value) => match &mut self.format_settings {
                DesignExportFormatSettings::Jpg(settings) => settings.quality = value,
                DesignExportFormatSettings::Pdf(settings) => settings.quality = value,
                DesignExportFormatSettings::Png(_) | DesignExportFormatSettings::Svg(_) => {}
            },
            DesignExportConfigurationChange::IncludeBounds(value) => {
                if let DesignExportFormatSettings::Svg(settings) = &mut self.format_settings {
                    settings.include_bounds = value;
                }
            }
            DesignExportConfigurationChange::IncludeIdAttribute(value) => {
                if let DesignExportFormatSettings::Svg(settings) = &mut self.format_settings {
                    settings.include_id_attribute = value;
                }
            }
            DesignExportConfigurationChange::OutlineText(value) => {
                if let DesignExportFormatSettings::Svg(settings) = &mut self.format_settings {
                    settings.outline_text = value;
                }
            }
            DesignExportConfigurationChange::SimplifyStroke(value) => {
                if let DesignExportFormatSettings::Svg(settings) = &mut self.format_settings {
                    settings.simplify_stroke = value;
                }
            }
        }
        self.sizing = if self.format().supports_custom_sizing() {
            self.sizing.normalized()
        } else {
            DesignExportSizing::Scale(1.)
        };
    }

    pub fn from_legacy(id: impl Into<SharedString>, setting: &DesignExportSetting) -> Self {
        Self {
            id: id.into(),
            sizing: DesignExportSizing::Scale(setting.scale),
            common: DesignExportCommonSettings {
                suffix: setting.suffix.clone(),
                ..Default::default()
            },
            format_settings: DesignExportFormatSettings::for_format(setting.format),
        }
        .normalized()
    }

    pub fn to_legacy(&self) -> Option<DesignExportSetting> {
        let DesignExportSizing::Scale(scale) = self.sizing else {
            return None;
        };
        Some(DesignExportSetting {
            scale,
            suffix: self.common.suffix.clone(),
            format: self.format(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignExportConfigurationChange {
    Sizing(DesignExportSizing),
    Suffix(SharedString),
    Format(DesignExportFormat),
    ColorProfile(DesignExportColorProfile),
    IgnoreOverlappingLayers(bool),
    IncludeTextBoundingBox(bool),
    Resampling(DesignExportImageResampling),
    Quality(DesignExportImageQuality),
    IncludeBounds(bool),
    IncludeIdAttribute(bool),
    OutlineText(bool),
    SimplifyStroke(bool),
}

/// The active half of Figma's Static/Animated export switch.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignExportMode {
    #[default]
    Static,
    Animated,
}

impl DesignExportMode {
    pub const ALL: [Self; 2] = [Self::Static, Self::Animated];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Static => "Static",
            Self::Animated => "Animated",
        }
    }
}

/// Host-resolved applicability and defaults for static export controls.
///
/// These values deliberately live outside the reusable panel: SVG defaults
/// and bounding-box applicability depend on the selected target and its
/// contents, not only on the selected export format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignStaticExportCapabilities {
    pub can_include_text_bounding_box: bool,
    pub can_include_svg_bounds: bool,
    pub svg_outline_text_default: bool,
    pub svg_simplify_stroke_default: bool,
}

impl Default for DesignStaticExportCapabilities {
    fn default() -> Self {
        Self {
            can_include_text_bounding_box: false,
            can_include_svg_bounds: false,
            svg_outline_text_default: true,
            svg_simplify_stroke_default: true,
        }
    }
}

/// Host-owned preview payload rendered by the export disclosure.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignExportPreview {
    /// Host-defined thumbnail/asset identifier. Rendering the actual bitmap is
    /// owned by the embedding application.
    pub thumbnail_id: Option<SharedString>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub estimated_output: Option<SharedString>,
}

impl DesignExportPreview {
    pub fn new(pixel_width: u32, pixel_height: u32) -> Self {
        Self {
            thumbnail_id: None,
            pixel_width,
            pixel_height,
            estimated_output: None,
        }
    }

    pub fn with_thumbnail(mut self, thumbnail_id: impl Into<SharedString>) -> Self {
        self.thumbnail_id = Some(thumbnail_id.into());
        self
    }

    pub fn with_estimated_output(mut self, output: impl Into<SharedString>) -> Self {
        self.estimated_output = Some(output.into());
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum DesignExportPreviewState {
    #[default]
    Idle,
    Loading,
    Ready(DesignExportPreview),
    Error {
        message: SharedString,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignAnimatedExportFormat {
    Mp4,
    WebM,
    Gif,
    Svg,
}

impl DesignAnimatedExportFormat {
    pub const ALL: [Self; 4] = [Self::Mp4, Self::WebM, Self::Gif, Self::Svg];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::WebM => "WebM",
            Self::Gif => "GIF",
            Self::Svg => "SVG",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignVideoExportFps {
    Fps12,
    Fps24,
    #[default]
    Fps30,
    Fps60,
}

impl DesignVideoExportFps {
    pub const ALL: [Self; 4] = [Self::Fps12, Self::Fps24, Self::Fps30, Self::Fps60];

    pub const fn value(self) -> u8 {
        match self {
            Self::Fps12 => 12,
            Self::Fps24 => 24,
            Self::Fps30 => 30,
            Self::Fps60 => 60,
        }
    }

    pub fn label(self) -> SharedString {
        format!("{} FPS", self.value()).into()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignGifExportFps {
    Fps8,
    Fps12,
    #[default]
    Fps15,
    Fps24,
    Fps30,
}

impl DesignGifExportFps {
    pub const ALL: [Self; 5] = [
        Self::Fps8,
        Self::Fps12,
        Self::Fps15,
        Self::Fps24,
        Self::Fps30,
    ];

    pub const fn value(self) -> u8 {
        match self {
            Self::Fps8 => 8,
            Self::Fps12 => 12,
            Self::Fps15 => 15,
            Self::Fps24 => 24,
            Self::Fps30 => 30,
        }
    }

    pub fn label(self) -> SharedString {
        format!("{} FPS", self.value()).into()
    }
}

/// One host-described option in animated SVG export. Figma exposes this format
/// in the UI even though the Plugin API does not currently define a stored
/// animated-SVG settings contract.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignAnimatedSvgOption {
    pub id: SharedString,
    pub label: SharedString,
    pub selected: SharedString,
    pub choices: Vec<SharedString>,
}

impl DesignAnimatedSvgOption {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        selected: impl Into<SharedString>,
        choices: impl IntoIterator<Item = impl Into<SharedString>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            selected: selected.into(),
            choices: choices.into_iter().map(Into::into).collect(),
        }
    }
}

/// Exact animated-export settings supported by Figma's current export APIs.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignAnimatedExportSettings {
    Mp4 {
        sizing: DesignExportSizing,
        fps: DesignVideoExportFps,
        quality: DesignExportImageQuality,
    },
    WebM {
        sizing: DesignExportSizing,
        fps: DesignVideoExportFps,
        quality: DesignExportImageQuality,
    },
    Gif {
        sizing: DesignExportSizing,
        fps: DesignGifExportFps,
        loop_count: u16,
    },
    Svg {
        options: Vec<DesignAnimatedSvgOption>,
    },
}

impl Default for DesignAnimatedExportSettings {
    fn default() -> Self {
        Self::Mp4 {
            sizing: DesignExportSizing::Scale(1.),
            fps: DesignVideoExportFps::Fps30,
            quality: DesignExportImageQuality::High,
        }
    }
}

impl DesignAnimatedExportSettings {
    pub const fn format(&self) -> DesignAnimatedExportFormat {
        match self {
            Self::Mp4 { .. } => DesignAnimatedExportFormat::Mp4,
            Self::WebM { .. } => DesignAnimatedExportFormat::WebM,
            Self::Gif { .. } => DesignAnimatedExportFormat::Gif,
            Self::Svg { .. } => DesignAnimatedExportFormat::Svg,
        }
    }

    pub fn for_format(format: DesignAnimatedExportFormat) -> Self {
        match format {
            DesignAnimatedExportFormat::Mp4 => Self::default(),
            DesignAnimatedExportFormat::WebM => Self::WebM {
                sizing: DesignExportSizing::Scale(1.),
                fps: DesignVideoExportFps::Fps30,
                quality: DesignExportImageQuality::High,
            },
            DesignAnimatedExportFormat::Gif => Self::Gif {
                sizing: DesignExportSizing::Scale(1.),
                fps: DesignGifExportFps::Fps15,
                loop_count: 0,
            },
            DesignAnimatedExportFormat::Svg => Self::Svg {
                options: Vec::new(),
            },
        }
    }

    pub fn normalized(mut self) -> Self {
        match &mut self {
            Self::Mp4 { sizing, .. } | Self::WebM { sizing, .. } | Self::Gif { sizing, .. } => {
                *sizing = normalize_video_export_sizing(*sizing);
            }
            Self::Svg { options } => {
                for option in options {
                    if !option.choices.contains(&option.selected)
                        && let Some(first) = option.choices.first()
                    {
                        option.selected = first.clone();
                    }
                }
            }
        }
        if let Self::Gif { loop_count, .. } = &mut self {
            *loop_count = (*loop_count).min(1000);
        }
        self
    }

    pub fn apply_change(&mut self, change: DesignAnimatedExportChange) -> bool {
        match change {
            DesignAnimatedExportChange::Format(format) => {
                *self = Self::for_format(format);
            }
            DesignAnimatedExportChange::Sizing(next) => match self {
                Self::Mp4 { sizing, .. } | Self::WebM { sizing, .. } | Self::Gif { sizing, .. } => {
                    *sizing = normalize_video_export_sizing(next)
                }
                Self::Svg { .. } => return false,
            },
            DesignAnimatedExportChange::VideoFps(next) => match self {
                Self::Mp4 { fps, .. } | Self::WebM { fps, .. } => *fps = next,
                Self::Gif { .. } | Self::Svg { .. } => return false,
            },
            DesignAnimatedExportChange::GifFps(next) => match self {
                Self::Gif { fps, .. } => *fps = next,
                Self::Mp4 { .. } | Self::WebM { .. } | Self::Svg { .. } => return false,
            },
            DesignAnimatedExportChange::Quality(next) => match self {
                Self::Mp4 { quality, .. } | Self::WebM { quality, .. } => *quality = next,
                Self::Gif { .. } | Self::Svg { .. } => return false,
            },
            DesignAnimatedExportChange::GifLoopCount(next) => match self {
                Self::Gif { loop_count, .. } => *loop_count = next.min(1000),
                Self::Mp4 { .. } | Self::WebM { .. } | Self::Svg { .. } => return false,
            },
            DesignAnimatedExportChange::SvgOption { option_id, value } => match self {
                Self::Svg { options } => {
                    let Some(option) = options.iter_mut().find(|option| option.id == option_id)
                    else {
                        return false;
                    };
                    if !option.choices.contains(&value) {
                        return false;
                    }
                    option.selected = value;
                }
                Self::Mp4 { .. } | Self::WebM { .. } | Self::Gif { .. } => return false,
            },
        }
        true
    }

    pub fn exceeds_starter_limits(&self, source_width: u32, source_height: u32) -> bool {
        let (sizing, high_fps) = match self {
            Self::Mp4 { sizing, fps, .. } | Self::WebM { sizing, fps, .. } => {
                (*sizing, fps.value() > 30)
            }
            Self::Gif { sizing, .. } => (*sizing, false),
            Self::Svg { .. } => return false,
        };
        let (width, height) = resolved_export_dimensions(sizing, source_width, source_height);
        high_fps || width > 1920 || height > 1080
    }
}

fn normalize_video_export_sizing(sizing: DesignExportSizing) -> DesignExportSizing {
    match sizing {
        DesignExportSizing::Scale(scale) if [0.5, 0.75, 1., 1.5, 2., 3., 4.].contains(&scale) => {
            sizing
        }
        DesignExportSizing::Width(width) if width.is_finite() && width > 0. => sizing,
        DesignExportSizing::Height(height) if height.is_finite() && height > 0. => sizing,
        _ => DesignExportSizing::Scale(1.),
    }
}

fn resolved_export_dimensions(
    sizing: DesignExportSizing,
    source_width: u32,
    source_height: u32,
) -> (u32, u32) {
    let source_width = source_width.max(1) as f64;
    let source_height = source_height.max(1) as f64;
    let scale = match sizing {
        DesignExportSizing::Scale(scale) => f64::from(scale),
        DesignExportSizing::Width(width) => f64::from(width) / source_width,
        DesignExportSizing::Height(height) => f64::from(height) / source_height,
    };
    (
        (source_width * scale).round().max(1.) as u32,
        (source_height * scale).round().max(1.) as u32,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignAnimatedExportChange {
    Format(DesignAnimatedExportFormat),
    Sizing(DesignExportSizing),
    VideoFps(DesignVideoExportFps),
    GifFps(DesignGifExportFps),
    Quality(DesignExportImageQuality),
    GifLoopCount(u16),
    SvgOption {
        option_id: SharedString,
        value: SharedString,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignAnimatedExportCapability {
    pub eligible: bool,
    pub disabled_reason: Option<SharedString>,
    pub available_formats: Vec<DesignAnimatedExportFormat>,
    pub high_resolution_allowed: bool,
    pub high_resolution_reason: Option<SharedString>,
    pub source_width: u32,
    pub source_height: u32,
}

impl DesignAnimatedExportCapability {
    pub fn eligible(source_width: u32, source_height: u32) -> Self {
        Self {
            eligible: true,
            disabled_reason: None,
            available_formats: DesignAnimatedExportFormat::ALL.to_vec(),
            high_resolution_allowed: true,
            high_resolution_reason: None,
            source_width,
            source_height,
        }
    }

    pub fn disabled(reason: impl Into<SharedString>) -> Self {
        Self {
            eligible: false,
            disabled_reason: Some(reason.into()),
            available_formats: DesignAnimatedExportFormat::ALL.to_vec(),
            high_resolution_allowed: false,
            high_resolution_reason: None,
            source_width: 0,
            source_height: 0,
        }
    }

    pub fn allows(&self, settings: &DesignAnimatedExportSettings) -> bool {
        self.eligible
            && self.available_formats.contains(&settings.format())
            && (self.high_resolution_allowed
                || !settings.exceeds_starter_limits(self.source_width, self.source_height))
    }

    pub fn reason_for(&self, settings: &DesignAnimatedExportSettings) -> Option<&str> {
        if !self.eligible {
            return self.disabled_reason.as_ref().map(|reason| reason.as_ref());
        }
        if !self.available_formats.contains(&settings.format()) {
            return Some("This animated format is unavailable");
        }
        if !self.high_resolution_allowed
            && settings.exceeds_starter_limits(self.source_width, self.source_height)
        {
            return self
                .high_resolution_reason
                .as_ref()
                .map(|reason| reason.as_ref())
                .or(Some(
                    "Exports above 1080p or 30 FPS require an upgraded plan",
                ));
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignAnimatedExportViewData {
    pub capability: DesignAnimatedExportCapability,
    pub settings: DesignAnimatedExportSettings,
}

impl DesignAnimatedExportViewData {
    pub fn new(
        capability: DesignAnimatedExportCapability,
        settings: DesignAnimatedExportSettings,
    ) -> Self {
        Self {
            capability,
            settings: settings.normalized(),
        }
    }
}

/// Complete controlled input for the static and animated export section.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignExportViewData {
    pub target: DesignPanelTarget,
    pub configurations: Vec<DesignExportConfiguration>,
    pub mode: DesignExportMode,
    pub static_capabilities: DesignStaticExportCapabilities,
    /// `None` omits Preview entirely. A present state is always host-owned.
    pub preview: Option<DesignExportPreviewState>,
    /// Present for nodes for which the host can explain Motion eligibility.
    pub animated: Option<DesignAnimatedExportViewData>,
}
