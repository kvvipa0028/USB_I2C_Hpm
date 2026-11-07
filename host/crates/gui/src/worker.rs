//! USB runs on one worker thread so the window never blocks: the window
//! sends a `Job`, the worker answers with an `Event` and asks for a repaint.
//! The same jobs use either an explicitly selected offline model or USB.
//! Device capabilities gate port and speed selection; requests are never replayed.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use eframe::egui;
use usb_i2c::bridge::{list_devices, DeviceInfo};
use usb_i2c::control::{
    ControlCapabilities, FrontendConfig, FrontendState, PortConfig, PortState, RecoveryResult,
    ScanResult, TableCommit, TableSnapshot,
};
use usb_i2c::device::{BackendKind, Device, DeviceError};
use usb_i2c::proto::{Capabilities, TransferOutcome, TransferRequest};

pub const DISCONNECT_FIRST: &str = "已连接设备，先点断开";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Write,
    Read,
    WriteRead,
}

impl Op {
    pub fn label(self) -> &'static str {
        match self {
            Op::Write => "写入",
            Op::Read => "读取",
            Op::WriteRead => "写后读",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub port: u8,
    pub speed_hz: u32,
    pub offline: bool,
    pub op: Op,
    pub addr: u8,
    pub wdata: Vec<u8>,
    pub rlen: usize,
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub firmware: String,
    pub capabilities: Capabilities,
    pub offline: bool,
    pub control_capabilities: Vec<ControlCapabilities>,
    pub port_states: Vec<PortState>,
}

/// A single operation submitted by the window. Scan cancellation is shared
/// independently of the job queue so it can interrupt a running scan.
#[derive(Debug, Clone)]
pub enum ControlRequest {
    State(u8),
    Configure {
        port: u8,
        config: PortConfig,
        expected: Option<u64>,
    },
    TableRead {
        port: u8,
        offset: u16,
        len: u16,
    },
    TableWrite {
        port: u8,
        offset: u16,
        data: Vec<u8>,
        expected: Option<u64>,
    },
    Scan {
        port: u8,
        cancel: Arc<AtomicBool>,
    },
    Recover(u8),
    FrontendGet(u8),
    FrontendSet {
        port: u8,
        config: FrontendConfig,
        expected: u64,
    },
}

impl ControlRequest {
    pub fn port(&self) -> u8 {
        match self {
            Self::State(p) | Self::Recover(p) | Self::FrontendGet(p) => *p,
            Self::Configure { port, .. }
            | Self::TableRead { port, .. }
            | Self::TableWrite { port, .. }
            | Self::Scan { port, .. }
            | Self::FrontendSet { port, .. } => *port,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::State(_) => "读取端口配置",
            Self::Configure { .. } => "应用端口配置",
            Self::TableRead { .. } => "读取从机表",
            Self::TableWrite { .. } => "下发从机表",
            Self::Scan { .. } => "扫描地址",
            Self::Recover(_) => "恢复总线",
            Self::FrontendGet(_) => "读取前端状态",
            Self::FrontendSet { .. } => "应用前端配置",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ControlReply {
    State(PortState),
    Table(TableSnapshot),
    Committed(TableCommit),
    Scan(ScanResult),
    Recovered(RecoveryResult),
    Frontend(FrontendState),
}

pub enum Job {
    Refresh,
    Connect(String),
    ConnectOffline,
    /// Host side only: drop the open interface. Nothing is sent to the device.
    Disconnect,
    Transfer(Transfer),
    Batch(Vec<Transfer>),
    Control(ControlRequest),
}

pub enum Event {
    Devices(Result<Vec<DeviceInfo>, String>),
    Connected(Result<Connection, String>),
    Disconnected,
    Control {
        request: ControlRequest,
        result: Result<ControlReply, DeviceError>,
        connection_lost: bool,
    },
    Batch {
        transfers: Vec<Transfer>,
        result: Result<Vec<TransferOutcome>, String>,
        connection_lost: bool,
    },
    Transfer {
        transfer: Transfer,
        result: Result<Vec<u8>, String>,
        /// No usable USB connection remains. Reconnecting is an explicit user
        /// action; the failed transfer must never be replayed automatically.
        connection_lost: bool,
    },
}

/// Start the worker; the receiver gets one `Event` per `Job`.
pub fn spawn(ctx: egui::Context) -> (Sender<Job>, Receiver<Event>) {
    let (job_tx, job_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    std::thread::spawn(move || run(job_rx, ev_tx, ctx));
    (job_tx, ev_rx)
}

fn run(jobs: Receiver<Job>, events: Sender<Event>, ctx: egui::Context) {
    let mut bridge: Option<Device> = None;
    while let Ok(job) = jobs.recv() {
        let event = match job {
            Job::Refresh => Event::Devices(list_devices().map_err(|e| e.to_string())),
            Job::Connect(serial) => connect_bridge(&mut bridge, &serial, |serial| {
                Device::open_usb(Some(serial))
                    .map_err(|e| e.to_string())
                    .and_then(connection_info)
            }),
            Job::ConnectOffline => connect_bridge(&mut bridge, "offline", |_| {
                connection_info(Device::offline()).map_err(|e| e.to_string())
            }),
            Job::Disconnect => {
                bridge = None;
                Event::Disconnected
            }
            Job::Control(request) => {
                let result = match bridge.as_mut() {
                    Some(device) => run_control(device, &request),
                    None => Err(DeviceError {
                        message: "未连接，先点连接".into(),
                        session_invalidated: false,
                        result_unknown: false,
                        status: None,
                        hpm_stat: None,
                    }),
                };
                let connection_lost = bridge.as_ref().is_none_or(|b| !b.is_usable());
                if connection_lost {
                    bridge = None;
                }
                Event::Control {
                    request,
                    result,
                    connection_lost,
                }
            }
            Job::Batch(transfers) => {
                let (result, usable) = match bridge.as_mut() {
                    Some(b) => {
                        let requests: Vec<_> = transfers
                            .iter()
                            .map(|t| TransferRequest {
                                port: t.port,
                                addr: t.addr,
                                wdata: t.wdata.clone(),
                                rlen: t.rlen,
                                speed_hz: t.speed_hz,
                            })
                            .collect();
                        let result = b.batch(&requests).map_err(|e| e.to_string());
                        (result, b.is_usable())
                    }
                    None => (Err("未连接，先点连接".into()), false),
                };
                if !usable {
                    bridge = None;
                }
                Event::Batch {
                    transfers,
                    result,
                    connection_lost: !usable,
                }
            }
            Job::Transfer(t) => transfer_bridge(&mut bridge, t, |b, t| {
                let result = b
                    .i2c(&TransferRequest {
                        port: t.port,
                        addr: t.addr,
                        wdata: t.wdata.clone(),
                        rlen: t.rlen,
                        speed_hz: t.speed_hz,
                    })
                    .map_err(transfer_error_text);
                (result, b.is_usable())
            }),
        };
        if events.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn transfer_error_text(error: DeviceError) -> String {
    // A matched I2C error can preserve USB while a write has already changed
    // the target. Keep that diagnostic before converting the legacy event to
    // text; transport loss gets its reconnect warning in transfer_bridge.
    if error.result_unknown && !error.session_invalidated {
        format!("写入结果未知，请读回确认后再操作；未自动重试。{}", error)
    } else {
        error.to_string()
    }
}

fn connection_info(mut device: Device) -> Result<(Device, Connection), String> {
    let control_capabilities = device.control_capabilities().to_vec();
    let mut port_states = Vec::new();
    for cap in &control_capabilities {
        if cap.supports_role_config {
            port_states.push(device.port_state(cap.port).map_err(|e| e.to_string())?);
        }
    }
    let info = Connection {
        firmware: device.ping_line().map_err(|e| e.to_string())?,
        capabilities: device.capabilities().clone(),
        offline: device.backend() == BackendKind::Offline,
        control_capabilities,
        port_states,
    };
    Ok((device, info))
}

fn run_control(device: &mut Device, request: &ControlRequest) -> Result<ControlReply, DeviceError> {
    match request {
        ControlRequest::State(port) => device.port_state(*port).map(ControlReply::State),
        ControlRequest::Configure {
            port,
            config,
            expected,
        } => device
            .configure_port(*port, config, *expected)
            .map(ControlReply::State),
        ControlRequest::TableRead { port, offset, len } => device
            .table_read(*port, *offset, *len)
            .map(ControlReply::Table),
        ControlRequest::TableWrite {
            port,
            offset,
            data,
            expected,
        } => device
            .table_write(*port, *offset, data, *expected)
            .map(ControlReply::Committed),
        ControlRequest::Scan { port, cancel } => device
            .scan(*port, || cancel.load(Ordering::Relaxed))
            .map(ControlReply::Scan),
        ControlRequest::Recover(port) => device.recover(*port).map(ControlReply::Recovered),
        ControlRequest::FrontendGet(port) => device.frontend_get(*port).map(ControlReply::Frontend),
        ControlRequest::FrontendSet {
            port,
            config,
            expected,
        } => device
            .frontend_set(*port, config, *expected)
            .map(ControlReply::Frontend),
    }
}

/// Keep the lifecycle guard outside the opener so a rejected Connect cannot
/// claim another interface or send even a PING. The callback allows headless
/// tests of the same path the USB worker uses.
fn connect_bridge<B>(
    bridge: &mut Option<B>,
    serial: &str,
    open: impl FnOnce(&str) -> Result<(B, Connection), String>,
) -> Event {
    if bridge.is_some() {
        return Event::Connected(Err(DISCONNECT_FIRST.into()));
    }
    match open(serial) {
        Ok((b, line)) => {
            *bridge = Some(b);
            Event::Connected(Ok(line))
        }
        Err(e) => Event::Connected(Err(e)),
    }
}

/// Run at most once and discard a poisoned connection before another queued
/// job can use it. A firmware I2C error can still leave the USB bridge usable.
fn transfer_bridge<B>(
    bridge: &mut Option<B>,
    transfer: Transfer,
    run: impl FnOnce(&mut B, &Transfer) -> (Result<Vec<u8>, String>, bool),
) -> Event {
    let (result, usable) = match bridge.as_mut() {
        Some(b) => {
            let (result, usable) = run(b, &transfer);
            let result = if usable {
                result
            } else {
                result.map_err(|e| {
                    let warning = if transfer.wdata.is_empty() {
                        "USB 连接已断开，请重新连接；未自动重试。"
                    } else {
                        "USB 连接已断开，请重新连接。写入结果未知，请确认设备状态后再操作；未自动重试。"
                    };
                    format!("{warning}\n{e}")
                })
            };
            (result, usable)
        }
        None => (Err("未连接，先点连接".into()), false),
    };
    if !usable {
        *bridge = None;
    }
    Event::Transfer {
        transfer,
        result,
        connection_lost: !usable,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn info(firmware: &str) -> Connection {
        Connection {
            firmware: firmware.into(),
            capabilities: Capabilities::legacy(),
            offline: false,
            control_capabilities: Vec::new(),
            port_states: Vec::new(),
        }
    }

    fn write() -> Transfer {
        Transfer {
            port: 0,
            speed_hz: 100_000,
            offline: false,
            op: Op::Write,
            addr: 0x50,
            wdata: vec![0, 0xaa],
            rlen: 0,
        }
    }

    #[test]
    fn connect_cannot_replace_an_open_bridge_even_with_the_same_serial() {
        for serial in ["B", "A"] {
            let mut bridge = Some("A".to_owned());
            let opened = Cell::new(0);
            let event = connect_bridge(&mut bridge, serial, |serial| {
                opened.set(opened.get() + 1);
                Ok((serial.to_owned(), info("new firmware")))
            });
            assert!(matches!(event, Event::Connected(Err(e)) if e == DISCONNECT_FIRST));
            assert_eq!(bridge.as_deref(), Some("A"));
            assert_eq!(opened.get(), 0, "replacement must not open or PING");
        }
    }

    #[test]
    fn connect_after_release_opens_once_and_keeps_ping_result() {
        let mut bridge: Option<String> = None;
        let opened = Cell::new(0);
        let event = connect_bridge(&mut bridge, "A", |serial| {
            opened.set(opened.get() + 1);
            Ok((serial.to_owned(), info("firmware A")))
        });
        assert!(matches!(event, Event::Connected(Ok(line)) if line.firmware == "firmware A"));
        assert_eq!(bridge.as_deref(), Some("A"));
        assert_eq!(opened.get(), 1);
    }

    #[test]
    fn failed_connect_does_not_create_a_session_or_retry() {
        let mut bridge: Option<String> = None;
        let opened = Cell::new(0);
        let event = connect_bridge(&mut bridge, "A", |_| {
            opened.set(opened.get() + 1);
            Err("PING failed".into())
        });
        assert!(matches!(event, Event::Connected(Err(e)) if e == "PING failed"));
        assert!(bridge.is_none());
        assert_eq!(opened.get(), 1);
    }

    #[test]
    fn transport_failure_drops_bridge_and_blocks_the_next_request_without_retry() {
        struct FakeBridge<'a>(&'a Cell<usize>);
        impl Drop for FakeBridge<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let dropped = Cell::new(0);
        let calls = Cell::new(0);
        let mut bridge = Some(FakeBridge(&dropped));
        let event = transfer_bridge(&mut bridge, write(), |_, _| {
            calls.set(calls.get() + 1);
            (Err("USB reply timeout".into()), false)
        });
        let Event::Transfer {
            result: Err(error),
            connection_lost,
            ..
        } = event
        else {
            panic!("expected failed transfer");
        };
        assert!(connection_lost);
        assert!(error.contains("重新连接"));
        assert!(error.contains("写入结果未知"));
        assert!(error.contains("未自动重试"));
        assert!(bridge.is_none());
        assert_eq!(
            dropped.get(),
            1,
            "release the bridge before publishing the event"
        );
        assert_eq!(calls.get(), 1, "never replay an uncertain write");

        let event = transfer_bridge(&mut bridge, write(), |_, _| {
            calls.set(calls.get() + 1);
            (Ok(Vec::new()), true)
        });
        assert!(matches!(
            event,
            Event::Transfer {
                result: Err(_),
                connection_lost: true,
                ..
            }
        ));
        assert_eq!(
            calls.get(),
            1,
            "a queued request must not reach invalid USB"
        );
    }

    #[test]
    fn firmware_i2c_errors_keep_the_bridge_and_are_not_retried() {
        for reason in ["I2C Nack", "I2C Timeout"] {
            let mut bridge = Some(0);
            let event = transfer_bridge(&mut bridge, write(), |calls, _| {
                *calls += 1;
                (Err(reason.into()), true)
            });
            assert!(matches!(event, Event::Transfer {
                result: Err(e), connection_lost: false, ..
            } if e == reason));
            assert_eq!(bridge, Some(1), "keep valid USB without retrying I2C");

            let event = transfer_bridge(&mut bridge, write(), |calls, _| {
                *calls += 1;
                (Ok(Vec::new()), true)
            });
            assert!(matches!(
                event,
                Event::Transfer {
                    result: Ok(_),
                    connection_lost: false,
                    ..
                }
            ));
            assert_eq!(bridge, Some(2), "an explicit later request may proceed");
        }
    }
    #[test]
    fn offline_worker_executes_independent_ports_and_partial_batch_without_usb() {
        let (jobs, events) = spawn(egui::Context::default());
        let receive = || {
            events
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        };
        jobs.send(Job::ConnectOffline).unwrap();
        assert!(
            matches!(receive(), Event::Connected(Ok(info)) if info.offline && info.capabilities.port_count == 2)
        );
        let mut t = write();
        t.offline = true;
        jobs.send(Job::Transfer(t.clone())).unwrap();
        assert!(matches!(
            receive(),
            Event::Transfer {
                result: Ok(_),
                connection_lost: false,
                ..
            }
        ));
        t.op = Op::WriteRead;
        t.wdata = vec![0];
        t.rlen = 1;
        t.port = 1;
        jobs.send(Job::Transfer(t.clone())).unwrap();
        assert!(matches!(receive(), Event::Transfer { result: Ok(d), .. } if d == vec![0]));
        t.port = 0;
        jobs.send(Job::Transfer(t.clone())).unwrap();
        assert!(matches!(receive(), Event::Transfer { result: Ok(d), .. } if d == vec![0xaa]));
        let mut absent = t.clone();
        absent.port = 1;
        absent.addr = 0x51;
        jobs.send(Job::Batch(vec![t, absent])).unwrap();
        match receive() {
            Event::Batch {
                result: Ok(outcomes),
                connection_lost: false,
                ..
            } => {
                assert_eq!(outcomes[0].data, [0xaa]);
                assert_eq!(outcomes[1].status, usb_i2c::proto::Status::Nack);
            }
            _ => panic!("expected independent batch results"),
        }
        jobs.send(Job::Disconnect).unwrap();
        assert!(matches!(receive(), Event::Disconnected));
    }

    #[test]
    fn control_worker_keeps_typed_conflict_and_does_not_retry_or_replace_table() {
        use usb_i2c::control::{ControlStatus, PortRole};
        let (jobs, events) = spawn(egui::Context::default());
        let receive = || {
            events
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        };
        jobs.send(Job::ConnectOffline).unwrap();
        let Event::Connected(Ok(info)) = receive() else {
            panic!("connect")
        };
        assert_eq!(info.port_states.len(), 2);
        let mut config = info.port_states[0].config.clone();
        config.role = PortRole::Slave;
        jobs.send(Job::Control(ControlRequest::Configure {
            port: 0,
            config,
            expected: Some(0),
        }))
        .unwrap();
        assert!(
            matches!(receive(), Event::Control { result: Ok(ControlReply::State(s)), .. }
            if s.config_generation == 1 && s.config.role == PortRole::Slave)
        );
        let write = ControlRequest::TableWrite {
            port: 0,
            offset: 12,
            data: vec![0x12, 0x34],
            expected: Some(0),
        };
        jobs.send(Job::Control(write)).unwrap();
        assert!(
            matches!(receive(), Event::Control { result: Ok(ControlReply::Committed(c)), .. }
            if c.generation == 1)
        );
        jobs.send(Job::Control(ControlRequest::TableWrite {
            port: 0,
            offset: 12,
            data: vec![0xff, 0xff],
            expected: Some(0),
        }))
        .unwrap();
        assert!(
            matches!(receive(), Event::Control { result: Err(e), connection_lost: false, .. }
            if e.status == Some(ControlStatus::Conflict) && !e.result_unknown && !e.session_invalidated)
        );
        jobs.send(Job::Control(ControlRequest::TableRead {
            port: 0,
            offset: 12,
            len: 2,
        }))
        .unwrap();
        assert!(
            matches!(receive(), Event::Control { result: Ok(ControlReply::Table(t)), .. }
            if t.generation == 1 && t.data == [0x12, 0x34])
        );
        jobs.send(Job::Control(ControlRequest::FrontendGet(0)))
            .unwrap();
        assert!(
            matches!(receive(), Event::Control { result: Err(e), connection_lost: false, .. }
            if e.status == Some(ControlStatus::Unsupported) && !e.result_unknown)
        );
        jobs.send(Job::Disconnect).unwrap();
        assert!(matches!(receive(), Event::Disconnected));
    }

    #[test]
    fn scan_cancel_flag_is_observed_without_a_queued_cancel_job() {
        let mut device = Device::offline();
        let cancel = Arc::new(AtomicBool::new(false));
        let request = ControlRequest::Scan {
            port: 0,
            cancel: cancel.clone(),
        };
        // The UI owns the same flag while the worker owns the request. It need
        // not enqueue another job behind the scan it wants to cancel.
        cancel.store(true, Ordering::Relaxed);
        let ControlReply::Scan(result) = run_control(&mut device, &request).unwrap() else {
            panic!("scan result")
        };
        assert!(result.cancelled && !result.complete && result.addresses.is_empty());
        assert!(result.error.is_none() && device.is_usable());
    }

    #[test]
    fn matched_write_error_keeps_unknown_outcome_warning_on_usable_session() {
        let mut bridge = Some(());
        let event = transfer_bridge(&mut bridge, write(), |_, _| {
            (
                Err(transfer_error_text(DeviceError {
                    message: "I2C 0x50: Nack (hpm_stat 2004)".into(),
                    session_invalidated: false,
                    result_unknown: true,
                    status: None,
                    hpm_stat: Some(2004),
                })),
                true,
            )
        });
        let Event::Transfer {
            result: Err(message),
            connection_lost: false,
            ..
        } = event
        else {
            panic!("matched write NACK must keep the session")
        };
        assert!(message.contains("写入结果未知"));
        assert!(message.contains("读回确认"));
        assert!(message.contains("未自动重试"));
        assert!(message.contains("Nack"));
        assert!(bridge.is_some());
    }
}
