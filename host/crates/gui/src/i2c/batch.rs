//! Two independent port transactions, not a script or sequential command list.

use eframe::egui;
use usb_i2c::control::PortRole;
use usb_i2c::proto::Capabilities;

use super::parse::{parse_addr, parse_bytes, parse_len};
use super::Ctx;
use crate::local::parse_num;
use crate::widgets::{self, Tone};
use crate::worker::{Op, Transfer};

struct Draft {
    op: Op,
    addr: String,
    speed_hz: String,
    write: String,
    read_len: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            op: Op::WriteRead,
            addr: "0x50".into(),
            speed_hz: "100000".into(),
            write: "00".into(),
            read_len: "16".into(),
        }
    }
}

pub struct Batch {
    drafts: [Draft; 2],
    caps: Capabilities,
    offline: bool,
    roles: [Option<PortRole>; 2],
    role_required: [bool; 2],
    pub result: Vec<String>,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            drafts: [Draft::default(), Draft::default()],
            caps: Capabilities::legacy(),
            offline: false,
            roles: [None; 2],
            role_required: [false; 2],
            result: Vec::new(),
        }
    }
}

impl Batch {
    pub fn set_connection(&mut self, caps: &Capabilities, offline: bool) {
        self.caps = caps.clone();
        self.offline = offline;
        self.roles = [None; 2];
        self.role_required = [false; 2];
        self.result.clear();
        for d in &mut self.drafts {
            d.speed_hz = caps.fixed_speed_hz.to_string();
        }
    }

    pub(super) fn set_role(&mut self, port: u8, role: Option<PortRole>, required: bool) {
        if let Some(slot) = self.roles.get_mut(usize::from(port)) {
            *slot = role;
        }
        if let Some(slot) = self.role_required.get_mut(usize::from(port)) {
            *slot = required;
        }
    }

    fn requests(&self) -> Result<Vec<Transfer>, String> {
        if !self.caps.supports_batch || self.caps.port_count != 2 {
            return Err("设备未报告双端口批次能力".into());
        }
        self.drafts
            .iter()
            .enumerate()
            .map(|(port, d)| {
                if self.roles[port] == Some(PortRole::Slave)
                    || (self.role_required[port] && self.roles[port].is_none())
                {
                    return Err(format!("CH{port} 主机角色未确认，不能提交双端口批次"));
                }
                let wdata = if d.op == Op::Read {
                    Vec::new()
                } else {
                    parse_bytes(&d.write)?
                };
                let rlen = if d.op == Op::Write {
                    0
                } else {
                    parse_len(&d.read_len)?
                };
                if d.op != Op::Read && wdata.is_empty() {
                    return Err(format!("CH{port} 写入数据为空"));
                }
                if wdata.len() > self.caps.batch_max_transfer || rlen > self.caps.batch_max_transfer
                {
                    return Err(format!(
                        "CH{port} 批次内读/写各最多 {} B",
                        self.caps.batch_max_transfer
                    ));
                }
                Ok(Transfer {
                    op: d.op,
                    port: port as u8,
                    addr: parse_addr(&d.addr)?,
                    speed_hz: parse_num(
                        &d.speed_hz,
                        self.caps.min_speed_hz,
                        self.caps.max_speed_hz,
                        "SCL 频率",
                    )?,
                    wdata,
                    rlen,
                    offline: self.offline,
                })
            })
            .collect()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, cx: Ctx) -> Option<Vec<Transfer>> {
        let mut send = false;
        widgets::note(
            ui,
            if self.offline {
                "离线模型：两端口数据独立，结果只证明 PC 逻辑。模型不模拟电气、拉伸或并发时序。"
            } else {
                "每端口只提交一笔事务，分别返回状态与数据；USB 失联时不重放，未确认的写入结果未知。"
            },
        );
        for (port, d) in self.drafts.iter_mut().enumerate() {
            widgets::section(ui, &format!("CH{port}"), |ui| {
                ui.add_enabled_ui(!cx.busy, |ui| {
                    ui.push_id(port, |ui| {
                        ui.horizontal(|ui| {
                            // pick fills its available width; reserve one field
                            // so address and speed remain inside this same row.
                            ui.allocate_ui(egui::vec2(120.0, 26.0), |ui| {
                                widgets::pick(
                                    ui,
                                    &mut d.op,
                                    &[
                                        (Op::Write, "写入"),
                                        (Op::Read, "读取"),
                                        (Op::WriteRead, "写后读"),
                                    ],
                                );
                            });
                            ui.label("地址");
                            ui.add(egui::TextEdit::singleline(&mut d.addr).desired_width(65.0));
                            ui.label("SCL");
                            ui.add_enabled_ui(self.caps.supports_custom_speed, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut d.speed_hz).desired_width(80.0),
                                );
                            });
                            ui.label("Hz");
                        });
                        if d.op != Op::Read {
                            widgets::field_row(ui, "写入 HEX", false, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut d.write)
                                        .desired_width(ui.available_width()),
                                );
                            });
                        }
                        if d.op != Op::Write {
                            widgets::field_row(ui, "读取长度", false, |ui| {
                                widgets::unit_edit(ui, &mut d.read_len, "16", "B");
                            });
                        }
                    });
                });
            });
        }
        let requests = self.requests();
        if let Err(e) = &requests {
            widgets::error(ui, e);
        }
        ui.horizontal(|ui| {
            send = widgets::ink_button(
                ui,
                cx.connected && !cx.busy && requests.is_ok(),
                "提交双端口批次",
            )
            .clicked();
            widgets::badge(
                ui,
                if self.offline { "离线" } else { "USB" },
                if self.offline {
                    Tone::Amber
                } else {
                    Tone::Neutral
                },
            );
            widgets::muted(
                ui,
                &format!("每端口读/写各最多 {} B", self.caps.batch_max_transfer),
            );
        });
        if !self.result.is_empty() {
            widgets::section(ui, "批次结果", |ui| {
                for line in &self.result {
                    ui.label(line);
                }
            });
        }
        if send {
            requests.ok()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use usb_i2c::device::Device;

    fn edit_text_rects(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) if matches!(text.galley.text(), "0x50" | "100000") => {
                out.push((
                    text.galley.text().into(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                ));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    edit_text_rects(shape, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn batch_controls_stay_inside_page_width() {
        for dark in [false, true] {
            for width in [900.0, 1200.0, 1600.0] {
                for op in [Op::Write, Op::Read, Op::WriteRead] {
                    let ctx = egui::Context::default();
                    crate::theme::install(&ctx);
                    crate::fonts::install(&ctx);
                    ctx.set_theme(if dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    let mut batch = Batch::default();
                    batch.set_connection(Device::offline().capabilities(), true);
                    for draft in &mut batch.drafts {
                        draft.op = op;
                    }
                    let mut page_rect = egui::Rect::NOTHING;
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 800.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            egui::Panel::left("batch_test_nav")
                                .exact_size(crate::nav::WIDTH)
                                .show(ui, |_| {});
                            egui::CentralPanel::default()
                                .frame(
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::symmetric(20, 16)),
                                )
                                .show(ui, |ui| {
                                    widgets::page_width(ui, |ui| {
                                        page_rect = ui.max_rect();
                                        egui::ScrollArea::vertical()
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                batch.show(
                                                    ui,
                                                    Ctx {
                                                        connected: true,
                                                        busy: false,
                                                        now: std::time::Duration::ZERO,
                                                    },
                                                );
                                                assert!(
                                                    ui.min_rect().right()
                                                        <= page_rect.right() + 0.5,
                                                    "batch overflow at width={width}, dark={dark}: {:?} beyond {:?}",
                                                    ui.min_rect(),
                                                    page_rect,
                                                );
                                            });
                                    });
                                });
                        },
                    );
                    output.textures_delta.clear();
                    let mut fields = Vec::new();
                    for shape in &output.shapes {
                        edit_text_rects(&shape.shape, &mut fields);
                    }
                    for value in ["0x50", "100000"] {
                        let matching: Vec<_> =
                            fields.iter().filter(|(text, _)| text == value).collect();
                        assert_eq!(matching.len(), 2, "both ports must show {value}");
                        for (_, rect) in matching {
                            assert!(page_rect.contains_rect(*rect), "{value}: {rect:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn legacy_batch_is_blocked_and_offline_requests_keep_independent_fields() {
        let mut batch = Batch::default();
        assert!(batch.requests().is_err());
        batch.set_connection(Device::offline().capabilities(), true);
        batch.drafts[0].write = "10 aa".into();
        batch.drafts[1].addr = "0x51".into();
        batch.drafts[1].speed_hz = "400000".into();
        let req = batch.requests().unwrap();
        assert_eq!(req.len(), 2);
        assert_eq!((req[0].port, req[0].addr), (0, 0x50));
        assert_eq!(req[0].wdata, [0x10, 0xaa]);
        assert_eq!(
            (req[1].port, req[1].addr, req[1].speed_hz),
            (1, 0x51, 400_000)
        );
        assert!(req.iter().all(|r| r.offline));
        batch.drafts[0].read_len = "256".into();
        assert!(batch.requests().unwrap_err().contains("最多"));
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;

    #[test]
    fn batch_requires_both_confirmed_master_roles_when_control_is_advertised() {
        let mut batch = Batch::default();
        batch.set_connection(usb_i2c::device::Device::offline().capabilities(), true);
        batch.set_role(0, Some(PortRole::Master), true);
        batch.set_role(1, None, true);
        assert!(batch.requests().is_err());
        batch.set_role(1, Some(PortRole::Slave), true);
        assert!(batch.requests().is_err());
        batch.set_role(1, Some(PortRole::Master), true);
        assert!(batch.requests().is_ok());
    }
}
