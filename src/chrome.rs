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

/// The pointing-hand cursor, as a one-call suffix on any response.
///
/// Windows uses the arrow for everything and leaves it to the visual design
/// to say what is clickable, which works for a button and does not work for
/// the things this app also makes interactive: whole cards, list rows, chart
/// range chips, accordion headers. Rather than remember per widget, every
/// interaction ends in `.pointer()` before its `.clicked()`.
///
/// Calling it twice is harmless -- the cursor is set per frame and the last
/// write wins with the same value -- so the house widgets set it themselves
/// and call sites may set it again without having to know.
pub trait Pointer {
    /// Show the pointing hand while this widget is hovered and enabled.
    ///
    /// Borrows rather than consumes, because plenty of call sites go on to
    /// ask the same response for something else afterwards.
    fn pointer(&self) -> &Self;
}

impl Pointer for Response {
    fn pointer(&self) -> &Self {
        // A disabled control is deliberately not inviting a click, and
        // offering the hand would contradict the greyed-out styling.
        if self.hovered() && self.enabled() {
            self.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        self
    }
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

    // A real semibold face. egui's `.strong()` only changes colour unless a
    // heavier font is registered, so until now nothing in this app could
    // actually be set bold -- which is fine for prose and not fine for a
    // numeral inside a 24px circle, where weight is the only thing that makes
    // it read at all.
    //
    // The family is always created, with Inter behind whatever bold face the
    // machine has: asking egui to lay text out in a family that does not
    // exist is a panic, and this one is named at call sites that cannot check.
    const STRONG_FILES: [&str; 3] = [
        r"C:\Windows\Fonts\seguisb.ttf",
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\arialbd.ttf",
    ];
    let mut strong = Vec::new();
    if let Some(bytes) = first_readable(&STRONG_FILES) {
        fonts.font_data.insert(
            "wrench_strong".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        strong.push("wrench_strong".to_owned());
    }
    strong.push("inter".to_owned());
    fonts
        .families
        .insert(FontFamily::Name("WrenchStrong".into()), strong);

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

/// The semibold face, for the few places where weight is doing a job that
/// colour and size cannot: a figure inside a circle, a value that has to hold
/// its own against a saturated fill.
pub fn strong_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("WrenchStrong".into()))
}

pub fn icon_text(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph).font(icon_font(size)).color(color)
}

/// Standard page section heading: one icon and one title, aligned consistently
/// across analytics panels.
pub fn section_heading(ui: &mut Ui, glyph: &str, title: &str, accent: Color32) {
    ui.horizontal(|ui| {
        ui.label(icon_text(glyph, 18.0, accent));
        ui.label(RichText::new(title).font(theme::section_title()).strong());
    });
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
    pub elevation_requested: bool,
}

/// Draw the title bar: app name and privilege status on the left, window buttons on the right.
/// The open space remains a drag surface.
pub fn title_bar(ui: &mut Ui, elevated: bool) -> TitleBarResult {
    let mut close_requested = false;
    let mut elevation_requested = false;
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
            // Keep the rounded app icon visible beside the app name.
            let icon_id = Id::new("wrenchmaster-official-titlebar-icon");
            let app_icon = if let Some(texture) = ui
                .ctx()
                .data(|data| data.get_temp::<egui::TextureHandle>(icon_id))
            {
                texture
            } else {
                let (icon_width, icon_height, rgba) = official_icon_rgba();
                let texture = ui.ctx().load_texture(
                    "wrenchmaster-official-titlebar-icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [icon_width as usize, icon_height as usize],
                        &rgba,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                ui.ctx()
                    .data_mut(|data| data.insert_temp(icon_id, texture.clone()));
                texture
            };
            let icon_size = 28.0;
            let icon_rect = ui.allocate_space(vec2(icon_size, icon_size)).1;
            ui.painter().image(
                app_icon.id(),
                icon_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("Wrenchmaster")
                    .size(14.5)
                    .strong()
                    .color(theme::text(ui)),
            );
            ui.add_space(10.0);
            if elevated {
                ui.label(
                    RichText::new("Administrator")
                        .small()
                        .color(theme::good(ui)),
                );
            } else {
                elevation_requested = ui
                    .add(
                        egui::Label::new(
                            RichText::new("Standard user")
                                .small()
                                .color(theme::warn(ui)),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Click to restart Wrenchmaster as administrator.")
                    .clicked();
            }
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

    TitleBarResult {
        close_requested,
        elevation_requested,
    }
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
    response.pointer();
    response
}

/// A rounded meter. Replaces `egui::ProgressBar`, which is unmistakably stock.
///
/// There is deliberately no threshold mark on the scale. One was tried and
/// cut a gap across the track near the right-hand end, which read as the bar
/// being broken; and the fill already changes colour when the reading passes
/// the point it stops being comfortable, so the mark only repeated it.
pub fn meter(ui: &mut Ui, fraction: f32, color: Color32, label: Option<&str>) -> Response {
    // Chunky and fully pill-rounded. A hairline bar reads as a divider with
    // some colour on it; at this weight the fill is an object on the card and
    // the length is legible from across the room, which is the entire job.
    let bar_height: f32 = if label.is_some() { 14.0 } else { 10.0 };
    let fraction = fraction.clamp(0.0, 1.0);

    // The label line carries the caption on the left and the reading on the
    // right. The percentage is derived rather than passed: every caller
    // already hands over the fraction, and a bar whose value you have to
    // estimate off its length is a bar missing its most useful digit.
    let value = label.map(|_| format!("{:.0}%", fraction * 100.0));
    let font = FontId::proportional(11.5);
    let painter = ui.painter().clone();
    let value_galley = value
        .as_ref()
        .map(|value| painter.layout_no_wrap(value.clone(), font.clone(), color));

    let line_height = value_galley.as_ref().map_or(0.0, |g| g.size().y + 6.0);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), bar_height + line_height),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }

    if let (Some(label), Some(value_galley)) = (label, value_galley) {
        // One line, hard. `layout` with a wrap width *wraps*, and only one
        // row of height was ever reserved for it, so a caption too long for
        // the column spilled its second line down over the bar and out of
        // the bottom of the card. Truncating with an ellipsis keeps the
        // caption inside the box it was measured for.
        let caption_room = (rect.width() - value_galley.size().x - 10.0).max(24.0);
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.to_owned(),
            theme::readout(),
            theme::muted(ui),
        );
        job.wrap.max_width = caption_room;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some(char::from_u32(0x2026).unwrap_or('.'));
        let caption = painter.layout_job(job);
        painter.galley(rect.min, caption, theme::muted(ui));
        painter.galley(
            pos2(rect.max.x - value_galley.size().x, rect.min.y),
            value_galley,
            color,
        );
    }

    let bar = Rect::from_min_size(
        pos2(rect.min.x, rect.max.y - bar_height),
        vec2(rect.width(), bar_height),
    );
    let radius = CornerRadius::same((bar_height * 0.5) as u8);
    // The unfilled part is context, not content: held well back so the bar
    // reads as one coloured shape rather than as two competing ones.
    painter.rect_filled(bar, radius, theme::track(ui).gamma_multiply(0.45));

    if fraction > 0.0 {
        // A fill narrower than the bar is tall cannot be drawn as a rounded
        // shape -- the two end caps overlap and it renders as a dark smudge
        // or nothing at all. Below that point the reading becomes a dot at
        // the start of the scale: honest about being near zero, and visible.
        let filled = Rect::from_min_size(
            bar.min,
            vec2((bar.width() * fraction).max(bar_height), bar_height),
        );
        painter.rect_filled(filled, radius, color);
    }

    response
}

/// A stacked bar: several parts of one whole on a single pill track.
///
/// Where a `meter` answers "how full", this answers "of what". The segments
/// butt against each other with no gap, so the bar still reads as one length
/// first and a breakdown second.
pub fn stack(ui: &mut Ui, parts: &[(f32, Color32)], height: f32) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let radius = CornerRadius::same((height * 0.5) as u8);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::track(ui).gamma_multiply(0.45));

    let total: f32 = parts.iter().map(|(value, _)| value.max(0.0)).sum();
    if total <= 0.0 {
        return response;
    }
    // Each segment is painted as a full-length pill clipped to its own right
    // edge, back to front. That way the leftmost keeps the round start cap,
    // the last one painted keeps the round end cap, and the joins between
    // them stay square -- which is what makes it read as one bar.
    let mut cursor = 0.0;
    let mut spans: Vec<(f32, Color32)> = Vec::with_capacity(parts.len());
    for (value, color) in parts {
        let value = value.max(0.0);
        cursor += value / total;
        // A part that is actually zero contributes nothing and must not be
        // drawn: its clip would still be widened to the minimum below, and
        // an empty category would show up as a stub of its own colour.
        if value > 0.0 {
            spans.push((cursor, *color));
        }
    }
    for (end, color) in spans.into_iter().rev() {
        // Never narrower than the bar is tall, or the clipped pill's two end
        // caps overlap and the segment renders as a smudge.
        let width = (rect.width() * end).max(height);
        let clip = Rect::from_min_size(rect.min, vec2(width, rect.height()));
        painter
            .with_clip_rect(clip)
            .rect_filled(rect, radius, color);
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
            if enabled {
                theme::text(ui)
            } else {
                theme::faint(ui)
            },
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
    painter.rect(
        rect,
        CornerRadius::same(5),
        fill,
        stroke,
        StrokeKind::Inside,
    );

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
    response.pointer();
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

/// A square icon-only control: one glyph, a hover fill, no label.
///
/// For a control whose glyph already says what it does and which sits
/// somewhere there is no room to say it twice -- the status bar, a card's
/// top-right corner. A labelled switch was tried in the status bar first and
/// gave a once-a-month setting the footprint of a primary control.
///
/// `on` lights the glyph in the accent, so a toggle shows its state without
/// a second element beside it.
pub fn icon_button(ui: &mut Ui, glyph: &str, on: bool, accent: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let painter = ui.painter();
        if hovered {
            painter.rect_filled(
                rect,
                CornerRadius::same(6),
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        let color = if on {
            accent
        } else if hovered {
            theme::text(ui)
        } else {
            theme::muted(ui)
        };
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            icon_font(13.0),
            color,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A card that is itself the button.
///
/// A tile with a small "Open" button in the corner is the worst of both: the
/// card looks clickable, is not, and the one part that is amounts to a tenth
/// of its area. This makes the whole surface the target and says so by
/// lifting on hover.
///
/// The hover fill is read from the previous frame, because the frame has to
/// be painted before the content can be laid inside it and the pointer test
/// needs a rectangle that does not exist yet. One frame of lag on a colour
/// change is not perceptible; reserving a shape and backfilling it would be.
pub fn card_button<R>(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    content: impl FnOnce(&mut Ui) -> R,
) -> (Response, R) {
    let id = ui.id().with(id_source);
    let hovered = ui
        .ctx()
        .read_response(id)
        .is_some_and(|response| response.hovered());

    let visuals = ui.visuals();
    let frame = egui::Frame::new()
        .fill(if hovered {
            visuals.widgets.hovered.bg_fill
        } else {
            visuals.widgets.inactive.bg_fill
        })
        .stroke(Stroke::new(
            1.0,
            if hovered {
                visuals.widgets.hovered.bg_stroke.color
            } else {
                Color32::TRANSPARENT
            },
        ))
        .corner_radius(theme::WIDGET_RADIUS)
        .inner_margin(egui::Margin::same(16));

    let inner = frame.show(ui, |ui| content(ui));
    let response = ui.interact(inner.response.rect, id, Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (response, inner.inner)
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
                        painter.rect_filled(rect, radius, theme::track(ui));
                        if fraction > 0.0 {
                            // At least as wide as it is tall, so a small
                            // reading is a visible dot rather than a smear
                            // the corner radius has eaten.
                            let filled = Rect::from_min_size(
                                rect.min,
                                vec2(
                                    (rect.width() * fraction.clamp(0.0, 1.0)).max(rect.height()),
                                    rect.height(),
                                ),
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
///
/// The trace is backed by a wash that fades out towards the floor. On a dark
/// card a bare hairline is almost nothing; the fill gives the shape mass
/// without drawing a second edge the eye has to read.
pub fn sparkline(ui: &mut Ui, values: &[f32], color: Color32, ceiling: f32) -> Response {
    let height = ui.available_height().clamp(30.0, 46.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
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
                // Held a hair off the ceiling so a pegged reading still draws
                // a line instead of merging with the top edge.
                rect.max.y - t * (rect.height() - 2.0),
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
    area_fill(painter, &points, rect.max.y, color);
    // The newest sample, marked. Without it the trace just stops and which
    // end is "now" is a guess.
    let last = *points.last().expect("checked len >= 2 above");
    painter.add(Shape::line(points, Stroke::new(1.6, color)));
    painter.circle_filled(last, 2.6, color);
    response
}

/// The gradient wash under a trace: the series colour at the line, clear at
/// the floor. Built as a mesh because egui has no gradient brush -- one quad
/// per sample, with the fade carried on the vertex colours.
fn area_fill(painter: &egui::Painter, points: &[Pos2], floor: f32, color: Color32) {
    if points.len() < 2 {
        return;
    }
    let top = color.gamma_multiply(0.28);
    let bottom = color.gamma_multiply(0.0);
    let mut mesh = egui::Mesh::default();
    for point in points {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(*point, top);
        mesh.colored_vertex(pos2(point.x, floor), bottom);
        if base >= 2 {
            mesh.add_triangle(base - 2, base - 1, base);
            mesh.add_triangle(base - 1, base, base + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A ring gauge: one fraction, with the figure it stands for in the hole.
///
/// For a percentage that is the whole story -- a health score, a drive that
/// is 73% full. A bar says the same thing in less room, but it says it as a
/// footnote; the ring makes the number the object on the card.
pub fn ring(
    ui: &mut Ui,
    diameter: f32,
    fraction: f32,
    color: Color32,
    center: &str,
    sub: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let fraction = fraction.clamp(0.0, 1.0);
    let thickness = (diameter * 0.11).clamp(6.0, 12.0);
    let radius = (diameter - thickness) * 0.5;
    let origin = rect.center();
    let painter = ui.painter();

    // egui paints no arcs, so the ring is a polyline with enough segments
    // that the curve reads as a curve rather than as a polygon.
    let arc = |from: f32, sweep: f32, color: Color32| {
        let steps = (sweep.abs() / 0.08).ceil().max(2.0) as usize;
        let points: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let angle = from + sweep * (i as f32 / steps as f32);
                pos2(
                    origin.x + radius * angle.cos(),
                    origin.y + radius * angle.sin(),
                )
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(thickness, color)));
    };

    // Twelve o'clock, clockwise: the way a dial is read.
    let start = -std::f32::consts::FRAC_PI_2;
    arc(start, std::f32::consts::TAU, theme::track(ui));
    if fraction > 0.001 {
        arc(start, std::f32::consts::TAU * fraction, color);
    }

    let has_sub = !sub.is_empty();
    painter.text(
        pos2(origin.x, if has_sub { origin.y - 7.0 } else { origin.y }),
        Align2::CENTER_CENTER,
        center,
        FontId::new((diameter * 0.26).clamp(14.0, 28.0), FontFamily::Monospace),
        color,
    );
    if has_sub {
        painter.text(
            pos2(origin.x, origin.y + 12.0),
            Align2::CENTER_CENTER,
            sub,
            FontId::proportional(10.5),
            theme::muted(ui),
        );
    }
    response
}

/// The key to a chart: a dot in the series colour and its name, laid along
/// one line under the plot. Replaces the plot library's boxed legend, which
/// floats over the data and carries a frame the card does not need.
pub fn legend(ui: &mut Ui, entries: &[(&str, Color32)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        for (label, color) in entries {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, *color);
                ui.label(RichText::new(*label).small().color(theme::muted(ui)));
            });
        }
    });
}

/// One choice in a segmented control -- the range switches above a chart.
///
/// `selectable_label` was doing this job and looked like a list selection: a
/// full-width highlight on a row. These read as what they are, a small set of
/// mutually exclusive options sitting on the card's header line.
pub fn segment(ui: &mut Ui, label: &str, selected: bool, accent: Color32) -> Response {
    let font = FontId::proportional(11.5);
    let width = ui
        .fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE))
        .size()
        .x;
    let (rect, response) = ui.allocate_exact_size(vec2(width + 18.0, 22.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if selected {
            accent.gamma_multiply(0.20)
        } else if hovered {
            theme::track(ui).gamma_multiply(0.6)
        } else {
            Color32::TRANSPARENT
        };
        let text_color = if selected {
            accent
        } else if hovered {
            theme::text(ui)
        } else {
            theme::muted(ui)
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(11),
            fill,
            if selected {
                Stroke::new(1.0, accent.gamma_multiply(0.5))
            } else {
                Stroke::NONE
            },
            StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            font,
            text_color,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A disclosure panel whose header behaves like one of this app's controls.
///
/// egui's `CollapsingHeader` was used at every one of these sites before. It
/// paints a bare triangle beside a default-styled label, which reads as a
/// debug affordance next to the hand-painted cards sharing the page, and its
/// hit area is only as wide as the text, so there is no obvious thing to aim
/// at. The row below spans the panel and fills on hover for that reason.
///
/// The title is laid out against the width actually left over and truncated
/// there. Every text-overflow bug this app has had came from handing a long
/// string straight to `painter.text`, which clips to nothing and happily
/// draws past its container, so headers never take that path.
///
/// The chevron swaps glyph rather than rotating: `painter.text` cannot rotate
/// a glyph, and the two Fluent chevrons are already the pair used elsewhere
/// for expansion, so a swap keeps this consistent with the rest of the chrome.
pub fn accordion<R>(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    default_open: bool,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let id = ui.make_persistent_id(id_source);
    // The open flag lives in egui's persisted store rather than in a field on
    // the caller. Most of these panels are built inside loops over findings or
    // connections, where there is no per-item struct to hang a bool off, and
    // the state still has to survive the next repaint.
    let was_open = ui
        .ctx()
        .data_mut(|data| *data.get_persisted_mut_or(id, default_open));

    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let open = if response.clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_persisted(id, !was_open));
        !was_open
    } else {
        was_open
    };

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        // Muted and small, not body-coloured. Drawn in the text colour at
        // card-title size, a closed "Details" row was indistinguishable from
        // the sentence above it -- a line of plain white text that happened
        // to be clickable. The chevron carries the accent so the row reads as
        // a control at a glance, and the title brightens to full text colour
        // on hover, which is the usual way a quiet control says it is live.
        let title_color = if hovered {
            theme::text(ui)
        } else {
            theme::muted(ui)
        };
        let mark_color = theme::accent_on(ui, theme::ACCENT);

        // Laid out before the painter is borrowed, and capped at the space
        // between the chevron and the right edge rather than at the full row.
        let text_left = rect.left() + 26.0;
        let text_width = (rect.right() - 10.0 - text_left).max(24.0);
        let mut job = egui::text::LayoutJob::simple_singleline(
            title.to_owned(),
            theme::disclosure(),
            title_color,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));

        let painter = ui.painter();
        // A standing surface rather than one that appears on hover: the row
        // has to look like something before the pointer is anywhere near it.
        painter.rect_filled(
            rect,
            theme::WIDGET_RADIUS,
            if hovered {
                ui.visuals().widgets.hovered.bg_fill
            } else {
                theme::track(ui).gamma_multiply(0.25)
            },
        );
        painter.text(
            pos2(rect.left() + 13.0, rect.center().y),
            Align2::CENTER_CENTER,
            if open {
                icon::CHEVRON_DOWN
            } else {
                icon::CHEVRON_RIGHT
            },
            icon_font(11.0),
            mark_color,
        );
        let baseline = rect.center().y - galley.size().y * 0.5;
        painter.galley(pos2(text_left, baseline), galley, title_color);
    }

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if !open {
        return None;
    }

    // A hairline instead of a gap: with several of these stacked, whitespace
    // alone left it ambiguous which body belonged to which header.
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rule.x_range(),
        rule.center().y,
        Stroke::new(1.0, theme::line(ui)),
    );
    ui.add_space(theme::GAP_TIGHT);
    let inner = ui.indent(id.with("body"), content).inner;
    ui.add_space(theme::GAP_TIGHT);
    Some(inner)
}

/// One step of a `steps` track: its colour, whether it has happened yet, and
/// what to say about it on hover.
pub struct Step {
    pub color: Color32,
    pub done: bool,
    pub hover: String,
}

/// A horizontal stepper: a dot per step, joined by a line, detail on hover.
///
/// For a sequence short enough to count and long enough that a row each would
/// take the page -- seventeen machine checks as seventeen lines pushed the
/// findings below the fold, while a plain bar at `done/total` loses which step
/// is which. The dots keep both, in one row: how far along, and that the thing
/// advancing is a list of named steps rather than a timer.
///
/// The detail goes in a tooltip rather than beside each dot, because seventeen
/// labels do not fit across any window and nobody needs all of them at once;
/// they need the one they are pointing at. The tooltip follows the pointer, so
/// which dot it belongs to is never in doubt.
///
/// A finished step is a filled dot and a done segment is a solid line; the
/// ones ahead are hollow dots on the track colour, so the boundary between
/// what has run and what has not is readable at a glance and without colour.
pub fn steps(
    ui: &mut Ui,
    marks: &[Step],
    active: Option<usize>,
    accent: Color32,
    height: f32,
) -> Response {
    // `Sense::click()` rather than `Sense::hover()`, even though nothing here
    // is clickable: a hover-sensed rect is not a widget egui will hang a
    // tooltip on, so the first version painted the dots correctly and showed
    // nothing when you pointed at them.
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if marks.is_empty() {
        return response;
    }
    let count = marks.len();
    // Big enough to hold its own number and to be an easy thing to point at.
    // The first pass drew six-point dots, which read as a dotted line rather
    // than as a row of steps and were fiddly to hover.
    let radius = (height * 0.42).clamp(8.0, 13.0);
    let span = if count > 1 {
        (rect.width() - radius * 2.0) / (count - 1) as f32
    } else {
        0.0
    };
    let centre = |index: usize| pos2(rect.left() + radius + span * index as f32, rect.center().y);
    // Numbers only while they fit inside the dot with room to spare. Past
    // twenty-odd steps in a narrow window the dots crowd and a numeral in
    // each is a smear, so the dots carry the progress alone and the tooltip
    // carries the name -- which is where the name was always going to be.
    let numbered = span > radius * 2.4 && radius >= 9.0;

    if ui.is_rect_visible(rect) {
        let track = theme::track(ui);
        let painter = ui.painter();
        // A joining line is coloured when the step it leads *into* has either
        // finished or started. The line arriving at the running step stands
        // for the work already behind it, which is real progress and should
        // look like it; it is only the line *leaving* the frontier -- into a
        // step nothing has happened to yet -- that stays grey, and that part
        // is dashed as well as grey, so the boundary between done and not
        // done survives being printed, screenshotted or looked at by someone
        // who does not separate those two colours.
        //
        // Each segment stops at the dots rather than running between their
        // centres. Drawn centre to centre it passes under every dot, and a
        // dot that is not filled -- which is every step still to come -- let
        // the line through the middle of its own number.
        for (index, mark) in marks.iter().enumerate().skip(1) {
            let reached = mark.done || active == Some(index);
            let from = centre(index - 1) + vec2(radius + 2.0, 0.0);
            let to = centre(index) - vec2(radius + 2.0, 0.0);
            if to.x <= from.x {
                continue;
            }
            if reached {
                painter.line_segment([from, to], Stroke::new(2.0, accent));
            } else {
                painter.add(egui::Shape::dashed_line(
                    &[from, to],
                    Stroke::new(2.0, track),
                    4.0,
                    4.0,
                ));
            }
        }
        // Inverted: the verdict is the ring and the accent is the fill. A
        // finished step is an accent disc outlined in its own status colour,
        // so the row keeps one body colour down its length and the severity
        // reads as an edge rather than as eighteen competing discs. Red and
        // amber are far louder than the accent at this size, and as fills they
        // dragged the eye to whichever check happened to be worst instead of
        // to the step being pointed at.
        for (index, mark) in marks.iter().enumerate() {
            let point = centre(index);
            let numeral = if mark.done {
                // A darkened accent, not the accent itself. The Windows
                // accent is a bright azure and eighteen of them at full
                // strength is a string of lamps; taken down it sits back on
                // the card and lets the status ring and the numeral read.
                painter.circle_filled(point, radius, accent.gamma_multiply(0.55));
                painter.circle_stroke(point, radius, Stroke::new(2.0, mark.color));
                // White rather than the card's background. The background was
                // near-black on a mid-blue disc, which is the lowest-contrast
                // pair available here; a numeral has to be legible at eleven
                // points or it is decoration.
                Color32::WHITE
            } else if active == Some(index) {
                // The one in progress is a ring, not a disc: filling it would
                // say finished, which during the step that can take a minute
                // is exactly the wrong thing to say. It keeps the accent on
                // the edge, where every unfinished step carries it.
                painter.circle_filled(point, radius, track);
                painter.circle_stroke(point, radius, Stroke::new(2.5, accent));
                theme::text(ui)
            } else {
                // Ahead of the frontier the same ring is dimmed rather than
                // swapped for a grey one: still obviously the same track,
                // still obviously not done yet. Filled with the card's own
                // colour first, so the dot is opaque and nothing drawn behind
                // it reads as part of the number inside it.
                painter.circle_filled(
                    point,
                    radius - 1.0,
                    ui.visuals().widgets.noninteractive.bg_fill,
                );
                painter.circle_stroke(
                    point,
                    radius - 1.0,
                    Stroke::new(1.5, accent.gamma_multiply(0.3)),
                );
                theme::faint(ui)
            };
            if numbered {
                // Laid out and centred on the galley rather than handed to
                // painter.text with CENTER_CENTER: that centres on the font's
                // full line box, ascenders and descenders included, so a digit
                // -- which has neither -- sits visibly low in the circle.
                // Centring the drawn glyph's own box puts it where the eye
                // expects it.
                let label = (index + 1).to_string();
                let font = strong_font((radius * 0.95).min(12.0));
                let galley = painter.layout_no_wrap(label, font, numeral);
                painter.galley(point - galley.size() * 0.5, galley, numeral);
            }
        }
    }

    // The nearest dot to the pointer, not strictly the one under it: pointing
    // between two dots should still answer, rather than blanking the tooltip
    // every time the mouse crosses a gap.
    //
    // `contains_pointer` rather than `hover_pos`, because the pointer being
    // over the row is the whole question here -- `hovered` additionally wants
    // the widget to be the one taking input, which a decorative strip inside a
    // card is not always judged to be.
    if response.contains_pointer() {
        let pointer = ui
            .ctx()
            .pointer_latest_pos()
            .unwrap_or_else(|| rect.center());
        let index = if span > 0.0 {
            (((pointer.x - rect.left() - radius) / span).round().max(0.0) as usize).min(count - 1)
        } else {
            0
        };
        let hover = marks[index].hover.clone();
        return response.on_hover_text_at_pointer(hover);
    }
    response
}

/// Which `accordion_panel` is open, for the whole window.
///
/// Temporary rather than persisted: a fresh run should open on the section the
/// page author chose, not on whatever happened to be open a week ago.
#[derive(Clone, Default)]
struct PanelGroup {
    /// Whether anyone has clicked a panel yet. Until they have, a panel that
    /// asked to start open is allowed to be open; afterwards the user's last
    /// click is the only thing that decides.
    touched: bool,
    open: Option<Id>,
}

/// A whole section behind one disclosure: icon, title, the verdict on the
/// right, and a surface under all three.
///
/// `accordion` is deliberately bare, because most of its uses sit inside a
/// card that already has a border -- a "Details" line under a list row wants
/// no chrome of its own. A section header is the opposite problem: standing
/// alone on the page with nothing but a chevron beside it, it reads as a line
/// of stray text rather than as the lid of the thing below it. This one is
/// built like a card so it looks like something you can open, carries its icon
/// so it is findable by shape, and keeps its status to the right so the page
/// still answers the question with every section shut.
///
/// Header and body share one bordered surface, so an open panel is visibly one
/// object. The same persisted-id scheme as `accordion`: state survives the
/// repaint without the caller holding a bool.
pub fn accordion_panel<R>(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    glyph: &str,
    title: &str,
    status: &str,
    // The colour of the status text. `None` keeps it quiet; a severity colour
    // makes a section announce from its own closed header that there is
    // something serious inside, which is the only way a folded page can still
    // say where to look first.
    status_color: Option<Color32>,
    accent: Color32,
    default_open: bool,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let id = ui.make_persistent_id(id_source);
    // One of these is open at a time, so the group keeps a single id rather
    // than every panel keeping a bool. Three sections open at once is a page
    // several screens long with its own headings scrolled off the top, and
    // closing the last one by hand before opening the next is work the user
    // should not be doing. Clicking the open one still shuts it: the state
    // with nothing open has to stay reachable.
    let group = ui.ctx().data_mut(|data| {
        let group: &mut PanelGroup = data.get_temp_mut_or_default(Id::new("accordion-panel-group"));
        if !group.touched && default_open {
            group.open = Some(id);
        }
        group.clone()
    });
    let was_open = group.open == Some(id);
    let mut inner = None;

    theme::card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());

        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
        let open = if response.clicked() {
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    Id::new("accordion-panel-group"),
                    PanelGroup {
                        touched: true,
                        open: (!was_open).then_some(id),
                    },
                );
            });
            !was_open
        } else {
            was_open
        };

        if ui.is_rect_visible(rect) {
            let hovered = response.hovered();
            let text_color = theme::text(ui);
            let mark_color = if hovered {
                text_color
            } else {
                theme::muted(ui)
            };

            // The status is measured first and the title gets what is left.
            // In the other order a long section name eats the row and the
            // verdict -- the one part worth reading while shut -- falls off
            // the right edge.
            let status_text = status_color.unwrap_or_else(|| theme::muted(ui));
            let status_galley = (!status.is_empty()).then(|| {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    status.to_owned(),
                    theme::readout(),
                    status_text,
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() * 0.45);
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            });
            let status_width = status_galley
                .as_ref()
                .map_or(0.0, |galley| galley.size().x + theme::GAP);

            let text_left = rect.left() + 26.0;
            let text_width = (rect.right() - 24.0 - status_width - text_left).max(24.0);
            let mut job = egui::text::LayoutJob::simple_singleline(
                title.to_owned(),
                theme::card_title(),
                text_color,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));

            let painter = ui.painter();
            painter.text(
                pos2(rect.left() + 9.0, rect.center().y),
                Align2::CENTER_CENTER,
                glyph,
                icon_font(15.0),
                accent,
            );
            painter.galley(
                pos2(text_left, rect.center().y - galley.size().y * 0.5),
                galley,
                text_color,
            );
            if let Some(status_galley) = status_galley {
                painter.galley(
                    pos2(
                        rect.right() - 22.0 - status_galley.size().x,
                        rect.center().y - status_galley.size().y * 0.5,
                    ),
                    status_galley,
                    status_text,
                );
            }
            // The chevron sits at the right edge rather than beside the title:
            // on a row this wide a mark on the left is a long way from the
            // verdict the eye ends on.
            painter.text(
                pos2(rect.right() - 8.0, rect.center().y),
                Align2::CENTER_CENTER,
                if open {
                    icon::CHEVRON_DOWN
                } else {
                    icon::CHEVRON_RIGHT
                },
                icon_font(11.0),
                mark_color,
            );
        }

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        if open {
            ui.add_space(theme::GAP);
            let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().hline(
                rule.x_range(),
                rule.center().y,
                Stroke::new(1.0, theme::line(ui)),
            );
            ui.add_space(theme::GAP);
            inner = Some(content(ui));
        }
    });

    inner
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
