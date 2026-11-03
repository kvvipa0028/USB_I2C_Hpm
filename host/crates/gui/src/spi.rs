//! SPI page. Config on the left; the MOSI composer, the selected
//! transaction, and this session's log on the right. Runs in the window
//! only: 执行收发 adds a record to the log and sends nothing over USB, so a
//! record has no MISO bytes.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::{clock, parse_bytes, preview, to_hex};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const LOG_MAX: usize = 200;

const CHANNELS: [(u8, &str); 2] = [(0, "SPI0"), (1, "SPI1")];
const CLOCKS: [(u32, &str); 5] = [
    (500_000, "500 kHz"),
    (1_000_000, "1 MHz"),
    (2_000_000, "2 MHz"),
    (4_000_000, "4 MHz"),
    (8_000_000, "8 MHz"),
];
const MODES: [(u8, &str); 4] = [(0, "Mode 0"), (1, "Mode 1"), (2, "Mode 2"), (3, "Mode 3")];
const CHIP_SELECTS: [(u8, &str); 2] = [(0, "CS0"), (1, "CS1")];
const WORDS: [(u8, &str); 2] = [(8, "8 bit"), (16, "16 bit")];
const ORDERS: [(bool, &str); 2] = [(true, "MSB first"), (false, "LSB first")];
/// Chip select active level: true is active low.
const CS_LEVELS: [(bool, &str); 2] = [(true, "低电平"), (false, "高电平")];

fn label<T: PartialEq>(options: &[(T, &'static str)], v: T) -> &'static str {
    options.iter().find(|o| o.0 == v).map_or("?", |o| o.1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Config {
    channel: u8,
    clock_hz: u32,
    mode: u8,
    cs: u8,
    word: u8,
    msb_first: bool,
    cs_low: bool,
    /// Pads an odd byte count to whole 16-bit words.
    fill: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 0,
            clock_hz: 1_000_000,
            mode: 0,
            cs: 0,
            word: 8,
            msb_first: true,
            cs_low: true,
            fill: 0xff,
        }
    }
}

impl Config {
    /// `SPI0 · CS0 · Mode 0 · 1 MHz · 8 bit · MSB first`
    fn describe(&self) -> String {
        format!(
            "{} · {} · {} · {} · {} · {}",
            label(&CHANNELS, self.channel),
            label(&CHIP_SELECTS, self.cs),
            label(&MODES, self.mode),
            label(&CLOCKS, self.clock_hz),
            label(&WORDS, self.word),
            label(&ORDERS, self.msb_first),
        )
    }
}

struct Record {
    id: u64,
    /// Since the window opened.
    at: Duration,
    cfg: Config,
    mosi: Vec<u8>,
    /// Fill bytes added to make whole words.
    padded: usize,
}

impl Record {
    fn clocks(&self) -> usize {
        self.mosi.len() * 8
    }

    fn micros(&self) -> f64 {
        self.clocks() as f64 * 1e6 / f64::from(self.cfg.clock_hz)
    }
}

pub struct Spi {
    cfg: Config,
    fill: String,
    advanced: bool,
    mosi: String,
    /// Why the last 执行收发 did not record; cleared when the input changes.
    err: Option<String>,
    /// Newest first.
    log: VecDeque<Record>,
    next_id: u64,
    selected: Option<u64>,
}

impl Default for Spi {
    fn default() -> Self {
        Self {
            cfg: Config::default(),
            fill: "FF".into(),
            advanced: false,
            mosi: "9F FF FF FF".into(),
            err: None,
            log: VecDeque::new(),
            next_id: 1,
            selected: None,
        }
    }
}

impl Spi {
    /// Check the MOSI field and log one transaction with the current config.
    fn execute(&mut self, at: Duration) -> Result<(), String> {
        let mut mosi = parse_bytes(&self.mosi)?;
        if mosi.is_empty() {
            return Err("MOSI 为空".into());
        }
        let mut padded = 0;
        if self.cfg.word == 16 && mosi.len() % 2 == 1 {
            mosi.push(self.cfg.fill);
            padded = 1;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.log.push_front(Record {
            id,
            at,
            cfg: self.cfg,
            mosi,
            padded,
        });
        self.log.truncate(LOG_MAX);
        self.selected = Some(id);
        Ok(())
    }

    fn reset(&mut self) {
        self.cfg = Config::default();
        self.fill = "FF".into();
    }

    fn shown(&self) -> Option<&Record> {
        let id = self.selected?;
        self.log.iter().find(|r| r.id == id)
    }
}

/// Bytes as hex, in 16-bit words when the word is 16 bits.
fn words(data: &[u8], word: u8) -> String {
    if word == 16 {
        let w: Vec<String> = data.chunks(2).map(|c| to_hex(c).replace(' ', "")).collect();
        w.join(" ")
    } else {
        to_hex(data)
    }
}

/// `250 µs`, `1.000 ms`, `80 ns`.
pub fn micros_text(us: f64) -> String {
    if us >= 1000.0 {
        format!("{:.3} ms", us / 1000.0)
    } else if us >= 1.0 {
        format!("{} µs", trim(us, 2))
    } else if us > 0.0 {
        format!("{:.0} ns", us * 1000.0)
    } else {
        "0 µs".to_owned()
    }
}

/// At most `places` decimals, without trailing zeros.
pub fn trim(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

pub fn page(ui: &mut egui::Ui, s: &mut Spi, now: Duration) {
    widgets::page_head(
        ui,
        "SPI",
        Some(("本地会话", Tone::Neutral)),
        "配置时钟、模式与片选，以 HEX 编写 MOSI；每笔事务记入本次会话。",
        |ui| {
            let changed = s.cfg != Config::default() || s.fill != "FF";
            if widgets::action_button(ui, changed, "恢复默认").clicked() {
                s.reset();
            }
        },
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::work_columns(ui, s, 264.0, 760.0, config, |ui, s| {
                summary(ui, s);
                ui.add_space(12.0);
                compose(ui, s, now);
                ui.add_space(14.0);
                result(ui, s);
                ui.add_space(14.0);
                log(ui, s);
            });
        });
}

fn config(ui: &mut egui::Ui, s: &mut Spi) {
    let t = theme::of(ui);
    let channel = label(&CHANNELS, s.cfg.channel);
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, channel, Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let c = &mut s.cfg;
            widgets::field_row(ui, "硬件通道", false, |ui| {
                widgets::pick(ui, &mut c.channel, &CHANNELS);
            });
            widgets::field_row(ui, "时钟频率", false, |ui| {
                widgets::pick(ui, &mut c.clock_hz, &CLOCKS);
            });
            widgets::field_row(ui, "时钟模式", false, |ui| {
                widgets::pick(ui, &mut c.mode, &MODES);
            });
            widgets::field_row(ui, "片选引脚", false, |ui| {
                widgets::pick(ui, &mut c.cs, &CHIP_SELECTS);
            });
            widgets::field_row(ui, "字长", false, |ui| {
                widgets::pick(ui, &mut c.word, &WORDS);
            });
            widgets::disclosure(ui, &mut s.advanced, "高级参数", 3);
            if s.advanced {
                widgets::hairline(ui);
                widgets::field_row(ui, "位序", false, |ui| {
                    widgets::pick(ui, &mut c.msb_first, &ORDERS);
                });
                widgets::field_row(ui, "片选有效", false, |ui| {
                    widgets::pick(ui, &mut c.cs_low, &CS_LEVELS);
                });
                let mut fill_err = None;
                widgets::field_row(ui, "填充字节", true, |ui| {
                    widgets::unit_edit(ui, &mut s.fill, "FF", "");
                    match parse_bytes(&s.fill).ok().filter(|b| b.len() == 1) {
                        Some(b) => c.fill = b[0],
                        None => fill_err = Some("填一个字节，如 FF"),
                    }
                });
                if let Some(e) = fill_err {
                    ui.label(egui::RichText::new(e).size(11.5).color(t.red));
                }
            }
        },
    );
    ui.add_space(8.0);
    widgets::note(
        ui,
        "一次事务内片选保持有效，MISO 与 MOSI 等长。16 bit 字长下奇数个字节用填充字节补齐。",
    );
}

fn summary(ui: &mut egui::Ui, s: &Spi) {
    let bytes: usize = s.log.iter().map(|r| r.mosi.len()).sum();
    // an empty float `sum` is -0.0
    let us = s.log.iter().fold(0.0, |acc, r| acc + r.micros());
    widgets::summary_strip(
        ui,
        &[
            ("事务", s.log.len().to_string()),
            ("MOSI 字节", format!("{bytes} B")),
            ("总线时间", micros_text(us)),
        ],
        "本次会话",
    );
}

fn compose(ui: &mut egui::Ui, s: &mut Spi, now: Duration) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "发送 MOSI",
        |ui| {
            ui.label(egui::RichText::new("HEX").size(11.0).color(t.muted));
        },
        |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("MOSI 数据").size(12.0).color(t.muted));
                widgets::trailing(ui, 18.0, |ui| {
                    let (text, color) = match parse_bytes(&s.mosi) {
                        Ok(b) => (format!("{} / 256 B", b.len()), t.muted),
                        Err(e) => (e, t.red),
                    };
                    ui.label(egui::RichText::new(text).size(11.5).color(color));
                });
            });
            let r = ui.add(
                egui::TextEdit::multiline(&mut s.mosi)
                    .hint_text("9F FF FF FF")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            if r.changed() {
                s.err = None;
            }
            ui.horizontal(|ui| {
                if widgets::ink_button(ui, true, "执行收发").clicked() {
                    s.err = s.execute(now).err();
                }
                widgets::badge(
                    ui,
                    &format!(
                        "{} · {} · {}",
                        label(&CHIP_SELECTS, s.cfg.cs),
                        label(&MODES, s.cfg.mode),
                        label(&CLOCKS, s.cfg.clock_hz)
                    ),
                    Tone::Neutral,
                );
                if let Some(e) = &s.err {
                    ui.label(egui::RichText::new(e).size(11.5).color(t.red));
                }
            });
        },
    );
}

fn result(ui: &mut egui::Ui, s: &Spi) {
    let t = theme::of(ui);
    let rec = s.shown();
    widgets::panel(
        ui,
        "事务详情",
        |ui| {
            let Some(r) = rec else { return };
            if widgets::quiet_button(ui, true, "复制 MOSI").clicked() {
                ui.ctx().copy_text(to_hex(&r.mosi));
            }
            widgets::badge(ui, "本地", Tone::Neutral);
        },
        |ui| {
            let Some(r) = rec else {
                ui.label(
                    egui::RichText::new("还没有事务。填写 MOSI 后点执行收发，详情显示在这里。")
                        .color(t.muted),
                );
                return;
            };
            ui.label(
                egui::RichText::new(format!(
                    "#{}  {}  ·  {}",
                    r.id,
                    clock(r.at),
                    r.cfg.describe()
                ))
                .size(11.5)
                .color(t.muted),
            );
            ui.spacing_mut().item_spacing.y = 4.0;
            widgets::field_row(ui, "MOSI", false, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(words(&r.mosi, r.cfg.word))
                            .monospace()
                            .color(t.fg),
                    )
                    .wrap()
                    .selectable(true),
                );
            });
            widgets::field_row(ui, "MISO", false, |ui| {
                ui.label(egui::RichText::new("—").monospace().color(t.faint));
                ui.label(
                    egui::RichText::new("本地记录，无回读数据")
                        .size(11.5)
                        .color(t.muted),
                );
            });
            widgets::field_row(ui, "时序", true, |ui| {
                let cpol = r.cfg.mode >> 1;
                let cpha = r.cfg.mode & 1;
                let mut text = format!(
                    "{} 个时钟 · 约 {} · CPOL {cpol} CPHA {cpha} · 片选{}有效",
                    r.clocks(),
                    micros_text(r.micros()),
                    label(&CS_LEVELS, r.cfg.cs_low)
                );
                if r.padded > 0 {
                    text.push_str(&format!(" · 末尾补 {:02X}", r.cfg.fill));
                }
                ui.label(egui::RichText::new(text).size(12.0).color(t.fg2));
            });
        },
    );
}

fn log(ui: &mut egui::Ui, s: &mut Spi) {
    let t = theme::of(ui);
    let mut clear = false;
    let mut pick = None;
    let count = s.log.len();
    widgets::panel(
        ui,
        "事务记录",
        |ui| {
            ui.label(
                egui::RichText::new(format!("{count} 条 · 本次会话"))
                    .size(11.0)
                    .color(t.muted),
            );
        },
        |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} · 时间 / HEX",
                        label(&CHANNELS, s.cfg.channel)
                    ))
                    .size(11.5)
                    .color(t.muted),
                );
                widgets::trailing(ui, 27.0, |ui| {
                    if widgets::quiet_button(ui, count > 0, "清空").clicked() {
                        clear = true;
                    }
                });
            });
            if s.log.is_empty() {
                ui.label(egui::RichText::new("暂无记录。").color(t.muted));
                return;
            }
            let table = Table {
                widths: &[84.0, 50.0, 50.0, 0.0, 0.0, 64.0],
            };
            table.header(ui, &["时间", "片选", "长度", "MOSI", "MISO", "结果"]);
            egui::ScrollArea::vertical()
                .id_salt("spi_log")
                .max_height(260.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for r in &s.log {
                        let cells = [
                            (clock(r.at), t.muted, true),
                            (label(&CHIP_SELECTS, r.cfg.cs).to_owned(), t.fg, true),
                            (r.mosi.len().to_string(), t.fg, true),
                            (preview(&r.mosi), t.fg2, true),
                            ("—".to_owned(), t.faint, true),
                            ("本地".to_owned(), t.muted, false),
                        ];
                        if table.row(ui, &cells, s.selected == Some(r.id)).clicked() {
                            pick = Some(r.id);
                        }
                    }
                });
            widgets::table_foot(ui, "最新在上 · 点行查看详情", &format!("最多 {LOG_MAX} 条"));
        },
    );
    if clear {
        s.log.clear();
        s.selected = None;
    }
    if pick.is_some() {
        s.selected = pick;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_logs_newest_first() {
        let mut s = Spi::default();
        s.execute(Duration::from_millis(5)).unwrap();
        let r = s.shown().unwrap();
        assert_eq!((r.id, r.mosi.len(), r.clocks()), (1, 4, 32));
        assert_eq!(micros_text(r.micros()), "32 µs");
        s.mosi = "zz".into();
        assert!(s.execute(Duration::ZERO).is_err());
        s.mosi = " ".into();
        assert_eq!(s.execute(Duration::ZERO).unwrap_err(), "MOSI 为空");
        assert_eq!(s.log.len(), 1);
        for _ in 0..LOG_MAX + 3 {
            s.mosi = "01".into();
            s.execute(Duration::ZERO).unwrap();
        }
        assert_eq!(s.log.len(), LOG_MAX);
        assert_eq!(s.shown().unwrap().id, (LOG_MAX + 4) as u64);
    }

    #[test]
    fn sixteen_bit_pads_with_fill() {
        let mut s = Spi::default();
        s.cfg.word = 16;
        s.cfg.fill = 0xa5;
        s.mosi = "12 34 56".into();
        s.execute(Duration::ZERO).unwrap();
        let r = s.shown().unwrap();
        assert_eq!(
            (r.mosi.clone(), r.padded),
            (vec![0x12, 0x34, 0x56, 0xa5], 1)
        );
        assert_eq!(words(&r.mosi, 16), "1234 56a5");
        assert_eq!(words(&r.mosi, 8), "12 34 56 a5");
    }

    #[test]
    fn durations_read_well() {
        assert_eq!(micros_text(1000.0), "1.000 ms");
        assert_eq!(micros_text(250.0), "250 µs");
        assert_eq!(micros_text(2.5), "2.5 µs");
        assert_eq!(micros_text(0.08), "80 ns");
        assert_eq!(micros_text(0.0), "0 µs");
        assert_eq!(trim(3.0, 3), "3");
        assert_eq!(trim(0.125, 3), "0.125");
        assert_eq!(
            Config::default().describe(),
            "SPI0 · CS0 · Mode 0 · 1 MHz · 8 bit · MSB first"
        );
    }
}
