//! User-facing appearance controls, kept separate from the main navigation
//! and theme palette so visual customization stays additive.

use std::path::PathBuf;

use egui::{
    Align, Align2, Color32, Context, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Theme, Ui, Vec2,
};

use crate::chrome::{self, Pointer};
use crate::settings::{Mode, Settings};
use crate::theme;

#[derive(Clone)]
pub struct Appearance {
    machine_accent: Color32,
    selected_accent: Color32,
    background: Option<Color32>,
    surface: Option<Color32>,
    foreground: Option<Color32>,
    text_scale: f32,
    advanced: bool,
}

impl Appearance {
    pub fn load(machine_accent: Color32) -> Self {
        let mut result = Self {
            machine_accent,
            selected_accent: machine_accent,
            background: None,
            surface: None,
            foreground: None,
            text_scale: 1.0,
            advanced: false,
        };
        if let Some(path) = Self::path() {
            if let Ok(text) = std::fs::read_to_string(path) {
                for line in text.lines() {
                    let Some((key, value)) = line.split_once('=') else {
                        continue;
                    };
                    match key.trim() {
                        "accent" => {
                            result.selected_accent = parse_color(value).unwrap_or(machine_accent)
                        }
                        "background" => result.background = parse_color(value),
                        "surface" => result.surface = parse_color(value),
                        "foreground" => result.foreground = parse_color(value),
                        "text_scale" => {
                            result.text_scale =
                                value.trim().parse::<f32>().unwrap_or(1.0).clamp(0.85, 1.25)
                        }
                        _ => {}
                    }
                }
            }
        }
        result
    }

    fn path() -> Option<PathBuf> {
        crate::util::expand("%APPDATA%").map(|p| p.join("Wrenchmaster").join("appearance.ini"))
    }

    pub fn accent(&self) -> Color32 {
        self.selected_accent
    }

    pub fn save(&self, accent: Color32) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = format!(
            "# Wrenchmaster appearance preferences\naccent = {}\nbackground = {}\nsurface = {}\nforeground = {}\ntext_scale = {:.2}\n",
            color_hex(accent),
            self.background.map(color_hex).unwrap_or_default(),
            self.surface.map(color_hex).unwrap_or_default(),
            self.foreground.map(color_hex).unwrap_or_default(),
            self.text_scale,
        );
        let _ = std::fs::write(path, text);
    }

    /// Reapply the base palette first, then overlay the user's own colors.
    ///
    /// Everything goes through `theme::apply`, including Custom, so no widget
    /// can end up reading a palette the rest of the window is not using. The
    /// overlay afterwards is confined to `Mode::Custom` on purpose: when it
    /// ran in every mode, a background chosen while dark survived the switch
    /// to light and left light-ramp text on a dark surface. Dark and Light now
    /// get exactly what `theme::apply` painted and nothing else, which is why
    /// switching between them is lossless.
    pub fn apply(&self, ctx: &Context, settings: &Settings, accent: Color32) {
        let light = settings.light();
        theme::apply(ctx, light, accent);
        if settings.mode != Mode::Custom {
            // The text scale is not a color and belongs to no single mode, so
            // it is the one thing that still applies everywhere.
            ctx.set_zoom_factor(self.text_scale);
            return;
        }
        let active_theme = if light { Theme::Light } else { Theme::Dark };
        let mut style = (*ctx.style_of(active_theme)).clone();
        if let Some(background) = self.background {
            // Keep every page-level canvas and native egui window on the chosen
            // background instead of leaving nested panels on the base palette.
            style.visuals.panel_fill = background;
            style.visuals.window_fill = background;
        }
        if let Some(surface) = self.surface {
            // Cards, controls, and recessed fields all use the selected surface
            // so the preset reads as a coordinated theme across every page.
            style.visuals.faint_bg_color = surface;
            style.visuals.extreme_bg_color = surface;
            style.visuals.widgets.noninteractive.bg_fill = surface;
            style.visuals.widgets.inactive.bg_fill = surface;
            style.visuals.widgets.inactive.weak_bg_fill = surface;
            style.visuals.widgets.hovered.bg_fill = surface.gamma_multiply(1.08);
            style.visuals.widgets.hovered.weak_bg_fill = surface.gamma_multiply(1.08);
            style.visuals.widgets.active.bg_fill = accent.gamma_multiply(0.45);
            style.visuals.widgets.active.weak_bg_fill = accent.gamma_multiply(0.45);
        }
        if let Some(foreground) = self.foreground {
            style.visuals.override_text_color = Some(foreground);
        }
        ctx.set_style_of(active_theme, style);
        ctx.set_zoom_factor(self.text_scale);
    }

    /// Appearance customization in a compact settings surface. Returns true
    /// when the caller should persist and reapply the selected appearance.
    pub fn page(&mut self, ui: &mut Ui, settings: &mut Settings, accent: &mut Color32) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Appearance")
                        .font(theme::page_title())
                        .strong(),
                );
                ui.label(
                    RichText::new("Choose a comfortable look. Changes apply immediately.")
                        .color(theme::muted(ui)),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Restore defaults").pointer().clicked() {
                    settings.mode = Mode::Dark;
                    settings.custom_light = false;
                    *accent = self.machine_accent;
                    self.background = None;
                    self.surface = None;
                    self.foreground = None;
                    self.text_scale = 1.0;
                    changed = true;
                }
            });
        });
        ui.add_space(theme::GAP_SECTION);

        ui.label(
            RichText::new("Display mode")
                .font(theme::section_title())
                .strong(),
        );
        ui.add_space(theme::GAP_TIGHT);
        let segment_accent = theme::accent_on(ui, *accent);
        ui.horizontal(|ui| {
            for mode in [Mode::Dark, Mode::Light, Mode::Custom] {
                let clicked =
                    chrome::segment(ui, mode.label(), settings.mode == mode, segment_accent)
                        .pointer()
                        .clicked();
                if clicked && settings.mode != mode {
                    settings.mode = mode;
                    changed = true;
                }
            }
        });
        if settings.mode == Mode::Custom {
            // Custom overrides a handful of surfaces, not every one of them,
            // so it still has to stand on a real theme: this picks what an
            // untouched widget looks like underneath the overrides. The two
            // previews are kept here rather than beside the segments because
            // they answer a different question once Custom exists.
            ui.add_space(theme::GAP_TIGHT);
            ui.label(
                RichText::new("Base palette")
                    .small()
                    .color(theme::muted(ui)),
            );
            let preview_width =
                ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(130.0);
            ui.horizontal(|ui| {
                let dark_clicked = mode_preview(
                    ui,
                    "Dark",
                    !settings.custom_light,
                    *accent,
                    false,
                    preview_width,
                );
                let light_clicked = mode_preview(
                    ui,
                    "Light",
                    settings.custom_light,
                    *accent,
                    true,
                    preview_width,
                );
                if dark_clicked && settings.custom_light {
                    settings.custom_light = false;
                    changed = true;
                }
                if light_clicked && !settings.custom_light {
                    settings.custom_light = true;
                    changed = true;
                }
            });
        }
        ui.add_space(theme::GAP);

        ui.label(RichText::new("Theme").font(theme::section_title()).strong());
        ui.add_space(theme::GAP_TIGHT);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let theme_name = preset_for(
                *accent,
                self.background,
                self.surface,
                self.foreground,
                self.machine_accent,
            );
            settings_row(ui, "Theme", "Choose a coordinated accent", |ui| {
                egui::ComboBox::from_id_salt("appearance-theme-preset")
                    .selected_text(theme_name)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(theme_name == "Above", "Above")
                            .pointer()
                            .clicked()
                        {
                            // A preset that paints surfaces is only honoured in
                            // Custom, so choosing one moves you there instead of
                            // appearing to do nothing.
                            settings.mode = Mode::Custom;
                            *accent = above_accent();
                            self.background = Some(above_background());
                            self.surface = Some(above_surface());
                            self.foreground = Some(above_foreground());
                            changed = true;
                        }
                        for (name, color) in accent_presets(self.machine_accent) {
                            if ui
                                .selectable_label(theme_name == name, name)
                                .pointer()
                                .clicked()
                            {
                                *accent = color;
                                self.background = None;
                                self.surface = None;
                                self.foreground = None;
                                changed = true;
                            }
                        }
                        if theme_name == "Custom" {
                            ui.label(RichText::new("Custom palette").color(theme::muted(ui)));
                        }
                    });
            });
            row_separator(ui);

            settings_row(
                ui,
                "Primary color",
                "Accent used for focus and actions",
                |ui| {
                    color_control(ui, accent, &mut changed);
                },
            );
            row_separator(ui);

            // Only Custom carries these three overrides. They are greyed out
            // rather than hidden in Dark and Light: a disabled control still
            // says the feature exists and where it lives, where a section that
            // vanishes reads as a bug. The accent above stays live because it
            // applies to all three modes.
            let custom = settings.mode == Mode::Custom;
            ui.add_enabled_ui(custom, |ui| {
                let mut background = self.background.unwrap_or(ui.visuals().panel_fill);
                settings_row(ui, "Background", "Main surface color", |ui| {
                    if color_control(ui, &mut background, &mut changed) {
                        self.background = Some(background);
                    }
                    if self.background.is_some() && ui.small_button("Default").pointer().clicked() {
                        self.background = None;
                        changed = true;
                    }
                });
                row_separator(ui);

                let mut surface = self
                    .surface
                    .unwrap_or(ui.visuals().widgets.noninteractive.bg_fill);
                settings_row(ui, "Surface / cards", "Panels and metric cards", |ui| {
                    if color_control(ui, &mut surface, &mut changed) {
                        self.surface = Some(surface);
                    }
                    if self.surface.is_some() && ui.small_button("Default").pointer().clicked() {
                        self.surface = None;
                        changed = true;
                    }
                });
                row_separator(ui);

                let mut foreground = self.foreground.unwrap_or(theme::text(ui));
                settings_row(ui, "Foreground", "Primary text color", |ui| {
                    if color_control(ui, &mut foreground, &mut changed) {
                        self.foreground = Some(foreground);
                    }
                    if self.foreground.is_some() && ui.small_button("Default").pointer().clicked() {
                        self.foreground = None;
                        changed = true;
                    }
                });
            });
            row_separator(ui);

            settings_row(
                ui,
                "Font",
                "Wrenchmaster type with Windows fallback",
                |ui| {
                    ui.label(RichText::new("Inter · Segoe UI fallback").color(theme::muted(ui)));
                },
            );
            row_separator(ui);

            settings_row(ui, "Text size", "Scale labels and controls", |ui| {
                ui.label(format!("{}%", (self.text_scale * 100.0).round() as u32));
                changed |= ui
                    .add_sized(
                        Vec2::new((ui.available_width() - 42.0).max(100.0), 20.0),
                        egui::Slider::new(&mut self.text_scale, 0.85..=1.25).show_value(false),
                    )
                    .changed();
            });
        });

        ui.add_space(theme::GAP);
        let advanced = crate::chrome::accordion(
            ui,
            "appearance-advanced",
            "Advanced",
            self.advanced,
            |ui| {
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Custom colors override the base Wrenchmaster palette while the Custom display mode is selected. Use Default on a color row to restore that surface.")
                            .color(theme::muted(ui)),
                    );
                });
            },
        );
        self.advanced = advanced.is_some();
        if changed {
            self.selected_accent = *accent;
            self.save(*accent);
        }
        changed
    }
}

fn settings_row(ui: &mut Ui, label: &str, detail: &str, add_control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).strong());
            ui.label(RichText::new(detail).small().color(theme::muted(ui)));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| add_control(ui));
    });
}

fn row_separator(ui: &mut Ui) {
    ui.add_space(3.0);
    ui.separator();
    ui.add_space(3.0);
}

fn color_control(ui: &mut Ui, color: &mut Color32, changed: &mut bool) -> bool {
    let mut row_changed = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(color_hex(*color))
                .monospace()
                .color(theme::muted(ui)),
        );
        row_changed = ui.color_edit_button_srgba(color).changed();
    });
    *changed |= row_changed;
    row_changed
}

fn mode_preview(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    accent: Color32,
    light: bool,
    width: f32,
) -> bool {
    let size = Vec2::new(width, 104.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let (surface, raised, text, muted) = if light {
        (
            Color32::from_rgb(0xE6, 0xF0, 0xF3),
            Color32::from_rgb(0xFA, 0xFC, 0xFD),
            Color32::from_rgb(0x1D, 0x2B, 0x32),
            Color32::from_rgb(0x75, 0x8A, 0x93),
        )
    } else {
        (
            Color32::from_rgb(0x0F, 0x14, 0x1C),
            Color32::from_rgb(0x21, 0x2B, 0x3A),
            Color32::from_rgb(0xDD, 0xE4, 0xEE),
            Color32::from_rgb(0x7B, 0x88, 0x99),
        )
    };
    let border = if selected { accent } else { theme::line(ui) };
    painter.rect_filled(rect, 10.0, ui.visuals().widgets.inactive.bg_fill);
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(if selected { 2.0 } else { 1.0 }, border),
        StrokeKind::Inside,
    );

    let preview = Rect::from_min_max(
        Pos2::new(rect.left() + 12.0, rect.top() + 10.0),
        Pos2::new(rect.right() - 12.0, rect.bottom() - 30.0),
    );
    painter.rect_filled(preview, 6.0, surface);
    let sidebar = Rect::from_min_size(
        preview.min,
        Vec2::new(preview.width() * 0.24, preview.height()),
    );
    painter.rect_filled(sidebar, 6.0, raised);
    let content_left = sidebar.right() + 8.0;
    let content_width = (preview.right() - content_left - 8.0).max(8.0);
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(content_left, preview.top() + 8.0),
            Vec2::new(content_width * 0.45, 4.0),
        ),
        2.0,
        text,
    );
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(content_left, preview.top() + 17.0),
            Vec2::new(content_width, 5.0),
        ),
        2.0,
        raised,
    );
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(content_left, preview.top() + 27.0),
            Vec2::new(content_width * 0.68, 5.0),
        ),
        2.0,
        accent,
    );
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 15.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        if selected { theme::text(ui) } else { muted },
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .pointer()
        .clicked()
}

fn accent_presets(machine_accent: Color32) -> [(&'static str, Color32); 4] {
    [
        ("Windows", machine_accent),
        ("Ocean", Color32::from_rgb(0x35, 0xA8, 0xE8)),
        ("Violet", Color32::from_rgb(0xA0, 0x78, 0xE8)),
        ("Green", Color32::from_rgb(0x45, 0xB7, 0x8B)),
    ]
}

fn above_accent() -> Color32 {
    Color32::from_rgb(0xED, 0x37, 0x52)
}

fn above_background() -> Color32 {
    Color32::from_rgb(0x16, 0x0A, 0x0D)
}

fn above_surface() -> Color32 {
    Color32::from_rgb(0x25, 0x13, 0x18)
}

fn above_foreground() -> Color32 {
    Color32::WHITE
}

fn preset_for(
    accent: Color32,
    background: Option<Color32>,
    surface: Option<Color32>,
    foreground: Option<Color32>,
    machine_accent: Color32,
) -> &'static str {
    if accent == above_accent()
        && background == Some(above_background())
        && surface == Some(above_surface())
        && foreground == Some(above_foreground())
    {
        return "Above";
    }
    if background.is_some() || surface.is_some() || foreground.is_some() {
        return "Custom";
    }
    accent_presets(machine_accent)
        .into_iter()
        .find(|(_, color)| *color == accent)
        .map(|(name, _)| name)
        .unwrap_or("Custom")
}

fn color_hex(color: Color32) -> String {
    let [r, g, b, _] = color.to_array();
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn parse_color(value: &str) -> Option<Color32> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(value.get(0..2)?, 16).ok()?;
    let green = u8::from_str_radix(value.get(2..4)?, 16).ok()?;
    let blue = u8::from_str_radix(value.get(4..6)?, 16).ok()?;
    Some(Color32::from_rgb(red, green, blue))
}
