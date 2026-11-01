//! Window for the usb_i2c bridge: connect, write, read, write-then-read.
//! The sidebar lists device connection, then I2C / SPI / PWM.
//! This firmware only implements I2C.
//! Scan, bus recovery, and the slave panel are not here; the bridge protocol
//! has no commands for them. The I2C rate is fixed in the firmware (100 kHz).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use usb_i2c::bridge::{list_devices, Bridge, DeviceInfo};
use usb_i2c::proto::MAX_XFER;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1032.0, 640.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "USB-I2C",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))),
    )
}

#[derive(Clone, Copy)]
enum Op {
    Write,
    Read,
    WriteRead,
}

struct Transfer {
    op: Op,
    addr: u8,
    wdata: Vec<u8>,
    rlen: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Device,
    I2c,
    Spi,
    Pwm,
}

impl Page {
    const FUNCS: [Page; 3] = [Page::I2c, Page::Spi, Page::Pwm];

    fn label(self) -> &'static str {
        match self {
            Page::Device => "设备连接",
            Page::I2c => "I2C",
            Page::Spi => "SPI",
            Page::Pwm => "PWM",
        }
    }
}

enum Job {
    Refresh,
    Connect(String),
    Transfer(Transfer),
}

enum Event {
    Devices(Result<Vec<DeviceInfo>, String>),
    Connected(Result<String, String>),
    Transfer {
        op: Op,
        addr: u8,
        nwrite: usize,
        result: Result<Vec<u8>, String>,
    },
}

struct App {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    devices: Vec<DeviceInfo>,
    selected: usize,
    connected: bool,
    firmware: String,
    addr: String,
    write_hex: String,
    read_len: String,
    busy: bool,
    error: bool,
    status: String,
    dump: String,
    page: Page,
    fitted: bool,
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        install_theme(&ctx);
        let (job_tx, job_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        std::thread::spawn(move || worker(job_rx, ev_tx, ctx));
        let app = Self {
            jobs: job_tx,
            events: ev_rx,
            devices: Vec::new(),
            selected: 0,
            connected: false,
            firmware: String::new(),
            addr: "0x50".into(),
            write_hex: String::new(),
            read_len: "16".into(),
            busy: true,
            error: false,
            status: "正在查找设备…".into(),
            dump: String::new(),
            page: Page::Device,
            fitted: false,
        };
        let _ = app.jobs.send(Job::Refresh);
        app
    }

    fn fail(&mut self, msg: String) {
        self.error = true;
        self.status = msg;
    }

    fn start(&mut self, job: Job) {
        self.busy = true;
        self.error = false;
        self.status = "进行中…".into();
        let _ = self.jobs.send(job);
    }

    fn start_transfer(&mut self, op: Op) {
        if self.busy || !self.connected {
            return;
        }
        let addr = match parse_addr(&self.addr) {
            Ok(a) => a,
            Err(e) => return self.fail(e),
        };
        let wdata = match parse_bytes(&self.write_hex) {
            Ok(b) => b,
            Err(e) => return self.fail(e),
        };
        let rlen = match op {
            Op::Write => 0,
            Op::Read | Op::WriteRead => match parse_len(&self.read_len) {
                Ok(n) => n,
                Err(e) => return self.fail(e),
            },
        };
        if matches!(op, Op::Write | Op::WriteRead) && wdata.is_empty() {
            return self.fail("写入数据为空".into());
        }
        self.start(Job::Transfer(Transfer {
            op,
            addr,
            wdata,
            rlen,
        }));
    }

    fn on_event(&mut self, ev: Event) {
        self.busy = false;
        match ev {
            Event::Devices(Ok(devs)) => {
                let keep = self.devices.get(self.selected).map(|d| d.serial.clone());
                self.devices = devs;
                self.selected = keep
                    .and_then(|s| self.devices.iter().position(|d| d.serial == s))
                    .unwrap_or(0);
                if self.connected {
                    if self.status == "进行中…" {
                        self.status = "已连接".into();
                    }
                } else {
                    self.status = if self.devices.is_empty() {
                        "未找到设备".into()
                    } else {
                        "选择设备后点连接".into()
                    };
                    self.error = false;
                }
            }
            Event::Devices(Err(e)) => self.fail(e),
            Event::Connected(Ok(line)) => {
                self.connected = true;
                self.error = false;
                self.firmware = line;
                self.status = "已连接".into();
            }
            Event::Connected(Err(e)) => {
                self.connected = false;
                self.firmware.clear();
                self.fail(e);
            }
            Event::Transfer {
                op,
                addr,
                nwrite,
                result,
            } => match result {
                Ok(data) => {
                    self.error = false;
                    self.dump = hex_dump(&data);
                    self.status = match op {
                        Op::Write => format!("已写入 {nwrite} 字节到 0x{addr:02x}"),
                        Op::Read => format!("读回 {} 字节", data.len()),
                        Op::WriteRead => format!("写后读回 {} 字节", data.len()),
                    };
                }
                Err(e) => {
                    self.dump.clear();
                    self.fail(e);
                }
            },
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
        let bg = tokens(ui.visuals().dark_mode).bg;
        egui::Panel::left("functions")
            .exact_size(212.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(bg)
                    .inner_margin(egui::Margin::symmetric(10, 14)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if nav_item(ui, self.page == Page::Device, Page::Device.label()).clicked() {
                    self.page = Page::Device;
                }
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("功能")
                        .size(11.0)
                        .color(ui.visuals().weak_text_color())
                        .extra_letter_spacing(0.4),
                );
                ui.add_space(4.0);
                for page in Page::FUNCS {
                    if nav_item(ui, self.page == page, page.label()).clicked() {
                        self.page = page;
                    }
                }
            });

        let frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(egui::Margin::same(16));
        egui::CentralPanel::default()
            .frame(frame)
            .show(ui, |ui| match self.page {
                Page::Device => self.device_page(ui),
                Page::I2c => self.i2c_page(ui),
                Page::Spi => soon_page(ui, "SPI"),
                Page::Pwm => soon_page(ui, "PWM"),
            });
    }
}

impl App {
    fn device_page(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("设备连接").size(13.5).strong());
        ui.label(
            egui::RichText::new("选择一块 usb_i2c，再点连接。")
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);

        section(ui, "设备", |ui| {
            ui.horizontal(|ui| {
                if self.devices.is_empty() {
                    ui.label(egui::RichText::new("无").color(ui.visuals().weak_text_color()));
                } else {
                    let t = tokens(ui.visuals().dark_mode);
                    ui.scope(|ui| {
                        let w = &mut ui.visuals_mut().widgets;
                        w.inactive.weak_bg_fill = t.pill;
                        w.hovered.weak_bg_fill = t.pill_hover;
                        w.active.weak_bg_fill = t.pill_hover;
                        w.open.weak_bg_fill = t.pill;
                        egui::ComboBox::from_id_salt("device")
                            .selected_text(device_label(&self.devices[self.selected]))
                            .width(360.0)
                            .show_ui(ui, |ui| {
                                for (i, d) in self.devices.iter().enumerate() {
                                    ui.selectable_value(&mut self.selected, i, device_label(d));
                                }
                            });
                    });
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("刷新"))
                    .clicked()
                {
                    self.start(Job::Refresh);
                }
                let can_connect = !self.busy && !self.devices.is_empty();
                if accent_button(ui, can_connect, "连接").clicked() {
                    let serial = self.devices[self.selected].serial.clone();
                    self.connected = false;
                    self.firmware.clear();
                    self.start(Job::Connect(serial));
                }
                if self.connected {
                    ui.label(&self.firmware);
                } else {
                    ui.label(egui::RichText::new("未连接").color(ui.visuals().weak_text_color()));
                }
            });
            let color = if self.error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(color, &self.status);
        });
    }

    fn i2c_page(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("I2C").size(13.5).strong());
        ui.label(
            egui::RichText::new("速率由固件固定为 100 kHz。单次最多 256 字节。")
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);

        section(ui, "主机", |ui| {
            ui.horizontal(|ui| {
                ui.label("地址");
                ui.add(
                    egui::TextEdit::singleline(&mut self.addr)
                        .hint_text("0x50 或 80")
                        .desired_width(100.0),
                );
                ui.label("读取长度");
                ui.add(
                    egui::TextEdit::singleline(&mut self.read_len)
                        .hint_text("1-256")
                        .desired_width(80.0),
                );
            });
            ui.label("写入数据（十六进制，空格分隔）");
            ui.add(
                egui::TextEdit::multiline(&mut self.write_hex)
                    .hint_text("00 11 22 33")
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            ui.label(
                egui::RichText::new("当前从机把第一个字节当作寄存器表偏移。")
                    .size(11.5)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.horizontal(|ui| {
                let on = self.connected && !self.busy;
                if ui.add_enabled(on, egui::Button::new("写入")).clicked() {
                    self.start_transfer(Op::Write);
                }
                if ui.add_enabled(on, egui::Button::new("读取")).clicked() {
                    self.start_transfer(Op::Read);
                }
                if ui.add_enabled(on, egui::Button::new("写后读")).clicked() {
                    self.start_transfer(Op::WriteRead);
                }
            });
        });

        section(ui, "结果", |ui| {
            let color = if self.error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(color, &self.status);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.dump)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(10)
                        .interactive(false),
                );
            });
        });
    }
}

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
    if rect.width() < 1000.0 {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            1032.0,
            rect.height().max(640.0),
        )));
    }
}

fn nav_item(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let t = tokens(ui.visuals().dark_mode);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    let radius = egui::CornerRadius::same(7);
    if selected {
        let shadow = egui::Shadow {
            offset: [0, 1],
            blur: 4,
            spread: 0,
            color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 70 } else { 22 }),
        };
        ui.painter().add(shadow.as_shape(rect, radius));
        ui.painter().rect_filled(rect, radius, t.card);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, t.pill);
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(12.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(13.0, egui::FontFamily::Proportional),
        t.fg,
    );
    resp
}

fn soon_page(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).size(13.5).strong());
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("当前固件只实现了 I2C，这里还没有命令。")
            .color(ui.visuals().weak_text_color()),
    );
}

#[derive(Clone, Copy)]
struct Tokens {
    bg: egui::Color32,
    card: egui::Color32,
    card2: egui::Color32,
    pill: egui::Color32,
    pill_hover: egui::Color32,
    line: egui::Color32,
    fg: egui::Color32,
    muted: egui::Color32,
    ctl: egui::Color32,
    accent: egui::Color32,
    accent_hover: egui::Color32,
    accent_fg: egui::Color32,
    sel: egui::Color32,
    red: egui::Color32,
    amber: egui::Color32,
}

fn rgb(hex: u32) -> egui::Color32 {
    egui::Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn tokens(dark: bool) -> Tokens {
    if dark {
        Tokens {
            bg: rgb(0x1a1a1e),
            card: rgb(0x232327),
            card2: rgb(0x1f1f23),
            pill: rgb(0x2d2d33),
            pill_hover: rgb(0x36363d),
            line: rgb(0x333339),
            fg: rgb(0xededf1),
            muted: rgb(0x8b8b94),
            ctl: rgb(0xa1a1aa),
            accent: rgb(0xa5b4fc),
            accent_hover: rgb(0xc7d2fe),
            accent_fg: rgb(0x14142a),
            sel: rgb(0x33345a),
            red: rgb(0xf28b82),
            amber: rgb(0xf2b544),
        }
    } else {
        Tokens {
            bg: rgb(0xf4f4f6),
            card: rgb(0xffffff),
            card2: rgb(0xf9f9fb),
            pill: rgb(0xf1f1f4),
            pill_hover: rgb(0xe9e9ee),
            line: rgb(0xe3e3e8),
            fg: rgb(0x1c1c21),
            muted: rgb(0x85858d),
            ctl: rgb(0x63636b),
            accent: rgb(0x4f46e5),
            accent_hover: rgb(0x4338ca),
            accent_fg: egui::Color32::WHITE,
            sel: rgb(0xecebfb),
            red: rgb(0xd64545),
            amber: rgb(0xb45309),
        }
    }
}

fn visuals(dark: bool) -> egui::Visuals {
    let t = tokens(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.override_text_color = None;
    v.weak_text_color = Some(t.muted);
    v.hyperlink_color = t.accent;
    v.faint_bg_color = t.pill;
    v.extreme_bg_color = t.card2;
    v.text_edit_bg_color = Some(t.card2);
    v.code_bg_color = t.card2;
    v.warn_fg_color = t.amber;
    v.error_fg_color = t.red;
    v.window_fill = t.card;
    v.window_stroke = egui::Stroke::new(1.0, t.line);
    v.window_corner_radius = egui::CornerRadius::same(12);
    v.menu_corner_radius = egui::CornerRadius::same(10);
    v.panel_fill = t.bg;
    v.selection.bg_fill = t.sel;
    v.selection.stroke = egui::Stroke::new(1.0, t.accent);
    v.text_cursor.stroke.color = t.accent;
    v.button_frame = true;

    let w = &mut v.widgets;
    w.noninteractive.weak_bg_fill = t.bg;
    w.noninteractive.bg_fill = t.bg;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, t.line);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.noninteractive.corner_radius = egui::CornerRadius::same(8);

    w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    w.inactive.bg_fill = t.pill;
    w.inactive.bg_stroke = egui::Stroke::NONE;
    w.inactive.fg_stroke = egui::Stroke::new(1.0, t.ctl);
    w.inactive.corner_radius = egui::CornerRadius::same(6);

    w.hovered.weak_bg_fill = t.pill;
    w.hovered.bg_fill = t.pill;
    w.hovered.bg_stroke = egui::Stroke::NONE;
    w.hovered.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.hovered.corner_radius = egui::CornerRadius::same(6);

    w.active.weak_bg_fill = t.pill_hover;
    w.active.bg_fill = t.pill_hover;
    w.active.bg_stroke = egui::Stroke::NONE;
    w.active.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.active.corner_radius = egui::CornerRadius::same(6);

    w.open.weak_bg_fill = t.pill;
    w.open.bg_fill = t.card;
    w.open.bg_stroke = egui::Stroke::new(1.0, t.line);
    w.open.fg_stroke = egui::Stroke::new(1.0, t.fg);
    w.open.corner_radius = egui::CornerRadius::same(6);
    v
}

fn install_theme(ctx: &egui::Context) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let dark = theme == egui::Theme::Dark;
        ctx.style_mut_of(theme, |style| {
            style.visuals = visuals(dark);
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(7.0, 3.0);
            style.spacing.interact_size.y = 22.0;
            style.animation_time = 0.15;
            let prop = egui::FontFamily::Proportional;
            style.text_styles.insert(
                egui::TextStyle::Small,
                egui::FontId::new(11.0, prop.clone()),
            );
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::new(13.0, prop.clone()));
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(12.0, prop.clone()),
            );
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::new(13.5, prop));
            style.text_styles.insert(
                egui::TextStyle::Monospace,
                egui::FontId::new(11.5, egui::FontFamily::Monospace),
            );
        });
    }
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(
            egui::RichText::new(title)
                .size(11.0)
                .color(ui.visuals().weak_text_color())
                .extra_letter_spacing(0.4),
        );
        let t = tokens(ui.visuals().dark_mode);
        egui::Frame::new()
            .fill(t.card)
            .stroke(egui::Stroke::new(1.0, t.line))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(12))
            .show(ui, body);
    });
}

fn accent_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    let t = tokens(ui.visuals().dark_mode);
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = t.accent;
        w.hovered.weak_bg_fill = t.accent_hover;
        w.active.weak_bg_fill = t.accent_hover;
        w.inactive.fg_stroke.color = t.accent_fg;
        w.hovered.fg_stroke.color = t.accent_fg;
        w.active.fg_stroke.color = t.accent_fg;
        let text = egui::RichText::new(label).size(12.0).color(t.accent_fg);
        ui.add_enabled(enabled, egui::Button::new(text))
    })
    .inner
}

fn worker(jobs: Receiver<Job>, events: Sender<Event>, ctx: egui::Context) {
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
                    Err(e) => {
                        bridge = None;
                        Event::Connected(Err(e.to_string()))
                    }
                }
            }
            Job::Transfer(t) => {
                let nwrite = t.wdata.len();
                let result = match bridge.as_mut() {
                    Some(b) => b.i2c(t.addr, &t.wdata, t.rlen).map_err(|e| e.to_string()),
                    None => Err("未连接".into()),
                };
                Event::Transfer {
                    op: t.op,
                    addr: t.addr,
                    nwrite,
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

fn device_label(d: &DeviceInfo) -> String {
    let speed = d
        .speed
        .strip_prefix("Some(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(d.speed.as_str());
    format!("{}   USB {speed}", d.serial)
}

fn parse_addr(s: &str) -> Result<u8, String> {
    let s = s.trim();
    let v = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u8::from_str_radix(h, 16).map_err(|e| format!("地址无效：{e}"))?,
        None => s.parse::<u8>().map_err(|e| format!("地址无效：{e}"))?,
    };
    if v > 0x7F {
        return Err(format!("0x{v:02x} 不是 7 位地址"));
    }
    Ok(v)
}

fn parse_bytes(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        let h = tok
            .strip_prefix("0x")
            .or_else(|| tok.strip_prefix("0X"))
            .unwrap_or(tok);
        let b = u8::from_str_radix(h, 16).map_err(|_| format!("无法解析字节 {tok}"))?;
        out.push(b);
    }
    if out.len() > MAX_XFER {
        return Err(format!("单次最多 {MAX_XFER} 字节"));
    }
    Ok(out)
}

fn parse_len(s: &str) -> Result<usize, String> {
    let n: usize = s.trim().parse().map_err(|_| format!("读取长度无效：{s}"))?;
    if n == 0 || n > MAX_XFER {
        return Err(format!("读取长度要在 1 到 {MAX_XFER} 之间"));
    }
    Ok(n)
}

fn hex_dump(data: &[u8]) -> String {
    let mut out = String::new();
    for (i, row) in data.chunks(16).enumerate() {
        let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
        out.push_str(&format!("{:04x}: {}\n", i * 16, hex.join(" ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_matches_cli_rules() {
        assert_eq!(parse_addr("0x50").unwrap(), 0x50);
        assert_eq!(parse_addr("80").unwrap(), 80);
        assert!(parse_addr("0x80").is_err());
        assert!(parse_addr("50").unwrap() != 0x50);
    }

    #[test]
    fn bytes_and_length() {
        assert_eq!(parse_bytes("00 de 0xAD").unwrap(), vec![0x00, 0xde, 0xad]);
        assert!(parse_bytes("  ").unwrap().is_empty());
        assert!(parse_bytes("zz").is_err());
        assert!(parse_bytes(&"00 ".repeat(257)).is_err());
        assert_eq!(parse_len("256").unwrap(), 256);
        assert!(parse_len("0").is_err());
        assert!(parse_len("257").is_err());
    }

    #[test]
    fn dump_rows() {
        let s = hex_dump(&[0x11, 0x22]);
        assert_eq!(s, "0000: 11 22\n");
    }
}
