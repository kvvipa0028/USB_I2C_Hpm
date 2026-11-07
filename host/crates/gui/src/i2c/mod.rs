//! Capability-aware I2C workbench with explicit port controls and table snapshots.

mod batch;
mod control;
mod master;
pub mod parse;
mod slave;

use std::time::Duration;

use eframe::egui;

use batch::Batch;
use control::Controls;
pub use master::Master;
use slave::Slave;
use usb_i2c::control::{ControlCapabilities, PortState};
use usb_i2c::device::DeviceError;
use usb_i2c::proto::Capabilities;

use crate::widgets::{self, Tone};
use crate::worker::{ControlReply, ControlRequest, Op, Transfer};

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
    controls: Controls,
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            tab: Tab::Master,
            master: Master::default(),
            slave: Slave::default(),
            batch: Batch::default(),
            controls: Controls::default(),
        }
    }
}

impl I2c {
    pub fn set_connection(&mut self, caps: &Capabilities, offline: bool) {
        self.master.set_connection(caps, offline);
        self.batch.set_connection(caps, offline);
        self.controls.set_connection(caps, offline);
        self.slave.disconnected();
    }

    pub fn set_controls(&mut self, caps: &[ControlCapabilities], states: &[PortState]) {
        self.controls.set_controls(caps, states);
        self.sync_roles();
    }

    fn sync_roles(&mut self) {
        for port in 0..2 {
            let control = &self.controls.ports[usize::from(port)];
            let role = control.confirmed.as_ref().map(|s| s.config.role);
            self.master
                .set_role(port, role, control.caps.supports_role_config);
            self.batch
                .set_role(port, role, control.caps.supports_role_config);
        }
    }

    pub fn disconnected(&mut self) {
        self.controls.disconnected();
        self.slave.disconnected();
        self.master.stop_loop("设备已断开，循环已停止");
        self.batch.result.clear();
        self.sync_roles();
    }

    pub fn on_control(
        &mut self,
        request: &ControlRequest,
        result: &Result<ControlReply, DeviceError>,
    ) -> (String, Tone) {
        self.slave.on_control(request, result);
        let message = self.controls.on_control(request, result);
        self.sync_roles();
        message
    }
}

pub enum Action {
    Transfer(Op),
    Batch(Vec<Transfer>),
    Control(ControlRequest),
}

/// Returns the transfer the user asked for, if any.
pub fn page(ui: &mut egui::Ui, st: &mut I2c, cx: Ctx) -> Option<Action> {
    let badge = if cx.connected {
        ("已连接", Tone::Green)
    } else {
        ("未连接", Tone::Neutral)
    };
    let subtitle = match st.tab {
        Tab::Master => "写入、读取、写后读，可按周期循环。端口、速率与长度按连接设备能力开放。",
        Tab::Slave => "每端口独立的 256 B 从机表草稿，显式读回快照与 CAS 片段下发。",
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
        .show(ui, |ui| {
            let control_action = st.controls.show(ui, cx);
            let port = st.controls.port;
            st.master.select_port(port);
            ui.add_space(12.0);
            let page_action = match st.tab {
                Tab::Master => master::show(ui, &mut st.master, cx).map(Action::Transfer),
                Tab::Batch => st.batch.show(ui, cx).map(Action::Batch),
                Tab::Slave => {
                    let control = &st.controls.ports[usize::from(port)];
                    slave::show(
                        ui,
                        &mut st.slave,
                        port,
                        &control.caps,
                        control.confirmed.as_ref(),
                        cx,
                    )
                    .map(Action::Control)
                }
            };
            st.master.select_port(port);
            let action = control_action.map(Action::Control).or(page_action);
            if matches!(action, Some(Action::Control(_))) {
                st.master.stop_loop("执行端口控制，循环已停止");
            }
            action
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

#[cfg(test)]
mod control_ui_tests {
    use super::*;
    use usb_i2c::control::{PortConfig, PortRole};
    use usb_i2c::device::Device;

    #[test]
    fn connected_controls_keep_all_tabs_within_900px_and_never_send_on_render() {
        for dark in [false, true] {
            for tab in [Tab::Master, Tab::Slave, Tab::Batch] {
                let mut device = Device::offline();
                device
                    .configure_port(
                        1,
                        &PortConfig {
                            role: PortRole::Slave,
                            ..PortConfig::default()
                        },
                        None,
                    )
                    .unwrap();
                let states = [device.port_state(0).unwrap(), device.port_state(1).unwrap()];
                let mut workbench = I2c::default();
                workbench.set_connection(device.capabilities(), true);
                workbench.set_controls(device.control_capabilities(), &states);
                workbench.tab = tab;
                if tab == Tab::Slave {
                    workbench.controls.port = 1;
                }
                let ctx = egui::Context::default();
                crate::theme::install(&ctx);
                crate::fonts::install(&ctx);
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900.0, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::left("control_test_nav")
                            .exact_size(crate::nav::WIDTH)
                            .show(ui, |_| {});
                        egui::CentralPanel::default()
                            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 16)))
                            .show(ui, |ui| {
                                widgets::page_width(ui, |ui| {
                                    let right = ui.max_rect().right();
                                    assert!(page(
                                        ui,
                                        &mut workbench,
                                        Ctx {
                                            connected: true,
                                            busy: false,
                                            now: Duration::ZERO
                                        }
                                    )
                                    .is_none());
                                    assert!(
                                        ui.min_rect().right() <= right + 0.5,
                                        "controls overflow at dark={dark}: {:?} > {right}",
                                        ui.min_rect()
                                    );
                                });
                            });
                    },
                );
                output.textures_delta.clear();
                for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
                    assert!(primitive.clip_rect.is_finite());
                }
            }
        }
    }

    #[test]
    fn disconnect_clears_confirmed_roles_stops_loops_and_reconnect_selects_ch0() {
        let mut device = Device::offline();
        let states = [device.port_state(0).unwrap(), device.port_state(1).unwrap()];
        let mut workbench = I2c::default();
        workbench.set_connection(device.capabilities(), true);
        workbench.set_controls(device.control_capabilities(), &states);
        workbench.controls.port = 1;
        workbench.master.arm_test_write_loop();
        workbench.disconnected();
        assert!(!workbench.master.looping());
        assert!(workbench
            .controls
            .ports
            .iter()
            .all(|p| p.confirmed.is_none()));
        workbench.set_connection(device.capabilities(), true);
        assert_eq!(workbench.controls.port, 0);
    }
}
