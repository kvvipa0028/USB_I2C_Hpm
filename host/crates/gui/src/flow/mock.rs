//! The simulated execution backend of P1, and the demo package format it
//! reads.
//!
//! The format, `uds-mock-fixture` version 1, is invented for this demo and
//! documented in `fixtures/uds/README.md`; it is not ODX, PDX, OTX or any
//! vendor format. `inspect` and `prepare` only parse its text: image files
//! are names in a list, never opened, and a `script` line is reported, not
//! run. `start` plays a fixed timeline: each step takes [`STEP`], the
//! download step reports confirmed bytes in [`PARTS`] parts, and every
//! event says 模拟. Nothing here opens a bus or sends a request.
//!
//! Stop rule of this model: a stop request during the download step is
//! handled at the next block boundary without waiting for the block already
//! sent, so that block stays unconfirmed and the run ends 结果未知; during
//! any other step the step finishes and the run ends 已停止 before the next.

use std::time::Duration;

use super::api::{
    Asset, Backend, BackendEvent, Bound, CancelReply, Capabilities, Entry, EventKind, Image,
    Inspection, Issue, Level, Mode, Outcome, Param, PrepareRequest, RunId, RunPlan, RunResult,
    StepDef, API_VERSION,
};
use crate::job::range_text;

pub const FORMAT: &str = "uds-mock-fixture";
pub const VERSION: &str = "1";
pub const NAME: &str = "MockBackend";
pub const BACKEND_VERSION: &str = "0.1.0";
/// How long one simulated step takes.
pub const STEP: Duration = Duration::from_millis(700);
/// Parts the download step reports its confirmed bytes in.
pub const PARTS: u32 = 4;

/// The built-in example packages: (label, file name, text).
pub const FIXTURES: [(&str, &str, &str); 3] = [
    (
        "单入口示例",
        "single.udsmock",
        include_str!("../../fixtures/uds/single.udsmock"),
    ),
    (
        "多入口示例",
        "multi.udsmock",
        include_str!("../../fixtures/uds/multi.udsmock"),
    ),
    (
        "含不支持项示例",
        "blocked.udsmock",
        include_str!("../../fixtures/uds/blocked.udsmock"),
    ),
];

/// Step kinds the model knows, and whether each changes the target.
const KINDS: [(&str, bool); 7] = [
    ("read_id", false),
    ("session", false),
    ("security", false),
    ("erase", true),
    ("download", true),
    ("verify", false),
    ("reset", true),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FaultKind {
    /// The step ends with a negative response.
    Fail,
    /// The backend goes silent halfway through the step.
    Lost,
}

/// `simulate = fail@verify`: a fault the fixture asks the model to play.
#[derive(Clone, PartialEq, Debug)]
struct Fault {
    kind: FaultKind,
    step: String,
}

impl Fault {
    fn note(&self) -> String {
        match self.kind {
            FaultKind::Fail => format!("故障注入（模拟）：{} 步骤返回否定响应", self.step),
            FaultKind::Lost => format!("故障注入（模拟）：{} 步骤中后端失联", self.step),
        }
    }
}

struct Parsed {
    inspection: Inspection,
    faults: Vec<(String, Fault)>,
}

/// An entry while its lines are read.
struct Draft {
    entry: Entry,
    needs: Vec<String>,
    fault: Option<Fault>,
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn fields(value: &str) -> Vec<&str> {
    value.split('|').map(str::trim).collect()
}

fn hex(s: &str) -> Option<u32> {
    let h = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u32::from_str_radix(h, 16).ok()
}

impl Draft {
    fn new(id: &str) -> Self {
        Self {
            entry: Entry {
                id: id.to_owned(),
                title: String::new(),
                ecu: String::new(),
                steps: Vec::new(),
                images: Vec::new(),
                params: Vec::new(),
                issues: Vec::new(),
            },
            needs: Vec::new(),
            fault: None,
        }
    }

    fn bad(&mut self, n: usize, what: String) {
        self.entry
            .issues
            .push(Issue::unsupported(format!("第 {n} 行：{what}")));
    }

    fn field(&mut self, n: usize, key: &str, value: &str, notes: &mut Vec<String>) {
        let e = &mut self.entry;
        match key {
            "title" => e.title = value.to_owned(),
            "ecu" => e.ecu = value.to_owned(),
            "image" => match fields(value)[..] {
                [file, base, len] => match (hex(base), len.parse::<u32>()) {
                    (Some(base), Ok(len))
                        if len > 0 && u64::from(base) + u64::from(len) <= 1 << 32 =>
                    {
                        e.images.push(Image {
                            file: file.to_owned(),
                            base,
                            len,
                        });
                    }
                    (Some(_), Ok(_)) => self.bad(n, format!("镜像 {file} 地址越界或长度为 0")),
                    _ => self.bad(n, format!("镜像 {file} 的地址或长度无法解析")),
                },
                _ => self.bad(n, "image 应为「文件 | 起始地址 | 字节数」".into()),
            },
            "needs" => self.needs.push(value.to_owned()),
            "step" => match fields(value)[..] {
                [kind, label] => {
                    let known = KINDS.iter().find(|k| k.0 == kind);
                    if known.is_none() {
                        self.bad(n, format!("步骤类型 {kind} 不受模拟后端支持"));
                    }
                    self.entry.steps.push(StepDef {
                        id: kind.to_owned(),
                        label: label.to_owned(),
                        effect: known.is_some_and(|k| k.1),
                    });
                }
                _ => self.bad(n, "step 应为「类型 | 名称」".into()),
            },
            "param" => match fields(value)[..] {
                [id, label, unit, range, default] => {
                    let range = range.split_once("..").and_then(|(lo, hi)| {
                        Some((lo.parse::<u32>().ok()?, hi.parse::<u32>().ok()?))
                    });
                    match (range, default.parse::<u32>()) {
                        (Some((lo, hi)), Ok(d)) if (lo..=hi).contains(&d) => e.params.push(Param {
                            id: id.to_owned(),
                            label: label.to_owned(),
                            unit: unit.to_owned(),
                            lo,
                            hi,
                            default: d,
                            text: d.to_string(),
                            editable: true,
                            origin: "流程文件".into(),
                        }),
                        _ => self.bad(n, format!("参数 {id} 的范围或默认值无效")),
                    }
                }
                _ => self.bad(
                    n,
                    "param 应为「ID | 名称 | 单位 | 下限..上限 | 默认值」".into(),
                ),
            },
            "const" => match fields(value)[..] {
                [id, label, text] => e.params.push(Param {
                    id: id.to_owned(),
                    label: label.to_owned(),
                    unit: String::new(),
                    lo: 0,
                    hi: 0,
                    default: 0,
                    text: text.to_owned(),
                    editable: false,
                    origin: "流程文件（只读）".into(),
                }),
                _ => self.bad(n, "const 应为「ID | 名称 | 值」".into()),
            },
            "simulate" => {
                let fault = match value.split_once('@') {
                    Some(("fail", s)) => Some((FaultKind::Fail, s)),
                    Some(("lost", s)) => Some((FaultKind::Lost, s)),
                    _ => None,
                };
                match fault {
                    Some((kind, step)) => {
                        self.fault = Some(Fault {
                            kind,
                            step: step.trim().to_owned(),
                        })
                    }
                    None => self.bad(n, format!("simulate = {value} 无法识别")),
                }
            }
            "requires" => self.bad(n, format!("需要 {value}：模拟后端不提供")),
            "script" => {
                self.bad(n, format!("外部脚本 {value}：模拟后端不执行"));
                notes.push(format!(
                    "入口 {} 引用脚本 {value}：离线检查只记录，未执行",
                    self.entry.id
                ));
            }
            _ => self.bad(n, format!("未知字段 {key}，不跳过")),
        }
    }

    /// Checks that need the whole entry and the package's file list.
    fn finish(mut self, files: &[String]) -> (Entry, Option<Fault>) {
        let e = &mut self.entry;
        if e.title.is_empty() {
            e.title = e.id.clone();
        }
        if e.ecu.is_empty() {
            e.issues.push(Issue::missing("未声明目标 ECU"));
        }
        if e.steps.is_empty() {
            e.issues
                .push(Issue::missing("没有可执行步骤（缺少 Flash Job）"));
        }
        for i in &e.images {
            if !files.contains(&i.file) {
                e.issues.push(Issue::missing(format!(
                    "镜像 {} 不在包清单（files）中",
                    i.file
                )));
            }
        }
        for f in &self.needs {
            if !files.contains(f) {
                e.issues
                    .push(Issue::missing(format!("依赖 {f} 不在包清单（files）中")));
            }
        }
        if e.steps.iter().any(|s| s.id == "download") && e.images.is_empty() {
            e.issues.push(Issue::missing("下载步骤没有镜像"));
        }
        if let Some(f) = &self.fault {
            if !e.steps.iter().any(|s| s.id == f.step) {
                e.issues.push(Issue::unsupported(format!(
                    "simulate 指向的步骤 {} 不在入口中",
                    f.step
                )));
            }
        }
        (self.entry, self.fault)
    }
}

fn parse(asset: &Asset) -> Result<Parsed, String> {
    let text = std::str::from_utf8(&asset.bytes)
        .map_err(|_| "不是 UTF-8 文本，无法识别为模拟流程包".to_owned())?;
    let (mut format, mut version, mut package) = (None, None, None);
    let mut files = Vec::new();
    let mut issues = Vec::new();
    let mut notes = Vec::new();
    let mut drafts: Vec<Draft> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let id = line
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.trim().strip_prefix("entry "))
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match id {
                Some(id) if drafts.iter().any(|d| d.entry.id == id) => {
                    issues.push(Issue::unsupported(format!("第 {n} 行：入口 {id} 重复")));
                }
                Some(id) => drafts.push(Draft::new(id)),
                None => issues.push(Issue::unsupported(format!(
                    "第 {n} 行：无法识别的分节 {line}"
                ))),
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            issues.push(Issue::unsupported(format!("第 {n} 行：缺少「=」")));
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match drafts.last_mut() {
            Some(d) => d.field(n, key, value, &mut notes),
            None => match key {
                "format" => format = Some(value),
                "version" => version = Some(value),
                "package" => package = Some(value),
                "files" => files = list(value),
                _ => issues.push(Issue::unsupported(format!(
                    "第 {n} 行：未知字段 {key}，不跳过"
                ))),
            },
        }
    }
    if format != Some(FORMAT) {
        return Err(format!(
            "无法识别：缺少「format = {FORMAT}」，不是模拟流程包"
        ));
    }
    match version {
        Some(VERSION) => {}
        Some(v) => issues.push(Issue::unsupported(format!(
            "包版本 {v} 不受支持（模拟后端只支持 {VERSION}）"
        ))),
        None => issues.push(Issue::missing("缺少 version")),
    }
    if drafts.is_empty() {
        issues.push(Issue::missing("包内没有入口"));
    }
    let mut entries = Vec::new();
    let mut faults = Vec::new();
    for d in drafts {
        let (entry, fault) = d.finish(&files);
        if let Some(f) = fault {
            faults.push((entry.id.clone(), f));
        }
        entries.push(entry);
    }
    Ok(Parsed {
        inspection: Inspection {
            format: FORMAT.into(),
            version: version.unwrap_or("?").into(),
            package: package.map_or_else(|| asset.label.clone(), str::to_owned),
            issues,
            entries,
            notes,
        },
        faults,
    })
}

/// One simulated run.
struct Run {
    id: RunId,
    plan: RunPlan,
    fault: Option<Fault>,
    /// Index of the step under way.
    step: usize,
    /// When it began.
    since: Duration,
    /// Parts of it done.
    parts: u32,
    /// Steps finished.
    done: usize,
    cancel: Option<Duration>,
    confirmed: u64,
    effects: Vec<String>,
    events: Vec<BackendEvent>,
    result: Option<RunResult>,
}

impl Run {
    fn new(id: RunId, plan: RunPlan, fault: Option<Fault>, now: Duration) -> Self {
        let mut r = Self {
            id,
            plan,
            fault,
            step: 0,
            since: now,
            parts: 0,
            done: 0,
            cancel: None,
            confirmed: 0,
            effects: Vec::new(),
            events: Vec::new(),
            result: None,
        };
        r.begin(0, now);
        r
    }

    fn step_tag(&self, i: usize) -> String {
        format!("{}:{}", i + 1, self.plan.steps[i].id)
    }

    fn emit(&mut self, at: Duration, level: Level, text: String, kind: EventKind) {
        let step = Some(self.step_tag(self.step));
        self.events.push(BackendEvent {
            at,
            step,
            level,
            text,
            kind,
        });
    }

    fn begin(&mut self, i: usize, at: Duration) {
        self.step = i;
        self.since = at;
        self.parts = 0;
        // Per-download scope: Bytes.total is image_bytes(), so confirmed must not
        // accumulate across multiple download steps in one run.
        if self.plan.steps[i].id == "download" {
            self.confirmed = 0;
        }
        let label = self.plan.steps[i].label.clone();
        self.emit(
            at,
            Level::Info,
            format!("模拟：开始「{label}」"),
            EventKind::StepStarted(i),
        );
    }

    fn parts_of(&self, i: usize) -> u32 {
        if self.plan.steps[i].id == "download" {
            PARTS
        } else {
            1
        }
    }

    fn fault_here(&self, kind: FaultKind) -> bool {
        self.fault
            .as_ref()
            .is_some_and(|f| f.kind == kind && f.step == self.plan.steps[self.step].id)
    }

    fn end(&mut self, outcome: Outcome, error: Option<String>, unconfirmed: Vec<String>) {
        let evidence = vec![
            format!(
                "模拟事件流：运行 {} 完成 {} / {} 个步骤",
                self.id,
                self.done,
                self.plan.steps.len()
            ),
            "未与任何 ECU 通信；不能作为刷写、校验或应用版本的证据".into(),
        ];
        self.result = Some(RunResult {
            outcome,
            mode: Mode::Simulated,
            error,
            effects: self.effects.clone(),
            unconfirmed,
            evidence,
        });
    }

    fn effect_text(&self, s: &StepDef) -> String {
        match s.id.as_str() {
            "erase" => {
                let ranges: Vec<String> = self
                    .plan
                    .images
                    .iter()
                    .map(|i| range_text(i.base, i.len, 8))
                    .collect();
                format!("已擦除（模拟）：{}", ranges.join("、"))
            }
            "download" => format!("已确认下载 {} B（模拟）", self.confirmed),
            "reset" => "已发出 ECU 复位（模拟）".into(),
            _ => format!("「{}」已完成（模拟）", s.label),
        }
    }

    /// Run the timeline up to `now`.
    fn advance(&mut self, now: Duration) {
        while self.result.is_none() {
            let n = self.parts_of(self.step);
            let at = self.since + STEP * (self.parts + 1) / n;
            if at > now {
                break;
            }
            self.part_done(at);
        }
    }

    fn part_done(&mut self, at: Duration) {
        let i = self.step;
        let s = self.plan.steps[i].clone();
        let n = self.parts_of(i);
        let k = self.parts + 1;
        let cancelled = self.cancel.is_some_and(|c| c <= at);
        let total = self.plan.image_bytes();
        if s.id == "download" && cancelled {
            self.emit(
                at,
                Level::Warn,
                format!("模拟：在第 {k} / {n} 段处理停止请求，不等待已发出的该段响应"),
                EventKind::Log,
            );
            if self.confirmed > 0 {
                self.effects.push(format!(
                    "已确认下载 {} / {total} B（模拟），目标镜像不完整",
                    self.confirmed
                ));
            }
            self.end(
                Outcome::Unknown,
                Some("停止于下载中途，最后一段请求是否写入无法确认（模拟）".into()),
                vec![format!(
                    "「{}」第 {k} / {n} 段：已发出，响应未确认（模拟）",
                    s.label
                )],
            );
            return;
        }
        if self.fault_here(FaultKind::Lost) && k == (n / 2).max(1) {
            self.emit(
                at,
                Level::Error,
                format!("模拟：后端失联，「{}」的在途请求未确认", s.label),
                EventKind::Log,
            );
            if self.confirmed > 0 {
                self.effects
                    .push(format!("已确认下载 {} / {total} B（模拟）", self.confirmed));
            }
            self.end(
                Outcome::Unknown,
                Some("后端失联（模拟）；不自动重连，也不继续刷写".into()),
                vec![format!(
                    "「{}」第 {k} / {n} 段：已发出，响应未确认（模拟）",
                    s.label
                )],
            );
            return;
        }
        self.parts = k;
        if s.id == "download" {
            self.confirmed +=
                total * u64::from(k) / u64::from(n) - total * u64::from(k - 1) / u64::from(n);
            let confirmed = self.confirmed;
            self.emit(
                at,
                Level::Info,
                format!("模拟：已确认下载 {confirmed} / {total} B"),
                EventKind::Bytes { confirmed, total },
            );
        }
        if k < n {
            return;
        }
        if self.fault_here(FaultKind::Fail) {
            self.emit(
                at,
                Level::Error,
                format!("模拟：「{}」收到否定响应 NRC 0x72", s.label),
                EventKind::Log,
            );
            let unconfirmed = if s.effect {
                vec![format!("「{}」失败时目标可能已部分改变（模拟）", s.label)]
            } else {
                Vec::new()
            };
            self.end(
                Outcome::Failed,
                Some(format!("「{}」失败：NRC 0x72（模拟）", s.label)),
                unconfirmed,
            );
            return;
        }
        self.emit(
            at,
            Level::Info,
            format!("模拟：完成「{}」", s.label),
            EventKind::StepDone(i),
        );
        self.done += 1;
        if s.effect {
            let text = self.effect_text(&s);
            self.effects.push(text);
        }
        if i + 1 == self.plan.steps.len() {
            self.end(Outcome::Succeeded, None, Vec::new());
            return;
        }
        if cancelled {
            self.emit(
                at,
                Level::Warn,
                "模拟：在步骤边界停止，后续步骤未执行".into(),
                EventKind::Log,
            );
            self.end(Outcome::Stopped, None, Vec::new());
            return;
        }
        self.begin(i + 1, at);
    }
}

/// The simulated backend. Plans and runs live in memory only.
#[derive(Default)]
pub struct MockBackend {
    plans: u64,
    prepared: Vec<(u64, Option<Fault>)>,
    started: Vec<u64>,
    runs: Vec<Run>,
}

impl MockBackend {
    fn run_mut(&mut self, id: RunId) -> Option<&mut Run> {
        self.runs.iter_mut().find(|r| r.id == id)
    }
}

impl Backend for MockBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            name: NAME.into(),
            version: BACKEND_VERSION.into(),
            api: API_VERSION,
            mode: Mode::Simulated,
            formats: vec![(FORMAT.into(), VERSION.into())],
            platforms: "任意（不访问硬件）".into(),
            link: "无：不打开总线，不经 USB".into(),
            cancel: Some("下载中途按段，其余在步骤边界".into()),
            events: "步骤 + 已确认字节（模拟），无原始报文".into(),
        }
    }

    fn inspect(&self, asset: &Asset) -> Result<Inspection, String> {
        parse(asset).map(|p| p.inspection)
    }

    fn prepare(&mut self, req: &PrepareRequest) -> Result<RunPlan, Vec<String>> {
        let p = parse(req.asset).map_err(|e| vec![e])?;
        let ins = &p.inspection;
        let e = ins
            .entry(req.entry)
            .ok_or_else(|| vec![format!("入口 {} 不在包中", req.entry)])?;
        let mut errs: Vec<String> = ins.blockers(e).map(|i| i.text.clone()).collect();
        for (id, _) in req.params {
            if !e.params.iter().any(|p| p.editable && &p.id == id) {
                errs.push(format!("参数 {id} 不是该入口可修改的参数"));
            }
        }
        let mut params = Vec::new();
        for p in &e.params {
            let value = if p.editable {
                let text = req
                    .params
                    .iter()
                    .find(|(id, _)| *id == p.id)
                    .map_or_else(|| p.default.to_string(), |(_, v)| v.trim().to_owned());
                match text.parse::<u32>() {
                    Ok(v) if (p.lo..=p.hi).contains(&v) => format!("{v} {}", p.unit),
                    _ => {
                        errs.push(format!("{}填 {} 到 {} 的整数", p.label, p.lo, p.hi));
                        continue;
                    }
                }
            } else {
                p.text.clone()
            };
            params.push(Bound {
                id: p.id.clone(),
                label: p.label.clone(),
                value: value.trim().to_owned(),
                editable: p.editable,
            });
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let fault = p
            .faults
            .iter()
            .find(|(id, _)| *id == e.id)
            .map(|(_, f)| f.clone());
        self.plans += 1;
        let mut notes = vec!["模拟后端：不打开总线，不发送 UDS 请求".to_owned()];
        notes.extend(fault.as_ref().map(Fault::note));
        self.prepared.push((self.plans, fault));
        Ok(RunPlan {
            plan_id: self.plans,
            backend: format!("{NAME} {BACKEND_VERSION}"),
            api: API_VERSION,
            mode: Mode::Simulated,
            asset: req.asset.label.clone(),
            asset_hash: req.asset.hash,
            package: ins.package.clone(),
            entry: e.id.clone(),
            title: e.title.clone(),
            ecu: e.ecu.clone(),
            steps: e.steps.clone(),
            images: e.images.clone(),
            params,
            notes,
        })
    }

    fn start(&mut self, plan: &RunPlan, now: Duration) -> Result<RunId, String> {
        if self.started.contains(&plan.plan_id) {
            return Err(format!(
                "运行计划 P{:04} 已启动过；同一计划只启动一次",
                plan.plan_id
            ));
        }
        let Some((_, fault)) = self.prepared.iter().find(|(id, _)| *id == plan.plan_id) else {
            return Err("运行计划不是本后端准备的".into());
        };
        if plan.steps.is_empty() {
            return Err("运行计划没有步骤".into());
        }
        let fault = fault.clone();
        self.started.push(plan.plan_id);
        let id = RunId(self.runs.len() as u64 + 1);
        self.runs.push(Run::new(id, plan.clone(), fault, now));
        Ok(id)
    }

    fn poll(&mut self, run: RunId, now: Duration) -> Vec<BackendEvent> {
        match self.run_mut(run) {
            Some(r) => {
                r.advance(now);
                std::mem::take(&mut r.events)
            }
            None => Vec::new(),
        }
    }

    fn request_cancel(&mut self, run: RunId, now: Duration) -> CancelReply {
        let Some(r) = self.run_mut(run) else {
            return CancelReply::NotRunning;
        };
        r.advance(now);
        if r.result.is_some() {
            return CancelReply::NotRunning;
        }
        if r.cancel.is_some() {
            return CancelReply::Pending;
        }
        r.cancel = Some(now);
        let how = if r.plan.steps[r.step].id == "download" {
            "下载中途：在下一段边界停止，不等待已发出的段"
        } else {
            "在当前步骤结束后停止"
        };
        r.emit(
            now,
            Level::Warn,
            format!("模拟：停止请求已接收；{how}"),
            EventKind::Log,
        );
        CancelReply::Accepted
    }

    fn result(&self, run: RunId) -> Option<RunResult> {
        self.runs
            .iter()
            .find(|r| r.id == run)
            .and_then(|r| r.result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::api::IssueKind;
    use crate::flow::fnv1a64;

    fn asset(text: &str) -> Asset {
        Asset {
            label: "test.udsmock".into(),
            bytes: text.as_bytes().to_vec(),
            hash: fnv1a64(text.as_bytes()),
        }
    }

    fn inspect(text: &str) -> Inspection {
        MockBackend::default().inspect(&asset(text)).unwrap()
    }

    const HEAD: &str = "format = uds-mock-fixture\nversion = 1\nfiles = a.bin\n";

    #[test]
    fn builtin_fixtures_parse_as_documented() {
        let b = MockBackend::default();
        let ins: Vec<Inspection> = FIXTURES
            .iter()
            .map(|(_, _, text)| b.inspect(&asset(text)).unwrap())
            .collect();
        // single: one entry, nothing missing or unsupported
        assert_eq!(ins[0].entries.len(), 1);
        assert!(ins[0].issues.is_empty());
        assert!(ins[0].entries[0].issues.is_empty());
        // multi: three runnable entries
        assert_eq!(ins[1].entries.len(), 3);
        assert!(ins[1].entries.iter().all(|e| e.issues.is_empty()));
        // blocked: one entry with missing and unsupported items, a script note
        let e = &ins[2].entries[0];
        assert!(e.issues.iter().any(|i| i.kind == IssueKind::Missing));
        assert!(e.issues.iter().any(|i| i.kind == IssueKind::Unsupported));
        assert!(ins[2].notes.iter().any(|n| n.contains("未执行")));
    }

    #[test]
    fn unknown_formats_are_not_recognised() {
        let b = MockBackend::default();
        assert!(b.inspect(&asset("<ODX/>")).is_err());
        assert!(b
            .inspect(&Asset {
                label: "bin".into(),
                bytes: vec![0xff, 0xfe, 0],
                hash: 0,
            })
            .is_err());
    }

    #[test]
    fn checks_name_what_is_wrong() {
        let ins = inspect(&format!(
            "{HEAD}vendor = x\n[entry e]\necu = E\nimage = b.bin | 0x0 | 16\n\
             step = session | S\nstep = flash_magic | M\nwibble = 1\n\
             param = p | P | ms | 10..1 | 5\nsimulate = fail@verify\n[entry e]\n"
        ));
        let pkg: Vec<&str> = ins.issues.iter().map(|i| i.text.as_str()).collect();
        assert!(pkg.iter().any(|t| t.contains("未知字段 vendor")), "{pkg:?}");
        assert!(pkg.iter().any(|t| t.contains("入口 e 重复")), "{pkg:?}");
        let e: Vec<&str> = ins.entries[0]
            .issues
            .iter()
            .map(|i| i.text.as_str())
            .collect();
        for want in [
            "镜像 b.bin 不在包清单",
            "步骤类型 flash_magic",
            "未知字段 wibble",
            "参数 p 的范围",
            "simulate 指向的步骤 verify",
        ] {
            assert!(e.iter().any(|t| t.contains(want)), "{want}: {e:?}");
        }
    }

    #[test]
    fn package_version_and_empty_entries_block() {
        let ins = inspect("format = uds-mock-fixture\nversion = 2\n");
        let texts: Vec<&str> = ins.issues.iter().map(|i| i.text.as_str()).collect();
        assert!(texts.iter().any(|t| t.contains("包版本 2")));
        assert!(texts.iter().any(|t| t.contains("没有入口")));
        let ins = inspect(&format!("{HEAD}[entry e]\necu = E\n"));
        assert!(ins.entries[0]
            .issues
            .iter()
            .any(|i| i.text.contains("Flash Job")));
    }

    #[test]
    fn images_out_of_range_are_refused() {
        let ins = inspect(&format!(
            "{HEAD}[entry e]\necu = E\nimage = a.bin | 0xFFFFFFF0 | 32\nstep = session | S\n"
        ));
        assert!(ins.entries[0].images.is_empty());
        assert!(ins.entries[0].issues[0].text.contains("越界"));
    }

    #[test]
    fn prepare_binds_and_checks_parameters() {
        let mut b = MockBackend::default();
        let a = asset(FIXTURES[0].2);
        let id = b.inspect(&a).unwrap().entries[0].id.clone();
        let bad = [("p2".to_owned(), "0".to_owned())];
        let err = b
            .prepare(&PrepareRequest {
                asset: &a,
                entry: &id,
                params: &bad,
            })
            .unwrap_err();
        assert!(err[0].contains("P2"), "{err:?}");
        let stray = [("nope".to_owned(), "1".to_owned())];
        assert!(b
            .prepare(&PrepareRequest {
                asset: &a,
                entry: &id,
                params: &stray,
            })
            .is_err());
        let good = [("p2".to_owned(), "80".to_owned())];
        let plan = b
            .prepare(&PrepareRequest {
                asset: &a,
                entry: &id,
                params: &good,
            })
            .unwrap();
        let p2 = plan.params.iter().find(|p| p.id == "p2").unwrap();
        assert_eq!(p2.value, "80 ms");
        // read-only ones keep their text; defaults fill what was not given
        assert!(plan
            .params
            .iter()
            .any(|p| !p.editable && p.value == "0x7E0"));
        assert!(plan
            .params
            .iter()
            .any(|p| p.id == "p2x" && p.value == "5000 ms"));
        assert_eq!(plan.mode, Mode::Simulated);
        assert_eq!(plan.asset_hash, a.hash);
    }

    #[test]
    fn a_plan_starts_once() {
        let mut b = MockBackend::default();
        let a = asset(FIXTURES[0].2);
        let plan = b
            .prepare(&PrepareRequest {
                asset: &a,
                entry: "app",
                params: &[],
            })
            .unwrap();
        assert!(b.start(&plan, Duration::ZERO).is_ok());
        assert!(b
            .start(&plan, Duration::ZERO)
            .unwrap_err()
            .contains("只启动一次"));
        let mut foreign = plan.clone();
        foreign.plan_id = 99;
        assert!(b.start(&foreign, Duration::ZERO).is_err());
    }

    #[test]
    fn every_event_says_simulated() {
        let mut b = MockBackend::default();
        let a = asset(FIXTURES[0].2);
        let plan = b
            .prepare(&PrepareRequest {
                asset: &a,
                entry: "app",
                params: &[],
            })
            .unwrap();
        let run = b.start(&plan, Duration::ZERO).unwrap();
        let events = b.poll(run, STEP * 100);
        assert!(!events.is_empty());
        assert!(events.iter().all(|e| e.text.starts_with("模拟")));
        let r = b.result(run).unwrap();
        assert_eq!((r.outcome, r.mode), (Outcome::Succeeded, Mode::Simulated));
        assert!(r.evidence.iter().any(|e| e.contains("不能作为刷写")));
        // download bytes are reported apart from steps, up to the image size
        let last = events.iter().rev().find_map(|e| match e.kind {
            EventKind::Bytes { confirmed, total } => Some((confirmed, total)),
            _ => None,
        });
        assert_eq!(last, Some((262_144, 262_144)));
    }

    #[test]
    fn two_download_steps_keep_confirmed_within_each_step_total() {
        // Reproduce the Codex finding: without a per-download reset, confirmed
        // would climb past image_bytes() once a second download step finishes.
        let text = "\
format = uds-mock-fixture
version = 1
package = 双下载作用域
files = app.bin
[entry dual]
title = 两个 download（模拟）
ecu = DEMO
image = app.bin | 0x10000 | 100
step = erase | 擦除
step = download | 第一段下载
step = download | 第二段下载
step = verify | 校验
";
        let mut b = MockBackend::default();
        let a = asset(text);
        let plan = b
            .prepare(&PrepareRequest {
                asset: &a,
                entry: "dual",
                params: &[],
            })
            .unwrap();
        assert_eq!(plan.image_bytes(), 100);
        assert_eq!(plan.steps.iter().filter(|s| s.id == "download").count(), 2);
        let run = b.start(&plan, Duration::ZERO).unwrap();
        let events = b.poll(run, STEP * 100);
        let bytes: Vec<(u64, u64)> = events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Bytes { confirmed, total } => Some((confirmed, total)),
                _ => None,
            })
            .collect();
        assert!(!bytes.is_empty(), "expected Bytes events");
        for &(confirmed, total) in &bytes {
            assert_eq!(total, 100);
            assert!(
                confirmed <= total,
                "confirmed {confirmed} > total {total} in {bytes:?}"
            );
        }
        // Each download reports PARTS progress events ending at total.
        let ends: Vec<_> = bytes.iter().copied().filter(|&(c, t)| c == t).collect();
        assert_eq!(
            ends.len(),
            2,
            "one end-of-step Bytes per download: {bytes:?}"
        );
        let r = b.result(run).unwrap();
        assert_eq!(r.outcome, Outcome::Succeeded);
    }
}
