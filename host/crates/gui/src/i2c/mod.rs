//! I2C workbench: 主机 and 从机 tabs.
//! 主机 runs on the bridge's one I2C command (write, read, write-then-read).
//! Scan, bus recovery, and everything on 从机 are laid out but not wired:
//! the protocol has no commands for them, and nothing here pretends it does.

mod master;
pub mod parse;
mod slave;

use eframe::egui;

pub use master::Master;
use slave::Slave;

use crate::widgets::{self, Tone};
use crate::worker::Op;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Master,
    Slave,
}

#[derive(Clone, Copy)]
pub struct Ctx {
    pub connected: bool,
    pub busy: bool,
}

pub struct I2c {
    tab: Tab,
    pub master: Master,
    slave: Slave,
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            tab: Tab::Master,
            master: Master::default(),
            slave: Slave::default(),
        }
    }
}

/// Firmware facts shown on both tabs.
const FIXED_PORT: &str = "固件把 I2C0 固定为主机；第二路端口和主从切换要等后续固件。";
const FIXED_RATE: &str = "速率由固件固定为 100 kHz；调速要等后续固件和协议。";
/// Why a control is off.
const NO_CMD: &str = "协议未实现：USB 桥目前只有 PING 和 I2C 读写两个命令。";

/// Returns the transfer the user asked for, if any.
pub fn page(ui: &mut egui::Ui, st: &mut I2c, cx: Ctx) -> Option<Op> {
    let badge = if cx.connected {
        ("已连接", Tone::Green)
    } else {
        ("未连接", Tone::Neutral)
    };
    let subtitle = match st.tab {
        Tab::Master => "写入、读取、写后读。速率固件固定 100 kHz，单次最多 256 字节。",
        Tab::Slave => "EEPROM 式寄存器表，256 字节、1 字节偏移。从机命令还没有，表格只在本机编辑。",
    };
    widgets::page_head(ui, "I2C", Some(badge), subtitle, |ui| {
        widgets::seg(
            ui,
            &mut st.tab,
            &[(Tab::Master, "主机"), (Tab::Slave, "从机")],
        );
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match st.tab {
            Tab::Master => master::show(ui, &mut st.master, cx),
            Tab::Slave => {
                slave::show(ui, &mut st.slave);
                None
            }
        })
        .inner
}
