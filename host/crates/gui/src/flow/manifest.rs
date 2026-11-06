//! `uds-flow-manifest` version 1, the declarative JSON flow package of the
//! manifest design (`uds-flow-manifest-v1-方案.md` §3 and §5, decisions
//! T1–T8 of its §10): the parse layer only. [`parse`] reads a bare manifest
//! (`*.udsflow.json`, no ZIP) and checks it against the field tables. It
//! returns a [`Manifest`] whose issues use the backend API's [`Issue`]
//! kinds, so a backend's `inspect` can map it onto an `Inspection`.
//!
//! Like `.udsmock`, this is a demo format made by the project. It is not
//! ODX, PDX, OTX or a vendor format, and a run from it is still a simulated
//! run. The check reads the manifest text and nothing else. Listed files
//! are only names and are never opened; a `sha256` is checked for form,
//! not against content. A `script`, `hooks` or `url` key is reported and
//! never run or fetched. `MockBackend` does not read this format yet.
//!
//! An error means the input cannot be read as a manifest: it is over the
//! size limit, it is not JSON, it has no `"format": "uds-flow-manifest"`,
//! or a value in it has the wrong JSON type. Everything else becomes an
//! issue: a missing field, an unknown key, a step kind outside the
//! whitelist. As in `api`, every issue blocks prepare.

// Only the tests call it until MockBackend reads manifests.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use serde_json::Value;

use super::api::{Issue, Mode, Param, StepDef};
use super::host::MAX_ASSET;
use super::mock::KINDS;

pub const FORMAT: &str = "uds-flow-manifest";
pub const VERSION: u64 = 1;
/// How the name of a bare manifest ends (T1).
pub const EXTENSION: &str = ".udsflow.json";

/// Keys an object had beyond its own fields. None is skipped (T7).
type Extra = BTreeMap<String, Value>;

#[derive(Deserialize)]
struct RawManifest {
    format: Option<String>,
    version: Option<Value>,
    package: Option<String>,
    #[serde(default)]
    notes: Vec<String>,
    files: Option<Vec<RawFile>>,
    entries: Option<Vec<RawEntry>>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize)]
struct RawFile {
    path: Option<String>,
    sha256: Option<String>,
    role: Option<String>,
    media: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize)]
struct RawEntry {
    id: Option<String>,
    title: Option<String>,
    ecu: Option<String>,
    #[serde(default)]
    images: Vec<RawImage>,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    params: Vec<RawParam>,
    #[serde(default)]
    steps: Vec<RawStep>,
    simulate: Option<RawSimulate>,
    #[serde(default)]
    requires: Vec<Value>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize)]
struct RawImage {
    file: Option<String>,
    base: Option<String>,
    len: Option<u64>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize)]
struct RawParam {
    id: Option<String>,
    label: Option<String>,
    unit: Option<String>,
    editable: Option<bool>,
    value: Option<String>,
    lo: Option<u64>,
    hi: Option<u64>,
    default: Option<u64>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize)]
struct RawStep {
    kind: Option<String>,
    label: Option<String>,
    effect: Option<bool>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize)]
struct RawSimulate {
    #[serde(rename = "failAt")]
    fail_at: Option<String>,
    #[serde(rename = "lostAt")]
    lost_at: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Image,
    Driver,
    Other,
}

impl Role {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "image" => Some(Role::Image),
            "driver" => Some(Role::Driver),
            "other" => Some(Role::Other),
            _ => None,
        }
    }
}

/// How a file holds its data. Only HEX and S19 carry their own addresses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Media {
    Bin,
    Hex,
    S19,
    Unknown,
}

impl Media {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "bin" => Some(Media::Bin),
            "hex" => Some(Media::Hex),
            "s19" => Some(Media::S19),
            "unknown" => Some(Media::Unknown),
            _ => None,
        }
    }
}

/// One `files[]` item. Its content is never read.
#[derive(Clone, PartialEq, Debug)]
pub struct FileRef {
    pub path: String,
    /// Lowercase hex, checked for form only.
    pub sha256: Option<String>,
    pub role: Option<Role>,
    pub media: Option<Media>,
}

/// One `images[]` item. `base` and `len` can be absent only for HEX / S19.
#[derive(Clone, PartialEq, Debug)]
pub struct ImageRef {
    pub file: String,
    pub base: Option<u32>,
    pub len: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaultKind {
    /// `failAt`: the step ends with a negative response.
    Fail,
    /// `lostAt`: the backend goes silent halfway through the step.
    Lost,
}

/// `simulate`: a fault the entry asks the simulated backend to play.
#[derive(Clone, PartialEq, Debug)]
pub struct Fault {
    pub kind: FaultKind,
    /// The step kind it hits.
    pub step: String,
}

/// One entry, checked. Invalid steps, images and parameters are left out
/// and named in `issues`; unknown step kinds stay in, to be shown.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub ecu: String,
    pub images: Vec<ImageRef>,
    pub needs: Vec<String>,
    pub params: Vec<Param>,
    pub steps: Vec<StepDef>,
    pub fault: Option<Fault>,
    pub issues: Vec<Issue>,
}

/// A manifest as [`parse`] read and checked it.
#[derive(Clone, PartialEq, Debug)]
pub struct Manifest {
    /// As written, `"?"` when absent.
    pub version: String,
    /// `package`, or the label the caller gave.
    pub package: String,
    /// The package's own `notes`, for display only.
    pub notes: Vec<String>,
    pub files: Vec<FileRef>,
    pub entries: Vec<Entry>,
    /// Issues of the whole package; they block every entry.
    pub issues: Vec<Issue>,
    /// What the check saw and left alone (a script it did not run).
    pub check_notes: Vec<String>,
    /// Some entry has `simulate`, so only a simulated backend may take the
    /// package (T8). For any other mode, [`parse`] has already put an
    /// Unsupported issue in `issues`.
    pub simulated_only: bool,
}

impl Manifest {
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// What keeps `entry` from being prepared: the package's issues, then
    /// the entry's own.
    pub fn blockers<'a>(&'a self, entry: &'a Entry) -> impl Iterator<Item = &'a Issue> {
        self.issues.iter().chain(&entry.issues)
    }
}

/// Why the input cannot be read as a manifest at all.
#[derive(Clone, PartialEq, Debug)]
pub enum ParseError {
    TooLarge(usize),
    /// Not JSON: a syntax error, bad UTF-8, or cut short.
    Malformed(String),
    /// JSON without `"format": "uds-flow-manifest"`.
    NotRecognised,
    /// It says it is a manifest, but a value has the wrong JSON type.
    Shape(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::TooLarge(n) => write!(f, "流程清单有 {n} B，超过导入上限 {MAX_ASSET} B"),
            ParseError::Malformed(e) => write!(f, "不是有效的 JSON：{e}"),
            ParseError::NotRecognised => write!(
                f,
                "无法识别：缺少「\"format\": \"{FORMAT}\"」，不是流程清单"
            ),
            ParseError::Shape(e) => write!(f, "流程清单字段类型错误：{e}"),
        }
    }
}

/// Read and check a bare manifest. `label` names the package when it has
/// no `package`; `mode` is that of the backend asking, and anything but
/// [`Mode::Simulated`] refuses `simulate` (T8).
pub fn parse(bytes: &[u8], label: &str, mode: Mode) -> Result<Manifest, ParseError> {
    if bytes.len() as u64 > MAX_ASSET {
        return Err(ParseError::TooLarge(bytes.len()));
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let raw: RawManifest = match serde_json::from_slice(bytes) {
        Ok(raw) => raw,
        // Well-formed JSON of the wrong shape: a type error if it says it
        // is a manifest, else just something else.
        Err(e) if e.is_data() => {
            let v: Value =
                serde_json::from_slice(bytes).map_err(|e| ParseError::Malformed(e.to_string()))?;
            return Err(if v.get("format").and_then(Value::as_str) == Some(FORMAT) {
                ParseError::Shape(e.to_string())
            } else {
                ParseError::NotRecognised
            });
        }
        Err(e) => return Err(ParseError::Malformed(e.to_string())),
    };
    if raw.format.as_deref() != Some(FORMAT) {
        return Err(ParseError::NotRecognised);
    }
    let check = Check {
        mode,
        issues: Vec::new(),
        notes: Vec::new(),
        simulated_only: false,
    };
    Ok(check.manifest(raw, label))
}

/// `v` as an issue shows it: a string bare, anything else as JSON, cut
/// after 60 characters.
fn brief(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    match s.char_indices().nth(60) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

/// Report every key in `extra`, found in the object at `at`. Keys that
/// would run or fetch something say so, and a script is noted as seen and
/// not run.
fn keys(at: &str, extra: &Extra, issues: &mut Vec<Issue>, notes: &mut Vec<String>) {
    for (key, value) in extra {
        let key_at = if at.is_empty() {
            key.clone()
        } else {
            format!("{at}.{key}")
        };
        let v = brief(value);
        let why = match key.as_str() {
            "script" | "scripts" | "hooks" => {
                notes.push(format!("{key_at} 引用 {v}：离线检查只记录，未执行"));
                format!("外部脚本 {v}，模拟后端不执行")
            }
            "uri" | "url" => format!("远程地址 {v}，不拉取"),
            "frames" => "自由报文列表不受支持".to_owned(),
            "eval" => "内嵌代码，不执行".to_owned(),
            _ => "未知字段，不跳过".to_owned(),
        };
        issues.push(Issue::unsupported(format!("{key_at}：{why}")));
    }
}

/// Why `p` is not a plain relative path inside the package, if it is not.
fn path_problem(p: &str) -> Option<&'static str> {
    let lower = p.to_ascii_lowercase();
    if p.is_empty() {
        Some("为空")
    } else if p.contains("://") || lower.starts_with("http:") || lower.starts_with("https:") {
        Some("是远程地址，不拉取")
    } else if p.contains('\\') {
        Some("含反斜杠")
    } else if p.contains(':') {
        Some("含盘符或冒号")
    } else if p.starts_with('/') {
        Some("是绝对路径")
    } else if p.split('/').any(|c| c == "..") {
        Some("含 ..，会逃出包根")
    } else if p.split('/').any(|c| c.is_empty() || c == ".") || p.chars().any(char::is_control) {
        Some("不是规范的相对路径")
    } else {
        None
    }
}

fn nonempty(v: Option<String>) -> Option<String> {
    v.filter(|v| !v.trim().is_empty())
}

fn is_sha256(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `0x` and up to 32 bits of hex. A bare number is refused: it could be
/// read as decimal or hex.
fn hex32(s: &str) -> Option<u32> {
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if h.is_empty() || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

/// The checks, with what they found about the package as a whole.
struct Check {
    mode: Mode,
    issues: Vec<Issue>,
    notes: Vec<String>,
    simulated_only: bool,
}

impl Check {
    fn manifest(mut self, raw: RawManifest, label: &str) -> Manifest {
        keys("", &raw.extra, &mut self.issues, &mut self.notes);
        let version = match &raw.version {
            Some(v) => {
                if v.as_u64() != Some(VERSION) {
                    self.issues.push(Issue::unsupported(format!(
                        "包版本 {v} 不受支持（只支持 {VERSION}）"
                    )));
                }
                v.to_string()
            }
            None => {
                self.issues.push(Issue::missing("缺少 version"));
                "?".into()
            }
        };
        let files = self.files(raw.files);
        let raw_entries = raw.entries.unwrap_or_default();
        if raw_entries.is_empty() {
            self.issues.push(Issue::missing("包内没有入口"));
        }
        let entries: Vec<Entry> = raw_entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| self.entry(i, e, &files))
            .collect();
        for (i, e) in entries.iter().enumerate() {
            if !e.id.is_empty() && entries[..i].iter().any(|d| d.id == e.id) {
                self.issues.push(Issue::unsupported(format!(
                    "entries[{i}]：入口 {} 重复",
                    e.id
                )));
            }
        }
        Manifest {
            version,
            package: nonempty(raw.package).unwrap_or_else(|| label.to_owned()),
            notes: raw.notes,
            files,
            entries,
            issues: self.issues,
            check_notes: self.notes,
            simulated_only: self.simulated_only,
        }
    }

    fn files(&mut self, raw: Option<Vec<RawFile>>) -> Vec<FileRef> {
        let Some(raw) = raw else {
            self.issues.push(Issue::missing("缺少 files 清单"));
            return Vec::new();
        };
        let mut files: Vec<FileRef> = Vec::new();
        for (i, f) in raw.into_iter().enumerate() {
            let at = format!("files[{i}]");
            keys(&at, &f.extra, &mut self.issues, &mut self.notes);
            let bad = |what: String| Issue::unsupported(format!("{at}：{what}"));
            let Some(path) = f.path else {
                self.issues.push(Issue::missing(format!("{at}：缺少 path")));
                continue;
            };
            if let Some(why) = path_problem(&path) {
                self.issues.push(bad(format!("路径 {path} {why}")));
                continue;
            }
            if files.iter().any(|g| g.path == path) {
                self.issues.push(bad(format!("路径 {path} 重复")));
                continue;
            }
            let sha256 = f.sha256.filter(|h| {
                let ok = is_sha256(h);
                if !ok {
                    self.issues
                        .push(bad("sha256 应为 64 位小写十六进制".into()));
                }
                ok
            });
            let role = f.role.as_deref().and_then(|r| {
                let role = Role::parse(r);
                if role.is_none() {
                    self.issues.push(bad(format!(
                        "role {r} 不受支持（只认 image / driver / other）"
                    )));
                }
                role
            });
            let media = f.media.as_deref().and_then(|m| {
                let media = Media::parse(m);
                if media.is_none() {
                    self.issues.push(bad(format!(
                        "media {m} 不受支持（只认 bin / hex / s19 / unknown）"
                    )));
                }
                media
            });
            files.push(FileRef {
                path,
                sha256,
                role,
                media,
            });
        }
        if files.iter().any(|f| f.sha256.is_some()) {
            self.notes
                .push("sha256 只检查了格式，未与文件内容核对（不读取镜像）".into());
        }
        files
    }

    fn entry(&mut self, i: usize, raw: RawEntry, files: &[FileRef]) -> Entry {
        let at = format!("entries[{i}]");
        let mut issues = Vec::new();
        keys(&at, &raw.extra, &mut issues, &mut self.notes);
        let id = raw.id.unwrap_or_default();
        if id.trim().is_empty() {
            self.issues
                .push(Issue::missing(format!("{at}：入口缺少 id")));
        }
        let ecu = raw.ecu.unwrap_or_default();
        if ecu.trim().is_empty() {
            issues.push(Issue::missing(format!("{at}：未声明目标 ECU")));
        }
        for (k, r) in raw.requires.iter().enumerate() {
            issues.push(Issue::unsupported(format!(
                "{at}.requires[{k}]：需要 {}，模拟后端不提供",
                brief(r)
            )));
        }
        let steps = self.steps(&at, raw.steps, &mut issues);
        let images = self.images(&at, raw.images, files, &mut issues);
        let params = self.params(&at, raw.params, &mut issues);
        for (k, n) in raw.needs.iter().enumerate() {
            if let Some(why) = path_problem(n) {
                issues.push(Issue::unsupported(format!(
                    "{at}.needs[{k}]：路径 {n} {why}"
                )));
            } else if !files.iter().any(|f| f.path == *n) {
                issues.push(Issue::missing(format!(
                    "{at}.needs[{k}]：依赖 {n} 不在包清单（files）中"
                )));
            }
        }
        if steps.is_empty() {
            issues.push(Issue::missing(format!(
                "{at}：没有可执行步骤（缺少 Flash Job）"
            )));
        }
        if steps.iter().any(|s| s.id == "download") && images.is_empty() {
            issues.push(Issue::missing(format!("{at}：下载步骤没有镜像")));
        }
        let fault = raw
            .simulate
            .and_then(|s| self.simulate(&at, s, &steps, &mut issues));
        Entry {
            title: nonempty(raw.title).unwrap_or_else(|| id.clone()),
            id,
            ecu,
            images,
            needs: raw.needs,
            params,
            steps,
            fault,
            issues,
        }
    }

    fn steps(&mut self, at: &str, raw: Vec<RawStep>, issues: &mut Vec<Issue>) -> Vec<StepDef> {
        let mut steps = Vec::new();
        for (k, s) in raw.into_iter().enumerate() {
            let at = format!("{at}.steps[{k}]");
            keys(&at, &s.extra, issues, &mut self.notes);
            let (Some(kind), Some(label)) = (nonempty(s.kind), nonempty(s.label)) else {
                issues.push(Issue::missing(format!("{at}：步骤需要 kind 和 label")));
                continue;
            };
            let effect = match (KINDS.iter().find(|k| k.0 == kind), s.effect) {
                (None, effect) => {
                    issues.push(Issue::unsupported(format!(
                        "{at}：步骤类型 {kind} 不受模拟后端支持"
                    )));
                    effect.unwrap_or(false)
                }
                // A package may not declare a step that changes the target
                // as one that does not: the host's no-retry rule rests on it.
                (Some(&(_, true)), Some(false)) => {
                    issues.push(Issue::unsupported(format!(
                        "{at}：{kind} 会改变目标，effect 不能为 false"
                    )));
                    true
                }
                (Some(&(_, default)), effect) => effect.unwrap_or(default),
            };
            steps.push(StepDef {
                id: kind,
                label,
                effect,
            });
        }
        steps
    }

    fn images(
        &mut self,
        at: &str,
        raw: Vec<RawImage>,
        files: &[FileRef],
        issues: &mut Vec<Issue>,
    ) -> Vec<ImageRef> {
        let mut images = Vec::new();
        for (k, im) in raw.into_iter().enumerate() {
            let at = format!("{at}.images[{k}]");
            keys(&at, &im.extra, issues, &mut self.notes);
            let bad = |what: String| Issue::unsupported(format!("{at}：{what}"));
            let Some(file) = im.file else {
                issues.push(Issue::missing(format!("{at}：缺少 file")));
                continue;
            };
            if let Some(why) = path_problem(&file) {
                issues.push(bad(format!("路径 {file} {why}")));
                continue;
            }
            let media = match files.iter().find(|f| f.path == file) {
                Some(f) => f.media,
                None => {
                    issues.push(Issue::missing(format!(
                        "{at}：镜像 {file} 不在包清单（files）中"
                    )));
                    None
                }
            };
            let base = match im.base.as_deref() {
                None => None,
                Some(s) => match hex32(s) {
                    Some(b) => Some(b),
                    None => {
                        issues.push(bad(format!("base {s} 应为 0x 开头的 32 位十六进制")));
                        continue;
                    }
                },
            };
            let len = match im.len.map(u32::try_from) {
                None => None,
                Some(Ok(n)) if n > 0 => Some(n),
                Some(_) => {
                    issues.push(bad(format!("镜像 {file} 长度为 0 或超出 32 位")));
                    continue;
                }
            };
            // Without its own addresses a file needs both, and nothing is
            // made up (§3.4).
            let segmented = matches!(media, Some(Media::Hex | Media::S19));
            if !segmented && (base.is_none() || len.is_none()) {
                issues.push(bad(format!(
                    "镜像 {file} 不是 hex / s19，必须给出 base 和 len"
                )));
                continue;
            }
            if let (Some(b), Some(n)) = (base, len) {
                if u64::from(b) + u64::from(n) > 1 << 32 {
                    issues.push(bad(format!("镜像 {file} 地址越界")));
                    continue;
                }
            }
            images.push(ImageRef { file, base, len });
        }
        images
    }

    fn params(&mut self, at: &str, raw: Vec<RawParam>, issues: &mut Vec<Issue>) -> Vec<Param> {
        let mut params: Vec<Param> = Vec::new();
        for (k, p) in raw.into_iter().enumerate() {
            let at = format!("{at}.params[{k}]");
            keys(&at, &p.extra, issues, &mut self.notes);
            let (Some(id), Some(label), Some(editable)) =
                (nonempty(p.id), nonempty(p.label), p.editable)
            else {
                issues.push(Issue::missing(format!(
                    "{at}：参数需要 id、label 和 editable"
                )));
                continue;
            };
            if params.iter().any(|q| q.id == id) {
                issues.push(Issue::unsupported(format!("{at}：参数 {id} 重复")));
                continue;
            }
            let unit = p.unit.unwrap_or_default();
            let n = |v: Option<u64>| v.and_then(|v| u32::try_from(v).ok());
            let param = match (editable, p.value, n(p.lo), n(p.hi), n(p.default)) {
                (true, None, Some(lo), Some(hi), Some(d)) if (lo..=hi).contains(&d) => Param {
                    id,
                    label,
                    unit,
                    lo,
                    hi,
                    default: d,
                    text: d.to_string(),
                    editable,
                    origin: "流程文件".into(),
                },
                (true, Some(_), ..) => {
                    issues.push(Issue::unsupported(format!(
                        "{at}：可修改参数 {id} 用 lo / hi / default，不用 value"
                    )));
                    continue;
                }
                (true, ..) => {
                    issues.push(Issue::unsupported(format!(
                        "{at}：参数 {id} 的范围或默认值无效"
                    )));
                    continue;
                }
                (false, Some(text), ..)
                    if p.lo.is_none() && p.hi.is_none() && p.default.is_none() =>
                {
                    Param {
                        id,
                        label,
                        unit,
                        lo: 0,
                        hi: 0,
                        default: 0,
                        text,
                        editable,
                        origin: "流程文件（只读）".into(),
                    }
                }
                (false, None, ..) => {
                    issues.push(Issue::missing(format!("{at}：只读参数 {id} 缺少 value")));
                    continue;
                }
                (false, ..) => {
                    issues.push(Issue::unsupported(format!(
                        "{at}：只读参数 {id} 不带 lo / hi / default"
                    )));
                    continue;
                }
            };
            params.push(param);
        }
        params
    }

    fn simulate(
        &mut self,
        at: &str,
        raw: RawSimulate,
        steps: &[StepDef],
        issues: &mut Vec<Issue>,
    ) -> Option<Fault> {
        let at = format!("{at}.simulate");
        self.simulated_only = true;
        if self.mode != Mode::Simulated {
            self.issues.push(Issue::unsupported(format!(
                "{at}：故障注入只供模拟后端；{}不接受含 simulate 的包",
                self.mode.label()
            )));
        }
        keys(&at, &raw.extra, issues, &mut self.notes);
        let fault = match (raw.fail_at, raw.lost_at) {
            (Some(step), None) => Fault {
                kind: FaultKind::Fail,
                step,
            },
            (None, Some(step)) => Fault {
                kind: FaultKind::Lost,
                step,
            },
            _ => {
                issues.push(Issue::unsupported(format!(
                    "{at}：只能给出 failAt 或 lostAt 其中一个"
                )));
                return None;
            }
        };
        if !steps.iter().any(|s| s.id == fault.step) {
            issues.push(Issue::unsupported(format!(
                "{at}：指向的步骤 {} 不在入口中",
                fault.step
            )));
        }
        Some(fault)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::flow::api::{Asset, Backend, IssueKind};
    use crate::flow::mock::{self, MockBackend};

    const SINGLE: &str = include_str!("../../fixtures/uds/single.udsflow.json");
    const MULTI: &str = include_str!("../../fixtures/uds/multi.udsflow.json");
    const BLOCKED: &str = include_str!("../../fixtures/uds/blocked.udsflow.json");

    fn read(text: &str, mode: Mode) -> Manifest {
        parse(text.as_bytes(), "t.udsflow.json", mode).unwrap()
    }

    fn check(v: &Value) -> Manifest {
        read(&v.to_string(), Mode::Simulated)
    }

    fn single() -> Value {
        serde_json::from_str(SINGLE).unwrap()
    }

    /// Every issue, package first, as (kind, text).
    fn all(m: &Manifest) -> Vec<(IssueKind, &str)> {
        m.issues
            .iter()
            .chain(m.entries.iter().flat_map(|e| &e.issues))
            .map(|i| (i.kind, i.text.as_str()))
            .collect()
    }

    fn has(m: &Manifest, kind: IssueKind, needle: &str) -> bool {
        all(m).iter().any(|(k, t)| *k == kind && t.contains(needle))
    }

    #[test]
    fn fixtures_parse_as_documented() {
        let s = read(SINGLE, Mode::Simulated);
        assert!(all(&s).is_empty(), "{:?}", all(&s));
        assert_eq!(
            (s.version.as_str(), s.package.as_str()),
            ("1", "演示包 · 单入口")
        );
        assert!(s.notes[0].contains("仅模拟"));
        assert!(s.check_notes.is_empty() && !s.simulated_only);
        assert_eq!(s.files.len(), 2);
        assert_eq!(
            (
                s.files[1].role,
                s.files[1].media,
                s.files[1].sha256.as_deref()
            ),
            (Some(Role::Driver), Some(Media::Bin), None)
        );
        let e = s.entry("app").unwrap();
        assert_eq!(e.ecu, "DEMO-ECU-A · 演示 Bootloader");
        assert_eq!(e.needs, ["flash_driver.bin"]);
        assert_eq!(
            e.images,
            [ImageRef {
                file: "app.bin".into(),
                base: Some(0x0001_0000),
                len: Some(262_144),
            }]
        );
        let effects: Vec<(&str, bool)> =
            e.steps.iter().map(|s| (s.id.as_str(), s.effect)).collect();
        assert_eq!(effects, KINDS);
        assert_eq!(e.params.len(), 4);
        assert_eq!(
            (e.params[0].text.as_str(), e.params[0].editable),
            ("0x7E0", false)
        );
        let p2 = &e.params[2];
        assert_eq!(
            (p2.lo, p2.hi, p2.default, p2.unit.as_str()),
            (1, 5000, 50, "ms")
        );
        assert_eq!(e.fault, None);

        let m = read(MULTI, Mode::Simulated);
        assert!(all(&m).is_empty(), "{:?}", all(&m));
        assert!(m.simulated_only);
        let faults: Vec<_> = m.entries.iter().map(|e| e.fault.clone()).collect();
        let fault = |kind, step: &str| {
            Some(Fault {
                kind,
                step: step.into(),
            })
        };
        assert_eq!(
            faults,
            [
                None,
                fault(FaultKind::Fail, "verify"),
                fault(FaultKind::Lost, "download")
            ]
        );

        let b = read(BLOCKED, Mode::Simulated);
        assert!(b.issues.is_empty());
        let e = &b.entries[0];
        assert!(b.blockers(e).next().is_some());
        assert!(has(&b, IssueKind::Missing, "依赖 flash_driver.bin"));
        assert!(has(&b, IssueKind::Unsupported, "requires[0]"));
        assert!(has(&b, IssueKind::Unsupported, "entries[0].script"));
        assert!(has(&b, IssueKind::Unsupported, "checksum_vendor"));
        assert!(b.check_notes.iter().any(|n| n.contains("未执行")));
    }

    /// Each JSON fixture reads like the `.udsmock` it was migrated from.
    #[test]
    fn fixtures_match_their_udsmock_twins() {
        let backend = MockBackend::default();
        for ((_, name, text), json) in mock::FIXTURES.iter().zip([SINGLE, MULTI, BLOCKED]) {
            let old = backend
                .inspect(&Asset {
                    label: (*name).into(),
                    bytes: text.as_bytes().to_vec(),
                    hash: 0,
                })
                .unwrap();
            let new = read(json, Mode::Simulated);
            assert_eq!(new.package, old.package, "{name}");
            assert_eq!(new.entries.len(), old.entries.len(), "{name}");
            for (n, o) in new.entries.iter().zip(&old.entries) {
                assert_eq!((&n.id, &n.title, &n.ecu), (&o.id, &o.title, &o.ecu));
                assert_eq!(n.steps, o.steps, "{name}");
                assert_eq!(n.params, o.params, "{name}");
                let images: Vec<_> = o
                    .images
                    .iter()
                    .map(|i| ImageRef {
                        file: i.file.clone(),
                        base: Some(i.base),
                        len: Some(i.len),
                    })
                    .collect();
                assert_eq!(n.images, images, "{name}");
                for kind in [IssueKind::Missing, IssueKind::Unsupported] {
                    assert_eq!(
                        n.issues.iter().any(|i| i.kind == kind),
                        o.issues.iter().any(|i| i.kind == kind),
                        "{name} {kind:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_fixture_file_is_a_manifest() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/uds");
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(EXTENSION) {
                let bytes = std::fs::read(&path).unwrap();
                assert!(parse(&bytes, &name, Mode::Simulated).is_ok(), "{name}");
                seen += 1;
            }
        }
        assert_eq!(seen, 3);
    }

    #[test]
    fn wrong_format_or_version_is_refused() {
        let p = |s: &str| parse(s.as_bytes(), "x", Mode::Simulated);
        assert_eq!(
            p(r#"{"format": "uds-mock-fixture", "version": 1}"#),
            Err(ParseError::NotRecognised)
        );
        assert_eq!(p(r#"{"version": 1}"#), Err(ParseError::NotRecognised));
        assert_eq!(p("[1, 2]"), Err(ParseError::NotRecognised));
        assert_eq!(p(r#"{"format": 7}"#), Err(ParseError::NotRecognised));
        assert!(matches!(
            p("format = uds-flow-manifest"),
            Err(ParseError::Malformed(_))
        ));
        let cut = SINGLE.find("\"entries\"").unwrap();
        assert!(matches!(p(&SINGLE[..cut]), Err(ParseError::Malformed(_))));
        assert!(matches!(
            parse(&[0xff, 0xfe, 0], "x", Mode::Simulated),
            Err(ParseError::Malformed(_))
        ));
        // the right format with a value of the wrong type
        let mut v = single();
        v["entries"][0]["images"][0]["len"] = json!("262144");
        let err = p(&v.to_string()).unwrap_err();
        assert!(matches!(err, ParseError::Shape(_)), "{err:?}");
        assert!(err.to_string().contains("类型错误"));
        // version: anything but the integer 1 blocks the package
        for (version, want) in [
            (json!(2), (IssueKind::Unsupported, "包版本 2")),
            (json!("1"), (IssueKind::Unsupported, "包版本 \"1\"")),
            (json!(1.0), (IssueKind::Unsupported, "包版本 1.0")),
        ] {
            let mut v = single();
            v["version"] = version;
            let m = check(&v);
            assert!(has(&m, want.0, want.1), "{:?}", all(&m));
            assert!(m.blockers(&m.entries[0]).next().is_some());
        }
        let mut v = single();
        v.as_object_mut().unwrap().remove("version");
        let m = check(&v);
        assert!(has(&m, IssueKind::Missing, "缺少 version"));
        assert_eq!(m.version, "?");
    }

    #[test]
    fn size_limit_and_bom() {
        let mut big = SINGLE.as_bytes().to_vec();
        big.resize(MAX_ASSET as usize + 1, b' ');
        assert_eq!(
            parse(&big, "x", Mode::Simulated),
            Err(ParseError::TooLarge(big.len()))
        );
        big.truncate(MAX_ASSET as usize);
        assert!(parse(&big, "x", Mode::Simulated).is_ok());
        let bom = [b"\xef\xbb\xbf".as_slice(), SINGLE.as_bytes()].concat();
        assert!(parse(&bom, "x", Mode::Simulated).is_ok());
    }

    /// T7: no key is skipped, at any level.
    #[test]
    fn unknown_keys_are_refused_not_skipped() {
        let mut v = single();
        v["vendor"] = json!("x");
        v["files"][0]["comment"] = json!("x");
        let e = &mut v["entries"][0];
        e["wibble"] = json!(1);
        e["images"][0]["fill"] = json!("0xFF");
        e["params"][0]["step"] = json!(1);
        e["steps"][0]["timeout"] = json!(5);
        let m = check(&v);
        for at in [
            "vendor：未知字段",
            "files[0].comment：未知字段",
            "entries[0].wibble：未知字段",
            "entries[0].images[0].fill：未知字段",
            "entries[0].params[0].step：未知字段",
            "entries[0].steps[0].timeout：未知字段",
        ] {
            assert!(has(&m, IssueKind::Unsupported, at), "{at}: {:?}", all(&m));
        }
        // still read: the entry is complete otherwise
        assert_eq!(m.entries[0].steps.len(), 7);
    }

    #[test]
    fn scripts_urls_frames_and_eval_are_refused() {
        let mut v = single();
        v["hooks"] = json!({"post": "x.py"});
        let e = &mut v["entries"][0];
        e["scripts"] = json!(["a.js"]);
        e["url"] = json!("https://example.com/app.bin");
        e["uri"] = json!("file:///etc/passwd");
        e["frames"] = json!([[0x10, 0x02]]);
        e["eval"] = json!("1+1");
        let m = check(&v);
        for want in [
            "hooks：外部脚本",
            "entries[0].scripts：外部脚本",
            "entries[0].url：远程地址",
            "entries[0].uri：远程地址",
            "entries[0].frames：自由报文",
            "entries[0].eval：内嵌代码",
        ] {
            assert!(
                has(&m, IssueKind::Unsupported, want),
                "{want}: {:?}",
                all(&m)
            );
        }
        assert_eq!(
            m.check_notes
                .iter()
                .filter(|n| n.contains("未执行"))
                .count(),
            2
        );
        assert!(m.blockers(&m.entries[0]).count() >= 6);
    }

    #[test]
    fn paths_stay_inside_the_package() {
        for path in [
            "../app.bin",
            "a/../../app.bin",
            "/etc/app.bin",
            "C:\\app.bin",
            "C:app.bin",
            "c:/app.bin",
            "a\\b.bin",
            "https://example.com/app.bin",
            "HTTP:app.bin",
            "a/./b.bin",
            "a//b.bin",
            "",
        ] {
            let mut v = single();
            v["files"][0]["path"] = json!(path);
            let m = check(&v);
            assert!(
                m.issues
                    .iter()
                    .any(|i| i.kind == IssueKind::Unsupported
                        && i.text.starts_with("files[0]：路径")),
                "{path}: {:?}",
                all(&m)
            );
            assert!(!m.files.iter().any(|f| f.path == path), "{path}");
        }
        // a subdirectory is fine
        let mut v = single();
        v["files"][0]["path"] = json!("img/app.bin");
        v["entries"][0]["images"][0]["file"] = json!("img/app.bin");
        assert!(all(&check(&v)).is_empty());
        // needs and image files are checked the same way
        let mut v = single();
        v["entries"][0]["needs"] = json!(["../flash_driver.bin"]);
        v["entries"][0]["images"][0]["file"] = json!("/app.bin");
        let m = check(&v);
        assert!(has(
            &m,
            IssueKind::Unsupported,
            "needs[0]：路径 ../flash_driver.bin 含 .."
        ));
        assert!(has(
            &m,
            IssueKind::Unsupported,
            "images[0]：路径 /app.bin 是绝对路径"
        ));
    }

    #[test]
    fn step_kinds_are_whitelisted() {
        let mut v = single();
        let steps = &mut v["entries"][0]["steps"];
        steps[0] = json!({"kind": "routine_ctrl", "label": "例程"});
        steps[1]["effect"] = json!(true);
        steps[3]["effect"] = json!(false);
        steps[4] = json!({"kind": "download"});
        let m = check(&v);
        let e = &m.entries[0];
        assert!(has(
            &m,
            IssueKind::Unsupported,
            "steps[0]：步骤类型 routine_ctrl"
        ));
        assert!(has(
            &m,
            IssueKind::Unsupported,
            "steps[3]：erase 会改变目标"
        ));
        assert!(has(
            &m,
            IssueKind::Missing,
            "steps[4]：步骤需要 kind 和 label"
        ));
        // the unknown kind stays in the list, to be shown; the bad one does not
        let effects: Vec<(&str, bool)> =
            e.steps.iter().map(|s| (s.id.as_str(), s.effect)).collect();
        assert_eq!(
            effects,
            [
                ("routine_ctrl", false),
                ("session", true),
                ("security", false),
                ("erase", true),
                ("verify", false),
                ("reset", true),
            ]
        );
        // no steps at all
        let mut v = single();
        v["entries"][0]["steps"] = json!([]);
        assert!(has(&check(&v), IssueKind::Missing, "缺少 Flash Job"));
    }

    /// T8: `simulate` is for the simulated backend only.
    #[test]
    fn simulate_needs_a_simulated_backend() {
        let sim = read(MULTI, Mode::Simulated);
        assert!(sim.simulated_only && sim.issues.is_empty());
        let hw = read(MULTI, Mode::Hardware);
        assert!(hw.simulated_only);
        let refused: Vec<&str> = hw
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Unsupported)
            .map(|i| i.text.as_str())
            .collect();
        assert_eq!(refused.len(), 2, "{refused:?}");
        assert!(refused[0].starts_with("entries[1].simulate：故障注入只供模拟后端"));
        // a package issue: it blocks app too, which has no simulate itself
        let app = hw.entry("app").unwrap();
        assert!(app.issues.is_empty() && hw.blockers(app).next().is_some());
        // a package without simulate is not affected
        let plain = read(SINGLE, Mode::Hardware);
        assert!(!plain.simulated_only && plain.issues.is_empty());

        for (simulate, want) in [
            (json!({"failAt": "verify"}), "指向的步骤 verify 不在入口中"),
            (json!({}), "只能给出 failAt 或 lostAt"),
            (
                json!({"failAt": "erase", "lostAt": "erase"}),
                "只能给出 failAt 或 lostAt",
            ),
            (json!({"failAt": "erase", "at": 2}), "simulate.at：未知字段"),
        ] {
            let mut v = single();
            v["entries"][0]["steps"][5] = json!({"kind": "reset", "label": "复位"});
            v["entries"][0]["simulate"] = simulate;
            let m = check(&v);
            assert!(m.simulated_only);
            assert!(
                has(&m, IssueKind::Unsupported, want),
                "{want}: {:?}",
                all(&m)
            );
        }
    }

    #[test]
    fn images_need_addresses_unless_segmented() {
        let image = |v: &mut Value, im: Value| v["entries"][0]["images"][0] = im;
        let cases = [
            (
                json!({"file": "app.bin", "base": "0x10000"}),
                "必须给出 base 和 len",
            ),
            (
                json!({"file": "app.bin", "len": 16}),
                "必须给出 base 和 len",
            ),
            (
                json!({"file": "app.bin", "base": "0x0", "len": 0}),
                "长度为 0",
            ),
            (
                json!({"file": "app.bin", "base": "0x0", "len": 1u64 << 32}),
                "超出 32 位",
            ),
            (
                json!({"file": "app.bin", "base": "65536", "len": 16}),
                "应为 0x 开头",
            ),
            (
                json!({"file": "app.bin", "base": "0x+10", "len": 16}),
                "应为 0x 开头",
            ),
            (
                json!({"file": "app.bin", "base": "0x1FFFFFFFF", "len": 16}),
                "应为 0x 开头",
            ),
            (
                json!({"file": "app.bin", "base": "0xFFFFFFF0", "len": 32}),
                "地址越界",
            ),
        ];
        for (im, want) in cases {
            let mut v = single();
            image(&mut v, im.clone());
            let m = check(&v);
            assert!(has(&m, IssueKind::Unsupported, want), "{im}: {:?}", all(&m));
            assert!(m.entries[0].images.is_empty(), "{im}");
            assert!(has(&m, IssueKind::Missing, "下载步骤没有镜像"), "{im}");
        }
        // HEX / S19 hold their own addresses: base and len may be left out
        let mut v = single();
        v["files"][0]["media"] = json!("hex");
        image(&mut v, json!({"file": "app.bin"}));
        let m = check(&v);
        assert!(all(&m).is_empty(), "{:?}", all(&m));
        assert_eq!(m.entries[0].images[0].base, None);
        // the image file must be listed
        let mut v = single();
        image(&mut v, json!({"file": "cal.bin", "base": "0x0", "len": 16}));
        let m = check(&v);
        assert!(has(&m, IssueKind::Missing, "镜像 cal.bin 不在包清单"));
    }

    #[test]
    fn params_follow_the_api_shape() {
        let cases = [
            (
                json!({"id": "p", "label": "P", "editable": true, "lo": 10, "hi": 1, "default": 5}),
                (IssueKind::Unsupported, "范围或默认值无效"),
            ),
            (
                json!({"id": "p", "label": "P", "editable": true, "lo": 1, "hi": 5_000_000_000u64, "default": 5}),
                (IssueKind::Unsupported, "范围或默认值无效"),
            ),
            (
                json!({"id": "p", "label": "P", "editable": true, "value": "5"}),
                (IssueKind::Unsupported, "不用 value"),
            ),
            (
                json!({"id": "p", "label": "P", "editable": false}),
                (IssueKind::Missing, "缺少 value"),
            ),
            (
                json!({"id": "p", "label": "P", "editable": false, "value": "1", "hi": 2}),
                (IssueKind::Unsupported, "不带 lo / hi / default"),
            ),
            (
                json!({"id": "p", "label": "P"}),
                (IssueKind::Missing, "需要 id、label 和 editable"),
            ),
            (
                json!({"id": "p2", "label": "P", "editable": false, "value": "1"}),
                (IssueKind::Unsupported, "参数 p2 重复"),
            ),
        ];
        for (param, (kind, want)) in cases {
            let mut v = single();
            v["entries"][0]["params"]
                .as_array_mut()
                .unwrap()
                .push(param.clone());
            let m = check(&v);
            assert!(has(&m, kind, want), "{param}: {:?}", all(&m));
            assert_eq!(m.entries[0].params.len(), 4, "{param}");
        }
    }

    #[test]
    fn files_and_entries_are_checked() {
        let mut v = single();
        v["files"][0]["sha256"] = json!("AB".repeat(32));
        v["files"][1]["sha256"] = json!("ab".repeat(32));
        v["files"][0]["role"] = json!("firmware");
        v["files"][1]["media"] = json!("elf");
        v["files"]
            .as_array_mut()
            .unwrap()
            .push(json!({"path": "app.bin"}));
        v["entries"][0]["requires"] = json!(["otx-extension:x"]);
        let mut twin = v["entries"][0].clone();
        twin["ecu"] = json!("");
        twin.as_object_mut().unwrap().remove("title");
        v["entries"].as_array_mut().unwrap().push(twin);
        let m = check(&v);
        for (kind, want) in [
            (IssueKind::Unsupported, "files[0]：sha256 应为 64 位小写"),
            (IssueKind::Unsupported, "files[0]：role firmware"),
            (IssueKind::Unsupported, "files[1]：media elf"),
            (IssueKind::Unsupported, "files[2]：路径 app.bin 重复"),
            (IssueKind::Unsupported, "requires[0]：需要 otx-extension:x"),
            (IssueKind::Unsupported, "entries[1]：入口 app 重复"),
            (IssueKind::Missing, "entries[1]：未声明目标 ECU"),
        ] {
            assert!(has(&m, kind, want), "{want}: {:?}", all(&m));
        }
        assert_eq!(m.files[0].sha256, None);
        assert_eq!(m.files[1].sha256.as_deref(), Some("ab".repeat(32).as_str()));
        assert!(m.check_notes.iter().any(|n| n.contains("未与文件内容核对")));
        assert_eq!(m.entries[1].title, "app");

        let m = check(&json!({"format": FORMAT, "version": 1, "entries": []}));
        assert!(has(&m, IssueKind::Missing, "缺少 files 清单"));
        assert!(has(&m, IssueKind::Missing, "包内没有入口"));
        assert_eq!(m.package, "t.udsflow.json");
        let m = check(&json!({"format": FORMAT, "version": 1, "files": [],
            "entries": [{"ecu": "E", "steps": [{"kind": "session", "label": "S"}]}]}));
        assert!(has(&m, IssueKind::Missing, "entries[0]：入口缺少 id"));
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(ParseError::NotRecognised.to_string().contains(FORMAT));
        assert!(ParseError::TooLarge(1)
            .to_string()
            .contains(&MAX_ASSET.to_string()));
        assert!(ParseError::Malformed("eof".into())
            .to_string()
            .contains("不是有效的 JSON"));
    }
}
