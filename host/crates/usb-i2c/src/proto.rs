//! Wire format of the usb_i2c bridge (firmware/usb_i2c/src/usb_i2c.c), protocol 1.
//!
//! Request: cmd u8, seq u8, addr u8, rsvd u8, wlen u16 LE, rlen u16 LE, wdata.
//! Reply:   cmd u8, seq u8, status u8, rsvd u8, hpm_stat u32 LE, data.

pub const CMD_PING: u8 = 0x01;
pub const CMD_CAPABILITIES: u8 = 0x02;
pub const CMD_I2C: u8 = 0x10;
pub const CMD_I2C_PORT: u8 = 0x11;
pub const CMD_I2C_BATCH: u8 = 0x12;
pub const HDR: usize = 8;
pub const MAX_XFER: usize = 256;
pub const CAPABILITIES_DATA_LEN: usize = 20;
pub const EXTENSION_VERSION: u8 = 1;
pub const MAX_PORTS: u8 = 2;
pub const BATCH_MAX_XFER: usize = 240;
/// Includes the largest extension response and a possible transport pad.
pub const MAX_FRAME: usize = 512;
/// Upper bound of the PING reply data (version byte + firmware id).
pub const PING_DATA_MAX: usize = 64;

/// Capabilities reported by the device, not evidence of hardware acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// None identifies the original PING/I2C-only firmware.
    pub extension_version: Option<u8>,
    pub port_count: u8,
    pub max_transfer: usize,
    /// Fixed speed for legacy devices; default speed when custom speed is supported.
    pub fixed_speed_hz: u32,
    pub min_speed_hz: u32,
    pub max_speed_hz: u32,
    pub supports_custom_speed: bool,
    pub supports_port_transfer: bool,
    pub supports_batch: bool,
    pub batch_max_transfer: usize,
}

impl Capabilities {
    pub fn legacy() -> Self {
        Self {
            extension_version: None,
            port_count: 1,
            max_transfer: MAX_XFER,
            fixed_speed_hz: 100_000,
            min_speed_hz: 100_000,
            max_speed_hz: 100_000,
            supports_custom_speed: false,
            supports_port_transfer: false,
            supports_batch: false,
            batch_max_transfer: 0,
        }
    }

    /// Validate everything before a transfer can have side effects.
    pub fn validate_transfer(&self, request: &TransferRequest) -> Result<(), String> {
        validate_i2c(request.addr, &request.wdata, request.rlen)?;
        if request.port >= self.port_count {
            return Err(format!(
                "port {} is unavailable ({} port(s))",
                request.port, self.port_count
            ));
        }
        if !self.supports_port_transfer && request.port != 0 {
            return Err("explicit port transfers are unsupported".into());
        }
        if request.wdata.len() > self.max_transfer || request.rlen > self.max_transfer {
            return Err(format!(
                "device supports at most {} bytes per write and per read",
                self.max_transfer
            ));
        }
        if request.speed_hz < self.min_speed_hz
            || request.speed_hz > self.max_speed_hz
            || (!self.supports_custom_speed && request.speed_hz != self.fixed_speed_hz)
        {
            return Err(format!(
                "speed {} Hz is unsupported ({}..={} Hz)",
                request.speed_hz, self.min_speed_hz, self.max_speed_hz
            ));
        }
        Ok(())
    }

    pub fn validate_batch(&self, requests: &[TransferRequest]) -> Result<(), String> {
        if !self.supports_batch {
            return Err("dual-port batches are unsupported".into());
        }
        validate_batch_shape(requests)?;
        for request in requests {
            self.validate_transfer(request)?;
            if request.wdata.len() > self.batch_max_transfer
                || request.rlen > self.batch_max_transfer
            {
                return Err(format!(
                    "batch supports at most {} bytes per write and per read",
                    self.batch_max_transfer
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    pub port: u8,
    pub addr: u8,
    pub wdata: Vec<u8>,
    pub rlen: usize,
    pub speed_hz: u32,
}

/// A confirmed per-port result; an error on one port does not erase the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferOutcome {
    pub port: u8,
    pub status: Status,
    pub hpm_stat: u32,
    pub data: Vec<u8>,
}

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

pub fn encode_capabilities(seq: u8) -> Vec<u8> {
    header(CMD_CAPABILITIES, seq, 0, 0, 0)
}

/// Only the exact old-firmware unsupported-command response permits fallback.
/// The common decoder has already verified cmd, seq, and the reserved byte.
pub fn decode_capabilities(reply: &Reply) -> Result<Capabilities, String> {
    if reply.status == Status::BadRequest && reply.hpm_stat == 0 && reply.data.is_empty() {
        return Ok(Capabilities::legacy());
    }
    if reply.status != Status::Ok
        || reply.hpm_stat != 0
        || reply.data.len() != CAPABILITIES_DATA_LEN
    {
        return Err("invalid capabilities reply; legacy fallback is not permitted".into());
    }
    let d = &reply.data;
    let flags = d[2];
    let caps = Capabilities {
        extension_version: Some(d[0]),
        port_count: d[1],
        supports_port_transfer: flags & 1 != 0,
        supports_batch: flags & 2 != 0,
        supports_custom_speed: flags & 4 != 0,
        max_transfer: u16::from_le_bytes([d[4], d[5]]) as usize,
        batch_max_transfer: u16::from_le_bytes([d[6], d[7]]) as usize,
        fixed_speed_hz: u32::from_le_bytes(d[8..12].try_into().unwrap()),
        min_speed_hz: u32::from_le_bytes(d[12..16].try_into().unwrap()),
        max_speed_hz: u32::from_le_bytes(d[16..20].try_into().unwrap()),
    };
    if d[0] != EXTENSION_VERSION
        || d[3] != 0
        || flags & !7 != 0
        || !(1..=MAX_PORTS).contains(&caps.port_count)
        || !(1..=MAX_XFER).contains(&caps.max_transfer)
        || caps.min_speed_hz < 10_000
        || caps.max_speed_hz > 1_000_000
        || caps.min_speed_hz > caps.max_speed_hz
        || !(caps.min_speed_hz..=caps.max_speed_hz).contains(&caps.fixed_speed_hz)
        || (!caps.supports_custom_speed
            && (caps.min_speed_hz != caps.fixed_speed_hz
                || caps.max_speed_hz != caps.fixed_speed_hz))
        || (!caps.supports_port_transfer && (caps.port_count != 1 || caps.supports_custom_speed))
        || (caps.supports_batch
            && (!caps.supports_port_transfer
                || caps.port_count != 2
                || !(1..=BATCH_MAX_XFER.min(caps.max_transfer)).contains(&caps.batch_max_transfer)))
        || (!caps.supports_batch && caps.batch_max_transfer != 0)
    {
        return Err("unsupported or inconsistent capabilities data".into());
    }
    Ok(caps)
}

/// `wdata` only: write; `rlen` only: read; both: write, repeated START, read.
pub fn encode_i2c(seq: u8, addr: u8, wdata: &[u8], rlen: usize) -> Result<Vec<u8>, String> {
    validate_i2c(addr, wdata, rlen)?;
    let mut v = header(CMD_I2C, seq, addr, wdata.len(), rlen);
    v.extend_from_slice(wdata);
    Ok(v)
}

fn validate_i2c(addr: u8, wdata: &[u8], rlen: usize) -> Result<(), String> {
    if addr > 0x7F {
        return Err(format!("address 0x{addr:02x} is not a 7-bit address"));
    }
    if wdata.len() > MAX_XFER || rlen > MAX_XFER {
        return Err(format!("at most {MAX_XFER} bytes per write and per read"));
    }
    if wdata.is_empty() && rlen == 0 {
        return Err("nothing to write or read".into());
    }
    Ok(())
}

/// New command: the old I2C request's reserved byte remains reserved.
pub fn encode_i2c_port(seq: u8, request: &TransferRequest) -> Result<Vec<u8>, String> {
    validate_i2c(request.addr, &request.wdata, request.rlen)?;
    if request.port >= MAX_PORTS || !(10_000..=1_000_000).contains(&request.speed_hz) {
        return Err("invalid port or speed".into());
    }
    let mut v = header(
        CMD_I2C_PORT,
        seq,
        request.addr,
        8 + request.wdata.len(),
        request.rlen,
    );
    v.extend_from_slice(&[request.port, 0, 0, 0]);
    v.extend_from_slice(&request.speed_hz.to_le_bytes());
    v.extend_from_slice(&request.wdata);
    Ok(v)
}

fn validate_batch_shape(requests: &[TransferRequest]) -> Result<(), String> {
    if requests.len() != 2 || requests[0].port == requests[1].port {
        return Err("a batch requires exactly one request for CH0 and one for CH1".into());
    }
    for request in requests {
        validate_i2c(request.addr, &request.wdata, request.rlen)?;
        if request.port >= MAX_PORTS || !(10_000..=1_000_000).contains(&request.speed_hz) {
            return Err("invalid batch port or speed".into());
        }
        if request.wdata.len() > BATCH_MAX_XFER || request.rlen > BATCH_MAX_XFER {
            return Err(format!(
                "batch limit is {BATCH_MAX_XFER} bytes per write and per read"
            ));
        }
    }
    Ok(())
}

/// 8-byte common header, 2-byte count/reserved, two 10-byte entries + writes.
/// At 240 bytes per direction the OUT is <=510 and unpadded IN is <=506 bytes.
pub fn encode_batch(seq: u8, requests: &[TransferRequest]) -> Result<Vec<u8>, String> {
    validate_batch_shape(requests)?;
    let wlen = 2 + 20 + requests.iter().map(|r| r.wdata.len()).sum::<usize>();
    let rlen = 2 + 16 + requests.iter().map(|r| r.rlen).sum::<usize>();
    let mut bytes = header(CMD_I2C_BATCH, seq, 0, wlen, rlen);
    bytes.extend_from_slice(&[2, 0]);
    for request in requests {
        bytes.extend_from_slice(&[request.port, request.addr]);
        bytes.extend_from_slice(&(request.wdata.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(request.rlen as u16).to_le_bytes());
        bytes.extend_from_slice(&request.speed_hz.to_le_bytes());
        bytes.extend_from_slice(&request.wdata);
    }
    Ok(bytes)
}

/// Call only after validating the batch. One extra byte is reserved for the
/// deterministic reply pad that prevents a partial-error response ending at MPS.
pub(crate) fn batch_reply_max(requests: &[TransferRequest]) -> usize {
    HDR + 2 + 16 + requests.iter().map(|r| r.rlen).sum::<usize>() + 1
}

/// Validate the entire response before exposing even the first port's result.
pub fn decode_batch(
    reply: &Reply,
    requests: &[TransferRequest],
) -> Result<Vec<TransferOutcome>, String> {
    validate_batch_shape(requests)?;
    if reply.status != Status::Ok || reply.hpm_stat != 0 || reply.data.get(..2) != Some(&[2, 0]) {
        return Err("invalid batch response header/count".into());
    }
    let mut cursor = 2;
    let mut outcomes = Vec::with_capacity(2);
    for request in requests {
        let entry = reply
            .data
            .get(cursor..cursor + 8)
            .ok_or("truncated batch result")?;
        let port = entry[0];
        let status = Status::from(entry[1]);
        let length = u16::from_le_bytes([entry[2], entry[3]]) as usize;
        let hpm_stat = u32::from_le_bytes(entry[4..8].try_into().unwrap());
        if port != request.port
            || matches!(status, Status::Unknown(_))
            || (status == Status::Ok && (length != request.rlen || hpm_stat != 0))
            || (status != Status::Ok && length != 0)
        {
            return Err("invalid batch port/status/length".into());
        }
        cursor += 8;
        let data = reply
            .data
            .get(cursor..cursor + length)
            .ok_or("truncated batch data")?
            .to_vec();
        cursor += length;
        outcomes.push(TransferOutcome {
            port,
            status,
            hpm_stat,
            data,
        });
    }
    // Fixed 64-byte rule at both FS and HS: partial failure changes response
    // length, so it cannot rely on filling the requested IN allocation exactly.
    let expected_tail: &[u8] = if (HDR + cursor).is_multiple_of(64) {
        &[0]
    } else {
        &[]
    };
    if reply.data.get(cursor..) != Some(expected_tail) {
        return Err("invalid batch response trailing bytes or missing USB pad".into());
    }
    Ok(outcomes)
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
    if rsp[3] != 0 {
        return Err("reply reserved byte must be zero".into());
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
        let want = Reply {
            status: Status::Ok,
            hpm_stat: 0,
            data: vec![0x11, 0x22],
        };
        assert_eq!(decode(CMD_I2C, 5, &ok).unwrap(), want);
        let nack = [CMD_I2C, 5, 1, 0, 0xD3, 0x07, 0, 0]; // status_i2c_no_addr_hit = 2003
        let r = decode(CMD_I2C, 5, &nack).unwrap();
        assert_eq!(
            (r.status, r.hpm_stat, r.data.len()),
            (Status::Nack, 2003, 0)
        );
        let odd = [CMD_I2C, 5, 9, 0, 0, 0, 0, 0];
        assert_eq!(decode(CMD_I2C, 5, &odd).unwrap().status, Status::Unknown(9));
        assert!(decode(CMD_I2C, 6, &ok).is_err()); // stale seq
        assert!(decode(CMD_PING, 5, &ok).is_err()); // wrong cmd
        assert!(decode(CMD_I2C, 5, &ok[..7]).is_err());
    }

    fn capability_data() -> Vec<u8> {
        vec![
            1, 2, 5, 0, 0, 1, 0, 0, 0xa0, 0x86, 1, 0, 0x10, 0x27, 0, 0, 0x40, 0x42, 0x0f, 0,
        ]
    }

    #[test]
    fn capabilities_layout_and_strict_fallback() {
        assert_eq!(encode_capabilities(9), [2, 9, 0, 0, 0, 0, 0, 0]);
        let legacy = Reply {
            status: Status::BadRequest,
            hpm_stat: 0,
            data: vec![],
        };
        assert_eq!(
            decode_capabilities(&legacy).unwrap(),
            Capabilities::legacy()
        );
        let caps = decode_capabilities(&Reply {
            status: Status::Ok,
            hpm_stat: 0,
            data: capability_data(),
        })
        .unwrap();
        assert_eq!(caps.port_count, 2);
        assert!(caps.supports_port_transfer && caps.supports_custom_speed);
        assert_eq!((caps.min_speed_hz, caps.max_speed_hz), (10_000, 1_000_000));
        assert_eq!(caps.max_transfer, 256);
        for bad in [
            Reply {
                status: Status::Ok,
                hpm_stat: 0,
                data: vec![],
            },
            Reply {
                status: Status::BadRequest,
                hpm_stat: 1,
                data: vec![],
            },
            Reply {
                status: Status::BadRequest,
                hpm_stat: 0,
                data: vec![0],
            },
            Reply {
                status: Status::Nack,
                hpm_stat: 0,
                data: vec![],
            },
        ] {
            assert!(decode_capabilities(&bad).is_err());
        }
    }

    #[test]
    fn malformed_capability_fields_fail_closed() {
        for (index, value) in [
            (0, 2),
            (1, 0),
            (1, 3),
            (2, 0x85),
            (2, 7),
            (3, 1),
            (5, 2),
            (6, 1),
            (19, 1),
        ] {
            let mut data = capability_data();
            data[index] = value;
            assert!(
                decode_capabilities(&Reply {
                    status: Status::Ok,
                    hpm_stat: 0,
                    data
                })
                .is_err(),
                "index {index}"
            );
        }
        let mut data = capability_data();
        data.pop();
        assert!(decode_capabilities(&Reply {
            status: Status::Ok,
            hpm_stat: 0,
            data
        })
        .is_err());
    }

    #[test]
    fn explicit_port_has_new_command_and_preserves_reserved_byte() {
        let request = TransferRequest {
            port: 1,
            addr: 0x50,
            wdata: vec![0x80],
            rlen: 2,
            speed_hz: 400_000,
        };
        assert_eq!(
            encode_i2c_port(7, &request).unwrap(),
            [0x11, 7, 0x50, 0, 9, 0, 2, 0, 1, 0, 0, 0, 0x80, 0x1a, 6, 0, 0x80]
        );
        assert!(Capabilities::legacy().validate_transfer(&request).is_err());
        let mut legacy_request = request.clone();
        legacy_request.port = 0;
        assert!(Capabilities::legacy()
            .validate_transfer(&legacy_request)
            .is_err());
        legacy_request.speed_hz = 100_000;
        assert!(Capabilities::legacy()
            .validate_transfer(&legacy_request)
            .is_ok());
        for (port, speed_hz) in [(2, 100_000), (0, 9_999), (0, 1_000_001)] {
            let invalid = TransferRequest {
                port,
                speed_hz,
                ..legacy_request.clone()
            };
            assert!(encode_i2c_port(0, &invalid).is_err());
        }
        let max = TransferRequest {
            wdata: vec![0; 256],
            rlen: 256,
            ..legacy_request
        };
        assert_eq!(
            pad_for_mps(encode_i2c_port(0, &max).unwrap(), 64).len(),
            272
        );
    }

    #[test]
    fn nonzero_reply_reserved_byte_is_never_accepted() {
        assert!(decode(CMD_CAPABILITIES, 1, &[2, 1, 3, 1, 0, 0, 0, 0]).is_err());
    }

    fn batch_requests() -> [TransferRequest; 2] {
        [
            TransferRequest {
                port: 0,
                addr: 0x50,
                wdata: vec![0x10, 0xaa],
                rlen: 0,
                speed_hz: 100_000,
            },
            TransferRequest {
                port: 1,
                addr: 0x50,
                wdata: vec![0x20],
                rlen: 2,
                speed_hz: 400_000,
            },
        ]
    }

    #[test]
    fn batch_golden_layout_and_fixed_two_port_contract() {
        let requests = batch_requests();
        assert_eq!(
            encode_batch(9, &requests).unwrap(),
            [
                0x12, 9, 0, 0, 25, 0, 20, 0, 2, 0, 0, 0x50, 2, 0, 0, 0, 0xa0, 0x86, 1, 0, 0x10,
                0xaa, 1, 0x50, 1, 0, 2, 0, 0x80, 0x1a, 6, 0, 0x20,
            ]
        );
        assert!(encode_batch(0, &[]).is_err());
        assert!(encode_batch(0, &requests[..1]).is_err());
        assert!(encode_batch(0, &[requests[0].clone(), requests[0].clone()]).is_err());
        let mut invalid = requests.clone();
        invalid[1].wdata = vec![0; 241];
        assert!(encode_batch(0, &invalid).is_err());
        assert!(Capabilities::legacy().validate_batch(&requests).is_err());
    }

    #[test]
    fn all_batch_lengths_fit_the_existing_usb_buffer_after_padding() {
        for written in 0..=480 {
            let mut requests = batch_requests();
            requests[0].wdata = vec![0; written.min(240)];
            requests[1].wdata = vec![0; written.saturating_sub(240)];
            requests[0].rlen = 240;
            requests[1].rlen = 240;
            let frame = encode_batch(0, &requests).unwrap();
            assert_eq!(frame.len(), 30 + written);
            for mps in [64, 512] {
                assert!(pad_for_mps(frame.clone(), mps).len() < MAX_FRAME);
            }
            assert_eq!(batch_reply_max(&requests), 507);
        }
    }

    #[test]
    fn batch_reply_requires_every_matching_port_and_its_exact_data_length() {
        let requests = batch_requests();
        let data = vec![
            2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 0, 0, 0, 0, 0, 0x11, 0x22,
        ];
        let good = Reply {
            status: Status::Ok,
            hpm_stat: 0,
            data: data.clone(),
        };
        let outcomes = decode_batch(&good, &requests).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[1].data, [0x11, 0x22]);
        for (index, value) in [
            (0, 1),
            (0, 3),
            (1, 1),
            (2, 1),
            (3, 99),
            (6, 1),
            (10, 0),
            (11, 1),
            (12, 1),
        ] {
            let mut bad = data.clone();
            bad[index] = value;
            assert!(
                decode_batch(
                    &Reply {
                        status: Status::Ok,
                        hpm_stat: 0,
                        data: bad
                    },
                    &requests
                )
                .is_err(),
                "index {index}"
            );
        }
        for length in 0..data.len() {
            assert!(decode_batch(
                &Reply {
                    status: Status::Ok,
                    hpm_stat: 0,
                    data: data[..length].to_vec()
                },
                &requests
            )
            .is_err());
        }
        let mut extra = data;
        extra.push(0);
        assert!(decode_batch(
            &Reply {
                status: Status::Ok,
                hpm_stat: 0,
                data: extra
            },
            &requests
        )
        .is_err());
    }
}
