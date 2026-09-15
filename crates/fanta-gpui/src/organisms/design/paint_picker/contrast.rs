//! Contrast-checker projection and interactions for the retained paint editor.

use super::*;

impl PaintPicker {
    pub(super) fn current_contrast_leaf(&self) -> Option<&DesignColorContrastLeafViewData> {
        let color_target = self.selected_color_target()?;
        self.contrast_view_data.as_ref()?.leaf(&color_target)
    }

    pub(super) fn resolved_contrast_category(&self) -> Option<DesignColorContrastCategory> {
        let leaf = self.current_contrast_leaf()?;
        Some(match self.contrast_category {
            DesignColorContrastCategory::Auto => leaf.automatic_category,
            category => category,
        })
    }

    pub(super) fn normalized_contrast_level(&self) -> DesignColorContrastLevel {
        if self.resolved_contrast_category() == Some(DesignColorContrastCategory::Graphics) {
            DesignColorContrastLevel::Aa
        } else {
            self.contrast_level
        }
    }

    pub(super) fn render_paint_tools(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.contrast_checker_open
            .then(|| self.render_contrast_checker(cx))
            .flatten()
    }

    pub(super) fn render_contrast_checker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let leaf = self.current_contrast_leaf()?.clone();
        let paint = self.paint.as_ref()?;
        let foreground = picker_color_with_opacity(paint, self.selected_stop);
        let category = self.resolved_contrast_category()?;
        let level = self.normalized_contrast_level();
        let threshold = contrast_threshold(category, level)?;
        let passes = leaf.ratio + f32::EPSILON >= threshold;
        let correction = leaf.correction(category, level);
        let correction_disabled = passes
            || correction.is_none()
            || leaf.disabled_reason.is_some()
            || self.color_editing_disabled();
        let pass_color = if passes {
            cx.theme().green
        } else {
            cx.theme().red
        };

        let mut category_controls = h_flex().w_full().gap_1().flex_wrap();
        for option in DesignColorContrastCategory::ALL {
            category_controls = category_controls.child(
                Button::new(SharedString::from(format!(
                    "{}-contrast-category-{}",
                    self.id,
                    option.label().to_ascii_lowercase().replace(' ', "-")
                )))
                .label(option.label())
                .tooltip(match option {
                    DesignColorContrastCategory::Auto => {
                        SharedString::from(format!("Auto · {}", leaf.automatic_category.label()))
                    }
                    _ => SharedString::from(option.label()),
                })
                .xsmall()
                .compact()
                .ghost()
                .selected(self.contrast_category == option)
                .on_activate(cx.listener(move |this, _, _, cx| {
                    this.contrast_category = option;
                    if option == DesignColorContrastCategory::Graphics {
                        this.contrast_level = DesignColorContrastLevel::Aa;
                    }
                    cx.notify();
                })),
            );
        }

        let mut level_controls = h_flex().gap_1();
        for option in DesignColorContrastLevel::ALL {
            let unavailable = category == DesignColorContrastCategory::Graphics
                && option == DesignColorContrastLevel::Aaa;
            level_controls = level_controls.child(
                Button::new(SharedString::from(format!(
                    "{}-contrast-level-{}",
                    self.id,
                    option.label().to_ascii_lowercase()
                )))
                .label(option.label())
                .tooltip(if unavailable {
                    "AAA applies to text only"
                } else {
                    option.label()
                })
                .xsmall()
                .compact()
                .ghost()
                .selected(level == option)
                .disabled(unavailable)
                .on_activate(cx.listener(move |this, _, _, cx| {
                    if this.resolved_contrast_category()
                        != Some(DesignColorContrastCategory::Graphics)
                        || option == DesignColorContrastLevel::Aa
                    {
                        this.contrast_level = option;
                        cx.notify();
                    }
                })),
            );
        }

        Some(
            v_flex()
                .w_full()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(div().text_xs().font_semibold().child("Contrast"))
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_semibold()
                                        .child(format!("{:.2}:1", leaf.ratio)),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "{}-contrast-correct",
                                        self.id
                                    )))
                                    .label(if passes { "Pass" } else { "Fail" })
                                    .tooltip(leaf.disabled_reason.clone().unwrap_or_else(|| {
                                        if passes {
                                            "Contrast passes".into()
                                        } else if correction.is_some() {
                                            "Adjust to the nearest passing color".into()
                                        } else {
                                            "No host-supplied correction".into()
                                        }
                                    }))
                                    .xsmall()
                                    .compact()
                                    .ghost()
                                    .text_color(pass_color)
                                    .disabled(correction_disabled)
                                    .on_activate(
                                        cx.listener(|this, _, _, cx| {
                                            this.apply_contrast_correction(cx);
                                        }),
                                    ),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .size(px(24.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(color_to_hsla(foreground)),
                        )
                        .child(div().text_xs().child("Selected layer"))
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("on"),
                        )
                        .child(
                            div()
                                .size(px(24.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(color_to_hsla(leaf.effective_background)),
                        ),
                )
                .child(category_controls)
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(level_controls)
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(pass_color)
                                .child(if passes { "Pass" } else { "Fail" }),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} · {} requires {:.1}:1",
                            category.label(),
                            level.label(),
                            threshold
                        )),
                )
                .into_any_element(),
        )
    }

    pub(super) fn apply_contrast_correction(&mut self, cx: &mut Context<Self>) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(category) = self.resolved_contrast_category() else {
            return;
        };
        let level = self.normalized_contrast_level();
        let Some(color) = self
            .current_contrast_leaf()
            .filter(|leaf| {
                leaf.disabled_reason.is_none()
                    && contrast_threshold(category, level)
                        .is_some_and(|threshold| leaf.ratio < threshold)
            })
            .and_then(|leaf| leaf.correction(category, level))
        else {
            return;
        };
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let color = rgb_edit_color(paint, self.selected_stop, color);
        let _ = self.emit_edit(
            color_edit(paint, self.selected_stop, color),
            DesignPanelEditPhase::Commit,
            cx,
        );
    }
}
