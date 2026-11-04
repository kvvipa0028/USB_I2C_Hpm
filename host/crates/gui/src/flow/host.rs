//! Host task manager (proposal §4.2, §5.3): reads a flow asset, has the
//! backend inspect it, binds one explicitly chosen entry into a locked run
//! plan, starts that plan once, and archives every host and backend event
//! of the task in order. One task at a time (single ECU, single channel).
//!
//! Rules it keeps: a plan with any missing or unsupported item is never
//! prepared; a package with several entries is never run without an
//! explicit choice; a plan starts at most once and a failed or unknown run
//! is never retried; a stop request the backend accepts moves the task to
//! 停止处理中 only, and the final state comes from the backend's result. In
//! P1 the host accepts only a simulated backend.

use std::path::PathBuf;
use std::time::Duration;

use super::api::{
    Asset, Backend, BackendEvent, CancelReply, Capabilities, EventKind, Inspection, Level, Mode,
    Outcome, PrepareRequest, RunId, RunPlan, RunResult, API_VERSION,
};
use super::fnv1a64;

/// Largest asset the host reads (proposal §6.4: bounded package parsing).
pub const MAX_ASSET: u64 = 256 * 1024;

/// Task states, as §5.3 names them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Preparing,
    Ready,
    Running,
    Stopping,
    Succeeded,
    Failed,
    Stopped,
    Unknown,
}

impl State {
    pub const ALL: [State; 8] = [
        State::Preparing,
        State::Ready,
        State::Running,
        State::Stopping,
        State::Succeeded,
        State::Failed,
        State::Stopped,
        State::Unknown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            State::Preparing => "准备中",
            State::Ready => "就绪",
            State::Running => "运行中",
            State::Stopping => "停止处理中",
            State::Succeeded => "成功",
            State::Failed => "失败",
            State::Stopped => "已停止",
            State::Unknown => "结果未知",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(
            self,
            State::Succeeded | State::Failed | State::Stopped | State::Unknown
        )
    }

    /// A run is under way: no new import or plan until it ends.
    pub fn active(self) -> bool {
        matches!(self, State::Running | State::Stopping)
    }

    /// The transitions the host allows. Final states have none: a new run
    /// needs a new plan.
    pub fn can_go(self, to: State) -> bool {
        use State::*;
        matches!(
            (self, to),
            (Preparing, Ready)
                | (Ready, Running)
                // the backend refused to start the plan
                | (Ready, Failed)
                | (Running, Stopping | Succeeded | Failed | Unknown)
                // the last step may end before the stop request is handled
                | (Stopping, Stopped | Succeeded | Failed | Unknown)
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Host,
    Backend,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Host => "宿主",
            Origin::Backend => "后端",
        }
    }
}

/// One archived event (§4.3): run id once started, sequence number in the
/// task, time, step, level and text.
#[derive(Clone, PartialEq, Debug)]
pub struct Record {
    pub run: Option<RunId>,
    pub seq: u64,
    pub at: Duration,
    pub step: Option<String>,
    pub level: Level,
    pub origin: Origin,
    pub text: String,
}

/// Where an asset comes from.
#[derive(Clone, PartialEq, Debug)]
pub enum Source {
    Builtin {
        name: &'static str,
        text: &'static str,
    },
    File(PathBuf),
}

impl Source {
    /// Read the asset. Only reads: a file is never run or unpacked here.
    pub fn load(&self) -> Result<Asset, String> {
        let (label, bytes) = match self {
            Source::Builtin { name, text } => {
                (format!("内置示例 · {name}"), text.as_bytes().to_vec())
            }
            Source::File(path) => {
                let shown = path.display().to_string();
                let meta = std::fs::metadata(path).map_err(|e| format!("无法读取 {shown}：{e}"))?;
                if !meta.is_file() {
                    return Err(format!("{shown} 不是文件"));
                }
                if meta.len() > MAX_ASSET {
                    return Err(format!(
                        "{shown} 有 {} B，超过导入上限 {MAX_ASSET} B",
                        meta.len()
                    ));
                }
                let bytes = std::fs::read(path).map_err(|e| format!("无法读取 {shown}：{e}"))?;
                if bytes.len() as u64 > MAX_ASSET {
                    return Err(format!("{shown} 超过导入上限 {MAX_ASSET} B"));
                }
                (shown, bytes)
            }
        };
        let hash = fnv1a64(&bytes);
        Ok(Asset { label, bytes, hash })
    }
}

/// Step and byte progress, kept apart (§4.3): steps come from step events,
/// bytes only from what the backend says was confirmed.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Progress {
    pub done: usize,
    pub current: Option<usize>,
    pub bytes: Option<(u64, u64)>,
}

/// One task: the locked plan, where it stands, and its archive.
pub struct Task {
    state: State,
    plan: RunPlan,
    run: Option<RunId>,
    archive: Vec<Record>,
    progress: Progress,
    cancel: Option<CancelReply>,
    result: Option<RunResult>,
    prepared: Duration,
    started: Option<Duration>,
    ended: Option<Duration>,
}

impl Task {
    fn new(plan: RunPlan, now: Duration) -> Self {
        let mut t = Self {
            state: State::Preparing,
            plan,
            run: None,
            archive: Vec::new(),
            progress: Progress::default(),
            cancel: None,
            result: None,
            prepared: now,
            started: None,
            ended: None,
        };
        let text = format!(
            "状态：{}；后端 {} 返回运行计划 P{:04}（入口 {}）",
            State::Preparing.label(),
            t.plan.backend,
            t.plan.plan_id,
            t.plan.entry
        );
        t.log(now, None, Level::Info, Origin::Host, text);
        t
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// The snapshot: fixed once prepare succeeded.
    pub fn plan(&self) -> &RunPlan {
        &self.plan
    }

    pub fn run_id(&self) -> Option<RunId> {
        self.run
    }

    pub fn archive(&self) -> &[Record] {
        &self.archive
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    pub fn cancel(&self) -> Option<CancelReply> {
        self.cancel
    }

    pub fn result(&self) -> Option<&RunResult> {
        self.result.as_ref()
    }

    pub fn prepared(&self) -> Duration {
        self.prepared
    }

    pub fn started(&self) -> Option<Duration> {
        self.started
    }

    pub fn ended(&self) -> Option<Duration> {
        self.ended
    }

    fn log(
        &mut self,
        at: Duration,
        step: Option<String>,
        level: Level,
        origin: Origin,
        text: String,
    ) {
        let seq = self.archive.len() as u64;
        self.archive.push(Record {
            run: self.run,
            seq,
            at,
            step,
            level,
            origin,
            text,
        });
    }

    /// Move to `to` if the table allows it; otherwise archive the refusal
    /// and stay.
    fn go(&mut self, to: State, at: Duration, why: &str) -> bool {
        let from = self.state;
        if !from.can_go(to) {
            let text = format!("宿主拒绝状态转换 {} → {}", from.label(), to.label());
            self.log(at, None, Level::Error, Origin::Host, text);
            return false;
        }
        self.state = to;
        if to.terminal() {
            self.ended = Some(at);
        }
        let text = format!("状态：{} → {}；{why}", from.label(), to.label());
        let level = match to {
            State::Failed | State::Unknown => Level::Error,
            State::Stopping | State::Stopped => Level::Warn,
            _ => Level::Info,
        };
        self.log(at, None, level, Origin::Host, text);
        true
    }

    fn apply(&mut self, ev: BackendEvent) {
        match ev.kind {
            EventKind::StepStarted(i) => self.progress.current = Some(i),
            EventKind::StepDone(i) => {
                self.progress.done = self.progress.done.max(i + 1);
                self.progress.current = None;
            }
            EventKind::Bytes { confirmed, total } => self.progress.bytes = Some((confirmed, total)),
            EventKind::Log => {}
        }
        self.log(ev.at, ev.step, ev.level, Origin::Backend, ev.text);
    }

    fn finish(&mut self, mut res: RunResult, now: Duration) {
        let to = if res.mode != self.plan.mode {
            let note = format!(
                "后端报告的运行模式「{}」与运行计划「{}」不一致，宿主不采用其结论",
                res.mode.label(),
                self.plan.mode.label()
            );
            res.error = Some(match res.error.take() {
                Some(e) => format!("{note}；后端：{e}"),
                None => note,
            });
            State::Unknown
        } else {
            match (res.outcome, self.state) {
                (Outcome::Succeeded, _) => State::Succeeded,
                (Outcome::Failed, _) => State::Failed,
                (Outcome::Stopped, State::Stopping) => State::Stopped,
                (Outcome::Stopped, _) => {
                    res.error = Some("后端在没有停止请求时报告已停止，按结果未知处理".into());
                    State::Unknown
                }
                (Outcome::Unknown, _) => State::Unknown,
            }
        };
        let why = match to {
            State::Succeeded => "后端事件流结束；成功的含义以运行模式为准".to_owned(),
            _ => res.error.clone().unwrap_or_else(|| "后端已结束".into()),
        };
        self.progress.current = None;
        self.result = Some(res);
        self.go(to, now, &why);
    }
}

pub struct Host {
    backend: Box<dyn Backend>,
    caps: Capabilities,
    source: Option<Source>,
    asset: Option<Asset>,
    inspection: Option<Inspection>,
    task: Option<Task>,
}

impl Host {
    /// Take `backend` if it speaks this host's API version and, in P1, only
    /// simulates.
    pub fn new(backend: Box<dyn Backend>) -> Result<Self, String> {
        let caps = backend.capabilities();
        if !API_VERSION.accepts(caps.api) {
            return Err(format!(
                "后端 {} 的接口版本 {} 与宿主 {API_VERSION} 不兼容",
                caps.name, caps.api
            ));
        }
        if caps.mode != Mode::Simulated {
            return Err(format!(
                "后端 {} 声明为「{}」；P1 宿主只接入模拟后端",
                caps.name,
                caps.mode.label()
            ));
        }
        Ok(Self {
            backend,
            caps,
            source: None,
            asset: None,
            inspection: None,
            task: None,
        })
    }

    pub fn caps(&self) -> &Capabilities {
        &self.caps
    }

    pub fn asset(&self) -> Option<&Asset> {
        self.asset.as_ref()
    }

    pub fn inspection(&self) -> Option<&Inspection> {
        self.inspection.as_ref()
    }

    pub fn task(&self) -> Option<&Task> {
        self.task.as_ref()
    }

    pub fn state(&self) -> Option<State> {
        self.task.as_ref().map(Task::state)
    }

    pub fn active(&self) -> bool {
        self.state().is_some_and(State::active)
    }

    /// Read `source` and have the backend check it, offline. A plan that
    /// was ready for the previous asset is dropped; a finished task stays
    /// visible until the next plan replaces it.
    pub fn inspect(&mut self, source: Source) -> Result<(), String> {
        if self.active() {
            return Err("任务运行中，结束后才能重新导入".into());
        }
        if self.state() == Some(State::Ready) {
            self.task = None;
        }
        self.inspection = None;
        self.asset = None;
        self.source = None;
        let asset = source.load()?;
        let ins = self.backend.inspect(&asset)?;
        self.source = Some(source);
        self.asset = Some(asset);
        self.inspection = Some(ins);
        Ok(())
    }

    /// Bind `entry` and `params` into a locked plan. `entry` must be given;
    /// with several entries there is no default.
    pub fn prepare(
        &mut self,
        entry: Option<&str>,
        params: &[(String, String)],
        now: Duration,
    ) -> Result<(), Vec<String>> {
        if self.active() {
            return Err(vec!["任务运行中，不能准备新的运行计划".into()]);
        }
        let (Some(asset), Some(ins)) = (&self.asset, &self.inspection) else {
            return Err(vec!["先读取并检查流程文件".into()]);
        };
        let Some(id) = entry else {
            return Err(vec![if ins.entries.len() > 1 {
                format!(
                    "包内有 {} 个入口，必须明确选择一个；不会默认执行第一个",
                    ins.entries.len()
                )
            } else {
                "未选择入口".into()
            }]);
        };
        let Some(e) = ins.entry(id) else {
            return Err(vec![format!("入口 {id} 不在检查结果中")]);
        };
        let blockers: Vec<String> = ins
            .blockers(e)
            .map(|i| format!("{}：{}", i.kind.label(), i.text))
            .collect();
        if !blockers.is_empty() {
            return Err(blockers);
        }
        let plan = self.backend.prepare(&PrepareRequest {
            asset,
            entry: id,
            params,
        })?;
        let mut wrong = Vec::new();
        if plan.mode != self.caps.mode {
            wrong.push(format!(
                "运行计划的模式「{}」与后端声明不一致",
                plan.mode.label()
            ));
        }
        if plan.asset_hash != asset.hash {
            wrong.push("运行计划的文件哈希与已检查的文件不一致".into());
        }
        if plan.entry != id {
            wrong.push(format!("运行计划的入口 {} 不是所选的 {id}", plan.entry));
        }
        if !wrong.is_empty() {
            return Err(wrong);
        }
        let mut task = Task::new(plan, now);
        task.go(
            State::Ready,
            now,
            "运行计划已锁定，开始前和运行中都不再改变",
        );
        self.task = Some(task);
        Ok(())
    }

    /// Start the ready plan, once. The asset is read again first; if it
    /// changed since prepare, the plan is dropped and nothing starts.
    pub fn start(&mut self, now: Duration) -> Result<RunId, String> {
        let Some(task) = self.task.as_mut() else {
            return Err("没有运行计划".into());
        };
        if task.state != State::Ready {
            return Err(format!(
                "当前状态为「{}」：同一运行计划只启动一次，失败或未知结果不自动重试",
                task.state.label()
            ));
        }
        let fresh = self.source.as_ref().map(Source::load);
        let same = matches!(&fresh, Some(Ok(a)) if a.hash == task.plan.asset_hash);
        if !same {
            self.task = None;
            self.inspection = None;
            self.asset = None;
            return Err(
                "流程文件在准备后发生变化或无法再读取，运行计划已作废；请重新读取、检查并准备"
                    .into(),
            );
        }
        match self.backend.start(&task.plan, now) {
            Ok(run) => {
                task.run = Some(run);
                task.started = Some(now);
                task.go(State::Running, now, &format!("后端开始运行 {run}"));
                Ok(run)
            }
            Err(e) => {
                task.result = Some(RunResult {
                    outcome: Outcome::Failed,
                    mode: task.plan.mode,
                    error: Some(e.clone()),
                    effects: Vec::new(),
                    unconfirmed: Vec::new(),
                    evidence: vec!["后端拒绝启动".into()],
                });
                task.go(State::Failed, now, &format!("后端拒绝启动：{e}"));
                Err(e)
            }
        }
    }

    /// Ask the backend to stop. An accepted request moves the task to
    /// 停止处理中; it says nothing yet about the ECU.
    pub fn request_cancel(&mut self, now: Duration) -> Result<CancelReply, String> {
        self.poll(now);
        let Some(task) = self.task.as_mut() else {
            return Err("没有任务".into());
        };
        let (true, Some(run)) = (task.state.active(), task.run) else {
            return Err(format!(
                "当前状态为「{}」，没有可停止的运行",
                task.state.label()
            ));
        };
        let reply = if self.caps.cancel.is_some() {
            self.backend.request_cancel(run, now)
        } else {
            CancelReply::Unsupported
        };
        task.cancel = Some(reply);
        match reply {
            CancelReply::Accepted => {
                task.go(
                    State::Stopping,
                    now,
                    "停止请求已被后端接收；这不表示 ECU 已停止，也不表示回滚",
                );
            }
            CancelReply::Pending => {
                task.log(
                    now,
                    None,
                    Level::Info,
                    Origin::Host,
                    "停止请求仍在处理".into(),
                );
            }
            CancelReply::Unsupported => {
                task.log(
                    now,
                    None,
                    Level::Warn,
                    Origin::Host,
                    "后端不支持停止，运行继续".into(),
                );
            }
            CancelReply::NotRunning => {
                task.log(
                    now,
                    None,
                    Level::Info,
                    Origin::Host,
                    "后端报告运行已结束，等待其结果".into(),
                );
            }
        }
        self.poll(now);
        Ok(reply)
    }

    /// Collect the backend's events and, once it has one, its result.
    /// Returns whether the run is still under way.
    pub fn poll(&mut self, now: Duration) -> bool {
        let Some(task) = self.task.as_mut() else {
            return false;
        };
        let (true, Some(run)) = (task.state.active(), task.run) else {
            return false;
        };
        for ev in self.backend.poll(run, now) {
            task.apply(ev);
        }
        if let Some(res) = self.backend.result(run) {
            task.finish(res, now);
        }
        task.state.active()
    }

    /// Drop a ready plan or a finished task. Not while a run is under way.
    pub fn discard(&mut self) -> Result<(), String> {
        if self.active() {
            return Err("任务运行中，不能放弃".into());
        }
        self.task = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::api::ApiVersion;
    use crate::flow::mock::{MockBackend, FIXTURES, PARTS, STEP};

    /// A fresh directory under the system temp dir.
    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("uds-p1-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn builtin(i: usize) -> Source {
        Source::Builtin {
            name: FIXTURES[i].1,
            text: FIXTURES[i].2,
        }
    }

    fn host() -> Host {
        Host::new(Box::<MockBackend>::default()).unwrap()
    }

    fn ready(i: usize, entry: &str) -> Host {
        let mut h = host();
        h.inspect(builtin(i)).unwrap();
        h.prepare(Some(entry), &[], Duration::ZERO).unwrap();
        h
    }

    /// Poll every 50 ms until the run ends or `until`.
    fn run_to(h: &mut Host, from: Duration, until: Duration) -> Duration {
        let mut now = from;
        while h.poll(now) && now < until {
            now += Duration::from_millis(50);
        }
        now
    }

    fn states(h: &Host) -> Vec<String> {
        h.task()
            .unwrap()
            .archive()
            .iter()
            .filter(|r| r.origin == Origin::Host && r.text.starts_with("状态"))
            .map(|r| r.text.clone())
            .collect()
    }

    #[test]
    fn transitions_follow_the_table() {
        use State::*;
        for s in State::ALL {
            if s.terminal() {
                assert!(State::ALL.iter().all(|t| !s.can_go(*t)), "{s:?}");
            }
            // nothing goes back to preparing, and only preparing leads to ready
            assert!(!s.can_go(Preparing));
            assert_eq!(s.can_go(Ready), s == Preparing);
        }
        assert!(Preparing.can_go(Ready));
        assert!(!Preparing.can_go(Running));
        assert!(Ready.can_go(Running));
        assert!(!Ready.can_go(Succeeded));
        assert!(Running.can_go(Stopping));
        // stopping is only reached through a stop request
        assert!(!Running.can_go(Stopped));
        assert!(Stopping.can_go(Stopped) && Stopping.can_go(Unknown));
        assert!(!Stopping.can_go(Running));
    }

    #[test]
    fn single_entry_runs_to_simulated_success() {
        let mut h = ready(0, "app");
        assert_eq!(h.state(), Some(State::Ready));
        let run = h.start(Duration::ZERO).unwrap();
        assert_eq!(h.state(), Some(State::Running));
        let end = run_to(&mut h, Duration::ZERO, STEP * 40);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Succeeded);
        let r = t.result().unwrap();
        assert_eq!(r.mode, Mode::Simulated);
        assert!(r.evidence.iter().any(|e| e.contains("不能作为刷写")));
        assert_eq!(t.progress().done, t.plan().steps.len());
        assert_eq!(t.progress().bytes, Some((262_144, 262_144)));
        assert_eq!(t.ended(), Some(end));
        // the archive counts up from 0 and every record after start has the run id
        for (i, rec) in t.archive().iter().enumerate() {
            assert_eq!(rec.seq, i as u64);
        }
        let started = t.archive().iter().position(|r| r.run.is_some()).unwrap();
        assert!(t.archive()[started..].iter().all(|r| r.run == Some(run)));
        assert!(t.archive().iter().any(|r| r.origin == Origin::Backend));
        assert_eq!(
            states(&h).len(),
            // 准备中, → 就绪, → 运行中, → 成功
            4
        );
        // the same plan never starts again
        assert!(h.start(end).unwrap_err().contains("只启动一次"));
        assert_eq!(h.state(), Some(State::Succeeded));
    }

    #[test]
    fn unsupported_and_missing_items_block_start() {
        let mut h = host();
        h.inspect(builtin(2)).unwrap();
        let ins = h.inspection().unwrap();
        assert_eq!(ins.entries.len(), 1);
        let errs = h.prepare(Some("app"), &[], Duration::ZERO).unwrap_err();
        for want in [
            "flash_driver.bin",
            "SecureFlash",
            "post_flash.js",
            "checksum_vendor",
        ] {
            assert!(errs.iter().any(|e| e.contains(want)), "{want}: {errs:?}");
        }
        assert!(errs.iter().any(|e| e.starts_with("缺少依赖")));
        assert!(errs.iter().any(|e| e.starts_with("不支持")));
        assert!(h.task().is_none());
        assert!(h.start(Duration::ZERO).is_err());
    }

    #[test]
    fn package_issues_block_every_entry() {
        let mut h = host();
        let dir = scratch("pkg");
        let path = dir.join("v2.udsmock");
        std::fs::write(
            &path,
            "format = uds-mock-fixture\nversion = 2\nfiles = a.bin\n\
             [entry e]\necu = E\nstep = session | S\n",
        )
        .unwrap();
        h.inspect(Source::File(path)).unwrap();
        let errs = h.prepare(Some("e"), &[], Duration::ZERO).unwrap_err();
        assert!(errs[0].contains("包版本 2"), "{errs:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn several_entries_need_an_explicit_choice() {
        let mut h = host();
        h.inspect(builtin(1)).unwrap();
        assert_eq!(h.inspection().unwrap().entries.len(), 3);
        let errs = h.prepare(None, &[], Duration::ZERO).unwrap_err();
        assert!(errs[0].contains("必须明确选择"), "{errs:?}");
        assert!(h.task().is_none());
        assert!(h.prepare(Some("nope"), &[], Duration::ZERO).is_err());
        h.prepare(Some("cal"), &[], Duration::ZERO).unwrap();
        assert_eq!(h.task().unwrap().plan().entry, "cal");
        // a single entry still has to be named
        let mut h = host();
        h.inspect(builtin(0)).unwrap();
        assert!(h.prepare(None, &[], Duration::ZERO).is_err());
    }

    #[test]
    fn cancel_in_a_plain_step_stops_at_the_boundary() {
        let mut h = ready(0, "app");
        h.start(Duration::ZERO).unwrap();
        let ms = Duration::from_millis;
        h.poll(ms(100));
        assert_eq!(h.request_cancel(ms(200)).unwrap(), CancelReply::Accepted);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Stopping);
        assert_eq!(t.cancel(), Some(CancelReply::Accepted));
        assert!(t
            .archive()
            .iter()
            .any(|r| r.text.contains("不表示 ECU 已停止") && r.text.contains("回滚")));
        // still stopping until the backend says how it ended
        assert!(h.poll(ms(300)));
        assert_eq!(h.request_cancel(ms(310)).unwrap(), CancelReply::Pending);
        run_to(&mut h, ms(300), STEP * 40);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Stopped);
        assert_eq!(t.progress().done, 1);
        assert!(t.result().unwrap().effects.is_empty());
        assert!(t.result().unwrap().unconfirmed.is_empty());
    }

    #[test]
    fn cancel_after_erase_lists_the_side_effect() {
        let mut h = ready(0, "app");
        h.start(Duration::ZERO).unwrap();
        // step 4 (erase) runs from 3 STEP to 4 STEP
        let at = STEP * 3 + STEP / 2;
        h.poll(at);
        h.request_cancel(at).unwrap();
        run_to(&mut h, at, STEP * 40);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Stopped);
        let r = t.result().unwrap();
        assert!(r.effects[0].contains("已擦除"), "{:?}", r.effects);
    }

    #[test]
    fn cancel_mid_download_leaves_the_result_unknown() {
        let mut h = ready(0, "app");
        h.start(Duration::ZERO).unwrap();
        // download runs from 4 STEP; stop after its first part
        let at = STEP * 4 + STEP / PARTS + Duration::from_millis(10);
        h.poll(at);
        assert_eq!(h.request_cancel(at).unwrap(), CancelReply::Accepted);
        assert_eq!(h.state(), Some(State::Stopping));
        run_to(&mut h, at, STEP * 40);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Unknown);
        let r = t.result().unwrap();
        assert!(!r.unconfirmed.is_empty());
        let (confirmed, total) = t.progress().bytes.unwrap();
        assert!(confirmed < total);
        assert!(r.effects.iter().any(|e| e.contains("不完整")));
        assert_eq!(
            states(&h).last().map(|s| s.contains("结果未知")),
            Some(true)
        );
    }

    #[test]
    fn a_failed_run_is_not_retried() {
        let mut h = ready(1, "cal");
        h.start(Duration::ZERO).unwrap();
        let end = run_to(&mut h, Duration::ZERO, STEP * 40);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Failed);
        let r = t.result().unwrap();
        assert!(r.error.as_ref().unwrap().contains("NRC 0x72"));
        // erase and download happened before the failing verify
        assert_eq!(r.effects.len(), 2);
        let starts = t
            .archive()
            .iter()
            .filter(|r| r.text.contains("后端开始运行"))
            .count();
        assert_eq!(starts, 1);
        // polling on, or asking to start again, changes nothing
        assert!(!h.poll(end + STEP * 10));
        assert!(h.start(end).unwrap_err().contains("不自动重试"));
        assert_eq!(h.state(), Some(State::Failed));
    }

    #[test]
    fn a_lost_backend_gives_an_unknown_result() {
        let mut h = ready(1, "lost");
        h.start(Duration::ZERO).unwrap();
        run_to(&mut h, Duration::ZERO, STEP * 40);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Unknown);
        assert!(t.result().unwrap().error.as_ref().unwrap().contains("失联"));
        // the first quarter was confirmed; the second was in flight
        assert_eq!(t.progress().bytes, Some((65_536, 262_144)));
        assert!(!t.result().unwrap().unconfirmed.is_empty());
    }

    #[test]
    fn the_plan_is_a_locked_snapshot() {
        let dir = scratch("snap");
        let path = dir.join("pkg.udsmock");
        std::fs::write(&path, FIXTURES[0].2).unwrap();
        let mut h = host();
        h.inspect(Source::File(path.clone())).unwrap();
        let params = vec![("p2".to_owned(), "120".to_owned())];
        h.prepare(Some("app"), &params, Duration::ZERO).unwrap();
        let plan = h.task().unwrap().plan().clone();
        assert!(plan.params.iter().any(|p| p.value == "120 ms"));
        // the package changes on disk after prepare: start refuses, the plan is gone
        std::fs::write(&path, FIXTURES[0].2.replace("262144", "131072")).unwrap();
        let err = h.start(Duration::ZERO).unwrap_err();
        assert!(err.contains("重新读取"), "{err}");
        assert!(h.task().is_none() && h.inspection().is_none());
        // a fresh import picks the change up
        h.inspect(Source::File(path.clone())).unwrap();
        h.prepare(Some("app"), &[], Duration::ZERO).unwrap();
        assert_ne!(h.task().unwrap().plan().asset_hash, plan.asset_hash);
        h.start(Duration::ZERO).unwrap();
        // while it runs, nothing new can be imported or prepared
        assert!(h.inspect(Source::File(path)).is_err());
        assert!(h.prepare(Some("app"), &[], Duration::ZERO).is_err());
        assert!(h.discard().is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imports_are_bounded_and_recognised() {
        let dir = scratch("big");
        let big = dir.join("big.udsmock");
        std::fs::write(&big, vec![b'#'; MAX_ASSET as usize + 1]).unwrap();
        let mut h = host();
        assert!(h.inspect(Source::File(big)).unwrap_err().contains("上限"));
        let odx = dir.join("x.odx");
        std::fs::write(&odx, "<ODX/>").unwrap();
        assert!(h
            .inspect(Source::File(odx))
            .unwrap_err()
            .contains("无法识别"));
        assert!(h.inspect(Source::File(dir.join("absent"))).is_err());
        assert!(h.inspection().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A backend that lies about itself, to check what the host refuses.
    struct Liar {
        inner: MockBackend,
        api: ApiVersion,
        mode: Mode,
        result_mode: Mode,
    }

    impl Backend for Liar {
        fn capabilities(&self) -> Capabilities {
            let mut c = self.inner.capabilities();
            c.api = self.api;
            c.mode = self.mode;
            c
        }
        fn inspect(&self, asset: &Asset) -> Result<Inspection, String> {
            self.inner.inspect(asset)
        }
        fn prepare(&mut self, req: &PrepareRequest) -> Result<RunPlan, Vec<String>> {
            self.inner.prepare(req)
        }
        fn start(&mut self, plan: &RunPlan, now: Duration) -> Result<RunId, String> {
            self.inner.start(plan, now)
        }
        fn poll(&mut self, run: RunId, now: Duration) -> Vec<BackendEvent> {
            self.inner.poll(run, now)
        }
        fn request_cancel(&mut self, run: RunId, now: Duration) -> CancelReply {
            self.inner.request_cancel(run, now)
        }
        fn result(&self, run: RunId) -> Option<RunResult> {
            self.inner.result(run).map(|mut r| {
                r.mode = self.result_mode;
                r
            })
        }
    }

    fn liar(api: (u16, u16), mode: Mode, result_mode: Mode) -> Result<Host, String> {
        Host::new(Box::new(Liar {
            inner: MockBackend::default(),
            api: ApiVersion {
                major: api.0,
                minor: api.1,
            },
            mode,
            result_mode,
        }))
    }

    #[test]
    fn host_refuses_other_api_versions_and_real_backends() {
        assert!(liar((1, 0), Mode::Simulated, Mode::Simulated)
            .err()
            .unwrap()
            .contains("接口版本"));
        assert!(liar((0, 1), Mode::Hardware, Mode::Hardware)
            .err()
            .unwrap()
            .contains("只接入模拟后端"));
    }

    #[test]
    fn a_result_claiming_hardware_is_not_taken_as_success() {
        let mut h = liar((0, 1), Mode::Simulated, Mode::Hardware).unwrap();
        h.inspect(builtin(0)).unwrap();
        h.prepare(Some("app"), &[], Duration::ZERO).unwrap();
        h.start(Duration::ZERO).unwrap();
        run_to(&mut h, Duration::ZERO, STEP * 40);
        let t = h.task().unwrap();
        assert_eq!(t.state(), State::Unknown);
        assert!(t
            .result()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("不采用"));
    }

    #[test]
    fn reimport_drops_a_ready_plan_but_keeps_a_finished_task() {
        let mut h = ready(0, "app");
        h.inspect(builtin(1)).unwrap();
        assert!(h.task().is_none());
        let mut h = ready(1, "cal");
        h.start(Duration::ZERO).unwrap();
        run_to(&mut h, Duration::ZERO, STEP * 40);
        h.inspect(builtin(0)).unwrap();
        assert_eq!(h.state(), Some(State::Failed));
        h.discard().unwrap();
        assert!(h.task().is_none());
    }
}
