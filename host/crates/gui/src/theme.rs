//! Colour tokens and egui visuals, after Magpie: one quiet graphite surface,
//! a single indigo accent (light #4f46e5, dark #a5b4fc) kept for what acts,
//! ink (fg) for the main button, status as dots and soft badges.
//! Light and dark follow the system.

use eframe::egui;

#[derive(Clone, Copy)]
pub struct Tokens {
    pub bg: egui::Color32,
    pub card: egui::Color32,
    pub card2: egui::Color32,
    pub pill: egui::Color32,
    pub pill_hover: egui::Color32,
    pub line: egui::Color32,
    /// Row dividers inside a card, a step lighter than `line`.
    pub line2: egui::Color32,
    pub fg: egui::Color32,
    pub fg2: egui::Color32,
    pub muted: egui::Color32,
    pub faint: egui::Color32,
    /// Quiet control text, darker than `muted` so it keeps 4.5:1.
    pub ctl: egui::Color32,
    pub accent: egui::Color32,
    pub accent_soft: egui::Color32,
    pub sel: egui::Color32,
    /// The main button is ink, not accent.
    pub ink: egui::Color32,
    pub ink_hover: egui::Color32,
    pub ink_fg: egui::Color32,
    pub green: egui::Color32,
    pub green_soft: egui::Color32,
    pub red: egui::Color32,
    pub red_soft: egui::Color32,
    pub amber: egui::Color32,
    pub amber_soft: egui::Color32,
}

fn rgb(hex: u32) -> egui::Color32 {
    egui::Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub fn tokens(dark: bool) -> Tokens {
    if dark {
        Tokens {
            bg: rgb(0x1a1a1e),
            card: rgb(0x232327),
            card2: rgb(0x1f1f23),
            pill: rgb(0x2d2d33),
            pill_hover: rgb(0x36363d),
            line: rgb(0x333339),
            line2: rgb(0x2c2c32),
            fg: rgb(0xededf1),
            fg2: rgb(0xb9b9c1),
            muted: rgb(0x8b8b94),
            faint: rgb(0x56565f),
            ctl: rgb(0xa1a1aa),
            accent: rgb(0xa5b4fc),
            accent_soft: rgb(0x2b2c47),
            sel: rgb(0x33345a),
            ink: rgb(0xededf1),
            ink_hover: rgb(0xd4d4da),
            ink_fg: rgb(0x1a1a1e),
            green: rgb(0x6fd99b),
            green_soft: rgb(0x1d3328),
            red: rgb(0xf28b82),
            red_soft: rgb(0x3a2222),
            amber: rgb(0xf2b544),
            amber_soft: rgb(0x3a2e1c),
        }
    } else {
        Tokens {
            bg: rgb(0xf4f4f6),
            card: rgb(0xffffff),
            card2: rgb(0xf9f9fb),
            pill: rgb(0xf1f1f4),
            pill_hover: rgb(0xe9e9ee),
            line: rgb(0xe3e3e8),
            line2: rgb(0xececf0),
            fg: rgb(0x1c1c21),
            fg2: rgb(0x4b4b53),
            muted: rgb(0x85858d),
            faint: rgb(0xb9b9c2),
            ctl: rgb(0x63636b),
            accent: rgb(0x4f46e5),
            accent_soft: rgb(0xeeedfd),
            sel: rgb(0xecebfb),
            ink: rgb(0x1c1c21),
            ink_hover: rgb(0x3a3a42),
            ink_fg: egui::Color32::WHITE,
            green: rgb(0x189a58),
            green_soft: rgb(0xe5f4ec),
            red: rgb(0xd64545),
            red_soft: rgb(0xfbeaea),
            amber: rgb(0xb45309),
            amber_soft: rgb(0xfdf2e2),
        }
    }
}

/// Tokens for the theme `ui` is drawn in.
pub fn of(ui: &egui::Ui) -> Tokens {
    tokens(ui.visuals().dark_mode)
}

fn visuals(dark: bool) -> egui::Visuals {
    let t = tokens(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.override_text_color = None;
    v.weak_text_color = Some(t.muted);
    v.hyperlink_color = t.accent;
    v.faint_bg_color = t.pill;
    v.extreme_bg_color = t.card2;
    // inputs sit on a soft fill with no border, as BusStudio's fields do
    v.text_edit_bg_color = Some(t.pill);
    v.code_bg_color = t.card2;
    v.warn_fg_color = t.amber;
    v.error_fg_color = t.red;
    v.window_fill = t.card;
    v.window_stroke = egui::Stroke::new(1.0, t.line);
    v.window_corner_radius = egui::CornerRadius::same(12);
    v.menu_corner_radius = egui::CornerRadius::same(10);
    v.panel_fill = t.bg;
    v.selection.bg_fill = t.sel;
    v.selection.stroke = egui::Stroke::new(1.0, t.accent);
    v.text_cursor.stroke.color = t.accent;
    v.button_frame = true;

    let w = &mut v.widgets;
    w.noninteractive.weak_bg_fill = t.bg;
    w.noninteractive.bg_fill = t.bg;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, t.line);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.noninteractive.corner_radius = egui::CornerRadius::same(8);

    w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    w.inactive.bg_fill = t.pill;
    w.inactive.bg_stroke = egui::Stroke::NONE;
    w.inactive.fg_stroke = egui::Stroke::new(1.0, t.ctl);
    w.inactive.corner_radius = egui::CornerRadius::same(6);

    w.hovered.weak_bg_fill = t.pill;
    w.hovered.bg_fill = t.pill;
    w.hovered.bg_stroke = egui::Stroke::NONE;
    w.hovered.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.hovered.corner_radius = egui::CornerRadius::same(6);

    w.active.weak_bg_fill = t.pill_hover;
    w.active.bg_fill = t.pill_hover;
    w.active.bg_stroke = egui::Stroke::NONE;
    w.active.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.active.corner_radius = egui::CornerRadius::same(6);

    w.open.weak_bg_fill = t.pill;
    w.open.bg_fill = t.card;
    w.open.bg_stroke = egui::Stroke::new(1.0, t.line);
    w.open.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.open.corner_radius = egui::CornerRadius::same(6);
    v
}

pub fn install(ctx: &egui::Context) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let dark = theme == egui::Theme::Dark;
        ctx.style_mut_of(theme, |style| {
            style.visuals = visuals(dark);
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(7.0, 3.0);
            style.spacing.interact_size.y = 22.0;
            style.animation_time = 0.15;
            let prop = egui::FontFamily::Proportional;
            style.text_styles.insert(
                egui::TextStyle::Small,
                egui::FontId::new(11.0, prop.clone()),
            );
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::new(13.0, prop.clone()));
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(12.0, prop.clone()),
            );
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::new(13.5, prop));
            style.text_styles.insert(
                egui::TextStyle::Monospace,
                egui::FontId::new(11.5, egui::FontFamily::Monospace),
            );
        });
    }
}
