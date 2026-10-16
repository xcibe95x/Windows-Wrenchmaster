//! Window chrome and hand-painted widgets.
//!
//! The window is frameless (`with_decorations(false)`), so everything a title
//! bar normally gives you — dragging, double-click maximise, the three buttons,
//! the resize edges — is drawn and handled here. No Tauri, no WebView: this is
//! winit through egui, which already exposes the viewport commands for it.
//!
//! The small widgets below customize egui toggles and meters to match the app's
//! colors and rounded visual style.

use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Id, Pos2, Rect, Response, RichText, Sense,
    Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, ViewportCommand, pos2, vec2,
};

use crate::theme;

/// Segoe Fluent Icons code points. These are the same glyphs the Windows 11
/// shell uses, so the chrome matches the OS it is managing.
pub mod icon {
    pub const MINIMIZE: &str = "\u{E921}";
    pub const MAXIMIZE: &str = "\u{E922}";
    pub const RESTORE: &str = "\u{E923}";
    pub const CLOSE: &str = "\u{E8BB}";

    pub const OVERVIEW: &str = "\u{E80F}";
    pub const SYSTEM_INFO: &str = "\u{E9CE}";
    pub const STORAGE: &str = "\u{E8B7}";
    pub const MEMORY: &str = "\u{E950}";
    pub const DIAGNOSTICS: &str = "\u{E9D9}";
    pub const CHART: &str = "\u{E9D2}";
    pub const HEART: &str = "\u{E95E}";
    pub const EVENTS: &str = "\u{E9D9}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const DEVICES: &str = "\u{E7F4}";
    pub const SCHEDULE: &str = "\u{E823}";
    pub const REGISTRY: &str = "\u{E713}";
    pub const APPS: &str = "\u{E71D}";
    pub const INFO: &str = "\u{E946}";
    pub const CLEANER: &str = "\u{EA99}";
    pub const TWEAKS: &str = "\u{E9E9}";
    pub const TROUBLESHOOT: &str = "\u{E9D9}";
    pub const REPAIR: &str = "\u{E90F}";
    pub const TOOLS: &str = "\u{E957}";
    pub const BACKUP: &str = "\u{E8F7}";
    pub const ABOUT: &str = "\u{E946}";

    pub const SHIELD: &str = "\u{EA18}";
    pub const SUN: &str = "\u{E706}";
    pub const MOON: &str = "\u{E708}";
    pub const FOLDER: &str = "\u{E8B7}";

    // Expander arrows. These live in the icon font on purpose: the text fonts
    // Windows ships have no glyph for U+2304 "DOWN ARROWHEAD", so drawing a
    // chevron as ordinary text renders a tofu box.
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
}

pub const TITLE_BAR_HEIGHT: f32 = 44.0;
const RESIZE_MARGIN: f32 = 6.0;

/// Load the two font families the UI draws with.
///
/// Icons come from Segoe Fluent Icons where Windows 11 has it, falling back to
/// the older MDL2 Assets; both carry every glyph this app asks for.
///
/// The proportional family gets Segoe UI pushed in front of egui's bundled
/// font. That is partly so the app looks like it belongs on Windows, and partly
/// correctness: the bundled font lacks glyphs for some punctuation used in
/// labels, which renders as tofu boxes. egui's font stays behind it as a
/// fallback, so anything Segoe UI misses is still drawn by something.
pub fn install_fonts(ctx: &egui::Context) {
    const ICON_FILES: [&str; 2] = [
        r"C:\Windows\Fonts\SegoeIcons.ttf",
        r"C:\Windows\Fonts\segmdl2.ttf",
    ];
    const TEXT_FILES: [&str; 3] = [
        r"C:\Windows\Fonts\SegUIVar.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\tahoma.ttf",
    ];
    // Every figure in the app is set in this family, so it needs real tabular
    // digits. Cascadia ships with Windows 11 and Consolas with everything
    // older; egui's bundled monospace is the last resort.
    const MONO_FILES: [&str; 3] = [
        r"C:\Windows\Fonts\CascadiaMono.ttf",
        r"C:\Windows\Fonts\CascadiaCode.ttf",
        r"C:\Windows\Fonts\consola.ttf",
    ];

    let mut fonts = egui::FontDefinitions::default();

    if let Some(bytes) = first_readable(&ICON_FILES) {
        fonts.font_data.insert(
            "wrench_icons".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Name("WrenchIcons".into()))
            .or_default()
            .insert(0, "wrench_icons".to_owned());
    }

    // Segoe UI goes in as a fallback for anything Inter lacks.
    if let Some(bytes) = first_readable(&TEXT_FILES) {
        fonts.font_data.insert(
            "wrench_text".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "wrench_text".to_owned());
    }

    // Inter, bundled rather than loaded from the system: it is not installed on
    // a stock Windows, and the app has to look the same on any machine it is
    // carried to. SIL Open Font License, see assets/Inter-OFL.txt.
    const INTER: &[u8] = include_bytes!("../assets/Inter.ttf");
    fonts.font_data.insert(
        "inter".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(INTER)),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());

    if let Some(bytes) = first_readable(&MONO_FILES) {
        fonts.font_data.insert(
            "wrench_mono".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "wrench_mono".to_owned());
    }

    // Inter is always present, so the set is always worth installing.
    ctx.set_fonts(fonts);
}

/// The first of these files that exists and can be read.
fn first_readable(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(|path| std::fs::read(path).ok())
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("WrenchIcons".into()))
}

pub fn icon_text(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph).font(icon_font(size)).color(color)
}

/// Decode the official logo and apply the same 8 px radius it has at 32 px.
/// The mask is alpha-only, so the source artwork and colors remain untouched.
pub fn official_icon_rgba() -> (u32, u32, Vec<u8>) {
    const PNG: &[u8] = include_bytes!("../assets/wrench.png");
    let mut rgba = image::load_from_memory(PNG)
        .expect("bundled Wrenchmaster icon must be a valid PNG")
        .into_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let radius = width.min(height) as f32 * 8.0 / 32.0;
    for y in 0..height {
        for x in 0..width {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let cx = px.clamp(radius, width as f32 - radius);
            let cy = py.clamp(radius, height as f32 - radius);
            let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            let pixel = rgba.get_pixel_mut(x, y);
            pixel[3] = (pixel[3] as f32 * coverage).round() as u8;
        }
    }
    (width, height, rgba.into_raw())
}

/// What the user did on the title bar this frame.
pub struct TitleBarResult {
    pub close_requested: bool,
}

/// Draw the title bar: wordmark on the left, window buttons on the right.
/// The open space remains a drag surface.
pub fn title_bar(ui: &mut Ui, subtitle: &str) -> TitleBarResult {
    let mut close_requested = false;
    let height = ui.available_height();
    let full = ui.available_rect_before_wrap();

    // Drag surface first, so the buttons painted after it win the clicks.
    let drag = ui.interact(full, Id::new("titlebar-drag"), Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    // Left: the generated logo lockup, followed by the current page name.
    let left = Rect::from_min_size(full.min + vec2(16.0, 0.0), vec2(320.0, height));
    ui.scope_builder(UiBuilder::new().max_rect(left), |ui| {
        ui.horizontal_centered(|ui| {
            // Keyed by theme: the lettering is near-white, so the light theme
            // needs the recoloured copy or the title bar reads as empty.
            let dark = ui.visuals().dark_mode;
            let texture_id = Id::new(("wrenchmaster-wordmark-titlebar", dark));
            let wordmark = if let Some(texture) = ui
                .ctx()
                .data(|data| data.get_temp::<egui::TextureHandle>(texture_id))
            {
                texture
            } else {
                let texture = ui.ctx().load_texture(
                    if dark { "wordmark-bar-dark" } else { "wordmark-bar-light" },
                    wordmark_image(dark),
                    egui::TextureOptions::LINEAR,
                );
                ui.ctx()
                    .data_mut(|data| data.insert_temp(texture_id, texture.clone()));
                texture
            };
            let source_size = wordmark.size_vec2();
            let logo_height = height.min(34.0);
            let logo_size = vec2(logo_height * source_size.x / source_size.y, logo_height);
            let logo_rect = ui.allocate_space(logo_size).1;
            ui.painter().image(
                wordmark.id(),
                logo_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.add_space(8.0);
            ui.label(RichText::new(subtitle).size(12.0).color(theme::faint(ui)));
        });
    });

    // Right: minimise, maximise/restore, close.
    let button = vec2(46.0, height);
    let mut cursor = full.max.x;
    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));

    for (glyph, action) in [
        (icon::CLOSE, Action::Close),
        (
            if maximized {
                icon::RESTORE
            } else {
                icon::MAXIMIZE
            },
            Action::ToggleMaximize(maximized),
        ),
        (icon::MINIMIZE, Action::Minimize),
    ] {
        let rect = Rect::from_min_max(
            pos2(cursor - button.x, full.min.y),
            pos2(cursor, full.max.y),
        );
        cursor -= button.x;
        let response = ui.interact(rect, Id::new(("win-btn", glyph)), Sense::click());
        let danger = matches!(action, Action::Close);
        let hovered = response.hovered();

        if hovered {
            let fill = if danger {
                theme::BAD
            } else {
                ui.visuals().widgets.hovered.bg_fill
            };
            ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
        }
        let tint = if hovered && danger {
            Color32::WHITE
        } else if hovered {
            theme::text(ui)
        } else {
            theme::muted(ui)
        };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            icon_font(10.0),
            tint,
        );

        if response.clicked() {
            match action {
                Action::Close => close_requested = true,
                Action::Minimize => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
                Action::ToggleMaximize(now) => {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!now))
                }
            }
        }
    }

    // Hairline under the bar.
    ui.painter().hline(
        full.x_range(),
        full.max.y - 0.5,
        Stroke::new(1.0, theme::line(ui)),
    );

    TitleBarResult { close_requested }
}

enum Action {
    Close,
    Minimize,
    ToggleMaximize(bool),
}

/// Eight invisible hit zones around the window edge that start a native resize.
/// Without decorations the OS no longer provides these.
pub fn resize_edges(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    use egui::ResizeDirection as D;

    let Some(rect) = ctx.input(|i| i.viewport().inner_rect) else {
        return;
    };
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let rect = Rect::from_min_size(Pos2::ZERO, rect.size());
    let m = RESIZE_MARGIN;
    let corner = m * 3.0;

    let zones = [
        (
            Rect::from_min_max(rect.min, rect.min + vec2(corner, corner)),
            D::NorthWest,
        ),
        (
            Rect::from_min_max(
                pos2(rect.max.x - corner, rect.min.y),
                pos2(rect.max.x, rect.min.y + corner),
            ),
            D::NorthEast,
        ),
        (
            Rect::from_min_max(
                pos2(rect.min.x, rect.max.y - corner),
                pos2(rect.min.x + corner, rect.max.y),
            ),
            D::SouthWest,
        ),
        (
            Rect::from_min_max(rect.max - vec2(corner, corner), rect.max),
            D::SouthEast,
        ),
        (
            Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + m)),
            D::North,
        ),
        (
            Rect::from_min_max(pos2(rect.min.x, rect.max.y - m), rect.max),
            D::South,
        ),
        (
            Rect::from_min_max(rect.min, pos2(rect.min.x + m, rect.max.y)),
            D::West,
        ),
        (
            Rect::from_min_max(pos2(rect.max.x - m, rect.min.y), rect.max),
            D::East,
        ),
    ];
    for (index, (zone, direction)) in zones.into_iter().enumerate() {
        let id = Id::new(("resize", index));
        let response = ui.interact(zone.translate(Vec2::ZERO), id, Sense::drag());
        if response.hovered() || response.dragged() {
            ctx.set_cursor_icon(cursor_for(direction));
        }
        if response.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}

fn cursor_for(direction: egui::ResizeDirection) -> egui::CursorIcon {
    use egui::CursorIcon as C;
    use egui::ResizeDirection as D;
    match direction {
        D::North | D::South => C::ResizeVertical,
        D::East | D::West => C::ResizeHorizontal,
        D::NorthEast | D::SouthWest => C::ResizeNeSw,
        D::NorthWest | D::SouthEast => C::ResizeNwSe,
    }
}

/// A sidebar entry: icon, label, and an accent rail on the active one.
pub fn nav_item(
    ui: &mut Ui,
    selected: bool,
    glyph: &str,
    label: &str,
    accent: Color32,
) -> Response {
    nav_item_with_height(ui, selected, glyph, label, accent, 36.0, true)
}

/// A sidebar entry whose height can be animated when it appears in a subtree.
/// A sidebar row that owns a submenu. The chevron is an indicator, not a
/// button: the whole row toggles, so there is no small target to hit.
pub fn nav_group(
    ui: &mut Ui,
    selected: bool,
    glyph: &str,
    label: &str,
    accent: Color32,
    expanded: bool,
) -> Response {
    let response = nav_item(ui, selected, glyph, label, accent);
    if ui.is_rect_visible(response.rect) {
        let marker = if expanded {
            icon::CHEVRON_DOWN
        } else {
            icon::CHEVRON_RIGHT
        };
        ui.painter().text(
            pos2(response.rect.max.x - 12.0, response.rect.center().y),
            Align2::RIGHT_CENTER,
            marker,
            icon_font(10.0),
            theme::faint(ui),
        );
    }
    response
}

pub fn nav_item_with_height(
    ui: &mut Ui,
    selected: bool,
    glyph: &str,
    label: &str,
    accent: Color32,
    height: f32,
    interactive: bool,
) -> Response {
    let width = ui.available_width();
    let sense = if interactive {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let hovered = response.hovered();
    let painter = ui.painter().with_clip_rect(rect);

    if selected {
        painter.rect_filled(rect, CornerRadius::same(8), accent.gamma_multiply(0.16));
        // The rail: a short rounded bar hugging the left edge.
        let rail = Rect::from_min_size(
            pos2(rect.min.x + 1.0, rect.center().y - 8.0),
            vec2(3.0, 16.0),
        );
        painter.rect_filled(rail, CornerRadius::same(2), accent);
    } else if hovered {
        painter.rect_filled(
            rect,
            CornerRadius::same(8),
            ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.7),
        );
    }

    let content = if selected {
        accent
    } else if hovered {
        theme::text(ui)
    } else {
        theme::muted(ui)
    };
    painter.text(
        pos2(rect.min.x + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        glyph,
        icon_font(15.0),
        content,
    );
    painter.text(
        pos2(rect.min.x + 42.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        content,
    );

    response
}

/// A pill toggle. Animated, so flipping one reads as a state change rather than
/// a repaint.
pub fn toggle(ui: &mut Ui, on: &mut bool, accent: Color32) -> Response {
    let size = vec2(38.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let how_on = ui
        .ctx()
        .animate_bool_responsive(response.id.with("on"), *on);
    let enabled = ui.is_enabled();
    let track = if enabled {
        Color32::from_rgb(0x30, 0x3A, 0x49).lerp_to_gamma(accent, how_on)
    } else {
        Color32::from_gray(60)
    };
    let knob = if enabled {
        Color32::from_gray(190).lerp_to_gamma(Color32::WHITE, how_on)
    } else {
        Color32::from_gray(120)
    };

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), track);
    let travel = rect.width() - rect.height();
    let center = pos2(
        rect.min.x + rect.height() * 0.5 + travel * how_on,
        rect.center().y,
    );
    painter.circle_filled(center, rect.height() * 0.5 - 3.0, knob);
    response
}

/// A rounded meter. Replaces `egui::ProgressBar`, which is unmistakably stock.
pub fn meter(ui: &mut Ui, fraction: f32, color: Color32, label: Option<&str>) -> Response {
    meter_inner(ui, fraction, color, label, None)
}

/// The same gauge with a limit drawn on the scale -- the point at which the
/// reading stops being comfortable. Used where a threshold is real, so the
/// colour change has something visible to change against.
pub fn gauge(
    ui: &mut Ui,
    fraction: f32,
    color: Color32,
    label: Option<&str>,
    limit: f32,
) -> Response {
    meter_inner(ui, fraction, color, label, Some(limit))
}

fn meter_inner(
    ui: &mut Ui,
    fraction: f32,
    color: Color32,
    label: Option<&str>,
    limit: Option<f32>,
) -> Response {
    let height = if label.is_some() { 30.0 } else { 8.0 };
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let fraction = fraction.clamp(0.0, 1.0);
    let bar_height = 8.0_f32.min(rect.height());
    // With a label the bar sits on the bottom edge, so the text above it has a
    // line of its own; without one it is centred in the row it was given.
    let bar_top = if label.is_some() {
        rect.max.y - bar_height
    } else {
        rect.min.y + (rect.height() - bar_height) * 0.5
    };
    let bar = Rect::from_min_size(pos2(rect.min.x, bar_top), vec2(rect.width(), bar_height));
    let painter = ui.painter();
    let radius = CornerRadius::same((bar_height * 0.5) as u8);
    painter.rect_filled(bar, radius, ui.visuals().extreme_bg_color);

    if fraction > 0.0 {
        let filled = Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height()));
        painter.rect_filled(filled, radius, color);
    }

    if let Some(limit) = limit {
        // A notch under the track, not a line through it: the limit marks the
        // scale, so it must never be mistaken for part of the reading.
        let x = bar.min.x + bar.width() * limit.clamp(0.0, 1.0);
        painter.vline(
            x,
            bar.max.y..=(bar.max.y + 3.0),
            Stroke::new(1.0, theme::faint(ui)),
        );
    }
    if let Some(label) = label {
        // Left, under the heading it belongs to. Right-aligned it drifted far
        // from the bar on a full-width gauge and read as unrelated.
        painter.text(
            pos2(rect.min.x, rect.min.y),
            Align2::LEFT_TOP,
            label,
            theme::readout(),
            theme::muted(ui),
        );
    }
    response
}

/// Button weights. Three, because an action that deletes files must not look
/// like an action that refreshes a list: `Primary` for the one obvious move on
/// a page, `Ghost` for everything secondary, `Danger` for anything that
/// removes data or rewrites the registry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Primary,
    Ghost,
    Danger,
}

pub fn button(
    ui: &mut Ui,
    weight: Weight,
    glyph: Option<&str>,
    label: &str,
    accent: Color32,
) -> Response {
    let font = FontId::proportional(13.5);
    let text_width = ui
        .fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE))
        .size()
        .x;
    let icon_width = if glyph.is_some() { 22.0 } else { 0.0 };
    let size = vec2(text_width + icon_width + 32.0, 34.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let enabled = ui.is_enabled();
    let hovered = response.hovered() && enabled;
    let down = response.is_pointer_button_down_on() && enabled;

    let base = match weight {
        Weight::Primary => accent,
        Weight::Ghost => ui.visuals().widgets.inactive.bg_fill,
        Weight::Danger => theme::bad(ui),
    };
    let (fill, stroke, text_color) = match weight {
        Weight::Ghost => (
            if hovered {
                ui.visuals().widgets.hovered.bg_fill
            } else {
                base
            },
            Stroke::new(1.0, theme::line(ui)),
            if enabled { theme::text(ui) } else { theme::faint(ui) },
        ),
        _ => {
            let shade = if down {
                0.78
            } else if hovered {
                1.12
            } else {
                1.0
            };
            (
                base.gamma_multiply(if enabled { shade } else { 0.35 }),
                Stroke::NONE,
                if enabled {
                    contrast_on(base)
                } else {
                    theme::faint(ui)
                },
            )
        }
    };

    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(5), fill, stroke, StrokeKind::Inside);

    let mut x = rect.min.x + 13.0;
    if let Some(glyph) = glyph {
        painter.text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            glyph,
            icon_font(13.0),
            text_color,
        );
        x += icon_width;
    }
    painter.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        text_color,
    );
    response
}

/// Dark text on a light accent, light text on a dark one. The accent follows
/// the Windows generation, so it cannot be assumed to be dark.
fn contrast_on(background: Color32) -> Color32 {
    let [r, g, b, _] = background.to_array();
    let luma = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    if luma > 150.0 {
        Color32::from_rgb(0x0C, 0x11, 0x18)
    } else {
        Color32::WHITE
    }
}

/// A badge for a one-word state. Carries its own colour, so a status reads
/// without a sentence around it.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = FontId::proportional(11.0);
    let width = ui
        .fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), color))
        .size()
        .x;
    let (rect, response) = ui.allocate_exact_size(vec2(width + 16.0, 18.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(9),
            color.gamma_multiply(0.14),
            Stroke::new(1.0, color.gamma_multiply(0.45)),
            StrokeKind::Inside,
        );
        painter.text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    }
    response
}

/// Temperature bands, so one colour always means the same thing wherever a
/// reading is drawn: comfortable, normal, warm, running hot.
///
/// The thresholds suit CPU and GPU silicon, which is what reports temperature
/// here. They are deliberately conservative: 85 C is where consumer parts
/// generally start throttling, so that is where the colour turns red.
pub fn temp_color(ui: &Ui, celsius: f32) -> Color32 {
    if celsius < 50.0 {
        theme::good(ui)
    } else if celsius < 70.0 {
        theme::accent_on(ui, theme::ACCENT)
    } else if celsius < 85.0 {
        theme::warn(ui)
    } else {
        theme::bad(ui)
    }
}

/// How full a 0..=1 bar should read for a temperature, mapped over the range
/// worth looking at (30 C idle to 100 C throttling) rather than 0..=100.
pub fn temp_fraction(celsius: f32) -> f32 {
    ((celsius - 30.0) / 70.0).clamp(0.0, 1.0)
}

/// One sensor reading: name on the left, value in its band colour on the
/// right, and a thin bar underneath when the reading has a meaningful scale.
/// Replaces a flat line of grey text, so a hot part is visible at a glance.
pub fn sensor_chip(
    ui: &mut Ui,
    label: &str,
    value: &str,
    color: Color32,
    fraction: Option<f32>,
) -> Response {
    const CHIP_WIDTH: f32 = 168.0;
    ui.scope(|ui| {
        ui.set_width(CHIP_WIDTH);
        // Stack explicitly. A grid cell hands down a left-to-right layout, so
        // without this the bar is laid beside the value instead of under it.
        ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).small().color(theme::muted(ui)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(value).small().strong().color(color));
            });
        });
        match fraction {
            Some(fraction) => {
                // Explicit width rather than the inherited one: in a grid cell
                // the available width collapses, which left the bar invisible.
                let (rect, _) = ui.allocate_exact_size(vec2(CHIP_WIDTH, 6.0), Sense::hover());
                if ui.is_rect_visible(rect) {
                    let radius = CornerRadius::same(3);
                    let painter = ui.painter();
                    painter.rect_filled(rect, radius, ui.visuals().extreme_bg_color);
                    if fraction > 0.0 {
                        let filled = Rect::from_min_size(
                            rect.min,
                            vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
                        );
                        painter.rect_filled(filled, radius, color);
                    }
                }
            }
            // No scale to draw, but still take the space, so chips with and
            // without a bar keep a common baseline.
            None => {
                ui.add_space(6.0);
            }
        }
        ui.add_space(4.0);
        });
    })
    .response
}


/// A sparkline: the shape of a reading over its recent history, drawn small
/// enough to sit under the figure it belongs to.
///
/// It carries no axes on purpose. The question it answers is "steady, climbing
/// or spiking", and axes would cost more room than that answer is worth.
pub fn sparkline(ui: &mut Ui, values: &[f32], color: Color32, ceiling: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), 26.0),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) || values.len() < 2 {
        return response;
    }

    let ceiling = ceiling.max(1.0);
    let step = rect.width() / (values.len() - 1) as f32;
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let t = (value / ceiling).clamp(0.0, 1.0);
            pos2(
                rect.min.x + step * index as f32,
                rect.max.y - t * rect.height(),
            )
        })
        .collect();

    let painter = ui.painter();
    // A faint floor, so a flat line near zero still reads as "on the scale"
    // rather than as a missing chart.
    painter.hline(
        rect.x_range(),
        rect.max.y,
        Stroke::new(1.0, theme::line(ui)),
    );
    painter.add(Shape::line(points, Stroke::new(1.3, color)));
    response
}

/// The wordmark, decoded for the theme it will be drawn on.
///
/// Two things have to happen to the source art. It is exported on a large
/// transparent canvas, so the margins are trimmed -- otherwise it scales to its
/// padding and the logo ends up tiny. And the lettering is near-white, which is
/// invisible on the light theme, so on light those pixels are taken down to the
/// text colour. The blue mark is left alone: it reads on both grounds.
pub fn wordmark_image(dark: bool) -> egui::ColorImage {
    const PNG: &[u8] = include_bytes!("../assets/wrenchmaster-wordmark.png");
    let mut rgba = image::load_from_memory(PNG)
        .expect("bundled wordmark must be a valid PNG")
        .into_rgba8();

    if !dark {
        for px in rgba.pixels_mut() {
            let [r, g, b, a] = px.0;
            if a == 0 {
                continue;
            }
            // Near-white and unsaturated: that is the lettering, not the mark.
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            if min > 140 && (max - min) < 24 {
                let k = 0.16;
                px.0 = [
                    (r as f32 * k) as u8,
                    (g as f32 * k) as u8,
                    (b as f32 * k) as u8,
                    a,
                ];
            }
        }
    }

    // Trim fully transparent margins.
    let (w, h) = (rgba.width(), rgba.height());
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if rgba.get_pixel(x, y).0[3] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]);
    }

    let (cw, ch) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let mut pixels = Vec::with_capacity(cw * ch);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let [r, g, b, a] = rgba.get_pixel(x, y).0;
            pixels.push(Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }
    egui::ColorImage::new([cw, ch], pixels)
}
