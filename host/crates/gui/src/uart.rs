//! UART page. Port settings on the left; the receive terminal and the send
//! editor on the right, in fixed places. Runs in the window only: 打开串口
//! opens a local demo session whose replies come from a small responder in
//! this file (help / version / status, anything else is echoed). Nothing
//! goes over USB; the bridge has no UART command.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::{clock, parse_bytes};
use crate::local::{self, hex_upper, label, parse_num, Saved, LOG_MAX};
use crate::theme;
use crate::widgets::{self, Tone};

const CHANNELS: [(u8, &str); 2] = [(1, "UART1"), (2, "UART2")];
const BAUDS: [(u32, &str); 6] = [
    (9600, "9600 bit/s"),
    (19200, "19200 bit/s"),
    (38400, "38400 bit/s"),
    (57600, "57600 bit/s"),
    (115_200, "115200 bit/s"),
    (921_600, "921600 bit/s"),
];
const DATA_BITS: [(u8, &str); 2] = [(7, "7"), (8, "8")];
const STOP_BITS: [(u8, &str); 2] = [(1, "1"), (2, "2")];
const PARITIES: [(char, &str); 3] = [('N', "无"), ('O', "奇校验"), ('E', "偶校验")];
/// Flow control: true is RTS/CTS.
const FLOWS: [(bool, &str); 2] = [(false, "无"), (true, "RTS/CTS（需支持）")];
/// Display and send format: true is HEX.
const FORMATS: [(bool, &str); 2] = [(false, "文本"), (true, "HEX")];
const EOLS: [(&str, &str); 3] = [("", "无"), ("\n", "LF"), ("\r\n", "CR + LF")];
/// Text encoding: true is ASCII only.
const ENCODINGS: [(bool, &str); 2] = [(false, "UTF-8"), (true, "ASCII")];

#[derive(Clone, PartialEq, Debug)]
struct Config {
    channel: u8,
    baud: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: char,
    flow: bool,
    hex: bool,
    eol: &'static str,
    ascii: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 1,
            baud: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 'N',
            flow: false,
            hex: false,
            eol: "\r\n",
            ascii: false,
        }
    }
}

impl Config {
    /// `8-N-1`
    fn frame(&self) -> String {
        format!("{}-{}-{}", self.data_bits, self.parity, self.stop_bits)
    }

    /// Time on the wire for `n` bytes: start bit, data, parity, stop bits.
    fn wire_us(&self, n: usize) -> f64 {
        let bits = 1
            + u32::from(self.data_bits)
            + u32::from(self.parity != 'N')
            + u32::from(self.stop_bits);
        n as f64 * f64::from(bits) * 1e6 / f64::from(self.baud)
    }
}

struct Line {
    at: Duration,
    tx: bool,
    bytes: Vec<u8>,
    /// Where it came from: 示例 for the opening rows, 本地回显 for replies.
    tag: &'static str,
}

pub struct Uart {
    cfg: Config,
    saved: Saved<Config>,
    advanced: bool,
    open: bool,
    input: String,
    looping: bool,
    period: String,
    loop_next: Duration,
    send_err: Option<String>,
    /// Oldest first.
    log: VecDeque<Line>,
    /// While set, the terminal shows this many lines only.
    frozen: Option<usize>,
    stamps: bool,
}

const SEED: [(u64, bool, &str); 8] = [
    (0, false, "[DEMO] BusStudio serial session"),
    (10, true, "help"),
    (14, false, "Commands: version, status, read"),
    (200, true, "version"),
    (204, false, "Firmware: demo-1.0"),
    (400, true, "status"),
    (404, false, "Mode: idle"),
    (406, false, "Ready."),
];

impl Default for Uart {
    fn default() -> Self {
        let mut u = Self {
            cfg: Config::default(),
            saved: Saved::default(),
            advanced: false,
            open: false,
            input: "status".into(),
            looping: false,
            period: "1000".into(),
            loop_next: Duration::ZERO,
            send_err: None,
            log: VecDeque::new(),
            frozen: None,
            stamps: true,
        };
        for (ms, tx, text) in SEED {
            u.push(
                Duration::from_millis(ms),
                tx,
                text.as_bytes().to_vec(),
                "示例",
            );
        }
        u
    }
}

/// What the demo responder answers to one line.
fn reply(line: &str) -> String {
    match line.trim() {
        "help" => "Commands: version, status, read".into(),
        "version" => "Firmware: demo-1.0".into(),
        "status" => "Mode: idle".into(),
        "read" => "Value: 0x2A".into(),
        "" => "Ready.".into(),
        other => format!("echo: {other}"),
    }
}

impl Uart {
    pub fn running(&self) -> bool {
        self.open && self.looping
    }

    fn push(&mut self, at: Duration, tx: bool, bytes: Vec<u8>, tag: &'static str) {
        self.log.push_back(Line { at, tx, bytes, tag });
        while self.log.len() > LOG_MAX {
            self.log.pop_front();
            if let Some(n) = &mut self.frozen {
                *n = n.saturating_sub(1);
            }
        }
    }

    /// The bytes the send field stands for, line ending included.
    fn payload(&self) -> Result<Vec<u8>, String> {
        let mut bytes = if self.cfg.hex {
            parse_bytes(&self.input)?
        } else {
            if self.cfg.ascii && !self.input.is_ascii() {
                return Err("ASCII 编码只能发送英文字符".into());
            }
            self.input.as_bytes().to_vec()
        };
        if !self.cfg.hex {
            bytes.extend_from_slice(self.cfg.eol.as_bytes());
        }
        if bytes.is_empty() {
            return Err("发送内容为空".into());
        }
        Ok(bytes)
    }

    fn send(&mut self, at: Duration) -> Result<(), String> {
        if !self.open {
            return Err("先打开串口".into());
        }
        let bytes = self.payload()?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        self.push(at, true, bytes, "本地");
        // the reply arrives after the bytes are out, plus a little
        let back = at + Duration::from_micros(self.cfg.wire_us(text.len()) as u64 + 2000);
        let answer = format!("{}\r\n", reply(&text));
        self.push(back, false, answer.into_bytes(), "本地回显");
        Ok(())
    }

    fn loop_period(&self) -> Option<Duration> {
        parse_num(&self.period, 50, 60_000, "")
            .ok()
            .map(|ms| Duration::from_millis(ms.into()))
    }

    pub fn tick(&mut self, now: Duration) -> bool {
        if self.open && self.looping {
            match self.loop_period() {
                Some(p) => {
                    if now.saturating_sub(self.loop_next) > Duration::from_secs(2) {
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

    fn counts(&self) -> (usize, usize, usize, usize) {
        let mut c = (0, 0, 0, 0);
        for l in &self.log {
            if l.tx {
                c.0 += 1;
                c.2 += l.bytes.len();
            } else {
                c.1 += 1;
                c.3 += l.bytes.len();
            }
        }
        c
    }
}

/// A line as the terminal shows it: text with CR/LF dropped, or hex.
fn shown_text(bytes: &[u8], hex: bool) -> String {
    if hex {
        hex_upper(bytes)
    } else {
        String::from_utf8_lossy(bytes)
            .trim_end_matches(['\r', '\n'])
            .replace(['\r', '\n'], " ")
    }
}

pub fn page(ui: &mut egui::Ui, u: &mut Uart, now: Duration) {
    let badge = if u.open {
        ("已打开 · 本地回显", Tone::Green)
    } else {
        local::badge(None)
    };
    widgets::page_head(
        ui,
        "UART 串口终端",
        Some(badge),
        "配置串口参数，使用文本或 HEX 进行收发调试。以下为演示数据，不经 USB。",
        |ui| {
            let text = if u.open {
                "关闭串口"
            } else {
                "打开串口"
            };
            if widgets::ink_button(ui, true, text).clicked() {
                u.open = !u.open;
                let note = if u.open {
                    format!(
                        "[本地演示] {} {} {} 已打开，未接线，回复来自本窗口",
                        label(&CHANNELS, u.cfg.channel),
                        u.cfg.baud,
                        u.cfg.frame()
                    )
                } else {
                    "[本地演示] 串口已关闭".to_owned()
                };
                u.push(now, false, note.into_bytes(), "本地");
                u.loop_next = now;
            }
            u.saved.button(ui, &u.cfg, now);
        },
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::work_columns(ui, u, 264.0, 760.0, config, |ui, u| {
                summary(ui, u);
                ui.add_space(12.0);
                terminal(ui, u);
                ui.add_space(14.0);
                compose(ui, u, now);
            });
        });
}

fn config(ui: &mut egui::Ui, u: &mut Uart) {
    let channel = label(&CHANNELS, u.cfg.channel);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, channel, Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let g = &mut u.cfg;
            widgets::field_row(ui, "硬件通道", false, |ui| {
                widgets::pick(ui, &mut g.channel, &CHANNELS);
            });
            widgets::field_row(ui, "波特率", false, |ui| {
                widgets::pick(ui, &mut g.baud, &BAUDS);
            });
            widgets::field_row(ui, "数据位", false, |ui| {
                widgets::pick(ui, &mut g.data_bits, &DATA_BITS);
            });
            widgets::field_row(ui, "停止位", false, |ui| {
                widgets::pick(ui, &mut g.stop_bits, &STOP_BITS);
            });
            widgets::field_row(ui, "校验", false, |ui| {
                widgets::pick(ui, &mut g.parity, &PARITIES);
            });
            widgets::disclosure(ui, &mut u.advanced, "高级参数", 4);
            widgets::hairline(ui);
            if u.advanced {
                widgets::field_row(ui, "流控", false, |ui| {
                    widgets::pick(ui, &mut g.flow, &FLOWS);
                });
                widgets::field_row(ui, "显示格式", false, |ui| {
                    widgets::pick(ui, &mut g.hex, &FORMATS);
                });
                widgets::field_row(ui, "发送行尾", false, |ui| {
                    widgets::pick(ui, &mut g.eol, &EOLS);
                });
                widgets::field_row(ui, "文本编码", true, |ui| {
                    widgets::pick(ui, &mut g.ascii, &ENCODINGS);
                });
            }
            u.saved.foot(ui, &mut u.cfg);
        },
    );
    ui.add_space(8.0);
    widgets::note(ui, "循环发送与单次发送分别控制，切换页面不会停止循环。");
    widgets::note(
        ui,
        "TTL、RS-232 与 RS-485 需匹配实际电气接口。当前固件没有 UART 命令。",
    );
}

fn summary(ui: &mut egui::Ui, u: &Uart) {
    let (tx, rx, tx_b, rx_b) = u.counts();
    widgets::summary_strip(
        ui,
        &[
            ("发送", format!("{tx} 条 / {tx_b} B")),
            ("接收", format!("{rx} 条 / {rx_b} B")),
            ("串口格式", u.cfg.frame()),
        ],
        "演示数据",
    );
}

fn terminal(ui: &mut egui::Ui, u: &mut Uart) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "接收终端",
        |ui| {
            widgets::badge(ui, "本地回显 · 非真实串口", Tone::Neutral);
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::muted(
                    ui,
                    &format!(
                        "{} · {} · {}",
                        label(&CHANNELS, u.cfg.channel),
                        label(&BAUDS, u.cfg.baud),
                        label(&FORMATS, u.cfg.hex)
                    ),
                );
                widgets::trailing(ui, 27.0, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::quiet_button(ui, !u.log.is_empty(), "清空").clicked() {
                        u.log.clear();
                        u.frozen = None;
                    }
                    let text = if u.frozen.is_some() {
                        "继续显示"
                    } else {
                        "暂停显示"
                    };
                    if widgets::quiet_button(ui, true, text).clicked() {
                        u.frozen = match u.frozen {
                            Some(_) => None,
                            None => Some(u.log.len()),
                        };
                    }
                    widgets::check(ui, &mut u.stamps, "时间戳");
                    ui.add_space(6.0);
                    widgets::seg(ui, &mut u.cfg.hex, &FORMATS);
                });
            });
            let shown = u.frozen.unwrap_or(u.log.len()).min(u.log.len());
            egui::Frame::new()
                .fill(t.card2)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("uart_term")
                        .max_height(260.0)
                        .min_scrolled_height(260.0)
                        .auto_shrink([false, false])
                        .stick_to_bottom(u.frozen.is_none())
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            for l in u.log.iter().take(shown) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    if u.stamps {
                                        ui.label(
                                            egui::RichText::new(clock(l.at))
                                                .monospace()
                                                .color(t.muted),
                                        );
                                    }
                                    let (d, c) = if l.tx {
                                        ("TX", t.accent)
                                    } else {
                                        ("RX", t.green)
                                    };
                                    ui.label(egui::RichText::new(d).monospace().color(c));
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(shown_text(&l.bytes, u.cfg.hex))
                                                .monospace()
                                                .color(t.fg),
                                        )
                                        .wrap(),
                                    );
                                    ui.label(egui::RichText::new(l.tag).size(10.5).color(t.faint));
                                });
                            }
                        });
                });
            let left = if u.frozen.is_some() {
                format!("已暂停显示 · 共 {} 条，收发不受影响", u.log.len())
            } else {
                format!("{} 条 · 自动跟随", u.log.len())
            };
            widgets::table_foot(ui, &left, &format!("最多 {LOG_MAX} 条"));
        },
    );
}

fn compose(ui: &mut egui::Ui, u: &mut Uart, now: Duration) {
    let t = theme::of(ui);
    let format = label(&FORMATS, u.cfg.hex);
    widgets::panel(
        ui,
        "发送",
        |ui| {
            widgets::muted(ui, format);
        },
        |ui| {
            let hint = if u.cfg.hex { "68 65 6C 70" } else { "help" };
            ui.horizontal(|ui| {
                let w = ui.available_width() - 90.0;
                let r = ui.add(
                    egui::TextEdit::singleline(&mut u.input)
                        .hint_text(hint)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(w),
                );
                if r.changed() {
                    u.send_err = None;
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let clicked = widgets::ink_button(ui, u.open, "发送")
                    .on_disabled_hover_text("先打开串口")
                    .clicked();
                if clicked || (enter && u.open) {
                    u.send_err = u.send(now).err();
                }
            });
            ui.horizontal(|ui| {
                let eol = if u.cfg.hex {
                    "HEX 原样发送"
                } else {
                    label(&EOLS, u.cfg.eol)
                };
                widgets::muted(ui, &format!("行尾 {eol}"));
                ui.add_space(10.0);
                if widgets::check(ui, &mut u.looping, "循环").changed() {
                    u.loop_next = now;
                }
                ui.add(
                    egui::TextEdit::singleline(&mut u.period)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(56.0),
                );
                widgets::muted(ui, "ms");
                widgets::trailing(ui, 20.0, |ui| {
                    let text = match u.payload() {
                        Ok(b) => format!(
                            "{} B · 约 {}",
                            b.len(),
                            crate::spi::micros_text(u.cfg.wire_us(b.len()))
                        ),
                        Err(_) => String::new(),
                    };
                    widgets::muted(ui, &text);
                });
            });
            let problem = u
                .send_err
                .clone()
                .or_else(|| u.payload().err())
                .or_else(|| {
                    parse_num(&u.period, 50, 60_000, "循环周期")
                        .err()
                        .filter(|_| u.looping)
                });
            match problem {
                Some(e) => widgets::error(ui, &e),
                None if !u.open => {
                    ui.label(
                        egui::RichText::new("打开串口后可发送；回复由本窗口的演示应答生成。")
                            .size(11.5)
                            .color(t.muted),
                    );
                }
                None => {}
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_needs_open_and_gets_a_reply() {
        let mut u = Uart::default();
        assert_eq!(u.send(Duration::ZERO).unwrap_err(), "先打开串口");
        u.open = true;
        u.input = "version".into();
        u.send(Duration::from_secs(1)).unwrap();
        let n = u.log.len();
        assert_eq!(u.log[n - 2].bytes, b"version\r\n");
        assert_eq!(shown_text(&u.log[n - 1].bytes, false), "Firmware: demo-1.0");
        assert!(u.log[n - 1].at > u.log[n - 2].at);
        u.cfg.hex = true;
        u.input = "41 42".into();
        assert_eq!(u.payload().unwrap(), vec![0x41, 0x42]);
        assert_eq!(shown_text(b"AB", true), "41 42");
        u.cfg.hex = false;
        u.cfg.ascii = true;
        u.input = "中".into();
        assert!(u.payload().is_err());
    }

    #[test]
    fn frame_and_timing() {
        let c = Config::default();
        assert_eq!(c.frame(), "8-N-1");
        // ten bits per byte at 115200
        assert!((c.wire_us(1) - 86.8).abs() < 0.1);
        assert_eq!(reply("nope"), "echo: nope");
    }

    #[test]
    fn loop_sends_on_its_period() {
        let mut u = Uart {
            open: true,
            looping: true,
            period: "100".into(),
            ..Uart::default()
        };
        let before = u.log.len();
        u.tick(Duration::from_millis(250));
        // at 0, 100 and 200 ms, each with its reply
        assert_eq!(u.log.len(), before + 6);
    }
}
