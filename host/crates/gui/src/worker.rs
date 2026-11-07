//! USB runs on one worker thread so the window never blocks: the window
//! sends a `Job`, the worker answers with an `Event` and asks for a repaint.
//! The same jobs use either an explicitly selected offline model or USB.
//! Device capabilities gate port and speed selection; requests are never replayed.

use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use usb_i2c::bridge::{list_devices, DeviceInfo};
use usb_i2c::device::{BackendKind, Device};
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
}

pub enum Job {
    Refresh,
    Connect(String),
    ConnectOffline,
    /// Host side only: drop the open interface. Nothing is sent to the device.
    Disconnect,
    Transfer(Transfer),
    Batch(Vec<Transfer>),
}

pub enum Event {
    Devices(Result<Vec<DeviceInfo>, String>),
    Connected(Result<Connection, String>),
    Disconnected,
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
                    .map_err(|e| e.to_string());
                (result, b.is_usable())
            }),
        };
        if events.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn connection_info(mut device: Device) -> Result<(Device, Connection), String> {
    let info = Connection {
        firmware: device.ping_line().map_err(|e| e.to_string())?,
        capabilities: device.capabilities().clone(),
        offline: device.backend() == BackendKind::Offline,
    };
    Ok((device, info))
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
}
