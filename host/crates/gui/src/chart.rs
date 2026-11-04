//! Line chart for the sampling pages (AD 记录, ADS1015): grid with four
//! value ticks, the trace in accent with a soft fill under it, open dots
//! when there are few points, and a read-out of the nearest point on hover.

use eframe::egui;

use crate::theme;

pub struct Chart<'a> {
    /// (x, y) in order of x.
    pub points: &'a [(f64, f64)],
    pub y_name: &'a str,
    pub x_name: &'a str,
    pub height: f32,
    /// Decimals on the value axis.
    pub y_places: usize,
}

/// Value range with a little room above and below; a flat trace gets a
/// small band so it does not sit on an edge.
pub fn y_range(points: &[(f64, f64)]) -> (f64, f64) {
    let (mut lo, mut hi) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    if !lo.is_finite() {
        return (0.0, 1.0);
    }
    let pad = ((hi - lo) * 0.12).max(0.004);
    lo -= pad;
    hi += pad;
    (lo, hi)
}

impl Chart<'_> {
    pub fn show(&self, ui: &mut egui::Ui) {
        let t = theme::of(ui);
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), self.height),
            egui::Sense::hover(),
        );
        let p = ui.painter_at(rect);
        let small = egui::FontId::proportional(11.0);
        let mono = egui::FontId::monospace(10.5);
        let plot = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 48.0, rect.top() + 24.0),
            egui::pos2(rect.right() - 14.0, rect.bottom() - 40.0),
        );
        let thin = egui::Stroke::new(1.0, t.line2);
        p.text(
            egui::pos2(plot.left(), rect.top() + 9.0),
            egui::Align2::LEFT_CENTER,
            self.y_name,
            small.clone(),
            t.muted,
        );
        p.text(
            egui::pos2(plot.right(), rect.bottom() - 9.0),
            egui::Align2::RIGHT_CENTER,
            self.x_name,
            small.clone(),
            t.muted,
        );
        let (lo, hi) = y_range(self.points);
        for i in 0..4 {
            let f = i as f32 / 3.0;
            let y = plot.bottom() - f * plot.height();
            p.hline(plot.x_range(), y, thin);
            let v = lo + f64::from(f) * (hi - lo);
            p.text(
                egui::pos2(plot.left() - 8.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{v:.*}", self.y_places),
                mono.clone(),
                t.muted,
            );
        }
        let Some((&first, &last)) = self.points.first().zip(self.points.last()) else {
            p.text(
                plot.center(),
                egui::Align2::CENTER_CENTER,
                "暂无样本",
                small,
                t.muted,
            );
            return;
        };
        let (x0, x1) = (first.0, last.0.max(first.0 + 1e-9));
        let to_screen = |(x, y): (f64, f64)| {
            egui::pos2(
                plot.left() + ((x - x0) / (x1 - x0)) as f32 * plot.width(),
                plot.bottom() - ((y - lo) / (hi - lo)) as f32 * plot.height(),
            )
        };
        for i in 0..4 {
            let f = i as f64 / 3.0;
            let x = plot.left() + f as f32 * plot.width();
            p.vline(x, plot.y_range(), thin);
            p.text(
                egui::pos2(x, plot.bottom() + 12.0),
                egui::Align2::CENTER_CENTER,
                crate::spi::trim(x0 + f * (x1 - x0), 1),
                mono.clone(),
                t.muted,
            );
        }
        let pts: Vec<egui::Pos2> = self.points.iter().map(|&q| to_screen(q)).collect();
        // soft fill: one quad per segment down to the plot floor
        let fill = t.accent.gamma_multiply(0.10);
        let mut mesh = egui::Mesh::default();
        for w in pts.windows(2) {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(w[0], fill);
            mesh.colored_vertex(w[1], fill);
            mesh.colored_vertex(egui::pos2(w[1].x, plot.bottom()), fill);
            mesh.colored_vertex(egui::pos2(w[0].x, plot.bottom()), fill);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        p.add(egui::Shape::mesh(mesh));
        p.add(egui::Shape::line(
            pts.clone(),
            egui::Stroke::new(1.6, t.accent),
        ));
        if pts.len() <= 40 {
            for q in &pts {
                p.circle(*q, 3.0, t.card, egui::Stroke::new(1.2, t.accent));
            }
        }
        if let Some(hover) = resp.hover_pos() {
            let near = pts
                .iter()
                .enumerate()
                .min_by(|a, b| (a.1.x - hover.x).abs().total_cmp(&(b.1.x - hover.x).abs()))
                .map(|(i, _)| i);
            if let Some(i) = near {
                let q = pts[i];
                let (x, y) = self.points[i];
                p.vline(q.x, plot.y_range(), egui::Stroke::new(1.0, t.line));
                p.circle_filled(q, 3.5, t.accent);
                let text = format!("{}  ·  {y:.*}", crate::spi::trim(x, 3), self.y_places);
                let galley = p.layout_no_wrap(text, mono, t.fg);
                let at = if q.x + galley.size().x + 16.0 > plot.right() {
                    q.x - galley.size().x - 14.0
                } else {
                    q.x + 8.0
                };
                let bg = egui::Rect::from_min_size(
                    egui::pos2(at, plot.top() + 2.0),
                    galley.size() + egui::vec2(8.0, 4.0),
                );
                p.rect_filled(bg, egui::CornerRadius::same(4), t.card);
                p.rect_stroke(
                    bg,
                    egui::CornerRadius::same(4),
                    egui::Stroke::new(1.0, t.line),
                    egui::StrokeKind::Inside,
                );
                p.galley(bg.min + egui::vec2(4.0, 2.0), galley, t.fg);
            }
        }
    }
}

/// Min, max, and last value of a trace.
pub fn stats(points: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    let last = points.last()?.1;
    let (lo, hi) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    Some((lo, hi, last))
}

/// BusStudio's config strip over a chart: label / value pairs split by
/// hairlines, and 详细配置 at the end to open the full settings.
pub fn config_strip(ui: &mut egui::Ui, items: &[(&str, String)], open: &mut bool) {
    let t = theme::of(ui);
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.line))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, 16.0), egui::Sense::hover());
                        ui.painter().vline(
                            r.center().x,
                            r.y_range(),
                            egui::Stroke::new(1.0, t.line),
                        );
                    }
                    ui.label(egui::RichText::new(*k).size(11.5).color(t.muted));
                    ui.label(egui::RichText::new(v).monospace().color(t.fg));
                }
                crate::widgets::trailing(ui, 28.0, |ui| {
                    let text = if *open {
                        "收起配置"
                    } else {
                        "详细配置"
                    };
                    if crate::widgets::action_button(ui, true, text).clicked() {
                        *open = !*open;
                    }
                });
            });
        });
}

/// Under the chart: a short accent stroke, the series, its latest value
/// large, the raw code, and a tag.
pub fn legend(ui: &mut egui::Ui, series: &str, value: &str, code: &str, tag: &str) {
    let t = theme::of(ui);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 12.0), egui::Sense::hover());
        ui.painter()
            .hline(r.x_range(), r.center().y, egui::Stroke::new(2.5, t.accent));
        ui.label(egui::RichText::new(series).size(12.0).color(t.fg2));
        ui.label(
            egui::RichText::new(value)
                .monospace()
                .size(18.0)
                .color(t.fg),
        );
        ui.add_space(8.0);
        ui.label(egui::RichText::new("原始码值").size(11.5).color(t.muted));
        ui.label(egui::RichText::new(code).monospace().size(15.0).color(t.fg));
        ui.add_space(4.0);
        crate::widgets::badge(ui, tag, crate::widgets::Tone::Neutral);
    });
}

/// Points for the chart with x in ms when the span is under a second,
/// else in s; returns the unit for the axis.
pub fn scale_x(points: &[(f64, f64)]) -> (Vec<(f64, f64)>, &'static str) {
    let span = match (points.first(), points.last()) {
        (Some(a), Some(b)) => b.0 - a.0,
        _ => 0.0,
    };
    if span < 1.0 {
        (
            points.iter().map(|&(x, y)| (x * 1000.0, y)).collect(),
            "时间 / ms",
        )
    } else {
        (points.to_vec(), "时间 / s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_leave_room() {
        let (lo, hi) = y_range(&[(0.0, 1.0), (1.0, 2.0)]);
        assert!(lo < 1.0 && hi > 2.0);
        let (lo, hi) = y_range(&[(0.0, 2.4)]);
        assert!(lo < 2.4 && hi > 2.4);
        assert_eq!(y_range(&[]), (0.0, 1.0));
        assert_eq!(
            stats(&[(0.0, 2.0), (1.0, 1.0), (2.0, 1.5)]),
            Some((1.0, 2.0, 1.5))
        );
        assert_eq!(stats(&[]), None);
    }
}
