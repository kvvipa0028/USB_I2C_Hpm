//! Window for the usb_i2c bridge: connect, then write, read, and
//! write-then-read on I2C. The sidebar lists device connection, then every
//! BusStudio module in three groups. Only PING and I2C go over USB: the
//! I2C master's transfers (once or on a loop) are real; its scan and bus
//! recovery insert labelled demo rows, the slave sync buttons are switched
//! off, and every other page runs in the window only. The I2C rate is
//! fixed in the firmware (100 kHz).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adc;
mod ads1015;
mod app;
mod can;
mod chart;
mod device;
mod eeprom;
mod flash;
mod fonts;
mod frame;
mod gpio;
mod i2c;
mod job;
mod lin;
mod local;
mod nav;
mod pwm;
mod spi;
mod theme;
mod uart;
mod uds;
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
