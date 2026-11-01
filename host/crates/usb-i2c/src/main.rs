//! usb-i2c: command-line client for the usb_i2c bridge firmware.

use clap::{Parser, Subcommand};
use usb_i2c::bridge::{list_devices, Bridge};

#[derive(Parser)]
#[command(name = "usb-i2c", about = "USB-to-I2C bridge client (address: 0x50 or 80; data bytes: hex)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List connected bridges
    List,
    /// Check the link and print the firmware id
    Ping,
    /// Write bytes: usb-i2c write 0x50 00 de ad
    Write {
        #[arg(value_parser = parse_addr)]
        addr: u8,
        #[arg(required = true, value_parser = parse_hex_byte)]
        data: Vec<u8>,
    },
    /// Read N bytes: usb-i2c read 0x50 16
    Read {
        #[arg(value_parser = parse_addr)]
        addr: u8,
        len: usize,
    },
    /// Write, repeated START, read N bytes: usb-i2c wr 0x50 16 80
    Wr {
        #[arg(value_parser = parse_addr)]
        addr: u8,
        len: usize,
        #[arg(required = true, value_parser = parse_hex_byte)]
        data: Vec<u8>,
    },
}

fn parse_addr(s: &str) -> Result<u8, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u8::from_str_radix(h, 16),
        None => s.parse::<u8>(),
    };
    r.map_err(|e| format!("bad address '{s}': {e}"))
}

fn parse_hex_byte(s: &str) -> Result<u8, String> {
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u8::from_str_radix(h, 16).map_err(|e| format!("bad hex byte '{s}': {e}"))
}

fn dump(data: &[u8]) {
    for (i, row) in data.chunks(16).enumerate() {
        let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
        println!("{:04x}: {}", i * 16, hex.join(" "));
    }
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::List => {
            let devs = list_devices()?;
            for d in &devs {
                println!("usb_i2c serial={} speed={}", d.serial, d.speed);
            }
            if devs.is_empty() {
                println!("no usb_i2c device found");
            }
        }
        Cmd::Ping => {
            println!("{}", Bridge::open()?.ping_line()?);
        }
        Cmd::Write { addr, data } => {
            Bridge::open()?.i2c(addr, &data, 0)?;
            println!("OK wrote {} bytes to 0x{addr:02x}", data.len());
        }
        Cmd::Read { addr, len } => dump(&Bridge::open()?.i2c(addr, &[], len)?),
        Cmd::Wr { addr, len, data } => dump(&Bridge::open()?.i2c(addr, &data, len)?),
    }
    Ok(())
}
