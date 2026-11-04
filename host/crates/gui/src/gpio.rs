//! GPIO page. Pin settings on the left; the pins as cards grouped into
//! inputs and outputs, and a log of applied changes, on the right. Runs in
//! the window only: clicking an output card flips its draft level,
//! 应用引脚配置 takes the drafts as applied and logs what changed, and the
//! input levels are simulated. Nothing goes over USB.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::clock;
use crate::local::{self, label, parse_num, Saved, LOG_MAX};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const PINS: usize = 8;
const GROUPS: [(u8, &str); 1] = [(0, "PORT A")];
const PIN_NAMES: [(usize, &str); PINS] = [
    (0, "D0"),
    (1, "D1"),
    (2, "D2"),
    (3, "D3"),
    (4, "D4"),
    (5, "D5"),
    (6, "D6"),
    (7, "D7"),
];
/// Direction: true is output.
const DIRS: [(bool, &str); 2] = [(false, "输入"), (true, "输出")];
/// Output level: true is high.
const LEVELS: [(bool, &str); 2] = [(false, "低电平"), (true, "高电平")];
const PULLS: [(Pull, &str); 3] = [
    (Pull::None, "无"),
    (Pull::Up, "上拉（需支持）"),
    (Pull::Down, "下拉（需支持）"),
];
/// Event log: true logs every poll.
const EVENTS: [(bool, &str); 2] = [(false, "电平变化"), (true, "每次轮询")];
/// Pin view: true is the bitmap.
const VIEWS: [(bool, &str); 2] = [(false, "逐引脚"), (true, "二进制位图")];
/// Simulated input levels, bit n for Dn: D1 and D3 read high.
const INPUTS: u8 = 0b0000_1010;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pull {
    None,
    Up,
    Down,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Pin {
    output: bool,
    high: bool,
    pull: Pull,
}

#[derive(Clone, PartialEq, Debug)]
struct Config {
    group: u8,
    pins: [Pin; PINS],
    poll: String,
    every_poll: bool,
    bitmap: bool,
}

impl Default for Config {
    fn default() -> Self {
        // the design's split: D0–D3 inputs, D4–D7 outputs at H L H L
        let pins = std::array::from_fn(|i| Pin {
            output: i >= 4,
            high: i >= 4 && i % 2 == 0,
            pull: Pull::None,
        });
        Self {
            group: 0,
            pins,
            poll: "20".into(),
            every_poll: false,
            bitmap: false,
        }
    }
}

impl Config {
    /// What pin `i` reads back: its own level when it drives, the input otherwise.
    fn read(&self, i: usize) -> bool {
        let p = self.pins[i];
        if p.output {
            p.high
        } else {
            (INPUTS >> i) & 1 == 1
        }
    }

    /// Output levels as a bitmap, bit n for Dn; inputs count as 0.
    fn out_bits(&self) -> u8 {
        (0..PINS).fold(0, |acc, i| {
            let p = self.pins[i];
            acc | (u8::from(p.output && p.high) << i)
        })
    }
}

struct Entry {
    at: Duration,
    pin: usize,
    pin_cfg: Pin,
    event: String,
}

pub struct Gpio {
    draft: Config,
    applied: Config,
    saved: Saved<Config>,
    current: usize,
    advanced: bool,
    /// Newest first.
    log: VecDeque<Entry>,
}

impl Default for Gpio {
    fn default() -> Self {
        Self {
            draft: Config::default(),
            applied: Config::default(),
            saved: Saved::default(),
            current: 4,
            advanced: false,
            log: VecDeque::new(),
        }
    }
}

impl Gpio {
    fn pending(&self) -> Vec<usize> {
        (0..PINS)
            .filter(|&i| self.draft.pins[i] != self.applied.pins[i])
            .collect()
    }

    /// Take the drafts as applied and log one row per changed pin.
    fn apply(&mut self, at: Duration) -> usize {
        let changed = self.pending();
        for &i in &changed {
            let (old, new) = (self.applied.pins[i], self.draft.pins[i]);
            let mut what = Vec::new();
            if old.output != new.output {
                what.push(format!("方向 → {}", label(&DIRS, new.output)));
            }
            if new.output && (old.high != new.high || old.output != new.output) {
                what.push(format!("输出 → {}", label(&LEVELS, new.high)));
            }
            if old.pull != new.pull {
                what.push(format!("上下拉 → {}", label(&PULLS, new.pull)));
            }
            self.log.push_front(Entry {
                at,
                pin: i,
                pin_cfg: new,
                event: what.join("，"),
            });
        }
        self.log.truncate(LOG_MAX);
        self.applied = self.draft.clone();
        changed.len()
    }
}

pub fn page(ui: &mut egui::Ui, g: &mut Gpio, now: Duration) {
    widgets::page_head(
        ui,
        "GPIO 引脚控制",
        Some(local::badge(None)),
        "集中查看输入状态与输出设定，按组应用引脚配置。以下为演示数据，不经 USB。",
        |ui| {
            let n = g.pending().len();
            let text = if n > 0 {
                format!("应用引脚配置（{n}）")
            } else {
                "应用引脚配置".to_owned()
            };
            if widgets::ink_button(ui, n > 0, &text).clicked() {
                g.apply(now);
            }
            g.saved.button(ui, &g.draft, now);
        },
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::work_columns(ui, g, 264.0, 760.0, config, |ui, g| {
                summary(ui, g);
                ui.add_space(12.0);
                pins(ui, g);
                ui.add_space(14.0);
                log(ui, g);
            });
        });
}

fn config(ui: &mut egui::Ui, g: &mut Gpio) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, "GPIO", Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let c = &mut g.draft;
            widgets::field_row(ui, "引脚组", false, |ui| {
                widgets::pick(ui, &mut c.group, &GROUPS);
            });
            widgets::field_row(ui, "当前引脚", false, |ui| {
                widgets::pick(ui, &mut g.current, &PIN_NAMES);
            });
            let pin = &mut c.pins[g.current];
            widgets::field_row(ui, "方向", false, |ui| {
                widgets::pick(ui, &mut pin.output, &DIRS);
            });
            widgets::field_row(ui, "输出设定", false, |ui| {
                if pin.output {
                    widgets::pick(ui, &mut pin.high, &LEVELS);
                } else {
                    ui.label(egui::RichText::new("— 输入引脚").size(12.0).color(t.muted));
                }
            });
            widgets::field_row(ui, "上下拉", false, |ui| {
                widgets::pick(ui, &mut pin.pull, &PULLS);
            });
            widgets::disclosure(ui, &mut g.advanced, "高级参数", 3);
            widgets::hairline(ui);
            if g.advanced {
                widgets::field_row(ui, "轮询周期", false, |ui| {
                    widgets::unit_edit(ui, &mut c.poll, "20", "ms");
                });
                widgets::field_row(ui, "事件记录", false, |ui| {
                    widgets::pick(ui, &mut c.every_poll, &EVENTS);
                });
                widgets::field_row(ui, "显示格式", true, |ui| {
                    widgets::pick(ui, &mut c.bitmap, &VIEWS);
                });
                if let Err(e) = parse_num(&c.poll, 1, 10_000, "轮询周期") {
                    widgets::error(ui, &e);
                }
            }
            g.saved.foot(ui, &mut g.draft);
        },
    );
    ui.add_space(8.0);
    widgets::note(ui, "输出设定与读回电平分开显示，避免混淆。");
    widgets::note(
        ui,
        "示例引脚数量不代表设备能力；输入电平为模拟值。当前固件没有 GPIO 命令。",
    );
}

fn summary(ui: &mut egui::Ui, g: &Gpio) {
    let c = &g.applied;
    let outputs = c.pins.iter().filter(|p| p.output).count();
    let high = (0..PINS).filter(|&i| c.read(i)).count();
    widgets::summary_strip(
        ui,
        &[
            ("输入", format!("{} 路", PINS - outputs)),
            ("输出", format!("{outputs} 路")),
            ("读回高电平", format!("{high} 路")),
            ("输出位图", format!("0x{:02X}", c.out_bits())),
        ],
        "模拟电平",
    );
}

fn pins(ui: &mut egui::Ui, g: &mut Gpio) {
    let pending = g.pending().len();
    widgets::panel(
        ui,
        "引脚状态",
        |ui| {
            widgets::badge(ui, "模拟电平", Tone::Neutral);
        },
        |ui| {
            if g.draft.bitmap {
                bitmap(ui, g);
            } else {
                for output in [false, true] {
                    let idx: Vec<usize> = (0..PINS)
                        .filter(|&i| g.draft.pins[i].output == output)
                        .collect();
                    let name = if output { "输出" } else { "输入" };
                    widgets::group_title(ui, &format!("{name} · {} 路", idx.len()));
                    if idx.is_empty() {
                        widgets::muted(ui, "无");
                        continue;
                    }
                    for row in idx.chunks(4) {
                        ui.columns(4, |cols| {
                            for (col, &i) in cols.iter_mut().zip(row) {
                                card(col, g, i);
                            }
                        });
                    }
                }
            }
            let right = if pending > 0 {
                format!("{pending} 个引脚待应用")
            } else {
                "设定已全部应用".to_owned()
            };
            widgets::table_foot(ui, "输出卡片点击切换设定 · 读回在应用后更新", &right);
        },
    );
}

/// One pin: name, the big level, what it is, and the read-back voltage.
fn card(ui: &mut egui::Ui, g: &mut Gpio, i: usize) {
    let t = theme::of(ui);
    let pin = g.draft.pins[i];
    let applied = g.applied.pins[i];
    let changed = pin != applied;
    let read = g.applied.read(i);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), egui::Sense::click());
    let selected = g.current == i;
    let level = if pin.output { pin.high } else { read };
    let radius = egui::CornerRadius::same(10);
    let fill = if level { t.sel } else { t.card2 };
    let p = ui.painter();
    p.rect_filled(rect, radius, if resp.hovered() { t.pill } else { fill });
    let edge = if selected {
        egui::Stroke::new(1.5, t.accent)
    } else {
        egui::Stroke::new(1.0, t.line)
    };
    p.rect_stroke(rect, radius, edge, egui::StrokeKind::Inside);
    let x = rect.left() + 12.0;
    let dim = if pin.output { t.fg } else { t.fg2 };
    p.text(
        egui::pos2(x, rect.top() + 14.0),
        egui::Align2::LEFT_CENTER,
        format!("GPIO D{i}"),
        egui::FontId::proportional(12.0),
        t.muted,
    );
    p.text(
        egui::pos2(x, rect.top() + 38.0),
        egui::Align2::LEFT_CENTER,
        if level { "HIGH" } else { "LOW" },
        egui::FontId::monospace(19.0),
        dim,
    );
    let what = if pin.output {
        "输出设定 · 点击切换"
    } else {
        "输入读回 · 仅查看"
    };
    p.text(
        egui::pos2(x, rect.top() + 62.0),
        egui::Align2::LEFT_CENTER,
        what,
        egui::FontId::proportional(11.0),
        t.muted,
    );
    let volts = format!("读回 {}", if read { "3.3 V" } else { "0.0 V" });
    p.text(
        egui::pos2(x, rect.top() + 81.0),
        egui::Align2::LEFT_CENTER,
        volts,
        egui::FontId::monospace(10.5),
        t.green,
    );
    if changed {
        p.text(
            egui::pos2(rect.right() - 10.0, rect.top() + 14.0),
            egui::Align2::RIGHT_CENTER,
            "待应用",
            egui::FontId::proportional(10.5),
            t.amber,
        );
    }
    if resp.clicked() {
        g.current = i;
        if pin.output {
            g.draft.pins[i].high = !pin.high;
        }
    }
}

/// D7…D0 as a strip of bits: draft output levels, then what reads back.
fn bitmap(ui: &mut egui::Ui, g: &mut Gpio) {
    let t = theme::of(ui);
    for (name, bits) in [
        ("输出设定", g.draft.out_bits()),
        (
            "读回电平",
            (0..PINS).fold(0u8, |a, i| a | (u8::from(g.applied.read(i)) << i)),
        ),
    ] {
        ui.horizontal(|ui| {
            ui.add_sized(
                [64.0, 24.0],
                egui::Label::new(egui::RichText::new(name).size(12.0).color(t.muted)),
            );
            for i in (0..PINS).rev() {
                let on = (bits >> i) & 1 == 1;
                let output = g.draft.pins[i].output;
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                let (fg, bg) = if on {
                    (t.accent, t.accent_soft)
                } else {
                    (t.fg2, t.card2)
                };
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(6), bg);
                if g.current == i {
                    ui.painter().rect_stroke(
                        rect,
                        egui::CornerRadius::same(6),
                        egui::Stroke::new(1.0, t.accent),
                        egui::StrokeKind::Inside,
                    );
                }
                ui.painter().text(
                    rect.center() - egui::vec2(0.0, 7.0),
                    egui::Align2::CENTER_CENTER,
                    format!("D{i}"),
                    egui::FontId::proportional(10.0),
                    t.muted,
                );
                ui.painter().text(
                    rect.center() + egui::vec2(0.0, 7.0),
                    egui::Align2::CENTER_CENTER,
                    if output || name == "读回电平" {
                        u8::from(on).to_string()
                    } else {
                        "—".into()
                    },
                    egui::FontId::monospace(12.0),
                    fg,
                );
                if resp.clicked() {
                    g.current = i;
                    if name == "输出设定" && output {
                        g.draft.pins[i].high = !g.draft.pins[i].high;
                    }
                }
            }
            ui.label(
                egui::RichText::new(format!("0x{bits:02X}"))
                    .monospace()
                    .color(t.fg),
            );
        });
    }
}

fn log(ui: &mut egui::Ui, g: &mut Gpio) {
    let t = theme::of(ui);
    let count = g.log.len();
    let mut clear = false;
    widgets::panel(
        ui,
        "应用记录",
        |ui| {
            if widgets::quiet_button(ui, count > 0, "清空").clicked() {
                clear = true;
            }
            widgets::muted(ui, &format!("{count} 条 · 本地日志"));
        },
        |ui| {
            if g.log.is_empty() {
                ui.label(
                    egui::RichText::new("还没有记录。改动引脚后点应用引脚配置，记录显示在这里。")
                        .color(t.muted),
                );
                return;
            }
            let table = Table {
                widths: &[84.0, 44.0, 52.0, 70.0, 0.0],
            };
            table.header(ui, &["时间", "引脚", "方向", "输出设定", "事件"]);
            egui::ScrollArea::vertical()
                .id_salt("gpio_log")
                .max_height(200.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for e in &g.log {
                        let out = if e.pin_cfg.output {
                            label(&LEVELS, e.pin_cfg.high)
                        } else {
                            "—"
                        };
                        let cells = [
                            (clock(e.at), t.muted, true),
                            (format!("D{}", e.pin), t.fg, true),
                            (label(&DIRS, e.pin_cfg.output).to_owned(), t.fg, false),
                            (out.to_owned(), t.fg2, false),
                            (e.event.clone(), t.fg2, false),
                        ];
                        table.row(ui, &cells, false);
                    }
                });
            widgets::table_foot(ui, "最新在上", &format!("最多 {LOG_MAX} 条"));
        },
    );
    if clear {
        g.log.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_the_design() {
        let c = Config::default();
        assert_eq!(c.out_bits(), 0b0101_0000);
        let read: Vec<bool> = (0..PINS).map(|i| c.read(i)).collect();
        assert_eq!(read, [false, true, false, true, true, false, true, false]);
    }

    #[test]
    fn apply_logs_each_changed_pin() {
        let mut g = Gpio::default();
        assert!(g.pending().is_empty());
        g.draft.pins[5].high = true;
        g.draft.pins[0].output = true;
        assert_eq!(g.pending(), vec![0, 5]);
        // the read-back follows only once applied
        assert!(!g.applied.read(5));
        assert_eq!(g.apply(Duration::ZERO), 2);
        assert!(g.applied.read(5));
        assert!(g.pending().is_empty());
        assert_eq!(g.log[0].event, "输出 → 高电平");
        assert_eq!(g.log[1].event, "方向 → 输出，输出 → 低电平");
    }
}
