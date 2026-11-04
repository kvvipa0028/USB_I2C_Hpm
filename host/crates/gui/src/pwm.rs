//! Combined PWM / frequency-capture workbench. This entire module is a local
//! preview: no worker, Bridge, or USB command is reachable from it. Logical
//! channels and numeric limits describe the editor, not device capabilities.

use std::collections::VecDeque;
use std::fmt::Write;
use std::time::Duration;

use eframe::egui;

use crate::spi::trim;

mod capture;
#[cfg(test)]
mod tests;
mod ui;

use capture::{Capture, CaptureConfig, Reading, Scenario};

const LOG_MAX: usize = 200;
const PREVIEW_MAX_HZ: u32 = 1_000_000;
const MAX_DELAY_MS: u32 = 60_000;
const CHANNELS: [(u8, &str); 2] = [(0, "PWM0 · 待映射"), (1, "PWM1 · 待映射")];
const LEVELS: [(bool, &str); 2] = [(true, "高电平"), (false, "低电平")];
const IDLES: [(Idle, &str); 3] = [
    (Idle::Low, "低电平"),
    (Idle::High, "高电平"),
    (Idle::HiZ, "高阻态"),
];
const ALIGNS: [(bool, &str); 2] = [(false, "边沿对齐"), (true, "中心对齐")];

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
    /// Fraction of the period at the active level, expressed as a percentage.
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
            hz: 10_000,
            duty: 40.0,
            active_high: true,
            idle: Idle::Low,
            center: false,
            delay_ms: 0,
        }
    }
}

impl Config {
    fn period_us(self) -> f64 {
        1e6 / f64::from(self.hz)
    }

    fn high_duty(self) -> f64 {
        if self.active_high {
            f64::from(self.duty)
        } else {
            100.0 - f64::from(self.duty)
        }
    }

    fn high_us(self) -> f64 {
        self.period_us() * self.high_duty() / 100.0
    }
}

#[derive(Clone, Copy, Debug)]
struct Applied {
    cfg: Config,
    revision: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Output {
    #[default]
    Stopped,
    Running,
    Unknown,
}

impl Output {
    fn csv(self) -> &'static str {
        match self {
            Self::Stopped => "stopped_demo",
            Self::Running => "running_demo",
            Self::Unknown => "unknown_demo",
        }
    }
}

struct Record {
    at: Duration,
    event: &'static str,
    applied: Option<Applied>,
    capture: CaptureConfig,
    reading: Option<Reading>,
    output: Output,
    capturing: bool,
    scenario: Scenario,
    compare: bool,
}

struct Comparison {
    hz: f64,
    ppm: f64,
    period_us: f64,
    duty_pp: Option<f64>,
}

pub struct Pwm {
    draft: Config,
    applied: Option<Applied>,
    output: Output,
    saved: Option<(Config, CaptureConfig, bool, Duration)>,
    hz: String,
    duty: String,
    delay: String,
    advanced: bool,
    capture: Capture,
    scenario: Scenario,
    compare: bool,
    /// The applied revision at the instant the current example was generated.
    reading_revision: Option<u64>,
    log: VecDeque<Record>,
    show_log: bool,
    copied_at: Option<Duration>,
}

impl Default for Pwm {
    fn default() -> Self {
        let mut p = Self {
            draft: Config::default(),
            applied: None,
            output: Output::Stopped,
            saved: None,
            hz: String::new(),
            duty: String::new(),
            delay: String::new(),
            advanced: false,
            capture: Capture::default(),
            scenario: Scenario::Signal,
            compare: false,
            reading_revision: None,
            log: VecDeque::new(),
            show_log: false,
            copied_at: None,
        };
        p.sync_text();
        p
    }
}

impl Pwm {
    fn sync_text(&mut self) {
        self.hz = self.draft.hz.to_string();
        self.duty = format!("{:.1}", self.draft.duty);
        self.delay = self.draft.delay_ms.to_string();
    }

    /// Parse even hidden fields. Invalid text is retained after blur and cannot
    /// silently fall back to the previous value when Start/Apply/Save is clicked.
    fn validated_draft(&self) -> Result<Config, String> {
        Ok(Config {
            hz: parse_hz(&self.hz)?,
            duty: parse_duty(&self.duty)?,
            delay_ms: parse_delay(&self.delay)?,
            ..self.draft
        })
    }

    fn dirty(&self) -> bool {
        self.validated_draft()
            .map_or(true, |cfg| self.applied.is_none_or(|a| a.cfg != cfg))
    }

    fn save(&mut self, now: Duration) -> bool {
        let Ok(cfg) = self.validated_draft() else {
            return false;
        };
        self.saved = Some((cfg, self.capture.cfg, self.compare, now));
        true
    }

    fn restore_saved(&mut self, now: Duration) -> bool {
        // Restoring the input settings may not silently reconfigure an active
        // capture session. Output settings are only a draft, even while running.
        if self.capture.running {
            return false;
        }
        let Some((cfg, capture, compare, _)) = self.saved else {
            return false;
        };
        self.draft = cfg;
        self.sync_text();
        if self.capture.cfg != capture {
            self.capture.cfg = capture;
            self.clear_capture(now);
        }
        self.compare = compare;
        true
    }

    fn apply(&mut self, now: Duration) -> bool {
        if self.scenario == Scenario::Interrupted {
            return false;
        }
        let Ok(cfg) = self.validated_draft() else {
            return false;
        };
        if self.applied.is_some_and(|a| a.cfg == cfg) {
            return false;
        }
        let revision = self.applied.map_or(1, |a| a.revision + 1);
        self.draft = cfg;
        self.applied = Some(Applied { cfg, revision });
        self.record(now, "应用配置 · 演示");
        true
    }

    fn revert(&mut self) {
        self.draft = self.applied.map_or_else(Config::default, |a| a.cfg);
        self.sync_text();
    }

    fn start_output(&mut self, now: Duration) {
        if self.scenario == Scenario::Interrupted || self.output == Output::Running {
            return;
        }
        // Starting uses the existing applied snapshot, never an un-applied edit.
        // On first use only, starting also applies the validated initial draft.
        if self.applied.is_none() && !self.apply(now) {
            return;
        }
        self.output = Output::Running;
        self.record(now, "启动输出 · 演示");
    }

    fn stop_output(&mut self, now: Duration) {
        if self.output == Output::Running {
            self.output = Output::Stopped;
            self.record(now, "停止输出 · 演示");
        }
    }

    fn start_capture(&mut self, now: Duration) {
        if self.scenario != Scenario::Interrupted && !self.capture.running {
            self.capture.start(now);
            self.reading_revision = None;
            self.record(now, "开始捕获 · 演示");
        }
    }

    fn stop_capture(&mut self, now: Duration) {
        if self.capture.running {
            self.capture.stop();
            self.reading_revision = None;
            self.record(now, "停止捕获 · 演示");
        }
    }

    fn set_scenario(&mut self, scenario: Scenario, now: Duration) {
        if scenario == self.scenario {
            return;
        }
        self.scenario = scenario;
        self.capture.invalidate(now);
        self.reading_revision = None;
        if scenario == Scenario::Interrupted {
            self.capture.stop();
            if self.output == Output::Running {
                self.output = Output::Unknown;
            }
        }
        // Leaving the interruption scene never resumes either side automatically.
        self.record(now, "切换场景 · 演示");
    }

    fn clear_capture(&mut self, now: Duration) {
        self.capture.clear(now);
        self.reading_revision = None;
        self.log.retain(|r| r.event != "捕获样本 · 演示");
        self.record(now, "清空统计 · 演示");
    }

    /// Called from App on every frame, regardless of which page is selected.
    /// Skips missed windows rather than creating a backlog of synthetic samples.
    pub fn tick(&mut self, now: Duration) -> bool {
        if self.capture.tick(now, self.scenario) {
            self.reading_revision = self.applied.map(|a| a.revision);
            self.record(now, "捕获样本 · 演示");
        }
        self.capture.running
    }

    pub fn running(&self) -> bool {
        self.output == Output::Running || self.capture.running
    }

    fn comparison(&self) -> Option<Comparison> {
        let a = self.applied?;
        let r = self.capture.latest?;
        if !self.compare
            || self.output != Output::Running
            || !self.capture.running
            || self.scenario != Scenario::Signal
            || self.reading_revision != Some(a.revision)
            || a.cfg.duty <= 0.0
            || a.cfg.duty >= 100.0
        {
            return None;
        }
        let hz = r.hz - f64::from(a.cfg.hz);
        Some(Comparison {
            hz,
            ppm: hz / f64::from(a.cfg.hz) * 1e6,
            period_us: r.period_us() - a.cfg.period_us(),
            duty_pp: r.high_duty.map(|d| d - a.cfg.high_duty()),
        })
    }

    fn record(&mut self, at: Duration, event: &'static str) {
        self.log.push_front(Record {
            at,
            event,
            applied: self.applied,
            capture: self.capture.cfg,
            reading: if event == "捕获样本 · 演示" {
                self.capture.latest
            } else {
                None
            },
            output: self.output,
            capturing: self.capture.running,
            scenario: self.scenario,
            compare: self.compare,
        });
        self.log.truncate(LOG_MAX);
    }

    fn csv(&self) -> String {
        let mut out = String::from("source,time_s,event,pwm_revision,pwm_channel,pwm_hz,pwm_active_duty_pct,pwm_active_high,pwm_idle,pwm_center_aligned,pwm_delay_ms,output_state,capture_channel,capture_mode,capture_window_ms,capture_running,scenario,comparison_enabled,example_hz,example_high_duty_pct\n");
        for r in self.log.iter().rev() {
            let cfg = r.applied.map(|a| a.cfg);
            let value = |v: Option<String>| v.unwrap_or_default();
            // Event/status values are controlled constants, never user text.
            writeln!(
                out,
                "local_demo,{:.6},{},{},{},{},{},{},{},{},{},{},CAP{},{},{},{},{},{},{},{}",
                r.at.as_secs_f64(),
                r.event,
                value(r.applied.map(|a| a.revision.to_string())),
                value(cfg.map(|c| format!("PWM{}", c.channel))),
                value(cfg.map(|c| c.hz.to_string())),
                value(cfg.map(|c| format!("{:.1}", c.duty))),
                value(cfg.map(|c| c.active_high.to_string())),
                value(cfg.map(|c| format!("{:?}", c.idle))),
                value(cfg.map(|c| c.center.to_string())),
                value(cfg.map(|c| c.delay_ms.to_string())),
                r.output.csv(),
                r.capture.channel,
                r.capture.mode.csv(),
                r.capture.window_ms,
                r.capturing,
                r.scenario.csv(),
                r.compare,
                value(r.reading.map(|v| format!("{:.3}", v.hz))),
                value(
                    r.reading
                        .and_then(|v| v.high_duty)
                        .map(|v| format!("{v:.3}"))
                ),
            )
            .expect("writing into a String cannot fail");
        }
        out
    }
}

fn parse_hz(s: &str) -> Result<u32, String> {
    s.trim()
        .parse::<u32>()
        .ok()
        .filter(|v| (1..=PREVIEW_MAX_HZ).contains(v))
        .ok_or_else(|| "演示频率填 1 到 1000000 Hz 的整数；此范围不是硬件指标".into())
}

fn parse_duty(s: &str) -> Result<f32, String> {
    s.trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
        .map(|v| (v * 10.0).round() / 10.0)
        .ok_or_else(|| "有效占空比填 0 到 100".into())
}

fn parse_delay(s: &str) -> Result<u32, String> {
    s.trim()
        .parse::<u32>()
        .ok()
        .filter(|v| *v <= MAX_DELAY_MS)
        .ok_or_else(|| format!("演示启动延时填 0 到 {MAX_DELAY_MS} ms"))
}

fn hz_text(hz: f64) -> String {
    if hz >= 1000.0 {
        format!("{} kHz", trim(hz / 1000.0, 3))
    } else {
        format!("{} Hz", trim(hz, 3))
    }
}

pub fn page(ui: &mut egui::Ui, p: &mut Pwm, now: Duration) {
    ui::page(ui, p, now);
}
