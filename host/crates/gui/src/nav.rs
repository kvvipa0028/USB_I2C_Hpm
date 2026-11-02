//! Sidebar: device connection, then the functions (I2C / SPI / PWM).
//! As a Mac sidebar: graphite, the current page is a card with a small
//! shadow, and the label keeps its ink colour (no accent).

use eframe::egui;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Device,
    I2c,
    Spi,
    Pwm,
}

impl Page {
    const FUNCS: [Page; 3] = [Page::I2c, Page::Spi, Page::Pwm];

    pub fn label(self) -> &'static str {
        match self {
            Page::Device => "设备连接",
            Page::I2c => "I2C",
            Page::Spi => "SPI",
            Page::Pwm => "PWM",
        }
    }
}

enum Trail {
    None,
    Dot(egui::Color32),
    Tag(&'static str),
}

pub const WIDTH: f32 = 212.0;

pub fn sidebar(ui: &mut egui::Ui, page: &mut Page, link_dot: egui::Color32) {
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
    ui.add_space(10.0);
    group(ui, "功能");
    for p in Page::FUNCS {
        // SPI and PWM keep their place in the list; the firmware has no commands for them
        let trail = if p == Page::I2c {
            Trail::None
        } else {
            Trail::Tag("未实现")
        };
        if nav_item(ui, *page == p, p.label(), trail).clicked() {
            *page = p;
        }
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(concat!("usb-i2c-gui ", env!("CARGO_PKG_VERSION")))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
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
    match trail {
        Trail::None => {}
        Trail::Dot(c) => {
            ui.painter()
                .circle_filled(rect.right_center() - egui::vec2(14.0, 0.0), 3.0, c);
        }
        Trail::Tag(text) => {
            ui.painter().text(
                rect.right_center() - egui::vec2(10.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                text,
                egui::FontId::proportional(10.5),
                t.muted,
            );
        }
    }
    resp
}
