//! Per-port slave-table drafts with explicit snapshots and CAS commits.
use eframe::egui;
use usb_i2c::control::{ControlCapabilities, ControlStatus, PortRole, PortState, TableSnapshot};
use usb_i2c::device::DeviceError;

use super::parse::{parse_bytes, to_hex};
use super::Ctx;
use crate::local::{parse_hex, parse_num};
use crate::theme;
use crate::widgets::{self, Tone};
use crate::worker::{ControlReply, ControlRequest};

const SIZE: usize = 256;
const OFF_W: f32 = 40.0;
const HEAD_H: f32 = 22.0;
const ROW_H: f32 = 22.0;
const CHAR_W: f32 = 7.5;

pub struct Slave {
    editors: [Editor; 2],
}
impl Default for Slave {
    fn default() -> Self {
        Self {
            editors: std::array::from_fn(|_| Editor::default()),
        }
    }
}

struct Editor {
    table: [u8; SIZE],
    base: [u8; SIZE],
    snapshot: Option<TableSnapshot>,
    offset: String,
    len: String,
    cursor: usize,
    nibble: Option<u8>,
    value: String,
    value_err: Option<usize>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            table: [0; SIZE],
            base: [0; SIZE],
            snapshot: None,
            offset: "0x00".into(),
            len: "256".into(),
            cursor: 0,
            nibble: None,
            value: String::new(),
            value_err: None,
        }
    }
}

impl Editor {
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
    fn type_digit(&mut self, digit: u8) {
        match self.nibble.take() {
            None => self.nibble = Some(digit),
            Some(hi) => {
                self.table[self.cursor] = (hi << 4) | digit;
                self.cursor = (self.cursor + 1).min(SIZE - 1);
            }
        }
    }
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
    fn range(&self) -> Result<(u16, u16), String> {
        let offset = parse_hex(&self.offset, 255, "表偏移")? as u16;
        let len = parse_num(&self.len, 1, 256, "片段长度")? as u16;
        if u32::from(offset) + u32::from(len) > 256 {
            return Err("PC 表片段不能越过 256 B 边界，不会回绕".into());
        }
        Ok((offset, len))
    }
    fn write_request(&self, port: u8) -> Result<ControlRequest, String> {
        let (offset, len) = self.range()?;
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or("先从设备读回本片段，取得同步 generation")?;
        let end = usize::from(offset) + usize::from(len);
        if offset < snapshot.offset || end > usize::from(snapshot.offset) + snapshot.data.len() {
            return Err("下发片段须在最近一次读回或提交的范围内；先读取所选片段".into());
        }
        Ok(ControlRequest::TableWrite {
            port,
            offset,
            data: self.table[usize::from(offset)..end].to_vec(),
            expected: Some(snapshot.generation),
        })
    }
    fn readback(&mut self, snapshot: &TableSnapshot) {
        let start = usize::from(snapshot.offset);
        let end = start + snapshot.data.len();
        if end > SIZE {
            return;
        }
        self.table[start..end].copy_from_slice(&snapshot.data);
        self.base[start..end].copy_from_slice(&snapshot.data);
        self.snapshot = Some(snapshot.clone());
        self.nibble = None;
    }
}

impl Slave {
    pub(super) fn disconnected(&mut self) {
        for editor in &mut self.editors {
            editor.snapshot = None;
            editor.nibble = None;
        }
    }
    pub(super) fn on_control(
        &mut self,
        request: &ControlRequest,
        result: &Result<ControlReply, DeviceError>,
    ) {
        let Some(editor) = self.editors.get_mut(usize::from(request.port())) else {
            return;
        };
        match (request, result) {
            (_, Err(error))
                if error.session_invalidated
                    || matches!(
                        error.status,
                        Some(ControlStatus::Conflict | ControlStatus::WrongRole)
                    ) =>
            {
                editor.snapshot = None
            }
            (ControlRequest::TableRead { .. }, Ok(ControlReply::Table(snapshot))) => {
                editor.readback(snapshot)
            }
            (
                ControlRequest::TableWrite { offset, data, .. },
                Ok(ControlReply::Committed(commit)),
            ) => {
                let end = usize::from(*offset) + data.len();
                if end <= SIZE {
                    editor.base[usize::from(*offset)..end].copy_from_slice(data);
                    editor.snapshot = Some(TableSnapshot {
                        offset: *offset,
                        data: data.clone(),
                        generation: commit.generation,
                    });
                }
            }
            _ => {}
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    slave: &mut Slave,
    port: u8,
    caps: &ControlCapabilities,
    state: Option<&PortState>,
    cx: Ctx,
) -> Option<ControlRequest> {
    let editor = &mut slave.editors[usize::from(port)];
    let mut action = None;
    widgets::work_columns(
        ui,
        editor,
        264.0,
        820.0,
        |ui, editor| {
            action = config(ui, editor, port, caps, state, cx);
        },
        |ui, editor| {
            let address = state.map_or("未确认".into(), |s| {
                format!("0x{:02X}", s.config.slave_address)
            });
            widgets::summary_strip(
                ui,
                &[
                    ("端口", format!("CH{port}")),
                    ("实际地址", address),
                    ("草稿差异", format!("{} B", editor.edited())),
                ],
                "本地草稿；仅显式下发才写设备",
            );
            ui.add_space(12.0);
            ui.add_enabled_ui(!cx.busy, |ui| table(ui, editor));
        },
    );
    action
}

fn config(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    port: u8,
    caps: &ControlCapabilities,
    state: Option<&PortState>,
    cx: Ctx,
) -> Option<ControlRequest> {
    let mut action = None;
    let slave_role = state.is_some_and(|s| s.config.role == PortRole::Slave);
    widgets::panel(
        ui,
        "从机表同步",
        |ui| {
            widgets::badge(ui, &format!("CH{port}"), Tone::Neutral);
        },
        |ui| {
            widgets::note(ui,"角色、地址和超时在上方配置。此处编辑的是设备从机表，与主机页访问的外部 EEPROM 分开。");
            if !slave_role {
                widgets::note(ui, "先把所选端口配置为从机并读回确认，才能访问设备表。");
            }
            if !caps.supports_table {
                widgets::note(ui, "当前设备不支持 PC 从机表访问。");
            }
            ui.add_enabled_ui(!cx.busy, |ui| {
                widgets::field_row(ui, "片段偏移", false, |ui| {
                    widgets::unit_edit(ui, &mut editor.offset, "0x00", "");
                });
                widgets::field_row(ui, "片段长度", false, |ui| {
                    widgets::unit_edit(ui, &mut editor.len, "256", "B");
                });
            });
            match &editor.snapshot {
                Some(snapshot) => {
                    ui.label(format!(
                        "已确认快照：0x{:02X} 起 {} B，generation {}",
                        snapshot.offset,
                        snapshot.data.len(),
                        snapshot.generation
                    ));
                }
                None => widgets::note(ui, "尚无可用设备快照；草稿数值不代表设备内容。"),
            }
            let range = editor.range();
            if let Err(error) = &range {
                widgets::error(ui, error);
            }
            let write = editor.write_request(port);
            ui.horizontal_wrapped(|ui| {
                let on = cx.connected && !cx.busy && slave_role && caps.supports_table;
                if widgets::action_button(ui, on && range.is_ok(), "从设备读回片段").clicked()
                {
                    let (offset, len) = range.as_ref().copied().unwrap();
                    action = Some(ControlRequest::TableRead { port, offset, len });
                }
                if widgets::ink_button(
                    ui,
                    on && caps.supports_table_cas && write.is_ok(),
                    "下发片段 (CAS)",
                )
                .clicked()
                {
                    action = write.clone().ok();
                }
            });
            if let Err(error) = &write {
                widgets::muted(ui, error);
            }
            widgets::note(
                ui,
                "读回会替换所选草稿片段。CAS 冲突后保留草稿，需要重新读回；不会自动覆盖或重发。",
            );
        },
    );
    ui.add_space(8.0);
    widgets::note(ui,"外部 I2C 的第 1 个写入字节是偏移，随后字节写表并在 0xFF 回绕。PC 片段不回绕，也不改变外部 I2C 当前偏移。");
    widgets::note(
        ui,
        "PC 读回是一个提交点的快照；外部连续读可能跨 PC 提交，不承诺整笔单一版本。",
    );
    action
}

fn table(ui: &mut egui::Ui, s: &mut Editor) {
    let t = theme::of(ui);
    let edited = s.edited();
    widgets::panel(
        ui,
        "寄存器表",
        |ui| {
            let text = if edited > 0 {
                format!("已改 {edited} 字节")
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
            widgets::table_foot(ui, "256 B · 每行 16 B · 改过的格子标色", "0x00 – 0xFF");
            widgets::hairline(ui);
            inspector(ui, s);
        },
    );
}

fn grid(ui: &mut egui::Ui, s: &mut Editor) {
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
fn keys(ui: &mut egui::Ui, id: egui::Id, s: &mut Editor) {
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

fn inspector(ui: &mut egui::Ui, s: &mut Editor) {
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
        let mut s = Editor::default();
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
        let mut s = Editor::default();
        s.type_digit(0xa);
        s.fill([0xff; SIZE]);
        s.type_digit(0x1);
        assert_eq!((s.table[0], s.nibble), (0xff, Some(0x1)));
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use usb_i2c::control::TableCommit;

    #[test]
    fn port_snapshots_drafts_and_generation_are_independent() {
        let mut slave = Slave::default();
        assert!(slave.editors[0].write_request(0).is_err());
        let snapshot = TableSnapshot {
            offset: 0,
            data: vec![0x11; 256],
            generation: u64::from(u32::MAX) + 5,
        };
        slave.on_control(
            &ControlRequest::TableRead {
                port: 0,
                offset: 0,
                len: 256,
            },
            &Ok(ControlReply::Table(snapshot.clone())),
        );
        assert_eq!(slave.editors[0].table, [0x11; 256]);
        assert_eq!(slave.editors[1].table, [0; 256]);
        slave.editors[0].offset = "0xFF".into();
        slave.editors[0].len = "1".into();
        slave.editors[0].table[255] = 0x99;
        let request = slave.editors[0].write_request(0).unwrap();
        match &request {
            ControlRequest::TableWrite {
                port,
                offset,
                data,
                expected,
            } => {
                assert_eq!(
                    (*port, *offset, data.as_slice(), *expected),
                    (0, 255, &[0x99][..], Some(snapshot.generation))
                );
            }
            _ => panic!(),
        }
        slave.on_control(
            &request,
            &Ok(ControlReply::Committed(TableCommit {
                offset: 255,
                len: 1,
                generation: snapshot.generation + 1,
            })),
        );
        assert_eq!(slave.editors[0].base[255], 0x99);
        assert_eq!(
            slave.editors[0].snapshot.as_ref().unwrap().generation,
            snapshot.generation + 1
        );
        assert!(slave.editors[1].snapshot.is_none());
        slave.disconnected();
        assert!(slave.editors[0].snapshot.is_none());
        assert_eq!(slave.editors[0].table[255], 0x99);
        assert!(slave.editors[0].write_request(0).is_err());
    }

    #[test]
    fn conflict_retains_draft_without_replay_and_partial_snapshot_bounds_writes() {
        let mut slave = Slave::default();
        let snapshot = TableSnapshot {
            offset: 254,
            data: vec![1, 2],
            generation: 8,
        };
        slave.editors[0].readback(&snapshot);
        slave.editors[0].offset = "0xFE".into();
        slave.editors[0].len = "2".into();
        slave.editors[0].table[254] = 9;
        let request = slave.editors[0].write_request(0).unwrap();
        let error = DeviceError {
            message: "CAS conflict".into(),
            session_invalidated: false,
            result_unknown: false,
            status: Some(ControlStatus::Conflict),
            hpm_stat: Some(0),
        };
        slave.on_control(&request, &Err(error));
        assert_eq!(slave.editors[0].table[254], 9);
        assert!(slave.editors[0].snapshot.is_none());
        assert!(slave.editors[0].write_request(0).is_err());
        slave.editors[0].readback(&snapshot);
        for (offset, len) in [("0xFF", "2"), ("0x00", "256"), ("0x100", "1")] {
            slave.editors[0].offset = offset.into();
            slave.editors[0].len = len.into();
            assert!(slave.editors[0].write_request(0).is_err());
        }
    }
}
