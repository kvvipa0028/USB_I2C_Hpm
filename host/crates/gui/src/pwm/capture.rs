//! Independent, explicitly synthetic capture source. Fixed example readings do
//! not track PWM settings or output state; there is no simulated electrical link.

use std::time::Duration;

pub(super) const CHANNELS: [(u8, &str); 2] = [(0, "CAP0 · 待映射"), (1, "CAP1 · 待映射")];
pub(super) const WINDOWS: [(u32, &str); 3] = [(100, "100 ms"), (250, "250 ms"), (1000, "1000 ms")];
pub(super) const MODES: [(Mode, &str); 2] =
    [(Mode::Both, "频率 + 占空比"), (Mode::Frequency, "仅频率")];
pub(super) const SCENARIOS: [(Scenario, &str); 3] = [
    (Scenario::Signal, "正常捕获示例"),
    (Scenario::NoSignal, "无信号示例"),
    (Scenario::Interrupted, "连接中断示例"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    Both,
    Frequency,
}
impl Mode {
    pub(super) fn csv(self) -> &'static str {
        match self {
            Self::Both => "frequency_and_duty",
            Self::Frequency => "frequency_only",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Scenario {
    Signal,
    NoSignal,
    Interrupted,
}
impl Scenario {
    pub(super) fn csv(self) -> &'static str {
        match self {
            Self::Signal => "signal_demo",
            Self::NoSignal => "no_signal_demo",
            Self::Interrupted => "interrupted_demo",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct CaptureConfig {
    pub channel: u8,
    pub mode: Mode,
    pub window_ms: u32,
}
impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            channel: 0,
            mode: Mode::Both,
            window_ms: 100,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Reading {
    pub at: Duration,
    pub hz: f64,
    /// HIGH duty, regardless of the separate output's active polarity.
    pub high_duty: Option<f64>,
}
impl Reading {
    pub fn period_us(self) -> f64 {
        1e6 / self.hz
    }
    pub fn high_us(self) -> Option<f64> {
        self.high_duty.map(|d| self.period_us() * d / 100.0)
    }
}

#[derive(Default)]
pub(super) struct Capture {
    pub cfg: CaptureConfig,
    pub running: bool,
    pub latest: Option<Reading>,
    /// Number of displayed valid example windows since Start/Clear, not edges.
    pub samples: u64,
    last: Duration,
}
impl Capture {
    pub fn start(&mut self, now: Duration) {
        self.running = true;
        self.clear(now);
    }
    pub fn stop(&mut self) {
        self.running = false;
        self.latest = None;
    }
    pub fn invalidate(&mut self, now: Duration) {
        self.latest = None;
        self.last = now;
    }
    pub fn clear(&mut self, now: Duration) {
        self.samples = 0;
        self.invalidate(now);
    }
    pub fn tick(&mut self, now: Duration, scenario: Scenario) -> bool {
        if !self.running {
            return false;
        }
        if scenario != Scenario::Signal {
            self.invalidate(now);
            return false;
        }
        if now.saturating_sub(self.last) < Duration::from_millis(u64::from(self.cfg.window_ms)) {
            return false;
        }
        self.last = now;
        self.samples = self.samples.saturating_add(1);
        self.latest = Some(Reading {
            at: now,
            hz: 9998.0,
            high_duty: (self.cfg.mode == Mode::Both).then_some(39.98),
        });
        true
    }
}
