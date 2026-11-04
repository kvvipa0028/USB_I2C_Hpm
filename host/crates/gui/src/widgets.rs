//! Shared controls. Magpie rules: no native select (pop-up fields instead),
//! state as a dot or a soft badge (never a coloured stripe), ink for the main
//! button, the accent only on what acts (focus, open pop-up, selected cell).

use eframe::egui;

use crate::theme::{self, Tokens};

/// Content is at most this wide and centred, as Magpie's pages.
pub const PAGE_MAX_W: f32 = 1200.0;
const FIELD_LABEL_W: f32 = 76.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Green,
    Red,
    Amber,
}

impl Tone {
    /// (text, soft background)
    pub fn colors(self, t: &Tokens) -> (egui::Color32, egui::Color32) {
        match self {
            Tone::Neutral => (t.fg2, t.pill),
            Tone::Green => (t.green, t.green_soft),
            Tone::Red => (t.red, t.red_soft),
            Tone::Amber => (t.amber, t.amber_soft),
        }
    }

    pub fn dot(self, t: &Tokens) -> egui::Color32 {
        match self {
            Tone::Neutral => t.faint,
            other => other.colors(t).0,
        }
    }
}

/// Lay `body` out at most [`PAGE_MAX_W`] wide, centred.
pub fn page_width<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let full = ui.available_rect_before_wrap();
    let rect = if full.width() > PAGE_MAX_W {
        let x = full.center().x - PAGE_MAX_W / 2.0;
        egui::Rect::from_x_y_ranges(x..=x + PAGE_MAX_W, full.y_range())
    } else {
        full
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), body)
        .inner
}

/// Page title, a badge after it, one grey line under it, controls on the right.
pub fn page_head(
    ui: &mut egui::Ui,
    title: &str,
    badge: Option<(&str, Tone)>,
    subtitle: &str,
    right: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).size(16.0).strong());
                if let Some((text, tone)) = badge {
                    self::badge(ui, text, tone);
                }
            });
            ui.label(
                egui::RichText::new(subtitle)
                    .size(12.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        trailing(ui, 40.0, right);
    });
    ui.add_space(4.0);
}

/// Lay `add` out right to left in a strip `height` tall that fills the rest
/// of the row. (`with_layout` alone takes all the height below the row.)
pub fn trailing<R>(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        add,
    )
    .inner
}

/// Small grey group title.
pub fn group_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(11.0)
            .color(ui.visuals().weak_text_color())
            .extra_letter_spacing(0.4),
    );
}

fn card_frame(t: &Tokens, margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.line))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(margin))
}

/// Group title above a full-width card.
pub fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        group_title(ui, title);
        let t = theme::of(ui);
        card_frame(&t, 12).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            body(ui);
        });
    });
}

/// BusStudio panel: a card with a head row (title, then `head` on the right),
/// a hairline, and the body.
pub fn panel(
    ui: &mut egui::Ui,
    title: &str,
    head: impl FnOnce(&mut egui::Ui),
    body: impl FnOnce(&mut egui::Ui),
) {
    let t = theme::of(ui);
    card_frame(&t, 0).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(title).size(13.0).strong());
                    trailing(ui, 26.0, head);
                });
            });
        divider(ui, t.line);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                body(ui);
            });
    });
}

fn divider(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, color),
    );
}

/// Hairline between rows inside a card.
pub fn hairline(ui: &mut egui::Ui) {
    let t = theme::of(ui);
    divider(ui, t.line2);
}

/// One label/value row of a config card; `last` leaves out the divider.
pub fn field_row(ui: &mut egui::Ui, label: &str, last: bool, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(FIELD_LABEL_W, 22.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
        );
        body(ui);
    });
    if !last {
        hairline(ui);
    }
}

/// A value the firmware fixes: plain text in a soft field, with why on hover.
pub fn fixed_value(ui: &mut egui::Ui, value: &str, why: &str) -> egui::Response {
    let t = theme::of(ui);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(6), t.card2);
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::proportional(12.0),
        t.fg2,
    );
    ui.painter().text(
        rect.right_center() - egui::vec2(8.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "固定",
        egui::FontId::proportional(10.5),
        t.muted,
    );
    resp.on_hover_text(why)
}

/// Status dot, 6 px.
pub fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, color);
}

/// Soft pill badge.
pub fn badge(ui: &mut egui::Ui, text: &str, tone: Tone) -> egui::Response {
    let t = theme::of(ui);
    let (fg, bg) = tone.colors(&t);
    let font = egui::FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, fg);
    let size = egui::vec2(galley.size().x + 16.0, 19.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(99), bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

fn styled_button(
    ui: &mut egui::Ui,
    enabled: bool,
    label: &str,
    fill: [egui::Color32; 2],
    stroke: egui::Stroke,
    text: egui::Color32,
) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(11.0, 4.0);
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = fill[0];
        w.hovered.weak_bg_fill = fill[1];
        w.active.weak_bg_fill = fill[1];
        for s in [&mut w.inactive, &mut w.hovered, &mut w.active] {
            s.bg_stroke = stroke;
            s.corner_radius = egui::CornerRadius::same(7);
        }
        let text = egui::RichText::new(label).size(12.0).color(text);
        ui.add_enabled(
            enabled,
            egui::Button::new(text).min_size(egui::vec2(0.0, 27.0)),
        )
    })
    .inner
}

/// The main action of a block: ink fill (fg), not accent.
pub fn ink_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    let t = theme::of(ui);
    styled_button(
        ui,
        enabled,
        label,
        [t.ink, t.ink_hover],
        egui::Stroke::NONE,
        t.ink_fg,
    )
}

/// Secondary action: card fill with a hairline edge.
pub fn action_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    let t = theme::of(ui);
    styled_button(
        ui,
        enabled,
        label,
        [t.card, t.pill],
        egui::Stroke::new(1.0, t.line),
        t.fg,
    )
}

/// Text button with no fill until hovered.
pub fn quiet_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    let t = theme::of(ui);
    styled_button(
        ui,
        enabled,
        label,
        [egui::Color32::TRANSPARENT, t.pill],
        egui::Stroke::NONE,
        t.ctl,
    )
}

/// Magpie pop-up field: grey pill with a faint key, the value, and a chevron.
/// Open the menu with `egui::Popup::menu(&response)`.
pub fn popup_field(
    ui: &mut egui::Ui,
    key: &str,
    value: &str,
    dot: Option<egui::Color32>,
    width: f32,
) -> egui::Response {
    let t = theme::of(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let radius = egui::CornerRadius::same(6);
    let fill = if resp.hovered() || open {
        t.pill_hover
    } else {
        t.pill
    };
    let p = ui.painter();
    p.rect_filled(rect, radius, fill);
    if open {
        p.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.0, t.accent),
            egui::StrokeKind::Inside,
        );
    }
    let mut x = rect.left() + 10.0;
    if let Some(c) = dot {
        p.circle_filled(egui::pos2(x + 3.0, rect.center().y), 3.0, c);
        x += 12.0;
    }
    if !key.is_empty() {
        let r = p.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            key,
            egui::FontId::proportional(11.5),
            t.muted,
        );
        x = r.right() + 7.0;
    }
    let chevron_x = rect.right() - 13.0;
    let clip = egui::Rect::from_x_y_ranges(x..=chevron_x - 8.0, rect.y_range());
    p.with_clip_rect(clip).text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::proportional(12.5),
        t.fg,
    );
    let c = egui::pos2(chevron_x, rect.center().y);
    let stroke = egui::Stroke::new(1.3, t.muted);
    p.line_segment(
        [c + egui::vec2(-3.5, -1.5), c + egui::vec2(0.0, 2.0)],
        stroke,
    );
    p.line_segment(
        [c + egui::vec2(0.0, 2.0), c + egui::vec2(3.5, -1.5)],
        stroke,
    );
    resp
}

/// Pop-up field for one config row: the current option's label, a menu of
/// the rest. Returns true when the pick changed.
pub fn pick<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let label = options.iter().find(|o| o.0 == *value).map_or("", |o| o.1);
    let width = ui.available_width();
    let resp = popup_field(ui, "", label, None, width);
    let before = *value;
    egui::Popup::menu(&resp).width(width).show(|ui| {
        ui.set_min_width(width - 12.0);
        for &(v, label) in options {
            ui.selectable_value(value, v, label);
        }
    });
    *value != before
}

/// Single-line monospace field with a faint unit after it.
pub fn unit_edit(ui: &mut egui::Ui, text: &mut String, hint: &str, unit: &str) -> egui::Response {
    let t = theme::of(ui);
    let unit_w = if unit.is_empty() { 0.0 } else { 26.0 };
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .font(egui::TextStyle::Monospace)
            .desired_width(ui.available_width() - unit_w),
    );
    if !unit.is_empty() {
        ui.label(egui::RichText::new(unit).size(11.0).color(t.muted));
    }
    resp
}

/// 高级参数-style row: a chevron, the title, and how many items it hides.
/// Click flips `open`; the caller shows the items when it is set.
pub fn disclosure(ui: &mut egui::Ui, open: &mut bool, title: &str, count: usize) {
    let t = theme::of(ui);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), t.card2);
    }
    let c = rect.left_center() + egui::vec2(8.0, 0.0);
    let stroke = egui::Stroke::new(1.3, t.muted);
    let pts = if *open {
        [
            egui::vec2(-3.5, -1.5),
            egui::vec2(0.0, 2.0),
            egui::vec2(3.5, -1.5),
        ]
    } else {
        [
            egui::vec2(-1.5, -3.5),
            egui::vec2(2.0, 0.0),
            egui::vec2(-1.5, 3.5),
        ]
    };
    ui.painter().line_segment([c + pts[0], c + pts[1]], stroke);
    ui.painter().line_segment([c + pts[1], c + pts[2]], stroke);
    let r = ui.painter().text(
        rect.left_center() + egui::vec2(20.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(12.0),
        t.fg2,
    );
    ui.painter().text(
        r.right_center() + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{count} 项"),
        egui::FontId::proportional(11.5),
        t.muted,
    );
    if resp.clicked() {
        *open = !*open;
    }
}

/// BusStudio summary band: label / value pairs on a soft strip, a faint
/// note at the right end.
pub fn summary_strip(ui: &mut egui::Ui, items: &[(&str, String)], note: &str) {
    let t = theme::of(ui);
    egui::Frame::new()
        .fill(t.card2)
        .stroke(egui::Stroke::new(1.0, t.line2))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 7))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                for (label, value) in items {
                    ui.label(egui::RichText::new(*label).size(11.5).color(t.muted));
                    ui.label(egui::RichText::new(value).monospace().color(t.fg));
                    ui.add_space(10.0);
                }
                trailing(ui, 18.0, |ui| {
                    ui.label(egui::RichText::new(note).size(11.0).color(t.muted));
                });
            });
        });
}

/// A circled "i" and a short muted note that wraps.
pub fn note(ui: &mut egui::Ui, text: &str) {
    let t = theme::of(ui);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 16.0), egui::Sense::hover());
        let c = rect.center();
        ui.painter()
            .circle_stroke(c, 5.5, egui::Stroke::new(1.0, t.muted));
        ui.painter().line_segment(
            [c + egui::vec2(0.0, -0.5), c + egui::vec2(0.0, 2.8)],
            egui::Stroke::new(1.1, t.muted),
        );
        ui.painter()
            .circle_filled(c + egui::vec2(0.0, -2.6), 0.7, t.muted);
        ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(t.muted)).wrap());
    });
}

/// Muted line under a table: what it shows on the left, scope on the right.
pub fn table_foot(ui: &mut egui::Ui, left: &str, right: &str) {
    let t = theme::of(ui);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(left).size(11.0).color(t.muted));
        trailing(ui, 16.0, |ui| {
            ui.label(egui::RichText::new(right).size(11.0).color(t.muted));
        });
    });
}

/// Horizontal slider in Magpie's look: a thin track, the filled part and the
/// knob in accent. Snaps to `step`; ← / → move one step while focused.
pub fn slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
) -> egui::Response {
    let t = theme::of(ui);
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 22.0),
        egui::Sense::click_and_drag(),
    );
    let track = rect.shrink2(egui::vec2(8.0, 0.0));
    let snap = |v: f32| ((v / step).round() * step).clamp(lo, hi);
    let before = *value;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let f = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
            *value = snap(lo + f * (hi - lo));
            resp.request_focus();
        }
    }
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                resp.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            )
        });
        let (left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if left {
            *value = snap(*value - step);
        }
        if right {
            *value = snap(*value + step);
        }
    }
    if *value != before {
        resp.mark_changed();
    }
    let f = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0);
    let y = rect.center().y;
    let knob = egui::pos2(track.left() + f * track.width(), y);
    let bar = |a: f32, b: f32| egui::Rect::from_x_y_ranges(a..=b, y - 2.0..=y + 2.0);
    let p = ui.painter();
    p.rect_filled(
        bar(track.left(), track.right()),
        egui::CornerRadius::same(2),
        t.pill_hover,
    );
    p.rect_filled(
        bar(track.left(), knob.x),
        egui::CornerRadius::same(2),
        t.accent,
    );
    let r = if resp.hovered() || resp.dragged() {
        7.5
    } else {
        7.0
    };
    p.circle_filled(knob, r, t.accent);
    p.circle_filled(knob, r - 4.5, t.card);
    if resp.has_focus() {
        p.circle_stroke(knob, r + 2.5, egui::Stroke::new(1.0, t.accent_soft));
    }
    resp
}

/// Segmented control without a grey track: the selected item is a card
/// with a breath of shadow, the rest are quiet text.
pub fn seg<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    let t = theme::of(ui);
    let dark = ui.visuals().dark_mode;
    // inside a right-to-left layout the row runs backwards; keep the reading order
    let rtl = ui.layout().prefer_right_to_left();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let order: Vec<&(T, &str)> = if rtl {
            options.iter().rev().collect()
        } else {
            options.iter().collect()
        };
        for &&(v, label) in &order {
            let font = egui::FontId::proportional(12.5);
            let on = *value == v;
            let color = if on { t.fg } else { t.ctl };
            let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
            let size = egui::vec2(galley.size().x + 26.0, 26.0);
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            let radius = egui::CornerRadius::same(6);
            if on {
                let shadow = egui::Shadow {
                    offset: [0, 1],
                    blur: 3,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(if dark { 70 } else { 24 }),
                };
                ui.painter().add(shadow.as_shape(rect, radius));
                ui.painter().rect_filled(rect, radius, t.card);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, radius, t.pill);
            }
            ui.painter()
                .galley(rect.center() - galley.size() / 2.0, galley, color);
            if resp.clicked() {
                *value = v;
            }
        }
    });
}

/// Fixed-column table drawn with hairlines. Widths of 0 share what is left.
pub struct Table<'a> {
    pub widths: &'a [f32],
}

impl Table<'_> {
    fn columns(&self, rect: egui::Rect) -> Vec<egui::Rect> {
        let fixed: f32 = self.widths.iter().sum();
        let flex = self.widths.iter().filter(|w| **w == 0.0).count().max(1) as f32;
        let share = ((rect.width() - fixed) / flex).max(40.0);
        let mut x = rect.left();
        self.widths
            .iter()
            .map(|&w| {
                let w = if w == 0.0 { share } else { w };
                let r = egui::Rect::from_x_y_ranges(x..=x + w, rect.y_range());
                x += w;
                r
            })
            .collect()
    }

    pub fn header(&self, ui: &mut egui::Ui, names: &[&str]) {
        let t = theme::of(ui);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), t.card2);
        for (cell, name) in self.columns(rect).into_iter().zip(names) {
            ui.painter().text(
                cell.left_center() + egui::vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                *name,
                egui::FontId::proportional(11.0),
                t.muted,
            );
        }
    }

    /// One row; each cell is (text, colour, monospace). Returns the row response.
    pub fn row(
        &self,
        ui: &mut egui::Ui,
        cells: &[(String, egui::Color32, bool)],
        selected: bool,
    ) -> egui::Response {
        let t = theme::of(ui);
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        if selected {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(5), t.sel);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(5), t.card2);
        }
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, t.line2),
        );
        for (cell, (text, color, mono)) in self.columns(rect).into_iter().zip(cells) {
            let font = if *mono {
                egui::FontId::monospace(11.5)
            } else {
                egui::FontId::proportional(12.0)
            };
            ui.painter()
                .with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0)))
                .text(
                    cell.left_center() + egui::vec2(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font,
                    *color,
                );
        }
        resp
    }
}

/// Footer content. The strip has a fixed height, so a result never pushes
/// the page up; a result shows as a soft pill.
pub struct Footer<'a> {
    pub link: Tone,
    pub link_text: &'a str,
    pub busy: bool,
    pub result: Option<(&'a str, Tone)>,
    pub right: &'a str,
}

pub fn footer(ui: &mut egui::Ui, f: &Footer) {
    let t = theme::of(ui);
    let small = |s: &str| egui::RichText::new(s).size(11.5).color(t.muted);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        status_dot(ui, f.link.dot(&t));
        ui.label(small(f.link_text));
        ui.add_space(8.0);
        if f.busy {
            ui.add(egui::Spinner::new().size(11.0).color(t.muted));
            ui.label(small("进行中"));
        } else {
            ui.label(small("空闲"));
        }
        trailing(ui, 20.0, |ui| {
            ui.label(small(f.right));
            if let Some((text, tone)) = f.result {
                ui.add_space(8.0);
                let shown: String = if text.chars().count() > 72 {
                    text.chars().take(71).chain(['…']).collect()
                } else {
                    text.to_owned()
                };
                badge(ui, &shown, tone).on_hover_text(text);
            }
        });
    });
}

/// Config column on the left and the work area on the right, both working on
/// `state`; below `stack_below` points of width the two stack.
pub fn work_columns<S, R>(
    ui: &mut egui::Ui,
    state: &mut S,
    left_w: f32,
    stack_below: f32,
    left: impl FnOnce(&mut egui::Ui, &mut S),
    right: impl FnOnce(&mut egui::Ui, &mut S) -> R,
) -> R {
    const GAP: f32 = 14.0;
    if ui.available_width() < stack_below {
        left(ui, state);
        ui.add_space(GAP);
        return right(ui, state);
    }
    ui.horizontal_top(|ui| {
        let h = ui.available_height();
        ui.allocate_ui_with_layout(
            egui::vec2(left_w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(left_w);
                left(ui, state);
            },
        );
        ui.add_space(GAP - ui.spacing().item_spacing.x);
        let w = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(w);
                right(ui, state)
            },
        )
        .inner
    })
    .inner
}

/// Square check box with its label; ink when on (Magpie switches are ink,
/// not accent).
pub fn check(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let t = theme::of(ui);
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), t.fg2);
    let size = egui::vec2(22.0 + galley.size().x, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let bx = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 7.0, rect.center().y),
        egui::vec2(14.0, 14.0),
    );
    let radius = egui::CornerRadius::same(4);
    let p = ui.painter();
    if *on {
        p.rect_filled(bx, radius, t.ink);
        let c = bx.center();
        let s = egui::Stroke::new(1.6, t.ink_fg);
        p.line_segment([c + egui::vec2(-3.5, 0.0), c + egui::vec2(-1.0, 2.5)], s);
        p.line_segment([c + egui::vec2(-1.0, 2.5), c + egui::vec2(3.5, -2.5)], s);
    } else {
        let edge = if resp.hovered() { t.muted } else { t.faint };
        p.rect_filled(bx, radius, t.card);
        p.rect_stroke(
            bx,
            radius,
            egui::Stroke::new(1.0, edge),
            egui::StrokeKind::Inside,
        );
    }
    p.galley(
        egui::pos2(bx.right() + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        t.fg2,
    );
    resp
}

/// Detail row: the key on the left, the value right-aligned in monospace.
pub fn kv(ui: &mut egui::Ui, key: &str, value: &str, color: egui::Color32, last: bool) {
    let t = theme::of(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        key,
        egui::FontId::proportional(12.0),
        t.muted,
    );
    ui.painter().text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        value,
        egui::FontId::monospace(11.5),
        color,
    );
    if !last {
        hairline(ui);
    }
}

/// BusStudio's step bar (配置与检查 → 执行任务 → 验证与报告). `current` is
/// marked; steps up to `reachable` can be clicked. Returns the clicked step.
pub fn steps(
    ui: &mut egui::Ui,
    labels: &[&str],
    current: usize,
    reachable: usize,
) -> Option<usize> {
    let t = theme::of(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
    let p = ui.painter().clone();
    let radius = egui::CornerRadius::same(10);
    p.rect_filled(rect, radius, t.card);
    p.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, t.line),
        egui::StrokeKind::Inside,
    );
    let w = rect.width() / labels.len().max(1) as f32;
    let mut out = None;
    for (i, label) in labels.iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            egui::pos2(rect.left() + i as f32 * w, rect.top()),
            egui::vec2(w, rect.height()),
        );
        let open = i <= reachable;
        let sense = if open {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let resp = ui.interact(cell, ui.make_persistent_id(("step", i)), sense);
        let on = i == current;
        if on {
            p.rect_filled(cell.shrink(3.0), egui::CornerRadius::same(8), t.sel);
        } else if open && resp.hovered() {
            p.rect_filled(cell.shrink(3.0), egui::CornerRadius::same(8), t.card2);
        }
        if i > 0 {
            p.vline(
                cell.left(),
                cell.top() + 10.0..=cell.bottom() - 10.0,
                egui::Stroke::new(1.0, t.line2),
            );
        }
        let color = if on {
            t.accent
        } else if open {
            t.fg
        } else {
            t.muted
        };
        let galley = p.layout_no_wrap((*label).to_owned(), egui::FontId::proportional(12.5), color);
        let x0 = cell.center().x - (28.0 + galley.size().x) / 2.0;
        let c = egui::pos2(x0 + 10.0, cell.center().y);
        p.circle_filled(c, 9.5, t.card);
        p.circle_stroke(
            c,
            9.5,
            egui::Stroke::new(1.0, if on { t.accent } else { t.line }),
        );
        p.text(
            c,
            egui::Align2::CENTER_CENTER,
            (i + 1).to_string(),
            egui::FontId::proportional(11.0),
            color,
        );
        p.galley(
            egui::pos2(x0 + 28.0, cell.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}

/// Thin accent progress bar, `frac` from 0 to 1.
pub fn progress(ui: &mut egui::Ui, frac: f32) {
    let t = theme::of(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::hover());
    let bar = egui::Rect::from_x_y_ranges(
        rect.x_range(),
        rect.center().y - 3.0..=rect.center().y + 3.0,
    );
    let radius = egui::CornerRadius::same(3);
    ui.painter().rect_filled(bar, radius, t.pill_hover);
    let f = frac.clamp(0.0, 1.0);
    if f > 0.0 {
        let done =
            egui::Rect::from_x_y_ranges(bar.left()..=bar.left() + f * bar.width(), bar.y_range());
        ui.painter().rect_filled(done, radius, t.accent);
    }
}

/// The work area with a fixed-width column on its right (CAN's 报文详情),
/// both working on `state`; below `stack_below` points of width the side
/// column goes under the main one.
pub fn with_side<S, R>(
    ui: &mut egui::Ui,
    state: &mut S,
    side_w: f32,
    stack_below: f32,
    main: impl FnOnce(&mut egui::Ui, &mut S) -> R,
    side: impl FnOnce(&mut egui::Ui, &mut S),
) -> R {
    const GAP: f32 = 14.0;
    if ui.available_width() < stack_below {
        let r = main(ui, state);
        ui.add_space(GAP);
        side(ui, state);
        return r;
    }
    ui.horizontal_top(|ui| {
        let h = ui.available_height();
        let w = ui.available_width() - side_w - GAP;
        let r = ui
            .allocate_ui_with_layout(
                egui::vec2(w, h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(w);
                    main(ui, state)
                },
            )
            .inner;
        ui.add_space(GAP - ui.spacing().item_spacing.x);
        ui.allocate_ui_with_layout(
            egui::vec2(side_w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(side_w);
                side(ui, state);
            },
        );
        r
    })
    .inner
}

/// Muted small text, the most common label on the local pages.
pub fn muted(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = theme::of(ui);
    ui.label(egui::RichText::new(text).size(11.5).color(t.muted))
}

/// Red line under a field or a card for input that does not parse.
pub fn error(ui: &mut egui::Ui, text: &str) {
    let t = theme::of(ui);
    ui.label(egui::RichText::new(text).size(11.5).color(t.red));
}

/// [`panel`] with a segmented control in place of the title, for a card
/// that switches between views (LIN's 帧监视 / 调度表).
pub fn tab_panel<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
    head: impl FnOnce(&mut egui::Ui),
    body: impl FnOnce(&mut egui::Ui),
) {
    let t = theme::of(ui);
    card_frame(&t, 0).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    seg(ui, value, options);
                    trailing(ui, 30.0, head);
                });
            });
        divider(ui, t.line);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                body(ui);
            });
    });
}
