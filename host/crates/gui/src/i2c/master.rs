//! 主机 tab. Config on the left (port and rate are fixed by the firmware;
//! address, register address, and read length are fields), then the
//! transaction, its result, and this session's transaction log.
//!
//! Real transfers use the bridge's one I2C command, once or on a loop
//! (循环 sends the same transfer again each period until switched off or a
//! transfer fails). 扫描地址 and 总线恢复 have no command: they insert rows
//! marked 演示 into the log, which the counts leave out.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;
use usb_i2c::proto::MAX_XFER;

use super::parse::{
    clock, hex_dump, parse_addr, parse_bytes, parse_len, preview, short_error, to_hex,
};
use super::{Ctx, FIXED_PORT, FIXED_RATE};
use crate::local::{parse_hex, parse_num, Saved};
use crate::theme;
use crate::widgets::{self, Table, Tone};
use crate::worker::{Op, Transfer};

const LOG_MAX: usize = 200;
/// Register address width: true is 16 bit (sent high byte first).
const REG_WIDTHS: [(bool, &str); 2] = [(false, "8 bit"), (true, "16 bit")];
/// Addresses the demo scan reports.
const DEMO_SCAN: [u8; 3] = [0x48, 0x50, 0x68];
const DEMO_HOVER: &str = "本地演示：插入示例行，不发 USB（当前固件无此命令）";

pub struct Record {
    id: u64,
    /// Since the window opened.
    at: Duration,
    op: Op,
    addr: u8,
    wdata: Vec<u8>,
    rlen: usize,
    result: Result<Vec<u8>, String>,
    /// Set on rows the window made up (scan, bus recovery): what it stands for.
    demo: Option<&'static str>,
}

impl Record {
    /// One line for the result card and the footer.
    pub fn summary(&self) -> String {
        if let Some(what) = self.demo {
            return format!("本地演示：{what} 0x{:02x}，未发 USB", self.addr);
        }
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

/// What 保存配置 keeps.
#[derive(Clone, PartialEq, Debug)]
struct Cfg {
    addr: String,
    /// Register address sent before the write bytes; empty for none.
    reg: String,
    reg16: bool,
    read_len: String,
    period: String,
}

impl Default for Cfg {
    fn default() -> Self {
        Self {
            addr: "0x50".into(),
            reg: String::new(),
            reg16: false,
            read_len: "16".into(),
            period: "100".into(),
        }
    }
}

impl Cfg {
    /// The register address as bytes, high first; none when the field is empty.
    fn reg_bytes(&self) -> Result<Vec<u8>, String> {
        if self.reg.trim().is_empty() {
            return Ok(Vec::new());
        }
        if self.reg16 {
            let v = parse_hex(&self.reg, 0xffff, "寄存器地址")?;
            Ok(vec![(v >> 8) as u8, v as u8])
        } else {
            Ok(vec![parse_hex(&self.reg, 0xff, "寄存器地址")? as u8])
        }
    }

    fn period(&self) -> Result<Duration, String> {
        parse_num(&self.period, 20, 60_000, "循环周期").map(|ms| Duration::from_millis(ms.into()))
    }
}

pub struct Master {
    cfg: Cfg,
    saved: Saved<Cfg>,
    advanced: bool,
    write_hex: String,
    /// 循环 switch.
    looping: bool,
    /// The transfer being repeated and when it is next due.
    loop_op: Option<(Op, Duration)>,
    /// Why the loop stopped by itself.
    loop_note: Option<String>,
    /// Newest first.
    log: VecDeque<Record>,
    next_id: u64,
    /// The record shown in the result card.
    selected: Option<u64>,
    /// While set, the log shows records up to this id only.
    frozen: Option<u64>,
}

impl Default for Master {
    fn default() -> Self {
        Self {
            cfg: Cfg::default(),
            saved: Saved::default(),
            advanced: false,
            write_hex: String::new(),
            looping: false,
            loop_op: None,
            loop_note: None,
            log: VecDeque::new(),
            next_id: 1,
            selected: None,
            frozen: None,
        }
    }
}

impl Master {
    /// Check the fields and build the transfer for `op`. A register
    /// address, if given, goes before the write bytes.
    pub fn transfer(&self, op: Op) -> Result<Transfer, String> {
        let addr = parse_addr(&self.cfg.addr)?;
        // 读取 is a plain read, as the CLI's `read`: nothing is written first
        let wdata = match op {
            Op::Read => Vec::new(),
            Op::Write | Op::WriteRead => {
                let mut w = self.cfg.reg_bytes()?;
                w.extend(parse_bytes(&self.write_hex)?);
                w
            }
        };
        let rlen = match op {
            Op::Write => 0,
            Op::Read | Op::WriteRead => parse_len(&self.cfg.read_len)?,
        };
        if matches!(op, Op::Write | Op::WriteRead) && wdata.is_empty() {
            return Err("写入数据为空".into());
        }
        if wdata.len() > MAX_XFER {
            return Err(format!("寄存器地址加数据超过 {MAX_XFER} 字节"));
        }
        Ok(Transfer {
            op,
            addr,
            wdata,
            rlen,
        })
    }

    /// Log a finished transfer and show it in the result card. A failure
    /// stops the loop.
    pub fn record(
        &mut self,
        at: Duration,
        t: Transfer,
        result: Result<Vec<u8>, String>,
    ) -> &Record {
        if result.is_err() && self.loop_op.take().is_some() {
            self.looping = false;
            self.loop_note = Some("传输失败，循环已停止".into());
        }
        self.push(Record {
            id: 0,
            at,
            op: t.op,
            addr: t.addr,
            wdata: t.wdata,
            rlen: t.rlen,
            result,
            demo: None,
        });
        &self.log[0]
    }

    fn push(&mut self, mut r: Record) {
        r.id = self.next_id;
        self.next_id += 1;
        if self.frozen.is_none() {
            self.selected = Some(r.id);
        }
        self.log.push_front(r);
        self.log.truncate(LOG_MAX);
    }

    /// 扫描地址 without a command: one demo row per example address.
    fn demo_scan(&mut self, at: Duration) {
        for (i, addr) in DEMO_SCAN.into_iter().enumerate() {
            self.push(Record {
                id: 0,
                at: at + Duration::from_millis(i as u64),
                op: Op::Write,
                addr,
                wdata: Vec::new(),
                rlen: 0,
                result: Ok(Vec::new()),
                demo: Some("扫描应答"),
            });
        }
    }

    /// 总线恢复 without a command: one demo row.
    fn demo_recover(&mut self, at: Duration) {
        self.push(Record {
            id: 0,
            at,
            op: Op::Write,
            addr: 0,
            wdata: Vec::new(),
            rlen: 0,
            result: Ok(Vec::new()),
            demo: Some("总线恢复 · 9 个 SCL 脉冲"),
        });
    }

    pub fn looping(&self) -> bool {
        self.loop_op.is_some()
    }

    /// The transfer to send again, when 循环 is on and its period has come.
    /// Disconnecting or a field turning bad stops the loop.
    pub fn loop_due(&mut self, now: Duration, connected: bool, busy: bool) -> Option<Op> {
        let (op, next) = self.loop_op?;
        let stop = if !connected {
            Some("设备已断开，循环已停止".to_owned())
        } else {
            self.transfer(op)
                .err()
                .or_else(|| self.cfg.period().err())
                .map(|e| format!("{e}，循环已停止"))
        };
        if let Some(why) = stop {
            self.loop_op = None;
            self.looping = false;
            self.loop_note = Some(why);
            return None;
        }
        if busy || now < next {
            return None;
        }
        let period = self.cfg.period().ok()?;
        self.loop_op = Some((op, now + period));
        Some(op)
    }

    fn shown(&self) -> Option<&Record> {
        let id = self.selected?;
        self.log.iter().find(|r| r.id == id)
    }

    /// Records that went over USB.
    fn real(&self) -> impl Iterator<Item = &Record> {
        self.log.iter().filter(|r| r.demo.is_none())
    }
}

/// 保存配置 for the page head.
pub fn save_button(ui: &mut egui::Ui, m: &mut Master, now: Duration) {
    m.saved.button(ui, &m.cfg, now);
}

pub fn show(ui: &mut egui::Ui, m: &mut Master, cx: Ctx) -> Option<Op> {
    widgets::work_columns(ui, m, 264.0, 760.0, config, |ui, m| {
        summary(ui, m);
        ui.add_space(12.0);
        let op = compose(ui, m, cx);
        ui.add_space(14.0);
        result(ui, m);
        ui.add_space(14.0);
        log(ui, m, cx.now);
        op
    })
}

fn config(ui: &mut egui::Ui, m: &mut Master) {
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, "I2C0", Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let c = &mut m.cfg;
            widgets::field_row(ui, "端口", false, |ui| {
                widgets::fixed_value(ui, "I2C0 · 主机", FIXED_PORT);
            });
            widgets::field_row(ui, "速率", false, |ui| {
                widgets::fixed_value(ui, "100 kHz", FIXED_RATE);
            });
            widgets::field_row(ui, "目标地址", false, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut c.addr)
                        .hint_text("0x50 或 80")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                );
            });
            widgets::field_row(ui, "寄存器地址", false, |ui| {
                widgets::unit_edit(ui, &mut c.reg, "可空，如 0x00", "");
            });
            widgets::field_row(ui, "读取长度", false, |ui| {
                widgets::unit_edit(ui, &mut c.read_len, "1-256", "B");
            });
            widgets::disclosure(ui, &mut m.advanced, "高级参数", 2);
            widgets::hairline(ui);
            if m.advanced {
                widgets::field_row(ui, "寄存器宽度", false, |ui| {
                    widgets::pick(ui, &mut c.reg16, &REG_WIDTHS);
                });
                widgets::field_row(ui, "循环周期", false, |ui| {
                    widgets::unit_edit(ui, &mut c.period, "100", "ms");
                });
            }
            let errs = [
                parse_addr(&c.addr)
                    .err()
                    .filter(|_| !c.addr.trim().is_empty()),
                c.reg_bytes().err(),
                parse_len(&c.read_len)
                    .err()
                    .filter(|_| !c.read_len.trim().is_empty()),
                c.period().err(),
            ];
            for e in errs.into_iter().flatten() {
                widgets::error(ui, &e);
            }
            m.saved.foot(ui, &mut m.cfg);
        },
    );
    ui.add_space(8.0);
    widgets::note(
        ui,
        "地址不含读写位，按 7 位（0x00–0x7F）填写。寄存器地址在写入与写后读时先发出，读取不发。",
    );
}

/// Counts over this session's real transfers; demo rows are left out.
fn summary(ui: &mut egui::Ui, m: &Master) {
    let total = m.real().count();
    let acked = m.real().filter(|r| r.ok()).count();
    let read: usize = m
        .real()
        .filter_map(|r| r.result.as_ref().ok())
        .map(Vec::len)
        .sum();
    widgets::summary_strip(
        ui,
        &[
            ("事务", total.to_string()),
            ("应答成功", format!("{acked} / {total}")),
            ("接收字节", format!("{read} B")),
        ],
        "本次会话 · 不含演示行",
    );
}

fn compose(ui: &mut egui::Ui, m: &mut Master, cx: Ctx) -> Option<Op> {
    let t = theme::of(ui);
    let mut op = None;
    widgets::panel(
        ui,
        "读写事务",
        |ui| {
            widgets::muted(ui, "HEX");
        },
        |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("写入数据").size(12.0).color(t.muted));
                widgets::trailing(ui, 18.0, |ui| {
                    let (text, color) = match parse_bytes(&m.write_hex) {
                        Ok(b) => {
                            let reg = m.cfg.reg_bytes().map_or(0, |r| r.len());
                            let text = if reg > 0 {
                                format!("寄存器 {reg} B + {} / {MAX_XFER} B", b.len())
                            } else {
                                format!("{} / {MAX_XFER} B", b.len())
                            };
                            (text, t.muted)
                        }
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
            widgets::muted(
                ui,
                "当前从机把第一个字节当作寄存器表偏移。写后读：写完不发 STOP，重复 START 后读取。",
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
                ui.add_space(6.0);
                let period = m
                    .cfg
                    .period()
                    .map_or("循环".into(), |p| format!("循环 · {} ms", p.as_millis()));
                let r = widgets::check(ui, &mut m.looping, &period)
                    .on_hover_text("开启后，点读写按钮会按周期重复同一笔 I2C 传输，直到关闭或失败");
                if r.changed() {
                    m.loop_note = None;
                    if !m.looping {
                        m.loop_op = None;
                    }
                }
                if cx.busy {
                    ui.add(egui::Spinner::new().size(13.0).color(t.muted));
                } else if !cx.connected {
                    widgets::muted(ui, "未连接");
                }
            });
            if let Some((o, _)) = m.loop_op {
                ui.horizontal(|ui| {
                    widgets::badge(ui, &format!("循环中 · {}", o.label()), Tone::Green);
                    widgets::muted(ui, "每次都是一笔真实 I2C 传输");
                });
            } else if let Some(n) = &m.loop_note {
                ui.label(egui::RichText::new(n).size(11.5).color(t.amber));
            }
        },
    );
    if let Some(o) = op {
        if m.looping {
            if let Ok(p) = m.cfg.period() {
                m.loop_op = Some((o, cx.now + p));
                m.loop_note = None;
            }
        }
    }
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
            if r.demo.is_some() {
                widgets::badge(ui, "演示", Tone::Neutral);
                return;
            }
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
            if r.demo.is_some() {
                ui.label(
                    egui::RichText::new(format!("#{}  {}  ·  {}", r.id, clock(r.at), r.summary()))
                        .size(11.5)
                        .color(t.muted),
                );
                ui.label(
                    egui::RichText::new(
                        "这是示例行：当前固件没有扫描与总线恢复命令，结果不代表总线上的器件。",
                    )
                    .color(t.fg2),
                );
                return;
            }
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

fn log(ui: &mut egui::Ui, m: &mut Master, now: Duration) {
    let t = theme::of(ui);
    let mut clear = false;
    let mut pick = None;
    let count = m.log.len();
    widgets::panel(
        ui,
        "事务记录",
        |ui| {
            widgets::muted(ui, &format!("{count} 条 · 本次会话"));
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::muted(ui, "I2C0 · 时间 / HEX");
                widgets::trailing(ui, 27.0, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::quiet_button(ui, count > 0, "清空").clicked() {
                        clear = true;
                    }
                    let text = if m.frozen.is_some() {
                        "继续显示"
                    } else {
                        "暂停显示"
                    };
                    if widgets::quiet_button(ui, true, text)
                        .on_hover_text("只冻结列表；传输照常进行")
                        .clicked()
                    {
                        m.frozen = match m.frozen {
                            Some(_) => None,
                            None => Some(m.next_id - 1),
                        };
                    }
                    if widgets::quiet_button(ui, true, "总线恢复 · 演示")
                        .on_hover_text(DEMO_HOVER)
                        .clicked()
                    {
                        m.demo_recover(now);
                    }
                    if widgets::quiet_button(ui, true, "扫描地址 · 演示")
                        .on_hover_text(DEMO_HOVER)
                        .clicked()
                    {
                        m.demo_scan(now);
                    }
                });
            });
            if m.log.is_empty() {
                ui.label(egui::RichText::new("暂无记录。").color(t.muted));
                return;
            }
            let table = Table {
                widths: &[84.0, 72.0, 50.0, 0.0, 0.0, 64.0],
            };
            table.header(ui, &["时间", "操作", "地址", "写入", "读回", "结果"]);
            egui::ScrollArea::vertical()
                .id_salt("i2c_log")
                .max_height(260.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for r in m.log.iter().filter(|r| m.frozen.is_none_or(|f| r.id <= f)) {
                        let cells = match r.demo {
                            Some(what) => {
                                let op = if what.starts_with("扫描") {
                                    "扫描"
                                } else {
                                    "总线恢复"
                                };
                                let addr = if r.addr == 0 {
                                    "—".to_owned()
                                } else {
                                    format!("0x{:02x}", r.addr)
                                };
                                [
                                    (clock(r.at), t.muted, true),
                                    (op.to_owned(), t.fg2, false),
                                    (addr, t.fg2, true),
                                    ("—".to_owned(), t.faint, true),
                                    ("—".to_owned(), t.faint, true),
                                    ("演示".to_owned(), t.accent, false),
                                ]
                            }
                            None => {
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
                                [
                                    (clock(r.at), t.muted, true),
                                    (r.op.label().to_owned(), t.fg, false),
                                    (format!("0x{:02x}", r.addr), t.fg, true),
                                    (write, t.fg2, true),
                                    (read, t.fg, true),
                                    (res.to_owned(), color, false),
                                ]
                            }
                        };
                        let resp = table.row(ui, &cells, m.selected == Some(r.id));
                        let resp = match (&r.result, r.demo) {
                            (_, Some(_)) => resp.on_hover_text(DEMO_HOVER),
                            (Err(e), None) => resp.on_hover_text(e),
                            (Ok(_), None) => resp,
                        };
                        if resp.clicked() {
                            pick = Some(r.id);
                        }
                    }
                });
            let left = if m.frozen.is_some() {
                format!("已暂停显示 · 共 {count} 条，传输不受影响")
            } else {
                "最新在上 · 点行查看结果 · 演示行不计入统计".to_owned()
            };
            widgets::table_foot(ui, &left, &format!("最多 {LOG_MAX} 条"));
        },
    );
    if clear {
        m.log.clear();
        m.selected = None;
        m.frozen = None;
    }
    if pick.is_some() {
        m.selected = pick;
    }
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
        m.cfg.addr = "0x80".into();
        assert!(m.transfer(Op::Read).is_err());
    }

    #[test]
    fn register_address_goes_first() {
        let mut m = Master::default();
        m.cfg.reg = "0x10".into();
        // the register alone is enough for write-then-read
        assert_eq!(m.transfer(Op::WriteRead).unwrap().wdata, vec![0x10]);
        m.write_hex = "aa".into();
        assert_eq!(m.transfer(Op::Write).unwrap().wdata, vec![0x10, 0xaa]);
        assert!(m.transfer(Op::Read).unwrap().wdata.is_empty());
        m.cfg.reg16 = true;
        m.cfg.reg = "0x1234".into();
        assert_eq!(m.transfer(Op::Write).unwrap().wdata, vec![0x12, 0x34, 0xaa]);
        m.cfg.reg16 = false;
        assert!(m.transfer(Op::Write).is_err());
        m.cfg.reg = "0x12".into();
        m.write_hex = "00 ".repeat(256);
        assert!(m.transfer(Op::Write).unwrap_err().contains("超过"));
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
    }

    #[test]
    fn demo_rows_stay_out_of_the_counts() {
        let mut m = Master::default();
        m.demo_scan(Duration::ZERO);
        m.demo_recover(Duration::ZERO);
        assert_eq!(m.log.len(), 4);
        assert_eq!(m.real().count(), 0);
        assert!(m
            .shown()
            .unwrap()
            .summary()
            .starts_with("本地演示：总线恢复"));
    }

    #[test]
    fn loop_repeats_until_stopped() {
        let ms = Duration::from_millis;
        let mut m = Master {
            looping: true,
            loop_op: Some((Op::Read, ms(100))),
            ..Master::default()
        };
        assert_eq!(m.loop_due(ms(50), true, false), None);
        assert_eq!(m.loop_due(ms(100), true, true), None);
        assert_eq!(m.loop_due(ms(100), true, false), Some(Op::Read));
        assert_eq!(m.loop_due(ms(150), true, false), None);
        assert_eq!(m.loop_due(ms(200), true, false), Some(Op::Read));
        // a failed transfer stops it
        let t = Transfer {
            op: Op::Read,
            addr: 0x50,
            wdata: Vec::new(),
            rlen: 1,
        };
        m.record(ms(210), t, Err("Nack".into()));
        assert!(!m.looping() && !m.looping);
        // so does losing the device
        m.loop_op = Some((Op::Read, ms(0)));
        assert_eq!(m.loop_due(ms(300), false, false), None);
        assert!(m.loop_note.as_deref().unwrap().contains("断开"));
    }
}
