//! I2C workbench: 主机 and 从机 tabs.
//! 主机 runs on the bridge's one I2C command (write, read, write-then-read,
//! once or on a loop). Scan and bus recovery have no command: they insert
//! rows marked 演示. 从机's sync buttons are shown switched off with a short
//! hover; the 从机 table is edited in the window only.

mod batch;
mod master;
pub mod parse;
mod slave;

use std::time::Duration;

use eframe::egui;

use batch::Batch;
pub use master::Master;
use slave::Slave;
use usb_i2c::proto::Capabilities;

use crate::widgets::{self, Tone};
use crate::worker::{Op, Transfer};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Master,
    Slave,
    Batch,
}

#[derive(Clone, Copy)]
pub struct Ctx {
    pub connected: bool,
    pub busy: bool,
    /// Since the window opened.
    pub now: Duration,
}

pub struct I2c {
    tab: Tab,
    pub master: Master,
    slave: Slave,
    pub batch: Batch,
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            tab: Tab::Master,
            master: Master::default(),
            slave: Slave::default(),
            batch: Batch::default(),
        }
    }
}

impl I2c {
    pub fn set_connection(&mut self, caps: &Capabilities, offline: bool) {
        self.master.set_connection(caps, offline);
        self.batch.set_connection(caps, offline);
    }
}

pub enum Action {
    Transfer(Op),
    Batch(Vec<Transfer>),
}

/// The slave editor is still a local draft (S3 integration is pending).
pub const FIXED_PORT: &str = "从机页仅编辑本地草稿，尚未接入设备";
/// Hover on a control that is switched off.
pub const NO_CMD: &str = "当前固件无此命令";

/// Returns the transfer the user asked for, if any.
pub fn page(ui: &mut egui::Ui, st: &mut I2c, cx: Ctx) -> Option<Action> {
    let badge = if cx.connected {
        ("已连接", Tone::Green)
    } else {
        ("未连接", Tone::Neutral)
    };
    let subtitle = match st.tab {
        Tab::Master => "写入、读取、写后读，可按周期循环。端口、速率与长度按连接设备能力开放。",
        Tab::Slave => "EEPROM 式寄存器表，256 字节、1 字节偏移，在本窗口编辑。",
        Tab::Batch => "一次提交 CH0/CH1 各一笔独立事务，分别显示结果。实际总线并发需波形验证。",
    };
    widgets::page_head(ui, "I2C", Some(badge), subtitle, |ui| {
        // Keep the action slot so switching tabs never moves the tab buttons.
        ui.scope(|ui| {
            if st.tab != Tab::Master {
                ui.set_invisible();
            }
            master::save_button(ui, &mut st.master, cx.now);
        });
        ui.add_space(6.0);
        widgets::seg(
            ui,
            &mut st.tab,
            &[
                (Tab::Master, "主机"),
                (Tab::Slave, "从机"),
                (Tab::Batch, "双端口批次"),
            ],
        );
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match st.tab {
            Tab::Master => master::show(ui, &mut st.master, cx).map(Action::Transfer),
            Tab::Batch => st.batch.show(ui, cx).map(Action::Batch),
            Tab::Slave => {
                slave::show(ui, &mut st.slave);
                None
            }
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.galley.text().into(), t.pos)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }

    #[test]
    fn all_i2c_tabs_keep_header_positions_with_legacy_and_offline_capabilities() {
        for dark in [false, true] {
            for (w, h) in [(900.0, 600.0), (1200.0, 800.0), (1600.0, 1000.0)] {
                for offline in [false, true] {
                    let ctx = egui::Context::default();
                    crate::fonts::install(&ctx);
                    ctx.set_visuals(if dark {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    });
                    let mut st = I2c::default();
                    if offline {
                        st.set_connection(usb_i2c::device::Device::offline().capabilities(), true);
                    }
                    let mut original = None;
                    for tab in [Tab::Master, Tab::Slave, Tab::Batch, Tab::Master] {
                        st.tab = tab;
                        let mut output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(w, h),
                                )),
                                ..Default::default()
                            },
                            |ui| {
                                egui::CentralPanel::default().show(ui, |ui| {
                                    page(
                                        ui,
                                        &mut st,
                                        Ctx {
                                            connected: true,
                                            busy: false,
                                            now: Duration::ZERO,
                                        },
                                    );
                                });
                            },
                        );
                        output.textures_delta.clear();
                        let mut labels = Vec::new();
                        for s in &output.shapes {
                            texts(&s.shape, &mut labels);
                        }
                        let positions: Vec<_> = ["主机", "从机", "双端口批次"]
                            .iter()
                            .map(|label| labels.iter().find(|(t, _)| t == label).unwrap().1)
                            .collect();
                        if let Some(expected) = &original {
                            assert_eq!(&positions, expected);
                        } else {
                            original = Some(positions);
                        }
                        for p in ctx.tessellate(output.shapes, output.pixels_per_point) {
                            assert!(p.clip_rect.is_finite());
                            if let egui::epaint::Primitive::Mesh(mesh) = p.primitive {
                                assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
                            }
                        }
                    }
                }
            }
        }
    }
}
