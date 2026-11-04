//! LIN page. Same layout as CAN: config on the left, the frame table with
//! the picked frame's bytes beside it, the send editor below; the table
//! card switches to the schedule (调度表) in place. Runs in the window only:
//! 启动调度 walks the applied schedule and logs a demo frame per slot,
//! with the real PID and checksum; nothing goes over USB.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::frame::byte_grid;
use crate::i2c::parse::{clock, parse_bytes};
use crate::local::{self, hex_upper, label, parse_hex, parse_num, Saved, LOG_MAX};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const CHANNELS: [(u8, &str); 2] = [(1, "LIN1"), (2, "LIN2")];
const ROLES: [(Role, &str); 3] = [
    (Role::Master, "主节点"),
    (Role::Slave, "从节点（需支持）"),
    (Role::Monitor, "监听（需支持）"),
];
const BAUDS: [(u32, &str); 2] = [(9600, "9600 bit/s"), (19200, "19200 bit/s")];
const LENS: [(u8, &str); 8] = [
    (1, "1 B"),
    (2, "2 B"),
    (3, "3 B"),
    (4, "4 B"),
    (5, "5 B"),
    (6, "6 B"),
    (7, "7 B"),
    (8, "8 B"),
];
/// Checksum: true is enhanced (covers the PID).
const CHECKSUMS: [(bool, &str); 2] = [(false, "经典型"), (true, "增强型")];
/// Who publishes the response: true is the master.
const PUBLISHERS: [(bool, &str); 2] = [(true, "主节点"), (false, "从节点")];
const ID_MAX: u32 = 0x3f;
/// More than this behind (the window was hidden) and the schedule skips ahead.
const CATCH_UP: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Master,
    Slave,
    Monitor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Frames,
    Schedule,
}

/// Protected ID: the 6-bit ID with its two parity bits.
pub fn pid(id: u8) -> u8 {
    let b = |n: u8| (id >> n) & 1;
    let p0 = b(0) ^ b(1) ^ b(2) ^ b(4);
    let p1 = !(b(1) ^ b(3) ^ b(4) ^ b(5)) & 1;
    (id & 0x3f) | (p0 << 6) | (p1 << 7)
}

/// Inverted sum with carry. Enhanced covers the PID too, except for the
/// diagnostic IDs 0x3C and 0x3D, which always use the classic sum.
pub fn checksum(id: u8, data: &[u8], enhanced: bool) -> u8 {
    let mut sum: u16 = if enhanced && id < 0x3c {
        u16::from(pid(id))
    } else {
        0
    };
    for &b in data {
        sum += u16::from(b);
        if sum > 0xff {
            sum -= 0xff;
        }
    }
    !(sum as u8)
}

#[derive(Clone, PartialEq, Debug)]
struct Config {
    channel: u8,
    role: Role,
    baud: u32,
    id: String,
    len: u8,
    enhanced: bool,
    master_pub: bool,
    period: String,
    timeout: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 1,
            role: Role::Master,
            baud: 19200,
            id: "0x10".into(),
            len: 8,
            enhanced: true,
            master_pub: true,
            period: "20".into(),
            timeout: "10".into(),
        }
    }
}

impl Config {
    fn period(&self) -> Option<Duration> {
        parse_num(&self.period, 5, 10_000, "")
            .ok()
            .map(|ms| Duration::from_millis(ms.into()))
    }

    fn errors(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(parse_hex(&self.id, ID_MAX, "帧 ID ").err());
        out.extend(parse_num(&self.period, 5, 10_000, "调度周期").err());
        out.extend(parse_num(&self.timeout, 1, 1000, "帧超时").err());
        out
    }
}

/// One schedule row as edited.
#[derive(Clone, PartialEq, Debug)]
struct Slot {
    id: String,
    len: u8,
    master_pub: bool,
    ms: String,
}

/// One schedule row once applied.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Entry {
    id: u8,
    len: u8,
    master_pub: bool,
    ms: u32,
}

fn parse_slots(slots: &[Slot]) -> Result<Vec<Entry>, String> {
    if slots.is_empty() {
        return Err("调度表为空".into());
    }
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let at = |e: String| format!("槽位 {}：{e}", i + 1);
            Ok(Entry {
                id: parse_hex(&s.id, ID_MAX, "帧 ID ").map_err(at)? as u8,
                len: s.len,
                master_pub: s.master_pub,
                ms: parse_num(&s.ms, 1, 1000, "时隙").map_err(at)?,
            })
        })
        .collect()
}

struct Frame {
    n: u64,
    at: Duration,
    /// The master published the response.
    tx: bool,
    id: u8,
    /// Empty when no slave answered.
    data: Vec<u8>,
    checksum: Option<u8>,
}

pub struct Lin {
    cfg: Config,
    saved: Saved<Config>,
    advanced: bool,
    view: View,
    slots: Vec<Slot>,
    applied: Vec<Entry>,
    slot_err: Option<String>,
    scheduling: bool,
    next_at: Duration,
    slot_i: usize,
    round: u32,
    send_id: String,
    send_data: String,
    looping: bool,
    loop_next: Duration,
    send_err: Option<String>,
    /// Oldest first.
    log: VecDeque<Frame>,
    next_n: u64,
    selected: Option<u64>,
    byte: usize,
    frozen: Option<u64>,
}

impl Default for Lin {
    fn default() -> Self {
        let slot = Slot {
            id: "0x10".into(),
            len: 8,
            master_pub: true,
            ms: "20".into(),
        };
        let applied = parse_slots(std::slice::from_ref(&slot)).unwrap_or_default();
        let mut l = Self {
            cfg: Config::default(),
            saved: Saved::default(),
            advanced: false,
            view: View::Frames,
            slots: vec![slot],
            applied,
            slot_err: None,
            scheduling: false,
            next_at: Duration::ZERO,
            slot_i: 0,
            round: 0,
            send_id: "0x10".into(),
            send_data: "01 02 03 04 05 06 07 08".into(),
            looping: false,
            loop_next: Duration::ZERO,
            send_err: None,
            log: VecDeque::new(),
            next_n: 1,
            selected: Some(1),
            byte: 0,
            frozen: None,
        };
        // the design's opening rows: eight rounds of the one slot, 20 ms apart
        for r in 0..8u8 {
            let mut data = vec![1, 2, 3, 4, 5, 6, 7, 8];
            data[0] = r + 1;
            l.push(Duration::from_millis(u64::from(r) * 20), true, 0x10, data);
        }
        l
    }
}

impl Lin {
    pub fn running(&self) -> bool {
        self.scheduling || self.looping
    }

    fn push(&mut self, at: Duration, tx: bool, id: u8, data: Vec<u8>) {
        let checksum = (!data.is_empty()).then(|| checksum(id, &data, self.cfg.enhanced));
        self.log.push_back(Frame {
            n: self.next_n,
            at,
            tx,
            id,
            data,
            checksum,
        });
        self.next_n += 1;
        while self.log.len() > LOG_MAX {
            self.log.pop_front();
        }
    }

    /// The frame for one slot: the master's demo bytes count up per round;
    /// a slave slot gets no answer, as nothing is wired.
    fn run_slot(&mut self, at: Duration, e: Entry) {
        if e.master_pub {
            let mut data: Vec<u8> = (1..=e.len).collect();
            data[0] = (self.round as u8).wrapping_add(1);
            self.push(at, true, e.id, data);
        } else {
            self.push(at, false, e.id, Vec::new());
        }
    }

    fn start(&mut self, now: Duration) {
        self.scheduling = true;
        self.next_at = now;
        self.slot_i = 0;
    }

    fn editor_frame(&self) -> Result<(u8, Vec<u8>), String> {
        let id = parse_hex(&self.send_id, ID_MAX, "帧 ID ")? as u8;
        let data = parse_bytes(&self.send_data)?;
        if data.is_empty() || data.len() > 8 {
            return Err("LIN 一帧 1 到 8 字节".into());
        }
        Ok((id, data))
    }

    fn send(&mut self, at: Duration) -> Result<(), String> {
        if self.cfg.role != Role::Master {
            return Err("仅主节点可发送帧头".into());
        }
        let (id, data) = self.editor_frame()?;
        self.push(at, true, id, data);
        if self.frozen.is_none() {
            self.selected = Some(self.next_n - 1);
        }
        Ok(())
    }

    /// Run the schedule and the send loop up to `now`; true while running.
    pub fn tick(&mut self, now: Duration) -> bool {
        if self.scheduling {
            if self.applied.is_empty() || self.cfg.role != Role::Master {
                self.scheduling = false;
            } else {
                if now.saturating_sub(self.next_at) > CATCH_UP {
                    self.next_at = now;
                }
                while self.next_at <= now {
                    let e = self.applied[self.slot_i % self.applied.len()];
                    self.run_slot(self.next_at, e);
                    self.next_at += Duration::from_millis(u64::from(e.ms.max(1)));
                    self.slot_i += 1;
                    if self.slot_i >= self.applied.len() {
                        self.slot_i = 0;
                        self.round += 1;
                    }
                }
            }
        }
        if self.looping {
            match self.cfg.period() {
                Some(p) => {
                    if now.saturating_sub(self.loop_next) > CATCH_UP {
                        self.loop_next = now;
                    }
                    while self.loop_next <= now {
                        if let Err(e) = self.send(self.loop_next) {
                            self.send_err = Some(e);
                            self.looping = false;
                            break;
                        }
                        self.loop_next += p;
                    }
                }
                None => self.looping = false,
            }
        }
        self.running()
    }

    fn shown(&self) -> Option<&Frame> {
        let n = self.selected?;
        self.log.iter().find(|f| f.n == n)
    }

    fn cycle_ms(&self) -> u32 {
        self.applied.iter().map(|e| e.ms).sum()
    }
}

pub fn page(ui: &mut egui::Ui, l: &mut Lin, now: Duration) {
    let badge = local::badge(l.scheduling.then_some("调度中 · 演示"));
    widgets::page_head(
        ui,
        "LIN 总线调试",
        Some(badge),
        "配置节点角色与调度表，检查帧数据和校验。以下为演示数据，不经 USB。",
        |ui| {
            let master = l.cfg.role == Role::Master;
            let text = if l.scheduling {
                "停止调度"
            } else {
                "启动调度"
            };
            let can = l.scheduling || (master && !l.applied.is_empty());
            let why = if master {
                "调度表为空，先在调度表里应用"
            } else {
                "仅主节点可执行调度；从节点与监听需硬件支持"
            };
            if widgets::ink_button(ui, can, text)
                .on_disabled_hover_text(why)
                .clicked()
            {
                if l.scheduling {
                    l.scheduling = false;
                } else {
                    l.start(now);
                }
            }
            l.saved.button(ui, &l.cfg, now);
        },
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // as CAN: the side card gets a column only on a wide page
            let wide = ui.available_width() >= crate::can::WIDE;
            let left = |ui: &mut egui::Ui, l: &mut Lin| {
                config(ui, l);
                if !wide {
                    ui.add_space(14.0);
                    side(ui, l);
                }
            };
            widgets::work_columns(ui, l, 264.0, 760.0, left, |ui, l| {
                summary(ui, l);
                ui.add_space(12.0);
                if wide {
                    widgets::with_side(ui, l, 230.0, 700.0, monitor, side);
                } else {
                    monitor(ui, l);
                }
                ui.add_space(14.0);
                compose(ui, l, now);
            });
        });
}

fn config(ui: &mut egui::Ui, l: &mut Lin) {
    let channel = label(&CHANNELS, l.cfg.channel);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, channel, Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let g = &mut l.cfg;
            widgets::field_row(ui, "硬件通道", false, |ui| {
                widgets::pick(ui, &mut g.channel, &CHANNELS);
            });
            widgets::field_row(ui, "节点角色", false, |ui| {
                widgets::pick(ui, &mut g.role, &ROLES);
            });
            widgets::field_row(ui, "波特率", false, |ui| {
                widgets::pick(ui, &mut g.baud, &BAUDS);
            });
            widgets::field_row(ui, "帧 ID", false, |ui| {
                widgets::unit_edit(ui, &mut g.id, "0x10", "");
            });
            widgets::field_row(ui, "数据长度", false, |ui| {
                widgets::pick(ui, &mut g.len, &LENS);
            });
            widgets::disclosure(ui, &mut l.advanced, "高级参数", 4);
            widgets::hairline(ui);
            if l.advanced {
                widgets::field_row(ui, "校验方式", false, |ui| {
                    widgets::pick(ui, &mut g.enhanced, &CHECKSUMS);
                });
                widgets::field_row(ui, "响应发布方", false, |ui| {
                    widgets::pick(ui, &mut g.master_pub, &PUBLISHERS);
                });
                widgets::field_row(ui, "调度周期", false, |ui| {
                    widgets::unit_edit(ui, &mut g.period, "20", "ms");
                });
                widgets::field_row(ui, "帧超时", true, |ui| {
                    widgets::unit_edit(ui, &mut g.timeout, "10", "ms");
                });
            }
            for e in g.errors() {
                widgets::error(ui, &e);
            }
            l.saved.foot(ui, &mut l.cfg);
        },
    );
    ui.add_space(8.0);
    widgets::note(ui, "仅主节点模式可执行调度；角色能力由硬件决定。");
    widgets::note(
        ui,
        "帧 ID 与含奇偶校验的 PID 分开显示。当前固件没有 LIN 命令，帧由本窗口生成。",
    );
}

fn summary(ui: &mut egui::Ui, l: &Lin) {
    let silent = l.log.iter().filter(|f| f.data.is_empty()).count();
    widgets::summary_strip(
        ui,
        &[
            ("帧数", l.log.len().to_string()),
            ("调度周期", format!("{} ms", l.cycle_ms())),
            ("校验错误", "0".into()),
            ("无响应", silent.to_string()),
        ],
        "演示数据",
    );
}

fn monitor(ui: &mut egui::Ui, l: &mut Lin) {
    let mut view = l.view;
    widgets::tab_panel(
        ui,
        &mut view,
        &[(View::Frames, "帧监视"), (View::Schedule, "调度表")],
        |ui| {
            widgets::badge(ui, "演示数据", Tone::Neutral);
        },
        |ui| match l.view {
            View::Frames => frames(ui, l),
            View::Schedule => schedule(ui, l),
        },
    );
    l.view = view;
}

fn frames(ui: &mut egui::Ui, l: &mut Lin) {
    let t = theme::of(ui);
    let mut pick = None;
    ui.horizontal(|ui| {
        widgets::muted(
            ui,
            &format!(
                "{} · HEX · {}",
                label(&CHANNELS, l.cfg.channel),
                label(&BAUDS, l.cfg.baud)
            ),
        );
        widgets::trailing(ui, 27.0, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if widgets::quiet_button(ui, !l.log.is_empty(), "清空").clicked() {
                l.log.clear();
                l.selected = None;
                l.frozen = None;
            }
            let text = if l.frozen.is_some() {
                "继续显示"
            } else {
                "暂停显示"
            };
            if widgets::quiet_button(ui, true, text).clicked() {
                l.frozen = match l.frozen {
                    Some(_) => None,
                    None => Some(l.next_n - 1),
                };
            }
        });
    });
    let table = Table {
        widths: &[84.0, 40.0, 92.0, 36.0, 0.0, 44.0, 84.0],
    };
    table.header(
        ui,
        &[
            "时间",
            "方向",
            "ID / PID",
            "长度",
            "数据 / HEX",
            "校验",
            "结果",
        ],
    );
    let shown: Vec<&Frame> = l
        .log
        .iter()
        .filter(|f| l.frozen.is_none_or(|n| f.n <= n))
        .collect();
    if shown.is_empty() {
        ui.label(egui::RichText::new("暂无帧。点启动调度生成演示帧。").color(t.muted));
    }
    egui::ScrollArea::vertical()
        .id_salt("lin_log")
        .max_height(292.0)
        .auto_shrink([false, true])
        .stick_to_bottom(l.frozen.is_none())
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for f in &shown {
                let (result, color) = if f.data.is_empty() {
                    ("无响应 · 未接线", t.amber)
                } else {
                    ("示例正常", t.green)
                };
                let cells = [
                    (clock(f.at), t.muted, true),
                    (
                        (if f.tx { "TX" } else { "RX" }).to_owned(),
                        if f.tx { t.accent } else { t.green },
                        true,
                    ),
                    (format!("0x{:02X} / 0x{:02X}", f.id, pid(f.id)), t.fg, true),
                    (f.data.len().to_string(), t.fg, true),
                    (
                        if f.data.is_empty() {
                            "—".into()
                        } else {
                            hex_upper(&f.data)
                        },
                        t.fg,
                        true,
                    ),
                    (
                        f.checksum.map_or("—".into(), |c| format!("0x{c:02X}")),
                        t.fg2,
                        true,
                    ),
                    (result.to_owned(), color, false),
                ];
                if table.row(ui, &cells, l.selected == Some(f.n)).clicked() {
                    pick = Some(f.n);
                }
            }
        });
    let left = if l.frozen.is_some() {
        format!("已暂停显示 · 共 {} 条，调度不受影响", l.log.len())
    } else {
        format!("{} 条帧 · 自动跟随", shown.len())
    };
    widgets::table_foot(ui, &left, &format!("点记录查看详情 · 最多 {LOG_MAX} 条"));
    if pick.is_some() {
        l.selected = pick;
    }
}

fn schedule(ui: &mut egui::Ui, l: &mut Lin) {
    let t = theme::of(ui);
    let table = Table {
        widths: &[48.0, 0.0, 0.0, 0.0, 0.0, 44.0],
    };
    table.header(ui, &["槽位", "帧 ID", "长度", "发布方", "时隙 / ms", ""]);
    let mut remove = None;
    let mut changed = false;
    let w = ui.available_width();
    let col = ((w - 92.0) / 4.0).max(60.0) - 10.0;
    for (i, s) in l.slots.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.set_min_height(32.0);
            ui.add_sized(
                [40.0, 20.0],
                egui::Label::new(egui::RichText::new((i + 1).to_string()).monospace()),
            );
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut s.id)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(col),
                )
                .changed();
            ui.allocate_ui(egui::vec2(col, 26.0), |ui| {
                changed |= widgets::pick(ui, &mut s.len, &LENS);
            });
            ui.allocate_ui(egui::vec2(col, 26.0), |ui| {
                changed |= widgets::pick(ui, &mut s.master_pub, &PUBLISHERS);
            });
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut s.ms)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(col),
                )
                .changed();
            if widgets::quiet_button(ui, true, "删除").clicked() {
                remove = Some(i);
            }
        });
        widgets::hairline(ui);
    }
    if let Some(i) = remove {
        l.slots.remove(i);
        changed = true;
    }
    if changed {
        l.slot_err = None;
    }
    let draft = parse_slots(&l.slots);
    let pending = draft.as_ref().is_ok_and(|d| *d != l.applied);
    ui.horizontal(|ui| {
        if widgets::action_button(ui, l.slots.len() < 16, "＋ 增加槽位").clicked() {
            let id = l.cfg.id.clone();
            let ms = l.cfg.period.clone();
            l.slots.push(Slot {
                id,
                len: l.cfg.len,
                master_pub: l.cfg.master_pub,
                ms,
            });
        }
        if widgets::ink_button(ui, pending, "应用调度表").clicked() {
            match &draft {
                Ok(d) => {
                    l.applied = d.clone();
                    l.slot_i = 0;
                }
                Err(e) => l.slot_err = Some(e.clone()),
            }
        }
        let status = match &draft {
            Err(e) => (e.clone(), t.red),
            Ok(_) if pending => ("有修改，应用后生效".into(), t.amber),
            Ok(_) => ("已应用".into(), t.muted),
        };
        ui.label(egui::RichText::new(status.0).size(11.5).color(status.1));
    });
    if let Some(e) = &l.slot_err {
        widgets::error(ui, e);
    }
}

fn side(ui: &mut egui::Ui, l: &mut Lin) {
    match l.view {
        View::Frames => detail(ui, l),
        View::Schedule => info(ui, l),
    }
}

fn detail(ui: &mut egui::Ui, l: &mut Lin) {
    let t = theme::of(ui);
    let mut byte = l.byte;
    widgets::panel(
        ui,
        "报文详情",
        |_| {},
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let Some(f) = l.shown() else {
                widgets::muted(ui, "点选一帧查看字节。");
                return;
            };
            widgets::kv(ui, "记录", &format!("#{:03}", f.n), t.fg, false);
            widgets::kv(
                ui,
                "ID / PID",
                &format!("0x{:02X} / 0x{:02X}", f.id, pid(f.id)),
                t.fg,
                false,
            );
            let (d, dc) = if f.tx {
                ("TX", t.accent)
            } else {
                ("RX", t.green)
            };
            widgets::kv(ui, "方向", d, dc, false);
            widgets::kv(ui, "长度", &format!("{} B", f.data.len()), t.fg, false);
            let sum = f.checksum.map_or("—".into(), |c| format!("0x{c:02X}"));
            widgets::kv(ui, "校验和", &sum, t.fg, false);
            let (r, rc) = if f.data.is_empty() {
                ("无响应", t.amber)
            } else {
                ("示例正常", t.green)
            };
            widgets::kv(ui, "结果", r, rc, true);
            ui.add_space(6.0);
            byte_grid(ui, &f.data, &mut byte);
        },
    );
    l.byte = byte;
}

fn info(ui: &mut egui::Ui, l: &mut Lin) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "调度信息",
        |_| {},
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            widgets::kv(ui, "角色", label(&ROLES, l.cfg.role), t.fg, false);
            widgets::kv(ui, "槽位数", &l.applied.len().to_string(), t.fg, false);
            widgets::kv(ui, "周期", &format!("{} ms", l.cycle_ms()), t.fg, false);
            widgets::kv(ui, "校验", label(&CHECKSUMS, l.cfg.enhanced), t.fg, true);
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("按槽位顺序循环执行，收发结果保留在帧监视中。")
                        .size(11.5)
                        .color(t.fg2),
                )
                .wrap(),
            );
        },
    );
}

fn compose(ui: &mut egui::Ui, l: &mut Lin, now: Duration) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "发送编辑器",
        |ui| {
            widgets::muted(ui, "HEX");
        },
        |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(130.0);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    widgets::muted(ui, "帧 ID");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut l.send_id)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(124.0),
                        )
                        .changed()
                    {
                        l.send_err = None;
                    }
                    widgets::badge(ui, "单次事务", Tone::Neutral);
                });
                let right_w = 150.0;
                let mid = ui.available_width() - right_w - 8.0;
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut l.send_data)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(mid)
                            .desired_rows(3),
                    )
                    .changed()
                {
                    l.send_err = None;
                }
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    let master = l.cfg.role == Role::Master;
                    let period = l
                        .cfg
                        .period()
                        .map_or("循环".into(), |p| format!("循环 · {} ms", p.as_millis()));
                    if widgets::check(ui, &mut l.looping, &period).changed() {
                        l.loop_next = now;
                        if !master {
                            l.looping = false;
                        }
                    }
                    ui.add_space(6.0);
                    if widgets::ink_button(ui, master, "发送")
                        .on_disabled_hover_text("仅主节点可发送帧头")
                        .clicked()
                    {
                        l.send_err = l.send(now).err();
                    }
                });
            });
            match l.send_err.clone().or_else(|| l.editor_frame().err()) {
                Some(e) => widgets::error(ui, &e),
                None => {
                    ui.label(
                        egui::RichText::new("发送只在本页追加一帧，不经 USB。")
                            .size(11.5)
                            .color(t.muted),
                    );
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_and_checksum_match_the_spec() {
        assert_eq!(pid(0x10), 0x50);
        assert_eq!(pid(0x3c), 0x3c);
        assert_eq!(pid(0x00), 0x80);
        let data = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(checksum(0x10, &data, true), 0x8b);
        assert_eq!(checksum(0x10, &[2, 2, 3, 4, 5, 6, 7, 8], true), 0x8a);
        assert_eq!(checksum(0x10, &data, false), !36u8);
        // diagnostic frames stay classic; the carry wraps
        assert_eq!(checksum(0x3c, &data, true), checksum(0x3c, &data, false));
        assert_eq!(checksum(0x01, &[0xff, 0x02], false), !0x02u8);
    }

    #[test]
    fn schedule_runs_slots_in_order() {
        let mut l = Lin::default();
        l.log.clear();
        l.slots.push(Slot {
            id: "0x21".into(),
            len: 2,
            master_pub: false,
            ms: "30".into(),
        });
        l.applied = parse_slots(&l.slots).unwrap();
        l.start(Duration::ZERO);
        l.tick(Duration::from_millis(100));
        // 0, 20 (0x21), 50 (0x10), 70 (0x21), 100 (0x10)
        let ids: Vec<u8> = l.log.iter().map(|f| f.id).collect();
        assert_eq!(ids, vec![0x10, 0x21, 0x10, 0x21, 0x10]);
        assert!(l.log[1].data.is_empty() && l.log[1].checksum.is_none());
        assert_eq!(l.log[2].data[0], 2);
        l.cfg.role = Role::Slave;
        l.tick(Duration::from_millis(200));
        assert!(!l.scheduling);
        assert!(l.send(Duration::ZERO).is_err());
    }

    #[test]
    fn slots_check_their_fields() {
        let bad = Slot {
            id: "0x40".into(),
            len: 1,
            master_pub: true,
            ms: "10".into(),
        };
        assert!(parse_slots(&[bad]).unwrap_err().starts_with("槽位 1："));
        assert!(parse_slots(&[]).is_err());
    }
}
