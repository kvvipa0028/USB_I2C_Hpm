//! Versioned interface between the host task manager and an execution
//! backend (proposal §4.3): `capabilities`, `inspect`, `prepare`, `start`,
//! `request_cancel` and `result`, plus `poll`, which collects the events a
//! started run produced since the last call. A backend that runs in its
//! own process (§4.2) would implement this trait over its IPC; P1 has only
//! the in-process simulated backend in `mock`.
//!
//! Every method returns at once: `start` begins a run and later `poll`
//! calls collect what happened, so the window never waits on a target.

use std::fmt;
use std::time::Duration;

/// The interface version the host speaks.
pub const API_VERSION: ApiVersion = ApiVersion { major: 0, minor: 1 };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ApiVersion {
    pub major: u16,
    pub minor: u16,
}

impl ApiVersion {
    /// Whether a host speaking `self` can drive a backend speaking
    /// `backend`. Before 1.0 every minor version may break, so both must
    /// match; from 1.0 the major must match and the backend's minor may be
    /// newer (minor versions only add).
    pub fn accepts(self, backend: ApiVersion) -> bool {
        if self.major == 0 {
            self == backend
        } else {
            self.major == backend.major && backend.minor >= self.minor
        }
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// How a run reaches its target (report requirement §8.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Events come from a model; nothing reaches an ECU.
    Simulated,
    /// A backend that talks to a real target. Reserved for P2: nothing in
    /// P1 builds one, and the host refuses a backend that reports it.
    #[allow(dead_code)]
    Hardware,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Simulated => "模拟执行（非真实刷写）",
            Mode::Hardware => "真实硬件执行",
        }
    }
}

/// What a backend says it can do. Used for the window and the checks before
/// import; it is not evidence that any target ECU was tested.
#[derive(Clone, Debug)]
pub struct Capabilities {
    pub name: String,
    pub version: String,
    pub api: ApiVersion,
    pub mode: Mode,
    /// (format, versions) the backend can inspect and run.
    pub formats: Vec<(String, String)>,
    pub platforms: String,
    /// The bus or diagnostic interface the backend drives.
    pub link: String,
    /// Where a stop request is handled; `None` when stopping is not supported.
    pub cancel: Option<String>,
    /// How fine its events are (steps, confirmed bytes, raw frames).
    pub events: String,
}

/// A flow asset (§4.1) as the host read it: where from, its bytes, and their
/// hash. The host keeps it unchanged; backends only read it.
#[derive(Clone, Debug)]
pub struct Asset {
    pub label: String,
    pub bytes: Vec<u8>,
    pub hash: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IssueKind {
    /// A file, image, target or job the flow needs is not there.
    Missing,
    /// Something the backend cannot run: an extension, a script, a step
    /// kind, a package version, a field it does not know.
    Unsupported,
}

impl IssueKind {
    pub fn label(self) -> &'static str {
        match self {
            IssueKind::Missing => "缺少依赖",
            IssueKind::Unsupported => "不支持",
        }
    }
}

/// One finding of an offline check. Every issue blocks prepare.
#[derive(Clone, PartialEq, Debug)]
pub struct Issue {
    pub kind: IssueKind,
    pub text: String,
}

impl Issue {
    pub fn missing(text: impl Into<String>) -> Self {
        Self {
            kind: IssueKind::Missing,
            text: text.into(),
        }
    }

    pub fn unsupported(text: impl Into<String>) -> Self {
        Self {
            kind: IssueKind::Unsupported,
            text: text.into(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct StepDef {
    pub id: String,
    pub label: String,
    /// The step can change the target (erase, write, reset).
    pub effect: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Image {
    pub file: String,
    pub base: u32,
    pub len: u32,
}

/// A parameter the flow declares, with its unit, range, default, and where
/// the default comes from (§5.1). Read-only ones have `lo == hi == default`.
#[derive(Clone, PartialEq, Debug)]
pub struct Param {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub lo: u32,
    pub hi: u32,
    pub default: u32,
    /// Shown as given for read-only parameters (`0x7E0`).
    pub text: String,
    pub editable: bool,
    pub origin: String,
}

/// One entry of a package: an ECU and procedure the user can choose.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub ecu: String,
    pub steps: Vec<StepDef>,
    pub images: Vec<Image>,
    pub params: Vec<Param>,
    pub issues: Vec<Issue>,
}

/// What `inspect` found, read offline.
#[derive(Clone, PartialEq, Debug)]
pub struct Inspection {
    pub format: String,
    pub version: String,
    pub package: String,
    /// Issues of the whole package; they block every entry.
    pub issues: Vec<Issue>,
    pub entries: Vec<Entry>,
    /// What the check saw and left alone (a script it did not run).
    pub notes: Vec<String>,
}

impl Inspection {
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// What keeps `entry` from being prepared: the package's issues, then
    /// the entry's own.
    pub fn blockers<'a>(&'a self, entry: &'a Entry) -> impl Iterator<Item = &'a Issue> {
        self.issues.iter().chain(&entry.issues)
    }
}

/// `prepare` input: the asset, the chosen entry, and the editable
/// parameters as the user typed them.
pub struct PrepareRequest<'a> {
    pub asset: &'a Asset,
    pub entry: &'a str,
    pub params: &'a [(String, String)],
}

/// A parameter as bound into a plan.
#[derive(Clone, PartialEq, Debug)]
pub struct Bound {
    pub id: String,
    pub label: String,
    /// With its unit, as it will be reported.
    pub value: String,
    pub editable: bool,
}

/// The immutable plan `prepare` returns: everything one run will use. The
/// host stores it as the run snapshot and hands out only `&RunPlan`.
#[derive(Clone, PartialEq, Debug)]
pub struct RunPlan {
    pub plan_id: u64,
    /// Name and version of the backend that made it.
    pub backend: String,
    pub api: ApiVersion,
    pub mode: Mode,
    pub asset: String,
    pub asset_hash: u64,
    pub package: String,
    pub entry: String,
    pub title: String,
    pub ecu: String,
    pub steps: Vec<StepDef>,
    pub images: Vec<Image>,
    pub params: Vec<Bound>,
    /// What the backend wants the report to carry (fault injection, limits).
    pub notes: Vec<String>,
}

impl RunPlan {
    /// Bytes the plan's images add up to.
    pub fn image_bytes(&self) -> u64 {
        self.images.iter().map(|i| u64::from(i.len)).sum()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RunId(pub u64);

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R{:04}", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Info => "信息",
            Level::Warn => "警告",
            Level::Error => "错误",
        }
    }
}

/// What an event tells the host besides its text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventKind {
    Log,
    StepStarted(usize),
    StepDone(usize),
    /// Download bytes the target confirmed, apart from step progress.
    Bytes {
        confirmed: u64,
        total: u64,
    },
}

/// One event of a run, in the order the backend produced it.
#[derive(Clone, PartialEq, Debug)]
pub struct BackendEvent {
    pub at: Duration,
    /// The step it belongs to, `3:erase` style.
    pub step: Option<String>,
    pub level: Level,
    pub text: String,
    pub kind: EventKind,
}

/// Answer to `request_cancel`. Accepted means the backend has the request;
/// it does not mean the ECU stopped, and never that anything was undone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CancelReply {
    Accepted,
    Unsupported,
    /// A stop request is already being handled.
    Pending,
    /// The run had already ended.
    NotRunning,
}

impl CancelReply {
    pub fn label(self) -> &'static str {
        match self {
            CancelReply::Accepted => "停止请求已接收",
            CancelReply::Unsupported => "后端不支持停止",
            CancelReply::Pending => "停止请求仍在处理",
            CancelReply::NotRunning => "运行已结束",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Succeeded,
    Failed,
    Stopped,
    /// Whether a request with side effects completed cannot be confirmed.
    Unknown,
}

/// Final answer for a run: how it ended, the error, the evidence, the side
/// effects known to have happened and the requests still unconfirmed.
#[derive(Clone, PartialEq, Debug)]
pub struct RunResult {
    pub outcome: Outcome,
    pub mode: Mode,
    pub error: Option<String>,
    pub effects: Vec<String>,
    pub unconfirmed: Vec<String>,
    pub evidence: Vec<String>,
}

/// An execution backend. See the module doc for the call order.
pub trait Backend {
    fn capabilities(&self) -> Capabilities;
    /// Offline only: read the asset, never open a bus, run a script, or
    /// send a request.
    fn inspect(&self, asset: &Asset) -> Result<Inspection, String>;
    /// Bind entry and parameters into a plan, or say why not. Never erases,
    /// downloads, or resets.
    fn prepare(&mut self, req: &PrepareRequest) -> Result<RunPlan, Vec<String>>;
    /// Start a run of `plan`; a plan starts at most once.
    fn start(&mut self, plan: &RunPlan, now: Duration) -> Result<RunId, String>;
    /// Events since the last call.
    fn poll(&mut self, run: RunId, now: Duration) -> Vec<BackendEvent>;
    fn request_cancel(&mut self, run: RunId, now: Duration) -> CancelReply;
    /// `Some` once the run has ended.
    fn result(&self, run: RunId) -> Option<RunResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_versions_match_before_one_and_add_after() {
        let v = |major, minor| ApiVersion { major, minor };
        assert!(v(0, 1).accepts(v(0, 1)));
        assert!(!v(0, 1).accepts(v(0, 2)));
        assert!(!v(0, 1).accepts(v(1, 1)));
        assert!(v(1, 2).accepts(v(1, 3)));
        assert!(!v(1, 2).accepts(v(1, 1)));
        assert!(!v(1, 2).accepts(v(2, 2)));
        assert_eq!(API_VERSION.to_string(), "0.1");
        assert_eq!(RunId(7).to_string(), "R0007");
    }
}
