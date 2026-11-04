//! The UDS page's 真实执行路径（骨架）: the P1 host path in the window. Pick
//! a package, check it offline, choose an entry explicitly, prepare a
//! locked plan, start it on the simulated backend, ask it to stop, and read
//! the archived events and the result. Everything here is a simulated run
//! and says 模拟; it shares nothing with the 页面演示 task of `uds.rs` or the
//! demo task in `job.rs`.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;

use crate::flow::api::{Issue, IssueKind, Level, Mode, StepDef, API_VERSION};
use crate::flow::host::{Host, Source, State, MAX_ASSET};
use crate::flow::mock::{MockBackend, FIXTURES};
use crate::i2c::parse::clock;
use crate::job;
use crate::theme;
use crate::widgets::{self, Table, Tone};

/// The pick-list value for a file on disk; lower values are the examples.
const LOCAL: usize = FIXTURES.len();
const SOURCES: [(usize, &str); 4] = [
    (0, FIXTURES[0].0),
    (1, FIXTURES[1].0),
    (2, FIXTURES[2].0),
    (LOCAL, "本地文件"),
];

pub struct Sim {
    host: Host,
    pick: usize,
    path: String,
    /// The entry the user chose; with several entries nothing is chosen
    /// until they click one.
    selected: Option<String>,
    /// Editable parameters as typed, for the chosen entry.
    params: Vec<(String, String)>,
    /// Why the last action was refused.
    refused: Vec<String>,
}

impl Default for Sim {
    fn default() -> Self {
        Self {
            host: Host::new(Box::<MockBackend>::default())
                .expect("the mock backend speaks the host's API"),
            pick: 0,
            path: String::new(),
            selected: None,
            params: Vec::new(),
            refused: Vec::new(),
        }
    }
}

/// What a button on the task card asked for, applied after the card is drawn.
enum Act {
    Start,
    Cancel,
    Discard,
}

impl Sim {
    /// Collect the simulated run's events; true while it is under way.
    pub fn tick(&mut self, now: Duration) -> bool {
        self.host.poll(now)
    }

    pub fn active(&self) -> bool {
        self.host.active()
    }

    fn source(&self) -> Result<Source, String> {
        match FIXTURES.get(self.pick) {
            Some(&(_, name, text)) => Ok(Source::Builtin { name, text }),
            None if self.path.trim().is_empty() => Err("填写本地文件路径".into()),
            None => Ok(Source::File(PathBuf::from(self.path.trim()))),
        }
    }

    fn inspect(&mut self) {
        self.refused.clear();
        self.selected = None;
        self.params.clear();
        match self.source().and_then(|s| self.host.inspect(s)) {
            Ok(()) => {
                // one entry is unambiguous; several wait for a click
                let only = match self.host.inspection().map(|i| &i.entries[..]) {
                    Some([e]) => Some(e.id.clone()),
                    _ => None,
                };
                if let Some(id) = only {
                    self.select(&id);
                }
            }
            Err(e) => self.refused = vec![e],
        }
    }

    fn select(&mut self, id: &str) {
        let Some(e) = self.host.inspection().and_then(|i| i.entry(id)) else {
            return;
        };
        self.params = e
            .params
            .iter()
            .filter(|p| p.editable)
            .map(|p| (p.id.clone(), p.default.to_string()))
            .collect();
        self.selected = Some(id.to_owned());
        self.refused.clear();
    }

    fn prepare(&mut self, now: Duration) {
        match self
            .host
            .prepare(self.selected.as_deref(), &self.params, now)
        {
            Ok(()) => self.refused.clear(),
            Err(e) => self.refused = e,
        }
    }

    fn act(&mut self, act: Act, now: Duration) {
        let res = match act {
            Act::Start => self.host.start(now).map(|_| ()),
            Act::Cancel => self.host.request_cancel(now).map(|_| ()),
            Act::Discard => self.host.discard(),
        };
        match res {
            Ok(()) => self.refused.clear(),
            Err(e) => self.refused = vec![e],
        }
    }
}

fn state_badge(st: State) -> (&'static str, Tone) {
    match st {
        State::Preparing => ("模拟 · 准备中", Tone::Neutral),
        State::Ready => ("模拟 · 就绪", Tone::Neutral),
        State::Running => ("模拟 · 运行中", Tone::Green),
        State::Stopping => ("模拟 · 停止处理中", Tone::Amber),
        // a simulated run that went to the end proves nothing about an ECU
        State::Succeeded => ("模拟成功 · 非真实刷写", Tone::Neutral),
        State::Failed => ("模拟 · 失败", Tone::Red),
        State::Stopped => ("模拟 · 已停止", Tone::Amber),
        State::Unknown => ("模拟 · 结果未知", Tone::Red),
    }
}

/// Page-head badge while this view is open.
pub fn badge(s: &Sim) -> (&'static str, Tone) {
    s.host
        .state()
        .map_or(("模拟 · 未准备", Tone::Neutral), state_badge)
}

fn headline(st: State, mode: Mode) -> String {
    let what = match st {
        State::Preparing => "准备中",
        State::Ready => "运行计划已就绪，尚未启动",
        State::Running => "模拟运行中",
        State::Stopping => "停止请求已被接收，等待后端给出结果",
        State::Succeeded => "模拟事件流按计划结束",
        State::Failed => "模拟运行失败",
        State::Stopped => "模拟运行已在边界停止",
        State::Unknown => "模拟运行结果未知",
    };
    format!("{what} · {}", mode.label())
}

/// §5.1's import stages for what is open now.
fn import_status(s: &Sim) -> (&'static str, Tone) {
    let Some(ins) = s.host.inspection() else {
        return ("未导入", Tone::Neutral);
    };
    let entry = s.selected.as_deref().and_then(|id| ins.entry(id));
    let issues: Vec<&Issue> = match entry {
        Some(e) => ins.blockers(e).collect(),
        None => ins.issues.iter().collect(),
    };
    if issues.iter().any(|i| i.kind == IssueKind::Unsupported) {
        return ("已识别 · 含不支持特性", Tone::Red);
    }
    if !issues.is_empty() {
        return ("已解析 · 缺少依赖", Tone::Amber);
    }
    match s.host.state() {
        Some(State::Ready) => ("已准备 · 可启动", Tone::Green),
        Some(st) if st.active() => ("已准备 · 运行中", Tone::Neutral),
        _ if entry.is_none() && ins.entries.len() > 1 => ("已解析 · 待选择入口", Tone::Neutral),
        _ => ("已解析", Tone::Neutral),
    }
}

pub fn view(ui: &mut egui::Ui, s: &mut Sim, now: Duration) {
    banner(ui);
    ui.add_space(10.0);
    widgets::work_columns(
        ui,
        s,
        300.0,
        860.0,
        |ui, s| {
            source(ui, s);
            ui.add_space(12.0);
            backend(ui, s);
        },
        |ui, s| {
            inspection(ui, s, now);
            ui.add_space(12.0);
            task(ui, s, now);
            ui.add_space(12.0);
            archive(ui, s);
        },
    );
}

fn banner(ui: &mut egui::Ui) {
    let t = theme::of(ui);
    egui::Frame::new()
        .fill(t.amber_soft)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 9))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(
                egui::RichText::new("模拟执行（非真实刷写）")
                    .size(13.0)
                    .strong()
                    .color(t.amber),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "真实执行路径的骨架：宿主任务管理、离线导入检查和版本化后端接口已接通，\
                         执行后端只有模拟后端。不打开总线、不经 USB、不发送 UDS 请求；\
                         任何结束状态都不代表 ECU 已被刷写。真实 CAN / ISO-TP / UDS 执行属于 P2，\
                         D01–D07 仍待确认。",
                    )
                    .size(12.0)
                    .color(t.fg2),
                )
                .wrap(),
            );
        });
}

fn source(ui: &mut egui::Ui, s: &mut Sim) {
    let busy = s.active();
    widgets::panel(
        ui,
        "流程来源",
        |ui| {
            widgets::badge(ui, "离线导入", Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_enabled_ui(!busy, |ui| {
                widgets::field_row(ui, "来源", s.pick != LOCAL, |ui| {
                    widgets::pick(ui, &mut s.pick, &SOURCES);
                });
                if s.pick == LOCAL {
                    widgets::field_row(ui, "路径", true, |ui| {
                        widgets::unit_edit(ui, &mut s.path, "/path/to/package.udsmock", "");
                    });
                }
            });
            ui.add_space(4.0);
            if widgets::action_button(ui, !busy, "读取并检查")
                .on_disabled_hover_text("任务运行中，结束后才能重新导入")
                .clicked()
            {
                s.inspect();
            }
            if s.host.inspection().is_none() {
                refused(ui, &s.refused);
            }
        },
    );
    ui.add_space(8.0);
    widgets::note(
        ui,
        &format!(
            "离线检查只读文件文本：不打开总线、不执行包内脚本、不发送 UDS 请求。本地文件上限 {} KiB。",
            MAX_ASSET / 1024
        ),
    );
}

fn backend(ui: &mut egui::Ui, s: &Sim) {
    let c = s.host.caps();
    widgets::panel(
        ui,
        "执行后端",
        |ui| {
            widgets::badge(ui, "模拟", Tone::Amber);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            job::line(
                ui,
                "名称 / 版本",
                &format!("{} {}", c.name, c.version),
                true,
                false,
            );
            job::line(
                ui,
                "接口版本",
                &format!("{} · 宿主 {API_VERSION}", c.api),
                true,
                false,
            );
            job::line(ui, "运行模式", c.mode.label(), false, false);
            let formats: Vec<String> = c.formats.iter().map(|(f, v)| format!("{f} v{v}")).collect();
            job::line(ui, "支持格式", &formats.join("、"), false, false);
            job::line(ui, "平台", &c.platforms, false, false);
            job::line(ui, "通信接口", &c.link, false, false);
            job::line(
                ui,
                "停止",
                c.cancel.as_deref().unwrap_or("不支持"),
                false,
                false,
            );
            job::line(ui, "事件粒度", &c.events, false, true);
        },
    );
    ui.add_space(8.0);
    widgets::note(
        ui,
        "能力声明只用于界面和导入前检查，不代表已通过任何目标 ECU 验收。",
    );
}

fn refused(ui: &mut egui::Ui, lines: &[String]) {
    for l in lines {
        widgets::error(ui, l);
    }
}

fn issue_rows(ui: &mut egui::Ui, list: &[Issue]) {
    let t = theme::of(ui);
    for i in list {
        ui.horizontal(|ui| {
            ui.set_min_height(24.0);
            let tone = match i.kind {
                IssueKind::Missing => Tone::Amber,
                IssueKind::Unsupported => Tone::Red,
            };
            widgets::badge(ui, i.kind.label(), tone);
            ui.add(egui::Label::new(egui::RichText::new(&i.text).size(12.0).color(t.fg2)).wrap());
        });
    }
}

fn step_text(s: &StepDef) -> String {
    let effect = if s.effect { " · 有副作用" } else { "" };
    format!("{}（{}）{effect}", s.label, s.id)
}

fn inspection(ui: &mut egui::Ui, s: &mut Sim, now: Duration) {
    let t = theme::of(ui);
    let busy = s.active();
    let status = import_status(s);
    // a copy, so the rows below can change the selection and parameters
    let (Some(asset), Some(ins)) = (s.host.asset().cloned(), s.host.inspection().cloned()) else {
        widgets::panel(
            ui,
            "检查结果",
            |ui| {
                widgets::badge(ui, status.0, status.1);
            },
            |ui| {
                widgets::muted(ui, "选择来源后点「读取并检查」。");
            },
        );
        return;
    };
    let mut chosen = None;
    let mut prepare = false;
    widgets::panel(
        ui,
        "检查结果",
        |ui| {
            widgets::badge(ui, status.0, status.1);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            job::line(ui, "文件", &asset.label, false, false);
            job::line(
                ui,
                "格式 / 版本",
                &format!("{} · v{}", ins.format, ins.version),
                true,
                false,
            );
            job::line(ui, "包名", &ins.package, false, false);
            job::line(
                ui,
                "内容哈希",
                &format!(
                    "FNV-1a 64 · {:016X}（非加密，只用于发现准备后的改动）",
                    asset.hash
                ),
                false,
                false,
            );
            issue_rows(ui, &ins.issues);
            for n in &ins.notes {
                widgets::note(ui, n);
            }
            ui.add_space(8.0);
            widgets::group_title(ui, &format!("入口 · {} 个", ins.entries.len()));
            if ins.entries.len() > 1 && s.selected.is_none() {
                ui.label(
                    egui::RichText::new(format!(
                        "包内有 {} 个入口：请在下表明确选择一个，不会默认执行第一个。",
                        ins.entries.len()
                    ))
                    .size(12.0)
                    .color(t.amber),
                );
            }
            let table = Table {
                widths: &[28.0, 64.0, 0.0, 150.0, 96.0],
            };
            table.header(ui, &["", "ID", "名称", "目标 ECU", "检查"]);
            ui.spacing_mut().item_spacing.y = 0.0;
            for e in &ins.entries {
                let on = s.selected.as_deref() == Some(e.id.as_str());
                let blockers: Vec<&Issue> = ins.blockers(e).collect();
                let (check, color) = if blockers.iter().any(|i| i.kind == IssueKind::Unsupported) {
                    ("含不支持特性", t.red)
                } else if !blockers.is_empty() {
                    ("缺少依赖", t.amber)
                } else {
                    ("可准备", t.green)
                };
                let cells = [
                    ((if on { "●" } else { "○" }).to_owned(), t.accent, false),
                    (e.id.clone(), t.fg, true),
                    (e.title.clone(), t.fg, false),
                    (e.ecu.clone(), t.fg2, false),
                    (check.to_owned(), color, false),
                ];
                let row = table.row(ui, &cells, on);
                if row.clicked() && !busy && !on {
                    chosen = Some(e.id.clone());
                }
            }
            ui.spacing_mut().item_spacing.y = 2.0;
            let entry = s.selected.as_deref().and_then(|id| ins.entry(id));
            let mut blockers = Vec::new();
            if let Some(e) = entry {
                ui.add_space(8.0);
                widgets::group_title(ui, &format!("所选入口 {} · 步骤、镜像与参数", e.id));
                let steps: Vec<String> = e.steps.iter().map(step_text).collect();
                job::plan(ui, &steps);
                for i in &e.images {
                    job::line(
                        ui,
                        "镜像",
                        &format!(
                            "{} · {}（{} B）",
                            i.file,
                            job::range_text(i.base, i.len, 8),
                            i.len
                        ),
                        true,
                        false,
                    );
                }
                issue_rows(ui, &e.issues);
                ui.add_enabled_ui(!busy, |ui| {
                    for p in &e.params {
                        if p.editable {
                            let hover = format!(
                                "范围 {}–{} {} · 默认 {} · 来源：{}",
                                p.lo, p.hi, p.unit, p.default, p.origin
                            );
                            if let Some((_, v)) = s.params.iter_mut().find(|(id, _)| *id == p.id) {
                                widgets::field_row(ui, &p.label, false, |ui| {
                                    widgets::unit_edit(ui, v, &p.default.to_string(), &p.unit)
                                        .on_hover_text(hover);
                                });
                            }
                        } else {
                            widgets::field_row(ui, &p.label, false, |ui| {
                                widgets::fixed_value(
                                    ui,
                                    &p.text,
                                    &format!("只读 · 来源：{}", p.origin),
                                );
                            });
                        }
                    }
                });
                blockers = ins.blockers(e).map(|i| i.text.clone()).collect();
            }
            ui.add_space(6.0);
            let (verdict, color) = match entry {
                None if ins.entries.len() > 1 => {
                    ("未选择入口：必须明确选择一个".to_owned(), t.amber)
                }
                None => ("没有可准备的入口".to_owned(), t.red),
                Some(_) if !blockers.is_empty() => {
                    (format!("{} 项阻止准备与启动", blockers.len()), t.red)
                }
                Some(_) => ("检查通过，可准备运行计划（模拟后端）".to_owned(), t.green),
            };
            ui.label(egui::RichText::new(verdict).size(12.5).color(color));
            if matches!(s.host.state(), Some(State::Ready)) {
                widgets::muted(ui, "运行计划已锁定：之后改动参数不影响它，需重新准备。");
            }
            refused(ui, &s.refused);
            ui.add_space(4.0);
            let can = entry.is_some() && blockers.is_empty() && !busy;
            let why = if busy {
                "任务运行中"
            } else if entry.is_none() {
                "先明确选择一个入口"
            } else {
                "先解决上面列出的缺失与不支持项"
            };
            if widgets::ink_button(ui, can, "准备运行计划")
                .on_disabled_hover_text(why)
                .clicked()
            {
                prepare = true;
            }
        },
    );
    if let Some(id) = chosen {
        s.select(&id);
    }
    if prepare {
        s.prepare(now);
    }
}

fn state_strip(ui: &mut egui::Ui, current: Option<State>) {
    let t = theme::of(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        widgets::muted(ui, "任务状态");
        for st in State::ALL {
            if Some(st) == current {
                let tone = match state_badge(st).1 {
                    Tone::Neutral => Tone::Amber,
                    tone => tone,
                };
                widgets::badge(ui, st.label(), tone);
            } else {
                ui.label(egui::RichText::new(st.label()).size(11.0).color(t.faint));
            }
        }
    });
}

fn step_row(ui: &mut egui::Ui, i: usize, step: &StepDef, status: &str, color: egui::Color32) {
    let t = theme::of(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.label(
            egui::RichText::new(format!("{}. {}", i + 1, step.label))
                .size(12.5)
                .color(t.fg),
        );
        if step.effect {
            widgets::badge(ui, "有副作用", Tone::Neutral);
        }
        widgets::trailing(ui, 22.0, |ui| {
            ui.label(egui::RichText::new(status).size(11.5).color(color));
        });
    });
    widgets::hairline(ui);
}

fn task(ui: &mut egui::Ui, s: &mut Sim, now: Duration) {
    let t = theme::of(ui);
    let (text, tone) = badge(s);
    let mut act = None;
    widgets::panel(
        ui,
        "运行任务",
        |ui| {
            widgets::badge(ui, text, tone);
        },
        |ui| {
            state_strip(ui, s.host.state());
            widgets::hairline(ui);
            let Some(task) = s.host.task() else {
                widgets::muted(ui, "检查通过并准备运行计划后，在这里启动模拟执行。");
                return;
            };
            let st = task.state();
            let plan = task.plan();
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(headline(st, plan.mode))
                    .size(15.0)
                    .color(t.fg),
            );
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            widgets::group_title(ui, "运行快照 · 准备后锁定");
            job::line(
                ui,
                "运行计划",
                &format!(
                    "P{:04} · 准备于 {} · 已锁定",
                    plan.plan_id,
                    clock(task.prepared())
                ),
                true,
                false,
            );
            let run = match (task.run_id(), task.started()) {
                (Some(r), Some(at)) => format!("{r} · 开始于 {}", clock(at)),
                _ => "未启动".to_owned(),
            };
            job::line(ui, "运行 ID", &run, true, false);
            job::line(
                ui,
                "入口",
                &format!("{} · {}", plan.entry, plan.title),
                false,
                false,
            );
            job::line(ui, "目标 ECU", &plan.ecu, false, false);
            job::line(
                ui,
                "后端 / 接口",
                &format!(
                    "{} · API {} · {}",
                    plan.backend,
                    plan.api,
                    plan.mode.label()
                ),
                false,
                false,
            );
            job::line(
                ui,
                "流程文件",
                &format!(
                    "{} · {} · FNV-1a {:016X}",
                    plan.asset, plan.package, plan.asset_hash
                ),
                false,
                false,
            );
            for i in &plan.images {
                job::line(
                    ui,
                    "镜像",
                    &format!("{} · {}", i.file, job::range_text(i.base, i.len, 8)),
                    true,
                    false,
                );
            }
            let params: Vec<String> = plan
                .params
                .iter()
                .map(|p| {
                    let ro = if p.editable { "" } else { "（只读）" };
                    format!("{}（{}）= {}{ro}", p.label, p.id, p.value)
                })
                .collect();
            let params = if params.is_empty() {
                "无".to_owned()
            } else {
                params.join("；")
            };
            job::line(ui, "参数", &params, false, false);
            for n in &plan.notes {
                job::line(ui, "备注", n, false, false);
            }

            let p = task.progress();
            let n = plan.steps.len();
            ui.add_space(8.0);
            widgets::muted(
                ui,
                &format!("步骤 {} / {n} · 按后端确认的步骤计，不按时间估算", p.done),
            );
            widgets::progress(ui, p.done as f32 / n.max(1) as f32);
            if plan.steps.iter().any(|s| s.id == "download") {
                let bytes = match p.bytes {
                    Some((c, total)) => format!("已确认下载（模拟）：{c} / {total} B"),
                    None => format!("已确认下载（模拟）：0 / {} B", plan.image_bytes()),
                };
                widgets::muted(ui, &bytes);
            }
            ui.add_space(4.0);
            for (i, step) in plan.steps.iter().enumerate() {
                let (status, color) = if i < p.done {
                    ("完成（模拟）", t.muted)
                } else if p.current == Some(i) && st.active() {
                    ("进行中（模拟）", t.accent)
                } else if st.terminal() {
                    ("未完成", t.amber)
                } else {
                    ("待执行", t.muted)
                };
                step_row(ui, i, step, status, color);
            }
            if let Some(reply) = task.cancel() {
                ui.add_space(6.0);
                widgets::note(
                    ui,
                    &format!(
                        "停止请求：{}。接收停止请求不等于 ECU 已停止，更不表示回滚；最终状态以后端结果为准。",
                        reply.label()
                    ),
                );
            }
            if let Some(r) = task.result() {
                ui.add_space(8.0);
                widgets::group_title(ui, "结果 · 模拟");
                let or_none = |v: &[String]| {
                    if v.is_empty() {
                        "无".to_owned()
                    } else {
                        v.join("；")
                    }
                };
                job::line(ui, "结论", &headline(st, r.mode), false, false);
                job::line(ui, "运行模式", r.mode.label(), false, false);
                job::line(ui, "错误", r.error.as_deref().unwrap_or("—"), false, false);
                job::line(ui, "已知副作用", &or_none(&r.effects), false, false);
                job::line(ui, "未确认", &or_none(&r.unconfirmed), false, false);
                job::line(ui, "执行证据", &or_none(&r.evidence), false, false);
                let ended = task.ended().map_or("—".to_owned(), clock);
                job::line(ui, "结束时间", &ended, true, true);
                widgets::muted(
                    ui,
                    "失败或未知结果不会自动重试；要再运行，需重新检查、准备并手动启动。",
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                widgets::trailing(ui, 30.0, |ui| match st {
                    State::Ready => {
                        if widgets::ink_button(ui, true, "开始模拟执行").clicked() {
                            act = Some(Act::Start);
                        }
                        if widgets::action_button(ui, true, "放弃计划").clicked() {
                            act = Some(Act::Discard);
                        }
                    }
                    State::Running => {
                        if widgets::action_button(ui, true, "请求停止")
                            .on_hover_text("只发出停止请求；后端在其支持的边界处理")
                            .clicked()
                        {
                            act = Some(Act::Cancel);
                        }
                    }
                    State::Stopping => {
                        widgets::action_button(ui, false, "停止处理中…")
                            .on_disabled_hover_text("等待后端给出最终结果");
                    }
                    State::Preparing => {}
                    _ => {
                        if widgets::ink_button(ui, true, "新建任务")
                            .on_hover_text("清除本任务；不会重新执行")
                            .clicked()
                        {
                            act = Some(Act::Discard);
                        }
                    }
                });
            });
        },
    );
    if let Some(a) = act {
        s.act(a, now);
    }
}

fn archive(ui: &mut egui::Ui, s: &Sim) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        "事件归档",
        |ui| {
            widgets::badge(ui, "模拟事件", Tone::Amber);
        },
        |ui| {
            let Some(task) = s.host.task() else {
                widgets::muted(ui, "准备运行计划后，宿主和后端的事件按序号记在这里。");
                return;
            };
            let table = Table {
                widths: &[44.0, 76.0, 56.0, 40.0, 84.0, 40.0, 0.0],
            };
            table.header(
                ui,
                &["序号", "时间", "运行", "来源", "步骤", "级别", "内容"],
            );
            egui::ScrollArea::vertical()
                .id_salt("uds_sim_archive")
                .max_height(280.0)
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for r in task.archive() {
                        let color = match r.level {
                            Level::Info => t.fg,
                            Level::Warn => t.amber,
                            Level::Error => t.red,
                        };
                        let cells = [
                            (r.seq.to_string(), t.muted, true),
                            (clock(r.at), t.muted, true),
                            (
                                r.run.map_or("—".to_owned(), |id| id.to_string()),
                                t.fg2,
                                true,
                            ),
                            (r.origin.label().to_owned(), t.fg2, false),
                            (
                                r.step.clone().unwrap_or_else(|| "—".to_owned()),
                                t.fg2,
                                true,
                            ),
                            (r.level.label().to_owned(), color, false),
                            (r.text.clone(), color, false),
                        ];
                        table.row(ui, &cells, false).on_hover_text(r.text.as_str());
                    }
                });
            widgets::table_foot(
                ui,
                &format!(
                    "{} 条 · 运行 ID / 序号 / 时间 / 步骤 / 级别 / 内容",
                    task.archive().len()
                ),
                "模拟事件 · 非实测",
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::mock::STEP;
    use crate::fonts;

    fn render(s: &mut Sim, dark: bool, w: f32, now: Duration) {
        let ctx = egui::Context::default();
        fonts::install(&ctx);
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w, 900.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| view(ui, s, now));
            });
        });
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
        for p in ctx.tessellate(output.shapes, output.pixels_per_point) {
            assert!(p.clip_rect.is_finite());
            if let egui::epaint::Primitive::Mesh(m) = p.primitive {
                assert!(m.vertices.iter().all(|v| v.pos.is_finite()));
            }
        }
    }

    fn picked(pick: usize) -> Sim {
        let mut s = Sim {
            pick,
            ..Sim::default()
        };
        s.inspect();
        s
    }

    /// The single-entry example, prepared and started at 0.
    fn started() -> Sim {
        let mut s = picked(0);
        s.prepare(Duration::ZERO);
        s.act(Act::Start, Duration::ZERO);
        s
    }

    /// Every stage of the flow, each drawn headless in both themes and in a
    /// stacked and a two-column width. Not a window or screenshot check.
    #[test]
    fn every_stage_renders() {
        let mut stages = vec![
            (Sim::default(), Duration::ZERO),
            (picked(LOCAL), Duration::ZERO),
            (picked(2), Duration::ZERO),
            (picked(1), Duration::ZERO),
        ];
        let mut s = picked(1);
        s.select("cal");
        s.prepare(Duration::ZERO);
        assert_eq!(s.host.state(), Some(State::Ready));
        stages.push((s, Duration::ZERO));
        let mut s = started();
        let mid = STEP * 4 + STEP / 2;
        s.tick(mid);
        stages.push((s, mid));
        let mut s = started();
        s.act(Act::Cancel, STEP / 2);
        assert_eq!(s.host.state(), Some(State::Stopping));
        stages.push((s, STEP / 2));
        let mut s = started();
        s.tick(STEP * 50);
        assert_eq!(s.host.state(), Some(State::Succeeded));
        stages.push((s, STEP * 50));
        for (s, now) in &mut stages {
            for dark in [false, true] {
                for w in [900.0, 1500.0] {
                    render(s, dark, w, *now);
                }
            }
        }
    }

    #[test]
    fn simulated_texts_never_claim_a_flash() {
        let banned = [
            ["刷写", "成功"].concat(),
            ["刷写", "完成"].concat(),
            ["升级", "成功"].concat(),
        ];
        for st in State::ALL {
            let (b, _) = state_badge(st);
            let h = headline(st, Mode::Simulated);
            assert!(b.contains("模拟") && h.contains("模拟"), "{st:?}");
            for x in &banned {
                assert!(!b.contains(x.as_str()) && !h.contains(x.as_str()), "{st:?}");
            }
        }
        for text in [
            include_str!("sim.rs"),
            include_str!("../flow/mock.rs"),
            include_str!("../flow/host.rs"),
        ] {
            for x in &banned {
                assert!(!text.contains(x.as_str()));
            }
        }
        // a run that went to the end still says it was simulated
        let mut s = started();
        s.tick(STEP * 50);
        assert_eq!(badge(&s).0, "模拟成功 · 非真实刷写");
    }

    #[test]
    fn several_entries_start_unselected() {
        let mut s = picked(1);
        assert!(s.selected.is_none());
        assert_eq!(import_status(&s).0, "已解析 · 待选择入口");
        s.prepare(Duration::ZERO);
        assert!(s.refused[0].contains("必须明确选择"));
        assert!(s.host.task().is_none());
        s.select("app");
        assert_eq!(s.params, vec![("p2".to_owned(), "50".to_owned())]);
        s.prepare(Duration::ZERO);
        assert_eq!(import_status(&s).0, "已准备 · 可启动");
        // one entry is chosen for the user, visibly
        let s = picked(0);
        assert_eq!(s.selected.as_deref(), Some("app"));
    }

    #[test]
    fn blocked_packages_cannot_be_prepared() {
        let mut s = picked(2);
        assert_eq!(import_status(&s).0, "已识别 · 含不支持特性");
        s.prepare(Duration::ZERO);
        assert!(!s.refused.is_empty());
        assert!(s.host.task().is_none());
    }

    #[test]
    fn local_file_needs_a_path() {
        let s = picked(LOCAL);
        assert_eq!(s.refused, vec!["填写本地文件路径".to_owned()]);
    }
}
