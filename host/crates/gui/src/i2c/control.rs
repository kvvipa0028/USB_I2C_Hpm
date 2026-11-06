//! Device-confirmed port controls. Draft edits never send automatically.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use eframe::egui;
use usb_i2c::control::*;
use usb_i2c::device::DeviceError;
use usb_i2c::proto::Capabilities;

use super::{parse::parse_addr, Ctx};
use crate::local::parse_num;
use crate::widgets::{self, Tone};
use crate::worker::{ControlReply, ControlRequest};

#[derive(Clone, PartialEq)]
struct Draft {
    role: PortRole,
    address: String,
    speed: String,
    stretch: String,
    total: String,
}

impl From<&PortConfig> for Draft {
    fn from(config: &PortConfig) -> Self {
        Self {
            role: config.role,
            address: format!("0x{:02X}", config.slave_address),
            speed: config.speed_hz.to_string(),
            stretch: config.stretch_timeout_us.to_string(),
            total: config.transaction_timeout_us.to_string(),
        }
    }
}

impl Draft {
    fn config(&self) -> Result<PortConfig, String> {
        let config = PortConfig {
            role: self.role,
            slave_address: parse_addr(&self.address)?,
            speed_hz: parse_num(&self.speed, 10_000, 1_000_000, "默认速率")?,
            stretch_timeout_us: parse_num(&self.stretch, 1000, 100_000, "拉伸超时")?,
            transaction_timeout_us: parse_num(&self.total, 1000, 2_000_000, "整笔超时")?,
        };
        config.validate()?;
        Ok(config)
    }
}

pub(super) struct PortControl {
    pub caps: ControlCapabilities,
    pub confirmed: Option<PortState>,
    draft: Draft,
    dirty: bool,
    frontend: Option<FrontendState>,
    frontend_draft: FrontendConfig,
    frontend_dirty: bool,
    scan_cancel: Option<Arc<AtomicBool>>,
    scan: Option<ScanResult>,
    recovery: Option<RecoveryResult>,
    notice: Option<(String, Tone)>,
}

impl PortControl {
    fn new(port: u8) -> Self {
        Self {
            caps: ControlCapabilities::unsupported(port),
            confirmed: None,
            draft: Draft::from(&PortConfig::default()),
            dirty: false,
            frontend: None,
            frontend_draft: FrontendConfig {
                source: FrontendSource::External,
                pullup: PullupSetting::Off,
            },
            frontend_dirty: false,
            scan_cancel: None,
            scan: None,
            recovery: None,
            notice: None,
        }
    }

    fn update_state(&mut self, state: &PortState, applied: bool) {
        if self
            .frontend
            .as_ref()
            .is_some_and(|f| f.config_generation != state.config_generation)
        {
            self.frontend = None;
        }
        if applied || !self.dirty {
            self.draft = Draft::from(&state.config);
            self.dirty = false;
        }
        self.confirmed = Some(state.clone());
    }

    fn frontend_request(&self, port: u8) -> Option<ControlRequest> {
        if !self.caps.supports_frontend_config(&self.frontend_draft) {
            return None;
        }
        // GET and SET are independent capabilities. A confirmed port state
        // supplies the same configuration generation even on a SET-only device.
        let generation = self
            .confirmed
            .as_ref()
            .map(|s| s.config_generation)
            .or_else(|| self.frontend.as_ref().map(|s| s.config_generation))?;
        Some(ControlRequest::FrontendSet {
            port,
            config: self.frontend_draft,
            expected: generation,
        })
    }

    fn apply_request(&self, port: u8) -> Result<ControlRequest, String> {
        if !self.caps.supports_role_config {
            return Err("设备未报告端口配置能力".into());
        }
        let current = self.confirmed.as_ref().ok_or("先读取端口实际配置")?;
        let config = self.draft.config()?;
        if self.caps.supports_timeouts {
            if !(self.caps.stretch_min_us..=self.caps.stretch_max_us)
                .contains(&config.stretch_timeout_us)
                || !(self.caps.transaction_min_us..=self.caps.transaction_max_us)
                    .contains(&config.transaction_timeout_us)
            {
                return Err("超时参数超出当前端口能力".into());
            }
        } else if config.stretch_timeout_us != current.config.stretch_timeout_us
            || config.transaction_timeout_us != current.config.transaction_timeout_us
        {
            return Err("设备不支持修改超时参数".into());
        }
        Ok(ControlRequest::Configure {
            port,
            config,
            expected: Some(current.config_generation),
        })
    }

    fn master_ready(&self) -> bool {
        self.confirmed
            .as_ref()
            .is_some_and(|s| s.config.role == PortRole::Master && s.activity == PortActivity::Idle)
    }

    fn start_scan(&mut self, port: u8) -> ControlRequest {
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan_cancel = Some(cancel.clone());
        self.scan = None;
        ControlRequest::Scan { port, cancel }
    }

    fn cancel_scan(&self) {
        if let Some(cancel) = &self.scan_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn recover_ready(&self) -> bool {
        self.confirmed
            .as_ref()
            .is_some_and(|s| s.config.role == PortRole::Master && s.activity != PortActivity::Busy)
    }

    fn clear_confirmed(&mut self) {
        self.confirmed = None;
        self.frontend = None;
        if let Some(cancel) = self.scan_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.scan = None;
        self.recovery = None;
        self.notice = None;
    }
}

pub(super) struct Controls {
    pub port: u8,
    pub ports: [PortControl; 2],
    port_count: u8,
    offline: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            port: 0,
            ports: std::array::from_fn(|p| PortControl::new(p as u8)),
            port_count: 1,
            offline: false,
        }
    }
}

impl Controls {
    pub fn set_connection(&mut self, caps: &Capabilities, offline: bool) {
        self.disconnected();
        self.port_count = caps.port_count.min(2);
        self.port = 0;
        self.offline = offline;
    }

    pub fn set_controls(&mut self, caps: &[ControlCapabilities], states: &[PortState]) {
        for item in caps {
            if let Some(port) = self.ports.get_mut(usize::from(item.port)) {
                port.caps = item.clone();
            }
        }
        for state in states {
            if let Some(port) = self.ports.get_mut(usize::from(state.port)) {
                port.update_state(state, false);
            }
        }
    }

    pub fn disconnected(&mut self) {
        for port in &mut self.ports {
            port.clear_confirmed();
            port.caps = ControlCapabilities::unsupported(port.caps.port);
        }
    }

    pub fn on_control(
        &mut self,
        request: &ControlRequest,
        result: &Result<ControlReply, DeviceError>,
    ) -> (String, Tone) {
        let port = request.port();
        let Some(p) = self.ports.get_mut(usize::from(port)) else {
            return ("控制回复端口不在范围内".into(), Tone::Red);
        };
        if matches!(request, ControlRequest::Scan { .. }) {
            p.scan_cancel = None;
        }
        let summary = match result {
            Err(error) => {
                if error.session_invalidated
                    || (error.status == Some(ControlStatus::Conflict)
                        && !matches!(request, ControlRequest::TableWrite { .. }))
                    || error.status == Some(ControlStatus::WrongRole)
                {
                    p.confirmed = None;
                    p.frontend = None;
                }
                (
                    format!(
                        "CH{port} {}失败：{}{}",
                        request.label(),
                        error,
                        if error.result_unknown {
                            "；结果未知，请读回确认，不会自动重发"
                        } else {
                            ""
                        }
                    ),
                    Tone::Red,
                )
            }
            Ok(ControlReply::State(state)) => {
                p.update_state(state, matches!(request, ControlRequest::Configure { .. }));
                (
                    format!(
                        "CH{port} 已读回 {:?} 配置，generation {}",
                        state.config.role, state.config_generation
                    ),
                    Tone::Green,
                )
            }
            Ok(ControlReply::Scan(scan)) => {
                p.scan = Some(scan.clone());
                let outcome = if scan.complete {
                    "完整完成"
                } else if scan.cancelled {
                    "已取消，保留部分结果"
                } else {
                    "未完成，保留部分结果"
                };
                let error = scan
                    .error
                    .as_ref()
                    .map_or(String::new(), |e| format!("：{e}"));
                (
                    format!(
                        "CH{port} 扫描{outcome}，发现 {} 个地址{error}",
                        scan.addresses.len()
                    ),
                    if scan.complete {
                        Tone::Green
                    } else if scan.cancelled {
                        Tone::Amber
                    } else {
                        Tone::Red
                    },
                )
            }
            Ok(ControlReply::Recovered(recovery)) => {
                p.recovery = Some(recovery.clone());
                (
                    format!(
                        "CH{port} 恢复{}：{} 个脉冲，线状态 {:02b} → {:02b}，reason {}",
                        if recovery.succeeded() {
                            "成功"
                        } else {
                            "失败"
                        },
                        recovery.clocks_sent,
                        recovery.initial_lines,
                        recovery.final_lines,
                        recovery.reason
                    ),
                    if recovery.succeeded() {
                        Tone::Green
                    } else {
                        Tone::Red
                    },
                )
            }
            Ok(ControlReply::Frontend(frontend)) => {
                if !p.frontend_dirty || matches!(request, ControlRequest::FrontendSet { .. }) {
                    p.frontend_draft = frontend.config;
                    p.frontend_dirty = false;
                }
                p.frontend = Some(frontend.clone());
                if let Some(state) = &mut p.confirmed {
                    state.config_generation = frontend.config_generation;
                }
                (
                    format!(
                        "CH{port} 前端状态已读回，generation {}，fault {}",
                        frontend.config_generation, frontend.fault
                    ),
                    if frontend.fault == 0 {
                        Tone::Green
                    } else {
                        Tone::Red
                    },
                )
            }
            Ok(ControlReply::Table(snapshot)) => {
                if let Some(state) = &mut p.confirmed {
                    state.table_generation = snapshot.generation;
                }
                (
                    format!(
                        "CH{port} 读回 {} B，table generation {}",
                        snapshot.data.len(),
                        snapshot.generation
                    ),
                    Tone::Green,
                )
            }
            Ok(ControlReply::Committed(commit)) => {
                if let Some(state) = &mut p.confirmed {
                    state.table_generation = commit.generation;
                }
                (
                    format!(
                        "CH{port} 已提交 {} B，table generation {}",
                        commit.len, commit.generation
                    ),
                    Tone::Green,
                )
            }
        };
        p.notice = Some(summary.clone());
        summary
    }

    pub fn show(&mut self, ui: &mut egui::Ui, cx: Ctx) -> Option<ControlRequest> {
        let mut action = None;
        widgets::section(ui, "端口配置与控制", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("所选端口");
                for port in 0..self.port_count {
                    ui.add_enabled_ui(!cx.busy, |ui| {
                        ui.selectable_value(&mut self.port, port, format!("CH{port}"));
                    });
                }
                widgets::badge(
                    ui,
                    if self.offline { "离线模型" } else { "USB" },
                    if self.offline {
                        Tone::Amber
                    } else {
                        Tone::Neutral
                    },
                );
            });
            let port = self.port;
            let p = &mut self.ports[usize::from(port)];
            if !p.caps.supports_role_config {
                widgets::note(
                    ui,
                    "当前连接未提供端口配置能力；保留原有主机传输，角色与表同步不可用。",
                );
                frontend(ui, port, p, cx, &mut action);
                return;
            }
            match &p.confirmed {
                Some(state) => {
                    ui.label(format!("实际：{:?} · {:?} · 地址 0x{:02X} · 默认 {} Hz · 计算 {} Hz · config generation {} · table generation {}",
                        state.config.role, state.activity, state.config.slave_address, state.config.speed_hz, state.calculated_speed_hz,
                        state.config_generation, state.table_generation));
                    widgets::muted(
                        ui,
                        &format!(
                            "实际超时：拉伸 {} us / 整笔 {} us；计算速率不等于实测波形。",
                            state.config.stretch_timeout_us, state.config.transaction_timeout_us
                        ),
                    );
                }
                None => widgets::note(ui, "实际配置尚未确认，先读取端口状态；草稿不会自动应用。"),
            }
            let before = p.draft.clone();
            ui.add_enabled_ui(cx.connected && !cx.busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("角色草稿");
                    ui.selectable_value(&mut p.draft.role, PortRole::Master, "主机");
                    ui.selectable_value(&mut p.draft.role, PortRole::Slave, "从机");
                    small_edit(ui, "从机地址", &mut p.draft.address, 65.0, "");
                    small_edit(ui, "默认速率", &mut p.draft.speed, 80.0, "Hz");
                });
                ui.add_enabled_ui(p.caps.supports_timeouts, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        small_edit(ui, "拉伸超时", &mut p.draft.stretch, 80.0, "us");
                        small_edit(ui, "整笔超时", &mut p.draft.total, 90.0, "us");
                    });
                });
            });
            if before != p.draft {
                p.dirty = true;
            }
            let apply = p.apply_request(port);
            if let Err(error) = &apply {
                widgets::muted(ui, error);
            }
            ui.horizontal_wrapped(|ui| {
                let on = cx.connected && !cx.busy;
                if widgets::action_button(ui, on, "读取实际配置").clicked() {
                    action = Some(ControlRequest::State(port));
                }
                if widgets::ink_button(ui, on && apply.is_ok(), "应用配置草稿 (CAS)").clicked()
                {
                    action = apply.ok();
                }
                if widgets::quiet_button(ui, on && p.confirmed.is_some(), "用实际值重填草稿")
                    .clicked()
                {
                    if let Some(state) = &p.confirmed {
                        p.draft = Draft::from(&state.config);
                        p.dirty = false;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                let on = cx.connected && !cx.busy && p.master_ready();
                if widgets::action_button(ui, on && p.caps.supports_probe, "扫描地址 08–77")
                    .clicked()
                {
                    action = Some(p.start_scan(port));
                }
                if let Some(cancel) = &p.scan_cancel {
                    if widgets::action_button(ui, !cancel.load(Ordering::Relaxed), "取消扫描")
                        .clicked()
                    {
                        p.cancel_scan();
                    }
                    widgets::muted(
                        ui,
                        if cancel.load(Ordering::Relaxed) {
                            "正在结束当前地址事务"
                        } else {
                            "扫描中"
                        },
                    );
                }
                if widgets::action_button(
                    ui,
                    cx.connected && !cx.busy && p.recover_ready() && p.caps.supports_recover,
                    "恢复总线",
                )
                .clicked()
                {
                    action = Some(ControlRequest::Recover(port));
                }
            });
            if let Some(scan) = &p.scan {
                let addresses = scan
                    .addresses
                    .iter()
                    .map(|a| format!("0x{a:02X}"))
                    .collect::<Vec<_>>()
                    .join("  ");
                ui.label(format!(
                    "{}扫描结果：{}",
                    if scan.complete { "完整" } else { "部分" },
                    if addresses.is_empty() {
                        "无已确认地址"
                    } else {
                        &addresses
                    }
                ));
            }
            if let Some((message, tone)) = &p.notice {
                let palette = crate::theme::of(ui);
                let color = match tone {
                    Tone::Red => palette.red,
                    Tone::Green => palette.green,
                    Tone::Amber => palette.amber,
                    _ => palette.muted,
                };
                ui.label(egui::RichText::new(message).color(color));
            }
            frontend(ui, port, p, cx, &mut action);
            if self.offline {
                widgets::note(
                    ui,
                    "离线控制仅验证 PC 合同；不会产生总线波形、真实拉伸或前端电压测量。",
                );
            }
        });
        action
    }
}

fn small_edit(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32, unit: &str) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(width));
    if !unit.is_empty() {
        ui.label(unit);
    }
}

fn frontend(
    ui: &mut egui::Ui,
    port: u8,
    p: &mut PortControl,
    cx: Ctx,
    action: &mut Option<ControlRequest>,
) {
    egui::CollapsingHeader::new("前端与 VREF")
        .id_salt(("frontend", port))
        .show(ui, |ui| {
            if !p.caps.supports_frontend_get && !p.caps.supports_frontend_set {
                widgets::note(
                    ui,
                    "当前设备未提供前端能力；VREF 无有效测量，上拉与参考源不可配置。",
                );
                return;
            }
            if let Some(state) = &p.frontend {
                ui.label(format!(
                    "实际：{:?} / {:?} · generation {} · fault {}",
                    state.config.source, state.config.pullup, state.config_generation, state.fault
                ));
                ui.label(
                    state
                        .measured_vref_mv
                        .map_or("VREF：无有效测量".into(), |mv| {
                            format!("VREF 实测：{mv} mV")
                        }),
                );
            } else {
                ui.label("尚未读取前端状态；VREF 无有效测量");
            }
            let before = p.frontend_draft;
            ui.add_enabled_ui(
                cx.connected && !cx.busy && p.caps.supports_frontend_set,
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt(("source", port))
                            .selected_text(format!("{:?}", p.frontend_draft.source))
                            .show_ui(ui, |ui| {
                                for (source, name) in [
                                    (FrontendSource::External, "外部参考"),
                                    (FrontendSource::Internal1V8, "内部 1.8 V"),
                                    (FrontendSource::Internal3V3, "内部 3.3 V"),
                                    (FrontendSource::Internal5V, "内部 5 V"),
                                ] {
                                    if p.caps.frontend_options & (1 << source as u8) != 0 {
                                        ui.selectable_value(
                                            &mut p.frontend_draft.source,
                                            source,
                                            name,
                                        );
                                    }
                                }
                            });
                        egui::ComboBox::from_id_salt(("pullup", port))
                            .selected_text(format!("{:?}", p.frontend_draft.pullup))
                            .show_ui(ui, |ui| {
                                for (pullup, name) in [
                                    (PullupSetting::Off, "关闭上拉"),
                                    (PullupSetting::Ohm4K7, "4.7 kΩ"),
                                    (PullupSetting::Ohm2K2, "2.2 kΩ"),
                                ] {
                                    if p.caps.frontend_options & (1 << (8 + pullup as u8)) != 0 {
                                        ui.selectable_value(
                                            &mut p.frontend_draft.pullup,
                                            pullup,
                                            name,
                                        );
                                    }
                                }
                            });
                    });
                },
            );
            if before != p.frontend_draft {
                p.frontend_dirty = true;
            }
            ui.horizontal_wrapped(|ui| {
                let on = cx.connected && !cx.busy;
                if widgets::action_button(ui, on && p.caps.supports_frontend_get, "读取前端状态")
                    .clicked()
                {
                    *action = Some(ControlRequest::FrontendGet(port));
                }
                let request = p.frontend_request(port);
                if widgets::action_button(ui, on && request.is_some(), "应用前端草稿 (CAS)")
                    .clicked()
                {
                    *action = request;
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use usb_i2c::device::Device;

    fn connected() -> Controls {
        let mut device = Device::offline();
        let mut controls = Controls::default();
        controls.set_connection(device.capabilities(), true);
        let states = [device.port_state(0).unwrap(), device.port_state(1).unwrap()];
        controls.set_controls(device.control_capabilities(), &states);
        controls
    }

    #[test]
    fn drafts_are_independent_and_apply_uses_confirmed_generation() {
        let mut controls = connected();
        controls.ports[0].draft.role = PortRole::Slave;
        controls.ports[0].draft.address = "0x52".into();
        controls.ports[0].dirty = true;
        let request = controls.ports[0].apply_request(0).unwrap();
        match request {
            ControlRequest::Configure {
                port,
                config,
                expected,
            } => {
                assert_eq!(
                    (port, config.role, config.slave_address, expected),
                    (0, PortRole::Slave, 0x52, Some(0))
                );
            }
            _ => panic!("expected configuration"),
        }
        assert_eq!(controls.ports[1].draft.role, PortRole::Master);
        assert!(controls.ports[0].master_ready()); // A draft is not an applied role.
        let readback = controls.ports[0].confirmed.clone().unwrap();
        controls.on_control(
            &ControlRequest::State(0),
            &Ok(ControlReply::State(readback)),
        );
        assert_eq!(controls.ports[0].draft.role, PortRole::Slave); // Read-only refresh preserves dirty draft.
        controls.disconnected();
        assert!(controls.ports[0].confirmed.is_none());
        assert!(controls.ports[0].apply_request(0).is_err());
        assert_eq!(controls.ports[0].draft.address, "0x52");
    }

    #[test]
    fn scan_cancellation_uses_the_same_flag_without_a_queued_request() {
        let mut controls = connected();
        let request = controls.ports[0].start_scan(0);
        let ControlRequest::Scan { cancel, .. } = &request else {
            panic!()
        };
        assert!(!cancel.load(Ordering::Relaxed));
        controls.ports[0].cancel_scan();
        assert!(cancel.load(Ordering::Relaxed));
        let result = ScanResult {
            addresses: vec![0x50],
            complete: false,
            cancelled: true,
            error: None,
        };
        let (text, tone) = controls.on_control(&request, &Ok(ControlReply::Scan(result)));
        assert!(tone == Tone::Amber);
        assert!(text.contains("部分"));
        assert!(controls.ports[0].scan_cancel.is_none());
        assert_eq!(controls.ports[0].scan.as_ref().unwrap().addresses, [0x50]);
        let request = controls.ports[1].start_scan(1);
        controls.disconnected();
        let ControlRequest::Scan { cancel, .. } = request else {
            panic!()
        };
        assert!(cancel.load(Ordering::Relaxed));
        assert!(controls
            .ports
            .iter()
            .all(|p| p.scan.is_none() && p.scan_cancel.is_none()));
    }

    #[test]
    fn recovery_failure_and_scan_error_are_not_success_even_with_ok_transport() {
        let mut controls = connected();
        let recovery = RecoveryResult {
            port: 0,
            clocks_sent: 4,
            initial_lines: 1,
            final_lines: 1,
            reason: 2,
        };
        let (text, tone) = controls.on_control(
            &ControlRequest::Recover(0),
            &Ok(ControlReply::Recovered(recovery)),
        );
        assert!(tone == Tone::Red && text.contains("失败") && text.contains("reason 2"));
        let error = DeviceError {
            message: "bus timeout".into(),
            session_invalidated: false,
            result_unknown: false,
            status: Some(ControlStatus::Timeout),
            hpm_stat: Some(2001),
        };
        let scan = ScanResult {
            addresses: vec![0x20],
            complete: false,
            cancelled: false,
            error: Some(error),
        };
        let request = controls.ports[0].start_scan(0);
        let (text, tone) = controls.on_control(&request, &Ok(ControlReply::Scan(scan)));
        assert!(tone == Tone::Red && text.contains("未完成") && text.contains("bus timeout"));
    }

    #[test]
    fn configuration_conflict_requires_readback_but_table_conflict_keeps_role() {
        let mut controls = connected();
        let error = DeviceError {
            message: "conflict".into(),
            session_invalidated: false,
            result_unknown: false,
            status: Some(ControlStatus::Conflict),
            hpm_stat: Some(0),
        };
        controls.on_control(
            &ControlRequest::TableWrite {
                port: 0,
                offset: 0,
                data: vec![1],
                expected: Some(0),
            },
            &Err(error.clone()),
        );
        assert!(controls.ports[0].confirmed.is_some());
        let request = controls.ports[0].apply_request(0).unwrap();
        controls.on_control(&request, &Err(error));
        assert!(controls.ports[0].confirmed.is_none());
        assert!(controls.ports[0].apply_request(0).is_err());
    }

    #[test]
    fn frontend_set_without_get_uses_confirmed_port_generation_and_drops_stale_measurement() {
        let mut controls = connected();
        let p = &mut controls.ports[0];
        p.caps.supports_frontend_set = true;
        p.caps.supports_frontend_get = false;
        p.caps.frontend_options = 1 | (1 << 8); // External / Off.
        p.confirmed.as_mut().unwrap().config_generation = 7;
        assert!(p.frontend.is_none());
        assert!(matches!(
            p.frontend_request(0),
            Some(ControlRequest::FrontendSet {
                expected: 7,
                port: 0,
                ..
            })
        ));
        p.frontend = Some(FrontendState {
            port: 0,
            config: p.frontend_draft,
            measured_vref_mv: Some(3300),
            fault: 0,
            config_generation: 7,
        });
        let mut updated = p.confirmed.clone().unwrap();
        updated.config_generation = 8;
        p.update_state(&updated, true);
        assert!(
            p.frontend.is_none(),
            "previous generation must not appear as current measurement"
        );
        assert!(matches!(
            p.frontend_request(0),
            Some(ControlRequest::FrontendSet { expected: 8, .. })
        ));
        controls.disconnected();
        assert!(controls.ports[0].frontend_request(0).is_none());
    }
}

#[cfg(test)]
mod fault_tests {
    use super::*;

    #[test]
    fn recovery_is_available_for_faulted_master_but_not_busy_or_slave() {
        let mut port = PortControl::new(0);
        port.confirmed = Some(PortState {
            port: 0,
            config: PortConfig::default(),
            activity: PortActivity::Fault,
            calculated_speed_hz: 100_000,
            config_generation: 2,
            table_generation: 0,
        });
        assert!(!port.master_ready());
        assert!(port.recover_ready());
        port.confirmed.as_mut().unwrap().activity = PortActivity::Busy;
        assert!(!port.recover_ready());
        port.confirmed.as_mut().unwrap().activity = PortActivity::Fault;
        port.confirmed.as_mut().unwrap().config.role = PortRole::Slave;
        assert!(!port.recover_ready());
    }
}
