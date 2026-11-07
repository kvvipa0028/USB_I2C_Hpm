//! Open the bridge and run one request at a time.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use nusb::MaybeFuture;

use crate::proto::{
    self, Capabilities, Reply, Status, TransferOutcome, TransferRequest, CAPABILITIES_DATA_LEN,
    CMD_CAPABILITIES, CMD_I2C, CMD_I2C_BATCH, CMD_I2C_PORT, CMD_PING, HDR, MAX_FRAME,
    PING_DATA_MAX,
};

mod control;
mod transport;
use control::control_transfer_timeout;
pub(crate) use control::ControlReplyError;
use transport::{Transport, UsbTransport};

const VID: u16 = 0x1209;
const PID: u16 = 0x0001;
const PRODUCT: &str = "usb_i2c";
const TIMEOUT: Duration = Duration::from_secs(1);
const CLEANUP_TIMEOUT: Duration = Duration::from_millis(100);
const SYNC_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_SYNC_REPLIES: usize = 8;

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
    session: Session<UsbTransport>,
    capabilities: Option<Capabilities>,
    control_capabilities: Vec<crate::control::ControlCapabilities>,
}

/// The exact whole-batch rejection proves neither port was executed.
#[derive(Debug)]
pub(crate) struct BatchRejectedBeforeExecution;

impl std::fmt::Display for BatchRejectedBeforeExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("batch rejected by firmware before execution")
    }
}

impl std::error::Error for BatchRejectedBeforeExecution {}

impl Bridge {
    /// Open a connection and synchronize it using only harmless PING commands.
    /// This never retries an earlier I2C operation or determines its outcome.
    pub fn open() -> anyhow::Result<Self> {
        Self::open_serial(None)
    }

    /// `serial` selects one bridge when more than one is plugged in.
    /// Reopening is deliberate recovery, not a replay of a failed transaction.
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
        let transport = UsbTransport::new(iface)?;
        // Vary the initial sequence, but do not rely on this 8-bit value alone
        // to distinguish a fresh connection from an old reply.
        let seq = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u8)
            .unwrap_or(0);
        let mut session = Session {
            transport: Some(transport),
            seq,
        };
        session.synchronize().map_err(|e| {
            anyhow!("could not synchronize bridge: {e:#}; unplug/replug USB if reconnect fails")
        })?;
        Ok(Bridge {
            session,
            capabilities: None,
            control_capabilities: Vec::new(),
        })
    }

    /// False after a USB or protocol failure. Such a session cannot be reused:
    /// drop it and explicitly open a new connection before sending more work.
    /// A normal I2C NACK or firmware Timeout reply does not invalidate it.
    pub fn is_usable(&self) -> bool {
        self.session.is_usable()
    }

    pub fn ping_line(&mut self) -> anyhow::Result<String> {
        self.session.ping_line()
    }

    pub fn i2c(&mut self, addr: u8, wdata: &[u8], rlen: usize) -> anyhow::Result<Vec<u8>> {
        let caps = self.control_capabilities(0)?;
        self.session
            .i2c_timeout(addr, wdata, rlen, control_transfer_timeout(&caps))
    }

    /// Read-only discovery, cached for this connection. Only a complete old-firmware
    /// BadRequest reply permits legacy fallback; transport failures fail closed.
    pub fn capabilities(&mut self) -> anyhow::Result<Capabilities> {
        self.session.transport()?;
        if let Some(caps) = &self.capabilities {
            return Ok(caps.clone());
        }
        let caps = self.session.capabilities()?;
        self.capabilities = Some(caps.clone());
        Ok(caps)
    }

    /// Explicit port/speed access. Original firmware is limited to CH0/100 kHz;
    /// it is never sent an extension transfer or silently reconfigured.
    pub fn i2c_port(&mut self, request: &TransferRequest) -> anyhow::Result<Vec<u8>> {
        let caps = self.capabilities()?;
        caps.validate_transfer(request).map_err(|e| anyhow!(e))?;
        let control = self.control_capabilities(request.port)?;
        let timeout = control_transfer_timeout(&control);
        if caps.supports_port_transfer {
            self.session.i2c_port_timeout(request, timeout)
        } else {
            self.session
                .i2c_timeout(request.addr, &request.wdata, request.rlen, timeout)
        }
    }

    /// Submit one bounded batch; never emulate hardware concurrency by issuing
    /// two legacy commands when the device does not advertise batch support.
    pub fn batch(&mut self, requests: &[TransferRequest]) -> anyhow::Result<Vec<TransferOutcome>> {
        self.capabilities()?
            .validate_batch(requests)
            .map_err(|e| anyhow!(e))?;
        let mut timeout = Duration::ZERO;
        for request in requests {
            let caps = self.control_capabilities(request.port)?;
            timeout = timeout.max(control_transfer_timeout(&caps));
        }
        self.session.batch_timeout(requests, timeout)
    }
}

// The same request/session code runs against nusb and the focused queue model
// in tests. There is never more than one submitted transfer per endpoint.
struct Session<T> {
    transport: Option<T>,
    seq: u8,
}

impl<T: Transport> Session<T> {
    fn is_usable(&self) -> bool {
        self.transport.is_some()
    }

    fn transport(&mut self) -> anyhow::Result<&mut T> {
        self.transport
            .as_mut()
            .ok_or_else(|| anyhow!("bridge session is unusable; reconnect before another request"))
    }

    fn fail(&mut self, cmd: u8, error: anyhow::Error) -> anyhow::Error {
        // Mark unusable and remove ownership before cleanup. nusb cancellation
        // is asynchronous, and a timed-out OUT may already have reached I2C.
        let mut pending = false;
        if let Some(mut transport) = self.transport.take() {
            transport.cancel_all();
            let deadline = Instant::now() + CLEANUP_TIMEOUT;
            if transport.pending_out() != 0 {
                let _ = transport.wait_out(deadline.saturating_duration_since(Instant::now()));
            }
            if transport.pending_in() != 0 {
                let _ = transport.wait_in(deadline.saturating_duration_since(Instant::now()));
            }
            pending = transport.pending_out() != 0 || transport.pending_in() != 0;
            // Drop endpoints even if cancellation has not finished. nusb owns
            // their buffers until completion; never wait indefinitely here.
        }
        let uncertainty = if matches!(
            cmd,
            CMD_I2C | CMD_I2C_PORT | CMD_I2C_BATCH | 0x14 | 0x16 | 0x18 | 0x1a
        ) {
            "; operation may have executed and was not retried; USB cancellation cannot undo it"
        } else {
            ""
        };
        let cleanup = if pending {
            "; USB cancellation is still pending"
        } else {
            ""
        };
        anyhow!(
            "{error:#}{uncertainty}; bridge session is unusable; reconnect before another request{cleanup}"
        )
    }

    fn transact(&mut self, cmd: u8, req: Vec<u8>, reply_max: usize) -> anyhow::Result<Reply> {
        self.transact_timeout(cmd, req, reply_max, TIMEOUT)
    }

    fn transact_timeout(
        &mut self,
        cmd: u8,
        req: Vec<u8>,
        reply_max: usize,
        timeout: Duration,
    ) -> anyhow::Result<Reply> {
        let transport = self.transport()?;
        let seq = req[1];
        transport.submit_in(reply_max);
        let sent_len = transport.submit_out(req);
        let deadline = Instant::now() + timeout;
        let result = (|| {
            wait_out(
                transport,
                sent_len,
                deadline.saturating_duration_since(Instant::now()),
            )?;
            let bytes = wait_in(
                transport,
                deadline.saturating_duration_since(Instant::now()),
            )?;
            if bytes.len() > reply_max {
                bail!("reply exceeds expected maximum of {reply_max} bytes");
            }
            proto::decode(cmd, seq, &bytes).map_err(|e| anyhow!(e))
        })();
        result.map_err(|e| self.fail(cmd, e))
    }

    fn next_seq(&mut self) -> u8 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn synchronize(&mut self) -> anyhow::Result<()> {
        let result = self.synchronize_inner();
        result.map_err(|e| self.fail(CMD_PING, e))
    }

    fn synchronize_inner(&mut self) -> anyhow::Result<()> {
        let deadline = Instant::now() + SYNC_TIMEOUT;
        // First discard a whole completion. At full speed a stale exact-MPS
        // I2C reply without a ZLP can coalesce with this short PING response.
        // Discard both together rather than guessing boundaries in I2C data.
        let seq = self.next_seq();
        let transport = self.transport()?;
        transport.submit_in(MAX_FRAME);
        let sent_len = transport.submit_out(proto::encode_ping(seq));
        wait_out(
            transport,
            sent_len,
            deadline.saturating_duration_since(Instant::now()),
        )?;
        let _ = wait_in(
            transport,
            deadline.saturating_duration_since(Instant::now()),
        )?;
        let mut replies = 1;
        // With one caller, this firmware has at most one old TX reply and one
        // accepted RX request. The discard removes the first old response.
        // Two distinct PING sequences then protect against a remaining stale
        // PING whose 8-bit seq collides. Fresh PINGs execute after old I2C work.
        // This relies on the current firmware's single RX/TX slots and short
        // PING response, not an unlimited reply queue or a device reset. A
        // lost/corrupted reply fails closed; the old I2C outcome stays unknown.
        for _ in 0..2 {
            if Instant::now() >= deadline {
                bail!("timeout synchronizing bridge");
            }
            if replies >= MAX_SYNC_REPLIES {
                bail!("too many stale replies while synchronizing bridge");
            }
            let seq = self.next_seq();
            let transport = self.transport()?;
            // Keep accepting full-sized completions while draining, but only
            // trust an entire correctly matched PING, never an embedded header.
            transport.submit_in(MAX_FRAME);
            let sent_len = transport.submit_out(proto::encode_ping(seq));
            wait_out(
                transport,
                sent_len,
                deadline.saturating_duration_since(Instant::now()),
            )?;
            loop {
                if replies >= MAX_SYNC_REPLIES {
                    bail!("too many stale replies while synchronizing bridge");
                }
                let bytes = wait_in(
                    transport,
                    deadline.saturating_duration_since(Instant::now()),
                )?;
                replies += 1;
                if let Ok(reply) = proto::decode(CMD_PING, seq, &bytes) {
                    validate_ping(&reply)?;
                    break;
                }
                if replies >= MAX_SYNC_REPLIES || Instant::now() >= deadline {
                    bail!("too many stale replies while synchronizing bridge");
                }
                // Discard only during connection setup, never retry an I2C
                // request after an unexpected reply in an established session.
                transport.submit_in(MAX_FRAME);
            }
        }
        Ok(())
    }

    fn ping_line(&mut self) -> anyhow::Result<String> {
        let seq = self.next_seq();
        let r = self.transact(CMD_PING, proto::encode_ping(seq), HDR + PING_DATA_MAX)?;
        validate_ping(&r).map_err(|e| self.fail(CMD_PING, e))?;
        Ok(format!(
            "protocol {} firmware \"{}\"",
            r.data[0],
            String::from_utf8_lossy(&r.data[1..])
        ))
    }

    fn capabilities(&mut self) -> anyhow::Result<Capabilities> {
        let seq = self.next_seq();
        let reply = self.transact(
            CMD_CAPABILITIES,
            proto::encode_capabilities(seq),
            HDR + CAPABILITIES_DATA_LEN,
        )?;
        proto::decode_capabilities(&reply).map_err(|e| self.fail(CMD_CAPABILITIES, anyhow!(e)))
    }

    #[cfg(test)]
    fn i2c_port(&mut self, request: &TransferRequest) -> anyhow::Result<Vec<u8>> {
        self.i2c_port_timeout(request, TIMEOUT)
    }

    fn i2c_port_timeout(
        &mut self,
        request: &TransferRequest,
        timeout: Duration,
    ) -> anyhow::Result<Vec<u8>> {
        self.transport()?;
        let seq = self.next_seq();
        let encoded = proto::encode_i2c_port(seq, request).map_err(|e| anyhow!(e))?;
        let reply = self.transact_timeout(CMD_I2C_PORT, encoded, HDR + request.rlen, timeout)?;
        if matches!(reply.status, Status::Unknown(_))
            || (reply.status != Status::Ok && !reply.data.is_empty())
            || (reply.status == Status::Ok && reply.hpm_stat != 0)
        {
            return Err(self.fail(CMD_I2C_PORT, anyhow!("invalid port I2C reply")));
        }
        if reply.status != Status::Ok {
            if reply.status == Status::BadRequest && reply.hpm_stat == 0 {
                return Err(ControlReplyError {
                    status: crate::control::ControlStatus::BadRequest,
                    hpm_stat: 0,
                }
                .into());
            }
            bail!(
                "CH{} I2C 0x{:02x}: {:?} (hpm_stat {})",
                request.port,
                request.addr,
                reply.status,
                reply.hpm_stat
            );
        }
        if reply.data.len() != request.rlen {
            return Err(self.fail(
                CMD_I2C_PORT,
                anyhow!("expected {} bytes, got {}", request.rlen, reply.data.len()),
            ));
        }
        Ok(reply.data)
    }

    #[cfg(test)]
    fn batch(&mut self, requests: &[TransferRequest]) -> anyhow::Result<Vec<TransferOutcome>> {
        self.batch_timeout(requests, TIMEOUT)
    }

    fn batch_timeout(
        &mut self,
        requests: &[TransferRequest],
        timeout: Duration,
    ) -> anyhow::Result<Vec<TransferOutcome>> {
        self.transport()?;
        let seq = self.next_seq();
        let encoded = proto::encode_batch(seq, requests).map_err(|e| anyhow!(e))?;
        let reply = self.transact_timeout(
            CMD_I2C_BATCH,
            encoded,
            proto::batch_reply_max(requests),
            timeout,
        )?;
        if reply.status == Status::BadRequest && reply.hpm_stat == 0 && reply.data.is_empty() {
            return Err(BatchRejectedBeforeExecution.into());
        }
        proto::decode_batch(&reply, requests).map_err(|e| self.fail(CMD_I2C_BATCH, anyhow!(e)))
    }

    #[cfg(test)]
    fn i2c(&mut self, addr: u8, wdata: &[u8], rlen: usize) -> anyhow::Result<Vec<u8>> {
        self.i2c_timeout(addr, wdata, rlen, TIMEOUT)
    }

    fn i2c_timeout(
        &mut self,
        addr: u8,
        wdata: &[u8],
        rlen: usize,
        timeout: Duration,
    ) -> anyhow::Result<Vec<u8>> {
        self.transport()?;
        let seq = self.next_seq();
        let req = proto::encode_i2c(seq, addr, wdata, rlen).map_err(|e| anyhow!(e))?;
        let r = self.transact_timeout(CMD_I2C, req, HDR + rlen, timeout)?;
        if matches!(r.status, Status::Unknown(_))
            || (r.status != Status::Ok && !r.data.is_empty())
            || (r.status == Status::Ok && r.hpm_stat != 0)
        {
            return Err(self.fail(CMD_I2C, anyhow!("invalid I2C status reply: {:?}", r.status)));
        }
        if r.status != Status::Ok {
            if r.status == Status::BadRequest && r.hpm_stat == 0 {
                return Err(ControlReplyError {
                    status: crate::control::ControlStatus::BadRequest,
                    hpm_stat: 0,
                }
                .into());
            }
            // These are complete, matched firmware replies, not USB failures.
            // Keeping the session does not imply that retrying an I2C write is safe.
            bail!("I2C 0x{addr:02x}: {:?} (hpm_stat {})", r.status, r.hpm_stat);
        }
        if r.data.len() != rlen {
            return Err(self.fail(
                CMD_I2C,
                anyhow!("expected {rlen} bytes, got {}", r.data.len()),
            ));
        }
        Ok(r.data)
    }
}

fn wait_out(
    transport: &mut impl Transport,
    sent_len: usize,
    timeout: Duration,
) -> anyhow::Result<()> {
    let c = transport
        .wait_out(timeout)
        .ok_or_else(|| anyhow!("timeout sending request"))?;
    c.status.context("sending request")?;
    if c.actual_len != sent_len {
        bail!(
            "short USB request: sent {} of {sent_len} bytes",
            c.actual_len
        );
    }
    Ok(())
}

fn wait_in(transport: &mut impl Transport, timeout: Duration) -> anyhow::Result<Vec<u8>> {
    let c = transport
        .wait_in(timeout)
        .ok_or_else(|| anyhow!("timeout waiting for reply"))?;
    c.status.context("receiving reply")?;
    c.buffer
        .get(..c.actual_len)
        .map(|b| b.to_vec())
        .ok_or_else(|| anyhow!("invalid USB reply length {}", c.actual_len))
}

fn validate_ping(reply: &Reply) -> anyhow::Result<()> {
    if reply.status != Status::Ok || reply.data.is_empty() || reply.data.len() > PING_DATA_MAX {
        bail!(
            "invalid PING reply: {:?} ({} data bytes)",
            reply.status,
            reply.data.len()
        );
    }
    if reply.data[0] != 1 {
        bail!("unsupported bridge protocol {}", reply.data[0]);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
