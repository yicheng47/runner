use super::*;

pub(super) fn missions_settings_pane(default_crew: Entity<StyledSelect>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .debug_selector(|| "SETTINGS_MISSIONS_PANE".into())
        .child(PaneHeader::new(crate::i18n::text("ui.missions"), ""))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(14. / 16.))
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_DEFAULTS_HEADING".into())
                        .text_size(theme::text_heading())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(crate::i18n::text("ui.defaults")),
                )
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_CARD".into())
                        .child(SettingsCard::new(vec![SettingsRow::new(
                            crate::i18n::text("ui.default_crew"),
                            div()
                                .debug_selector(|| "SETTINGS_DEFAULT_CREW".into())
                                .child(default_crew),
                        )
                        .subtitle(crate::i18n::text(
                            "ui.pre_selected_when_starting_a_new_mission",
                        ))
                        .into_any_element()])),
                ),
        )
        .into_any_element()
}

impl NativeRoot {
    pub(super) fn render_missions_settings(&self, _cx: &mut Context<Self>) -> AnyElement {
        missions_settings_pane(self.settings_page.default_crew.clone())
    }

    pub(super) fn render_appearance_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("ui.appearance"),
                crate::i18n::text(
                    "ui.theme_and_the_app_palette_for_each_mode_terminal_palettes_are_in_terminal",
                ),
            ))
            .child(SettingsCard::new({
                let rows = vec![SettingsRow::new(crate::i18n::text("ui.theme"), self.render_theme_segmented(cx))
                    .subtitle(crate::i18n::text("ui.match_the_os_or_pin_to_light_or_dark"))
                    .into_any_element()];
                #[cfg(target_os = "macos")]
                let rows = {
                    let mut rows = rows;
                    rows.push(SettingsRow::new(crate::i18n::text("ui.window_material"), self.render_material_segmented(cx))
                        .subtitle(crate::i18n::text("ui.glass_adds_depth_to_the_sidebar_and_window_chrome"))
                        .into_any_element());
                    rows
                };
                rows
            }))
            .child(self.render_theme_preview(cx))
            .child(settings_section(
                crate::i18n::text("ui.app_palette"),
                "SETTINGS_APP_PALETTE",
                false,
                vec![
                    SettingsRow::new(
                        crate::i18n::text("ui.light_mode"),
                        self.settings_page.light_theme.clone(),
                    )
                    .subtitle(crate::i18n::text("ui.chrome_colours_when_the_app_is_light"))
                    .into_any_element(),
                    SettingsRow::new(
                        crate::i18n::text("ui.dark_mode"),
                        self.settings_page.dark_theme.clone(),
                    )
                    .subtitle(crate::i18n::text("ui.chrome_colours_when_the_app_is_dark"))
                    .into_any_element(),
                ],
            ))
            .when(cfg!(target_os = "macos"), |page| page.child(
                div().text_size(theme::text_ui()).text_color(theme::muted())
                    .child(crate::i18n::text("ui.runner_follows_macos_reduce_transparency_when_enabled_glass_uses_the_solid_appearance")),
            ))
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn render_material_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        use runner_app::appearance::WindowMaterial;
        let selected = self.settings(cx).window_material;
        div()
            .debug_selector(|| "SETTINGS_WINDOW_MATERIAL".into())
            .flex()
            .gap(rems(2. / 16.))
            .p(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .children(
                [
                    (WindowMaterial::Glass, crate::i18n::text("ui.glass")),
                    (WindowMaterial::Solid, crate::i18n::text("ui.solid")),
                ]
                .into_iter()
                .map(|(material, label)| {
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-material-{label}")))
                        .tab_index(0)
                        .px(rems(12. / 16.))
                        .py(rems(5. / 16.))
                        .rounded(rems(4. / 16.))
                        .text_size(theme::text_ui())
                        .when(material == selected, |item| item.bg(theme::raised()))
                        .cursor_pointer()
                        .child(label)
                        .on_click(move |_, _, cx| {
                            root.update(cx, |this, cx| this.set_window_material(material, cx));
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root
                                    .update(cx, |this, cx| this.set_window_material(material, cx));
                            }
                        })
                }),
            )
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn set_window_material(
        &self,
        material: runner_app::appearance::WindowMaterial,
        cx: &mut Context<Self>,
    ) {
        self.update_app_settings(cx, true, |settings| {
            if settings.window_material == material {
                return false;
            }
            settings.window_material = material;
            true
        });
        runner_app::appearance::configure(material, cx);
        cx.refresh_windows();
    }

    /// Both panes are built from the picks, not from the active variant: the
    /// left pane is always the light picks and the right the dark picks, and
    /// only the outline and caption follow which mode is resolved now.
    pub(super) fn render_theme_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx);
        let light_variant = theme::resolve_variant(
            ThemeIntent::Light,
            true,
            settings.light_app_theme,
            settings.dark_app_theme,
        );
        let dark_variant = theme::resolve_variant(
            ThemeIntent::Dark,
            false,
            settings.light_app_theme,
            settings.dark_app_theme,
        );
        let is_light = theme::active_variant().is_light();
        let pick_root = cx.entity();
        let on_pick: theme_preview::PickHandler = Rc::new(move |mode, _, cx| {
            pick_root.update(cx, |this, cx| this.set_theme_intent(mode.intent(), cx));
        });
        theme_preview::theme_preview(
            PreviewPane {
                mode: PreviewMode::Light,
                colors: theme::colors_for(light_variant),
                palette: settings.light_terminal_theme.palette(),
                font: settings.terminal_font_family.font(cx),
                caption: crate::i18n::tf(
                    "format.light",
                    &[
                        (
                            "app_theme",
                            (light_theme_label(settings.light_app_theme)).to_string(),
                        ),
                        (
                            "terminal_theme",
                            (settings.light_terminal_theme.label()).to_string(),
                        ),
                    ],
                )
                .into(),
                active: is_light,
            },
            PreviewPane {
                mode: PreviewMode::Dark,
                colors: theme::colors_for(dark_variant),
                palette: settings.dark_terminal_theme.palette(),
                font: settings.terminal_font_family.font(cx),
                caption: crate::i18n::tf(
                    "format.dark",
                    &[
                        (
                            "app_theme",
                            (dark_theme_label(settings.dark_app_theme)).to_string(),
                        ),
                        (
                            "terminal_theme",
                            (settings.dark_terminal_theme.label()).to_string(),
                        ),
                    ],
                )
                .into(),
                active: !is_light,
            },
            on_pick,
        )
    }

    pub(super) fn render_theme_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .p(rems(2. / 16.))
            .children(
                [
                    (
                        ThemeIntent::Auto,
                        crate::i18n::text("ui.auto"),
                        "monitor.svg",
                    ),
                    (ThemeIntent::Light, crate::i18n::text("ui.light"), "sun.svg"),
                    (ThemeIntent::Dark, crate::i18n::text("ui.dark"), "moon.svg"),
                ]
                .into_iter()
                .map(|(intent, label, icon)| {
                    let active = intent == self.settings(cx).app_theme;
                    let foreground = if active {
                        theme::text()
                    } else {
                        theme::muted()
                    };
                    let hover_group = SharedString::from(format!(
                        "settings-theme-{}",
                        label.to_ascii_lowercase()
                    ));
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-theme-{label}")))
                        .group(hover_group.clone())
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .rounded(rems(4. / 16.))
                        .px(rems(10. / 16.))
                        .py(rems(5. / 16.))
                        .bg(if active {
                            theme::raised()
                        } else {
                            gpui::transparent_black()
                        })
                        .text_size(theme::text_ui())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(foreground)
                        .cursor(CursorStyle::PointingHand)
                        .hover(|button| button.text_color(theme::text()))
                        .child(
                            svg()
                                .path(icon)
                                .size(rems(12. / 16.))
                                .flex_none()
                                .text_color(foreground)
                                .group_hover(hover_group, |icon| icon.text_color(theme::text())),
                        )
                        .child(label)
                        .on_click(move |_, _, cx| {
                            root.update(cx, |this, root_cx| this.set_theme_intent(intent, root_cx));
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root.update(cx, |this, root_cx| {
                                    this.set_theme_intent(intent, root_cx)
                                });
                            }
                        })
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_terminal_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx);
        let size = settings
            .terminal_font_size
            .clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX);
        let line_height = normalize_terminal_line_height(settings.terminal_line_height);
        let cursor_blink = settings.terminal_cursor_blink;
        let copy_on_select = settings.terminal_copy_on_select;
        let root = cx.entity();
        let size_stepper = {
            let (down, up) = (root.clone(), root.clone());
            Stepper::new(
                "settings-terminal-font-size",
                64.,
                stepper_value(size.to_string(), Some("px")),
                Rc::new(move |_, cx| {
                    down.update(cx, |this, cx| {
                        this.set_terminal_font_size(size.saturating_sub(1), cx)
                    });
                }),
                Rc::new(move |_, cx| {
                    up.update(cx, |this, cx| {
                        this.set_terminal_font_size(size.saturating_add(1), cx)
                    });
                }),
            )
            .decrement_disabled(size == TERMINAL_FONT_SIZE_MIN)
            .increment_disabled(size == TERMINAL_FONT_SIZE_MAX)
        };
        let line_height_stepper = {
            let (down, up) = (root.clone(), root.clone());
            Stepper::new(
                "settings-terminal-line-height",
                64.,
                stepper_value(format!("{line_height:.1}"), None),
                Rc::new(move |_, cx| {
                    down.update(cx, |this, cx| {
                        this.set_terminal_line_height(line_height - 0.1, cx)
                    });
                }),
                Rc::new(move |_, cx| {
                    up.update(cx, |this, cx| {
                        this.set_terminal_line_height(line_height + 0.1, cx)
                    });
                }),
            )
            .decrement_disabled(line_height <= TERMINAL_LINE_HEIGHT_MIN)
            .increment_disabled(line_height >= TERMINAL_LINE_HEIGHT_MAX)
        };
        let toggle = |id: &'static str, on: bool, set: fn(&mut Self, bool, &mut Context<Self>)| {
            let root = root.clone();
            Toggle::new(id, on)
                .on_change(move |enabled, _, cx| {
                    root.update(cx, |this, cx| set(this, enabled, cx));
                })
                .into_any_element()
        };

        let text = settings_section(
            crate::i18n::text("terminal.section_text"),
            "SETTINGS_TERMINAL_TEXT",
            false,
            vec![
                SettingsRow::new(
                    crate::i18n::text("ui.font_family"),
                    div()
                        .debug_selector(|| "SETTINGS_TERMINAL_FONT".into())
                        .child(self.settings_page.terminal_font.clone()),
                )
                .subtitle(crate::i18n::text(
                    "ui.typeface_used_by_the_embedded_terminal",
                ))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("ui.font_size"),
                    ranged_stepper(
                        format!("{TERMINAL_FONT_SIZE_MIN}–{TERMINAL_FONT_SIZE_MAX}"),
                        size_stepper,
                    ),
                )
                .subtitle(crate::i18n::text("ui.glyph_size_for_the_embedded_terminal"))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("terminal.line_height"),
                    ranged_stepper(
                        format!("{TERMINAL_LINE_HEIGHT_MIN:.1}–{TERMINAL_LINE_HEIGHT_MAX:.1}"),
                        line_height_stepper,
                    ),
                )
                .subtitle(crate::i18n::text("terminal.line_height_description"))
                .into_any_element(),
            ],
        );
        let colors = settings_section(
            crate::i18n::text("terminal.section_colors"),
            "SETTINGS_TERMINAL_COLORS",
            false,
            vec![
                SettingsRow::new(
                    crate::i18n::text("ui.light_mode_palette"),
                    self.settings_page.light_terminal_theme.clone(),
                )
                .subtitle(crate::i18n::text(
                    "ui.terminal_colours_when_the_app_is_light",
                ))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("ui.dark_mode_palette"),
                    self.settings_page.dark_terminal_theme.clone(),
                )
                .subtitle(crate::i18n::text(
                    "ui.terminal_colours_when_the_app_is_dark",
                ))
                .into_any_element(),
            ],
        );
        let cursor = settings_section(
            crate::i18n::text("terminal.section_cursor"),
            "SETTINGS_TERMINAL_CURSOR",
            false,
            vec![
                SettingsRow::new(
                    crate::i18n::text("ui.cursor_style"),
                    self.settings_page.terminal_cursor.clone(),
                )
                .subtitle(crate::i18n::text(
                    "ui.block_underline_or_bar_affects_the_prompt_caret_only",
                ))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("terminal.blink_cursor"),
                    toggle(
                        "settings-terminal-cursor-blink",
                        cursor_blink,
                        Self::set_terminal_cursor_blink,
                    ),
                )
                .subtitle(crate::i18n::text("terminal.blink_cursor_description"))
                .into_any_element(),
            ],
        );
        #[cfg(target_os = "macos")]
        let keyboard = Some(settings_section(
            crate::i18n::text("terminal.section_keyboard"),
            "SETTINGS_TERMINAL_KEYBOARD",
            true,
            vec![SettingsRow::new(
                crate::i18n::text("terminal.option_as_meta"),
                div()
                    .debug_selector(|| "SETTINGS_OPTION_AS_META".into())
                    .child(self.settings_page.option_as_meta.clone()),
            )
            .subtitle(crate::i18n::text("terminal.option_as_meta_description"))
            .into_any_element()],
        ));
        #[cfg(not(target_os = "macos"))]
        let keyboard: Option<AnyElement> = None;
        let scrollback = settings_section(
            crate::i18n::text("terminal.section_scrollback_and_selection"),
            "SETTINGS_TERMINAL_SCROLLBACK",
            false,
            vec![
                SettingsRow::new(
                    crate::i18n::text("terminal.scrollback"),
                    self.settings_page.terminal_scrollback.clone(),
                )
                .subtitle(crate::i18n::text("terminal.scrollback_description"))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("terminal.copy_on_select"),
                    toggle(
                        "settings-terminal-copy-on-select",
                        copy_on_select,
                        Self::set_terminal_copy_on_select,
                    ),
                )
                .subtitle(crate::i18n::text("terminal.copy_on_select_description"))
                .into_any_element(),
            ],
        );

        div()
            .flex()
            .flex_col()
            .gap_6()
            .debug_selector(|| "SETTINGS_TERMINAL_PANE".into())
            .child(PaneHeader::new(
                crate::i18n::text("ui.terminal"),
                crate::i18n::text(
                    "ui.text_colours_cursor_keyboard_and_scrollback_for_the_embedded_terminal",
                ),
            ))
            .child(text)
            .child(colors)
            .child(cursor)
            .children(keyboard)
            .child(scrollback)
            .into_any_element()
    }
}

fn stepper_value(value: String, unit: Option<&'static str>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(rems(3. / 16.))
        .font_family(theme::UI_MONOSPACE_FONT)
        .child(
            div()
                .text_size(theme::text_ui())
                .font_weight(FontWeight::MEDIUM)
                .child(value),
        )
        .children(unit.map(|unit| {
            div()
                .text_size(theme::text_caption())
                .text_color(theme::faint())
                .child(unit)
        }))
        .into_any_element()
}

pub(super) fn light_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("runner-light", "Runner Light").swatch(0x00a66a),
        SelectOption::new("catppuccin-latte", "Catppuccin Latte").swatch(0x8839ef),
    ]
}

pub(super) fn light_terminal_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
        SelectOption::new("runner", "Runner").swatch(0x00ff9c),
        SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
    ]
}

pub(super) fn dark_terminal_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("runner", "Runner").swatch(0x00ff9c),
        SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
        SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
    ]
}

/// A section label, with a platform tag when the section exists on one
/// platform only, above its own card.
pub(super) fn settings_section(
    label: &'static str,
    selector: &'static str,
    macos_only: bool,
    rows: Vec<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .debug_selector(move || selector.into())
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(8. / 16.))
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(label)
                .when(macos_only, |heading| {
                    heading.child(
                        div()
                            .px(rems(7. / 16.))
                            .py(rems(2. / 16.))
                            .rounded(rems(5. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .text_size(theme::text_caption())
                            .font_weight(FontWeight::NORMAL)
                            .child("macOS"),
                    )
                }),
        )
        .child(SettingsCard::new(rows))
        .into_any_element()
}

/// A stepper with its range shown to its left, as `8–32`.
fn ranged_stepper(range: String, stepper: Stepper) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .child(
            div()
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(range),
        )
        .child(stepper)
        .into_any_element()
}

/// The presets, plus a saved value that is not one of them so the select can
/// still show it.
pub(super) fn terminal_scrollback_options(saved: usize) -> Vec<SelectOption> {
    let mut values = TERMINAL_SCROLLBACK_PRESETS.to_vec();
    if !values.contains(&saved) {
        values.push(saved);
        values.sort_unstable();
    }
    values
        .into_iter()
        .map(|lines| {
            SelectOption::new(
                lines.to_string(),
                crate::i18n::messagef(
                    "terminal.scrollback_lines",
                    &[("lines", grouped_thousands(lines))],
                ),
            )
        })
        .collect()
}

fn grouped_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

pub(super) fn light_theme_label(value: LightTheme) -> &'static str {
    match value {
        LightTheme::RunnerLight => "Runner Light",
        LightTheme::CatppuccinLatte => "Catppuccin Latte",
    }
}

pub(super) fn dark_theme_label(value: DarkTheme) -> &'static str {
    match value {
        DarkTheme::Runner => "Runner",
        DarkTheme::CatppuccinMocha => "Catppuccin Mocha",
    }
}

pub(super) fn light_theme_value(value: LightTheme) -> &'static str {
    match value {
        LightTheme::RunnerLight => "runner-light",
        LightTheme::CatppuccinLatte => "catppuccin-latte",
    }
}

pub(super) fn parse_light_theme(value: &str) -> Option<LightTheme> {
    match value {
        "runner-light" => Some(LightTheme::RunnerLight),
        "catppuccin-latte" => Some(LightTheme::CatppuccinLatte),
        _ => None,
    }
}

pub(super) fn dark_theme_value(value: DarkTheme) -> &'static str {
    match value {
        DarkTheme::Runner => "carbon",
        DarkTheme::CatppuccinMocha => "catppuccin-mocha",
    }
}

pub(super) fn parse_dark_theme(value: &str) -> Option<DarkTheme> {
    match value {
        "carbon" => Some(DarkTheme::Runner),
        "catppuccin-mocha" => Some(DarkTheme::CatppuccinMocha),
        _ => None,
    }
}

pub(super) fn terminal_cursor_value(value: TerminalCursorStyle) -> &'static str {
    match value {
        TerminalCursorStyle::Block => "block",
        TerminalCursorStyle::Underline => "underline",
        TerminalCursorStyle::Bar => "bar",
    }
}

pub(super) fn parse_terminal_cursor(value: &str) -> Option<TerminalCursorStyle> {
    match value {
        "block" => Some(TerminalCursorStyle::Block),
        "underline" => Some(TerminalCursorStyle::Underline),
        "bar" => Some(TerminalCursorStyle::Bar),
        _ => None,
    }
}
