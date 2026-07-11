#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Fanta UI visual tests require macOS");
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    fanta_ui_visual_tests::run()
}

#[cfg(target_os = "macos")]
mod fanta_ui_visual_tests {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use anyhow::{Context as _, Result, bail};
    use assets::Assets;
    use fanta_ui::{
        animation_panel::{AnimationDetailHeader, AnimationProperty, AnimationPropertyRow},
        font_family_picker::FontFamilyPicker,
        inspector::{InspectorEmptyState, InspectorFieldRow, InspectorSection},
        timeline::{
            TimelineGridLine, TimelinePlayhead, TimelineRulerHeader, TimelineRulerTick,
            TimelineScale, TimelineTimecode, TimelineToolbarButton, format_ruler_time,
        },
    };
    use gpui::{
        AppContext as _, Bounds, IntoElement as _, Pixels, VisualTestAppContext, WindowBounds,
        WindowOptions, point, px, relative, size,
    };
    use image::{Rgba, RgbaImage};
    use ui::{Tooltip, prelude::*};

    const CHANNEL_THRESHOLD: i16 = 2;
    const MATCH_THRESHOLD: f64 = 0.9995;

    pub fn run() -> Result<()> {
        env_logger::builder().try_init().ok();
        let mut cx = VisualTestAppContext::with_asset_source(
            gpui_platform::current_platform(false),
            Arc::new(Assets),
        );
        cx.update(|cx| -> Result<()> {
            Assets.load_fonts(cx).context("loading UI fonts")?;
            settings::init(cx);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            Ok(())
        })?;

        let cases = [
            VisualCase {
                name: "animation_panel",
                size: size(px(760.), px(560.)),
                kind: GalleryKind::AnimationPanel,
            },
            VisualCase {
                name: "timeline",
                size: size(px(1240.), px(330.)),
                kind: GalleryKind::Timeline,
            },
            VisualCase {
                name: "inspector",
                size: size(px(420.), px(500.)),
                kind: GalleryKind::Inspector,
            },
        ];
        let update_baseline = std::env::var_os("UPDATE_BASELINE").is_some()
            || std::env::var_os("UPDATE_BASELINES").is_some();

        for case in cases {
            run_case(&mut cx, case, update_baseline)?;
        }
        Ok(())
    }

    #[derive(Clone, Copy)]
    enum GalleryKind {
        AnimationPanel,
        Timeline,
        Inspector,
    }

    struct VisualCase {
        name: &'static str,
        size: gpui::Size<Pixels>,
        kind: GalleryKind,
    }

    fn run_case(
        cx: &mut VisualTestAppContext,
        case: VisualCase,
        update_baseline: bool,
    ) -> Result<()> {
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: case.size,
                    })),
                    focus: false,
                    show: false,
                    ..WindowOptions::default()
                },
                |_, cx| cx.new(|_| Gallery { kind: case.kind }),
            )
        })?;
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();

        let screenshot = cx.capture_screenshot(window.into())?;
        let output_path = output_directory().join(format!("{}.png", case.name));
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        screenshot
            .save(&output_path)
            .with_context(|| format!("saving {}", output_path.display()))?;

        let baseline_path = baseline_path(case.name);
        if update_baseline {
            if let Some(parent) = baseline_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            screenshot
                .save(&baseline_path)
                .with_context(|| format!("saving {}", baseline_path.display()))?;
            println!("updated {}", baseline_path.display());
            return Ok(());
        }

        if !baseline_path.exists() {
            bail!(
                "baseline missing at {}; run with UPDATE_BASELINE=1",
                baseline_path.display()
            );
        }
        let baseline = image::open(&baseline_path)
            .with_context(|| format!("opening {}", baseline_path.display()))?
            .to_rgba8();
        let comparison = compare_images(&screenshot, &baseline);
        println!(
            "{}: {:.3}% match ({} different pixels)",
            case.name,
            comparison.match_ratio * 100.0,
            comparison.different_pixels
        );
        if comparison.match_ratio < MATCH_THRESHOLD {
            let diff_path = output_directory().join(format!("{}_diff.png", case.name));
            comparison
                .diff_image
                .save(&diff_path)
                .with_context(|| format!("saving {}", diff_path.display()))?;
            bail!(
                "{} differed from its baseline: {:.3}% match, expected at least {:.3}%; diff at {}",
                case.name,
                comparison.match_ratio * 100.0,
                MATCH_THRESHOLD * 100.0,
                diff_path.display()
            );
        }
        Ok(())
    }

    fn output_directory() -> PathBuf {
        std::env::var_os("FANTA_UI_VISUAL_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/visual_tests/fanta_ui"))
    }

    fn baseline_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_fixtures/visual/macos")
            .join(format!("{name}.png"))
    }

    struct ImageComparison {
        match_ratio: f64,
        different_pixels: u64,
        diff_image: RgbaImage,
    }

    fn compare_images(actual: &RgbaImage, expected: &RgbaImage) -> ImageComparison {
        let width = actual.width().max(expected.width());
        let height = actual.height().max(expected.height());
        let total_pixels = u64::from(width) * u64::from(height);
        let mut different_pixels = 0;
        let mut diff_image = RgbaImage::new(width, height);

        for y in 0..height {
            for x in 0..width {
                let actual_pixel = actual
                    .get_pixel_checked(x, y)
                    .copied()
                    .unwrap_or(Rgba([0, 0, 0, 0]));
                let expected_pixel = expected
                    .get_pixel_checked(x, y)
                    .copied()
                    .unwrap_or(Rgba([0, 0, 0, 0]));
                if pixels_are_similar(actual_pixel, expected_pixel) {
                    diff_image.put_pixel(
                        x,
                        y,
                        Rgba([
                            expected_pixel[0] / 3,
                            expected_pixel[1] / 3,
                            expected_pixel[2] / 3,
                            255,
                        ]),
                    );
                } else {
                    different_pixels += 1;
                    diff_image.put_pixel(x, y, Rgba([255, 36, 36, 255]));
                }
            }
        }
        let match_ratio = if total_pixels == 0 {
            1.0
        } else {
            (total_pixels - different_pixels) as f64 / total_pixels as f64
        };
        ImageComparison {
            match_ratio,
            different_pixels,
            diff_image,
        }
    }

    fn pixels_are_similar(actual: Rgba<u8>, expected: Rgba<u8>) -> bool {
        actual
            .0
            .into_iter()
            .zip(expected.0)
            .all(|(actual, expected)| {
                (i16::from(actual) - i16::from(expected)).abs() <= CHANNEL_THRESHOLD
            })
    }

    struct Gallery {
        kind: GalleryKind,
    }

    impl Render for Gallery {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let ui_font = theme_settings::setup_ui_font(window, cx);
            let content = match self.kind {
                GalleryKind::AnimationPanel => animation_panel_gallery(cx),
                GalleryKind::Timeline => timeline_gallery(cx),
                GalleryKind::Inspector => inspector_gallery(cx),
            };
            div()
                .size_full()
                .bg(cx.theme().colors().editor_background)
                .text_color(cx.theme().colors().text)
                .font(ui_font)
                .child(content)
        }
    }

    fn animation_panel_gallery(cx: &mut Context<Gallery>) -> AnyElement {
        h_flex()
            .size_full()
            .p_5()
            .gap_4()
            .child(
                v_flex()
                    .w(px(330.))
                    .h_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().panel_background)
                    .child(
                        InspectorSection::new("visual-animation-list", "Animations")
                            .separated(false)
                            .action(
                                IconButton::new("visual-add-animation", IconName::Plus)
                                    .icon_size(IconSize::Small)
                                    .tooltip(Tooltip::text("Add animation")),
                            )
                            .child(
                                AnimationPropertyRow::new(
                                    "visual-position",
                                    AnimationProperty::Position,
                                )
                                .subtitle("Slide in · From left · 500 ms")
                                .selected(true),
                            )
                            .child(
                                AnimationPropertyRow::new("visual-scale", AnimationProperty::Scale)
                                    .subtitle("Scale in · 350 ms"),
                            )
                            .child(AnimationPropertyRow::new(
                                "visual-rotation",
                                AnimationProperty::Rotation,
                            ))
                            .child(AnimationPropertyRow::new(
                                "visual-size",
                                AnimationProperty::Size,
                            ))
                            .child(
                                AnimationPropertyRow::new(
                                    "visual-opacity",
                                    AnimationProperty::Opacity,
                                )
                                .subtitle("Fade in · 400 ms"),
                            )
                            .child(
                                AnimationPropertyRow::new("visual-path", AnimationProperty::Path)
                                    .subtitle("Available for vector paths")
                                    .enabled(false),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().panel_background)
                    .child(AnimationDetailHeader::new(AnimationProperty::Position))
                    .child(InspectorFieldRow::new(
                        "Type",
                        Button::new("visual-type", "Slide in").full_width(),
                    ))
                    .child(InspectorFieldRow::new(
                        "Direction",
                        Button::new("visual-direction", "From left").full_width(),
                    ))
                    .child(InspectorFieldRow::new(
                        "Distance",
                        Button::new("visual-distance", "200").full_width(),
                    ))
                    .child(div().h_3())
                    .child(InspectorFieldRow::new(
                        "Delay",
                        Button::new("visual-delay", "0 ms").full_width(),
                    ))
                    .child(InspectorFieldRow::new(
                        "Duration",
                        Button::new("visual-duration", "500 ms").full_width(),
                    ))
                    .child(InspectorFieldRow::new(
                        "Easing",
                        Button::new("visual-easing", "Ease out").full_width(),
                    ))
                    .child(InspectorEmptyState::new(
                        "Preview ready",
                        "Changes preview live and commit as one undoable edit.",
                    )),
            )
            .into_any_element()
    }

    fn inspector_gallery(cx: &mut Context<Gallery>) -> AnyElement {
        h_flex()
            .size_full()
            .p_5()
            .items_start()
            .justify_center()
            .child(
                v_flex()
                    .w(px(320.))
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().panel_background)
                    .child(
                        InspectorSection::new("visual-typography", "Typography")
                            .separated(false)
                            .child(InspectorFieldRow::new(
                                "Family",
                                FontFamilyPicker::new(
                                    "visual-font-family",
                                    "Source Sans 3",
                                    ["Inter", "Source Sans 3", "Source Serif 4", "Zed Sans"],
                                    |_, _, _| {},
                                ),
                            ))
                            .child(InspectorFieldRow::new(
                                "Weight",
                                Button::new("visual-font-weight", "Regular")
                                    .size(ButtonSize::Medium)
                                    .style(ButtonStyle::Outlined)
                                    .end_icon(
                                        Icon::new(IconName::ChevronDown)
                                            .size(IconSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .full_width(),
                            ))
                            .child(InspectorFieldRow::new(
                                "Size",
                                Button::new("visual-font-size", "16 px")
                                    .size(ButtonSize::Medium)
                                    .style(ButtonStyle::Outlined)
                                    .full_width(),
                            ))
                            .child(InspectorFieldRow::new(
                                "Line height",
                                Button::new("visual-line-height", "1.2×")
                                    .size(ButtonSize::Medium)
                                    .style(ButtonStyle::Outlined)
                                    .full_width(),
                            )),
                    )
                    .child(
                        InspectorSection::new("visual-appearance", "Appearance")
                            .child(InspectorFieldRow::new(
                                "Opacity",
                                Button::new("visual-opacity", "100%")
                                    .size(ButtonSize::Medium)
                                    .style(ButtonStyle::Outlined)
                                    .full_width(),
                            ))
                            .child(InspectorFieldRow::new(
                                "Blend",
                                Button::new("visual-blend", "Normal")
                                    .size(ButtonSize::Medium)
                                    .style(ButtonStyle::Outlined)
                                    .end_icon(
                                        Icon::new(IconName::ChevronDown)
                                            .size(IconSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .full_width(),
                            )),
                    )
                    .child(InspectorEmptyState::new(
                        "Live inspector",
                        "Controls share one compact rhythm and preserve canvas selection while menus are open.",
                    )),
            )
            .into_any_element()
    }

    fn timeline_gallery(cx: &mut Context<Gallery>) -> AnyElement {
        let rail_width = px(242.);
        v_flex()
            .size_full()
            .p_5()
            .justify_end()
            .child(
                v_flex()
                    .h(px(262.))
                    .w_full()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().panel_background)
                    .child(timeline_toolbar(cx))
                    .child(
                        h_flex()
                            .h(px(42.))
                            .flex_none()
                            .border_b_1()
                            .border_color(cx.theme().colors().border_variant)
                            .child(TimelineRulerHeader::new(rail_width, "Layers", "2.0 s"))
                            .child(timeline_ruler()),
                    )
                    .child(track_row(
                        rail_width,
                        IconName::ToolText,
                        "EDIT ME",
                        "Text layer",
                        "EDIT ME",
                        0.0,
                        0.5,
                        true,
                        cx,
                    ))
                    .child(track_row(
                        rail_width,
                        IconName::ToolScale,
                        "Scale",
                        "Property",
                        "Scale",
                        0.0,
                        0.5,
                        false,
                        cx,
                    ))
                    .child(track_row(
                        rail_width,
                        IconName::Eye,
                        "Opacity",
                        "Property",
                        "Opacity",
                        0.36,
                        0.86,
                        false,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn timeline_toolbar(cx: &mut Context<Gallery>) -> impl IntoElement {
        h_flex()
            .h(px(48.))
            .flex_none()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(TimelineToolbarButton::new(
                "visual-play",
                IconName::PlayFilled,
                "Play",
                |_, _| {},
            ))
            .child(TimelineToolbarButton::new(
                "visual-restart",
                IconName::RotateCcw,
                "Restart",
                |_, _| {},
            ))
            .child(
                TimelineToolbarButton::new(
                    "visual-loop",
                    IconName::RotateCw,
                    "Loop playback",
                    |_, _| {},
                )
                .active(true),
            )
            .child(div().w_2())
            .child(TimelineTimecode::new(840_000))
            .child(div().flex_1())
            .child(
                Label::new("Motion timeline")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(div().w_2())
            .child(TimelineToolbarButton::new(
                "visual-zoom-out",
                IconName::Dash,
                "Zoom out",
                |_, _| {},
            ))
            .child(
                div()
                    .relative()
                    .w(px(92.))
                    .h(px(20.))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(9.))
                            .h(px(2.))
                            .rounded_full()
                            .bg(cx.theme().colors().border_variant),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(0.62))
                            .top(px(4.))
                            .size(px(12.))
                            .rounded_full()
                            .bg(cx.theme().colors().text_accent),
                    ),
            )
            .child(TimelineToolbarButton::new(
                "visual-zoom-in",
                IconName::Plus,
                "Zoom in",
                |_, _| {},
            ))
    }

    fn timeline_ruler() -> impl IntoElement {
        let scale = TimelineScale::new(2_000_000, 1.0);
        div()
            .relative()
            .h_full()
            .flex_1()
            .overflow_hidden()
            .children((0..scale.tick_count()).map(move |index| {
                TimelineRulerTick::new(
                    scale.tick_fraction(index),
                    format_ruler_time(scale.tick_time_us(index)),
                )
            }))
            .child(TimelinePlayhead::new(0.42).with_cap(true))
    }

    #[allow(clippy::too_many_arguments)]
    fn track_row(
        rail_width: Pixels,
        icon: IconName,
        name: &'static str,
        kind: &'static str,
        bar_label: &'static str,
        start: f32,
        end: f32,
        selected: bool,
        cx: &mut Context<Gallery>,
    ) -> impl IntoElement {
        h_flex()
            .h(px(56.))
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant.opacity(0.6))
            .when(selected, |row| {
                row.bg(cx.theme().colors().element_selected.opacity(0.6))
            })
            .child(
                h_flex()
                    .h_full()
                    .w(rail_width)
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .border_r_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Icon::new(icon).size(IconSize::Small))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(Label::new(name).size(LabelSize::Small))
                            .child(Label::new(kind).size(LabelSize::XSmall).color(Color::Muted)),
                    ),
            )
            .child(
                div()
                    .relative()
                    .h_full()
                    .flex_1()
                    .overflow_hidden()
                    .children(
                        (0..=10).map(|index| {
                            TimelineGridLine::new(index as f32 / 10.0, index % 5 == 0)
                        }),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(start))
                            .right(relative(1.0 - end))
                            .top(px(11.))
                            .h(px(32.))
                            .px_2()
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().colors().text_accent)
                            .bg(cx.theme().colors().element_selected)
                            .child(
                                Label::new(bar_label)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Accent),
                            ),
                    )
                    .child(TimelinePlayhead::new(0.42)),
            )
    }

    #[cfg(test)]
    mod tests {
        use image::{Rgba, RgbaImage};

        use super::{compare_images, pixels_are_similar};

        #[test]
        fn comparison_is_exact_for_identical_images() {
            let image = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
            let comparison = compare_images(&image, &image);
            assert_eq!(comparison.match_ratio, 1.0);
            assert_eq!(comparison.different_pixels, 0);
        }

        #[test]
        fn comparison_tolerates_two_channel_steps_but_reports_larger_differences() {
            assert!(pixels_are_similar(
                Rgba([10, 20, 30, 255]),
                Rgba([12, 18, 31, 255])
            ));
            assert!(!pixels_are_similar(
                Rgba([10, 20, 30, 255]),
                Rgba([13, 20, 30, 255])
            ));

            let actual = RgbaImage::from_pixel(1, 1, Rgba([13, 20, 30, 255]));
            let expected = RgbaImage::from_pixel(1, 1, Rgba([10, 20, 30, 255]));
            let comparison = compare_images(&actual, &expected);
            assert_eq!(comparison.match_ratio, 0.0);
            assert_eq!(comparison.different_pixels, 1);
        }
    }
}
