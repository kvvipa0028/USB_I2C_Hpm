//! EEPROM page: 配置与检查 → 执行任务 → 验证与报告. The check builds a plan
//! (read a range, or write bytes split at page boundaries) against a demo
//! image of the chip kept in the window; the task steps through it one
//! click at a time and the report compares what reads back. Nothing goes
//! over USB: the I2C master page is the place for real transfers.

use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::{clock, parse_addr, parse_bytes};
use crate::job::{self, Step, Task};
use crate::local::{label, parse_hex, parse_num, Saved};
use crate::theme;
use crate::widgets::{self, Tone};

const CHANNELS: [(u8, &str); 2] = [(1, "I2C1"), (2, "I2C2")];
const CAPACITIES: [(u32, &str); 8] = [
    (128, "128 B · 24C01"),
    (256, "256 B · 24C02"),
    (512, "512 B · 24C04"),
    (1024, "1024 B · 24C08"),
    (2048, "2048 B · 24C16"),
    (4096, "4096 B · 24C32"),
    (8192, "8192 B · 24C64"),
    (32768, "32768 B · 24C256"),
];
/// Word address width: true is 16 bit.
const WIDTHS: [(bool, &str); 2] = [(false, "8 bit"), (true, "16 bit")];
const PAGES: [(u32, &str); 5] = [
    (8, "8 B"),
    (16, "16 B"),
    (32, "32 B"),
    (64, "64 B"),
    (128, "128 B"),
];
/// Completion check: true is ACK polling.
const DONES: [(bool, &str); 2] = [(true, "ACK 轮询"), (false, "固定等待 5 ms")];
const VERIFIES: [(u8, &str); 1] = [(0, "读回比较")];
/// Bytes per page of the address grid.
const VIEW: u32 = 128;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Read,
    Write,
}

#[derive(Clone, PartialEq, Debug)]
struct Config {
    channel: u8,
    addr: String,
    capacity: u32,
    start: String,
    len: String,
    wide: bool,
    page: u32,
    ack_poll: bool,
    verify: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 1,
            addr: "0x50".into(),
            capacity: 8192,
            start: "0x0000".into(),
            len: "128".into(),
            wide: true,
            page: 32,
            ack_poll: true,
            verify: 0,
        }
    }
}

/// What a checked config will do.
#[derive(Clone, Debug, PartialEq)]
struct Plan {
    mode: Mode,
    addr: u8,
    start: u32,
    len: u32,
    data: Vec<u8>,
    /// Write chunks that keep to page boundaries, (start, len).
    chunks: Vec<(u32, u32)>,
}

impl Plan {
    fn stages(&self, ack_poll: bool) -> Vec<String> {
        match self.mode {
            Mode::Read => vec![
                "读取指定地址范围".into(),
                "整理读出缓冲".into(),
                "核对读取长度与地址范围".into(),
            ],
            Mode::Write => vec![
                format!("按页拆分写入 · {} 段", self.chunks.len()),
                format!("等待写入完成 · {}", label(&DONES, ack_poll)),
                "读回比较".into(),
            ],
        }
    }
}

/// `len` bytes from `start` cut where a `page`-byte page ends.
fn chunks(start: u32, len: u32, page: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let (mut at, end) = (start, start + len);
    while at < end {
        let next = ((at / page) + 1) * page;
        let n = next.min(end) - at;
        out.push((at, n));
        at += n;
    }
    out
}

/// The demo image: the design's first 128 bytes, then erased (FF).
fn image(capacity: u32) -> Vec<u8> {
    let mut m = vec![0xff; capacity as usize];
    let head = b"BusStudio\x00\x01\x02\x03\x04\x05\x06";
    for (i, b) in m.iter_mut().take(128).enumerate() {
        *b = if i < head.len() { head[i] } else { i as u8 };
    }
    m
}

struct Report {
    plan: Plan,
    cfg: Config,
    readback: Vec<u8>,
    mismatches: usize,
    complete: bool,
    ended: Duration,
}

pub struct Eeprom {
    cfg: Config,
    saved: Saved<Config>,
    advanced: bool,
    mode: Mode,
    data: String,
    mem: Vec<u8>,
    view: u32,
    step: Step,
    task: Option<Task>,
    plan: Option<Plan>,
    report: Option<Report>,
    snapshot: bool,
    /// Why 检查并进入执行 refused, until the config changes.
    refused: Vec<String>,
}

impl Default for Eeprom {
    fn default() -> Self {
        let cfg = Config::default();
        Self {
            mem: image(cfg.capacity),
            cfg,
            saved: Saved::default(),
            advanced: false,
            mode: Mode::Read,
            data: "42 75 73 53 74 75 64 69 6F".into(),
            view: 0,
            step: Step::Check,
            task: None,
            plan: None,
            report: None,
            snapshot: true,
            refused: Vec::new(),
        }
    }
}

impl Eeprom {
    pub fn running(&self) -> bool {
        job::is_running(&self.task)
    }

    fn check(&self) -> Result<Plan, Vec<String>> {
        let c = &self.cfg;
        let mut errs = Vec::new();
        let addr = parse_addr(&c.addr).map_err(|e| errs.push(e)).ok();
        let start = parse_hex(&c.start, c.capacity - 1, "起始地址")
            .map_err(|e| errs.push(e))
            .ok();
        if !c.wide && c.capacity > 256 {
            errs.push("8 bit 地址只覆盖 256 B，容量更大时选 16 bit".into());
        }
        let (len, data) = match self.mode {
            Mode::Read => (
                parse_num(&c.len, 1, c.capacity, "操作长度")
                    .map_err(|e| errs.push(e))
                    .ok(),
                Vec::new(),
            ),
            Mode::Write => match parse_bytes(&self.data) {
                Ok(d) if d.is_empty() => {
                    errs.push("写入数据为空".into());
                    (None, d)
                }
                Ok(d) => (Some(d.len() as u32), d),
                Err(e) => {
                    errs.push(e);
                    (None, Vec::new())
                }
            },
        };
        if let (Some(s), Some(n)) = (start, len) {
            if s + n > c.capacity {
                errs.push(format!("0x{s:04X} 起 {n} B 超出容量 {} B", c.capacity));
            }
        }
        match (addr, start, len) {
            (Some(addr), Some(start), Some(len)) if errs.is_empty() => Ok(Plan {
                mode: self.mode,
                addr,
                start,
                len,
                chunks: chunks(start, len, c.page),
                data,
            }),
            _ => Err(errs),
        }
    }

    fn start(&mut self, now: Duration) {
        match self.check() {
            Ok(plan) => {
                self.task = Some(Task::new(plan.stages(self.cfg.ack_poll), now));
                self.plan = Some(plan);
                self.report = None;
                self.step = Step::Run;
                self.refused.clear();
            }
            Err(e) => self.refused = e,
        }
    }

    /// A stage finished: writes land in the image after the first stage,
    /// the report is made after the last.
    fn on_stage(&mut self, i: usize, now: Duration) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        let (s, n) = (plan.start as usize, plan.len as usize);
        if plan.mode == Mode::Write && i == 0 {
            self.mem[s..s + n].copy_from_slice(&plan.data);
        }
        if self.task.as_ref().is_some_and(|t| !t.running()) {
            self.finish(now);
        }
    }

    fn finish(&mut self, now: Duration) {
        let (Some(plan), Some(task)) = (self.plan.clone(), self.task.as_ref()) else {
            return;
        };
        let (s, n) = (plan.start as usize, plan.len as usize);
        let readback = self.mem[s..s + n].to_vec();
        let mismatches = match plan.mode {
            Mode::Read => 0,
            Mode::Write => readback
                .iter()
                .zip(&plan.data)
                .filter(|(a, b)| a != b)
                .count(),
        };
        self.report = Some(Report {
            plan,
            cfg: self.cfg.clone(),
            readback,
            mismatches,
            complete: task.state == job::State::Done,
            ended: now,
        });
    }

    fn set_capacity(&mut self) {
        if self.mem.len() != self.cfg.capacity as usize {
            self.mem.resize(self.cfg.capacity as usize, 0xff);
            self.view = self.view.min(self.cfg.capacity.saturating_sub(1) / VIEW);
        }
    }
}

pub fn page(ui: &mut egui::Ui, e: &mut Eeprom, now: Duration) {
    widgets::page_head(
        ui,
        "EEPROM 读写",
        Some(job::badge(&e.task)),
        "按地址查看内容，分页写入并比较读回结果。存储内容为演示镜像，不经 USB。",
        |ui| {
            let was_running = e.running();
            if job::head_button(ui, &mut e.step, &mut e.task, now) {
                e.start(now);
            }
            if was_running && !e.running() {
                e.finish(now);
            }
            e.saved.button(ui, &e.cfg, now);
        },
    );
    ui.add_space(4.0);
    job::step_bar(ui, &mut e.step, e.task.as_ref());
    ui.add_space(10.0);
    e.set_capacity();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match e.step {
            Step::Check => {
                widgets::work_columns(ui, e, 264.0, 760.0, config, |ui, e| {
                    plan_card(ui, e, now);
                    ui.add_space(14.0);
                    memory(ui, e);
                });
            }
            Step::Run => run(ui, e, now),
            Step::Report => report(ui, e),
        });
}

fn config(ui: &mut egui::Ui, e: &mut Eeprom) {
    let channel = label(&CHANNELS, e.cfg.channel);
    let before = e.cfg.clone();
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, channel, Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let c = &mut e.cfg;
            widgets::field_row(ui, "接口 / 通道", false, |ui| {
                widgets::pick(ui, &mut c.channel, &CHANNELS);
            });
            widgets::field_row(ui, "器件地址", false, |ui| {
                widgets::unit_edit(ui, &mut c.addr, "0x50", "");
            });
            widgets::field_row(ui, "容量配置", false, |ui| {
                widgets::pick(ui, &mut c.capacity, &CAPACITIES);
            });
            widgets::field_row(ui, "起始地址", false, |ui| {
                widgets::unit_edit(ui, &mut c.start, "0x0000", "");
            });
            widgets::field_row(ui, "操作长度", false, |ui| {
                widgets::unit_edit(ui, &mut c.len, "128", "B");
            });
            widgets::disclosure(ui, &mut e.advanced, "高级参数", 4);
            widgets::hairline(ui);
            if e.advanced {
                widgets::field_row(ui, "地址宽度", false, |ui| {
                    widgets::pick(ui, &mut c.wide, &WIDTHS);
                });
                widgets::field_row(ui, "写入页大小", false, |ui| {
                    widgets::pick(ui, &mut c.page, &PAGES);
                });
                widgets::field_row(ui, "完成检测", false, |ui| {
                    widgets::pick(ui, &mut c.ack_poll, &DONES);
                });
                widgets::field_row(ui, "写后验证", true, |ui| {
                    widgets::pick(ui, &mut c.verify, &VERIFIES);
                });
            }
            e.saved.foot(ui, &mut e.cfg);
        },
    );
    if e.cfg != before {
        e.refused.clear();
    }
    ui.add_space(8.0);
    widgets::note(ui, "容量、地址组织与页大小必须匹配具体器件。");
    widgets::note(ui, "跨页自动拆分；写入前展示覆盖范围，写后读回校验。");
}

fn plan_card(ui: &mut egui::Ui, e: &mut Eeprom, now: Duration) {
    let t = theme::of(ui);
    let mut mode = e.mode;
    let checked = e.check();
    widgets::panel(
        ui,
        "作业计划",
        |ui| {
            widgets::seg(
                ui,
                &mut mode,
                &[(Mode::Read, "读取"), (Mode::Write, "写入")],
            );
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let (name, note, tag) = match e.mode {
                Mode::Read => (
                    "eeprom-demo.bin",
                    format!("导出文件示例 · {} B 操作范围", e.cfg.len.trim()),
                    "读取",
                ),
                Mode::Write => (
                    "写入数据",
                    format!("HEX · {} B", parse_bytes(&e.data).map_or(0, |d| d.len())),
                    "写入",
                ),
            };
            job::file_row(ui, name, &note, tag, |_| {});
            widgets::hairline(ui);
            if e.mode == Mode::Write {
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut e.data)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(2),
                    )
                    .changed()
                {
                    e.refused.clear();
                }
                ui.add_space(4.0);
            }
            let addr = parse_addr(&e.cfg.addr).map_or("—".into(), |a| format!("0x{a:02X}"));
            job::line(ui, "目标", &format!("EEPROM · 地址 {addr}"), false, false);
            job::line(ui, "通道", label(&CHANNELS, e.cfg.channel), false, false);
            match &checked {
                Ok(p) => {
                    let key = if p.mode == Mode::Read {
                        "读取范围"
                    } else {
                        "写入范围"
                    };
                    job::line(ui, key, &job::range_text(p.start, p.len, 4), true, false);
                    if p.mode == Mode::Write {
                        let cut: Vec<String> = p
                            .chunks
                            .iter()
                            .map(|&(s, n)| format!("0x{s:04X}+{n}"))
                            .collect();
                        job::line(
                            ui,
                            "分页",
                            &format!("页大小 {} B · {}", e.cfg.page, cut.join("  ")),
                            true,
                            false,
                        );
                    }
                    job::plan(ui, &p.stages(e.cfg.ack_poll));
                }
                Err(_) => {
                    job::line(ui, "操作范围", "—", false, false);
                }
            }
            let problems = match &checked {
                Ok(_) if e.refused.is_empty() => Vec::new(),
                Ok(_) => e.refused.clone(),
                Err(errs) => errs.clone(),
            };
            if job::check_foot(ui, &problems, e.running()) {
                e.start(now);
            }
            ui.label(
                egui::RichText::new("执行只推进本页演示，不向设备发送 I2C 事务。")
                    .size(11.5)
                    .color(t.muted),
            );
        },
    );
    if mode != e.mode {
        e.mode = mode;
        e.refused.clear();
    }
}

fn memory(ui: &mut egui::Ui, e: &mut Eeprom) {
    let pages = e.cfg.capacity.div_ceil(VIEW);
    let base = e.view * VIEW;
    let plan = e.check().ok();
    widgets::panel(
        ui,
        "存储内容",
        |ui| {
            widgets::badge(ui, "演示镜像", Tone::Neutral);
        },
        |ui| {
            ui.horizontal(|ui| {
                widgets::muted(
                    ui,
                    &format!(
                        "{} · 第 {} / {pages} 页",
                        job::range_text(base, VIEW.min(e.cfg.capacity), 4),
                        e.view + 1
                    ),
                );
                widgets::trailing(ui, 27.0, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::quiet_button(ui, e.view + 1 < pages, "下一页 ›").clicked() {
                        e.view += 1;
                    }
                    if widgets::quiet_button(ui, e.view > 0, "‹ 上一页").clicked() {
                        e.view -= 1;
                    }
                    if let Some(p) = &plan {
                        if widgets::quiet_button(ui, true, "跳到操作范围").clicked() {
                            e.view = p.start / VIEW;
                        }
                    }
                });
            });
            let end = (base + VIEW).min(e.cfg.capacity) as usize;
            let range = plan.as_ref().map(|p| p.start..p.start + p.len);
            job::hex_rows(ui, base, &e.mem[base as usize..end], |a| {
                range.as_ref().is_some_and(|r| r.contains(&a))
            });
            widgets::table_foot(ui, "软底为本次操作范围", "演示镜像 · 关闭后不保留");
        },
    );
}

fn run(ui: &mut egui::Ui, e: &mut Eeprom, now: Duration) {
    let (Some(task), Some(plan)) = (e.task.as_mut(), e.plan.as_ref()) else {
        widgets::muted(ui, "还没有任务。在配置与检查里点检查并进入执行。");
        return;
    };
    let (tag, what) = match plan.mode {
        Mode::Read => ("读取", "EEPROM · 读取"),
        Mode::Write => ("写入", "EEPROM · 写入"),
    };
    let note = format!(
        "{} · {} B 操作范围",
        job::range_text(plan.start, plan.len, 4),
        plan.len
    );
    let finished = job::run_card(ui, ("eeprom-demo.bin", &note, tag), what, task, now);
    let ended = !task.running();
    let summary = format!(
        "已完成 {} / {} 个阶段；整体进度 {:.0}%。",
        task.done,
        task.stages.len(),
        task.frac() * 100.0
    );
    ui.add_space(8.0);
    widgets::note(ui, &summary);
    if let Some(i) = finished {
        e.on_stage(i, now);
    } else if ended && e.report.is_none() {
        e.finish(now);
    }
}

fn report(ui: &mut egui::Ui, e: &mut Eeprom) {
    let t = theme::of(ui);
    let Some(r) = &e.report else {
        widgets::muted(ui, "任务结束后在这里查看报告。");
        return;
    };
    let (badge, tone) = if r.complete {
        ("演示完成", Tone::Green)
    } else {
        ("已停止 · 未完成", Tone::Amber)
    };
    widgets::panel(
        ui,
        "作业报告",
        |ui| {
            widgets::badge(ui, badge, tone);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let p = &r.plan;
            job::line(
                ui,
                "目标 / 范围",
                &format!(
                    "EEPROM 0x{:02X} · {} · {}",
                    p.addr,
                    label(&CHANNELS, r.cfg.channel),
                    job::range_text(p.start, p.len, 4)
                ),
                false,
                false,
            );
            let verdict = match (p.mode, r.complete) {
                (_, false) => "任务在安全边界停止，未完成验证".to_owned(),
                (Mode::Read, true) => format!("读取 {} B，长度与地址范围核对一致", p.len),
                (Mode::Write, true) if r.mismatches == 0 => {
                    format!("写入 {} B，分 {} 段，读回比较一致", p.len, p.chunks.len())
                }
                (Mode::Write, true) => format!("读回比较 {} 处不一致", r.mismatches),
            };
            job::line(ui, "最终验证", &verdict, false, false);
            job::line(ui, "结束时间", &clock(r.ended), true, false);
            job::line(
                ui,
                "记录性质",
                "界面演示记录 · 数据来自本窗口的演示镜像",
                false,
                true,
            );
            ui.add_space(8.0);
            widgets::group_title(ui, "读回数据");
            job::hex_rows(ui, p.start, &r.readback, |a| {
                p.mode == Mode::Write && a >= p.start && a < p.start + p.len
            });
            ui.add_space(6.0);
            widgets::disclosure(ui, &mut e.snapshot, "执行前配置快照", 7);
            if e.snapshot {
                let c = &r.cfg;
                job::line(ui, "接口 / 通道", label(&CHANNELS, c.channel), false, false);
                job::line(ui, "器件地址", &c.addr, true, false);
                job::line(ui, "容量配置", label(&CAPACITIES, c.capacity), false, false);
                job::line(ui, "起始地址", &c.start, true, false);
                job::line(ui, "地址宽度", label(&WIDTHS, c.wide), false, false);
                job::line(ui, "写入页大小", label(&PAGES, c.page), false, false);
                job::line(ui, "完成检测", label(&DONES, c.ack_poll), false, true);
            }
        },
    );
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("报告来自本页演示；真实读写请在 I2C 主机页执行。")
            .size(11.5)
            .color(t.muted),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_keep_to_pages() {
        assert_eq!(chunks(0, 9, 32), vec![(0, 9)]);
        assert_eq!(chunks(30, 40, 32), vec![(30, 2), (32, 32), (64, 6)]);
        assert_eq!(chunks(64, 32, 32), vec![(64, 32)]);
    }

    #[test]
    fn check_catches_bad_config() {
        let mut e = Eeprom::default();
        assert!(e.check().is_ok());
        e.cfg.start = "0x1FF0".into();
        assert!(e.check().unwrap_err()[0].contains("超出容量"));
        e.cfg.start = "0".into();
        e.cfg.wide = false;
        assert!(e.check().is_err());
        e.cfg.wide = true;
        e.mode = Mode::Write;
        e.data = " ".into();
        assert_eq!(e.check().unwrap_err(), vec!["写入数据为空".to_owned()]);
    }

    #[test]
    fn write_task_lands_and_verifies() {
        let mut e = Eeprom {
            mode: Mode::Write,
            data: "de ad be ef".into(),
            ..Eeprom::default()
        };
        e.cfg.start = "0x0100".into();
        e.start(Duration::ZERO);
        assert_eq!(e.step, Step::Run);
        for _ in 0..3 {
            let i = e.task.as_mut().unwrap().advance(Duration::ZERO).unwrap();
            e.on_stage(i, Duration::from_secs(1));
        }
        assert_eq!(&e.mem[0x100..0x104], &[0xde, 0xad, 0xbe, 0xef]);
        let r = e.report.as_ref().unwrap();
        assert!(r.complete && r.mismatches == 0);
        assert_eq!(r.readback, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn stopping_reports_incomplete() {
        let mut e = Eeprom::default();
        e.start(Duration::ZERO);
        e.task.as_mut().unwrap().stop(Duration::from_secs(1));
        e.finish(Duration::from_secs(1));
        assert!(!e.report.as_ref().unwrap().complete);
        assert_eq!(&e.mem[..9], b"BusStudio");
    }
}
