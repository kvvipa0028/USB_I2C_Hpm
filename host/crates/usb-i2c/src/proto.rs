//! Wire format of the usb_i2c bridge (firmware/usb_i2c/src/usb_i2c.c), protocol 1.
//!
//! Request: cmd u8, seq u8, addr u8, rsvd u8, wlen u16 LE, rlen u16 LE, wdata.
//! Reply:   cmd u8, seq u8, status u8, rsvd u8, hpm_stat u32 LE, data.

pub const CMD_PING: u8 = 0x01;
pub const CMD_I2C: u8 = 0x10;
pub const HDR: usize = 8;
pub const MAX_XFER: usize = 256;
/// Upper bound of the PING reply data (version byte + firmware id).
pub const PING_DATA_MAX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Nack,
    Timeout,
    BadRequest,
    I2cError,
    Unknown(u8),
}

impl From<u8> for Status {
    fn from(v: u8) -> Self {
        match v {
            0 => Status::Ok,
            1 => Status::Nack,
            2 => Status::Timeout,
            3 => Status::BadRequest,
            4 => Status::I2cError,
            x => Status::Unknown(x),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: Status,
    pub hpm_stat: u32,
    pub data: Vec<u8>,
}

fn header(cmd: u8, seq: u8, addr: u8, wlen: usize, rlen: usize) -> Vec<u8> {
    let mut v = vec![cmd, seq, addr, 0];
    v.extend_from_slice(&(wlen as u16).to_le_bytes());
    v.extend_from_slice(&(rlen as u16).to_le_bytes());
    v
}

pub fn encode_ping(seq: u8) -> Vec<u8> {
    header(CMD_PING, seq, 0, 0, 0)
}

/// `wdata` only: write; `rlen` only: read; both: write, repeated START, read.
pub fn encode_i2c(seq: u8, addr: u8, wdata: &[u8], rlen: usize) -> Result<Vec<u8>, String> {
    if addr > 0x7F {
        return Err(format!("address 0x{addr:02x} is not a 7-bit address"));
    }
    if wdata.len() > MAX_XFER || rlen > MAX_XFER {
        return Err(format!("at most {MAX_XFER} bytes per write and per read"));
    }
    if wdata.is_empty() && rlen == 0 {
        return Err("nothing to write or read".into());
    }
    let mut v = header(CMD_I2C, seq, addr, wdata.len(), rlen);
    v.extend_from_slice(wdata);
    Ok(v)
}

/// The device reads requests into a 512-byte buffer, so a request whose length is a
/// multiple of the packet size (only possible at full speed, 64) would never end with
/// a short packet. Add one byte; the device ignores bytes beyond 8 + wlen.
pub fn pad_for_mps(mut req: Vec<u8>, mps: usize) -> Vec<u8> {
    if req.len().is_multiple_of(mps) {
        req.push(0);
    }
    req
}

/// IN buffer length: the reply is at most `reply_max` bytes. A multiple of the packet
/// size, so a reply of exactly `reply_max` bytes also completes the transfer.
pub fn in_len(reply_max: usize, mps: usize) -> usize {
    reply_max.div_ceil(mps) * mps
}

pub fn decode(cmd: u8, seq: u8, rsp: &[u8]) -> Result<Reply, String> {
    if rsp.len() < HDR {
        return Err(format!("short reply ({} bytes)", rsp.len()));
    }
    if rsp[0] != cmd || rsp[1] != seq {
        return Err(format!(
            "reply is for cmd 0x{:02x} seq {}, expected cmd 0x{cmd:02x} seq {seq}",
            rsp[0], rsp[1]
        ));
    }
    Ok(Reply {
        status: Status::from(rsp[2]),
        hpm_stat: u32::from_le_bytes([rsp[4], rsp[5], rsp[6], rsp[7]]),
        data: rsp[HDR..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_layout() {
        assert_eq!(encode_ping(7), vec![CMD_PING, 7, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn i2c_layouts() {
        let w = encode_i2c(1, 0x50, &[0xAA, 0xBB], 0).unwrap();
        assert_eq!(w, vec![CMD_I2C, 1, 0x50, 0, 2, 0, 0, 0, 0xAA, 0xBB]);
        let r = encode_i2c(2, 0x50, &[], 256).unwrap();
        assert_eq!(r, vec![CMD_I2C, 2, 0x50, 0, 0, 0, 0x00, 0x01]);
        let wr = encode_i2c(3, 0x50, &[0x80], 10).unwrap();
        assert_eq!(wr, vec![CMD_I2C, 3, 0x50, 0, 1, 0, 10, 0, 0x80]);
    }

    #[test]
    fn i2c_rejects_bad_requests() {
        assert!(encode_i2c(0, 0x80, &[1], 0).is_err());
        assert!(encode_i2c(0, 0x50, &[0; 257], 0).is_err());
        assert!(encode_i2c(0, 0x50, &[], 257).is_err());
        assert!(encode_i2c(0, 0x50, &[], 0).is_err());
        assert!(encode_i2c(0, 0x50, &[0; 256], 256).is_ok());
    }

    #[test]
    fn padding_only_when_needed() {
        assert_eq!(pad_for_mps(vec![0; 64], 64).len(), 65); // full speed, 8 + 56
        assert_eq!(pad_for_mps(vec![0; 72], 64).len(), 72);
        assert_eq!(pad_for_mps(vec![0; 264], 512).len(), 264); // high speed never pads
    }

    #[test]
    fn in_buffer_length() {
        assert_eq!(in_len(8, 512), 512);
        assert_eq!(in_len(64, 64), 64);
        assert_eq!(in_len(72, 64), 128);
        assert_eq!(in_len(264, 512), 512);
    }

    #[test]
    fn decode_replies() {
        let ok = [CMD_I2C, 5, 0, 0, 0, 0, 0, 0, 0x11, 0x22];
        let want = Reply { status: Status::Ok, hpm_stat: 0, data: vec![0x11, 0x22] };
        assert_eq!(decode(CMD_I2C, 5, &ok).unwrap(), want);
        let nack = [CMD_I2C, 5, 1, 0, 0xD3, 0x07, 0, 0]; // status_i2c_no_addr_hit = 2003
        let r = decode(CMD_I2C, 5, &nack).unwrap();
        assert_eq!((r.status, r.hpm_stat, r.data.len()), (Status::Nack, 2003, 0));
        let odd = [CMD_I2C, 5, 9, 0, 0, 0, 0, 0];
        assert_eq!(decode(CMD_I2C, 5, &odd).unwrap().status, Status::Unknown(9));
        assert!(decode(CMD_I2C, 6, &ok).is_err()); // stale seq
        assert!(decode(CMD_PING, 5, &ok).is_err()); // wrong cmd
        assert!(decode(CMD_I2C, 5, &ok[..7]).is_err());
    }
}
