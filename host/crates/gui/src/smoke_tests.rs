//! Headless egui layout/tessellation smoke test. This is not native-window or
//! screenshot verification, and it never opens a USB connection.

use super::*;
use std::time::Duration;

#[test]
fn all_pages_render_finite_geometry_in_both_themes_and_supported_sizes() {
    let mut rendered = 0;
    for dark in [false, true] {
        for (w, h) in [(900.0, 600.0), (1200.0, 800.0), (1600.0, 1000.0)] {
            for which in 0..13 {
                let ctx = egui::Context::default();
                fonts::install(&ctx);
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut device = device::Devices::default();
                let mut i2c = i2c::I2c::default();
                let mut spi = spi::Spi::default();
                let mut pwm = pwm::Pwm::default();
                let mut can = can::Can::default();
                let mut lin = lin::Lin::default();
                let mut uart = uart::Uart::default();
                let mut gpio = gpio::Gpio::default();
                let mut adc = adc::Adc::default();
                let mut ads = ads1015::Ads1015::default();
                let mut eeprom = eeprom::Eeprom::default();
                let mut flash = flash::Flash::default();
                let mut uds = uds::Uds::default();
                let mut page = nav::Page::Device;
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(w, h),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(raw, |ui| {
                    egui::Panel::top("test_top")
                        .exact_size(48.0)
                        .show(ui, |ui| {
                            device::top_bar(ui, &mut device, false, device::Link::Off);
                        });
                    egui::Panel::left("test_nav")
                        .exact_size(nav::WIDTH)
                        .show(ui, |ui| {
                            nav::sidebar(ui, &mut page, egui::Color32::GRAY, &[]);
                        });
                    egui::CentralPanel::default().show(ui, |ui| {
                        let now = Duration::ZERO;
                        match which {
                            0 => {
                                device::page(
                                    ui,
                                    &mut device,
                                    false,
                                    device::Link::Off,
                                    ("offline", widgets::Tone::Neutral),
                                );
                            }
                            1 => {
                                i2c::page(
                                    ui,
                                    &mut i2c,
                                    i2c::Ctx {
                                        connected: false,
                                        busy: false,
                                        now,
                                    },
                                );
                            }
                            2 => spi::page(ui, &mut spi, now),
                            3 => pwm::page(ui, &mut pwm, now),
                            4 => can::page(ui, &mut can, now),
                            5 => lin::page(ui, &mut lin, now),
                            6 => uart::page(ui, &mut uart, now),
                            7 => gpio::page(ui, &mut gpio, now),
                            8 => adc::page(ui, &mut adc, now),
                            9 => ads1015::page(ui, &mut ads, now),
                            10 => eeprom::page(ui, &mut eeprom, now),
                            11 => flash::page(ui, &mut flash, now),
                            12 => uds::page(ui, &mut uds, now),
                            _ => unreachable!(),
                        }
                    });
                });
                assert!(
                    !output.shapes.is_empty(),
                    "page {which}, {w}x{h}, dark={dark}"
                );
                output.textures_delta.clear();
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                assert!(!primitives.is_empty());
                for p in primitives {
                    assert!(p.clip_rect.is_finite());
                    if let egui::epaint::Primitive::Mesh(m) = p.primitive {
                        assert!(m.vertices.iter().all(|v| v.pos.is_finite()));
                    }
                }
                rendered += 1;
            }
        }
    }
    assert_eq!(rendered, 78);
}
