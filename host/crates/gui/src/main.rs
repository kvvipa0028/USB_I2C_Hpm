//! Window for the usb_i2c bridge: connect, then write, read, and
//! write-then-read on I2C. The sidebar lists device connection, then
//! I2C / SPI / PWM. Only PING and I2C go over USB: scan, bus recovery, and
//! the slave sync buttons are switched off (no commands for them), and the
//! SPI and PWM pages run in the window only. The I2C rate is fixed in the
//! firmware (100 kHz).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod device;
mod fonts;
mod i2c;
mod nav;
mod pwm;
mod spi;
mod theme;
mod widgets;
mod worker;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(app::SIZE)
            .with_min_inner_size(app::MIN_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        "USB-I2C",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc.egui_ctx.clone())))),
    )
}
