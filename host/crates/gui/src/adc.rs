//! AD 记录 page. The chart takes most of the page; a strip above it shows
//! the main settings and opens the full config on demand. Runs in the
//! window only: 开始记录 generates samples from a synthetic signal at the
//! target rate (a schematic, not a measurement), quantised with the
//! configured reference and resolution. Nothing goes over USB.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::chart::{self, Chart};
use crate::local::{self, label, parse_num, parse_real, Noise, Saved, Ticker, TICK};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const CHANNELS: [(u8, &str); 2] = [(1, "AD1"), (2, "AD2")];
/// Trigger: true is the threshold trigger.
const TRIGGERS: [(bool, &str); 2] = [(false, "立即开始"), (true, "阈值触发（需支持）")];
const BITS: [(u32, &str); 3] = [(10, "10 bit"), (12, "12 bit"), (16, "16 bit")];
/// Export: true is raw codes.
const EXPORTS: [(bool, &str); 2] = [(false, "CSV"), (true, "原始二进制")];
/// Samples kept for the chart, the table, and the copy.
const KEEP: usize = 4000;
/// Points drawn at most; older ones scroll off the left.
const SHOWN: usize = 240;
const RATE_MAX: u32 = 10_000;

#[derive(Clone, PartialEq, Debug)]
struct Config {
    channel: u8,
    rate: String,
    vref: String,
    seconds: String,
    trigger: bool,
    bits: u32,
    gain: String,
    offset: String,
    raw: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 1,
            rate: "1000".into(),
            vref: "3.300".into(),
            seconds: "60".into(),
            trigger: false,
            bits: 12,
            gain: "1.0000".into(),
            offset: "0.000".into(),
            raw: false,
        }
    }
}

/// The numbers a recording runs on.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Params {
    rate: u32,
    vref: f64,
    seconds: u32,
    bits: u32,
    gain: f64,
    offset: f64,
}

impl Config {
    fn params(&self) -> Result<Params, Vec<String>> {
        let mut errs = Vec::new();
        let mut keep = |r: Result<f64, String>| r.map_err(|e| errs.push(e)).unwrap_or(0.0);
        let rate = keep(parse_num(&self.rate, 1, RATE_MAX, "采样率").map(f64::from));
        let vref = keep(parse_real(&self.vref, 0.5, 6.0, "参考电压"));
        let seconds = keep(parse_num(&self.seconds, 1, 3600, "记录时长").map(f64::from));
        let gain = keep(parse_real(&self.gain, 0.5, 2.0, "校准增益"));
        let offset = keep(parse_real(&self.offset, -1.0, 1.0, "校准偏移"));
        if errs.is_empty() {
            Ok(Params {
                rate: rate as u32,
                vref,
                seconds: seconds as u32,
                bits: self.bits,
                gain,
                offset,
            })
        } else {
            Err(errs)
        }
    }
}

impl Params {
    fn full(&self) -> u32 {
        (1 << self.bits) - 1
    }

    /// Volts in to (code, calibrated volts).
    fn convert(&self, v: f64) -> (u32, f64) {
        let full = self.full();
        let code = (v / self.vref * f64::from(full))
            .round()
            .clamp(0.0, f64::from(full)) as u32;
        let volts = f64::from(code) / f64::from(full) * self.vref * self.gain + self.offset;
        (code, volts)
    }
}

/// The demo input: a slow swing around mid-scale with a faster ripple.
fn signal(t: f64, noise: &mut Noise) -> f64 {
    use std::f64::consts::TAU;
    1.65 + 0.07 * (TAU * 0.5 * t + 0.4).sin() + 0.012 * (TAU * 7.0 * t).sin() + 0.002 * noise.unit()
}

#[derive(Clone, Copy)]
struct Sample {
    t: f64,
    code: u32,
    volts: f64,
}

pub struct Adc {
    cfg: Config,
    saved: Saved<Config>,
    open: bool,
    recording: bool,
    ticker: Ticker,
    /// Samples taken in this recording, kept or not.
    taken: u64,
    params: Option<Params>,
    samples: VecDeque<Sample>,
    noise: Noise,
    /// While set, the chart shows samples up to this count only.
    frozen: Option<u64>,
    table: bool,
    copied: Option<usize>,
    done: bool,
}

/// The design's eight opening samples, 1 ms apart.
const SEED: [(u32, f64); 8] = [
    (2048, 1.650),
    (2072, 1.670),
    (2095, 1.688),
    (2114, 1.704),
    (2128, 1.715),
    (2136, 1.721),
    (2138, 1.723),
    (2133, 1.719),
];

impl Default for Adc {
    fn default() -> Self {
        let samples = SEED
            .iter()
            .enumerate()
            .map(|(i, &(code, volts))| Sample {
                t: i as f64 / 1000.0,
                code,
                volts,
            })
            .collect();
        Self {
            cfg: Config::default(),
            saved: Saved::default(),
            open: false,
            recording: false,
            ticker: Ticker::new(Duration::ZERO),
            taken: SEED.len() as u64,
            params: None,
            samples,
            noise: Noise::new(0xad1),
            frozen: None,
            table: false,
            copied: None,
            done: false,
        }
    }
}

impl Adc {
    fn start(&mut self, now: Duration) -> Result<(), Vec<String>> {
        let p = self.cfg.params()?;
        self.params = Some(p);
        self.samples.clear();
        self.taken = 0;
        self.frozen = None;
        self.recording = true;
        self.done = false;
        self.copied = None;
        self.ticker = Ticker::new(now);
        Ok(())
    }

    /// Take the samples due since the last tick; stop at the set duration.
    pub fn tick(&mut self, now: Duration) -> bool {
        let Some(p) = self.params.filter(|_| self.recording) else {
            return false;
        };
        let ticks = self.ticker.take(now, TICK, 4).len() as u64;
        let per_tick = (u64::from(p.rate) * TICK.as_millis() as u64).div_ceil(1000);
        let end = u64::from(p.seconds) * u64::from(p.rate);
        for _ in 0..ticks * per_tick {
            if self.taken >= end {
                self.recording = false;
                self.done = true;
                break;
            }
            let t = self.taken as f64 / f64::from(p.rate);
            let (code, volts) = p.convert(signal(t, &mut self.noise));
            self.samples.push_back(Sample { t, code, volts });
            if self.samples.len() > KEEP {
                self.samples.pop_front();
            }
            self.taken += 1;
        }
        self.recording
    }

    /// Kept samples up to the freeze point.
    fn visible(&self) -> Vec<Sample> {
        let skip_tail = self
            .frozen
            .map_or(0, |n| self.taken.saturating_sub(n) as usize);
        let n = self.samples.len().saturating_sub(skip_tail);
        self.samples.iter().take(n).copied().collect()
    }

    fn export(&self) -> String {
        let name = label(&CHANNELS, self.cfg.channel);
        let mut out = String::new();
        if self.cfg.raw {
            for s in &self.samples {
                out.push_str(&format!("{:04X}\n", s.code));
            }
        } else {
            out.push_str("time_s,channel,code,volts\n");
            for s in &self.samples {
                out.push_str(&format!("{:.6},{name},{},{:.4}\n", s.t, s.code, s.volts));
            }
        }
        out
    }
}

pub fn page(ui: &mut egui::Ui, a: &mut Adc, now: Duration) {
    let mut refused = None;
    widgets::page_head(
        ui,
        "AD 数据记录",
        Some(local::badge(a.recording.then_some("记录中 · 演示"))),
        "配置采样与记录条件，查看曲线、原始码值和丢样状态。曲线为本地生成的示意，非实测。",
        |ui| {
            let text = if a.recording {
                "停止记录"
            } else {
                "开始记录"
            };
            let ok = a.recording || a.cfg.params().is_ok();
            if widgets::ink_button(ui, ok, text)
                .on_disabled_hover_text("先修正详细配置里的参数")
                .clicked()
            {
                if a.recording {
                    a.recording = false;
                } else {
                    refused = a.start(now).err();
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
                    ("采样通道", label(&CHANNELS, c.channel).to_owned()),
                    ("目标采样率", format!("{} SPS", c.rate.trim())),
                    ("参考电压", format!("{} V", c.vref.trim())),
                    ("记录时长", format!("{} s", c.seconds.trim())),
                ],
                &mut a.open,
            );
            if a.open {
                ui.add_space(10.0);
                config(ui, a);
            }
            if let Some(errs) = refused {
                for e in errs {
                    widgets::error(ui, &e);
                }
            }
            ui.add_space(12.0);
            plot(ui, a);
            ui.add_space(12.0);
            foot(ui, a);
            ui.add_space(8.0);
            widgets::note(ui, "采样率、通道数与参考电压依据设备能力配置。");
            widgets::note(
                ui,
                "采集与显示解耦：暂停显示只冻结曲线，记录继续。当前固件没有 AD 命令。",
            );
        });
}

fn config(ui: &mut egui::Ui, a: &mut Adc) {
    let channel = label(&CHANNELS, a.cfg.channel);
    widgets::panel(
        ui,
        "详细配置",
        |ui| {
            widgets::badge(ui, channel, Tone::Neutral);
        },
        |ui| {
            let c = &mut a.cfg;
            ui.columns(2, |cols| {
                let l = &mut cols[0];
                l.spacing_mut().item_spacing.y = 4.0;
                widgets::field_row(l, "采样通道", false, |ui| {
                    widgets::pick(ui, &mut c.channel, &CHANNELS);
                });
                widgets::field_row(l, "目标采样率", false, |ui| {
                    widgets::unit_edit(ui, &mut c.rate, "1000", "SPS");
                });
                widgets::field_row(l, "参考电压", false, |ui| {
                    widgets::unit_edit(ui, &mut c.vref, "3.300", "V");
                });
                widgets::field_row(l, "记录时长", false, |ui| {
                    widgets::unit_edit(ui, &mut c.seconds, "60", "s");
                });
                widgets::field_row(l, "触发方式", true, |ui| {
                    widgets::pick(ui, &mut c.trigger, &TRIGGERS);
                });
                let r = &mut cols[1];
                r.spacing_mut().item_spacing.y = 4.0;
                widgets::field_row(r, "分辨率", false, |ui| {
                    widgets::pick(ui, &mut c.bits, &BITS);
                });
                widgets::field_row(r, "校准增益", false, |ui| {
                    widgets::unit_edit(ui, &mut c.gain, "1.0000", "");
                });
                widgets::field_row(r, "校准偏移", false, |ui| {
                    widgets::unit_edit(ui, &mut c.offset, "0.000", "V");
                });
                widgets::field_row(r, "导出格式", true, |ui| {
                    widgets::pick(ui, &mut c.raw, &EXPORTS);
                });
            });
            if let Err(errs) = a.cfg.params() {
                for e in errs {
                    widgets::error(ui, &e);
                }
            }
            if a.cfg.trigger {
                widgets::muted(ui, "阈值触发需设备支持；本页演示按立即开始记录。");
            }
            a.saved.foot(ui, &mut a.cfg);
        },
    );
}

fn plot(ui: &mut egui::Ui, a: &mut Adc) {
    let t = theme::of(ui);
    let vis = a.visible();
    let name = label(&CHANNELS, a.cfg.channel);
    widgets::panel(
        ui,
        "电压记录",
        |ui| {
            widgets::badge(ui, "模拟波形 · 非实测", Tone::Neutral);
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::muted(
                    ui,
                    &format!(
                        "示例 {name} · 已采 {} 个样本 · 保留 {} 个",
                        a.taken,
                        a.samples.len()
                    ),
                );
                widgets::trailing(ui, 27.0, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let view = if a.table { "曲线" } else { "样本数据" };
                    if widgets::quiet_button(ui, true, view).clicked() {
                        a.table = !a.table;
                    }
                    let text = if a.frozen.is_some() {
                        "继续显示"
                    } else {
                        "暂停显示"
                    };
                    if widgets::quiet_button(ui, true, text).clicked() {
                        a.frozen = match a.frozen {
                            Some(_) => None,
                            None => Some(a.taken),
                        };
                    }
                });
            });
            let tail = &vis[vis.len().saturating_sub(SHOWN)..];
            if a.table {
                let table = Table {
                    widths: &[90.0, 60.0, 90.0, 90.0, 0.0],
                };
                table.header(ui, &["时间 / s", "通道", "原始码值", "电压 / V", "质量"]);
                egui::ScrollArea::vertical()
                    .id_salt("adc_rows")
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for s in &tail[tail.len().saturating_sub(60)..] {
                            let cells = [
                                (format!("{:.3}", s.t), t.muted, true),
                                (name.to_owned(), t.fg, true),
                                (s.code.to_string(), t.fg, true),
                                (format!("{:.3}", s.volts), t.fg, true),
                                ("示例".to_owned(), t.muted, false),
                            ];
                            table.row(ui, &cells, false);
                        }
                    });
            } else {
                let pts: Vec<(f64, f64)> = tail.iter().map(|s| (s.t, s.volts)).collect();
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
            chart::legend(ui, name, &v, &code, "示例最新值");
        },
    );
}

fn foot(ui: &mut egui::Ui, a: &mut Adc) {
    let t = theme::of(ui);
    let pts: Vec<(f64, f64)> = a.samples.iter().map(|s| (s.t, s.volts)).collect();
    let (lo, hi) = chart::stats(&pts).map_or(("—".into(), "—".into()), |(lo, hi, _)| {
        (format!("{lo:.3} V"), format!("{hi:.3} V"))
    });
    let status = if a.recording {
        "记录中".to_owned()
    } else if a.done {
        "已到记录时长".to_owned()
    } else {
        "空闲".to_owned()
    };
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
                    ("丢样", "0".into()),
                    ("状态", status),
                    ("记录文件", "adc-session-demo.csv".into()),
                ] {
                    ui.label(egui::RichText::new(k).size(11.5).color(t.muted));
                    ui.label(egui::RichText::new(v).monospace().color(t.fg));
                    ui.add_space(10.0);
                }
                widgets::trailing(ui, 28.0, |ui| {
                    let what = label(&EXPORTS, a.cfg.raw);
                    if widgets::action_button(ui, !a.samples.is_empty(), &format!("复制 {what}"))
                        .on_hover_text("把保留的样本复制到剪贴板，不写磁盘")
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
    fn quantise_and_calibrate() {
        let p = Config::default().params().unwrap();
        assert_eq!(p.convert(1.65), (2048, 2048.0 / 4095.0 * 3.3));
        assert_eq!(p.convert(5.0).0, 4095);
        assert_eq!(p.convert(-1.0).0, 0);
        let c = Config {
            vref: "9".into(),
            ..Config::default()
        };
        assert!(c.params().is_err());
    }

    #[test]
    fn recording_runs_for_its_duration() {
        let mut a = Adc::default();
        a.cfg.rate = "100".into();
        a.cfg.seconds = "1".into();
        a.start(Duration::ZERO).unwrap();
        for i in 1..=10 {
            a.tick(Duration::from_millis(50 * i));
        }
        assert_eq!(a.taken, 50);
        assert!(a.recording);
        for i in 11..=40 {
            a.tick(Duration::from_millis(50 * i));
        }
        assert_eq!(a.taken, 100);
        assert!(!a.recording && a.done);
        a.frozen = Some(60);
        assert_eq!(a.visible().len(), 60);
        assert!(a
            .export()
            .starts_with("time_s,channel,code,volts\n0.000000,AD1,"));
    }
}
