//! ADS1015 page: the AD 记录 layout for TI's 12-bit I2C ADC. The strip
//! shows the I2C channel, address, input and range, and the config
//! register word those settings make (the real ADS1015 encoding). Runs in
//! the window only: 读取转换结果 takes one demo conversion (or starts
//! continuous reads), quantised as the chip would; nothing is sent on I2C
//! or over USB.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::chart::{self, Chart};
use crate::local::{self, label, parse_num, Noise, Saved, LOG_MAX};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const CHANNELS: [(u8, &str); 2] = [(1, "I2C1"), (2, "I2C2")];
const ADDRS: [(u8, &str); 4] = [
    (0x48, "0x48"),
    (0x49, "0x49"),
    (0x4a, "0x4A"),
    (0x4b, "0x4B"),
];
/// Input multiplexer: (MUX field, label, demo volts on that input).
const MUXES: [(u8, &str, f64); 6] = [
    (0b100, "AIN0 − GND", 2.400),
    (0b101, "AIN1 − GND", 1.250),
    (0b110, "AIN2 − GND", 0.600),
    (0b111, "AIN3 − GND", 3.010),
    (0b000, "AIN0 − AIN1", 1.150),
    (0b011, "AIN2 − AIN3", -2.410),
];
/// PGA: (field, label, full scale in volts).
const PGAS: [(u8, &str, f64); 6] = [
    (0b000, "±6.144 V", 6.144),
    (0b001, "±4.096 V", 4.096),
    (0b010, "±2.048 V", 2.048),
    (0b011, "±1.024 V", 1.024),
    (0b100, "±0.512 V", 0.512),
    (0b101, "±0.256 V", 0.256),
];
/// Data rate: (field, samples per second).
const RATES: [(u8, &str); 7] = [
    (0b000, "128 SPS"),
    (0b001, "250 SPS"),
    (0b010, "490 SPS"),
    (0b011, "920 SPS"),
    (0b100, "1600 SPS"),
    (0b101, "2400 SPS"),
    (0b110, "3300 SPS"),
];
/// Conversion mode: true is single-shot.
const MODES: [(bool, &str); 2] = [(true, "单次转换"), (false, "连续转换")];
const COMPARATORS: [(u8, &str); 3] = [(0, "禁用"), (1, "传统比较器"), (2, "窗口比较器")];
const VERIFIES: [(u8, &str); 1] = [(0, "写后读回")];

fn pairs<T: Copy>(table: &[(u8, &'static str, T)]) -> Vec<(u8, &'static str)> {
    table.iter().map(|r| (r.0, r.1)).collect()
}

#[derive(Clone, PartialEq, Debug)]
struct Config {
    channel: u8,
    addr: u8,
    mux: u8,
    pga: u8,
    rate: u8,
    single: bool,
    comparator: u8,
    interval: String,
    verify: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 1,
            addr: 0x48,
            mux: 0b100,
            pga: 0b001,
            rate: 0b100,
            single: true,
            comparator: 0,
            interval: "10".into(),
            verify: 0,
        }
    }
}

impl Config {
    /// The config register (pointer 0x01) these settings write: OS to
    /// start a single conversion, MUX, PGA, MODE, DR, comparator bits.
    fn register(&self) -> u16 {
        let os = u16::from(self.single) << 15;
        let (comp_mode, que) = match self.comparator {
            0 => (0, 0b11),
            1 => (0, 0b00),
            _ => (1, 0b00),
        };
        os | u16::from(self.mux) << 12
            | u16::from(self.pga) << 9
            | u16::from(self.single) << 8
            | u16::from(self.rate) << 5
            | comp_mode << 4
            | que
    }

    fn full_scale(&self) -> f64 {
        PGAS.iter().find(|p| p.0 == self.pga).map_or(4.096, |p| p.2)
    }

    fn input(&self) -> (&'static str, f64) {
        MUXES
            .iter()
            .find(|m| m.0 == self.mux)
            .map_or(("?", 0.0), |m| (m.1, m.2))
    }

    fn interval(&self) -> Option<Duration> {
        parse_num(&self.interval, 1, 10_000, "")
            .ok()
            .map(|ms| Duration::from_millis(ms.into()))
    }
}

/// Volts to the 12-bit two's-complement code and the conversion register
/// (code in the top 12 bits). Over range clamps, as the chip does.
fn convert(v: f64, fs: f64) -> (i16, u16, bool) {
    let raw = (v / fs * 2048.0).round();
    let over = !(-2048.0..=2047.0).contains(&raw);
    let code = raw.clamp(-2048.0, 2047.0) as i16;
    (code, (code as u16) << 4, over)
}

#[derive(Clone, Copy)]
struct Sample {
    t: f64,
    mux: u8,
    code: i16,
    reg: u16,
    volts: f64,
    over: bool,
}

pub struct Ads1015 {
    cfg: Config,
    saved: Saved<Config>,
    open: bool,
    reading: bool,
    next: Duration,
    /// Oldest first.
    samples: VecDeque<Sample>,
    noise: Noise,
    table: bool,
    frozen: Option<usize>,
    copied: Option<usize>,
    /// Time of the first read, so the axis starts at zero.
    t0: Option<Duration>,
}

/// The design's opening codes, 10 ms apart on AIN0 − GND at ±4.096 V.
const SEED: [i16; 8] = [1200, 1201, 1199, 1202, 1200, 1198, 1201, 1200];

impl Default for Ads1015 {
    fn default() -> Self {
        let samples = SEED
            .iter()
            .enumerate()
            .map(|(i, &code)| Sample {
                t: i as f64 * 0.01,
                mux: 0b100,
                code,
                reg: (code as u16) << 4,
                volts: f64::from(code) * 4.096 / 2048.0,
                over: false,
            })
            .collect();
        Self {
            cfg: Config::default(),
            saved: Saved::default(),
            open: false,
            reading: false,
            next: Duration::ZERO,
            samples,
            noise: Noise::new(0x1015),
            table: false,
            frozen: None,
            copied: None,
            t0: None,
        }
    }
}

impl Ads1015 {
    /// One demo conversion at `at`.
    fn read(&mut self, at: Duration) {
        let fs = self.cfg.full_scale();
        let (_, v) = self.cfg.input();
        let v = v + 0.003 * self.noise.unit();
        let (code, reg, over) = convert(v, fs);
        let t = match (self.t0, self.samples.back()) {
            (Some(t0), _) => at.saturating_sub(t0).as_secs_f64(),
            (None, last) => {
                // continue after the seed rows
                let t = last.map_or(0.0, |s| s.t + 0.01);
                self.t0 = Some(at.saturating_sub(Duration::from_secs_f64(t)));
                t
            }
        };
        self.samples.push_back(Sample {
            t,
            mux: self.cfg.mux,
            code,
            reg,
            volts: f64::from(code) * fs / 2048.0,
            over,
        });
        while self.samples.len() > LOG_MAX {
            self.samples.pop_front();
        }
    }

    pub fn tick(&mut self, now: Duration) -> bool {
        if !self.reading {
            return false;
        }
        let Some(p) = self.cfg.interval() else {
            self.reading = false;
            return false;
        };
        if now.saturating_sub(self.next) > Duration::from_secs(2) {
            self.next = now;
        }
        while self.next <= now {
            self.read(self.next);
            self.next += p;
        }
        true
    }

    fn visible(&self) -> Vec<Sample> {
        let n = self
            .frozen
            .unwrap_or(self.samples.len())
            .min(self.samples.len());
        self.samples.iter().take(n).copied().collect()
    }

    fn export(&self) -> String {
        let mut out = String::from("time_s,input,register,code,volts\n");
        for s in &self.samples {
            let input = MUXES.iter().find(|m| m.0 == s.mux).map_or("?", |m| m.1);
            out.push_str(&format!(
                "{:.4},{input},0x{:04X},{},{:.4}\n",
                s.t, s.reg, s.code, s.volts
            ));
        }
        out
    }
}

pub fn page(ui: &mut egui::Ui, a: &mut Ads1015, now: Duration) {
    widgets::page_head(
        ui,
        "ADS1015 数据读取",
        Some(local::badge(a.reading.then_some("连续读取 · 演示"))),
        "通过 I2C 配置输入复用与量程，读取转换结果。数据为本地生成的示意，不经 USB。",
        |ui| {
            if a.cfg.single {
                if widgets::ink_button(ui, true, "读取转换结果").clicked() {
                    a.read(now);
                    a.reading = false;
                }
            } else {
                let text = if a.reading {
                    "停止读取"
                } else {
                    "开始连续读取"
                };
                let ok = a.reading || a.cfg.interval().is_some();
                if widgets::ink_button(ui, ok, text).clicked() {
                    a.reading = !a.reading;
                    a.next = now;
                }
            }
            a.saved.button(ui, &a.cfg, now);
        },
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let c = &a.cfg;
            chart::config_strip(
                ui,
                &[
                    ("I2C 通道", label(&CHANNELS, c.channel).to_owned()),
                    ("器件地址", format!("0x{:02X}", c.addr)),
                    ("输入选择", c.input().0.to_owned()),
                    ("PGA 量程", label(&pairs(&PGAS), c.pga).to_owned()),
                    ("配置寄存器", format!("0x{:04X}", c.register())),
                ],
                &mut a.open,
            );
            if a.open {
                ui.add_space(10.0);
                config(ui, a);
            }
            ui.add_space(12.0);
            plot(ui, a);
            ui.add_space(12.0);
            foot(ui, a);
            ui.add_space(8.0);
            widgets::note(ui, "多通道使用输入复用轮询，不代表四路同时转换。");
            widgets::note(ui, "PGA 量程不等于可接受输入电压；同时满足供电与输入限制。");
        });
}

fn config(ui: &mut egui::Ui, a: &mut Ads1015) {
    let t = theme::of(ui);
    let muxes = pairs(&MUXES);
    let pgas = pairs(&PGAS);
    let target = format!("{} · 0x{:02X}", label(&CHANNELS, a.cfg.channel), a.cfg.addr);
    widgets::panel(
        ui,
        "详细配置",
        |ui| {
            widgets::badge(ui, &target, Tone::Neutral);
        },
        |ui| {
            let c = &mut a.cfg;
            ui.columns(2, |cols| {
                let l = &mut cols[0];
                l.spacing_mut().item_spacing.y = 4.0;
                widgets::field_row(l, "I2C 通道", false, |ui| {
                    widgets::pick(ui, &mut c.channel, &CHANNELS);
                });
                widgets::field_row(l, "器件地址", false, |ui| {
                    widgets::pick(ui, &mut c.addr, &ADDRS);
                });
                widgets::field_row(l, "输入选择", false, |ui| {
                    widgets::pick(ui, &mut c.mux, &muxes);
                });
                widgets::field_row(l, "PGA 量程", false, |ui| {
                    widgets::pick(ui, &mut c.pga, &pgas);
                });
                widgets::field_row(l, "数据率", true, |ui| {
                    widgets::pick(ui, &mut c.rate, &RATES);
                });
                let r = &mut cols[1];
                r.spacing_mut().item_spacing.y = 4.0;
                widgets::field_row(r, "转换模式", false, |ui| {
                    widgets::pick(ui, &mut c.single, &MODES);
                });
                widgets::field_row(r, "比较器", false, |ui| {
                    widgets::pick(ui, &mut c.comparator, &COMPARATORS);
                });
                widgets::field_row(r, "读取间隔", false, |ui| {
                    widgets::unit_edit(ui, &mut c.interval, "10", "ms");
                });
                widgets::field_row(r, "配置验证", true, |ui| {
                    widgets::pick(ui, &mut c.verify, &VERIFIES);
                });
            });
            if let Err(e) = parse_num(&a.cfg.interval, 1, 10_000, "读取间隔") {
                widgets::error(ui, &e);
            }
            let reg = a.cfg.register();
            ui.label(
                egui::RichText::new(format!(
                    "待写：寄存器 0x01 ← {:02X} {:02X}；读取：寄存器 0x00（转换结果）。本页不发送。",
                    reg >> 8,
                    reg & 0xff
                ))
                .size(11.5)
                .color(t.fg2),
            );
            if a.cfg.single && a.reading {
                a.reading = false;
            }
            a.saved.foot(ui, &mut a.cfg);
        },
    );
}

fn plot(ui: &mut egui::Ui, a: &mut Ads1015) {
    let t = theme::of(ui);
    let vis = a.visible();
    let (input, _) = a.cfg.input();
    widgets::panel(
        ui,
        "ADS1015 采样视图",
        |ui| {
            widgets::badge(ui, "模拟波形 · 非实测", Tone::Neutral);
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::muted(
                    ui,
                    &format!(
                        "示例 {input} · {} · {} · 0x{:02X} · {} 个样本",
                        label(&MODES, a.cfg.single),
                        label(&CHANNELS, a.cfg.channel),
                        a.cfg.addr,
                        a.samples.len()
                    ),
                );
                widgets::trailing(ui, 27.0, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let view = if a.table { "曲线" } else { "样本数据" };
                    if widgets::quiet_button(ui, true, view).clicked() {
                        a.table = !a.table;
                    }
                    if widgets::quiet_button(ui, !a.samples.is_empty(), "清空").clicked() {
                        a.samples.clear();
                        a.frozen = None;
                        a.t0 = None;
                    }
                    let text = if a.frozen.is_some() {
                        "继续显示"
                    } else {
                        "暂停显示"
                    };
                    if widgets::quiet_button(ui, true, text).clicked() {
                        a.frozen = match a.frozen {
                            Some(_) => None,
                            None => Some(a.samples.len()),
                        };
                    }
                });
            });
            if a.table {
                let table = Table {
                    widths: &[80.0, 100.0, 90.0, 90.0, 0.0],
                };
                table.header(
                    ui,
                    &["时间 / s", "输入", "转换寄存器", "有符号码值", "电压 / V"],
                );
                egui::ScrollArea::vertical()
                    .id_salt("ads_rows")
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for s in &vis {
                            let name = MUXES.iter().find(|m| m.0 == s.mux).map_or("?", |m| m.1);
                            let volts = if s.over {
                                format!("{:.3} 超量程", s.volts)
                            } else {
                                format!("{:.3}", s.volts)
                            };
                            let cells = [
                                (format!("{:.3}", s.t), t.muted, true),
                                (name.to_owned(), t.fg, false),
                                (format!("0x{:04X}", s.reg), t.fg, true),
                                (s.code.to_string(), t.fg, true),
                                (volts, if s.over { t.amber } else { t.fg }, true),
                            ];
                            table.row(ui, &cells, false);
                        }
                    });
            } else {
                let pts: Vec<(f64, f64)> = vis.iter().map(|s| (s.t, s.volts)).collect();
                let (pts, unit) = chart::scale_x(&pts);
                Chart {
                    points: &pts,
                    y_name: "电压 / V",
                    x_name: unit,
                    height: 300.0,
                    y_places: 3,
                }
                .show(ui);
            }
            widgets::hairline(ui);
            let (v, code) = vis.last().map_or(("—".into(), "—".into()), |s| {
                (format!("{:.3} V", s.volts), s.code.to_string())
            });
            chart::legend(ui, input, &v, &code, "示例最新值");
        },
    );
}

fn foot(ui: &mut egui::Ui, a: &mut Ads1015) {
    let t = theme::of(ui);
    let pts: Vec<(f64, f64)> = a.samples.iter().map(|s| (s.t, s.volts)).collect();
    let (lo, hi) = chart::stats(&pts).map_or(("—".into(), "—".into()), |(lo, hi, _)| {
        (format!("{lo:.3} V"), format!("{hi:.3} V"))
    });
    let lsb = a.cfg.full_scale() / 2048.0 * 1000.0;
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.line))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                for (k, v) in [
                    ("最小值", lo),
                    ("最大值", hi),
                    ("LSB", format!("{lsb:.3} mV")),
                    ("采样方式", "单路转换".into()),
                ] {
                    ui.label(egui::RichText::new(k).size(11.5).color(t.muted));
                    ui.label(egui::RichText::new(v).monospace().color(t.fg));
                    ui.add_space(10.0);
                }
                widgets::trailing(ui, 28.0, |ui| {
                    if widgets::action_button(ui, !a.samples.is_empty(), "复制 CSV")
                        .on_hover_text("把样本复制到剪贴板，不写磁盘")
                        .clicked()
                    {
                        ui.ctx().copy_text(a.export());
                        a.copied = Some(a.samples.len());
                    }
                    if let Some(n) = a.copied {
                        widgets::muted(ui, &format!("已复制 {n} 行"));
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_word_matches_the_datasheet() {
        let mut c = Config::default();
        // AIN0-GND, ±4.096 V, single-shot, 1600 SPS, comparator off
        assert_eq!(c.register(), 0xc383);
        c.single = false;
        assert_eq!(c.register(), 0x4283);
        c.mux = 0b000;
        c.pga = 0b010;
        c.rate = 0b100;
        c.comparator = 2;
        assert_eq!(c.register(), 0x0490);
    }

    #[test]
    fn conversion_codes() {
        assert_eq!(convert(2.4, 4.096), (1200, 0x4b00, false));
        assert_eq!(convert(-2.41, 4.096).0, -1205);
        assert_eq!(convert(-2.41, 4.096).1, 0xb4b0);
        assert_eq!(convert(5.0, 4.096), (2047, 0x7ff0, true));
        assert_eq!(convert(-5.0, 4.096).0, -2048);
    }

    #[test]
    fn continuous_reads_follow_the_interval() {
        let mut a = Ads1015::default();
        a.cfg.single = false;
        a.reading = true;
        a.next = Duration::from_secs(1);
        a.tick(Duration::from_millis(1045));
        assert_eq!(a.samples.len(), 8 + 5);
        let last = a.samples.back().unwrap();
        assert!((last.t - 0.12).abs() < 1e-6);
        assert!(a
            .export()
            .lines()
            .nth(1)
            .unwrap()
            .contains(",AIN0 − GND,0x4B00,1200,"));
    }
}
