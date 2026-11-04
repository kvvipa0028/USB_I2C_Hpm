//! USB runs on one worker thread so the window never blocks: the window
//! sends a `Job`, the worker answers with an `Event` and asks for a repaint.
//! Only PING (on connect) and I2C go over the wire; the bridge protocol has
//! no other commands.

use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use usb_i2c::bridge::{list_devices, Bridge, DeviceInfo};

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

#[derive(Debug)]
pub struct Transfer {
    pub op: Op,
    pub addr: u8,
    pub wdata: Vec<u8>,
    pub rlen: usize,
}

pub enum Job {
    Refresh,
    Connect(String),
    /// Host side only: drop the open interface. Nothing is sent to the device.
    Disconnect,
    Transfer(Transfer),
}

pub enum Event {
    Devices(Result<Vec<DeviceInfo>, String>),
    Connected(Result<String, String>),
    Disconnected,
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
    let mut bridge: Option<Bridge> = None;
    while let Ok(job) = jobs.recv() {
        let event = match job {
            Job::Refresh => Event::Devices(list_devices().map_err(|e| e.to_string())),
            Job::Connect(serial) => connect_bridge(&mut bridge, &serial, |serial| {
                Bridge::open_serial(Some(serial))
                    .and_then(|mut b| {
                        let line = b.ping_line()?;
                        Ok((b, line))
                    })
                    .map_err(|e| e.to_string())
            }),
            Job::Disconnect => {
                bridge = None;
                Event::Disconnected
            }
            Job::Transfer(t) => transfer_bridge(&mut bridge, t, |b, t| {
                let result = b.i2c(t.addr, &t.wdata, t.rlen).map_err(|e| e.to_string());
                (result, b.is_usable())
            }),
        };
        if events.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

/// Keep the lifecycle guard outside the opener so a rejected Connect cannot
/// claim another interface or send even a PING. The callback allows headless
/// tests of the same path the USB worker uses.
fn connect_bridge<B>(
    bridge: &mut Option<B>,
    serial: &str,
    open: impl FnOnce(&str) -> Result<(B, String), String>,
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

    fn write() -> Transfer {
        Transfer {
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
                Ok((serial.to_owned(), "new firmware".into()))
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
            Ok((serial.to_owned(), "firmware A".into()))
        });
        assert!(matches!(event, Event::Connected(Ok(line)) if line == "firmware A"));
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
}
