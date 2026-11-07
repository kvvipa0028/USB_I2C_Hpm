//! The window: top device bar, sidebar, page, and a status footer.
//! State lives here; the device and I2C pages return what the user asked
//! for and the app turns it into a worker `Job`. Every other page (SPI,
//! PWM, CAN, LIN, UART, GPIO, AD 记录, ADS1015, EEPROM, Flash, UDS) keeps
//! its state in its own module and never reaches the worker; their demo
//! generators tick here each frame, whichever page is open.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use eframe::egui;

use crate::adc::{self, Adc};
use crate::ads1015::{self, Ads1015};
use crate::can::{self, Can};
use crate::device::{self, Action, Conn, Devices, Link};
use crate::eeprom::{self, Eeprom};
use crate::flash::{self, Flash};
use crate::gpio::{self, Gpio};
use crate::i2c::{self, I2c};
use crate::lin::{self, Lin};
use crate::local;
use crate::nav::{self, Page};
use crate::pwm::{self, Pwm};
use crate::spi::{self, Spi};
use crate::theme;
use crate::uart::{self, Uart};
use crate::uds::{self, Uds};
use crate::widgets::{self, Footer, Tone};
use crate::worker::{self, Event, Job, Op};

pub const SIZE: [f32; 2] = [1200.0, 800.0];
pub const MIN_SIZE: [f32; 2] = [900.0, 600.0];

pub struct App {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    dev: Devices,
    i2c: I2c,
    spi: Spi,
    pwm: Pwm,
    local: Local,
    page: Page,
    busy: bool,
    /// Serial being connected to.
    connecting: Option<String>,
    connect_failed: bool,
    status: String,
    tone: Tone,
    opened: Instant,
    fitted: bool,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        theme::install(&ctx);
        crate::fonts::install(&ctx);
        let (jobs, events) = worker::spawn(ctx);
        let app = Self {
            jobs,
            events,
            dev: Devices::default(),
            i2c: I2c::default(),
            spi: Spi::default(),
            pwm: Pwm::default(),
            local: Local::default(),
            page: Page::Device,
            busy: true,
            connecting: None,
            connect_failed: false,
            status: "正在查找设备…".into(),
            tone: Tone::Neutral,
            opened: Instant::now(),
            fitted: false,
        };
        let _ = app.jobs.send(Job::Refresh);
        app
    }

    fn link(&self) -> Link {
        self.dev
            .link(self.connecting.is_some(), self.connect_failed)
    }

    fn fail(&mut self, msg: String) {
        self.i2c.master.stop_loop("操作失败，循环已停止");
        self.tone = Tone::Red;
        self.status = msg;
    }

    fn start(&mut self, job: Job) {
        if matches!(
            job,
            Job::Connect(_) | Job::ConnectOffline | Job::Disconnect | Job::Batch(_)
        ) {
            self.i2c.master.stop_loop("连接操作，循环已停止");
        }
        // Enforce the same single-session rule as the controls and worker.
        // A repeated Connect must not replace even the same physical bridge.
        if matches!(job, Job::Connect(_) | Job::ConnectOffline) && self.dev.conn.is_some() {
            self.fail(worker::DISCONNECT_FIRST.into());
            return;
        }
        self.busy = true;
        // a refresh while connected leaves the last result in the footer
        if !(matches!(job, Job::Refresh) && self.dev.can_send()) {
            self.tone = Tone::Neutral;
            self.status = match &job {
                Job::Refresh => "正在刷新设备列表…".into(),
                Job::Connect(serial) => format!("正在连接 {serial}…"),
                Job::ConnectOffline => "正在连接离线模型…".into(),
                Job::Disconnect => "正在断开…".into(),
                Job::Batch(_) => "正在执行 CH0/CH1 批次…".into(),
                Job::Transfer(t) => format!("CH{} 正在{} 0x{:02x}…", t.port, t.op.label(), t.addr),
            };
        }
        match &job {
            Job::Connect(serial) => {
                self.connect_failed = false;
                self.connecting = Some(serial.clone());
            }
            Job::ConnectOffline => {
                self.connect_failed = false;
                self.connecting = Some("OFFLINE-MODEL".into());
            }
            _ => {}
        }
        if self.jobs.send(job).is_err() {
            self.busy = false;
            self.connecting = None;
            self.dev.conn = None;
            self.connect_failed = true;
            self.fail("USB worker 已关闭，需重开窗口".into());
        }
    }

    fn act(&mut self, action: Action) {
        if self.busy {
            return;
        }
        match action {
            Action::Refresh => self.start(Job::Refresh),
            Action::Connect(serial) => self.start(Job::Connect(serial)),
            Action::ConnectOffline => self.start(Job::ConnectOffline),
            Action::Disconnect => self.start(Job::Disconnect),
        }
    }

    fn start_transfer(&mut self, op: Op) {
        if self.busy || !self.dev.can_send() {
            return;
        }
        match self.i2c.master.transfer(op) {
            Ok(t) => self.start(Job::Transfer(t)),
            Err(e) => self.fail(e),
        }
    }

    fn on_event(&mut self, ev: Event) {
        self.busy = false;
        match ev {
            Event::Devices(Ok(devs)) => {
                // `start` left the footer alone only if it was listed before
                let was_listed = self.dev.can_send();
                self.dev.set_list(devs);
                self.connect_failed = false;
                if self.dev.can_send() {
                    if !was_listed {
                        self.tone = Tone::Neutral;
                        self.status = "已连接".into();
                    }
                } else if self.dev.can_disconnect() {
                    self.i2c.master.stop_loop("设备已断开，循环已停止");
                    self.fail("设备不在列表，先点断开".into());
                } else {
                    self.tone = Tone::Neutral;
                    self.status = if self.dev.list.is_empty() {
                        "未找到设备".into()
                    } else {
                        "选择设备后点连接".into()
                    };
                }
            }
            Event::Devices(Err(e)) => self.fail(e),
            Event::Connected(result) => {
                self.i2c.master.stop_loop("连接操作，循环已停止");
                let Some(serial) = self.connecting.take() else {
                    return;
                };
                match result {
                    Ok(info) => {
                        self.i2c.set_connection(&info.capabilities, info.offline);
                        self.dev.conn = Some(Conn {
                            serial,
                            firmware: info.firmware,
                            capabilities: info.capabilities,
                            offline: info.offline,
                        });
                        self.tone = Tone::Green;
                        self.status = "已连接".into();
                    }
                    Err(e) => {
                        self.connect_failed = self.dev.conn.is_none();
                        self.fail(e);
                    }
                }
            }
            Event::Disconnected => {
                self.i2c.master.stop_loop("设备已断开，循环已停止");
                self.dev.conn = None;
                self.connecting = None;
                self.connect_failed = false;
                self.tone = Tone::Neutral;
                self.status = "已断开".into();
            }
            Event::Batch {
                transfers,
                result,
                connection_lost,
            } => {
                if connection_lost {
                    self.dev.conn = None;
                    self.connecting = None;
                    self.connect_failed = true;
                }
                self.i2c.master.stop_loop("批次已完成，循环保持停止");
                self.i2c.batch.result.clear();
                match result {
                    Ok(outcomes) => {
                        let mut failed = false;
                        for (transfer, outcome) in transfers.into_iter().zip(outcomes) {
                            let result = if outcome.status == usb_i2c::proto::Status::Ok {
                                Ok(outcome.data)
                            } else {
                                failed = true;
                                Err(format!(
                                    "CH{}: {:?} (hpm_stat {})",
                                    outcome.port, outcome.status, outcome.hpm_stat
                                ))
                            };
                            let data = result
                                .as_ref()
                                .ok()
                                .map(|d| i2c::parse::to_hex(d))
                                .unwrap_or_default();
                            let record =
                                self.i2c
                                    .master
                                    .record(self.opened.elapsed(), transfer, result);
                            self.i2c
                                .batch
                                .result
                                .push(format!("{} {}", record.summary(), data));
                        }
                        self.tone = if failed { Tone::Red } else { Tone::Green };
                        self.status = if failed {
                            "批次已返回：部分端口失败，未自动重试"
                        } else {
                            "双端口批次完成"
                        }
                        .into();
                    }
                    Err(e) => {
                        let message =
                            if connection_lost && transfers.iter().any(|t| !t.wdata.is_empty()) {
                                format!("连接已失效，写入结果未知，未自动重试。{e}")
                            } else {
                                e
                            };
                        for transfer in transfers {
                            self.i2c.master.record(
                                self.opened.elapsed(),
                                transfer,
                                Err(message.clone()),
                            );
                        }
                        self.i2c.batch.result.push(message.clone());
                        self.fail(message);
                    }
                }
            }
            Event::Transfer {
                transfer,
                result,
                connection_lost,
            } => {
                if connection_lost {
                    self.i2c.master.stop_loop("设备已断开，循环已停止");
                    self.dev.conn = None;
                    self.connecting = None;
                    self.connect_failed = true;
                }
                let rec = self
                    .i2c
                    .master
                    .record(self.opened.elapsed(), transfer, result);
                self.tone = if rec.ok() { Tone::Green } else { Tone::Red };
                self.status = rec.summary();
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.events.try_recv() {
            self.on_event(ev);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        fit_window(ui.ctx(), &mut self.fitted);
        let t = theme::of(ui);
        let link = self.link();
        let bar = |x: i8, y: i8| {
            egui::Frame::new()
                .fill(t.bg)
                .inner_margin(egui::Margin::symmetric(x, y))
        };

        let action = egui::Panel::top("device_bar")
            .exact_size(48.0)
            .resizable(false)
            .frame(bar(14, 0))
            .show(ui, |ui| device::top_bar(ui, &mut self.dev, self.busy, link))
            .inner;
        if let Some(a) = action {
            self.act(a);
        }

        let now = self.opened.elapsed();
        if let Some(op) = self
            .i2c
            .master
            .loop_due(now, self.dev.can_send(), self.busy)
        {
            self.start_transfer(op);
        }
        let mut live = self.local.tick(now);
        if self.spi.tick(now) {
            live.push(Page::Spi);
        }
        let pwm_timed = self.pwm.tick(now);
        if self.pwm.running() {
            live.push(Page::Pwm);
        }
        // EEPROM, Flash and the UDS demo step on clicks; only timed demos and
        // the UDS simulated run need frames
        let timed = live
            .iter()
            .any(|p| !matches!(p, Page::Eeprom | Page::Flash | Page::Uds | Page::Pwm))
            || self.local.uds.timed();
        if timed || pwm_timed || self.i2c.master.looping() {
            ui.ctx().request_repaint_after(local::TICK);
        }

        let link_text = match (&self.dev.conn, link) {
            (Some(c), Link::On | Link::Missing) => format!("{} {}", link.text(), c.serial),
            _ => link.text().to_owned(),
        };
        let mut right = footer_note(self.page).to_owned();
        if matches!(self.page, Page::Device | Page::I2c) {
            if let Some(c) = &self.dev.conn {
                right = if c.offline {
                    "离线模型 · 未发 USB · 时序未验证".into()
                } else {
                    format!("USB · {} 路 · 能力按设备报告", c.capabilities.port_count)
                };
            }
        }
        if !live.is_empty() {
            let names: Vec<&str> = live.iter().map(|p| p.label()).collect();
            right = format!("{right} · 演示运行中：{}", names.join("、"));
        }
        egui::Panel::bottom("status_bar")
            .exact_size(28.0)
            .resizable(false)
            .frame(bar(16, 0))
            .show(ui, |ui| {
                widgets::footer(
                    ui,
                    &Footer {
                        link: link.tone(),
                        link_text: &link_text,
                        busy: self.busy,
                        result: Some((&self.status, self.tone)),
                        right: &right,
                    },
                )
            });

        egui::Panel::left("functions")
            .exact_size(nav::WIDTH)
            .resizable(false)
            .frame(bar(10, 14))
            .show(ui, |ui| {
                nav::sidebar(ui, &mut self.page, link.tone().dot(&t), &live)
            });

        let cx = i2c::Ctx {
            connected: self.dev.can_send(),
            busy: self.busy,
            now,
        };
        let l = &mut self.local;
        let request = egui::CentralPanel::default()
            .frame(bar(20, 16))
            .show(ui, |ui| {
                widgets::page_width(ui, |ui| match self.page {
                    Page::Device => device::page(
                        ui,
                        &mut self.dev,
                        self.busy,
                        link,
                        (&self.status, self.tone),
                    )
                    .map(Request::Device),
                    Page::I2c => i2c::page(ui, &mut self.i2c, cx).map(Request::I2c),
                    Page::Spi => {
                        spi::page(ui, &mut self.spi, now);
                        None
                    }
                    Page::Pwm => {
                        pwm::page(ui, &mut self.pwm, now);
                        None
                    }
                    Page::Can => {
                        can::page(ui, &mut l.can, now);
                        None
                    }
                    Page::Lin => {
                        lin::page(ui, &mut l.lin, now);
                        None
                    }
                    Page::Uart => {
                        uart::page(ui, &mut l.uart, now);
                        None
                    }
                    Page::Gpio => {
                        gpio::page(ui, &mut l.gpio, now);
                        None
                    }
                    Page::Adc => {
                        adc::page(ui, &mut l.adc, now);
                        None
                    }
                    Page::Ads1015 => {
                        ads1015::page(ui, &mut l.ads1015, now);
                        None
                    }
                    Page::Eeprom => {
                        eeprom::page(ui, &mut l.eeprom, now);
                        None
                    }
                    Page::Flash => {
                        flash::page(ui, &mut l.flash, now);
                        None
                    }
                    Page::Uds => {
                        uds::page(ui, &mut l.uds, now);
                        None
                    }
                })
            })
            .inner;
        match request {
            Some(Request::Device(a)) => self.act(a),
            Some(Request::I2c(i2c::Action::Transfer(op))) => self.start_transfer(op),
            Some(Request::I2c(i2c::Action::Batch(transfers)))
                if !self.busy && self.dev.can_send() =>
            {
                self.start(Job::Batch(transfers));
            }
            None | Some(Request::I2c(i2c::Action::Batch(_))) => {}
        }
    }
}

enum Request {
    Device(Action),
    I2c(i2c::Action),
}

/// The pages that run in the window only and never reach the worker.
#[derive(Default)]
struct Local {
    can: Can,
    lin: Lin,
    uart: Uart,
    gpio: Gpio,
    adc: Adc,
    ads1015: Ads1015,
    eeprom: Eeprom,
    flash: Flash,
    uds: Uds,
}

impl Local {
    /// Advance every demo generator to `now`; returns the pages still running.
    fn tick(&mut self, now: std::time::Duration) -> Vec<Page> {
        let mut live = Vec::new();
        for (running, page) in [
            (self.can.tick(now), Page::Can),
            (self.lin.tick(now), Page::Lin),
            (self.uart.tick(now), Page::Uart),
            (self.adc.tick(now), Page::Adc),
            (self.ads1015.tick(now), Page::Ads1015),
            (self.eeprom.running(), Page::Eeprom),
            (self.flash.running(), Page::Flash),
            (self.uds.tick(now), Page::Uds),
        ] {
            if running {
                live.push(page);
            }
        }
        live
    }
}

/// Right end of the footer: what the open page is wired to.
fn footer_note(page: Page) -> &'static str {
    match page {
        Page::Device | Page::I2c => "连接后按设备能力开放控制",
        Page::Spi => "SPI · 本地会话",
        Page::Pwm => "PWM / 捕获 · 本地演示，不发 USB",
        Page::Can => "CAN · 本地演示，未接线",
        Page::Lin => "LIN · 本地演示，未接线",
        Page::Uart => "UART · 本地回显，未接线",
        Page::Gpio => "GPIO · 模拟电平",
        Page::Adc => "AD · 模拟波形",
        Page::Ads1015 => "ADS1015 · 模拟数据，不发 I2C",
        Page::Eeprom => "EEPROM · 演示镜像",
        Page::Flash => "Flash · 演示镜像",
        Page::Uds => "UDS · 页面演示 / 模拟执行，不经 USB",
    }
}

/// Some window managers ignore the initial size; widen the first frame if so.
fn fit_window(ctx: &egui::Context, fitted: &mut bool) {
    if *fitted {
        return;
    }
    let Some(rect) = ctx.input(|i| i.viewport().inner_rect) else {
        return;
    };
    if rect.width() <= 0.0 {
        return;
    }
    *fitted = true;
    if rect.width() < SIZE[0] - 100.0 {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            SIZE[0],
            rect.height().max(MIN_SIZE[1] + 120.0),
        )));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use usb_i2c::bridge::DeviceInfo;

    use super::*;

    fn connection(firmware: &str) -> worker::Connection {
        worker::Connection {
            firmware: firmware.into(),
            capabilities: usb_i2c::proto::Capabilities::legacy(),
            offline: false,
        }
    }

    fn device(serial: &str) -> DeviceInfo {
        DeviceInfo {
            serial: serial.into(),
            speed: "High".into(),
        }
    }

    /// No worker thread or USB access: inspect exactly what the App queues.
    fn connected_app() -> (App, Receiver<Job>) {
        let (jobs, requests) = mpsc::channel();
        let (_, events) = mpsc::channel();
        let mut dev = Devices::default();
        dev.set_list(vec![device("A"), device("B")]);
        dev.conn = Some(Conn {
            serial: "A".into(),
            firmware: "firmware A".into(),
            capabilities: usb_i2c::proto::Capabilities::legacy(),
            offline: false,
        });
        let mut app = App {
            jobs,
            events,
            dev,
            i2c: I2c::default(),
            spi: Spi::default(),
            pwm: Pwm::default(),
            local: Local::default(),
            page: Page::Device,
            busy: false,
            connecting: None,
            connect_failed: false,
            status: String::new(),
            tone: Tone::Neutral,
            opened: Instant::now(),
            fitted: true,
        };
        app.i2c.master.arm_test_write_loop();
        (app, requests)
    }

    fn poll_loop(app: &mut App) {
        if let Some(op) =
            app.i2c
                .master
                .loop_due(Duration::from_secs(60), app.dev.can_send(), app.busy)
        {
            app.start_transfer(op);
        }
    }

    fn reconnect_a(app: &mut App, requests: &Receiver<Job>) {
        app.act(Action::Connect("A".into()));
        assert!(matches!(requests.try_recv(), Ok(Job::Connect(s)) if s == "A"));
        app.on_event(Event::Connected(Ok(connection("firmware A again"))));
        assert!(app.dev.can_send());
        assert_eq!(app.dev.conn.as_ref().unwrap().serial, "A");
        assert!(!app.i2c.master.looping());
        poll_loop(app);
        assert!(
            requests.try_recv().is_err(),
            "reconnect must not replay old writes"
        );
    }

    #[test]
    fn connect_action_refuses_replacement_and_same_serial_reopen() {
        for serial in ["B", "A"] {
            let (mut app, requests) = connected_app();
            app.act(Action::Connect(serial.into()));
            assert!(requests.try_recv().is_err());
            let conn = app.dev.conn.as_ref().unwrap();
            assert_eq!(conn.serial, "A");
            assert_eq!(conn.firmware, "firmware A");
            assert!(!app.busy);
            assert!(app.connecting.is_none());
            assert_eq!(app.status, worker::DISCONNECT_FIRST);
            assert!(!app.i2c.master.looping());
            poll_loop(&mut app);
            assert!(requests.try_recv().is_err());
        }
    }

    #[test]
    fn disconnect_action_stops_loop_immediately_and_reconnect_stays_disarmed() {
        let (mut app, requests) = connected_app();
        app.act(Action::Disconnect);
        assert!(matches!(requests.try_recv(), Ok(Job::Disconnect)));
        assert!(
            !app.i2c.master.looping(),
            "do not wait for an event or UI frame"
        );
        app.on_event(Event::Disconnected);
        assert!(!app.dev.can_send());
        // Reconnect immediately, with no disconnected UI frame in between.
        reconnect_a(&mut app, &requests);
    }

    #[test]
    fn disconnect_event_stops_loop_without_waiting_for_a_frame() {
        let (mut app, requests) = connected_app();
        app.on_event(Event::Disconnected);
        assert!(!app.i2c.master.looping());
        assert!(app.dev.conn.is_none());
        reconnect_a(&mut app, &requests);
    }

    #[test]
    fn connected_event_disarms_any_loop_enabled_during_connection() {
        let (mut app, requests) = connected_app();
        app.dev.conn = None;
        app.act(Action::Connect("A".into()));
        assert!(matches!(requests.try_recv(), Ok(Job::Connect(_))));
        app.i2c.master.arm_test_write_loop();
        app.on_event(Event::Connected(Ok(connection("firmware A"))));
        assert!(!app.i2c.master.looping());
        poll_loop(&mut app);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn unsolicited_connected_event_does_not_replace_active_bridge() {
        let (mut app, _) = connected_app();
        app.on_event(Event::Connected(Ok(connection("other firmware"))));
        let conn = app.dev.conn.as_ref().unwrap();
        assert_eq!(conn.serial, "A");
        assert_eq!(conn.firmware, "firmware A");
        assert!(!app.i2c.master.looping());
    }

    #[test]
    fn transport_failure_clears_connection_blocks_requests_and_never_retries() {
        let (mut app, requests) = connected_app();
        let transfer = app.i2c.master.transfer(Op::Write).unwrap();
        app.on_event(Event::Transfer {
            transfer,
            result: Err("USB reply timeout; reconnect required; write outcome unknown".into()),
            connection_lost: true,
        });
        assert!(app.dev.conn.is_none());
        assert!(!app.dev.can_send());
        assert!(!app.i2c.master.looping());
        assert_eq!(app.link(), Link::Failed);
        assert!(app.status.contains("write outcome unknown"));
        app.start_transfer(Op::Write);
        poll_loop(&mut app);
        assert!(requests.try_recv().is_err());
        reconnect_a(&mut app, &requests);
    }

    #[test]
    fn firmware_i2c_errors_stop_loop_but_allow_explicit_next_transfer() {
        for reason in ["I2C 0x50: Nack", "I2C 0x50: Timeout"] {
            let (mut app, requests) = connected_app();
            let transfer = app.i2c.master.transfer(Op::Write).unwrap();
            app.on_event(Event::Transfer {
                transfer,
                result: Err(reason.into()),
                connection_lost: false,
            });
            assert!(app.dev.can_send());
            assert_eq!(app.dev.conn.as_ref().unwrap().serial, "A");
            assert!(!app.i2c.master.looping());
            assert_eq!(app.link(), Link::On);
            poll_loop(&mut app);
            assert!(
                requests.try_recv().is_err(),
                "do not automatically retry an I2C error"
            );
            app.start_transfer(Op::Write);
            assert!(matches!(requests.try_recv(), Ok(Job::Transfer(_))));
            assert!(requests.try_recv().is_err());
        }
    }

    #[test]
    fn successful_transfer_preserves_the_requested_loop() {
        let (mut app, requests) = connected_app();
        let transfer = app.i2c.master.transfer(Op::Write).unwrap();
        app.on_event(Event::Transfer {
            transfer,
            result: Ok(Vec::new()),
            connection_lost: false,
        });
        assert!(app.dev.can_send());
        assert!(app.i2c.master.looping());
        poll_loop(&mut app);
        let Ok(Job::Transfer(transfer)) = requests.try_recv() else {
            panic!("the next requested loop transfer should be queued");
        };
        assert_eq!(transfer.op, Op::Write);
        assert_eq!(transfer.wdata, [0, 0xaa]);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn missing_then_returning_device_does_not_resume_loop_between_frames() {
        let (mut app, requests) = connected_app();
        app.on_event(Event::Devices(Ok(vec![device("B")])));
        assert!(!app.i2c.master.looping());
        assert!(!app.dev.can_send());
        assert_eq!(app.dev.connect_target(), None);
        app.on_event(Event::Devices(Ok(vec![device("A"), device("B")])));
        assert!(app.dev.can_send());
        poll_loop(&mut app);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn failed_connect_or_refresh_disarms_loop_without_retry() {
        let (mut app, requests) = connected_app();
        app.dev.conn = None;
        app.act(Action::Connect("A".into()));
        assert!(matches!(requests.try_recv(), Ok(Job::Connect(_))));
        app.i2c.master.arm_test_write_loop();
        app.on_event(Event::Connected(Err("PING failed".into())));
        assert!(!app.i2c.master.looping());
        assert!(app.dev.conn.is_none());
        assert_eq!(app.link(), Link::Failed);
        assert!(requests.try_recv().is_err());

        let (mut app, requests) = connected_app();
        app.on_event(Event::Devices(Err("USB enumeration failed".into())));
        assert!(!app.i2c.master.looping());
        assert!(
            app.dev.can_send(),
            "enumeration failure alone does not poison USB"
        );
        poll_loop(&mut app);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn closed_worker_clears_connection_and_loop() {
        let (mut app, requests) = connected_app();
        drop(requests);
        app.start_transfer(Op::Write);
        assert!(!app.busy);
        assert!(app.dev.conn.is_none());
        assert!(!app.i2c.master.looping());
        assert_eq!(app.link(), Link::Failed);
    }
    #[test]
    fn offline_connect_requires_release_and_receives_capabilities_without_enumeration() {
        let (mut app, requests) = connected_app();
        app.act(Action::ConnectOffline);
        assert!(requests.try_recv().is_err());
        assert_eq!(app.dev.conn.as_ref().unwrap().serial, "A");
        app.on_event(Event::Disconnected);
        app.dev.set_list(Vec::new());
        app.act(Action::ConnectOffline);
        assert!(matches!(requests.try_recv(), Ok(Job::ConnectOffline)));
        let device = usb_i2c::device::Device::offline();
        app.on_event(Event::Connected(Ok(worker::Connection {
            firmware: "offline model".into(),
            capabilities: device.capabilities().clone(),
            offline: true,
        })));
        assert!(app.dev.can_send());
        app.start_transfer(Op::Read);
        assert!(matches!(requests.try_recv(), Ok(Job::Transfer(t)) if t.offline));
    }

    #[test]
    fn batch_transport_loss_stops_loop_marks_write_unknown_and_never_replays() {
        let (mut app, requests) = connected_app();
        let transfer = app.i2c.master.transfer(Op::Write).unwrap();
        app.on_event(Event::Batch {
            transfers: vec![transfer],
            result: Err("USB timeout".into()),
            connection_lost: true,
        });
        assert!(app.dev.conn.is_none());
        assert!(!app.i2c.master.looping());
        assert!(app.status.contains("写入结果未知"));
        assert!(app.i2c.batch.result[0].contains("未自动重试"));
        poll_loop(&mut app);
        assert!(requests.try_recv().is_err());
        reconnect_a(&mut app, &requests);
    }
}
