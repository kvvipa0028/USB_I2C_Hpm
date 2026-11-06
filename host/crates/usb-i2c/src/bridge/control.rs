//! Control transactions share the same single-request session and fail-closed cleanup.
use super::*;
use crate::control::*;

#[derive(Debug, Clone)]
pub(crate) struct ControlReplyError {
    pub status: ControlStatus,
    pub hpm_stat: u32,
}

impl std::fmt::Display for ControlReplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "control request: {:?} (hpm_stat {})",
            self.status, self.hpm_stat
        )
    }
}
impl std::error::Error for ControlReplyError {}

pub(super) fn control_transfer_timeout(caps: &ControlCapabilities) -> Duration {
    if caps.supports_timeouts {
        Duration::from_micros(u64::from(caps.transaction_max_us) + 500_000)
    } else {
        TIMEOUT
    }
}

fn control_timeout(caps: &ControlCapabilities) -> Duration {
    if caps.version.is_some() {
        Duration::from_micros(u64::from(caps.control_deadline_us) + 500_000)
    } else {
        TIMEOUT
    }
}

impl Bridge {
    pub fn control_capabilities(&mut self, port: u8) -> anyhow::Result<ControlCapabilities> {
        self.session.transport()?;
        if port >= self.capabilities()?.port_count {
            bail!("port {port} is unavailable");
        }
        if let Some(caps) = self.control_capabilities.iter().find(|c| c.port == port) {
            return Ok(caps.clone());
        }
        let caps = self.session.control_capabilities(port)?;
        self.control_capabilities.push(caps.clone());
        Ok(caps)
    }

    pub fn port_state(&mut self, port: u8) -> anyhow::Result<PortState> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_role_config)?;
        self.session
            .control_query(CMD_GET_PORT_STATE, port, 40, control_timeout(&caps), |r| {
                decode_port_state(r, port)
            })
    }

    pub fn configure_port(
        &mut self,
        port: u8,
        config: &PortConfig,
        expected: Option<u64>,
    ) -> anyhow::Result<PortState> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_role_config)?;
        if caps.supports_timeouts {
            if !(caps.stretch_min_us..=caps.stretch_max_us).contains(&config.stretch_timeout_us)
                || !(caps.transaction_min_us..=caps.transaction_max_us)
                    .contains(&config.transaction_timeout_us)
            {
                bail!("timeouts are outside device capabilities");
            }
        } else {
            let current = self.port_state(port)?;
            require(
                config.stretch_timeout_us == current.config.stretch_timeout_us
                    && config.transaction_timeout_us == current.config.transaction_timeout_us,
            )?;
        }
        let seq = self.session.next_seq();
        let request = encode_config(seq, port, config, expected).map_err(anyhow::Error::msg)?;
        self.session
            .control_execute(CMD_CONFIG_PORT, request, 40, control_timeout(&caps), |r| {
                let state = decode_port_state(r, port)?;
                if state.config != *config
                    || expected.is_some_and(|g| state.config_generation != g.wrapping_add(1))
                {
                    return Err("configuration reply does not confirm the requested commit".into());
                }
                Ok(state)
            })
    }

    pub fn table_read(&mut self, port: u8, offset: u16, len: u16) -> anyhow::Result<TableSnapshot> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_table)?;
        if len > caps.max_table_len
            || u32::from(offset) + u32::from(len) > u32::from(caps.table_size)
        {
            bail!("table range exceeds device capabilities");
        }
        let seq = self.session.next_seq();
        let request = encode_table_read(seq, port, offset, len).map_err(anyhow::Error::msg)?;
        self.session.control_execute(
            CMD_TABLE_READ,
            request,
            16 + usize::from(len),
            control_timeout(&caps),
            |r| decode_table_snapshot(r, port, offset, len),
        )
    }

    pub fn table_write(
        &mut self,
        port: u8,
        offset: u16,
        data: &[u8],
        expected: Option<u64>,
    ) -> anyhow::Result<TableCommit> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_table && (expected.is_none() || caps.supports_table_cas))?;
        if data.len() > usize::from(caps.max_table_len)
            || usize::from(offset) + data.len() > usize::from(caps.table_size)
        {
            bail!("table range exceeds device capabilities");
        }
        let seq = self.session.next_seq();
        let request =
            encode_table_write(seq, port, offset, data, expected).map_err(anyhow::Error::msg)?;
        self.session
            .control_execute(CMD_TABLE_WRITE, request, 16, control_timeout(&caps), |r| {
                let commit = decode_table_commit(r, port, offset, data.len() as u16)?;
                if expected.is_some_and(|g| commit.generation != g.wrapping_add(1)) {
                    return Err("table reply does not confirm the requested CAS commit".into());
                }
                Ok(commit)
            })
    }

    pub fn probe(&mut self, port: u8, address: u8) -> anyhow::Result<bool> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_probe)?;
        let seq = self.session.next_seq();
        let request = encode_probe(seq, port, address).map_err(anyhow::Error::msg)?;
        let reply =
            self.session
                .control_reply(CMD_PROBE, request, 4, control_transfer_timeout(&caps))?;
        if reply.status == Status::Nack {
            return Ok(false);
        }
        require_success(&reply)?;
        decode_probe(&reply, port, address).map_err(|e| self.session.fail(CMD_PROBE, anyhow!(e)))
    }

    pub fn recover(&mut self, port: u8) -> anyhow::Result<RecoveryResult> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_recover)?;
        self.session
            .control_query(CMD_RECOVER, port, 8, control_timeout(&caps), |r| {
                decode_recovery(r, port)
            })
    }

    pub fn frontend_get(&mut self, port: u8) -> anyhow::Result<FrontendState> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_frontend_get)?;
        self.session
            .control_query(CMD_FRONTEND_GET, port, 24, control_timeout(&caps), |r| {
                decode_frontend_state(r, port)
            })
    }

    pub fn frontend_set(
        &mut self,
        port: u8,
        config: &FrontendConfig,
        expected: u64,
    ) -> anyhow::Result<FrontendState> {
        let caps = self.control_capabilities(port)?;
        require(caps.supports_frontend_set)?;
        require(caps.supports_frontend_config(config))?;
        let seq = self.session.next_seq();
        let request =
            encode_frontend_set(seq, port, config, expected).map_err(anyhow::Error::msg)?;
        self.session
            .control_execute(CMD_FRONTEND_SET, request, 24, control_timeout(&caps), |r| {
                let state = decode_frontend_state(r, port)?;
                if state.config != *config || state.config_generation != expected.wrapping_add(1) {
                    return Err("frontend reply does not confirm the requested commit".into());
                }
                Ok(state)
            })
    }
}

fn require(supported: bool) -> anyhow::Result<()> {
    if supported {
        Ok(())
    } else {
        Err(ControlReplyError {
            status: ControlStatus::Unsupported,
            hpm_stat: 0,
        }
        .into())
    }
}

fn control_status(reply: &Reply) -> Result<ControlStatus, String> {
    ControlStatus::try_from(match reply.status {
        Status::Ok => 0,
        Status::Nack => 1,
        Status::Timeout => 2,
        Status::BadRequest => 3,
        Status::I2cError => 4,
        Status::Unknown(v) => v,
    })
}

fn require_success(reply: &Reply) -> anyhow::Result<()> {
    let status = control_status(reply).map_err(anyhow::Error::msg)?;
    if status == ControlStatus::Ok {
        Ok(())
    } else {
        Err(ControlReplyError {
            status,
            hpm_stat: reply.hpm_stat,
        }
        .into())
    }
}

impl<T: Transport> Session<T> {
    pub(super) fn control_capabilities(&mut self, port: u8) -> anyhow::Result<ControlCapabilities> {
        let seq = self.next_seq();
        let request = encode_query(CMD_CONTROL_CAPS, seq, port, 48);
        let reply = self.control_reply(CMD_CONTROL_CAPS, request, 48, TIMEOUT)?;
        // The decoder alone permits the exact legacy BadRequest fallback.
        decode_capabilities(&reply, port).map_err(|e| self.fail(CMD_CONTROL_CAPS, anyhow!(e)))
    }

    pub(super) fn control_query<R>(
        &mut self,
        cmd: u8,
        port: u8,
        len: usize,
        timeout: Duration,
        decode: impl FnOnce(&Reply) -> Result<R, String>,
    ) -> anyhow::Result<R> {
        let seq = self.next_seq();
        self.control_execute(cmd, encode_query(cmd, seq, port, len), len, timeout, decode)
    }

    pub(super) fn control_execute<R>(
        &mut self,
        cmd: u8,
        request: Vec<u8>,
        len: usize,
        timeout: Duration,
        decode: impl FnOnce(&Reply) -> Result<R, String>,
    ) -> anyhow::Result<R> {
        let reply = self.control_reply(cmd, request, len, timeout)?;
        require_success(&reply)?;
        decode(&reply).map_err(|e| self.fail(cmd, anyhow!(e)))
    }

    pub(super) fn control_reply(
        &mut self,
        cmd: u8,
        request: Vec<u8>,
        data_len: usize,
        timeout: Duration,
    ) -> anyhow::Result<Reply> {
        let pad = usize::from((HDR + data_len).is_multiple_of(64));
        let reply = self.transact_timeout(cmd, request, HDR + data_len + pad, timeout)?;
        let status = control_status(&reply).map_err(|e| self.fail(cmd, anyhow!(e)))?;
        let protocol_rejection = matches!(
            status,
            ControlStatus::BadRequest
                | ControlStatus::Busy
                | ControlStatus::Unsupported
                | ControlStatus::Conflict
                | ControlStatus::WrongRole
        );
        if (status != ControlStatus::Ok && !reply.data.is_empty())
            || ((status == ControlStatus::Ok || protocol_rejection) && reply.hpm_stat != 0)
        {
            return Err(self.fail(cmd, anyhow!("invalid control status/data/hpm_stat reply")));
        }
        Ok(reply)
    }
}
