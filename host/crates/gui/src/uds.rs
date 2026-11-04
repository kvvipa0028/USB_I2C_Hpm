//! UDS Bootloader page: 配置与检查 → 执行任务 → 验证与报告. The check
//! shows the target, the write range, and the six stages of a flash
//! session; each click on 推进演示阶段 finishes one stage and logs the
//! demo CAN frames its services would exchange (session, security access,
//! erase, download, check, reset). The report keeps the result and the
//! config as it was before the run. Nothing goes over USB.

use std::time::Duration;

use eframe::egui;

use crate::i2c::parse::clock;
use crate::job::{self, Step, Task};
use crate::local::{hex_upper, label, parse_hex, parse_num, Saved};
use crate::theme;
use crate::widgets::{self, Table, Tone};

const CARRIERS: [(u8, &str); 1] = [(1, "CAN1 · 经典 CAN")];
const RATES: [(u32, &str); 3] = [
    (250, "250 kbit/s"),
    (500, "500 kbit/s"),
    (1000, "1000 kbit/s"),
];
const STMINS: [(u8, &str); 1] = [(0, "遵循接收方协商（示例 0 ms）")];
const SECURITIES: [(u8, &str); 1] = [(0, "项目策略 · 未配置")];
/// Size of the demo firmware.
const FILE_LEN: u32 = 256 * 1024;
const STAGES: [&str; 6] = [
    "确认目标与固件",
    "进入诊断会话 / 安全访问",
    "擦除指定下载区域",
    "分块下载固件",
    "ECU 完整性校验",
    "复位并确认应用版本",
];

#[derive(Clone, PartialEq, Debug)]
struct Config {
    carrier: u8,
    rate: u32,
    req: String,
    resp: String,
    file: String,
    base: String,
    stmin: u8,
    p2: String,
    p2x: String,
    security: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            carrier: 1,
            rate: 500,
            req: "0x7E0".into(),
            resp: "0x7E8".into(),
            file: "ecu-demo-v1.0.bin".into(),
            base: "0x00010000".into(),
            stmin: 0,
            p2: "50".into(),
            p2x: "5000".into(),
            security: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Plan {
    req: u32,
    resp: u32,
    base: u32,
}

impl Config {
    fn check(&self) -> Result<Plan, Vec<String>> {
        let mut errs = Vec::new();
        let mut hex = |s: &str, max, what| parse_hex(s, max, what).map_err(|e| errs.push(e)).ok();
        let req = hex(&self.req, 0x7ff, "请求 ID ");
        let resp = hex(&self.resp, 0x7ff, "响应 ID ");
        let base = hex(&self.base, u32::MAX - FILE_LEN + 1, "下载起始地址");
        for (s, lo, hi, what) in [(&self.p2, 1, 5000, "P2 "), (&self.p2x, 100, 60_000, "P2* ")] {
            if let Err(e) = parse_num(s, lo, hi, what) {
                errs.push(e);
            }
        }
        if req.is_some() && req == resp {
            errs.push("请求 ID 与响应 ID 不能相同".into());
        }
        if self.file.trim().is_empty() {
            errs.push("固件文件为空".into());
        }
        match (req, resp, base) {
            (Some(req), Some(resp), Some(base)) if errs.is_empty() => Ok(Plan { req, resp, base }),
            _ => Err(errs),
        }
    }
}

struct Frame {
    at: Duration,
    tx: bool,
    id: u32,
    data: [u8; 8],
    what: &'static str,
}

/// The frames one stage exchanges: (tx, bytes, note). The download stage
/// stands for its 64 blocks with the first and the exit.
fn stage_frames(stage: usize) -> Vec<(bool, [u8; 8], &'static str)> {
    match stage {
        0 => vec![
            (true, [2, 0x10, 3, 0, 0, 0, 0, 0], "示例：扩展会话请求"),
            (false, [6, 0x50, 3, 0, 0x32, 1, 0xf4, 0], "示例：会话响应"),
            (true, [3, 0x22, 0xf1, 0x90, 0, 0, 0, 0], "示例：读取 VIN"),
            (
                false,
                [0x10, 0x14, 0x62, 0xf1, 0x90, 0x44, 0x45, 0x4d],
                "示例：ISO-TP 首帧",
            ),
            (true, [0x30, 0, 0, 0, 0, 0, 0, 0], "示例：流控，STmin = 0"),
            (
                false,
                [0x21, 0x4f, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30],
                "示例：连续帧 1",
            ),
            (
                false,
                [0x22, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x31],
                "示例：连续帧 2",
            ),
        ],
        1 => vec![
            (true, [2, 0x10, 2, 0, 0, 0, 0, 0], "示例：编程会话请求"),
            (
                false,
                [6, 0x50, 2, 0, 0x32, 1, 0xf4, 0],
                "示例：编程会话响应",
            ),
            (true, [2, 0x27, 1, 0, 0, 0, 0, 0], "示例：请求种子"),
            (
                false,
                [6, 0x67, 1, 0x12, 0x34, 0x56, 0x78, 0],
                "示例：种子（演示值）",
            ),
            (
                true,
                [6, 0x27, 2, 0xa5, 0xa5, 0xa5, 0xa5, 0],
                "示例：发送密钥（演示值）",
            ),
            (false, [2, 0x67, 2, 0, 0, 0, 0, 0], "示例：安全访问通过"),
        ],
        2 => vec![
            (
                true,
                [0x10, 0x0d, 0x31, 1, 0xff, 0, 0x44, 0],
                "示例：擦除例程首帧",
            ),
            (false, [0x30, 0, 0, 0, 0, 0, 0, 0], "示例：流控"),
            (true, [0x21, 1, 0, 0, 0, 4, 0, 0], "示例：地址与长度"),
            (
                false,
                [3, 0x7f, 0x31, 0x78, 0, 0, 0, 0],
                "示例：等待（NRC 0x78）",
            ),
            (false, [5, 0x71, 1, 0xff, 0, 0, 0, 0], "示例：擦除完成"),
        ],
        3 => vec![
            (
                true,
                [0x10, 0x0b, 0x34, 0, 0x44, 0, 1, 0],
                "示例：请求下载首帧",
            ),
            (false, [0x30, 0, 0, 0, 0, 0, 0, 0], "示例：流控"),
            (true, [0x21, 0, 0, 0, 4, 0, 0, 0], "示例：地址与长度"),
            (
                false,
                [4, 0x74, 0x20, 0x0f, 0xfa, 0, 0, 0],
                "示例：块长 4090 B",
            ),
            (
                true,
                [0x1f, 0xfa, 0x36, 1, 0x42, 0x55, 0x53, 0x53],
                "示例：传输数据 块 1 / 64",
            ),
            (false, [2, 0x76, 1, 0, 0, 0, 0, 0], "示例：块 1 确认"),
            (
                true,
                [1, 0x37, 0, 0, 0, 0, 0, 0],
                "示例：退出传输（块 2–64 略）",
            ),
            (false, [1, 0x77, 0, 0, 0, 0, 0, 0], "示例：传输结束"),
        ],
        4 => vec![
            (true, [4, 0x31, 1, 0xff, 1, 0, 0, 0], "示例：完整性校验例程"),
            (false, [5, 0x71, 1, 0xff, 1, 0, 0, 0], "示例：校验通过"),
        ],
        5 => vec![
            (true, [2, 0x11, 1, 0, 0, 0, 0, 0], "示例：ECU 复位"),
            (false, [2, 0x51, 1, 0, 0, 0, 0, 0], "示例：复位响应"),
            (
                true,
                [3, 0x22, 0xf1, 0x89, 0, 0, 0, 0],
                "示例：读取应用版本",
            ),
            (
                false,
                [7, 0x62, 0xf1, 0x89, 0x31, 0x2e, 0x30, 0],
                "示例：版本 1.0",
            ),
        ],
        _ => Vec::new(),
    }
}

struct Report {
    plan: Plan,
    cfg: Config,
    complete: bool,
    ended: Duration,
}

pub struct Uds {
    cfg: Config,
    saved: Saved<Config>,
    advanced: bool,
    step: Step,
    task: Option<Task>,
    plan: Option<Plan>,
    log: Vec<Frame>,
    report: Option<Report>,
    snapshot: bool,
    services: bool,
    refused: Vec<String>,
}

impl Default for Uds {
    fn default() -> Self {
        Self {
            cfg: Config::default(),
            saved: Saved::default(),
            advanced: false,
            step: Step::Check,
            task: None,
            plan: None,
            log: Vec::new(),
            report: None,
            snapshot: true,
            services: false,
            refused: Vec::new(),
        }
    }
}

impl Uds {
    pub fn running(&self) -> bool {
        job::is_running(&self.task)
    }

    fn start(&mut self, now: Duration) {
        match self.cfg.check() {
            Ok(plan) => {
                let stages = STAGES.iter().map(|s| (*s).to_owned()).collect();
                self.task = Some(Task::new(stages, now));
                self.plan = Some(plan);
                self.log.clear();
                self.report = None;
                self.step = Step::Run;
                self.refused.clear();
            }
            Err(e) => self.refused = e,
        }
    }

    fn on_stage(&mut self, i: usize, now: Duration) {
        let Some(plan) = self.plan else { return };
        for (k, (tx, data, what)) in stage_frames(i).into_iter().enumerate() {
            self.log.push(Frame {
                at: now + Duration::from_millis(k as u64 * 6),
                tx,
                id: if tx { plan.req } else { plan.resp },
                data,
                what,
            });
        }
        if self.task.as_ref().is_some_and(|t| !t.running()) {
            self.finish(now);
        }
    }

    fn finish(&mut self, now: Duration) {
        let (Some(plan), Some(task)) = (self.plan, self.task.as_ref()) else {
            return;
        };
        self.report = Some(Report {
            plan,
            cfg: self.cfg.clone(),
            complete: task.state == job::State::Done,
            ended: now,
        });
    }
}

pub fn page(ui: &mut egui::Ui, u: &mut Uds, now: Duration) {
    widgets::page_head(
        ui,
        "UDS Bootloader 升级",
        Some(job::badge(&u.task)),
        "按项目流程执行诊断刷写，保留阶段状态与服务记录。以下为演示数据，不经 USB。",
        |ui| {
            let was_running = u.running();
            if job::head_button(ui, &mut u.step, &mut u.task, now) {
                u.start(now);
            }
            if was_running && !u.running() {
                u.finish(now);
            }
            u.saved.button(ui, &u.cfg, now);
        },
    );
    ui.add_space(4.0);
    job::step_bar(ui, &mut u.step, u.task.as_ref());
    ui.add_space(10.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match u.step {
            Step::Check => {
                widgets::work_columns(ui, u, 264.0, 760.0, config, |ui, u| {
                    plan_card(ui, u, now);
                });
            }
            Step::Run => run(ui, u, now),
            Step::Report => report(ui, u),
        });
}

fn config(ui: &mut egui::Ui, u: &mut Uds) {
    let before = u.cfg.clone();
    widgets::panel(
        ui,
        "接口配置",
        |ui| {
            widgets::badge(ui, "CAN1", Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let c = &mut u.cfg;
            widgets::field_row(ui, "承载 / 通道", false, |ui| {
                widgets::pick(ui, &mut c.carrier, &CARRIERS);
            });
            widgets::field_row(ui, "CAN 速率", false, |ui| {
                widgets::pick(ui, &mut c.rate, &RATES);
            });
            widgets::field_row(ui, "请求 ID", false, |ui| {
                widgets::unit_edit(ui, &mut c.req, "0x7E0", "");
            });
            widgets::field_row(ui, "响应 ID", false, |ui| {
                widgets::unit_edit(ui, &mut c.resp, "0x7E8", "");
            });
            widgets::field_row(ui, "固件文件", false, |ui| {
                widgets::fixed_value(ui, &c.file, "演示版使用内置示例文件，不读取磁盘");
            });
            widgets::disclosure(ui, &mut u.advanced, "高级参数", 5);
            widgets::hairline(ui);
            if u.advanced {
                widgets::field_row(ui, "下载地址", false, |ui| {
                    widgets::unit_edit(ui, &mut c.base, "0x00010000", "");
                });
                widgets::field_row(ui, "STmin", false, |ui| {
                    widgets::pick(ui, &mut c.stmin, &STMINS);
                });
                widgets::field_row(ui, "响应超时 P2", false, |ui| {
                    widgets::unit_edit(ui, &mut c.p2, "50", "ms");
                });
                widgets::field_row(ui, "扩展超时 P2*", false, |ui| {
                    widgets::unit_edit(ui, &mut c.p2x, "5000", "ms");
                });
                widgets::field_row(ui, "安全访问", true, |ui| {
                    widgets::pick(ui, &mut c.security, &SECURITIES);
                });
            }
            u.saved.foot(ui, &mut u.cfg);
        },
    );
    if u.cfg != before {
        u.refused.clear();
    }
    ui.add_space(8.0);
    widgets::note(ui, "会话、安全访问、擦除与校验顺序必须匹配 ECU 项目规范。");
    widgets::note(ui, "STmin 遵循接收方流控；断线后不得自动恢复擦写。");
}

fn plan_card(ui: &mut egui::Ui, u: &mut Uds, now: Duration) {
    let t = theme::of(ui);
    let checked = u.cfg.check();
    widgets::panel(
        ui,
        "作业计划",
        |ui| {
            widgets::badge(ui, "演示流程", Tone::Neutral);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            job::file_row(
                ui,
                &u.cfg.file,
                "已选示例文件 · 256 KiB",
                "UDS 升级",
                |ui| {
                    widgets::action_button(ui, false, "选择文件")
                        .on_disabled_hover_text("演示版使用内置示例文件，不读取磁盘");
                },
            );
            widgets::hairline(ui);
            match &checked {
                Ok(p) => {
                    job::line(
                        ui,
                        "目标",
                        &format!("演示 ECU · 0x{:03X} / 0x{:03X}", p.req, p.resp),
                        false,
                        false,
                    );
                    job::line(
                        ui,
                        "通道",
                        &format!(
                            "{} · {}",
                            label(&CARRIERS, u.cfg.carrier),
                            label(&RATES, u.cfg.rate)
                        ),
                        false,
                        false,
                    );
                    job::line(
                        ui,
                        "写入范围",
                        &job::range_text(p.base, FILE_LEN, 8),
                        true,
                        false,
                    );
                }
                Err(_) => job::line(ui, "目标", "—", false, false),
            }
            let stages: Vec<String> = STAGES.iter().map(|s| (*s).to_owned()).collect();
            job::plan(ui, &stages);
            let problems = match &checked {
                Ok(_) => u.refused.clone(),
                Err(e) => e.clone(),
            };
            if job::check_foot(ui, &problems, u.running()) {
                u.start(now);
            }
            ui.label(
                egui::RichText::new("执行只推进本页演示，不向 ECU 发送 CAN 报文。")
                    .size(11.5)
                    .color(t.muted),
            );
        },
    );
}

fn run(ui: &mut egui::Ui, u: &mut Uds, now: Duration) {
    let Some(task) = u.task.as_mut() else {
        widgets::muted(ui, "还没有任务。在配置与检查里点检查并进入执行。");
        return;
    };
    let finished = job::run_card(
        ui,
        (&u.cfg.file, "已选示例文件 · 256 KiB", "UDS 升级"),
        "UDS · 刷写",
        task,
        now,
    );
    let ended = !task.running();
    if let Some(i) = finished {
        u.on_stage(i, now);
    } else if ended && u.report.is_none() {
        u.finish(now);
    }
    ui.add_space(14.0);
    services(ui, &u.log, "服务记录");
}

/// The frames the stages so far exchanged.
fn services(ui: &mut egui::Ui, log: &[Frame], title: &str) {
    let t = theme::of(ui);
    widgets::panel(
        ui,
        title,
        |ui| {
            widgets::badge(ui, "演示数据", Tone::Neutral);
        },
        |ui| {
            if log.is_empty() {
                widgets::muted(ui, "推进第一个阶段后，这里列出它交换的报文。");
                return;
            }
            let table = Table {
                widths: &[84.0, 40.0, 64.0, 40.0, 190.0, 0.0],
            };
            table.header(
                ui,
                &[
                    "时间",
                    "方向",
                    "CAN ID",
                    "长度",
                    "原始 CAN 数据 / HEX",
                    "说明",
                ],
            );
            egui::ScrollArea::vertical()
                .id_salt("uds_log")
                .max_height(260.0)
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for f in log {
                        let cells = [
                            (clock(f.at), t.muted, true),
                            (
                                (if f.tx { "TX" } else { "RX" }).to_owned(),
                                if f.tx { t.accent } else { t.green },
                                true,
                            ),
                            (format!("0x{:03X}", f.id), t.fg, true),
                            ("8".to_owned(), t.fg, true),
                            (hex_upper(&f.data), t.fg, true),
                            (f.what.to_owned(), t.fg2, false),
                        ];
                        table.row(ui, &cells, false);
                    }
                });
            widgets::table_foot(
                ui,
                &format!("{} 帧 · 按阶段追加", log.len()),
                "示例报文 · 非实测",
            );
        },
    );
}

fn report(ui: &mut egui::Ui, u: &mut Uds) {
    let t = theme::of(ui);
    let Some(r) = &u.report else {
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
            let p = r.plan;
            job::line(
                ui,
                "目标 / 范围",
                &format!(
                    "演示 ECU · 0x{:03X} / 0x{:03X} · {}",
                    p.req,
                    p.resp,
                    job::range_text(p.base, FILE_LEN, 8)
                ),
                false,
                false,
            );
            let verdict = if r.complete {
                "ECU 校验、复位与应用版本确认演示通过"
            } else {
                "任务在安全边界停止，未完成校验；不得自动恢复擦写"
            };
            job::line(ui, "最终验证", verdict, false, false);
            job::line(ui, "结束时间", &clock(r.ended), true, false);
            job::line(ui, "记录性质", "界面演示记录", false, true);
            ui.add_space(6.0);
            widgets::disclosure(ui, &mut u.snapshot, "执行前配置快照", 10);
            if u.snapshot {
                let c = &r.cfg;
                job::line(ui, "承载 / 通道", label(&CARRIERS, c.carrier), false, false);
                job::line(ui, "CAN 速率", label(&RATES, c.rate), true, false);
                job::line(ui, "请求 ID", &c.req, true, false);
                job::line(ui, "响应 ID", &c.resp, true, false);
                job::line(ui, "固件文件", &c.file, true, false);
                job::line(ui, "下载起始地址", &c.base, true, false);
                job::line(ui, "STmin 策略", label(&STMINS, c.stmin), false, false);
                job::line(
                    ui,
                    "响应超时 P2",
                    &format!("{} ms", c.p2.trim()),
                    true,
                    false,
                );
                job::line(
                    ui,
                    "扩展超时 P2*",
                    &format!("{} ms", c.p2x.trim()),
                    true,
                    false,
                );
                job::line(ui, "安全访问", label(&SECURITIES, c.security), false, true);
            }
        },
    );
    ui.add_space(12.0);
    let t2 = theme::of(ui);
    egui::Frame::new()
        .fill(t2.card)
        .stroke(egui::Stroke::new(1.0, t2.line))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            widgets::disclosure(ui, &mut u.services, "查看服务交互示例", u.log.len());
        });
    if u.services {
        ui.add_space(8.0);
        services(ui, &u.log, "服务交互示例");
    }
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("报告来自本页演示；当前固件没有 CAN 命令，未与任何 ECU 通信。")
            .size(11.5)
            .color(t.muted),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_catches_bad_ids() {
        let mut c = Config::default();
        assert_eq!(
            c.check().unwrap(),
            Plan {
                req: 0x7e0,
                resp: 0x7e8,
                base: 0x10000
            }
        );
        c.resp = "0x7E0".into();
        assert_eq!(
            c.check().unwrap_err(),
            vec!["请求 ID 与响应 ID 不能相同".to_owned()]
        );
        c.resp = "0x800".into();
        assert!(c.check().is_err());
    }

    #[test]
    fn stages_log_their_frames() {
        let mut u = Uds::default();
        u.start(Duration::ZERO);
        let mut total = 0;
        for i in 0..STAGES.len() {
            assert_eq!(u.task.as_mut().unwrap().advance(Duration::ZERO), Some(i));
            u.on_stage(i, Duration::ZERO);
            total += stage_frames(i).len();
            assert_eq!(u.log.len(), total);
        }
        assert!(u.report.as_ref().unwrap().complete);
        // requests go to 0x7E0, answers come from 0x7E8
        assert!(u
            .log
            .iter()
            .all(|f| f.id == if f.tx { 0x7e0 } else { 0x7e8 }));
        // single frames carry their length in the first byte
        for i in 0..STAGES.len() {
            for (_, d, _) in stage_frames(i) {
                if d[0] >> 4 == 0 {
                    assert!(d[0] <= 7);
                }
            }
        }
    }
}
