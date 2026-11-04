//! Flash page: 配置与检查 → 执行任务 → 验证与报告, against a demo SPI NOR
//! image kept in the window. The check shows the write range, the erase
//! range rounded out to whole erase units, and the reserved bytes around
//! the write that are backed up and put back. The task steps through
//! backup, erase, program and restore, and read-back one click at a time;
//! stopping after the erase leaves the reserved bytes erased, as on a real
//! chip. Nothing goes over USB.

use std::collections::BTreeMap;
use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::clock;
use crate::job::{self, Step, Task};
use crate::local::{label, parse_hex, parse_num, Saved};
use crate::theme;
use crate::widgets::{self, Tone};

const CHANNELS: [(u8, &str); 1] = [(1, "SPI1")];
const DEVICES: [(u8, &str); 1] = [(0, "SPI NOR · 示例")];
const CAPACITIES: [(u32, &str); 5] = [
    (1, "1 MiB"),
    (2, "2 MiB"),
    (4, "4 MiB"),
    (8, "8 MiB"),
    (16, "16 MiB"),
];
const CHIP_SELECTS: [(u8, &str); 2] = [(0, "CS0"), (1, "CS1")];
const ERASE_UNITS: [(u32, &str); 3] = [
    (4096, "4096 B · 扇区"),
    (32768, "32 KiB · 块"),
    (65536, "64 KiB · 块"),
];
const PAGES: [(u32, &str); 1] = [(256, "256 B")];
const VERIFIES: [(u8, &str); 1] = [(0, "读回比较")];
const FILE: &str = "busstudio-demo.bin";
/// The design's header, at the start of the demo image and of the demo file.
const HEADER: [u8; 32] = [
    0x42, 0x55, 0x53, 0x53, 0x54, 0x55, 0x44, 0x49, 0x4f, 0x2d, 0x44, 0x45, 0x4d, 0x4f, 0, 0, 0x01,
    0, 0, 0, 0, 0x10, 0, 0, 0x80, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,
];
/// Bytes of the preview under the plan.
const PREVIEW: u32 = 128;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Read,
    Program,
}

/// The 128-byte demo file: the header, then a counting pattern.
fn demo_file() -> Vec<u8> {
    (0..128u32)
        .map(|i| HEADER.get(i as usize).copied().unwrap_or(i as u8))
        .collect()
}

#[derive(Clone, PartialEq, Debug)]
struct Config {
    channel: u8,
    device: u8,
    mib: u32,
    start: String,
    len: String,
    cs: u8,
    erase: u32,
    page: u32,
    verify: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 1,
            device: 0,
            mib: 8,
            start: "0x000000".into(),
            len: "128".into(),
            cs: 0,
            erase: 4096,
            page: 256,
            verify: 0,
        }
    }
}

impl Config {
    fn bytes(&self) -> u32 {
        self.mib << 20
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Plan {
    mode: Mode,
    start: u32,
    len: u32,
    /// Erase range rounded out to whole units, (start, end exclusive).
    erase: (u32, u32),
}

impl Plan {
    fn stages(&self) -> Vec<String> {
        match self.mode {
            Mode::Read => vec![
                "读取指定范围".into(),
                "整理读出缓冲".into(),
                "核对读取长度与范围".into(),
            ],
            Mode::Program => vec![
                "备份擦除单元内保留区".into(),
                "擦除对齐单元".into(),
                "分页编程与保留区恢复".into(),
                "读回校验".into(),
            ],
        }
    }

    fn reserved(&self) -> u32 {
        (self.erase.1 - self.erase.0) - self.len
    }

    fn pages(&self, page: u32) -> u32 {
        (self.start + self.len).div_ceil(page) - self.start / page
    }
}

/// `start..start+len` widened to whole `unit`s.
fn align(start: u32, len: u32, unit: u32) -> (u32, u32) {
    (start / unit * unit, (start + len).div_ceil(unit) * unit)
}

/// The chip: erased (FF) except the header and what tasks wrote.
#[derive(Default)]
struct Image {
    written: BTreeMap<u32, u8>,
}

impl Image {
    fn get(&self, a: u32) -> u8 {
        self.written
            .get(&a)
            .copied()
            .unwrap_or_else(|| HEADER.get(a as usize).copied().unwrap_or(0xff))
    }

    fn read(&self, start: u32, len: u32) -> Vec<u8> {
        (start..start + len).map(|a| self.get(a)).collect()
    }

    fn write(&mut self, start: u32, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.written.insert(start + i as u32, *b);
        }
    }
}

struct Report {
    plan: Plan,
    cfg: Config,
    readback: Vec<u8>,
    mismatches: usize,
    complete: bool,
    /// Stopped after the erase: the reserved bytes were not put back.
    reserved_lost: bool,
    ended: Duration,
}

pub struct Flash {
    cfg: Config,
    saved: Saved<Config>,
    advanced: bool,
    mode: Mode,
    image: Image,
    step: Step,
    task: Option<Task>,
    plan: Option<Plan>,
    /// Reserved bytes backed up by the first stage.
    backup: Vec<u8>,
    report: Option<Report>,
    snapshot: bool,
    refused: Vec<String>,
}

impl Default for Flash {
    fn default() -> Self {
        Self {
            cfg: Config::default(),
            saved: Saved::default(),
            advanced: false,
            mode: Mode::Program,
            image: Image::default(),
            step: Step::Check,
            task: None,
            plan: None,
            backup: Vec::new(),
            report: None,
            snapshot: true,
            refused: Vec::new(),
        }
    }
}

impl Flash {
    pub fn running(&self) -> bool {
        job::is_running(&self.task)
    }

    fn check(&self) -> Result<Plan, Vec<String>> {
        let c = &self.cfg;
        let cap = c.bytes();
        let mut errs = Vec::new();
        let start = parse_hex(&c.start, cap - 1, "起始地址")
            .map_err(|e| errs.push(e))
            .ok();
        let len = match self.mode {
            Mode::Read => parse_num(&c.len, 1, 4096, "操作长度")
                .map_err(|e| errs.push(e))
                .ok(),
            Mode::Program => Some(demo_file().len() as u32),
        };
        if let (Some(s), Some(n)) = (start, len) {
            if u64::from(s) + u64::from(n) > u64::from(cap) {
                errs.push(format!("0x{s:06X} 起 {n} B 超出容量 {} MiB", c.mib));
            }
        }
        match (start, len) {
            (Some(start), Some(len)) if errs.is_empty() => Ok(Plan {
                mode: self.mode,
                start,
                len,
                erase: align(start, len, c.erase),
            }),
            _ => Err(errs),
        }
    }

    fn start(&mut self, now: Duration) {
        match self.check() {
            Ok(plan) => {
                self.task = Some(Task::new(plan.stages(), now));
                self.plan = Some(plan);
                self.backup.clear();
                self.report = None;
                self.step = Step::Run;
                self.refused.clear();
            }
            Err(e) => self.refused = e,
        }
    }

    /// Program stages: 0 backs up the erase range, 1 erases it, 2 writes
    /// the file and puts the backup back around it.
    fn on_stage(&mut self, i: usize, now: Duration) {
        let Some(p) = self.plan.clone() else { return };
        if p.mode == Mode::Program {
            let (es, ee) = p.erase;
            match i {
                0 => self.backup = self.image.read(es, ee - es),
                1 => self.image.write(es, &vec![0xff; (ee - es) as usize]),
                2 => {
                    let backup = std::mem::take(&mut self.backup);
                    self.image.write(es, &backup);
                    self.image.write(p.start, &demo_file());
                }
                _ => {}
            }
        }
        if self.task.as_ref().is_some_and(|t| !t.running()) {
            self.finish(now);
        }
    }

    fn finish(&mut self, now: Duration) {
        let (Some(plan), Some(task)) = (self.plan.clone(), self.task.as_ref()) else {
            return;
        };
        let readback = self.image.read(plan.start, plan.len);
        let mismatches = match plan.mode {
            Mode::Read => 0,
            Mode::Program => readback
                .iter()
                .zip(demo_file())
                .filter(|(a, b)| **a != *b)
                .count(),
        };
        self.report = Some(Report {
            readback,
            mismatches,
            complete: task.state == job::State::Done,
            reserved_lost: plan.mode == Mode::Program && (task.done == 2),
            cfg: self.cfg.clone(),
            plan,
            ended: now,
        });
    }
}

pub fn page(ui: &mut egui::Ui, f: &mut Flash, now: Duration) {
    widgets::page_head(
        ui,
        "Flash 编程",
        Some(job::badge(&f.task)),
        "查看存储映射，按区域读取、擦除、编程和校验。器件为演示镜像，不经 USB。",
        |ui| {
            let was_running = f.running();
            if job::head_button(ui, &mut f.step, &mut f.task, now) {
                f.start(now);
            }
            if was_running && !f.running() {
                f.finish(now);
            }
            f.saved.button(ui, &f.cfg, now);
        },
    );
    ui.add_space(4.0);
    job::step_bar(ui, &mut f.step, f.task.as_ref());
    ui.add_space(10.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match f.step {
            Step::Check => {
                widgets::work_columns(ui, f, 264.0, 760.0, config, |ui, f| {
                    plan_card(ui, f, now);
                    ui.add_space(14.0);
                    map(ui, f);
                });
            }
            Step::Run => run(ui, f, now),
            Step::Report => report(ui, f),
        });
}

fn config(ui: &mut egui::Ui, f: &mut Flash) {
    let before = f.cfg.clone();
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, "SPI1", Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let c = &mut f.cfg;
            widgets::field_row(ui, "接口 / 通道", false, |ui| {
                widgets::pick(ui, &mut c.channel, &CHANNELS);
            });
            widgets::field_row(ui, "器件配置", false, |ui| {
                widgets::pick(ui, &mut c.device, &DEVICES);
            });
            widgets::field_row(ui, "容量配置", false, |ui| {
                widgets::pick(ui, &mut c.mib, &CAPACITIES);
            });
            widgets::field_row(ui, "起始地址", false, |ui| {
                widgets::unit_edit(ui, &mut c.start, "0x000000", "");
            });
            widgets::field_row(ui, "操作长度", false, |ui| {
                if f.mode == Mode::Program {
                    widgets::fixed_value(ui, "128 B", "编程长度等于示例文件大小");
                } else {
                    widgets::unit_edit(ui, &mut c.len, "128", "B");
                }
            });
            widgets::disclosure(ui, &mut f.advanced, "高级参数", 4);
            widgets::hairline(ui);
            if f.advanced {
                widgets::field_row(ui, "片选", false, |ui| {
                    widgets::pick(ui, &mut c.cs, &CHIP_SELECTS);
                });
                widgets::field_row(ui, "擦除单元", false, |ui| {
                    widgets::pick(ui, &mut c.erase, &ERASE_UNITS);
                });
                widgets::field_row(ui, "编程页大小", false, |ui| {
                    widgets::pick(ui, &mut c.page, &PAGES);
                });
                widgets::field_row(ui, "完成验证", true, |ui| {
                    widgets::pick(ui, &mut c.verify, &VERIFIES);
                });
            }
            f.saved.foot(ui, &mut f.cfg);
        },
    );
    if f.cfg != before {
        f.refused.clear();
    }
    ui.add_space(8.0);
    widgets::note(ui, "擦除按实际单元对齐，执行前展示完整影响范围。");
    widgets::note(ui, "芯片识别与保护状态确认后，才允许擦除或编程。");
}

fn plan_card(ui: &mut egui::Ui, f: &mut Flash, now: Duration) {
    let t = theme::of(ui);
    let mut mode = f.mode;
    let checked = f.check();
    widgets::panel(
        ui,
        "作业计划",
        |ui| {
            widgets::seg(
                ui,
                &mut mode,
                &[(Mode::Read, "读取"), (Mode::Program, "编程")],
            );
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let (note, tag) = match f.mode {
                Mode::Read => (
                    format!("导出文件示例 · {} B 操作范围", f.cfg.len.trim()),
                    "读取",
                ),
                Mode::Program => ("已选示例文件 · 128 B 操作范围".to_owned(), "编程"),
            };
            job::file_row(ui, FILE, &note, tag, |ui| {
                widgets::action_button(ui, false, "选择文件")
                    .on_disabled_hover_text("演示版使用内置示例文件，不读取磁盘");
            });
            widgets::hairline(ui);
            job::line(ui, "目标", label(&DEVICES, f.cfg.device), false, false);
            job::line(
                ui,
                "通道",
                &format!(
                    "{} · {}",
                    label(&CHANNELS, f.cfg.channel),
                    label(&CHIP_SELECTS, f.cfg.cs)
                ),
                false,
                false,
            );
            match &checked {
                Ok(p) if p.mode == Mode::Program => {
                    job::line(
                        ui,
                        "写入范围",
                        &job::range_text(p.start, p.len, 8),
                        true,
                        false,
                    );
                    let (es, ee) = p.erase;
                    job::line(
                        ui,
                        "擦除范围",
                        &job::range_text(es, ee - es, 8),
                        true,
                        false,
                    );
                    job::line(
                        ui,
                        "对齐方式",
                        &format!(
                            "{} B · 保留区 {} B 先备份，编程后恢复 · {} 个编程页",
                            f.cfg.erase,
                            p.reserved(),
                            p.pages(f.cfg.page)
                        ),
                        false,
                        false,
                    );
                    job::plan(ui, &p.stages());
                }
                Ok(p) => {
                    job::line(
                        ui,
                        "读取范围",
                        &job::range_text(p.start, p.len, 8),
                        true,
                        false,
                    );
                    job::plan(ui, &p.stages());
                }
                Err(_) => job::line(ui, "操作范围", "—", false, false),
            }
            let problems = match &checked {
                Ok(_) => f.refused.clone(),
                Err(errs) => errs.clone(),
            };
            if job::check_foot(ui, &problems, f.running()) {
                f.start(now);
            }
            ui.label(
                egui::RichText::new("执行只推进本页演示，不向设备发送 SPI 命令。")
                    .size(11.5)
                    .color(t.muted),
            );
        },
    );
    if mode != f.mode {
        f.mode = mode;
        f.refused.clear();
    }
}

/// The erase range as a bar: reserved bytes either side of the write.
fn map(ui: &mut egui::Ui, f: &mut Flash) {
    let t = theme::of(ui);
    let plan = f.check().ok();
    widgets::panel(
        ui,
        "存储映射",
        |ui| {
            widgets::badge(ui, "演示镜像", Tone::Neutral);
        },
        |ui| {
            let Some(p) = plan else {
                widgets::muted(ui, "参数有误，修正后显示映射。");
                return;
            };
            let (es, ee) = p.erase;
            widgets::muted(
                ui,
                &format!(
                    "容量 {} MiB · 共 {} 个擦除单元 · 本次涉及 {} 个",
                    f.cfg.mib,
                    f.cfg.bytes() / f.cfg.erase,
                    (ee - es) / f.cfg.erase
                ),
            );
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 58.0), egui::Sense::hover());
            let bar = egui::Rect::from_min_max(
                rect.left_top() + egui::vec2(0.0, 6.0),
                egui::pos2(rect.right(), rect.top() + 34.0),
            );
            let x = |a: u32| bar.left() + (a - es) as f32 / (ee - es) as f32 * bar.width();
            let p_ = ui.painter();
            let radius = egui::CornerRadius::same(6);
            let program = p.mode == Mode::Program;
            p_.rect_filled(bar, radius, if program { t.amber_soft } else { t.pill });
            let w = egui::Rect::from_x_y_ranges(
                x(p.start)..=x(p.start + p.len).max(x(p.start) + 3.0),
                bar.y_range(),
            );
            p_.rect_filled(w, egui::CornerRadius::same(3), t.accent);
            p_.rect_stroke(
                bar,
                radius,
                egui::Stroke::new(1.0, t.line),
                egui::StrokeKind::Inside,
            );
            let mono = egui::FontId::monospace(10.5);
            for (a, align) in [(es, egui::Align2::LEFT_TOP), (ee, egui::Align2::RIGHT_TOP)] {
                p_.text(
                    egui::pos2(x(a), bar.bottom() + 6.0),
                    align,
                    format!("0x{a:08X}"),
                    mono.clone(),
                    t.muted,
                );
            }
            ui.horizontal(|ui| {
                let key = |ui: &mut egui::Ui, c: egui::Color32, name: &str| {
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, egui::CornerRadius::same(2), c);
                    widgets::muted(ui, name);
                };
                let what = if program {
                    "写入范围"
                } else {
                    "读取范围"
                };
                key(ui, t.accent, &format!("{what} {} B", p.len));
                if program {
                    key(
                        ui,
                        t.amber_soft,
                        &format!("保留区 {} B · 备份后恢复", p.reserved()),
                    );
                }
            });
            ui.add_space(4.0);
            let shown = p.len.min(PREVIEW);
            let (base, data) = if program {
                (p.start, demo_file())
            } else {
                (p.start, f.image.read(p.start, shown))
            };
            widgets::group_title(
                ui,
                if program {
                    "待写入数据 · 示例文件"
                } else {
                    "当前内容 · 演示镜像"
                },
            );
            job::hex_rows(ui, base, &data[..shown as usize], |_| false);
        },
    );
}

fn run(ui: &mut egui::Ui, f: &mut Flash, now: Duration) {
    let (Some(task), Some(plan)) = (f.task.as_mut(), f.plan.as_ref()) else {
        widgets::muted(ui, "还没有任务。在配置与检查里点检查并进入执行。");
        return;
    };
    let (tag, what) = match plan.mode {
        Mode::Read => ("读取", "Flash · 读取"),
        Mode::Program => ("编程", "Flash · 编程"),
    };
    let note = format!("已选示例文件 · {} B 操作范围", plan.len);
    let finished = job::run_card(ui, (FILE, &note, tag), what, task, now);
    let ended = !task.running();
    let done: Vec<&str> = task.stages[..task.done]
        .iter()
        .map(String::as_str)
        .collect();
    let summary = if done.is_empty() {
        format!("尚未完成阶段；整体进度 {:.0}%。", task.frac() * 100.0)
    } else {
        format!(
            "已完成：{}；整体进度 {:.0}%。",
            done.join("、"),
            task.frac() * 100.0
        )
    };
    ui.add_space(8.0);
    widgets::note(ui, &summary);
    if plan.mode == Mode::Program && task.done == 2 && !task.running() {
        widgets::note(ui, "擦除后停止：保留区已擦除、尚未恢复，备份只在本窗口。");
    }
    if let Some(i) = finished {
        f.on_stage(i, now);
    } else if ended && f.report.is_none() {
        f.finish(now);
    }
}

fn report(ui: &mut egui::Ui, f: &mut Flash) {
    let t = theme::of(ui);
    let Some(r) = &f.report else {
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
                    "{} · {} · {}",
                    label(&DEVICES, r.cfg.device),
                    label(&CHIP_SELECTS, r.cfg.cs),
                    job::range_text(p.start, p.len, 8)
                ),
                false,
                false,
            );
            let verdict = match (p.mode, r.complete) {
                (_, false) if r.reserved_lost => "擦除后停止：保留区未恢复，需重新执行".to_owned(),
                (_, false) => "任务在安全边界停止，未完成校验".to_owned(),
                (Mode::Read, true) => format!("读取 {} B，长度与范围核对一致", p.len),
                (Mode::Program, true) if r.mismatches == 0 => format!(
                    "编程 {} B，保留区 {} B 已恢复，读回校验一致",
                    p.len,
                    p.reserved()
                ),
                (Mode::Program, true) => format!("读回校验 {} 处不一致", r.mismatches),
            };
            job::line(ui, "最终验证", &verdict, false, false);
            if p.mode == Mode::Program {
                let (es, ee) = p.erase;
                job::line(
                    ui,
                    "擦除范围",
                    &job::range_text(es, ee - es, 8),
                    true,
                    false,
                );
            }
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
            let n = r.readback.len().min(PREVIEW as usize);
            job::hex_rows(ui, p.start, &r.readback[..n], |_| false);
            ui.add_space(6.0);
            widgets::disclosure(ui, &mut f.snapshot, "执行前配置快照", 6);
            if f.snapshot {
                let c = &r.cfg;
                job::line(ui, "接口 / 通道", label(&CHANNELS, c.channel), false, false);
                job::line(ui, "器件配置", label(&DEVICES, c.device), false, false);
                job::line(ui, "容量配置", label(&CAPACITIES, c.mib), false, false);
                job::line(ui, "起始地址", &c.start, true, false);
                job::line(ui, "擦除单元", label(&ERASE_UNITS, c.erase), false, false);
                job::line(ui, "编程页大小", label(&PAGES, c.page), false, true);
            }
        },
    );
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("报告来自本页演示，未访问任何真实器件。")
            .size(11.5)
            .color(t.muted),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_range_rounds_out() {
        assert_eq!(align(0, 128, 4096), (0, 4096));
        assert_eq!(align(4000, 200, 4096), (0, 8192));
        assert_eq!(align(4096, 4096, 4096), (4096, 8192));
        let p = Plan {
            mode: Mode::Program,
            start: 0x100,
            len: 128,
            erase: align(0x100, 128, 4096),
        };
        assert_eq!(p.reserved(), 4096 - 128);
        assert_eq!(p.pages(256), 1);
    }

    #[test]
    fn program_keeps_reserved_bytes() {
        let mut f = Flash::default();
        f.cfg.start = "0x000080".into();
        f.start(Duration::ZERO);
        for _ in 0..4 {
            let i = f.task.as_mut().unwrap().advance(Duration::ZERO).unwrap();
            f.on_stage(i, Duration::ZERO);
        }
        // the header before the write survived the erase
        assert_eq!(f.image.read(0, 32), HEADER.to_vec());
        assert_eq!(f.image.read(0x80, 128), demo_file());
        let r = f.report.as_ref().unwrap();
        assert!(r.complete && r.mismatches == 0);
    }

    #[test]
    fn stop_after_erase_loses_reserved() {
        let mut f = Flash::default();
        f.cfg.start = "0x000080".into();
        f.start(Duration::ZERO);
        for _ in 0..2 {
            let i = f.task.as_mut().unwrap().advance(Duration::ZERO).unwrap();
            f.on_stage(i, Duration::ZERO);
        }
        f.task.as_mut().unwrap().stop(Duration::ZERO);
        f.finish(Duration::ZERO);
        assert_eq!(f.image.get(0), 0xff);
        assert!(f.report.as_ref().unwrap().reserved_lost);
    }

    #[test]
    fn check_limits_to_capacity() {
        let mut f = Flash::default();
        f.cfg.mib = 1;
        f.cfg.start = "0x0FFFF0".into();
        assert!(f.check().unwrap_err()[0].contains("超出容量"));
        f.mode = Mode::Read;
        f.cfg.len = "16".into();
        assert!(f.check().is_ok());
    }
}
