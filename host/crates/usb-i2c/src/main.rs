//! usb-i2c: command-line client for the usb_i2c bridge firmware
//! (VID:PID 1209:0001, product "usb_i2c", bulk OUT 0x01, bulk IN 0x81).

mod proto;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use clap::{Parser, Subcommand};
use nusb::transfer::{Buffer, Bulk, In, Out};
use nusb::MaybeFuture;
use proto::{Reply, Status, CMD_I2C, CMD_PING, HDR, PING_DATA_MAX};

const VID: u16 = 0x1209;
const PID: u16 = 0x0001;
const PRODUCT: &str = "usb_i2c";
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;
const TIMEOUT: Duration = Duration::from_secs(1);

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

struct Bridge {
    _iface: nusb::Interface,
    out: nusb::Endpoint<Bulk, Out>,
    inp: nusb::Endpoint<Bulk, In>,
    seq: u8,
}

impl Bridge {
    fn open() -> anyhow::Result<Self> {
        let info = nusb::list_devices()
            .wait()?
            .find(|d| d.vendor_id() == VID && d.product_id() == PID && d.product_string() == Some(PRODUCT))
            .ok_or_else(|| anyhow!("no {PRODUCT} device ({VID:04x}:{PID:04x}) found"))?;
        let iface = info.open().wait()?.claim_interface(0).wait()?;
        let out = iface.endpoint::<Bulk, Out>(EP_OUT)?;
        let inp = iface.endpoint::<Bulk, In>(EP_IN)?;
        // start from a time-derived seq so a reply left over from an earlier run is not accepted
        let seq = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() as u8).unwrap_or(0);
        Ok(Bridge { _iface: iface, out, inp, seq })
    }

    fn transact(&mut self, cmd: u8, req: Vec<u8>, reply_max: usize) -> anyhow::Result<Reply> {
        let seq = req[1];
        self.inp.submit(self.inp.allocate(proto::in_len(reply_max, self.inp.max_packet_size())));
        let req = proto::pad_for_mps(req, self.out.max_packet_size());
        self.out.submit(Buffer::from(req));
        let c = self.out.wait_next_complete(TIMEOUT).ok_or_else(|| anyhow!("timeout sending request"))?;
        c.status.context("sending request")?;
        let c = self.inp.wait_next_complete(TIMEOUT).ok_or_else(|| anyhow!("timeout waiting for reply"))?;
        c.status.context("receiving reply")?;
        proto::decode(cmd, seq, &c.buffer[..c.actual_len]).map_err(|e| anyhow!(e))
    }

    fn next_seq(&mut self) -> u8 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn ping(&mut self) -> anyhow::Result<Reply> {
        let seq = self.next_seq();
        self.transact(CMD_PING, proto::encode_ping(seq), HDR + PING_DATA_MAX)
    }

    fn i2c(&mut self, addr: u8, wdata: &[u8], rlen: usize) -> anyhow::Result<Vec<u8>> {
        let seq = self.next_seq();
        let req = proto::encode_i2c(seq, addr, wdata, rlen).map_err(|e| anyhow!(e))?;
        let r = self.transact(CMD_I2C, req, HDR + rlen)?;
        if r.status != Status::Ok {
            bail!("I2C 0x{addr:02x}: {:?} (hpm_stat {})", r.status, r.hpm_stat);
        }
        if r.data.len() != rlen {
            bail!("expected {rlen} bytes, got {}", r.data.len());
        }
        Ok(r.data)
    }
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
            let devs: Vec<_> = nusb::list_devices()
                .wait()?
                .filter(|d| d.vendor_id() == VID && d.product_id() == PID && d.product_string() == Some(PRODUCT))
                .collect();
            for d in &devs {
                println!("{PRODUCT} serial={} speed={:?}", d.serial_number().unwrap_or("?"), d.speed());
            }
            if devs.is_empty() {
                println!("no {PRODUCT} device found");
            }
        }
        Cmd::Ping => {
            let r = Bridge::open()?.ping()?;
            if r.status != Status::Ok || r.data.is_empty() {
                bail!("ping failed: {:?}", r.status);
            }
            println!("protocol {} firmware \"{}\"", r.data[0], String::from_utf8_lossy(&r.data[1..]));
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
