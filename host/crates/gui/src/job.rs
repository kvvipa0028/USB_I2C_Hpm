//! The 配置与检查 → 执行任务 → 验证与报告 flow shared by EEPROM, Flash and
//! UDS Bootloader. Execution is a local demo: each click on 推进演示阶段
//! finishes one stage, 在安全边界停止 stops before the next. Nothing goes
//! over USB.

use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::clock;
use crate::theme;
use crate::widgets::{self, Tone};

pub const STEPS: [&str; 3] = ["配置与检查", "执行任务", "验证与报告"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Check,
    Run,
    Report,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Running,
    Stopped,
    Done,
}

pub struct Task {
    pub stages: Vec<String>,
    /// Stages finished so far.
    pub done: usize,
    pub state: State,
    pub started: Duration,
    pub ended: Option<Duration>,
}

impl Task {
    pub fn new(stages: Vec<String>, now: Duration) -> Self {
        Self {
            stages,
            done: 0,
            state: State::Running,
            started: now,
            ended: None,
        }
    }

    /// Finish the current stage. Returns its index when one was finished.
    pub fn advance(&mut self, now: Duration) -> Option<usize> {
        if self.state != State::Running {
            return None;
        }
        self.done += 1;
        if self.done >= self.stages.len() {
            self.state = State::Done;
            self.ended = Some(now);
        }
        Some(self.done - 1)
    }

    /// Stop at the boundary: the finished stages stay, the rest never run.
    pub fn stop(&mut self, now: Duration) {
        if self.state == State::Running {
            self.state = State::Stopped;
            self.ended = Some(now);
        }
    }

    pub fn frac(&self) -> f32 {
        self.done as f32 / self.stages.len().max(1) as f32
    }

    pub fn running(&self) -> bool {
        self.state == State::Running
    }
}

pub fn is_running(task: &Option<Task>) -> bool {
    task.as_ref().is_some_and(Task::running)
}

/// Page-head badge for a flow page.
pub fn badge(task: &Option<Task>) -> (&'static str, Tone) {
    match task.as_ref().map(|t| t.state) {
        Some(State::Running) => ("演示运行中", Tone::Green),
        Some(State::Done) => ("演示完成", Tone::Green),
        Some(State::Stopped) => ("已停止", Tone::Amber),
        None => ("本地演示", Tone::Neutral),
    }
}

/// The step bar; 执行任务 opens once a task exists, the report once it has
/// ended.
pub fn step_bar(ui: &mut egui::Ui, step: &mut Step, task: Option<&Task>) {
    let reach = match task.map(|t| t.state) {
        None => 0,
        Some(State::Running) => 1,
        Some(_) => 2,
    };
    if let Some(i) = widgets::steps(ui, &STEPS, *step as usize, reach) {
        *step = [Step::Check, Step::Run, Step::Report][i];
    }
}

/// The page-head main button, by where the flow stands. Returns true when
/// 检查并进入执行 was clicked; the page checks its config and starts the task.
pub fn head_button(
    ui: &mut egui::Ui,
    step: &mut Step,
    task: &mut Option<Task>,
    now: Duration,
) -> bool {
    let state = task.as_ref().map(|t| t.state);
    match (*step, state) {
        (Step::Check, Some(State::Running)) => {
            widgets::ink_button(ui, false, "检查并进入执行")
                .on_disabled_hover_text("先停止当前任务");
        }
        (Step::Check, _) => return widgets::ink_button(ui, true, "检查并进入执行").clicked(),
        (Step::Run, Some(State::Running)) => {
            if widgets::ink_button(ui, true, "停止任务").clicked() {
                if let Some(t) = task {
                    t.stop(now);
                }
            }
        }
        (Step::Run, Some(_)) | (Step::Run, None) => {
            if widgets::ink_button(ui, task.is_some(), "查看作业报告").clicked() {
                *step = Step::Report;
            }
        }
        (Step::Report, _) => {
            if widgets::ink_button(ui, true, "新建作业").clicked() {
                *task = None;
                *step = Step::Check;
            }
        }
    }
    false
}

/// A small document glyph, as BusStudio's file rows have.
fn file_icon(ui: &mut egui::Ui) {
    let t = theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 20.0), egui::Sense::hover());
    let r = rect.shrink2(egui::vec2(2.0, 2.0));
    let s = egui::Stroke::new(1.2, t.fg2);
    let fold = 4.0;
    let pts = vec![
        r.left_top(),
        egui::pos2(r.right() - fold, r.top()),
        egui::pos2(r.right(), r.top() + fold),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    ui.painter().add(egui::Shape::line(pts, s));
    ui.painter().line_segment(
        [
            egui::pos2(r.left() + 3.0, r.center().y + 1.0),
            egui::pos2(r.right() - 3.0, r.center().y + 1.0),
        ],
        egui::Stroke::new(1.0, t.muted),
    );
}

/// File row on top of 作业计划: icon, name, a note under it, a tag, and
/// whatever `right` adds at the end of the row.
pub fn file_row(
    ui: &mut egui::Ui,
    name: &str,
    note: &str,
    tag: &str,
    right: impl FnOnce(&mut egui::Ui),
) {
    let t = theme::of(ui);
    ui.horizontal(|ui| {
        file_icon(ui);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(egui::RichText::new(name).size(13.0).color(t.fg));
            ui.label(egui::RichText::new(note).size(11.0).color(t.muted));
        });
        widgets::badge(ui, tag, Tone::Neutral);
        widgets::trailing(ui, 28.0, right);
    });
}

/// Key and value on one line, the key in a fixed column.
pub fn line(ui: &mut egui::Ui, key: &str, value: &str, mono: bool, last: bool) {
    let t = theme::of(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(104.0, 20.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            key,
            egui::FontId::proportional(12.0),
            t.muted,
        );
        let text = egui::RichText::new(value).color(t.fg);
        let text = if mono {
            text.monospace()
        } else {
            text.size(12.5)
        };
        ui.add(egui::Label::new(text).wrap());
    });
    if !last {
        widgets::hairline(ui);
    }
}

/// The stages of a plan, numbered, one per line.
pub fn plan(ui: &mut egui::Ui, stages: &[String]) {
    let t = theme::of(ui);
    for (i, s) in stages.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            ui.label(
                egui::RichText::new(format!("{}. {s}", i + 1))
                    .size(12.5)
                    .color(t.fg2),
            );
        });
        widgets::hairline(ui);
    }
}

/// 检查状态 and the check's verdict, with the button that starts the task.
/// Returns true when the button was clicked.
pub fn check_foot(ui: &mut egui::Ui, problems: &[String], blocked: bool) -> bool {
    let t = theme::of(ui);
    let verdict = if problems.is_empty() {
        "参数检查通过，可进入执行".to_owned()
    } else {
        problems.join("；")
    };
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(104.0, 20.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            "检查状态",
            egui::FontId::proportional(12.0),
            t.muted,
        );
        let color = if problems.is_empty() { t.green } else { t.red };
        ui.add(egui::Label::new(egui::RichText::new(verdict).size(12.5).color(color)).wrap());
    });
    let ok = problems.is_empty() && !blocked;
    let resp = widgets::ink_button(ui, ok, "检查并进入执行");
    let resp = if blocked {
        resp.on_disabled_hover_text("先停止当前任务")
    } else {
        resp
    };
    resp.clicked()
}

/// 执行任务 card (design 15): what runs, the current stage in large type,
/// percent, the bar, every stage with its state, and the two controls.
/// Returns true when a stage was finished this frame.
pub fn run_card(
    ui: &mut egui::Ui,
    file: (&str, &str, &str),
    what: &str,
    task: &mut Task,
    now: Duration,
) -> Option<usize> {
    let t = theme::of(ui);
    let mut finished = None;
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.line))
        .corner_radius(egui::CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    file_row(ui, file.0, file.1, file.2, |_| {});
                });
            widgets::hairline(ui);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let pct = format!("{:.0}%", task.frac() * 100.0);
                    let headline = match task.state {
                        State::Running => format!("正在演示：{}", task.stages[task.done]),
                        State::Done => "演示完成：全部阶段已结束".to_owned(),
                        State::Stopped => {
                            format!("已在安全边界停止：{} 未执行", task.stages[task.done])
                        }
                    };
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.label(
                                egui::RichText::new(what)
                                    .size(11.0)
                                    .color(t.muted)
                                    .extra_letter_spacing(0.6),
                            );
                            ui.label(egui::RichText::new(headline).size(16.0).color(t.fg));
                        });
                        widgets::trailing(ui, 44.0, |ui| {
                            ui.label(egui::RichText::new(pct).size(28.0).color(t.fg));
                        });
                    });
                    widgets::progress(ui, task.frac());
                    let state = match task.state {
                        State::Running => "演示运行中",
                        State::Done => "演示完成",
                        State::Stopped => "已停止",
                    };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        widgets::muted(
                            ui,
                            &format!(
                                "整体作业 · 已完成 {} / {} 个阶段",
                                task.done,
                                task.stages.len()
                            ),
                        );
                        widgets::muted(ui, state);
                        widgets::muted(ui, &format!("开始于 {}", clock(task.started)));
                        widgets::muted(ui, "界面演示");
                    });
                    ui.add_space(6.0);
                    for (i, s) in task.stages.iter().enumerate() {
                        stage_row(ui, i, s, task);
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        widgets::trailing(ui, 30.0, |ui| {
                            let run = task.running();
                            if widgets::action_button(ui, run, "在安全边界停止").clicked() {
                                task.stop(now);
                            }
                            if widgets::ink_button(ui, run, "推进演示阶段").clicked() {
                                finished = task.advance(now);
                            }
                        });
                    });
                });
        });
    finished
}

fn stage_row(ui: &mut egui::Ui, i: usize, name: &str, task: &Task) {
    let t = theme::of(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::hover());
    let p = ui.painter();
    let c = egui::pos2(rect.left() + 12.0, rect.center().y);
    let done = i < task.done;
    let current = i == task.done && task.state == State::Running;
    let (status, color) = if done {
        ("演示完成", t.muted)
    } else if current {
        ("执行中", t.accent)
    } else if task.state == State::Stopped {
        ("未执行", t.amber)
    } else {
        ("待执行", t.muted)
    };
    if done {
        p.circle_filled(c, 10.0, t.green_soft);
        let s = egui::Stroke::new(1.6, t.green);
        p.line_segment([c + egui::vec2(-4.0, 0.0), c + egui::vec2(-1.0, 3.0)], s);
        p.line_segment([c + egui::vec2(-1.0, 3.0), c + egui::vec2(4.5, -3.0)], s);
    } else {
        p.circle_filled(c, 10.0, if current { t.accent_soft } else { t.pill });
        p.text(
            c,
            egui::Align2::CENTER_CENTER,
            (i + 1).to_string(),
            egui::FontId::proportional(11.0),
            if current { t.accent } else { t.muted },
        );
    }
    p.text(
        egui::pos2(rect.left() + 32.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(13.0),
        t.fg,
    );
    p.text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        status,
        egui::FontId::proportional(11.5),
        color,
    );
    p.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, t.line2),
    );
}

/// Offset, 16 hex bytes, and ASCII per row; bytes for which `mark` holds
/// sit on the accent's soft fill.
pub fn hex_rows(ui: &mut egui::Ui, base: u32, data: &[u8], mark: impl Fn(u32) -> bool) {
    let t = theme::of(ui);
    let mono = egui::FontId::monospace(11.5);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(6), t.card2);
    let hex_x = rect.left() + 86.0;
    let cell = 22.0;
    let ascii_x = hex_x + 16.0 * cell + 14.0;
    for (x, name) in [
        (rect.left() + 8.0, "偏移"),
        (hex_x, "数据 / HEX · 16 B"),
        (ascii_x, "ASCII"),
    ] {
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(11.0),
            t.muted,
        );
    }
    for (r, row) in data.chunks(16).enumerate() {
        let addr = base + r as u32 * 16;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
        let p = ui.painter();
        p.hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, t.line2),
        );
        p.text(
            egui::pos2(rect.left() + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{addr:08X}"),
            mono.clone(),
            t.muted,
        );
        let mut ascii = String::new();
        for (i, b) in row.iter().enumerate() {
            let x = hex_x + i as f32 * cell;
            if mark(addr + i as u32) {
                p.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x - 3.0, rect.top() + 3.0),
                        egui::vec2(cell - 2.0, rect.height() - 6.0),
                    ),
                    egui::CornerRadius::same(3),
                    t.accent_soft,
                );
            }
            p.text(
                egui::pos2(x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{b:02X}"),
                mono.clone(),
                t.fg,
            );
            ascii.push(if (0x20..0x7f).contains(b) {
                *b as char
            } else {
                '.'
            });
        }
        p.text(
            egui::pos2(ascii_x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            ascii,
            mono.clone(),
            t.fg2,
        );
    }
}

/// `0x0000–0x007F` for `len` bytes at `start`, `digits` hex digits wide.
pub fn range_text(start: u32, len: u32, digits: usize) -> String {
    let end = start + (len.max(1) - 1);
    format!("0x{start:0digits$X} – 0x{end:0digits$X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_runs_stops_and_finishes() {
        let mut t = Task::new(vec!["a".into(), "b".into()], Duration::ZERO);
        assert!(t.running());
        assert_eq!(t.advance(Duration::from_secs(1)), Some(0));
        assert_eq!(t.frac(), 0.5);
        t.stop(Duration::from_secs(2));
        assert_eq!(t.state, State::Stopped);
        assert_eq!(t.advance(Duration::from_secs(3)), None);
        let mut t = Task::new(vec!["a".into()], Duration::ZERO);
        t.advance(Duration::from_secs(1));
        assert_eq!(
            (t.state, t.ended),
            (State::Done, Some(Duration::from_secs(1)))
        );
        t.stop(Duration::from_secs(2));
        assert_eq!(t.state, State::Done);
    }

    #[test]
    fn ranges_read_well() {
        assert_eq!(range_text(0, 128, 4), "0x0000 – 0x007F");
        assert_eq!(range_text(0x10000, 0x40000, 8), "0x00010000 – 0x0004FFFF");
        assert_eq!(range_text(0xffff_fff0, 16, 8), "0xFFFFFFF0 – 0xFFFFFFFF");
    }
}
