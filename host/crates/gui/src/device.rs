//! Device strip in the top bar and the connection page. Connecting opens the
//! bridge and sends one PING, whose reply carries the firmware id.

use eframe::egui;
use usb_i2c::bridge::DeviceInfo;

use crate::nav;
use crate::theme;
use crate::widgets::{self, Tone};

pub struct Conn {
    pub serial: String,
    /// The PING line, e.g. `protocol 1 firmware "usb_i2c 0.1"`.
    pub firmware: String,
}

#[derive(Default)]
pub struct Devices {
    pub list: Vec<DeviceInfo>,
    pub selected: usize,
    pub conn: Option<Conn>,
}

impl Devices {
    pub fn selected_serial(&self) -> Option<&str> {
        self.list.get(self.selected).map(|d| d.serial.as_str())
    }

    /// Replace the list, keeping the selection on the same serial.
    pub fn set_list(&mut self, devs: Vec<DeviceInfo>) {
        let keep = self.selected_serial().map(str::to_owned);
        self.list = devs;
        self.selected = keep
            .and_then(|s| self.list.iter().position(|d| d.serial == s))
            .unwrap_or(0);
    }

    fn connected_to_selected(&self) -> bool {
        match (&self.conn, self.selected_serial()) {
            (Some(c), Some(s)) => c.serial == s,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Off,
    Connecting,
    On,
    Failed,
}

impl Link {
    pub fn tone(self) -> Tone {
        match self {
            Link::Off => Tone::Neutral,
            Link::Connecting => Tone::Amber,
            Link::On => Tone::Green,
            Link::Failed => Tone::Red,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Link::Off => "未连接",
            Link::Connecting => "正在连接…",
            Link::On => "已连接",
            Link::Failed => "连接失败",
        }
    }
}

pub enum Action {
    Refresh,
    Connect(String),
    Disconnect,
}

/// Top bar: brand, the device pop-up field, firmware, refresh and connect.
pub fn top_bar(ui: &mut egui::Ui, dev: &mut Devices, busy: bool, link: Link) -> Option<Action> {
    let t = theme::of(ui);
    let mut action = None;
    ui.horizontal_centered(|ui| {
        // as wide as the sidebar, so the device strip starts where the page does
        let brand_w = nav::WIDTH - 14.0 - ui.spacing().item_spacing.x;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(brand_w, 28.0), egui::Sense::hover());
        let r = ui.painter().text(
            rect.left_center() + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "USB-I2C",
            egui::FontId::proportional(14.0),
            t.fg,
        );
        ui.painter().text(
            r.right_center() + egui::vec2(7.0, 1.0),
            egui::Align2::LEFT_CENTER,
            "桥接上位机",
            egui::FontId::proportional(11.0),
            t.muted,
        );

        let value = match dev.list.get(dev.selected) {
            Some(d) => device_label(d),
            None => "未找到设备".to_owned(),
        };
        let resp = widgets::popup_field(ui, "设备", &value, Some(link.tone().dot(&t)), 300.0)
            .on_hover_text(link.text());
        let width = resp.rect.width();
        egui::Popup::menu(&resp).width(width).show(|ui| {
            // as wide as the field, less the menu's own margin
            ui.set_min_width(width - 12.0);
            device_menu(ui, dev);
        });

        if let Some(c) = &dev.conn {
            let (proto, fw) = parse_ping(&c.firmware).unwrap_or(("?", &c.firmware));
            widgets::badge(ui, &format!("固件 {fw} · 协议 {proto}"), Tone::Neutral)
                .on_hover_text(&c.firmware);
        }

        widgets::trailing(ui, 30.0, |ui| {
            if let Some(a) = connect_button(ui, dev, busy) {
                action = Some(a);
            }
            if widgets::quiet_button(ui, !busy, "刷新").clicked() {
                action = Some(Action::Refresh);
            }
        });
    });
    action
}

fn device_menu(ui: &mut egui::Ui, dev: &mut Devices) {
    if dev.list.is_empty() {
        ui.label(egui::RichText::new("未找到设备").color(ui.visuals().weak_text_color()));
        return;
    }
    for i in 0..dev.list.len() {
        let label = device_label(&dev.list[i]);
        ui.selectable_value(&mut dev.selected, i, label);
    }
}

/// 断开 when the selected device is the open one, otherwise 连接.
fn connect_button(ui: &mut egui::Ui, dev: &Devices, busy: bool) -> Option<Action> {
    if dev.connected_to_selected() {
        widgets::action_button(ui, !busy, "断开")
            .clicked()
            .then_some(Action::Disconnect)
    } else {
        let serial = dev.selected_serial()?;
        widgets::ink_button(ui, !busy, "连接")
            .clicked()
            .then(|| Action::Connect(serial.to_owned()))
    }
}

/// Connection page: the device list, then what the connection reports.
pub fn page(
    ui: &mut egui::Ui,
    dev: &mut Devices,
    busy: bool,
    link: Link,
    status: (&str, Tone),
) -> Option<Action> {
    let t = theme::of(ui);
    let mut action = None;
    widgets::page_head(
        ui,
        "设备连接",
        Some((link.text(), link.tone())),
        "选择一块 usb_i2c，再点连接。连接时发一次 PING，读回固件版本。",
        |ui| {
            if widgets::action_button(ui, !busy, "刷新").clicked() {
                action = Some(Action::Refresh);
            }
        },
    );

    widgets::section(ui, "设备", |ui| {
        if dev.list.is_empty() {
            ui.label(egui::RichText::new("未找到设备").strong());
            ui.label(
                egui::RichText::new("插上 usb_i2c（VID 1209，PID 0001）后点刷新。").color(t.muted),
            );
        } else {
            ui.spacing_mut().item_spacing.y = 2.0;
            let open = dev.conn.as_ref().map(|c| c.serial.clone());
            for i in 0..dev.list.len() {
                let here = open.as_deref() == Some(dev.list[i].serial.as_str());
                if device_row(ui, &dev.list[i], i == dev.selected, here).clicked() {
                    dev.selected = i;
                }
            }
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            if let Some(a) = connect_button(ui, dev, busy) {
                action = Some(a);
            }
            let (fg, _) = status.1.colors(&t);
            let color = if status.1 == Tone::Neutral { t.fg2 } else { fg };
            ui.label(egui::RichText::new(status.0).size(12.0).color(color));
        });
    });

    ui.add_space(6.0);
    widgets::section(ui, "连接信息", |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let ping = dev.conn.as_ref().and_then(|c| parse_ping(&c.firmware));
        widgets::field_row(ui, "状态", false, |ui| {
            widgets::status_dot(ui, link.tone().dot(&t));
            ui.label(link.text());
        });
        widgets::field_row(ui, "序列号", false, |ui| {
            ui.label(dev.conn.as_ref().map_or("—", |c| c.serial.as_str()));
        });
        widgets::field_row(ui, "固件", false, |ui| {
            let text = match (ping, &dev.conn) {
                (Some((_, fw)), _) => fw.to_owned(),
                (None, Some(c)) => c.firmware.clone(),
                (None, None) => "—".to_owned(),
            };
            ui.label(text);
        });
        widgets::field_row(ui, "协议版本", false, |ui| {
            ui.label(ping.map_or("—", |(p, _)| p));
        });
        widgets::field_row(ui, "端口", false, |ui| {
            widgets::fixed_value(
                ui,
                "I2C0 · 主机",
                "固件把 I2C0 固定为主机，第二路端口还没有接出。",
            );
        });
        widgets::field_row(ui, "速率", false, |ui| {
            widgets::fixed_value(ui, "100 kHz", "速率由固件固定；调速需要后续固件和协议。");
        });
        widgets::field_row(ui, "单次传输", true, |ui| {
            ui.label("写、读各最多 256 字节");
        });
    });
    action
}

fn device_row(ui: &mut egui::Ui, d: &DeviceInfo, selected: bool, open: bool) -> egui::Response {
    let t = theme::of(ui);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());
    let radius = egui::CornerRadius::same(8);
    if selected {
        ui.painter().rect_filled(rect, radius, t.card2);
        ui.painter().rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.0, t.line),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, t.card2);
    }
    let dot = if open { t.green } else { t.faint };
    ui.painter()
        .circle_filled(rect.left_center() + egui::vec2(14.0, 0.0), 3.0, dot);
    ui.painter().text(
        rect.left_top() + egui::vec2(28.0, 7.0),
        egui::Align2::LEFT_TOP,
        &d.serial,
        egui::FontId::proportional(13.0),
        t.fg,
    );
    ui.painter().text(
        rect.left_bottom() + egui::vec2(28.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        format!("USB {}", speed(d)),
        egui::FontId::proportional(11.0),
        t.muted,
    );
    let (tag, color) = match (open, selected) {
        (true, _) => ("已连接", t.green),
        (false, true) => ("已选", t.muted),
        _ => ("", t.muted),
    };
    ui.painter().text(
        rect.right_center() - egui::vec2(12.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        tag,
        egui::FontId::proportional(11.5),
        color,
    );
    resp
}

fn speed(d: &DeviceInfo) -> &str {
    d.speed
        .strip_prefix("Some(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(d.speed.as_str())
}

pub fn device_label(d: &DeviceInfo) -> String {
    format!("{}   USB {}", d.serial, speed(d))
}

/// Split `protocol 1 firmware "usb_i2c 0.1"` into ("1", "usb_i2c 0.1").
pub fn parse_ping(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("protocol ")?;
    let (proto, fw) = rest.split_once(" firmware ")?;
    Some((proto, fw.trim_matches('"')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_line_splits() {
        assert_eq!(
            parse_ping("protocol 1 firmware \"usb_i2c 0.1\""),
            Some(("1", "usb_i2c 0.1"))
        );
        assert_eq!(parse_ping("something else"), None);
    }

    #[test]
    fn selection_follows_serial() {
        let dev = |s: &str| DeviceInfo {
            serial: s.into(),
            speed: "Some(High)".into(),
        };
        let mut d = Devices::default();
        d.set_list(vec![dev("A"), dev("B")]);
        d.selected = 1;
        d.set_list(vec![dev("C"), dev("B")]);
        assert_eq!(d.selected_serial(), Some("B"));
        d.set_list(vec![dev("C")]);
        assert_eq!(d.selected, 0);
        assert_eq!(device_label(&dev("A")), "A   USB High");
    }
}
