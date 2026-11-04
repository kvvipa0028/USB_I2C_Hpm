//! Sidebar: device connection, then the functions in three groups
//! (总线通信 / 引脚与采集 / 存储与升级). As a Mac sidebar: graphite, the
//! current page is a card with a small shadow, and the label keeps its ink
//! colour (no accent). A green dot marks a page whose local demo is running.
//! 外观 and the version sit at the foot.

use eframe::egui;

use crate::theme;
use crate::widgets;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Device,
    I2c,
    Spi,
    Can,
    Lin,
    Uart,
    Pwm,
    Gpio,
    Adc,
    Ads1015,
    Eeprom,
    Flash,
    Uds,
}

/// Group titles and their pages, in sidebar order.
pub const GROUPS: [(&str, &[Page]); 3] = [
    (
        "总线通信",
        &[Page::I2c, Page::Spi, Page::Can, Page::Lin, Page::Uart],
    ),
    (
        "引脚与采集",
        &[Page::Pwm, Page::Gpio, Page::Adc, Page::Ads1015],
    ),
    ("存储与升级", &[Page::Eeprom, Page::Flash, Page::Uds]),
];

impl Page {
    pub fn label(self) -> &'static str {
        match self {
            Page::Device => "设备连接",
            Page::I2c => "I2C",
            Page::Spi => "SPI",
            Page::Can => "CAN",
            Page::Lin => "LIN",
            Page::Uart => "UART",
            Page::Pwm => "PWM / 频率捕获",
            Page::Gpio => "GPIO",
            Page::Adc => "AD 记录",
            Page::Ads1015 => "ADS1015",
            Page::Eeprom => "EEPROM",
            Page::Flash => "Flash",
            Page::Uds => "UDS Bootloader",
        }
    }
}

enum Trail {
    None,
    Dot(egui::Color32),
}

pub const WIDTH: f32 = 212.0;

/// `live` lists the pages whose local demo is running.
pub fn sidebar(ui: &mut egui::Ui, page: &mut Page, link_dot: egui::Color32, live: &[Page]) {
    let t = theme::of(ui);
    // the foot (外观 and version) keeps its place; the list scrolls above it
    let list_h = (ui.available_height() - 64.0).max(80.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), list_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("nav")
                .max_height(list_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if nav_item(
                        ui,
                        *page == Page::Device,
                        Page::Device.label(),
                        Trail::Dot(link_dot),
                    )
                    .clicked()
                    {
                        *page = Page::Device;
                    }
                    for (title, pages) in GROUPS {
                        ui.add_space(10.0);
                        group(ui, title);
                        for &p in pages {
                            let trail = if live.contains(&p) {
                                Trail::Dot(t.green)
                            } else {
                                Trail::None
                            };
                            if nav_item(ui, *page == p, p.label(), trail).clicked() {
                                *page = p;
                            }
                        }
                    }
                });
        },
    );
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(concat!("usb-i2c-gui ", env!("CARGO_PKG_VERSION")))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);
        theme_field(ui);
    });
}

/// 外观: 浅色, 深色, or 跟随系统, applied and saved on pick.
fn theme_field(ui: &mut egui::Ui) {
    let current = ui.ctx().options(|o| o.theme_preference);
    let label = theme::CHOICES
        .iter()
        .find(|c| c.0 == current)
        .map_or("", |c| c.2);
    let width = ui.available_width();
    let resp = widgets::popup_field(ui, "外观", label, None, width);
    egui::Popup::menu(&resp)
        .width(width)
        .align(egui::RectAlign::TOP_START)
        .show(|ui| {
            ui.set_min_width(width - 12.0);
            let mut pick = current;
            for (pref, _, label) in theme::CHOICES {
                ui.selectable_value(&mut pick, pref, label);
            }
            if pick != current {
                theme::set(ui.ctx(), pick);
            }
        });
}

fn group(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(11.0)
            .color(ui.visuals().weak_text_color())
            .extra_letter_spacing(0.4),
    );
    ui.add_space(4.0);
}

fn nav_item(ui: &mut egui::Ui, selected: bool, label: &str, trail: Trail) -> egui::Response {
    let t = theme::of(ui);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
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
    match trail {
        Trail::None => {}
        Trail::Dot(c) => {
            ui.painter()
                .circle_filled(rect.right_center() - egui::vec2(14.0, 0.0), 3.0, c);
        }
    }
    resp
}
