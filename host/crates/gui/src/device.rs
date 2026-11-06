//! Device strip in the top bar and the connection page. Connecting opens the
//! bridge and uses PING, whose reply carries the firmware id.

use eframe::egui;
use usb_i2c::bridge::DeviceInfo;

use crate::nav;
use crate::theme;
use crate::widgets::{self, Tone};
use usb_i2c::proto::Capabilities;

pub struct Conn {
    pub serial: String,
    /// The PING line, e.g. `protocol 1 firmware "usb_i2c 0.1"`.
    pub firmware: String,
    pub capabilities: Capabilities,
    pub offline: bool,
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

    /// An open interface can always be released, listed or not.
    pub fn can_disconnect(&self) -> bool {
        self.conn.is_some()
    }

    /// I2C goes out only while the open device is still in the list.
    pub fn can_send(&self) -> bool {
        self.conn
            .as_ref()
            .is_some_and(|c| c.offline || self.list.iter().any(|d| d.serial == c.serial))
    }

    /// One bridge at a time. Even reopening the same serial needs an explicit
    /// disconnect first; a selection never replaces the active connection.
    pub fn connect_target(&self) -> Option<&str> {
        if self.conn.is_some() {
            None
        } else {
            self.selected_serial()
        }
    }

    pub fn link(&self, connecting: bool, connect_failed: bool) -> Link {
        if connecting {
            Link::Connecting
        } else if self.can_send() {
            Link::On
        } else if self.can_disconnect() {
            Link::Missing
        } else if connect_failed {
            Link::Failed
        } else {
            Link::Off
        }
    }

    /// Always identify the active bridge while connected, even if another
    /// row is selected for a later connection.
    fn field_text(&self) -> String {
        if let Some(c) = &self.conn {
            if c.offline {
                return "离线测试设备 · 双端口模型".into();
            }
            return self
                .list
                .iter()
                .find(|d| d.serial == c.serial)
                .map_or_else(|| format!("{}   不在列表", c.serial), device_label);
        }
        self.list
            .get(self.selected)
            .map_or_else(|| "未找到设备".to_owned(), device_label)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Link {
    Off,
    Connecting,
    On,
    /// Still open here, but the last refresh did not list it.
    Missing,
    Failed,
}

impl Link {
    pub fn tone(self) -> Tone {
        match self {
            Link::Off => Tone::Neutral,
            Link::Connecting => Tone::Amber,
            Link::On => Tone::Green,
            Link::Missing | Link::Failed => Tone::Red,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Link::Off => "未连接",
            Link::Connecting => "正在连接…",
            Link::On => "已连接",
            Link::Missing => "设备不在列表",
            Link::Failed => "连接失败",
        }
    }
}

pub enum Action {
    Refresh,
    Connect(String),
    ConnectOffline,
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

        let value = dev.field_text();
        let resp = widgets::popup_field(ui, "设备", &value, Some(link.tone().dot(&t)), 300.0)
            .on_hover_text(link.text());
        let width = resp.rect.width();
        egui::Popup::menu(&resp).width(width).show(|ui| {
            // as wide as the field, less the menu's own margin
            ui.set_min_width(width - 12.0);
            device_menu(ui, dev);
        });

        if let Some(c) = &dev.conn {
            if c.offline {
                widgets::badge(ui, "离线 · 未发 USB", Tone::Amber);
            } else {
                let (proto, fw) = parse_ping(&c.firmware).unwrap_or(("?", &c.firmware));
                widgets::badge(ui, &format!("固件 {fw} · 协议 {proto}"), Tone::Neutral)
                    .on_hover_text(&c.firmware);
            }
        }

        widgets::trailing(ui, 30.0, |ui| {
            if let Some(a) = connect_buttons(ui, dev, dev.connect_target(), busy) {
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

/// 连接 for `target`, and 断开 whenever a device is open, listed or not.
fn connect_buttons(
    ui: &mut egui::Ui,
    dev: &Devices,
    target: Option<&str>,
    busy: bool,
) -> Option<Action> {
    let mut action = None;
    if let Some(serial) = target {
        if widgets::ink_button(ui, !busy, "连接").clicked() {
            action = Some(Action::Connect(serial.to_owned()));
        }
    }
    if dev.can_disconnect() && widgets::action_button(ui, !busy, "断开").clicked() {
        action = Some(Action::Disconnect);
    }
    action
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
        "连接后查询固件与实际能力。没有开发板时可连接离线模型；离线结果不代表上板验收。",
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
            if widgets::action_button(ui, !busy && dev.conn.is_none(), "连接离线测试设备").clicked()
            {
                action = Some(Action::ConnectOffline);
            }
            if let Some(a) = connect_buttons(ui, dev, dev.connect_target(), busy) {
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
        if let Some(c) = &dev.conn {
            widgets::field_row(ui, "数据来源", false, |ui| {
                ui.label(if c.offline {
                    "离线内存模型，不产生 USB/I2C 信号"
                } else {
                    "USB 设备能力回复"
                });
            });
            widgets::field_row(ui, "扩展协议", false, |ui| {
                ui.label(
                    c.capabilities
                        .extension_version
                        .map_or("legacy v1".into(), |v| format!("P0 extension {v}")),
                );
            });
            widgets::field_row(ui, "端口", false, |ui| {
                ui.label(format!("{} 路主机端口", c.capabilities.port_count));
            });
            widgets::field_row(ui, "速率范围", false, |ui| {
                ui.label(format!(
                    "{}–{} Hz",
                    c.capabilities.min_speed_hz, c.capabilities.max_speed_hz
                ));
            });
            widgets::field_row(ui, "单次传输", true, |ui| {
                ui.label(format!("写、读各最多 {} 字节", c.capabilities.max_transfer));
            });
            widgets::note(
                ui,
                if c.offline {
                    "模型：CH0/CH1 各有独立的 0x50 EEPROM（256 B），写入首字节为偏移，表尾回绕；其他地址返回 NACK。断开后数据重置。速率仅检查参数，不模拟波形或时序。"
                } else if c.capabilities.extension_version.is_none() {
                    "旧固件兼容模式：仅 CH0、100 kHz、基本读写。CH1 与可调速率未开放。"
                } else {
                    "能力来自设备报告；实际频率、两路波形重叠与故障隔离仍需上板测量。"
                },
            );
        } else {
            widgets::note(ui, "连接后显示设备能力；未连接时不推断端口或速率支持。");
        }
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

    fn dev(s: &str) -> DeviceInfo {
        DeviceInfo {
            serial: s.into(),
            speed: "Some(High)".into(),
        }
    }

    /// Connected to A while A was listed.
    fn open_a() -> Devices {
        let mut d = Devices::default();
        d.set_list(vec![dev("A")]);
        d.conn = Some(Conn {
            serial: "A".into(),
            firmware: "protocol 1 firmware \"usb_i2c 0.1\"".into(),
            capabilities: Capabilities::legacy(),
            offline: false,
        });
        assert!(d.can_send());
        assert_eq!(d.link(false, false), Link::On);
        assert_eq!(d.connect_target(), None);
        d
    }

    #[test]
    fn open_device_gone_from_empty_list() {
        let mut d = open_a();
        d.set_list(vec![]);
        assert!(d.can_disconnect());
        assert!(!d.can_send());
        assert_eq!(d.link(false, false), Link::Missing);
        assert_eq!(d.connect_target(), None);
        assert_eq!(d.field_text(), "A   不在列表");
    }

    #[test]
    fn open_device_gone_from_list_of_others() {
        let mut d = open_a();
        d.set_list(vec![dev("B")]);
        assert!(d.can_disconnect());
        assert!(!d.can_send());
        assert_eq!(d.link(false, false), Link::Missing);
        assert_eq!(d.connect_target(), None);
        assert_eq!(d.field_text(), "A   不在列表");
        // back in the list: sendable again
        d.set_list(vec![dev("B"), dev("A")]);
        assert!(d.can_send());
        assert_eq!(d.field_text(), "A   USB High");
        d.conn = None;
        assert!(!d.can_disconnect() && !d.can_send());
        assert_eq!(d.link(false, true), Link::Failed);
        assert_eq!(d.connect_target(), Some("B"));
    }

    #[test]
    fn selecting_another_device_does_not_offer_replacement() {
        let mut d = open_a();
        d.set_list(vec![dev("A"), dev("B")]);
        d.selected = 1;
        assert_eq!(d.selected_serial(), Some("B"));
        assert_eq!(d.connect_target(), None);
        assert_eq!(d.field_text(), "A   USB High");
        assert!(d.can_send());
        d.conn = None;
        assert_eq!(d.connect_target(), Some("B"));
        assert_eq!(d.field_text(), "B   USB High");
    }

    #[test]
    fn selection_follows_serial() {
        let mut d = Devices::default();
        d.set_list(vec![dev("A"), dev("B")]);
        d.selected = 1;
        d.set_list(vec![dev("C"), dev("B")]);
        assert_eq!(d.selected_serial(), Some("B"));
        d.set_list(vec![dev("C")]);
        assert_eq!(d.selected, 0);
        assert_eq!(device_label(&dev("A")), "A   USB High");
    }
    #[test]
    fn offline_connection_does_not_depend_on_usb_enumeration() {
        let mut d = open_a();
        d.conn.as_mut().unwrap().offline = true;
        d.set_list(Vec::new());
        assert!(d.can_send());
        assert_eq!(d.link(false, false), Link::On);
        assert!(d.field_text().contains("离线"));
        assert!(d.connect_target().is_none());
    }
}
