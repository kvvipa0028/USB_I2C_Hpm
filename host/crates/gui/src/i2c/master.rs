//! 主机 tab. Config on the left (port and rate are fixed by the firmware;
//! address and read length are fields), then the transaction, its result,
//! and this session's transaction log.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;
use usb_i2c::proto::MAX_XFER;

use super::parse::{hex_dump, parse_addr, parse_bytes, parse_len, short_error, to_hex};
use super::{Ctx, FIXED_PORT, FIXED_RATE, NO_CMD};
use crate::theme;
use crate::widgets::{self, Table, Tone};
use crate::worker::{Op, Transfer};

const LOG_MAX: usize = 200;

pub struct Record {
    id: u64,
    /// Since the window opened.
    at: Duration,
    op: Op,
    addr: u8,
    wdata: Vec<u8>,
    rlen: usize,
    result: Result<Vec<u8>, String>,
}

impl Record {
    /// One line for the result card and the footer.
    pub fn summary(&self) -> String {
        match &self.result {
            Ok(data) => match self.op {
                Op::Write => format!("已写入 {} 字节到 0x{:02x}", self.wdata.len(), self.addr),
                Op::Read => format!("读回 {} 字节", data.len()),
                Op::WriteRead => format!("写后读回 {} 字节", data.len()),
            },
            Err(e) => e.clone(),
        }
    }

    pub fn ok(&self) -> bool {
        self.result.is_ok()
    }
}

pub struct Master {
    addr: String,
    write_hex: String,
    read_len: String,
    /// Newest first.
    log: VecDeque<Record>,
    next_id: u64,
    /// The record shown in the result card.
    selected: Option<u64>,
}

impl Default for Master {
    fn default() -> Self {
        Self {
            addr: "0x50".into(),
            write_hex: String::new(),
            read_len: "16".into(),
            log: VecDeque::new(),
            next_id: 1,
            selected: None,
        }
    }
}

impl Master {
    /// Check the fields and build the transfer for `op`.
    pub fn transfer(&self, op: Op) -> Result<Transfer, String> {
        let addr = parse_addr(&self.addr)?;
        // 读取 is a plain read, as the CLI's `read`: the write field is not sent
        let wdata = match op {
            Op::Read => Vec::new(),
            Op::Write | Op::WriteRead => parse_bytes(&self.write_hex)?,
        };
        let rlen = match op {
            Op::Write => 0,
            Op::Read | Op::WriteRead => parse_len(&self.read_len)?,
        };
        if matches!(op, Op::Write | Op::WriteRead) && wdata.is_empty() {
            return Err("写入数据为空".into());
        }
        Ok(Transfer {
            op,
            addr,
            wdata,
            rlen,
        })
    }

    /// Log a finished transfer and show it in the result card.
    pub fn record(
        &mut self,
        at: Duration,
        t: Transfer,
        result: Result<Vec<u8>, String>,
    ) -> &Record {
        let id = self.next_id;
        self.next_id += 1;
        self.log.push_front(Record {
            id,
            at,
            op: t.op,
            addr: t.addr,
            wdata: t.wdata,
            rlen: t.rlen,
            result,
        });
        self.log.truncate(LOG_MAX);
        self.selected = Some(id);
        &self.log[0]
    }

    fn shown(&self) -> Option<&Record> {
        let id = self.selected?;
        self.log.iter().find(|r| r.id == id)
    }
}

pub fn show(ui: &mut egui::Ui, m: &mut Master, cx: Ctx) -> Option<Op> {
    widgets::work_columns(ui, m, 264.0, 760.0, config, |ui, m| {
        let op = compose(ui, m, cx);
        ui.add_space(14.0);
        result(ui, m);
        ui.add_space(14.0);
        log(ui, m);
        op
    })
}

fn config(ui: &mut egui::Ui, m: &mut Master) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, "I2C0", Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            widgets::field_row(ui, "端口", false, |ui| {
                widgets::fixed_value(ui, "I2C0 · 主机", FIXED_PORT);
            });
            widgets::field_row(ui, "速率", false, |ui| {
                widgets::fixed_value(ui, "100 kHz", FIXED_RATE);
            });
            widgets::field_row(ui, "目标地址", false, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut m.addr)
                        .hint_text("0x50 或 80")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                );
            });
            widgets::field_row(ui, "读取长度", true, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut m.read_len)
                        .hint_text("1-256")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width() - 18.0),
                );
                ui.label(egui::RichText::new("B").size(11.0).color(t.muted));
            });
            let errs = [
                parse_addr(&m.addr)
                    .err()
                    .filter(|_| !m.addr.trim().is_empty()),
                parse_len(&m.read_len)
                    .err()
                    .filter(|_| !m.read_len.trim().is_empty()),
            ];
            for e in errs.into_iter().flatten() {
                ui.label(egui::RichText::new(e).size(11.5).color(t.red));
            }
        },
    );
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("地址不含读写位，按 7 位（0x00–0x7F）填写。")
            .size(11.5)
            .color(t.muted),
    );
    ui.add_space(10.0);
    widgets::panel(
        ui,
        "总线工具",
        |ui| {
            widgets::badge(ui, "待协议", Tone::Amber);
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::action_button(ui, false, "扫描地址").on_disabled_hover_text(NO_CMD);
                widgets::action_button(ui, false, "总线恢复").on_disabled_hover_text(NO_CMD);
            });
            ui.label(
                egui::RichText::new(
                    "地址扫描（0x08–0x77）和总线恢复（9 个时钟 + STOP）需要固件新增 USB 命令，当前版本不提供。",
                )
                .size(11.5)
                .color(t.muted),
            );
        },
    );
}

fn compose(ui: &mut egui::Ui, m: &mut Master, cx: Ctx) -> Option<Op> {
    let t = theme::of(ui);
    let mut op = None;
    widgets::panel(
        ui,
        "读写事务",
        |ui| {
            ui.label(egui::RichText::new("HEX").size(11.0).color(t.muted));
        },
        |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("写入数据").size(12.0).color(t.muted));
                widgets::trailing(ui, 18.0, |ui| {
                    let (text, color) = match parse_bytes(&m.write_hex) {
                        Ok(b) => (format!("{} / {MAX_XFER} B", b.len()), t.muted),
                        Err(e) => (e, t.red),
                    };
                    ui.label(egui::RichText::new(text).size(11.5).color(color));
                });
            });
            ui.add(
                egui::TextEdit::multiline(&mut m.write_hex)
                    .hint_text("00 11 22 33")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            ui.label(
                egui::RichText::new(
                    "当前从机把第一个字节当作寄存器表偏移。写后读：写完不发 STOP，重复 START 后读取。",
                )
                .size(11.5)
                .color(t.muted),
            );
            ui.horizontal(|ui| {
                let on = cx.connected && !cx.busy;
                let why = if cx.connected {
                    "上一个操作还在进行"
                } else {
                    "先在顶栏连接设备"
                };
                let buttons = [
                    (Op::Write, widgets::action_button(ui, on, Op::Write.label())),
                    (Op::Read, widgets::action_button(ui, on, Op::Read.label())),
                    (
                        Op::WriteRead,
                        widgets::ink_button(ui, on, Op::WriteRead.label()),
                    ),
                ];
                for (o, resp) in buttons {
                    if resp.on_disabled_hover_text(why).clicked() {
                        op = Some(o);
                    }
                }
                if cx.busy {
                    ui.add(egui::Spinner::new().size(13.0).color(t.muted));
                } else if !cx.connected {
                    ui.label(egui::RichText::new("未连接").size(11.5).color(t.muted));
                }
            });
        },
    );
    op
}

fn result(ui: &mut egui::Ui, m: &Master) {
    let t = theme::of(ui);
    let rec = m.shown();
    widgets::panel(
        ui,
        "结果",
        |ui| {
            let Some(r) = rec else { return };
            if let Ok(data) = &r.result {
                if !data.is_empty() && widgets::quiet_button(ui, true, "复制 HEX").clicked() {
                    ui.ctx().copy_text(to_hex(data));
                }
            }
            if r.ok() {
                widgets::badge(ui, "ACK", Tone::Green);
            } else if let Err(e) = &r.result {
                widgets::badge(ui, short_error(e), Tone::Red);
            }
        },
        |ui| {
            let Some(r) = rec else {
                ui.label(
                    egui::RichText::new(
                        "还没有传输。连接设备后执行写入、读取或写后读，结果显示在这里。",
                    )
                    .color(t.muted),
                );
                return;
            };
            ui.label(
                egui::RichText::new(format!(
                    "#{}  {}  ·  {}  ·  0x{:02x}  ·  写 {} B  ·  读 {} B",
                    r.id,
                    clock(r.at),
                    r.op.label(),
                    r.addr,
                    r.wdata.len(),
                    r.rlen
                ))
                .size(11.5)
                .color(t.muted),
            );
            match &r.result {
                Ok(data) if data.is_empty() => {
                    ui.label(r.summary());
                }
                Ok(data) => {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(hex_dump(data).trim_end())
                                .monospace()
                                .color(t.fg),
                        )
                        .selectable(true),
                    );
                }
                Err(e) => {
                    ui.add(egui::Label::new(egui::RichText::new(e).color(t.red)).selectable(true));
                }
            }
        },
    );
}

fn log(ui: &mut egui::Ui, m: &mut Master) {
    let t = theme::of(ui);
    let mut clear = false;
    let mut pick = None;
    let count = m.log.len();
    widgets::panel(
        ui,
        "事务记录",
        |ui| {
            if widgets::quiet_button(ui, count > 0, "清空").clicked() {
                clear = true;
            }
            ui.label(
                egui::RichText::new(format!("{count} 条 · 本次会话"))
                    .size(11.0)
                    .color(t.muted),
            );
        },
        |ui| {
            if m.log.is_empty() {
                ui.label(egui::RichText::new("暂无记录。").color(t.muted));
                return;
            }
            let table = Table {
                widths: &[84.0, 58.0, 50.0, 0.0, 0.0, 64.0],
            };
            table.header(ui, &["时间", "操作", "地址", "写入", "读回", "结果"]);
            egui::ScrollArea::vertical()
                .id_salt("i2c_log")
                .max_height(260.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for r in &m.log {
                        let (res, color) = match &r.result {
                            Ok(_) => ("ACK", t.green),
                            Err(e) => (short_error(e), t.red),
                        };
                        let read = match &r.result {
                            Ok(d) if !d.is_empty() => preview(d),
                            _ if r.rlen > 0 => format!("({} B)", r.rlen),
                            _ => "—".into(),
                        };
                        let write = if r.wdata.is_empty() {
                            "—".into()
                        } else {
                            preview(&r.wdata)
                        };
                        let cells = [
                            (clock(r.at), t.muted, true),
                            (r.op.label().to_owned(), t.fg, false),
                            (format!("0x{:02x}", r.addr), t.fg, true),
                            (write, t.fg2, true),
                            (read, t.fg, true),
                            (res.to_owned(), color, false),
                        ];
                        let resp = table.row(ui, &cells, m.selected == Some(r.id));
                        let resp = match &r.result {
                            Err(e) => resp.on_hover_text(e),
                            Ok(_) => resp,
                        };
                        if resp.clicked() {
                            pick = Some(r.id);
                        }
                    }
                });
        },
    );
    if clear {
        m.log.clear();
        m.selected = None;
    }
    if pick.is_some() {
        m.selected = pick;
    }
}

/// First bytes as hex, with an ellipsis when there are more.
fn preview(data: &[u8]) -> String {
    const N: usize = 12;
    if data.len() > N {
        format!("{} …", to_hex(&data[..N]))
    } else {
        to_hex(data)
    }
}

/// `mm:ss.mmm` since the window opened.
fn clock(d: Duration) -> String {
    let ms = d.as_millis();
    format!(
        "{:02}:{:02}.{:03}",
        ms / 60_000,
        (ms / 1000) % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_checks_fields() {
        let mut m = Master::default();
        assert_eq!(m.transfer(Op::Write).unwrap_err(), "写入数据为空");
        let r = m.transfer(Op::Read).unwrap();
        assert_eq!((r.addr, r.rlen, r.wdata.len()), (0x50, 16, 0));
        m.write_hex = "00 01".into();
        let w = m.transfer(Op::Write).unwrap();
        assert_eq!((w.wdata, w.rlen), (vec![0, 1], 0));
        let wr = m.transfer(Op::WriteRead).unwrap();
        assert_eq!((wr.wdata.len(), wr.rlen), (2, 16));
        // a read never sends the write field, and does not care if it is bad
        assert!(m.transfer(Op::Read).unwrap().wdata.is_empty());
        m.write_hex = "zz".into();
        assert!(m.transfer(Op::Read).is_ok());
        assert!(m.transfer(Op::WriteRead).is_err());
        m.addr = "0x80".into();
        assert!(m.transfer(Op::Read).is_err());
    }

    #[test]
    fn log_keeps_newest_first() {
        let mut m = Master::default();
        for i in 0..(LOG_MAX + 5) {
            let t = Transfer {
                op: Op::Read,
                addr: 0x50,
                wdata: Vec::new(),
                rlen: 1,
            };
            let rec = m.record(Duration::from_millis(i as u64), t, Ok(vec![i as u8]));
            assert!(rec.ok());
        }
        assert_eq!(m.log.len(), LOG_MAX);
        assert_eq!(m.shown().unwrap().id, (LOG_MAX + 5) as u64);
        // an old pick never falls off the end: each new record takes the card
        m.selected = Some(m.log.back().unwrap().id);
        let t = Transfer {
            op: Op::Read,
            addr: 0x50,
            wdata: Vec::new(),
            rlen: 1,
        };
        m.record(Duration::ZERO, t, Ok(vec![0]));
        assert_eq!(m.shown().unwrap().id, (LOG_MAX + 6) as u64);
        assert_eq!(clock(Duration::from_millis(61_234)), "01:01.234");
        assert_eq!(preview(&[0; 13]).matches("00").count(), 12);
    }
}
