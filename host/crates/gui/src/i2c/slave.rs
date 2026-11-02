//! 从机 tab: role and address, and the 256-byte register table as a hex grid.
//! Nothing here reaches the device. The bridge has no slave commands yet
//! (set role, set address, read/write the table), so the table is a local
//! draft that starts all zero, as slave_proto's table does after reset.

use eframe::egui;

use super::parse::{parse_addr, parse_bytes, to_hex};
use super::{FIXED_PORT, NO_CMD};
use crate::theme;
use crate::widgets::{self, Tone};

const SIZE: usize = 256;
const OFF_W: f32 = 40.0;
const HEAD_H: f32 = 22.0;
const ROW_H: f32 = 22.0;
const CHAR_W: f32 = 7.5;

pub struct Slave {
    addr: String,
    table: [u8; SIZE],
    /// What the table started from; cells that differ are marked as edited.
    base: [u8; SIZE],
    cursor: usize,
    /// High nibble typed into the cursor cell, waiting for the low one.
    nibble: Option<u8>,
    value: String,
    /// The cell whose 应用 failed; cleared when the cursor leaves it or the
    /// input changes.
    value_err: Option<usize>,
}

impl Default for Slave {
    fn default() -> Self {
        Self {
            addr: "0x50".into(),
            table: [0; SIZE],
            base: [0; SIZE],
            cursor: 0,
            nibble: None,
            value: String::new(),
            value_err: None,
        }
    }
}

impl Slave {
    fn edited(&self) -> usize {
        self.table
            .iter()
            .zip(&self.base)
            .filter(|(a, b)| a != b)
            .count()
    }

    fn move_by(&mut self, delta: isize) {
        self.nibble = None;
        self.cursor = (self.cursor as isize + delta).clamp(0, SIZE as isize - 1) as usize;
    }

    /// Two hex digits make a byte; then the cursor moves on.
    fn type_digit(&mut self, d: u8) {
        match self.nibble.take() {
            None => self.nibble = Some(d),
            Some(hi) => {
                self.table[self.cursor] = (hi << 4) | d;
                self.cursor = (self.cursor + 1).min(SIZE - 1);
            }
        }
    }

    /// Replace the whole table; a half-typed byte is dropped.
    fn fill(&mut self, table: [u8; SIZE]) {
        self.table = table;
        self.nibble = None;
    }

    fn as_text(&self) -> String {
        let mut out = String::new();
        for row in self.table.chunks(16) {
            out.push_str(&to_hex(row));
            out.push('\n');
        }
        out
    }
}

pub fn show(ui: &mut egui::Ui, s: &mut Slave) {
    widgets::work_columns(ui, s, 264.0, 820.0, config, table);
}

fn config(ui: &mut egui::Ui, s: &mut Slave) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "从机配置",
        |ui| {
            widgets::badge(ui, "本地草稿", Tone::Amber);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            widgets::field_row(ui, "端口", false, |ui| {
                widgets::fixed_value(ui, "I2C0", FIXED_PORT);
            });
            widgets::field_row(ui, "角色", false, |ui| {
                widgets::fixed_value(
                    ui,
                    "从机（未下发）",
                    "端口按主机/从机切换要等固件支持；当前固件只做主机。",
                );
            });
            widgets::field_row(ui, "从机地址", false, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut s.addr)
                        .hint_text("0x50")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                );
            });
            widgets::field_row(ui, "寄存器表", false, |ui| {
                ui.label(egui::RichText::new("256 B · 1 字节偏移").size(12.0));
            });
            widgets::field_row(ui, "访问方式", true, |ui| {
                ui.label(egui::RichText::new("EEPROM 式，同 24C02").size(12.0));
            });
            if let Err(e) = parse_addr(&s.addr) {
                if !s.addr.trim().is_empty() {
                    ui.label(egui::RichText::new(e).size(11.5).color(t.red));
                }
            }
        },
    );
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(
            "主机写入的第 1 个字节是偏移，之后逐字节写入；读取从当前偏移开始，到表尾回到 0。",
        )
        .size(11.5)
        .color(t.muted),
    );
    ui.add_space(10.0);
    widgets::panel(
        ui,
        "与设备同步",
        |ui| {
            widgets::badge(ui, "待协议", Tone::Amber);
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::action_button(ui, false, "下发到设备").on_disabled_hover_text(NO_CMD);
                widgets::action_button(ui, false, "从设备读回").on_disabled_hover_text(NO_CMD);
            });
            ui.label(
                egui::RichText::new(
                    "设置角色、地址和读写寄存器表都需要新的 USB 命令，当前固件还没有。表格只保存在本窗口，关闭即丢失。",
                )
                .size(11.5)
                .color(t.muted),
            );
        },
    );
}

fn table(ui: &mut egui::Ui, s: &mut Slave) {
    let t = theme::of(ui);
    let edited = s.edited();
    widgets::panel(
        ui,
        "寄存器表",
        |ui| {
            let text = if edited > 0 {
                format!("已改 {edited} 字节 · 未下发")
            } else {
                "未改动".to_owned()
            };
            ui.label(egui::RichText::new(text).size(11.0).color(t.muted));
        },
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if widgets::quiet_button(ui, true, "全部 00").clicked() {
                    s.fill([0; SIZE]);
                }
                if widgets::quiet_button(ui, true, "全部 FF").clicked() {
                    s.fill([0xff; SIZE]);
                }
                if widgets::quiet_button(ui, true, "递增 00–FF").clicked() {
                    s.fill(std::array::from_fn(|i| i as u8));
                }
                if widgets::quiet_button(ui, edited > 0, "还原").clicked() {
                    s.fill(s.base);
                }
                if widgets::quiet_button(ui, true, "复制 HEX").clicked() {
                    ui.ctx().copy_text(s.as_text());
                }
            });
            grid(ui, s);
            widgets::hairline(ui);
            inspector(ui, s);
        },
    );
}

fn grid(ui: &mut egui::Ui, s: &mut Slave) {
    let t = theme::of(ui);
    let avail = ui.available_width();
    let ascii = avail >= 600.0;
    let ascii_w = if ascii { 16.0 * CHAR_W + 22.0 } else { 0.0 };
    let cell_w = ((avail - OFF_W - ascii_w) / 16.0).clamp(22.0, 32.0);
    let size = egui::vec2(OFF_W + 16.0 * cell_w + ascii_w, HEAD_H + 16.0 * ROW_H);

    egui::ScrollArea::horizontal()
        .id_salt("slave_grid_scroll")
        .show(ui, |ui| {
            let id = ui.make_persistent_id("slave_grid");
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let resp = ui.interact(rect, id, egui::Sense::click());
            let ascii_x = rect.left() + OFF_W + 16.0 * cell_w + 22.0;

            let hit = |pos: egui::Pos2| -> Option<usize> {
                let y = pos.y - rect.top() - HEAD_H;
                if y < 0.0 {
                    return None;
                }
                let row = (y / ROW_H) as usize;
                let x = pos.x - rect.left() - OFF_W;
                let col = if x >= 0.0 && x < 16.0 * cell_w {
                    (x / cell_w) as usize
                } else if ascii && pos.x >= ascii_x && pos.x < ascii_x + 16.0 * CHAR_W {
                    ((pos.x - ascii_x) / CHAR_W) as usize
                } else {
                    return None;
                };
                (row < 16 && col < 16).then_some(row * 16 + col)
            };

            if resp.clicked() {
                if let Some(i) = resp.interact_pointer_pos().and_then(hit) {
                    s.cursor = i;
                    s.nibble = None;
                }
                resp.request_focus();
            }
            let focused = resp.has_focus();
            if focused {
                keys(ui, id, s);
            }
            let hover = resp.hover_pos().and_then(hit);

            let p = ui.painter_at(rect);
            let mono = egui::FontId::monospace(11.5);
            let small = egui::FontId::monospace(10.5);
            for c in 0..16 {
                let x = rect.left() + OFF_W + (c as f32 + 0.5) * cell_w;
                p.text(
                    egui::pos2(x, rect.top() + HEAD_H / 2.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{c:X}"),
                    small.clone(),
                    t.muted,
                );
            }
            if ascii {
                p.text(
                    egui::pos2(ascii_x, rect.top() + HEAD_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    "ASCII",
                    egui::FontId::proportional(10.5),
                    t.muted,
                );
            }
            p.hline(
                rect.x_range(),
                rect.top() + HEAD_H - 0.5,
                egui::Stroke::new(1.0, t.line2),
            );

            let radius = egui::CornerRadius::same(4);
            for row in 0..16 {
                let y = rect.top() + HEAD_H + row as f32 * ROW_H;
                p.text(
                    egui::pos2(rect.left() + 2.0, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{:02X}", row * 16),
                    small.clone(),
                    t.muted,
                );
                for col in 0..16 {
                    let i = row * 16 + col;
                    let v = s.table[i];
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(rect.left() + OFF_W + col as f32 * cell_w, y),
                        egui::vec2(cell_w, ROW_H),
                    )
                    .shrink(1.0);
                    let changed = v != s.base[i];
                    let here = i == s.cursor;
                    if here {
                        p.rect_filled(cell, radius, t.sel);
                        let stroke = if focused {
                            egui::Stroke::new(1.5, t.accent)
                        } else {
                            egui::Stroke::new(1.0, t.line)
                        };
                        p.rect_stroke(cell, radius, stroke, egui::StrokeKind::Inside);
                    } else if changed {
                        p.rect_filled(cell, radius, t.accent_soft);
                    } else if hover == Some(i) {
                        p.rect_filled(cell, radius, t.pill);
                    }
                    let pending = if here && focused { s.nibble } else { None };
                    let (text, color) = match pending {
                        Some(hi) => (format!("{hi:X}_"), t.accent),
                        None if changed => (format!("{v:02X}"), t.accent),
                        None if v == 0 => ("00".to_owned(), t.faint),
                        None => (format!("{v:02X}"), t.fg),
                    };
                    p.text(
                        cell.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        mono.clone(),
                        color,
                    );

                    if ascii {
                        let cx = ascii_x + (col as f32 + 0.5) * CHAR_W;
                        if here {
                            let r = egui::Rect::from_center_size(
                                egui::pos2(cx, y + ROW_H / 2.0),
                                egui::vec2(CHAR_W + 1.0, ROW_H - 4.0),
                            );
                            p.rect_filled(r, egui::CornerRadius::same(2), t.sel);
                        }
                        p.text(
                            egui::pos2(cx, y + ROW_H / 2.0),
                            egui::Align2::CENTER_CENTER,
                            printable(v).unwrap_or('·'),
                            mono.clone(),
                            if changed { t.accent } else { t.muted },
                        );
                    }
                }
            }
        });
}

/// Hex digits type into the cursor cell; arrows move; Esc drops a half byte.
fn keys(ui: &mut egui::Ui, id: egui::Id, s: &mut Slave) {
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: false,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        )
    });
    let events = ui.input(|i| i.events.clone());
    for ev in events {
        match ev {
            egui::Event::Text(text) => {
                for d in text.chars().filter_map(|c| c.to_digit(16)) {
                    s.type_digit(d as u8);
                }
            }
            egui::Event::Key {
                key, pressed: true, ..
            } => match key {
                egui::Key::ArrowLeft => s.move_by(-1),
                egui::Key::ArrowRight => s.move_by(1),
                egui::Key::ArrowUp => s.move_by(-16),
                egui::Key::ArrowDown => s.move_by(16),
                egui::Key::Escape | egui::Key::Backspace => s.nibble = None,
                _ => {}
            },
            _ => {}
        }
    }
}

fn inspector(ui: &mut egui::Ui, s: &mut Slave) {
    let t = theme::of(ui);
    let i = s.cursor;
    let v = s.table[i];
    if s.value_err != Some(i) {
        s.value_err = None;
    }
    let key = |ui: &mut egui::Ui, k: &str| {
        ui.label(egui::RichText::new(k).size(11.5).color(t.muted));
    };
    let val = |ui: &mut egui::Ui, v: String| {
        ui.label(egui::RichText::new(v).monospace());
    };
    ui.horizontal_wrapped(|ui| {
        key(ui, "偏移");
        val(ui, format!("0x{i:02X}"));
        ui.add_space(8.0);
        key(ui, "值");
        val(ui, format!("0x{v:02X}"));
        ui.add_space(8.0);
        key(ui, "十进制");
        val(ui, v.to_string());
        ui.add_space(8.0);
        key(ui, "二进制");
        val(ui, format!("{:04b} {:04b}", v >> 4, v & 0xf));
        ui.add_space(8.0);
        key(ui, "ASCII");
        val(ui, printable(v).map_or("—".to_owned(), |c| c.to_string()));
    });
    ui.horizontal(|ui| {
        key(ui, "改为");
        let id = ui.make_persistent_id("slave_value");
        // follow the cursor cell unless the user is typing here
        if !ui.memory(|m| m.has_focus(id)) {
            s.value = format!("{v:02X}");
        }
        let r = ui.add(
            egui::TextEdit::singleline(&mut s.value)
                .id(id)
                .font(egui::TextStyle::Monospace)
                .char_limit(4)
                .desired_width(44.0),
        );
        if r.changed() {
            s.value_err = None;
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if widgets::action_button(ui, true, "应用").clicked() || enter {
            match parse_bytes(&s.value).ok().filter(|b| b.len() == 1) {
                Some(b) => {
                    s.table[i] = b[0];
                    s.nibble = None;
                    s.value_err = None;
                }
                None => s.value_err = Some(i),
            }
        }
        if s.value_err.is_some() {
            ui.label(
                egui::RichText::new("填一个字节，如 3C")
                    .size(11.5)
                    .color(t.red),
            );
        } else {
            ui.label(
                egui::RichText::new("点格子选中，直接键入两位十六进制，方向键移动。")
                    .size(11.5)
                    .color(t.muted),
            );
        }
    });
}

fn printable(v: u8) -> Option<char> {
    (0x20..0x7f).contains(&v).then_some(v as char)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_two_digits_writes_and_advances() {
        let mut s = Slave::default();
        s.type_digit(0x3);
        assert_eq!(s.table[0], 0);
        s.type_digit(0xc);
        assert_eq!((s.table[0], s.cursor, s.edited()), (0x3c, 1, 1));
        s.move_by(-16);
        assert_eq!(s.cursor, 0);
        s.cursor = 255;
        s.type_digit(1);
        s.type_digit(2);
        assert_eq!((s.table[255], s.cursor), (0x12, 255));
        assert_eq!(s.as_text().lines().count(), 16);
        assert_eq!(printable(b'<'), Some('<'));
        assert_eq!(printable(0x7f), None);
    }

    #[test]
    fn fill_drops_half_typed_byte() {
        let mut s = Slave::default();
        s.type_digit(0xa);
        s.fill([0xff; SIZE]);
        s.type_digit(0x1);
        assert_eq!((s.table[0], s.nibble), (0xff, Some(0x1)));
    }
}
