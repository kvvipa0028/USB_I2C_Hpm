//! 主机 tab. Capability-gated port and rate, address, register address,
//! and read length, then the
//! transaction, its result, and this session's transaction log.
//!
//! Real transfers use the bridge's one I2C command, once or on a loop
//! (循环 sends the same transfer again each period until switched off or a
//! transfer fails). 扫描地址 and 总线恢复 have no command: they insert rows
//! marked 演示 into the log, which the counts leave out.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;
use usb_i2c::proto::{Capabilities, MAX_XFER};

use super::parse::{
    clock, hex_dump, parse_addr, parse_bytes, parse_len, preview, short_error, to_hex,
};
use super::Ctx;
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
    port: u8,
    speed_hz: u32,
    offline: bool,
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
        let prefix = format!(
            "{} CH{} · ",
            if self.offline { "离线" } else { "USB" },
            self.port
        );
        match &self.result {
            Ok(data) => match self.op {
                Op::Write => format!(
                    "{prefix}已写入 {} 字节到 0x{:02x}",
                    self.wdata.len(),
                    self.addr
                ),
                Op::Read => format!("{prefix}读回 {} 字节", data.len()),
                Op::WriteRead => format!("{prefix}写后读回 {} 字节", data.len()),
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
    port: u8,
    speed_hz: String,
    addr: String,
    /// UI-only preference, not sent to the bridge until its protocol supports it.
    clock_stretching: bool,
    /// Register address sent before the write bytes; empty for none.
    reg: String,
    reg16: bool,
    read_len: String,
    period: String,
}

impl Default for Cfg {
    fn default() -> Self {
        Self {
            port: 0,
            speed_hz: "100000".into(),
            addr: "0x50".into(),
            clock_stretching: true,
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
    capabilities: Capabilities,
    offline: bool,
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
            capabilities: Capabilities::legacy(),
            offline: false,
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
    pub fn set_connection(&mut self, capabilities: &Capabilities, offline: bool) {
        self.stop_loop("连接操作，循环已停止");
        self.capabilities = capabilities.clone();
        self.offline = offline;
        // Do not carry an unsupported draft over to a legacy connection.
        self.cfg.port = 0;
        self.cfg.speed_hz = capabilities.fixed_speed_hz.to_string();
        // A saved CH1/custom-rate draft must not be restorable on a legacy device.
        self.saved = Saved::default();
    }

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
        let speed_hz = parse_num(
            &self.cfg.speed_hz,
            self.capabilities.min_speed_hz,
            self.capabilities.max_speed_hz,
            "SCL 频率",
        )?;
        if self.cfg.port >= self.capabilities.port_count {
            return Err("所选端口不在设备能力范围内".into());
        }
        if wdata.len() > self.capabilities.max_transfer || rlen > self.capabilities.max_transfer {
            return Err(format!(
                "设备每次最多支持 {} 字节",
                self.capabilities.max_transfer
            ));
        }
        Ok(Transfer {
            port: self.cfg.port,
            speed_hz,
            offline: self.offline,
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
        if result.is_err() {
            self.stop_loop("传输失败，循环已停止");
        }
        self.push(Record {
            id: 0,
            port: t.port,
            speed_hz: t.speed_hz,
            offline: t.offline,
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
                port: self.cfg.port,
                speed_hz: 100_000,
                offline: self.offline,
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
            port: self.cfg.port,
            speed_hz: 100_000,
            offline: self.offline,
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

    /// Disarm both the scheduled transfer and the switch at the boundary that
    /// ends a connection or reports an error, without waiting for another frame.
    pub fn stop_loop(&mut self, reason: &str) {
        let was_enabled = self.looping || self.loop_op.is_some();
        self.looping = false;
        self.loop_op = None;
        if was_enabled {
            self.loop_note = Some(reason.to_owned());
        }
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
            self.stop_loop(&why);
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

    /// Shared fixture for App's queue-only connection lifecycle tests.
    #[cfg(test)]
    pub(crate) fn arm_test_write_loop(&mut self) {
        self.write_hex = "00 AA".into();
        self.looping = true;
        self.loop_op = Some((Op::Write, Duration::ZERO));
    }
}

/// 保存配置 for the page head.
pub fn save_button(ui: &mut egui::Ui, m: &mut Master, now: Duration) {
    m.saved.button(ui, &m.cfg, now);
}

pub fn show(ui: &mut egui::Ui, m: &mut Master, cx: Ctx) -> Option<Op> {
    widgets::work_columns(
        ui,
        m,
        264.0,
        760.0,
        |ui, m| config(ui, m, cx),
        |ui, m| {
            summary(ui, m);
            ui.add_space(12.0);
            let op = compose(ui, m, cx);
            ui.add_space(14.0);
            result(ui, m);
            ui.add_space(14.0);
            log(ui, m, cx.now);
            op
        },
    )
}

fn config(ui: &mut egui::Ui, m: &mut Master, cx: Ctx) {
    let before = m.cfg.clone();
    let port_label = format!("CH{}", m.cfg.port);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, &port_label, Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let c = &mut m.cfg;
            widgets::field_row(ui, "端口", false, |ui| {
                ui.add_enabled_ui(!cx.busy && !m.looping, |ui| {
                    egui::ComboBox::from_id_salt("master_port")
                        .selected_text(format!("CH{} · 主机", c.port))
                        .show_ui(ui, |ui| {
                            for port in 0..m.capabilities.port_count {
                                ui.selectable_value(&mut c.port, port, format!("CH{port} · 主机"));
                            }
                        });
                });
            });
            widgets::field_row(ui, "速率", false, |ui| {
                ui.add_enabled_ui(
                    !cx.busy && !m.looping && m.capabilities.supports_custom_speed,
                    |ui| {
                        widgets::unit_edit(ui, &mut c.speed_hz, "100000", "Hz");
                    },
                );
            });
            widgets::note(
                ui,
                if m.offline {
                    "离线模型：仅检查 10k–1M Hz 参数，不模拟实际速率。"
                } else if !m.capabilities.supports_custom_speed {
                    "设备仅支持固定速率；此处显示设备能力。"
                } else {
                    "频率随每笔请求发送，实际波形需上板测量。"
                },
            );
            widgets::field_row(ui, "时钟延展", false, |ui| {
                widgets::check(ui, &mut c.clock_stretching, "启用");
                widgets::badge(ui, "仅设置", Tone::Neutral);
            });
            widgets::note(ui, "仅保存本窗口设置，尚未接入固件，不影响实际 I2C 通信。");
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
                parse_num(
                    &c.speed_hz,
                    m.capabilities.min_speed_hz,
                    m.capabilities.max_speed_hz,
                    "SCL 频率",
                )
                .err(),
            ];
            for e in errs.into_iter().flatten() {
                widgets::error(ui, &e);
            }
            m.saved.foot(ui, &mut m.cfg);
            widgets::note(ui, "连接变化后，重新保存配置。");
        },
    );
    if m.cfg != before {
        m.stop_loop("配置已修改，循环已停止");
    }
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
        if m.offline {
            "当前连接：离线模型 · 记录保留来源标记"
        } else {
            "记录按 USB / 离线标记 · 不含演示行"
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
            let write_edit = ui.add(
                egui::TextEdit::multiline(&mut m.write_hex)
                    .hint_text("00 11 22 33")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            if write_edit.changed() {
                m.stop_loop("写入数据已修改，循环已停止");
            }
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
                    widgets::muted(
                        ui,
                        if m.offline {
                            "每次执行一笔离线模型事务"
                        } else {
                            "每次发送一笔 USB I2C 请求"
                        },
                    );
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
            widgets::muted(
                ui,
                &format!(
                    "{} · CH{} · 请求 {} Hz{}",
                    if r.offline { "离线模型" } else { "USB" },
                    r.port,
                    r.speed_hz,
                    if r.offline {
                        " · 未测量实际频率"
                    } else {
                        ""
                    }
                ),
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
                widgets::muted(ui, "CH0 / CH1 · 来源 / 时间 / HEX");
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
                widths: &[84.0, 120.0, 50.0, 0.0, 0.0, 64.0],
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
                                    (
                                        format!(
                                            "{} CH{} {}",
                                            if r.offline { "离线" } else { "USB" },
                                            r.port,
                                            r.op.label()
                                        ),
                                        t.fg,
                                        false,
                                    ),
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
                port: 0,
                speed_hz: 100_000,
                offline: false,
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
            port: 0,
            speed_hz: 100_000,
            offline: false,
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
            port: 0,
            speed_hz: 100_000,
            offline: false,
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

    #[test]
    fn stop_loop_disarms_switch_even_before_first_transfer() {
        let mut m = Master {
            looping: true,
            ..Master::default()
        };
        m.stop_loop("设备已断开，循环已停止");
        assert!(!m.looping && !m.looping());
        assert_eq!(m.loop_note.as_deref(), Some("设备已断开，循环已停止"));
        assert_eq!(m.loop_due(Duration::from_secs(1), true, false), None);
    }

    #[test]
    fn failed_transfer_disarms_switch_even_without_a_scheduled_loop() {
        let mut m = Master {
            looping: true,
            ..Master::default()
        };
        let transfer = m.transfer(Op::Read).unwrap();
        m.record(Duration::ZERO, transfer, Err("I2C Nack".into()));
        assert!(!m.looping && !m.looping());
        assert_eq!(m.loop_note.as_deref(), Some("传输失败，循环已停止"));
    }
    #[test]
    fn connection_capabilities_gate_port_rate_and_preserve_record_source() {
        let mut m = Master::default();
        let offline = usb_i2c::device::Device::offline();
        m.set_connection(offline.capabilities(), true);
        m.cfg.port = 1;
        m.cfg.speed_hz = "1000000".into();
        m.saved.save(&m.cfg, Duration::ZERO);
        let saved_offline = m.cfg.clone();
        assert!(!m.saved.dirty(&saved_offline));
        let t = m.transfer(Op::Read).unwrap();
        assert_eq!((t.port, t.speed_hz, t.offline), (1, 1_000_000, true));
        let record = m.record(Duration::ZERO, t, Ok(vec![0; 16]));
        assert!(record.summary().contains("离线 CH1"));
        m.cfg.speed_hz = "1000001".into();
        assert!(m.transfer(Op::Read).is_err());
        m.set_connection(&Capabilities::legacy(), false);
        assert!(
            m.saved.dirty(&saved_offline),
            "old connection snapshot must be cleared"
        );
        assert_eq!((m.cfg.port, m.cfg.speed_hz.as_str()), (0, "100000"));
        assert!(!m.transfer(Op::Read).unwrap().offline);
        assert!(m.shown().unwrap().summary().contains("离线 CH1"));
        m.cfg.port = 1;
        assert!(m.transfer(Op::Read).is_err());
        m.cfg.port = 0;
        m.cfg.speed_hz = "400000".into();
        assert!(m.transfer(Op::Read).is_err());
    }
}
