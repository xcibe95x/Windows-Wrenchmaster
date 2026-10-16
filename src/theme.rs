//! Visual identity: a bench instrument, not a dashboard.
//!
//! The brief this is built to: someone is standing in front of a machine that
//! is misbehaving and needs to read its condition and act on it. So the look is
//! borrowed from measuring equipment rather than from web apps.
//!
//! Three rules hold the whole thing together.
//!
//! 1. Colour means state. Green, amber and red are indicator lamps and are
//!    never spent on decoration, which is why the chassis is a neutral
//!    graphite: it gives the eye nothing to look at until something is wrong.
//! 2. Every number is a measurement, so numbers are set in a monospace face.
//!    This is not a stylistic tic -- readings refresh while you watch them, and
//!    proportional digits change width as they change value, so a figure
//!    jitters in place and is hard to compare down a column. Tabular figures
//!    hold still.
//! 3. Surfaces earn their weight. A raised panel means a readout you are meant
//!    to read; a hairline row means a list you are meant to scan. One corner
//!    radius on everything flattens that distinction, so there are two.
//!
//! The accent is the one hue that moves: `machine::Era` supplies the colour of
//! the Windows generation underneath, so the app is tinted by the OS it is
//! managing. A neutral chassis is what lets that work -- the blue-tinted ground
//! this replaces only ever flattered a blue accent.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, Style, TextStyle, Theme,
    ThemePreference, Visuals,
};

// ------------------------------------------------------------------ palette

pub const ACCENT: Color32 = Color32::from_rgb(0x4D, 0xA3, 0xFF);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x2B, 0x5E, 0x9E);

/// Indicator lamps, held just off full saturation so a row of them reads as
/// instrumentation rather than confectionery, and so amber and red stay
/// distinguishable from one another at small sizes.
pub const GOOD: Color32 = Color32::from_rgb(0x5F, 0xA9, 0x74);
pub const WARN: Color32 = Color32::from_rgb(0xD9, 0x9A, 0x2B);
pub const BAD: Color32 = Color32::from_rgb(0xD2, 0x56, 0x4F);

pub const TEXT: Color32 = Color32::from_rgb(0xDD, 0xE4, 0xEE);
/// Mid tones, picked to stay legible against both the dark chassis and the
/// light one, so secondary text needs no per-theme branch at the call site.
pub const MUTED: Color32 = Color32::from_rgb(0x7B, 0x88, 0x99);
pub const FAINT: Color32 = Color32::from_rgb(0x8C, 0x99, 0xAB);

/// Indicator colours for the current theme.
///
/// The dark values sit at roughly 6:1 against the dark panel but only 2.2-3.7:1
/// against the light one, which is below the 4.5:1 that small text needs. These
/// are the same hues taken down until they pass on a light ground.
const L_GOOD: Color32 = Color32::from_rgb(0x24, 0x6B, 0x42);
const L_WARN: Color32 = Color32::from_rgb(0x8A, 0x56, 0x06);
const L_BAD: Color32 = Color32::from_rgb(0xA8, 0x27, 0x21);

pub fn good(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode { GOOD } else { L_GOOD }
}

pub fn warn(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode { WARN } else { L_WARN }
}

pub fn bad(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode { BAD } else { L_BAD }
}

/// The era accent, darkened for a light ground. The accent is whatever the
/// Windows generation supplies, so it cannot be hand-picked per theme -- it is
/// taken down proportionally instead.
pub fn accent_on(ui: &egui::Ui, accent: Color32) -> Color32 {
    if ui.visuals().dark_mode {
        accent
    } else {
        let [r, g, b, _] = accent.to_array();
        Color32::from_rgb(
            (r as f32 * 0.55) as u8,
            (g as f32 * 0.55) as u8,
            (b as f32 * 0.55) as u8,
        )
    }
}

/// Primary text for the current theme. Painted widgets have to ask, because
/// unlike labels they do not inherit `override_text_color`.
pub fn text(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode { TEXT } else { L_TEXT }
}

pub fn muted(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        MUTED
    } else {
        Color32::from_rgb(0x4B, 0x63, 0x6D)
    }
}

pub fn faint(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        FAINT
    } else {
        Color32::from_rgb(0x76, 0x8F, 0x9A)
    }
}

/// The chassis. Blue-tinted rather than neutral: flat graphite read as empty,
/// and a cool cast gives the surfaces depth and somewhere for the accent to
/// belong. The ramp still steps clearly -- chassis, panel, raised -- so the
/// surfaces keep meaning different things.
const CHASSIS: Color32 = Color32::from_rgb(0x0F, 0x14, 0x1C);
const PANEL: Color32 = Color32::from_rgb(0x17, 0x1E, 0x29);
const PANEL_RAISED: Color32 = Color32::from_rgb(0x21, 0x2B, 0x3A);
const ETCH: Color32 = Color32::from_rgb(0x2E, 0x3A, 0x4C);

/// The same ramp inverted, tinted cyan rather than run up to white. Paper
/// white on a backlit panel is glare, and it gives the eye nothing to place
/// the surfaces against; a cool tint keeps the ramp readable and still lets
/// the accent sit on it.
const L_CHASSIS: Color32 = Color32::from_rgb(0xDC, 0xE8, 0xEC);
const L_PANEL: Color32 = Color32::from_rgb(0xEF, 0xF6, 0xF8);
const L_PANEL_RAISED: Color32 = Color32::from_rgb(0xCF, 0xE0, 0xE6);
const L_ETCH: Color32 = Color32::from_rgb(0xAF, 0xC4, 0xCD);
const L_TEXT: Color32 = Color32::from_rgb(0x13, 0x23, 0x2B);

// ------------------------------------------------------------------ metrics

/// A radius per role rather than one for everything: panels take the softer
/// shape, controls the tighter one.
const PANEL_RADIUS: CornerRadius = CornerRadius::same(6);
const CONTROL_RADIUS: CornerRadius = CornerRadius::same(5);

/// Vertical rhythm. Three steps, used in place of ad-hoc numbers: inside a
/// group, between groups, between sections.
pub const GAP_TIGHT: f32 = 4.0;
pub const GAP: f32 = 10.0;
pub const GAP_SECTION: f32 = 18.0;

// --------------------------------------------------------------- type scale

/// Named roles, so a heading is picked by what it is instead of by guessing a
/// pixel size at the call site. Sizes step by roughly 1.2 off a 13.5 body.
pub fn page_title() -> FontId {
    FontId::new(23.0, FontFamily::Proportional)
}

pub fn section_title() -> FontId {
    FontId::new(15.5, FontFamily::Proportional)
}

pub fn card_title() -> FontId {
    FontId::new(13.5, FontFamily::Proportional)
}

/// A measurement. Monospace for tabular figures; see rule 2 above.
pub fn readout() -> FontId {
    FontId::new(13.0, FontFamily::Monospace)
}

/// The one big figure on a page: a health score, a total reclaimed.
pub fn readout_large() -> FontId {
    FontId::new(27.0, FontFamily::Monospace)
}

/// `accent` is the Windows-generation colour from `machine::Era`, so the app
/// is tinted by the OS it is running on exactly like the 2017 splash screen.
pub fn apply(ctx: &egui::Context, light: bool, accent: Color32) {
    ctx.set_theme(if light {
        ThemePreference::Light
    } else {
        ThemePreference::Dark
    });

    ctx.all_styles_mut(shape);
    ctx.set_visuals_of(Theme::Dark, accented(dark_visuals(), accent));
    ctx.set_visuals_of(Theme::Light, accented(light_visuals(), accent));
}

fn accented(mut visuals: Visuals, accent: Color32) -> Visuals {
    visuals.selection.bg_fill = accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, accent);
    visuals.hyperlink_color = accent;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.6));
    visuals.widgets.active.bg_fill = accent.gamma_multiply(0.55);
    visuals.widgets.active.weak_bg_fill = accent.gamma_multiply(0.55);
    visuals
}

/// Typography and spacing, identical in both themes.
fn shape(style: &mut Style) {
    // egui enables pixel-alignment debug highlights by default in debug builds;
    // they paint red outlines around widgets and look like a broken UI here.
    #[cfg(debug_assertions)]
    {
        style.debug.show_unaligned = false;
    }

    style.text_styles = [
        (TextStyle::Heading, page_title()),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Proportional),
        ),
        (TextStyle::Monospace, readout()),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(10.0, 7.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.window_margin = Margin::same(14);
    style.spacing.scroll.bar_width = 9.0;
    style.spacing.indent = 20.0;
}

fn dark_visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = CHASSIS;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(0x0A, 0x0E, 0x14);
    visuals.faint_bg_color = PANEL;
    visuals.override_text_color = Some(TEXT);
    visuals.window_stroke = Stroke::new(1.0, ETCH);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, ETCH);
    visuals.widgets.inactive.bg_fill = PANEL_RAISED;
    visuals.widgets.inactive.weak_bg_fill = PANEL_RAISED;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x2B, 0x37, 0x49);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2B, 0x37, 0x49);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    visuals.widgets.active.bg_fill = ACCENT_DIM;
    visuals.widgets.active.weak_bg_fill = ACCENT_DIM;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = BAD;
    shaped(visuals)
}

fn light_visuals() -> Visuals {
    let mut visuals = Visuals::light();
    visuals.panel_fill = L_CHASSIS;
    visuals.window_fill = L_PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(0xC3, 0xD5, 0xDC);
    visuals.faint_bg_color = L_PANEL;
    visuals.override_text_color = Some(L_TEXT);
    visuals.window_stroke = Stroke::new(1.0, L_ETCH);
    visuals.widgets.noninteractive.bg_fill = L_PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, L_ETCH);
    visuals.widgets.inactive.bg_fill = L_PANEL_RAISED;
    visuals.widgets.inactive.weak_bg_fill = L_PANEL_RAISED;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0xC7, 0xDC, 0xE3);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xC7, 0xDC, 0xE3);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.30);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_DIM);
    visuals.hyperlink_color = ACCENT_DIM;
    visuals.warn_fg_color = Color32::from_rgb(0xA8, 0x72, 0x14);
    visuals.error_fg_color = Color32::from_rgb(0xB0, 0x3A, 0x34);
    shaped(visuals)
}

fn shaped(mut visuals: Visuals) -> Visuals {
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = CONTROL_RADIUS;
    }
    visuals.window_corner_radius = PANEL_RADIUS;
    visuals.menu_corner_radius = CONTROL_RADIUS;
    visuals
}

/// A readout panel: raised off the chassis, for figures meant to be read.
///
/// Reads its colours from the active visuals rather than the dark constants,
/// which is what the light theme needs -- hardcoding them left every card dark
/// on a light page.
pub fn card(ui: &egui::Ui) -> egui::Frame {
    let visuals = &ui.visuals().widgets.noninteractive;
    egui::Frame::new()
        .fill(visuals.bg_fill)
        .stroke(Stroke::new(1.0, visuals.bg_stroke.color))
        .corner_radius(PANEL_RADIUS)
        .inner_margin(Margin::same(13))
}

/// A list row: no fill, no border, separated by its hairline alone. For things
/// you scan down rather than read one at a time, where a box per item would
/// add thirty borders and no information.
pub fn row(_ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::TRANSPARENT)
        .inner_margin(Margin::symmetric(2, 7))
}

/// The flat fill used by the sidebar and the status bar.
pub fn chrome(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new().fill(ui.visuals().widgets.noninteractive.bg_fill)
}

pub fn line(ui: &egui::Ui) -> Color32 {
    ui.visuals().widgets.noninteractive.bg_stroke.color
}
