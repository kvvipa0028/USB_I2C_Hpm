//! Open the bridge and run one request at a time.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use nusb::transfer::{Buffer, Bulk, In, Out};
use nusb::MaybeFuture;

use crate::proto::{self, Reply, Status, CMD_I2C, CMD_PING, HDR, PING_DATA_MAX};

const VID: u16 = 0x1209;
const PID: u16 = 0x0001;
const PRODUCT: &str = "usb_i2c";
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub serial: String,
    /// `Debug` format of the nusb speed, same text the CLI prints.
    pub speed: String,
}

pub fn list_devices() -> anyhow::Result<Vec<DeviceInfo>> {
    Ok(nusb::list_devices()
        .wait()?
        .filter(|d| is_bridge(d) && d.product_string() == Some(PRODUCT))
        .map(|d| DeviceInfo {
            serial: d.serial_number().unwrap_or("?").to_string(),
            speed: format!("{:?}", d.speed()),
        })
        .collect())
}

fn is_bridge(d: &nusb::DeviceInfo) -> bool {
    d.vendor_id() == VID && d.product_id() == PID
}

pub struct Bridge {
    _iface: nusb::Interface,
    out: nusb::Endpoint<Bulk, Out>,
    inp: nusb::Endpoint<Bulk, In>,
    seq: u8,
}

impl Bridge {
    pub fn open() -> anyhow::Result<Self> {
        Self::open_serial(None)
    }

    /// `serial` selects one bridge when more than one is plugged in.
    pub fn open_serial(serial: Option<&str>) -> anyhow::Result<Self> {
        let info = nusb::list_devices()
            .wait()?
            .find(|d| {
                is_bridge(d)
                    && d.product_string() == Some(PRODUCT)
                    && serial.is_none_or(|s| d.serial_number() == Some(s))
            })
            .ok_or_else(|| match serial {
                Some(s) => anyhow!("no {PRODUCT} device with serial {s}"),
                None => anyhow!("no {PRODUCT} device ({VID:04x}:{PID:04x}) found"),
            })?;
        let iface = info.open().wait()?.claim_interface(0).wait()?;
        let out = iface.endpoint::<Bulk, Out>(EP_OUT)?;
        let inp = iface.endpoint::<Bulk, In>(EP_IN)?;
        // start from a time-derived seq so a reply left over from an earlier run is not accepted
        let seq = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u8)
            .unwrap_or(0);
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

    pub fn ping_line(&mut self) -> anyhow::Result<String> {
        let seq = self.next_seq();
        let r = self.transact(CMD_PING, proto::encode_ping(seq), HDR + PING_DATA_MAX)?;
        if r.status != Status::Ok || r.data.is_empty() {
            bail!("ping failed: {:?}", r.status);
        }
        Ok(format!(
            "protocol {} firmware \"{}\"",
            r.data[0],
            String::from_utf8_lossy(&r.data[1..])
        ))
    }

    pub fn i2c(&mut self, addr: u8, wdata: &[u8], rlen: usize) -> anyhow::Result<Vec<u8>> {
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
