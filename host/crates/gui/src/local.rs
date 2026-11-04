//! What the window-only pages share: the 保存配置 snapshot, a tick counter
//! for their demo generators, a small repeatable noise source, and number
//! parsing for their text fields. Nothing here touches USB.

use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::clock;
use crate::theme;
use crate::widgets::{self, Tone};

/// Rows a page log keeps; older ones drop off.
pub const LOG_MAX: usize = 200;
/// How often a running demo generator wants a frame.
pub const TICK: Duration = Duration::from_millis(50);
pub const SAVE_HOVER: &str = "保存在本窗口，关闭后不保留";

/// Page-head badge: what the page is, and whether its demo is running.
pub fn badge(running: Option<&'static str>) -> (&'static str, Tone) {
    match running {
        Some(text) => (text, Tone::Green),
        None => ("本地演示", Tone::Neutral),
    }
}

/// 保存配置: a snapshot of a page's config, kept in the window only.
pub struct Saved<C> {
    snap: Option<(C, Duration)>,
}

impl<C> Default for Saved<C> {
    fn default() -> Self {
        Self { snap: None }
    }
}

impl<C: Clone + PartialEq> Saved<C> {
    pub fn dirty(&self, c: &C) -> bool {
        self.snap.as_ref().is_none_or(|(s, _)| s != c)
    }

    pub fn save(&mut self, c: &C, at: Duration) {
        self.snap = Some((c.clone(), at));
    }

    /// The page-head 保存配置 button.
    pub fn button(&mut self, ui: &mut egui::Ui, c: &C, now: Duration) {
        if widgets::action_button(ui, self.dirty(c), "保存配置")
            .on_hover_text(SAVE_HOVER)
            .clicked()
        {
            self.save(c, now);
        }
    }

    /// Config card foot: whether the draft matches the snapshot, and 撤销修改
    /// to go back to it. Returns true when the draft was put back.
    pub fn foot(&self, ui: &mut egui::Ui, c: &mut C) -> bool {
        let t = theme::of(ui);
        let dirty = self.dirty(c);
        let mut back = false;
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            let (status, color) = match (&self.snap, dirty) {
                (None, _) => ("尚未保存".to_owned(), t.muted),
                (Some(_), true) => ("参数有修改".to_owned(), t.amber),
                (Some((_, at)), false) => (format!("已保存 · {}", clock(*at)), t.muted),
            };
            ui.label(egui::RichText::new(status).size(12.0).color(color));
            widgets::trailing(ui, 27.0, |ui| {
                let can = dirty && self.snap.is_some();
                if widgets::quiet_button(ui, can, "撤销修改").clicked() {
                    if let Some((s, _)) = &self.snap {
                        *c = s.clone();
                        back = true;
                    }
                }
            });
        });
        back
    }
}

/// Counts whole periods for a demo generator. After a long gap (the window
/// was hidden) it skips ahead instead of making up every missed period.
pub struct Ticker {
    last: Duration,
}

impl Ticker {
    pub fn new(now: Duration) -> Self {
        Self { last: now }
    }

    /// Periods passed since the last call, at most `cap`; the time of each
    /// is `at(i)` for i in 0..n.
    pub fn take(&mut self, now: Duration, period: Duration, cap: u32) -> Vec<Duration> {
        let period = period.max(Duration::from_millis(1));
        let n = (now.saturating_sub(self.last).as_nanos() / period.as_nanos()) as u32;
        if n > cap {
            self.last = now;
            return vec![now];
        }
        (0..n)
            .map(|_| {
                self.last += period;
                self.last
            })
            .collect()
    }
}

/// xorshift64*: the same seed gives the same demo values every run.
pub struct Noise(u64);

impl Noise {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in -1..1.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// `0x7E0`, `7e0`, or `0X7E0` up to `max`.
pub fn parse_hex(s: &str, max: u32, what: &str) -> Result<u32, String> {
    let s = s.trim();
    let h = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    match u32::from_str_radix(h, 16) {
        Ok(v) if v <= max => Ok(v),
        Ok(_) => Err(format!("{what}最大 0x{max:X}")),
        Err(_) => Err(format!("{what}填十六进制，如 0x{max:X}")),
    }
}

/// A whole number in `lo..=hi`.
pub fn parse_num(s: &str, lo: u32, hi: u32, what: &str) -> Result<u32, String> {
    s.trim()
        .parse::<u32>()
        .ok()
        .filter(|v| (lo..=hi).contains(v))
        .ok_or_else(|| format!("{what}填 {lo} 到 {hi} 的整数"))
}

/// A decimal number in `lo..=hi`.
pub fn parse_real(s: &str, lo: f64, hi: f64, what: &str) -> Result<f64, String> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| (lo..=hi).contains(v))
        .ok_or_else(|| format!("{what}填 {lo} 到 {hi}"))
}

/// Label of `v` in a pick list.
pub fn label<T: PartialEq>(options: &[(T, &'static str)], v: T) -> &'static str {
    options.iter().find(|o| o.0 == v).map_or("?", |o| o.1)
}

/// Upper-case hex bytes, `01 02 0A`.
pub fn hex_upper(data: &[u8]) -> String {
    let v: Vec<String> = data.iter().map(|b| format!("{b:02X}")).collect();
    v.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_tracks_the_snapshot() {
        let mut s = Saved::default();
        assert!(s.dirty(&1));
        s.save(&1, Duration::ZERO);
        assert!(!s.dirty(&1));
        assert!(s.dirty(&2));
    }

    #[test]
    fn ticker_counts_and_skips_ahead() {
        let ms = Duration::from_millis;
        let mut t = Ticker::new(Duration::ZERO);
        assert_eq!(t.take(ms(120), ms(50), 10), vec![ms(50), ms(100)]);
        assert!(t.take(ms(140), ms(50), 10).is_empty());
        assert_eq!(t.take(ms(150), ms(50), 10), vec![ms(150)]);
        // a long gap gives one tick now, not a burst
        assert_eq!(t.take(ms(60_000), ms(50), 10), vec![ms(60_000)]);
    }

    #[test]
    fn noise_repeats_and_stays_in_range() {
        let (mut a, mut b) = (Noise::new(7), Noise::new(7));
        for _ in 0..1000 {
            let v = a.unit();
            assert_eq!(v, b.unit());
            assert!((-1.0..1.0).contains(&v));
        }
    }

    #[test]
    fn numbers_parse() {
        assert_eq!(parse_hex("0x7E0", 0x7ff, "ID"), Ok(0x7e0));
        assert_eq!(parse_hex("7e0", 0x7ff, "ID"), Ok(0x7e0));
        assert_eq!(parse_hex("0x800", 0x7ff, "ID").unwrap_err(), "ID最大 0x7FF");
        assert!(parse_hex("zz", 0xff, "ID").is_err());
        assert_eq!(parse_num("100", 1, 1000, "周期"), Ok(100));
        assert!(parse_num("0", 1, 1000, "周期").is_err());
        assert_eq!(parse_real("3.3", 0.1, 6.0, "参考电压"), Ok(3.3));
        assert_eq!(hex_upper(&[1, 0xab]), "01 AB");
    }
}
