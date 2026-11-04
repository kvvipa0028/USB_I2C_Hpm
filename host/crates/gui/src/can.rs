//! CAN page (报文监控). Config on the left; the frame table with the picked
//! frame's bytes beside it, and the send editor under both. Runs in the
//! window only: 开始接收 starts a generator of demo frames, 发送 adds a TX
//! row, and nothing goes over USB (the bridge has no CAN command).

use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use eframe::egui;

use crate::frame::{byte_grid, gap_text, time_text};
use crate::i2c::parse::parse_bytes;
use crate::local::{self, hex_upper, label, parse_hex, parse_num, Saved, Ticker, LOG_MAX, TICK};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const CHANNELS: [(u8, &str); 2] = [(1, "CAN1"), (2, "CAN2")];
const RATES: [(u32, &str); 4] = [
    (125, "125 kbit/s"),
    (250, "250 kbit/s"),
    (500, "500 kbit/s"),
    (1000, "1000 kbit/s"),
];
const PROTOCOLS: [(u8, &str); 1] = [(0, "经典 CAN")];
/// Work mode: true is listen-only.
const MODES: [(bool, &str); 2] = [(false, "正常模式"), (true, "只听模式")];
/// Send format: true is extended.
const FORMATS: [(bool, &str); 2] = [(false, "标准帧 · 11 bit"), (true, "扩展帧 · 29 bit")];
/// Time column: true is the wall clock.
const TIMES: [(bool, &str); 2] = [(false, "相对时间"), (true, "绝对时间 · UTC")];

const STD_MAX: u32 = 0x7ff;
/// Page width from which the detail card gets its own column.
pub const WIDE: f32 = 1100.0;
const EXT_MAX: u32 = 0x1fff_ffff;

#[derive(Clone, PartialEq, Debug)]
struct Config {
    channel: u8,
    rate: u32,
    protocol: u8,
    listen_only: bool,
    ext: bool,
    filter_id: String,
    filter_mask: String,
    period: String,
    wall: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 1,
            rate: 500,
            protocol: 0,
            listen_only: false,
            ext: false,
            filter_id: "0x000".into(),
            filter_mask: "0x000".into(),
            period: "100".into(),
            wall: false,
        }
    }
}

impl Config {
    /// Receive filter (id, mask); a field that does not parse counts as 0.
    fn filter(&self) -> (u32, u32) {
        (
            parse_hex(&self.filter_id, EXT_MAX, "").unwrap_or(0),
            parse_hex(&self.filter_mask, EXT_MAX, "").unwrap_or(0),
        )
    }

    fn accepts(&self, id: u32) -> bool {
        let (f, m) = self.filter();
        id & m == f & m
    }

    fn period(&self) -> Option<Duration> {
        parse_num(&self.period, 10, 60_000, "")
            .ok()
            .map(|ms| Duration::from_millis(ms.into()))
    }

    fn errors(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(parse_hex(&self.filter_id, EXT_MAX, "过滤 ID ").err());
        out.extend(parse_hex(&self.filter_mask, EXT_MAX, "过滤掩码").err());
        out.extend(parse_num(&self.period, 10, 60_000, "循环周期").err());
        out
    }
}

struct Frame {
    n: u64,
    at: Duration,
    tx: bool,
    id: u32,
    ext: bool,
    data: Vec<u8>,
    /// Since the last frame with the same ID.
    gap: Option<Duration>,
}

pub struct Can {
    cfg: Config,
    saved: Saved<Config>,
    advanced: bool,
    receiving: bool,
    rx_ticker: Ticker,
    /// Generator ticks so far; picks which demo IDs are due.
    rx_k: u64,
    send_id: String,
    send_data: String,
    looping: bool,
    tx_ticker: Ticker,
    send_err: Option<String>,
    /// Oldest first.
    log: VecDeque<Frame>,
    next_n: u64,
    rx: u64,
    tx: u64,
    selected: Option<u64>,
    byte: usize,
    /// While set, the table shows frames up to this number only.
    frozen: Option<u64>,
    opened: SystemTime,
}

/// The design's opening rows: two cyclic RX IDs and one TX.
const SEED: [(u64, bool, u32, [u8; 8]); 8] = [
    (100, false, 0x180, [0x3c, 0, 0x78, 0, 1, 0, 0, 0xa5]),
    (102, false, 0x280, [0xe8, 3, 0, 0, 0x64, 0, 0, 1]),
    (104, true, 0x300, [1, 2, 3, 4, 5, 6, 7, 8]),
    (110, false, 0x180, [0x3d, 0, 0x78, 0, 1, 0, 0, 0xa6]),
    (120, false, 0x180, [0x3e, 0, 0x79, 0, 1, 0, 0, 0xa7]),
    (122, false, 0x280, [0xe9, 3, 0, 0, 0x64, 0, 0, 2]),
    (130, false, 0x180, [0x3f, 0, 0x79, 0, 1, 0, 0, 0xa8]),
    (140, false, 0x180, [0x40, 0, 0x7a, 0, 1, 0, 0, 0xa9]),
];

impl Default for Can {
    fn default() -> Self {
        let mut c = Self {
            cfg: Config::default(),
            saved: Saved::default(),
            advanced: false,
            receiving: false,
            rx_ticker: Ticker::new(Duration::ZERO),
            rx_k: 0,
            send_id: "0x300".into(),
            send_data: "01 02 03 04 05 06 07 08".into(),
            looping: false,
            tx_ticker: Ticker::new(Duration::ZERO),
            send_err: None,
            log: VecDeque::new(),
            next_n: 1,
            rx: 0,
            tx: 0,
            selected: Some(1),
            byte: 0,
            frozen: None,
            opened: SystemTime::now(),
        };
        for (ms, tx, id, data) in SEED {
            c.push(Duration::from_millis(ms), tx, id, false, data.to_vec());
        }
        c
    }
}

impl Can {
    pub fn running(&self) -> bool {
        self.receiving || self.looping
    }

    fn push(&mut self, at: Duration, tx: bool, id: u32, ext: bool, data: Vec<u8>) {
        let gap = self
            .log
            .iter()
            .rev()
            .find(|f| f.id == id)
            .map(|f| at.saturating_sub(f.at));
        if tx {
            self.tx += 1;
        } else {
            self.rx += 1;
        }
        self.log.push_back(Frame {
            n: self.next_n,
            at,
            tx,
            id,
            ext,
            data,
            gap,
        });
        self.next_n += 1;
        while self.log.len() > LOG_MAX {
            self.log.pop_front();
        }
    }

    /// One generator step: 0x180 every 200 ms, 0x280 every 500 ms, and an
    /// extended 0x18FF50E5 every second, each passed through the filter.
    fn generate(&mut self, at: Duration) {
        self.rx_k += 1;
        let k = self.rx_k;
        let s = (k / 4) as u8;
        let mut due = Vec::new();
        if k.is_multiple_of(4) {
            let d = [
                0x40u8.wrapping_add(s),
                0,
                0x7a_u8.wrapping_add(s / 4),
                0,
                1,
                0,
                0,
                0xa9_u8.wrapping_add(s),
            ];
            due.push((0, 0x180, false, d.to_vec()));
        }
        if k.is_multiple_of(10) {
            let s = (k / 10) as u8;
            let d = [
                0xe9_u8.wrapping_add(s),
                3,
                0,
                0,
                0x64,
                0,
                0,
                2_u8.wrapping_add(s),
            ];
            due.push((2, 0x280, false, d.to_vec()));
        }
        if k.is_multiple_of(20) {
            let s = (k / 20) as u8;
            due.push((
                4,
                0x18ff_50e5,
                true,
                vec![s, 0x10, 0x27, 0, 0, 0xff, 0xff, 0xff],
            ));
        }
        // IDs due on the same tick follow each other 2 ms apart
        for (ms, id, ext, data) in due {
            if self.cfg.accepts(id) {
                self.push(at + Duration::from_millis(ms), false, id, ext, data);
            }
        }
    }

    fn editor_frame(&self) -> Result<(u32, Vec<u8>), String> {
        let max = if self.cfg.ext { EXT_MAX } else { STD_MAX };
        let id = parse_hex(&self.send_id, max, "帧 ID ")?;
        let data = parse_bytes(&self.send_data)?;
        if data.len() > 8 {
            return Err("经典 CAN 一帧最多 8 字节".into());
        }
        Ok((id, data))
    }

    fn send(&mut self, at: Duration) -> Result<(), String> {
        if self.cfg.listen_only {
            return Err("只听模式不发送".into());
        }
        let (id, data) = self.editor_frame()?;
        self.push(at, true, id, self.cfg.ext, data);
        if self.frozen.is_none() {
            self.selected = Some(self.next_n - 1);
        }
        Ok(())
    }

    /// Run the generators up to `now`; true while one is running.
    pub fn tick(&mut self, now: Duration) -> bool {
        if self.receiving {
            for at in self.rx_ticker.take(now, TICK, 40) {
                self.generate(at);
            }
        }
        if self.looping {
            match self.cfg.period() {
                Some(p) if !self.cfg.listen_only => {
                    for at in self.tx_ticker.take(now, p, 20) {
                        if let Err(e) = self.send(at) {
                            self.send_err = Some(e);
                            self.looping = false;
                            break;
                        }
                    }
                }
                _ => self.looping = false,
            }
        }
        self.running()
    }

    fn shown(&self) -> Option<&Frame> {
        let n = self.selected?;
        self.log.iter().find(|f| f.n == n)
    }
}

pub fn page(ui: &mut egui::Ui, c: &mut Can, now: Duration) {
    let badge = local::badge(c.receiving.then_some("接收中 · 演示"));
    widgets::page_head(
        ui,
        "CAN 总线监视",
        Some(badge),
        "筛选报文、配置发送任务并检查通信状态。以下为演示数据，不经 USB。",
        |ui| {
            let text = if c.receiving {
                "暂停接收"
            } else {
                "开始接收"
            };
            if widgets::ink_button(ui, true, text).clicked() {
                c.receiving = !c.receiving;
                c.rx_ticker = Ticker::new(now);
            }
            c.saved.button(ui, &c.cfg, now);
        },
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // 报文详情 sits beside the table on a wide page, under the
            // config card otherwise
            let wide = ui.available_width() >= WIDE;
            let left = |ui: &mut egui::Ui, c: &mut Can| {
                config(ui, c);
                if !wide {
                    ui.add_space(14.0);
                    detail(ui, c);
                }
            };
            widgets::work_columns(ui, c, 264.0, 760.0, left, |ui, c| {
                summary(ui, c);
                ui.add_space(12.0);
                if wide {
                    widgets::with_side(ui, c, 230.0, 700.0, monitor, detail);
                } else {
                    monitor(ui, c);
                }
                ui.add_space(14.0);
                compose(ui, c, now);
            });
        });
}

fn config(ui: &mut egui::Ui, c: &mut Can) {
    let channel = label(&CHANNELS, c.cfg.channel);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, channel, Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let g = &mut c.cfg;
            widgets::field_row(ui, "硬件通道", false, |ui| {
                widgets::pick(ui, &mut g.channel, &CHANNELS);
            });
            widgets::field_row(ui, "总线速率", false, |ui| {
                widgets::pick(ui, &mut g.rate, &RATES);
            });
            widgets::field_row(ui, "协议模式", false, |ui| {
                widgets::pick(ui, &mut g.protocol, &PROTOCOLS);
            });
            widgets::field_row(ui, "工作模式", false, |ui| {
                widgets::pick(ui, &mut g.listen_only, &MODES);
            });
            widgets::field_row(ui, "发送帧格式", false, |ui| {
                widgets::pick(ui, &mut g.ext, &FORMATS);
            });
            widgets::disclosure(ui, &mut c.advanced, "高级参数", 4);
            widgets::hairline(ui);
            if c.advanced {
                widgets::field_row(ui, "过滤 ID", false, |ui| {
                    widgets::unit_edit(ui, &mut g.filter_id, "0x000", "");
                });
                widgets::field_row(ui, "过滤掩码", false, |ui| {
                    widgets::unit_edit(ui, &mut g.filter_mask, "0x000", "");
                });
                widgets::field_row(ui, "循环周期", false, |ui| {
                    widgets::unit_edit(ui, &mut g.period, "100", "ms");
                });
                widgets::field_row(ui, "时间显示", true, |ui| {
                    widgets::pick(ui, &mut g.wall, &TIMES);
                });
            }
            for e in g.errors() {
                widgets::error(ui, &e);
            }
            c.saved.foot(ui, &mut c.cfg);
        },
    );
    ui.add_space(8.0);
    widgets::note(ui, "CAN FD 为可选扩展，仅在硬件支持时开放。");
    widgets::note(
        ui,
        "当前固件没有 CAN 命令：报文由本窗口生成，发送只记入本页。",
    );
}

fn summary(ui: &mut egui::Ui, c: &Can) {
    let (f, m) = c.cfg.filter();
    let filter = if m == 0 {
        "全部".to_owned()
    } else {
        format!("0x{f:X} / 0x{m:X}")
    };
    widgets::summary_strip(
        ui,
        &[
            ("接收帧", c.rx.to_string()),
            ("发送帧", c.tx.to_string()),
            ("错误帧", "0".into()),
            ("过滤", filter),
        ],
        "演示数据",
    );
}

fn monitor(ui: &mut egui::Ui, c: &mut Can) {
    let t = theme::of(ui);
    let mut pick = None;
    widgets::panel(
        ui,
        "报文监视",
        |ui| {
            widgets::badge(ui, "演示数据", Tone::Neutral);
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::muted(
                    ui,
                    &format!(
                        "{} · HEX · {}",
                        label(&CHANNELS, c.cfg.channel),
                        label(&RATES, c.cfg.rate)
                    ),
                );
                widgets::trailing(ui, 27.0, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::quiet_button(ui, !c.log.is_empty(), "清空").clicked() {
                        c.log.clear();
                        c.selected = None;
                        c.frozen = None;
                    }
                    let text = if c.frozen.is_some() {
                        "继续显示"
                    } else {
                        "暂停显示"
                    };
                    if widgets::quiet_button(ui, true, text).clicked() {
                        c.frozen = match c.frozen {
                            Some(_) => None,
                            None => Some(c.next_n - 1),
                        };
                    }
                });
            });
            let table = Table {
                widths: &[92.0, 40.0, 88.0, 40.0, 36.0, 0.0, 66.0],
            };
            let time = if c.cfg.wall {
                "时间 · UTC"
            } else {
                "时间"
            };
            table.header(
                ui,
                &[time, "方向", "ID", "类型", "长度", "数据 / HEX", "间隔"],
            );
            let shown: Vec<&Frame> = c
                .log
                .iter()
                .filter(|f| c.frozen.is_none_or(|n| f.n <= n))
                .collect();
            if shown.is_empty() {
                ui.label(egui::RichText::new("暂无报文。点开始接收生成演示报文。").color(t.muted));
            }
            egui::ScrollArea::vertical()
                .id_salt("can_log")
                .max_height(292.0)
                .auto_shrink([false, true])
                .stick_to_bottom(c.frozen.is_none())
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for f in &shown {
                        let id = if f.ext {
                            format!("0x{:08X}", f.id)
                        } else {
                            format!("0x{:03X}", f.id)
                        };
                        let cells = [
                            (time_text(f.at, c.opened, c.cfg.wall), t.muted, true),
                            (
                                dir(f.tx).to_owned(),
                                if f.tx { t.accent } else { t.green },
                                true,
                            ),
                            (id, t.fg, true),
                            ((if f.ext { "EXT" } else { "STD" }).to_owned(), t.fg2, true),
                            (f.data.len().to_string(), t.fg, true),
                            (hex_upper(&f.data), t.fg, true),
                            (gap_text(f.gap), t.muted, true),
                        ];
                        if table.row(ui, &cells, c.selected == Some(f.n)).clicked() {
                            pick = Some(f.n);
                        }
                    }
                });
            let left = if c.frozen.is_some() {
                format!("已暂停显示 · 共 {} 条，接收不受影响", c.log.len())
            } else {
                format!("{} 条报文 · 自动跟随", shown.len())
            };
            widgets::table_foot(ui, &left, &format!("点记录查看详情 · 最多 {LOG_MAX} 条"));
        },
    );
    if pick.is_some() {
        c.selected = pick;
    }
}

fn dir(tx: bool) -> &'static str {
    if tx {
        "TX"
    } else {
        "RX"
    }
}

fn detail(ui: &mut egui::Ui, c: &mut Can) {
    let t = theme::of(ui);
    let mut byte = c.byte;
    widgets::panel(
        ui,
        "报文详情",
        |_| {},
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let Some(f) = c.shown() else {
                widgets::muted(ui, "点选一条报文查看字节。");
                return;
            };
            let id = if f.ext {
                format!("0x{:08X}", f.id)
            } else {
                format!("0x{:03X}", f.id)
            };
            widgets::kv(ui, "记录", &format!("#{:03}", f.n), t.fg, false);
            widgets::kv(ui, "ID", &id, t.fg, false);
            widgets::kv(
                ui,
                "方向",
                dir(f.tx),
                if f.tx { t.accent } else { t.green },
                false,
            );
            widgets::kv(ui, "长度", &format!("{} B", f.data.len()), t.fg, false);
            widgets::kv(ui, "帧格式", if f.ext { "EXT" } else { "STD" }, t.fg, false);
            widgets::kv(ui, "间隔", &gap_text(f.gap), t.fg, true);
            ui.add_space(6.0);
            byte_grid(ui, &f.data, &mut byte);
        },
    );
    c.byte = byte;
}

fn compose(ui: &mut egui::Ui, c: &mut Can, now: Duration) {
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
                            egui::TextEdit::singleline(&mut c.send_id)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(124.0),
                        )
                        .changed()
                    {
                        c.send_err = None;
                    }
                    let kind = if c.cfg.ext {
                        "扩展数据帧"
                    } else {
                        "标准数据帧"
                    };
                    widgets::badge(ui, kind, Tone::Neutral);
                });
                let right_w = 150.0;
                let mid = ui.available_width() - right_w - 8.0;
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut c.send_data)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(mid)
                            .desired_rows(3),
                    )
                    .changed()
                {
                    c.send_err = None;
                }
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    let period = c
                        .cfg
                        .period()
                        .map_or("—".into(), |p| format!("循环 · {} ms", p.as_millis()));
                    let can_send = !c.cfg.listen_only;
                    if widgets::check(ui, &mut c.looping, &period).changed() {
                        c.tx_ticker = Ticker::new(now);
                        if c.looping && !can_send {
                            c.looping = false;
                        }
                    }
                    ui.add_space(6.0);
                    let resp = widgets::ink_button(ui, can_send, "发送")
                        .on_disabled_hover_text("只听模式不发送");
                    if resp.clicked() {
                        c.send_err = c.send(now).err();
                    }
                });
            });
            let problem = c.send_err.clone().or_else(|| c.editor_frame().err());
            match problem {
                Some(e) => widgets::error(ui, &e),
                None => {
                    ui.label(
                        egui::RichText::new("发送只在本页追加一条 TX 记录，不经 USB。")
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
    fn seed_matches_the_design() {
        let c = Can::default();
        assert_eq!((c.rx, c.tx, c.log.len()), (7, 1, 8));
        // the third 0x180 follows the second by 10 ms
        assert_eq!(c.log[4].gap, Some(Duration::from_millis(10)));
        assert_eq!(c.log[0].gap, None);
    }

    #[test]
    fn generator_follows_filter_and_cap() {
        let mut c = Can {
            receiving: true,
            ..Can::default()
        };
        c.cfg.filter_id = "0x280".into();
        c.cfg.filter_mask = "0x7FF".into();
        for i in 1..=20 {
            c.tick(Duration::from_millis(50 * i));
        }
        // 20 ticks: two 0x280 frames; 0x180 and the extended ID are filtered
        assert_eq!(c.rx, 9);
        assert!(c.log.iter().skip(8).all(|f| f.id == 0x280));
        c.cfg.filter_mask = "0".into();
        for i in 21..=2000 {
            c.tick(Duration::from_millis(50 * i));
        }
        assert_eq!(c.log.len(), LOG_MAX);
    }

    #[test]
    fn send_checks_id_length_and_mode() {
        let mut c = Can::default();
        c.send(Duration::ZERO).unwrap();
        assert_eq!(c.tx, 2);
        c.send_id = "0x800".into();
        assert!(c.send(Duration::ZERO).is_err());
        c.cfg.ext = true;
        c.send(Duration::ZERO).unwrap();
        c.send_data = "00 ".repeat(9);
        assert_eq!(
            c.send(Duration::ZERO).unwrap_err(),
            "经典 CAN 一帧最多 8 字节"
        );
        c.cfg.listen_only = true;
        assert_eq!(c.send(Duration::ZERO).unwrap_err(), "只听模式不发送");
    }
}
