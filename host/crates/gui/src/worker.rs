//! USB runs on one worker thread so the window never blocks: the window
//! sends a `Job`, the worker answers with an `Event` and asks for a repaint.
//! Only PING (on connect) and I2C go over the wire; the bridge protocol has
//! no other commands.

use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use usb_i2c::bridge::{list_devices, Bridge, DeviceInfo};

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
            Job::Connect(serial) => {
                match Bridge::open_serial(Some(serial.as_str())).and_then(|mut b| {
                    let line = b.ping_line()?;
                    Ok((b, line))
                }) {
                    Ok((b, line)) => {
                        bridge = Some(b);
                        Event::Connected(Ok(line))
                    }
                    // the device that was open, if any, stays open
                    Err(e) => Event::Connected(Err(e.to_string())),
                }
            }
            Job::Disconnect => {
                bridge = None;
                Event::Disconnected
            }
            Job::Transfer(t) => {
                let result = match bridge.as_mut() {
                    Some(b) => b.i2c(t.addr, &t.wdata, t.rlen).map_err(|e| e.to_string()),
                    None => Err("未连接".into()),
                };
                Event::Transfer {
                    transfer: t,
                    result,
                }
            }
        };
        if events.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}
