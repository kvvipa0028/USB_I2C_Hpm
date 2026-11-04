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
        // EEPROM, Flash and UDS step on clicks; only timed demos need frames
        let timed = live
            .iter()
            .any(|p| !matches!(p, Page::Eeprom | Page::Flash | Page::Uds));
        if timed || self.i2c.master.looping() {
            ui.ctx().request_repaint_after(local::TICK);
        }

        let link_text = match (&self.dev.conn, link) {
            (Some(c), Link::On | Link::Missing) => format!("{} {}", link.text(), c.serial),
            _ => link.text().to_owned(),
        };
        let mut right = footer_note(self.page).to_owned();
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
            Some(Request::I2c(op)) => self.start_transfer(op),
            None => {}
        }
    }
}

enum Request {
    Device(Action),
    I2c(Op),
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
            (self.uds.running(), Page::Uds),
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
        Page::Device | Page::I2c => "I2C0 · 100 kHz（固件固定）",
        Page::Spi => "SPI · 本地会话",
        Page::Pwm => "PWM · 配置预览",
        Page::Can => "CAN · 本地演示，未接线",
        Page::Lin => "LIN · 本地演示，未接线",
        Page::Uart => "UART · 本地回显，未接线",
        Page::Gpio => "GPIO · 模拟电平",
        Page::Adc => "AD · 模拟波形",
        Page::Ads1015 => "ADS1015 · 模拟数据，不发 I2C",
        Page::Eeprom => "EEPROM · 演示镜像",
        Page::Flash => "Flash · 演示镜像",
        Page::Uds => "UDS · 演示流程",
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
