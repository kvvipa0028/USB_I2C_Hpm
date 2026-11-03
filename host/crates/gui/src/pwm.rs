//! PWM page. Output config on the left; the configured waveform with the
//! duty slider, and a log of applied configs on the right. Runs in the
//! window only: the waveform is drawn from the settings (a schematic, not a
//! measurement), and 应用配置 adds a record to the log without sending
//! anything over USB.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::clock;
use crate::spi::{micros_text, trim};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const LOG_MAX: usize = 200;
const MAX_HZ: u32 = 1_000_000;
const MAX_DELAY_MS: u32 = 60_000;
/// Periods drawn in the preview.
const CYCLES: usize = 4;

const CHANNELS: [(u8, &str); 2] = [(0, "PWM0"), (1, "PWM1")];
/// Active level: true is high.
const LEVELS: [(bool, &str); 2] = [(true, "高电平"), (false, "低电平")];
const IDLES: [(Idle, &str); 3] = [
    (Idle::Low, "低电平"),
    (Idle::High, "高电平"),
    (Idle::HiZ, "高阻态"),
];
/// Alignment: true is centre-aligned.
const ALIGNS: [(bool, &str); 2] = [(false, "边沿对齐"), (true, "中心对齐")];

fn label<T: PartialEq>(options: &[(T, &'static str)], v: T) -> &'static str {
    options.iter().find(|o| o.0 == v).map_or("?", |o| o.1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Idle {
    Low,
    High,
    HiZ,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Config {
    channel: u8,
    hz: u32,
    /// Percent of the period at the active level, 0 to 100.
    duty: f32,
    active_high: bool,
    idle: Idle,
    center: bool,
    delay_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 0,
            hz: 1000,
            duty: 25.0,
            active_high: true,
            idle: Idle::Low,
            center: false,
            delay_ms: 0,
        }
    }
}

impl Config {
    fn period_us(&self) -> f64 {
        1e6 / f64::from(self.hz)
    }

    /// Time the pin spends high in one period.
    fn high_us(&self) -> f64 {
        let active = self.period_us() * f64::from(self.duty) / 100.0;
        if self.active_high {
            active
        } else {
            self.period_us() - active
        }
    }

    /// `PWM0 · 1000 Hz · 25.0%`
    fn short(&self) -> String {
        format!(
            "{} · {} · {:.1}%",
            label(&CHANNELS, self.channel),
            hz_text(self.hz),
            self.duty
        )
    }

    /// Names of the settings that differ from `old`, for the log.
    fn changes(&self, old: &Config) -> Vec<&'static str> {
        let mut out = Vec::new();
        let mut diff = |changed: bool, name| {
            if changed {
                out.push(name);
            }
        };
        diff(self.channel != old.channel, "通道");
        diff(self.hz != old.hz, "频率");
        diff(self.duty != old.duty, "占空比");
        diff(self.active_high != old.active_high, "有效电平");
        diff(self.idle != old.idle, "停止状态");
        diff(self.center != old.center, "对齐方式");
        diff(self.delay_ms != old.delay_ms, "启动延时");
        out
    }
}

fn hz_text(hz: u32) -> String {
    if hz >= 100_000 {
        format!("{} kHz", trim(f64::from(hz) / 1000.0, 3))
    } else {
        format!("{hz} Hz")
    }
}

/// Active stretches of one period as (from, to) fractions of the period.
fn active_span(duty: f32, center: bool) -> (f32, f32) {
    let d = (duty / 100.0).clamp(0.0, 1.0);
    if center {
        ((1.0 - d) / 2.0, (1.0 + d) / 2.0)
    } else {
        (0.0, d)
    }
}

/// The preview trace over `cycles` periods: (x fraction of the whole width,
/// high) at each point, with vertical edges where the level flips.
fn trace(cfg: &Config, cycles: usize) -> Vec<(f32, bool)> {
    let (a, b) = active_span(cfg.duty, cfg.center);
    let level = |active: bool| active == cfg.active_high;
    let mut segs: Vec<(f32, f32, bool)> = Vec::new();
    for i in 0..cycles {
        let x0 = i as f32;
        for (from, to, active) in [(0.0, a, false), (a, b, true), (b, 1.0, false)] {
            if to > from {
                let high = level(active);
                match segs.last_mut() {
                    Some(last) if last.2 == high => last.1 = x0 + to,
                    _ => segs.push((x0 + from, x0 + to, high)),
                }
            }
        }
    }
    let n = cycles as f32;
    segs.iter()
        .flat_map(|&(from, to, high)| [(from / n, high), (to / n, high)])
        .collect()
}

struct Record {
    /// Since the window opened.
    at: Duration,
    cfg: Config,
    /// 应用配置 the first time, then what changed.
    event: String,
}

pub struct Pwm {
    draft: Config,
    applied: Option<(Config, Duration)>,
    hz: String,
    duty: String,
    delay: String,
    advanced: bool,
    /// Newest first.
    log: VecDeque<Record>,
}

impl Default for Pwm {
    fn default() -> Self {
        let mut p = Self {
            draft: Config::default(),
            applied: None,
            hz: String::new(),
            duty: String::new(),
            delay: String::new(),
            advanced: false,
            log: VecDeque::new(),
        };
        p.sync_text();
        p
    }
}

impl Pwm {
    fn dirty(&self) -> bool {
        self.applied.is_none_or(|(c, _)| c != self.draft)
    }

    fn sync_text(&mut self) {
        self.hz = self.draft.hz.to_string();
        self.duty = format!("{:.1}", self.draft.duty);
        self.delay = self.draft.delay_ms.to_string();
    }

    /// Keep the draft as the applied config and log it.
    fn apply(&mut self, at: Duration) {
        let changes = self.applied.map(|(old, _)| self.draft.changes(&old));
        let event = match changes {
            Some(names) if !names.is_empty() => format!("修改 {}", names.join("、")),
            _ => "应用配置".to_owned(),
        };
        self.applied = Some((self.draft, at));
        self.log.push_front(Record {
            at,
            cfg: self.draft,
            event,
        });
        self.log.truncate(LOG_MAX);
    }

    /// Back to the applied config, or the defaults before the first apply.
    fn revert(&mut self) {
        self.draft = self.applied.map_or_else(Config::default, |(c, _)| c);
        self.sync_text();
    }
}

fn parse_hz(s: &str) -> Result<u32, String> {
    s.trim()
        .parse::<u32>()
        .ok()
        .filter(|v| (1..=MAX_HZ).contains(v))
        .ok_or_else(|| "频率填 1 到 1000000 Hz 的整数".to_owned())
}

fn parse_duty(s: &str) -> Result<f32, String> {
    s.trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .filter(|v| (0.0..=100.0).contains(v))
        .map(|v| (v * 10.0).round() / 10.0)
        .ok_or_else(|| "占空比填 0 到 100".to_owned())
}

fn parse_delay(s: &str) -> Result<u32, String> {
    s.trim()
        .parse::<u32>()
        .ok()
        .filter(|v| *v <= MAX_DELAY_MS)
        .ok_or_else(|| format!("启动延时填 0 到 {MAX_DELAY_MS} ms"))
}

pub fn page(ui: &mut egui::Ui, p: &mut Pwm, now: Duration) {
    widgets::page_head(
        ui,
        "PWM",
        Some(("配置预览", Tone::Neutral)),
        "设置频率与占空比，预览待应用的输出配置。波形为配置示意。",
        |ui| {
            if widgets::ink_button(ui, p.dirty(), "应用配置").clicked() {
                p.apply(now);
            }
            let changed = p.draft != Config::default();
            if widgets::action_button(ui, changed, "恢复默认").clicked() {
                p.draft = Config::default();
                p.sync_text();
            }
        },
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::work_columns(ui, p, 264.0, 760.0, config, |ui, p| {
                summary(ui, &p.draft);
                ui.add_space(12.0);
                preview(ui, p);
                ui.add_space(14.0);
                log(ui, p);
            });
        });
}

/// A number field. While it has focus, valid input goes straight into
/// `value` and bad input gives the error to show; without focus it shows
/// `value`, so the slider, 撤销修改 and 恢复默认 show up here too.
fn number_row<T: Copy>(
    ui: &mut egui::Ui,
    name: &str,
    text: &mut String,
    unit: &str,
    parse: fn(&str) -> Result<T, String>,
    value: &mut T,
    show: fn(T) -> String,
) -> Option<String> {
    let mut err = None;
    widgets::field_row(ui, name, false, |ui| {
        let r = widgets::unit_edit(ui, text, "", unit);
        if r.has_focus() {
            match parse(text) {
                Ok(v) => *value = v,
                Err(e) => err = Some(e),
            }
        } else {
            *text = show(*value);
        }
    });
    err
}

fn config(ui: &mut egui::Ui, p: &mut Pwm) {
    let t = theme::of(ui);
    let channel = label(&CHANNELS, p.draft.channel);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, channel, Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let mut errs = Vec::new();
            let d = &mut p.draft;
            widgets::field_row(ui, "输出通道", false, |ui| {
                widgets::pick(ui, &mut d.channel, &CHANNELS);
            });
            errs.extend(number_row(
                ui,
                "输出频率",
                &mut p.hz,
                "Hz",
                parse_hz,
                &mut d.hz,
                |v| v.to_string(),
            ));
            errs.extend(number_row(
                ui,
                "占空比",
                &mut p.duty,
                "%",
                parse_duty,
                &mut d.duty,
                |v| format!("{v:.1}"),
            ));
            widgets::field_row(ui, "有效电平", false, |ui| {
                widgets::pick(ui, &mut d.active_high, &LEVELS);
            });
            widgets::field_row(ui, "停止状态", false, |ui| {
                widgets::pick(ui, &mut d.idle, &IDLES);
            });
            widgets::disclosure(ui, &mut p.advanced, "高级参数", 2);
            if p.advanced {
                widgets::hairline(ui);
                widgets::field_row(ui, "对齐方式", false, |ui| {
                    widgets::pick(ui, &mut d.center, &ALIGNS);
                });
                errs.extend(number_row(
                    ui,
                    "启动延时",
                    &mut p.delay,
                    "ms",
                    parse_delay,
                    &mut d.delay_ms,
                    |v| v.to_string(),
                ));
            } else {
                widgets::hairline(ui);
            }
            for e in errs {
                ui.label(egui::RichText::new(e).size(11.5).color(t.red));
            }
            let dirty = p.dirty();
            ui.horizontal(|ui| {
                ui.set_min_height(30.0);
                let status = match (p.applied, dirty) {
                    (None, _) => "尚未应用".to_owned(),
                    (Some(_), true) => "参数有修改".to_owned(),
                    (Some((_, at)), false) => format!("已应用 · {}", clock(at)),
                };
                let color = if dirty && p.applied.is_some() {
                    t.amber
                } else {
                    t.muted
                };
                ui.label(egui::RichText::new(status).size(12.0).color(color));
                widgets::trailing(ui, 27.0, |ui| {
                    if widgets::quiet_button(ui, dirty && p.applied.is_some(), "撤销修改").clicked()
                    {
                        p.revert();
                    }
                });
            });
        },
    );
    ui.add_space(8.0);
    widgets::note(ui, "波形仅为配置预览，不代表实测输出。");
}

fn summary(ui: &mut egui::Ui, c: &Config) {
    let period = c.period_us();
    let high = c.high_us();
    widgets::summary_strip(
        ui,
        &[
            ("配置周期", micros_text(period)),
            ("配置高电平", micros_text(high)),
            ("配置低电平", micros_text(period - high)),
        ],
        "随设置更新",
    );
}

fn preview(ui: &mut egui::Ui, p: &mut Pwm) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "输出配置预览",
        |ui| {
            widgets::badge(ui, "非实测波形", Tone::Neutral);
        },
        |ui| {
            let big = |ui: &mut egui::Ui, value: String, unit: &str| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        egui::RichText::new(value)
                            .monospace()
                            .size(26.0)
                            .color(t.fg),
                    );
                    ui.label(
                        egui::RichText::new(unit)
                            .monospace()
                            .size(12.0)
                            .color(t.fg2),
                    );
                });
            };
            let muted = |s: &str| egui::RichText::new(s).size(11.5).color(t.muted);
            ui.columns(2, |cols| {
                let (hz, unit) = match hz_text(p.draft.hz).split_once(' ') {
                    Some((v, u)) => (v.to_owned(), u.to_owned()),
                    None => (p.draft.hz.to_string(), "Hz".to_owned()),
                };
                cols[0].label(muted("输出频率"));
                big(&mut cols[0], hz, &unit);
                cols[0].label(
                    egui::RichText::new(format!("周期 {}", micros_text(p.draft.period_us())))
                        .size(12.0)
                        .color(t.fg2),
                );
                cols[1].label(muted("占空比"));
                big(&mut cols[1], trim(f64::from(p.draft.duty), 1), "%");
                let mut duty = p.draft.duty;
                if widgets::slider(&mut cols[1], &mut duty, 0.0..=100.0, 1.0).changed() {
                    p.draft.duty = duty;
                }
            });
            ui.add_space(6.0);
            wave(ui, &p.draft);
            widgets::hairline(ui);
            let applied = match p.applied {
                Some((c, _)) => format!("已应用：{}", c.short()),
                None => "尚未应用".to_owned(),
            };
            widgets::table_foot(ui, "拖动滑杆或填写占空比，预览随即更新", &applied);
        },
    );
}

/// Four periods of the configured output, H and L rails, time below.
fn wave(ui: &mut egui::Ui, c: &Config) {
    let t = theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 220.0),
        egui::Sense::hover(),
    );
    let p = ui.painter_at(rect);
    let small = egui::FontId::proportional(11.0);
    let mono = egui::FontId::monospace(10.5);
    let left = rect.left() + 30.0;
    let right = rect.right() - 12.0;
    let high = rect.top() + 34.0;
    let low = rect.bottom() - 46.0;
    let thin = egui::Stroke::new(1.0, t.line2);

    p.text(
        egui::pos2(left, rect.top() + 10.0),
        egui::Align2::LEFT_CENTER,
        "电平",
        small.clone(),
        t.muted,
    );
    for (y, name) in [(high, "H"), (low, "L")] {
        p.hline(left..=right, y, thin);
        p.text(
            egui::pos2(left - 10.0, y),
            egui::Align2::RIGHT_CENTER,
            name,
            mono.clone(),
            t.muted,
        );
    }

    let span = (right - left) / CYCLES as f32;
    let period = c.period_us();
    let (scale, unit) = if period >= 1000.0 {
        (1e-3, "ms")
    } else {
        (1.0, "µs")
    };
    for i in 0..=CYCLES {
        let x = left + i as f32 * span;
        p.vline(x, high - 8.0..=low + 4.0, thin);
        p.text(
            egui::pos2(x, low + 16.0),
            egui::Align2::CENTER_CENTER,
            trim(i as f64 * period * scale, 3),
            mono.clone(),
            t.muted,
        );
    }
    p.text(
        egui::pos2(right, rect.bottom() - 8.0),
        egui::Align2::RIGHT_CENTER,
        format!("时间 / {unit}"),
        small,
        t.muted,
    );

    let points: Vec<egui::Pos2> = trace(c, CYCLES)
        .into_iter()
        .map(|(f, h)| egui::pos2(left + f * (right - left), if h { high } else { low }))
        .collect();
    p.add(egui::Shape::line(points, egui::Stroke::new(1.8, t.accent)));
}

fn log(ui: &mut egui::Ui, p: &mut Pwm) {
    let t = theme::of(ui);
    let mut clear = false;
    let count = p.log.len();
    widgets::panel(
        ui,
        "配置记录",
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
            if p.log.is_empty() {
                ui.label(
                    egui::RichText::new("还没有记录。调好参数后点应用配置，记录显示在这里。")
                        .color(t.muted),
                );
                return;
            }
            let table = Table {
                widths: &[84.0, 58.0, 84.0, 64.0, 70.0, 0.0],
            };
            table.header(ui, &["时间", "通道", "频率", "占空比", "有效电平", "事件"]);
            egui::ScrollArea::vertical()
                .id_salt("pwm_log")
                .max_height(240.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, r) in p.log.iter().enumerate() {
                        let cells = [
                            (clock(r.at), t.muted, true),
                            (label(&CHANNELS, r.cfg.channel).to_owned(), t.fg, true),
                            (hz_text(r.cfg.hz), t.fg, true),
                            (format!("{:.1}%", r.cfg.duty), t.fg, true),
                            (label(&LEVELS, r.cfg.active_high).to_owned(), t.fg2, false),
                            (r.event.clone(), t.fg2, false),
                        ];
                        table.row(ui, &cells, i == 0);
                    }
                });
            widgets::table_foot(
                ui,
                "最新在上 · 第一行为当前应用",
                &format!("最多 {LOG_MAX} 条"),
            );
        },
    );
    if clear {
        p.log.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_follows_duty_and_level() {
        let mut c = Config::default();
        // 25 % edge-aligned, active high: high for the first quarter
        let tr = trace(&c, 1);
        assert_eq!(
            tr,
            vec![(0.0, true), (0.25, true), (0.25, false), (1.0, false)]
        );
        c.active_high = false;
        assert_eq!(trace(&c, 1)[0], (0.0, false));
        c.center = true;
        c.active_high = true;
        c.duty = 50.0;
        let tr = trace(&c, 1);
        assert_eq!(tr[2], (0.25, true));
        assert_eq!(tr[3], (0.75, true));
        // 0 % and 100 % are flat lines, merged over all periods
        c.duty = 0.0;
        assert_eq!(trace(&c, 4), vec![(0.0, false), (1.0, false)]);
        c.duty = 100.0;
        assert_eq!(trace(&c, 4), vec![(0.0, true), (1.0, true)]);
    }

    #[test]
    fn times_and_fields() {
        let mut c = Config::default();
        assert_eq!(micros_text(c.period_us()), "1.000 ms");
        assert_eq!(micros_text(c.high_us()), "250 µs");
        c.active_high = false;
        assert_eq!(micros_text(c.high_us()), "750 µs");
        assert_eq!(hz_text(250_000), "250 kHz");
        assert_eq!(c.short(), "PWM0 · 1000 Hz · 25.0%");
        assert_eq!(parse_hz("2000"), Ok(2000));
        assert!(parse_hz("0").is_err() && parse_hz("1000001").is_err());
        assert_eq!(parse_duty("12.34%"), Ok(12.3));
        assert!(parse_duty("101").is_err());
        assert!(parse_delay("60001").is_err());
    }

    #[test]
    fn apply_logs_what_changed() {
        let mut p = Pwm::default();
        assert!(p.dirty());
        p.apply(Duration::ZERO);
        assert!(!p.dirty());
        assert_eq!(p.log[0].event, "应用配置");
        p.draft.hz = 2000;
        p.draft.duty = 50.0;
        assert!(p.dirty());
        p.apply(Duration::from_secs(1));
        assert_eq!(p.log[0].event, "修改 频率、占空比");
        p.draft.channel = 1;
        p.revert();
        assert_eq!((p.draft.channel, p.hz.as_str()), (0, "2000"));
        assert_eq!(p.log.len(), 2);
    }
}
