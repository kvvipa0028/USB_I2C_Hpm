//! The window: top device bar, sidebar, page, and a status footer.
//! State lives here; the device and I2C pages return what the user asked
//! for and the app turns it into a worker `Job`. SPI and PWM keep their
//! state in their own pages and never reach the worker.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use eframe::egui;

use crate::device::{self, Action, Conn, Devices, Link};
use crate::i2c::{self, I2c};
use crate::nav::{self, Page};
use crate::pwm::{self, Pwm};
use crate::spi::{self, Spi};
use crate::theme;
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
        self.tone = Tone::Red;
        self.status = msg;
    }

    fn start(&mut self, job: Job) {
        self.busy = true;
        // a refresh while connected leaves the last result in the footer
        if !(matches!(job, Job::Refresh) && self.dev.can_send()) {
            self.tone = Tone::Neutral;
            self.status = match &job {
                Job::Refresh => "正在刷新设备列表…".into(),
                Job::Connect(serial) => format!("正在连接 {serial}…"),
                Job::Disconnect => "正在断开…".into(),
                Job::Transfer(t) => format!("正在{} 0x{:02x}…", t.op.label(), t.addr),
            };
        }
        // the open device, if any, stays until the worker has a new one
        if let Job::Connect(serial) = &job {
            self.connect_failed = false;
            self.connecting = Some(serial.clone());
        }
        if self.jobs.send(job).is_err() {
            self.busy = false;
            self.connecting = None;
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
                let serial = self.connecting.take().unwrap_or_default();
                match result {
                    Ok(firmware) => {
                        self.dev.conn = Some(Conn { serial, firmware });
                        self.tone = Tone::Green;
                        self.status = "已连接".into();
                    }
                    Err(e) => {
                        // the worker kept the device that was open, if any
                        self.connect_failed = self.dev.conn.is_none();
                        self.fail(e);
                    }
                }
            }
            Event::Disconnected => {
                self.dev.conn = None;
                self.tone = Tone::Neutral;
                self.status = "已断开".into();
            }
            Event::Transfer { transfer, result } => {
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

        let link_text = match (&self.dev.conn, link) {
            (Some(c), Link::On | Link::Missing) => format!("{} {}", link.text(), c.serial),
            _ => link.text().to_owned(),
        };
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
                        right: match self.page {
                            Page::Device | Page::I2c => "I2C0 · 100 kHz（固件固定）",
                            Page::Spi => "SPI · 本地会话",
                            Page::Pwm => "PWM · 配置预览",
                        },
                    },
                )
            });

        egui::Panel::left("functions")
            .exact_size(nav::WIDTH)
            .resizable(false)
            .frame(bar(10, 14))
            .show(ui, |ui| {
                nav::sidebar(ui, &mut self.page, link.tone().dot(&t))
            });

        let cx = i2c::Ctx {
            connected: self.dev.can_send(),
            busy: self.busy,
        };
        let now = self.opened.elapsed();
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
                })
            })
            .inner;
        match request {
            Some(Request::Device(a)) => self.act(a),
            Some(Request::I2c(op)) => self.start_transfer(op),
            None => {}
        }
    }
}

enum Request {
    Device(Action),
    I2c(Op),
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
