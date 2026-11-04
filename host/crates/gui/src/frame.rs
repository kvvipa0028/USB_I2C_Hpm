//! What the CAN and LIN monitors share: the 报文详情 byte grid (click a
//! byte to see it in decimal and as bits) and the time column format.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui;

use crate::i2c::parse::clock;
use crate::theme;
use crate::widgets;

/// D0…D7 in rows of four; the picked byte is outlined in accent. Under the
/// grid, the picked byte in decimal and its eight bits, b7 first.
pub fn byte_grid(ui: &mut egui::Ui, data: &[u8], sel: &mut usize) {
    let t = theme::of(ui);
    widgets::muted(ui, "数据字节 / HEX");
    if data.is_empty() {
        ui.label(egui::RichText::new("无数据字节").size(12.0).color(t.faint));
        return;
    }
    if *sel >= data.len() {
        *sel = 0;
    }
    let w = ui.available_width();
    let cell_w = (w / 4.0).min(48.0);
    for (r, row) in data.chunks(4).enumerate() {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 44.0), egui::Sense::hover());
        for (c, b) in row.iter().enumerate() {
            let i = r * 4 + c;
            let col = egui::Rect::from_min_size(
                egui::pos2(rect.left() + c as f32 * cell_w, rect.top()),
                egui::vec2(cell_w, 44.0),
            );
            ui.painter().text(
                egui::pos2(col.center().x, col.top() + 7.0),
                egui::Align2::CENTER_CENTER,
                format!("D{i}"),
                egui::FontId::proportional(10.5),
                t.muted,
            );
            let cell = egui::Rect::from_center_size(
                egui::pos2(col.center().x, col.top() + 29.0),
                egui::vec2(cell_w - 12.0, 24.0),
            );
            let resp = ui.interact(
                cell,
                ui.make_persistent_id(("byte", i)),
                egui::Sense::click(),
            );
            let on = i == *sel;
            let radius = egui::CornerRadius::same(5);
            let fill = if on {
                t.accent_soft
            } else if resp.hovered() {
                t.pill_hover
            } else {
                t.pill
            };
            ui.painter().rect_filled(cell, radius, fill);
            if on {
                ui.painter().rect_stroke(
                    cell,
                    radius,
                    egui::Stroke::new(1.0, t.accent),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter().text(
                cell.center(),
                egui::Align2::CENTER_CENTER,
                format!("{b:02X}"),
                egui::FontId::monospace(11.5),
                if on { t.accent } else { t.fg2 },
            );
            if resp.clicked() {
                *sel = i;
            }
        }
    }
    let b = data[*sel];
    widgets::kv(ui, &format!("D{} 十进制", *sel), &b.to_string(), t.fg, true);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    let bw = (rect.width() / 8.0).min(24.0);
    for k in 0..8 {
        let bit = (b >> (7 - k)) & 1;
        let cell = egui::Rect::from_min_size(
            egui::pos2(rect.left() + k as f32 * bw, rect.top()),
            egui::vec2(bw - 3.0, 20.0),
        );
        let (fg, bg) = if bit == 1 {
            (t.accent, t.accent_soft)
        } else {
            (t.fg2, t.card2)
        };
        ui.painter()
            .rect_filled(cell, egui::CornerRadius::same(4), bg);
        ui.painter().text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            bit.to_string(),
            egui::FontId::monospace(10.5),
            fg,
        );
    }
}

/// Row time: since the window opened, or the wall clock (UTC).
pub fn time_text(at: Duration, opened: SystemTime, wall: bool) -> String {
    if !wall {
        return clock(at);
    }
    let ms = (opened + at)
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (ms / 3_600_000) % 24,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// Gap to the previous row, `10 ms`, or `—` for the first.
pub fn gap_text(gap: Option<Duration>) -> String {
    gap.map_or("—".to_owned(), |g| format!("{} ms", g.as_millis()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_well() {
        let at = Duration::from_millis(1_234);
        assert_eq!(time_text(at, UNIX_EPOCH, false), "00:01.234");
        let opened = UNIX_EPOCH + Duration::from_secs(3600 * 25 + 61);
        assert_eq!(time_text(at, opened, true), "01:01:02.234");
        assert_eq!(gap_text(Some(Duration::from_millis(10))), "10 ms");
        assert_eq!(gap_text(None), "—");
    }
}
