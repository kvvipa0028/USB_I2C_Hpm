//! Client for the usb_i2c bridge firmware
//! (VID:PID 1209:0001, product "usb_i2c", bulk OUT 0x01, bulk IN 0x81).

pub mod bridge;
pub mod device;
pub mod proto;
