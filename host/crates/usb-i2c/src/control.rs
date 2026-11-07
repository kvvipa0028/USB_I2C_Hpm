//! Port control protocol v1. Advertised limits are contracts, not timing measurements.

use crate::device::DeviceError;
use crate::proto::{Reply, Status, HDR, MAX_PORTS};

pub const CMD_CONTROL_CAPS: u8 = 0x03;
pub const CMD_GET_PORT_STATE: u8 = 0x13;
pub const CMD_CONFIG_PORT: u8 = 0x14;
pub const CMD_TABLE_READ: u8 = 0x15;
pub const CMD_TABLE_WRITE: u8 = 0x16;
pub const CMD_PROBE: u8 = 0x17;
pub const CMD_RECOVER: u8 = 0x18;
pub const CMD_FRONTEND_GET: u8 = 0x19;
pub const CMD_FRONTEND_SET: u8 = 0x1a;
pub const CONTROL_VERSION: u8 = 1;
pub const CONTROL_CAPABILITIES_LEN: usize = 48;
pub const PORT_STATE_LEN: usize = 40;
pub const TABLE_META_LEN: usize = 16;
pub const PROBE_RESULT_LEN: usize = 4;
pub const RECOVERY_RESULT_LEN: usize = 8;
pub const FRONTEND_STATE_LEN: usize = 24;
pub const TABLE_SIZE: u16 = 256;
pub const TABLE_SERVICE_US: u32 = 2_000;
pub const STRETCH_MIN_US: u32 = 1_000;
pub const STRETCH_MAX_US: u32 = 100_000;
pub const STRETCH_DEFAULT_US: u32 = 25_000;
pub const TRANSACTION_MIN_US: u32 = 1_000;
pub const TRANSACTION_MAX_US: u32 = 2_000_000;
pub const TRANSACTION_DEFAULT_US: u32 = 500_000;
pub const CONTROL_DEADLINE_US: u32 = 1_500_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ControlStatus {
    Ok = 0,
    Nack = 1,
    Timeout = 2,
    BadRequest = 3,
    I2cError = 4,
    Busy = 5,
    Unsupported = 6,
    Conflict = 7,
    WrongRole = 8,
    ServiceDeadline = 9,
}

impl TryFrom<u8> for ControlStatus {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::Nack),
            2 => Ok(Self::Timeout),
            3 => Ok(Self::BadRequest),
            4 => Ok(Self::I2cError),
            5 => Ok(Self::Busy),
            6 => Ok(Self::Unsupported),
            7 => Ok(Self::Conflict),
            8 => Ok(Self::WrongRole),
            9 => Ok(Self::ServiceDeadline),
            _ => Err(format!("unknown control status {value}")),
        }
    }
}

impl ControlStatus {
    pub fn rejected_before_execution(self) -> bool {
        matches!(
            self,
            Self::BadRequest | Self::Busy | Self::Unsupported | Self::Conflict | Self::WrongRole
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlCapabilities {
    pub version: Option<u8>,
    pub port: u8,
    pub supports_role_config: bool,
    pub supports_table: bool,
    pub supports_timeouts: bool,
    pub supports_probe: bool,
    pub supports_recover: bool,
    pub supports_frontend_get: bool,
    pub supports_frontend_set: bool,
    pub supports_table_cas: bool,
    pub table_size: u16,
    pub max_table_len: u16,
    pub table_service_us: u32,
    pub stretch_min_us: u32,
    pub stretch_max_us: u32,
    pub stretch_default_us: u32,
    pub transaction_min_us: u32,
    pub transaction_max_us: u32,
    pub transaction_default_us: u32,
    pub control_deadline_us: u32,
    pub frontend_options: u32,
}

impl ControlCapabilities {
    pub fn unsupported(port: u8) -> Self {
        Self {
            version: None,
            port,
            supports_role_config: false,
            supports_table: false,
            supports_timeouts: false,
            supports_probe: false,
            supports_recover: false,
            supports_frontend_get: false,
            supports_frontend_set: false,
            supports_table_cas: false,
            table_size: 0,
            max_table_len: 0,
            table_service_us: 0,
            stretch_min_us: 0,
            stretch_max_us: 0,
            stretch_default_us: 0,
            transaction_min_us: 0,
            transaction_max_us: 0,
            transaction_default_us: 0,
            control_deadline_us: 0,
            frontend_options: 0,
        }
    }

    pub fn supports_frontend_config(&self, config: &FrontendConfig) -> bool {
        self.supports_frontend_set
            && self.frontend_options & (1 << config.source as u8) != 0
            && self.frontend_options & (1 << (8 + config.pullup as u8)) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PortRole {
    Master = 0,
    Slave = 1,
}

impl TryFrom<u8> for PortRole {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Master),
            1 => Ok(Self::Slave),
            _ => Err(format!("unknown port role {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PortActivity {
    Idle = 0,
    Busy = 1,
    Fault = 2,
}

impl TryFrom<u8> for PortActivity {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Idle),
            1 => Ok(Self::Busy),
            2 => Ok(Self::Fault),
            _ => Err(format!("unknown port activity {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortConfig {
    pub role: PortRole,
    pub slave_address: u8,
    pub speed_hz: u32,
    pub stretch_timeout_us: u32,
    pub transaction_timeout_us: u32,
}

impl Default for PortConfig {
    fn default() -> Self {
        Self {
            role: PortRole::Master,
            slave_address: 0x50,
            speed_hz: 100_000,
            stretch_timeout_us: STRETCH_DEFAULT_US,
            transaction_timeout_us: TRANSACTION_DEFAULT_US,
        }
    }
}

impl PortConfig {
    pub fn validate(&self) -> Result<(), String> {
        validate_address(self.slave_address)?;
        if !(10_000..=1_000_000).contains(&self.speed_hz)
            || (self.role == PortRole::Slave
                && !matches!(self.speed_hz, 100_000 | 400_000 | 1_000_000))
        {
            return Err("invalid speed for port role".into());
        }
        if !(STRETCH_MIN_US..=STRETCH_MAX_US).contains(&self.stretch_timeout_us)
            || !(TRANSACTION_MIN_US..=TRANSACTION_MAX_US).contains(&self.transaction_timeout_us)
            || self.transaction_timeout_us < self.stretch_timeout_us
        {
            return Err("invalid stretch or transaction timeout".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortState {
    pub port: u8,
    pub config: PortConfig,
    pub activity: PortActivity,
    /// Register calculation, not a measured SCL frequency.
    pub calculated_speed_hz: u32,
    pub config_generation: u64,
    pub table_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSnapshot {
    pub offset: u16,
    pub data: Vec<u8>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCommit {
    pub offset: u16,
    pub len: u16,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryResult {
    pub port: u8,
    pub clocks_sent: u8,
    pub initial_lines: u8,
    pub final_lines: u8,
    /// 0 success; 1 SCL low; 2 SDA low; 3 deadline; 4 reinitialization failed.
    pub reason: u32,
}

impl RecoveryResult {
    pub fn succeeded(&self) -> bool {
        self.reason == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrontendSource {
    External = 0,
    Internal1V8 = 1,
    Internal3V3 = 2,
    Internal5V = 3,
}

impl TryFrom<u8> for FrontendSource {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::External),
            1 => Ok(Self::Internal1V8),
            2 => Ok(Self::Internal3V3),
            3 => Ok(Self::Internal5V),
            _ => Err(format!("unknown frontend source {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PullupSetting {
    Off = 0,
    Ohm4K7 = 1,
    Ohm2K2 = 2,
}

impl TryFrom<u8> for PullupSetting {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Ohm4K7),
            2 => Ok(Self::Ohm2K2),
            _ => Err(format!("unknown pull-up setting {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontendConfig {
    pub source: FrontendSource,
    pub pullup: PullupSetting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendState {
    pub port: u8,
    pub config: FrontendConfig,
    /// None means the device did not advertise a valid voltage measurement.
    pub measured_vref_mv: Option<u32>,
    pub fault: u16,
    pub config_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub addresses: Vec<u8>,
    pub complete: bool,
    pub cancelled: bool,
    pub error: Option<DeviceError>,
}

fn request(cmd: u8, seq: u8, data: &[u8], rlen: usize) -> Vec<u8> {
    let mut bytes = vec![cmd, seq, 0, 0];
    bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(rlen as u16).to_le_bytes());
    bytes.extend_from_slice(data);
    bytes
}

fn validate_port(port: u8) -> Result<(), String> {
    if port >= MAX_PORTS {
        return Err(format!("invalid port {port}"));
    }
    Ok(())
}

fn validate_address(address: u8) -> Result<(), String> {
    if !(0x08..=0x77).contains(&address) {
        return Err(format!("address 0x{address:02x} is outside 0x08..0x77"));
    }
    Ok(())
}

pub(crate) fn validate_table_range(offset: u16, len: u16) -> Result<(), String> {
    if len == 0 || len > TABLE_SIZE || u32::from(offset) + u32::from(len) > u32::from(TABLE_SIZE) {
        return Err("table range must be 1..256 bytes within the 256-byte table".into());
    }
    Ok(())
}

pub(crate) fn encode_query(cmd: u8, seq: u8, port: u8, rlen: usize) -> Vec<u8> {
    request(cmd, seq, &[port, 0, 0, 0], rlen)
}

pub(crate) fn encode_config(
    seq: u8,
    port: u8,
    config: &PortConfig,
    expected_generation: Option<u64>,
) -> Result<Vec<u8>, String> {
    validate_port(port)?;
    config.validate()?;
    let mut data = vec![
        CONTROL_VERSION,
        port,
        config.role as u8,
        config.slave_address,
    ];
    data.extend_from_slice(&config.speed_hz.to_le_bytes());
    data.extend_from_slice(&config.stretch_timeout_us.to_le_bytes());
    data.extend_from_slice(&config.transaction_timeout_us.to_le_bytes());
    data.extend_from_slice(&u32::from(expected_generation.is_some()).to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&expected_generation.unwrap_or(0).to_le_bytes());
    Ok(request(CMD_CONFIG_PORT, seq, &data, PORT_STATE_LEN))
}

pub(crate) fn encode_table_read(
    seq: u8,
    port: u8,
    offset: u16,
    len: u16,
) -> Result<Vec<u8>, String> {
    validate_port(port)?;
    validate_table_range(offset, len)?;
    let mut data = vec![port, 0];
    data.extend_from_slice(&offset.to_le_bytes());
    data.extend_from_slice(&len.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    Ok(request(
        CMD_TABLE_READ,
        seq,
        &data,
        TABLE_META_LEN + usize::from(len),
    ))
}

pub(crate) fn encode_table_write(
    seq: u8,
    port: u8,
    offset: u16,
    data: &[u8],
    expected_generation: Option<u64>,
) -> Result<Vec<u8>, String> {
    validate_port(port)?;
    let len = u16::try_from(data.len()).map_err(|_| "table write is too long")?;
    validate_table_range(offset, len)?;
    let mut payload = vec![port, u8::from(expected_generation.is_some())];
    payload.extend_from_slice(&offset.to_le_bytes());
    payload.extend_from_slice(&len.to_le_bytes());
    payload.extend_from_slice(&[0, 0]);
    payload.extend_from_slice(&expected_generation.unwrap_or(0).to_le_bytes());
    payload.extend_from_slice(data);
    Ok(request(CMD_TABLE_WRITE, seq, &payload, TABLE_META_LEN))
}

pub(crate) fn encode_probe(seq: u8, port: u8, address: u8) -> Result<Vec<u8>, String> {
    validate_port(port)?;
    validate_address(address)?;
    Ok(request(
        CMD_PROBE,
        seq,
        &[port, address, 0, 0],
        PROBE_RESULT_LEN,
    ))
}

pub(crate) fn encode_frontend_set(
    seq: u8,
    port: u8,
    config: &FrontendConfig,
    expected_generation: u64,
) -> Result<Vec<u8>, String> {
    validate_port(port)?;
    let mut data = vec![port, config.source as u8, config.pullup as u8, 0];
    data.extend_from_slice(&expected_generation.to_le_bytes());
    Ok(request(CMD_FRONTEND_SET, seq, &data, FRONTEND_STATE_LEN))
}

/// The session verifies command, sequence and public-header reserved byte first.
/// This validates the success status, exact data length and unique 64-byte pad.
fn success_data(reply: &Reply, expected: usize) -> Result<&[u8], String> {
    if reply.status != Status::Ok || reply.hpm_stat != 0 {
        return Err("expected a successful control reply with hpm_stat=0".into());
    }
    let padded = (HDR + expected).is_multiple_of(64);
    if reply.data.len() != expected + usize::from(padded) || (padded && reply.data[expected] != 0) {
        return Err("invalid control reply length or USB padding".into());
    }
    Ok(&reply.data[..expected])
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

pub(crate) fn decode_capabilities(reply: &Reply, port: u8) -> Result<ControlCapabilities, String> {
    validate_port(port)?;
    if reply.status == Status::BadRequest && reply.hpm_stat == 0 && reply.data.is_empty() {
        return Ok(ControlCapabilities::unsupported(port));
    }
    let data = success_data(reply, CONTROL_CAPABILITIES_LEN)?;
    let features = u32_at(data, 4);
    if data[0] != CONTROL_VERSION
        || data[1] != port
        || data[2..4] != [0, 0]
        || features & !0xff != 0
    {
        return Err(
            "invalid control capabilities version, port, reserved bytes or features".into(),
        );
    }
    let caps = ControlCapabilities {
        version: Some(CONTROL_VERSION),
        port,
        supports_role_config: features & 1 != 0,
        supports_table: features & 2 != 0,
        supports_timeouts: features & 4 != 0,
        supports_probe: features & 8 != 0,
        supports_recover: features & 16 != 0,
        supports_frontend_get: features & 32 != 0,
        supports_frontend_set: features & 64 != 0,
        supports_table_cas: features & 128 != 0,
        table_size: u16_at(data, 8),
        max_table_len: u16_at(data, 10),
        table_service_us: u32_at(data, 12),
        stretch_min_us: u32_at(data, 16),
        stretch_max_us: u32_at(data, 20),
        stretch_default_us: u32_at(data, 24),
        transaction_min_us: u32_at(data, 28),
        transaction_max_us: u32_at(data, 32),
        transaction_default_us: u32_at(data, 36),
        control_deadline_us: u32_at(data, 40),
        frontend_options: u32_at(data, 44),
    };
    if (caps.supports_table
        && (caps.table_size != TABLE_SIZE
            || caps.max_table_len != TABLE_SIZE
            || caps.table_service_us == 0
            || caps.table_service_us > TABLE_SERVICE_US))
        || (!caps.supports_table
            && (caps.table_size != 0 || caps.max_table_len != 0 || caps.table_service_us != 0))
        || (caps.supports_table_cas && !caps.supports_table)
    {
        return Err("inconsistent table capabilities".into());
    }
    if caps.supports_timeouts {
        if caps.stretch_min_us < STRETCH_MIN_US
            || caps.stretch_max_us > STRETCH_MAX_US
            || caps.stretch_min_us > caps.stretch_max_us
            || !(caps.stretch_min_us..=caps.stretch_max_us).contains(&caps.stretch_default_us)
            || caps.transaction_min_us < TRANSACTION_MIN_US
            || caps.transaction_max_us > TRANSACTION_MAX_US
            || caps.transaction_min_us > caps.transaction_max_us
            || !(caps.transaction_min_us..=caps.transaction_max_us)
                .contains(&caps.transaction_default_us)
            || caps.transaction_default_us < caps.stretch_default_us
            || caps.transaction_max_us < caps.stretch_max_us
        {
            return Err("inconsistent timeout capabilities".into());
        }
    } else if data[16..40].iter().any(|&value| value != 0) {
        return Err("timeout ranges require timeout support".into());
    }
    if (features != 0
        && (caps.control_deadline_us == 0 || caps.control_deadline_us > CONTROL_DEADLINE_US))
        || (features == 0 && caps.control_deadline_us != 0)
    {
        return Err("invalid control deadline".into());
    }
    let has_frontend = caps.supports_frontend_get || caps.supports_frontend_set;
    if caps.frontend_options & !0x70f != 0
        || (!has_frontend && caps.frontend_options != 0)
        || (has_frontend
            && (caps.frontend_options & 0x0f == 0 || caps.frontend_options & 0x700 == 0))
    {
        return Err("inconsistent frontend capabilities".into());
    }
    Ok(caps)
}

pub(crate) fn decode_port_state(reply: &Reply, port: u8) -> Result<PortState, String> {
    validate_port(port)?;
    let data = success_data(reply, PORT_STATE_LEN)?;
    if data[0] != CONTROL_VERSION || data[1] != port || data[5..8] != [0, 0, 0] {
        return Err("invalid port state version, port or reserved bytes".into());
    }
    let config = PortConfig {
        role: PortRole::try_from(data[2])?,
        slave_address: data[4],
        speed_hz: u32_at(data, 8),
        stretch_timeout_us: u32_at(data, 16),
        transaction_timeout_us: u32_at(data, 20),
    };
    config.validate()?;
    let calculated_speed_hz = u32_at(data, 12);
    if calculated_speed_hz == 0 || calculated_speed_hz > 1_000_000 {
        return Err("invalid calculated speed".into());
    }
    Ok(PortState {
        port,
        config,
        activity: PortActivity::try_from(data[3])?,
        calculated_speed_hz,
        config_generation: u64_at(data, 24),
        table_generation: u64_at(data, 32),
    })
}

fn decode_table_meta(data: &[u8], port: u8, offset: u16, len: u16) -> Result<u64, String> {
    validate_port(port)?;
    validate_table_range(offset, len)?;
    if data[0] != port
        || data[1] != 0
        || u16_at(data, 2) != offset
        || u16_at(data, 4) != len
        || data[6..8] != [0, 0]
    {
        return Err("invalid table response port, range or reserved bytes".into());
    }
    Ok(u64_at(data, 8))
}

pub(crate) fn decode_table_snapshot(
    reply: &Reply,
    port: u8,
    offset: u16,
    len: u16,
) -> Result<TableSnapshot, String> {
    validate_table_range(offset, len)?;
    let data = success_data(reply, TABLE_META_LEN + usize::from(len))?;
    let generation = decode_table_meta(data, port, offset, len)?;
    Ok(TableSnapshot {
        offset,
        data: data[TABLE_META_LEN..].to_vec(),
        generation,
    })
}

pub(crate) fn decode_table_commit(
    reply: &Reply,
    port: u8,
    offset: u16,
    len: u16,
) -> Result<TableCommit, String> {
    let data = success_data(reply, TABLE_META_LEN)?;
    let generation = decode_table_meta(data, port, offset, len)?;
    Ok(TableCommit {
        offset,
        len,
        generation,
    })
}

pub(crate) fn decode_probe(reply: &Reply, port: u8, address: u8) -> Result<bool, String> {
    validate_port(port)?;
    validate_address(address)?;
    let data = success_data(reply, PROBE_RESULT_LEN)?;
    if data[0] != port || data[1] != address || data[2] > 1 || data[3] != 0 {
        return Err("invalid probe reply".into());
    }
    Ok(data[2] == 1)
}

pub(crate) fn decode_recovery(reply: &Reply, port: u8) -> Result<RecoveryResult, String> {
    validate_port(port)?;
    let data = success_data(reply, RECOVERY_RESULT_LEN)?;
    let reason = u32_at(data, 4);
    if data[0] != port
        || data[1] > 9
        || data[2] & !3 != 0
        || data[3] & !3 != 0
        || reason > 4
        || (reason == 0 && (data[1] != 9 || data[3] != 3))
    {
        return Err("invalid recovery diagnostics".into());
    }
    Ok(RecoveryResult {
        port,
        clocks_sent: data[1],
        initial_lines: data[2],
        final_lines: data[3],
        reason,
    })
}

pub(crate) fn decode_frontend_state(reply: &Reply, port: u8) -> Result<FrontendState, String> {
    validate_port(port)?;
    let data = success_data(reply, FRONTEND_STATE_LEN)?;
    let valid_flags = u16_at(data, 4);
    if data[0] != CONTROL_VERSION
        || data[1] != port
        || valid_flags & !1 != 0
        || data[12..16] != [0, 0, 0, 0]
    {
        return Err("invalid frontend state version, port, validity or reserved bytes".into());
    }
    Ok(FrontendState {
        port,
        config: FrontendConfig {
            source: FrontendSource::try_from(data[2])?,
            pullup: PullupSetting::try_from(data[3])?,
        },
        measured_vref_mv: (valid_flags & 1 != 0).then(|| u32_at(data, 8)),
        fault: u16_at(data, 6),
        config_generation: u64_at(data, 16),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(mut data: Vec<u8>) -> Reply {
        if (HDR + data.len()).is_multiple_of(64) {
            data.push(0);
        }
        Reply {
            status: Status::Ok,
            hpm_stat: 0,
            data,
        }
    }

    fn caps_data() -> Vec<u8> {
        let mut data = vec![CONTROL_VERSION, 1, 0, 0];
        data.extend_from_slice(&0x9fu32.to_le_bytes());
        data.extend_from_slice(&TABLE_SIZE.to_le_bytes());
        data.extend_from_slice(&TABLE_SIZE.to_le_bytes());
        for value in [
            TABLE_SERVICE_US,
            STRETCH_MIN_US,
            STRETCH_MAX_US,
            STRETCH_DEFAULT_US,
            TRANSACTION_MIN_US,
            TRANSACTION_MAX_US,
            TRANSACTION_DEFAULT_US,
            CONTROL_DEADLINE_US,
            0,
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data
    }

    fn state_data() -> Vec<u8> {
        let mut data = vec![CONTROL_VERSION, 1, 1, 0, 0x50, 0, 0, 0];
        for value in [
            400_000u32,
            399_999,
            STRETCH_DEFAULT_US,
            TRANSACTION_DEFAULT_US,
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&u64::MAX.to_le_bytes());
        data.extend_from_slice(&17u64.to_le_bytes());
        data
    }

    fn table_meta(offset: u16, len: u16) -> Vec<u8> {
        let mut data = vec![1, 0];
        data.extend_from_slice(&offset.to_le_bytes());
        data.extend_from_slice(&len.to_le_bytes());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&u64::MAX.to_le_bytes());
        data
    }

    #[test]
    fn all_control_statuses_are_exact_and_rejections_are_distinct() {
        for value in 0..=9 {
            assert_eq!(ControlStatus::try_from(value).unwrap() as u8, value);
        }
        for value in 10..=u8::MAX {
            assert!(ControlStatus::try_from(value).is_err());
        }
        for value in [3, 5, 6, 7, 8] {
            assert!(ControlStatus::try_from(value)
                .unwrap()
                .rejected_before_execution());
        }
        for value in [0, 1, 2, 4, 9] {
            assert!(!ControlStatus::try_from(value)
                .unwrap()
                .rejected_before_execution());
        }
    }

    #[test]
    fn query_and_config_layouts_preserve_full_width_cas() {
        assert_eq!(
            encode_query(CMD_CONTROL_CAPS, 7, 1, 48),
            [3, 7, 0, 0, 4, 0, 48, 0, 1, 0, 0, 0]
        );
        let config = PortConfig::default();
        for expected in [None, Some(0), Some(u64::MAX)] {
            let bytes = encode_config(7, 1, &config, expected).unwrap();
            assert_eq!(&bytes[..12], &[0x14, 7, 0, 0, 32, 0, 40, 0, 1, 1, 0, 0x50]);
            assert_eq!(u32_at(&bytes, 12), 100_000);
            assert_eq!(u32_at(&bytes, 16), 25_000);
            assert_eq!(u32_at(&bytes, 20), 500_000);
            assert_eq!(u32_at(&bytes, 24), u32::from(expected.is_some()));
            assert_eq!(&bytes[28..32], &[0; 4]);
            assert_eq!(u64_at(&bytes, 32), expected.unwrap_or(0));
            assert_eq!(bytes.len(), 40);
        }
        assert!(encode_config(0, 2, &config, None).is_err());
    }

    #[test]
    fn config_validates_role_specific_speed_address_and_timeout_bounds() {
        for address in [0x08, 0x77] {
            assert!(PortConfig {
                slave_address: address,
                ..PortConfig::default()
            }
            .validate()
            .is_ok());
        }
        for address in [0, 7, 0x78, 0xff] {
            assert!(PortConfig {
                slave_address: address,
                ..PortConfig::default()
            }
            .validate()
            .is_err());
        }
        for speed_hz in [10_000, 50_000, 1_000_000] {
            assert!(PortConfig {
                speed_hz,
                ..PortConfig::default()
            }
            .validate()
            .is_ok());
        }
        for speed_hz in [9_999, 1_000_001] {
            assert!(PortConfig {
                speed_hz,
                ..PortConfig::default()
            }
            .validate()
            .is_err());
        }
        for speed_hz in [100_000, 400_000, 1_000_000] {
            assert!(PortConfig {
                role: PortRole::Slave,
                speed_hz,
                ..PortConfig::default()
            }
            .validate()
            .is_ok());
        }
        assert!(PortConfig {
            role: PortRole::Slave,
            speed_hz: 50_000,
            ..PortConfig::default()
        }
        .validate()
        .is_err());
        for (stretch_timeout_us, transaction_timeout_us) in [
            (0, 500_000),
            (999, 500_000),
            (100_001, 500_000),
            (25_000, 24_999),
            (25_000, 2_000_001),
        ] {
            assert!(PortConfig {
                stretch_timeout_us,
                transaction_timeout_us,
                ..PortConfig::default()
            }
            .validate()
            .is_err());
        }
        assert!(PortConfig {
            stretch_timeout_us: 1_000,
            transaction_timeout_us: 1_000,
            ..PortConfig::default()
        }
        .validate()
        .is_ok());
        assert!(PortConfig {
            stretch_timeout_us: 100_000,
            transaction_timeout_us: 2_000_000,
            ..PortConfig::default()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn capabilities_only_fall_back_on_exact_legacy_rejection() {
        let caps = decode_capabilities(&ok(caps_data()), 1).unwrap();
        assert_eq!(caps.version, Some(1));
        assert!(caps.supports_role_config && caps.supports_table && caps.supports_timeouts);
        assert!(caps.supports_probe && caps.supports_recover && caps.supports_table_cas);
        assert!(!caps.supports_frontend_get && !caps.supports_frontend_set);
        assert_eq!(caps.table_size, 256);
        assert_eq!(caps.transaction_max_us, 2_000_000);
        let legacy = Reply {
            status: Status::BadRequest,
            hpm_stat: 0,
            data: vec![],
        };
        assert_eq!(
            decode_capabilities(&legacy, 1).unwrap(),
            ControlCapabilities::unsupported(1)
        );
        for reply in [
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
                status: Status::Unknown(6),
                hpm_stat: 0,
                data: vec![],
            },
            Reply {
                status: Status::Ok,
                hpm_stat: 0,
                data: vec![],
            },
        ] {
            assert!(decode_capabilities(&reply, 1).is_err());
        }
        assert!(decode_capabilities(&legacy, 2).is_err());
    }

    #[test]
    fn capabilities_reject_malformed_and_inconsistent_fields() {
        let original = caps_data();
        for (offset, value) in [
            (0, 2),
            (1, 0),
            (2, 1),
            (3, 1),
            (5, 1),
            (8, 1),
            (10, 1),
            (44, 1),
        ] {
            let mut data = original.clone();
            data[offset] = value;
            assert!(
                decode_capabilities(&ok(data), 1).is_err(),
                "offset {offset}"
            );
        }
        for (offset, value) in [
            (12, 0),
            (12, 2001),
            (16, 999),
            (20, 100001),
            (24, 100001),
            (28, 999),
            (32, 2000001),
            (36, 999),
            (40, 0),
            (40, 1500001),
        ] {
            let mut data = original.clone();
            data[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
            assert!(
                decode_capabilities(&ok(data), 1).is_err(),
                "field {offset}={value}"
            );
        }
        for flags in [0x9b, 0x9d] {
            let mut data = original.clone();
            data[4] = flags;
            assert!(decode_capabilities(&ok(data), 1).is_err());
        }
        for len in [0, 47, 49] {
            let mut data = original.clone();
            data.resize(len, 0);
            assert!(decode_capabilities(&ok(data), 1).is_err());
        }
        let mut frontend = original;
        frontend[4] |= 0x60;
        frontend[44..48].copy_from_slice(&0x70fu32.to_le_bytes());
        let caps = decode_capabilities(&ok(frontend.clone()), 1).unwrap();
        assert!(caps.supports_frontend_config(&FrontendConfig {
            source: FrontendSource::Internal5V,
            pullup: PullupSetting::Ohm2K2
        }));
        for options in [0, 0xf, 0x700, 0x800, 0x710] {
            frontend[44..48].copy_from_slice(&u32::to_le_bytes(options));
            assert!(decode_capabilities(&ok(frontend.clone()), 1).is_err());
        }
    }

    #[test]
    fn port_state_keeps_generations_and_rejects_invalid_state() {
        let original = state_data();
        let state = decode_port_state(&ok(original.clone()), 1).unwrap();
        assert_eq!(state.config.role, PortRole::Slave);
        assert_eq!(state.calculated_speed_hz, 399_999);
        assert_eq!(state.config_generation, u64::MAX);
        assert_eq!(state.table_generation, 17);
        for (offset, value) in [
            (0, 2),
            (1, 0),
            (2, 2),
            (3, 3),
            (4, 7),
            (5, 1),
            (6, 1),
            (7, 1),
        ] {
            let mut data = original.clone();
            data[offset] = value;
            assert!(decode_port_state(&ok(data), 1).is_err(), "offset {offset}");
        }
        for len in [0, 39, 41] {
            let mut data = original.clone();
            data.resize(len, 0);
            assert!(decode_port_state(&ok(data), 1).is_err());
        }
        for value in [0u32, 1_000_001] {
            let mut data = original.clone();
            data[12..16].copy_from_slice(&value.to_le_bytes());
            assert!(decode_port_state(&ok(data), 1).is_err());
        }
    }

    #[test]
    fn table_requests_are_bounded_nonwrapping_and_encode_cas_zero_distinctly() {
        assert_eq!(
            encode_table_read(3, 1, 0, 256).unwrap(),
            [0x15, 3, 0, 0, 8, 0, 16, 1, 1, 0, 0, 0, 0, 1, 0, 0]
        );
        for expected in [None, Some(0), Some(u64::MAX)] {
            let bytes = encode_table_write(3, 1, 254, &[0xab, 0xcd], expected).unwrap();
            assert_eq!(&bytes[..8], &[0x16, 3, 0, 0, 18, 0, 16, 0]);
            assert_eq!(
                &bytes[8..16],
                &[1, u8::from(expected.is_some()), 254, 0, 2, 0, 0, 0]
            );
            assert_eq!(u64_at(&bytes, 16), expected.unwrap_or(0));
            assert_eq!(&bytes[24..], &[0xab, 0xcd]);
        }
        assert_eq!(
            encode_table_write(0, 0, 0, &[0; 256], None).unwrap().len(),
            280
        );
        for (offset, len) in [(0, 0), (0, 257), (255, 2), (256, 1), (u16::MAX, 1)] {
            assert!(encode_table_read(0, 0, offset, len).is_err());
            assert!(encode_table_write(0, 0, offset, &vec![0; usize::from(len)], None).is_err());
        }
        assert!(encode_table_write(0, 0, 0, &vec![0; 65536], None).is_err());
        assert!(encode_table_read(0, 2, 0, 1).is_err());
    }

    #[test]
    fn table_decoders_validate_request_identity_and_unique_64_byte_pad() {
        for len in [1u16, 40, 104, 168, 232, 256] {
            let mut data = table_meta(0, len);
            data.extend((0..len).map(|n| n as u8));
            let reply = ok(data);
            let snapshot = decode_table_snapshot(&reply, 1, 0, len).unwrap();
            assert_eq!(snapshot.data.len(), usize::from(len));
            assert_eq!(snapshot.generation, u64::MAX);
            assert_eq!(
                reply.data.len(),
                TABLE_META_LEN
                    + usize::from(len)
                    + usize::from((24 + usize::from(len)).is_multiple_of(64))
            );
            let mut too_long = ok(table_meta(0, len));
            too_long.data = reply.data.clone();
            too_long.data.push(0);
            assert!(decode_table_snapshot(&too_long, 1, 0, len).is_err());
            let mut short = Reply {
                status: Status::Ok,
                hpm_stat: 0,
                data: reply.data.clone(),
            };
            short.data.pop();
            assert!(decode_table_snapshot(&short, 1, 0, len).is_err());
            if (24 + usize::from(len)).is_multiple_of(64) {
                let mut wrong_pad = Reply {
                    status: Status::Ok,
                    hpm_stat: 0,
                    data: reply.data.clone(),
                };
                *wrong_pad.data.last_mut().unwrap() = 1;
                assert!(decode_table_snapshot(&wrong_pad, 1, 0, len).is_err());
            }
        }
        let original = table_meta(254, 2);
        assert_eq!(
            decode_table_commit(&ok(original.clone()), 1, 254, 2).unwrap(),
            TableCommit {
                offset: 254,
                len: 2,
                generation: u64::MAX
            }
        );
        for (offset, value) in [(0, 0), (1, 1), (2, 253), (4, 1), (6, 1), (7, 1)] {
            let mut data = original.clone();
            data[offset] = value;
            assert!(decode_table_commit(&ok(data), 1, 254, 2).is_err());
        }
    }

    #[test]
    fn probe_accepts_only_requested_address_and_boolean_presence() {
        assert_eq!(
            encode_probe(4, 1, 0x50).unwrap(),
            [0x17, 4, 0, 0, 4, 0, 4, 0, 1, 0x50, 0, 0]
        );
        for present in [0, 1] {
            assert_eq!(
                decode_probe(&ok(vec![1, 0x50, present, 0]), 1, 0x50).unwrap(),
                present == 1
            );
        }
        for data in [
            vec![0, 0x50, 1, 0],
            vec![1, 0x51, 1, 0],
            vec![1, 0x50, 2, 0],
            vec![1, 0x50, 1, 1],
            vec![1, 0x50, 1],
        ] {
            assert!(decode_probe(&ok(data), 1, 0x50).is_err());
        }
        for address in [7, 0x78, 0xff] {
            assert!(encode_probe(0, 0, address).is_err());
        }
    }

    #[test]
    fn recovery_retains_failure_diagnostics_and_requires_released_lines_for_success() {
        let success = decode_recovery(&ok(vec![1, 9, 0, 3, 0, 0, 0, 0]), 1).unwrap();
        assert!(success.succeeded());
        for reason in 1..=4 {
            let result = decode_recovery(&ok(vec![1, 4, 0, 0, reason, 0, 0, 0]), 1).unwrap();
            assert_eq!(result.reason, u32::from(reason));
            assert!(!result.succeeded());
        }
        for data in [
            vec![0, 9, 0, 3, 0, 0, 0, 0],
            vec![1, 10, 0, 3, 0, 0, 0, 0],
            vec![1, 9, 4, 3, 0, 0, 0, 0],
            vec![1, 9, 0, 7, 0, 0, 0, 0],
            vec![1, 9, 0, 3, 5, 0, 0, 0],
            vec![1, 9, 0, 2, 0, 0, 0, 0],
            vec![1, 8, 0, 3, 0, 0, 0, 0],
        ] {
            assert!(decode_recovery(&ok(data), 1).is_err());
        }
    }

    #[test]
    fn frontend_state_never_presents_unvalidated_voltage_as_measured() {
        let config = FrontendConfig {
            source: FrontendSource::Internal3V3,
            pullup: PullupSetting::Ohm4K7,
        };
        let bytes = encode_frontend_set(7, 1, &config, u64::MAX).unwrap();
        assert_eq!(&bytes[..12], &[0x1a, 7, 0, 0, 12, 0, 24, 0, 1, 2, 1, 0]);
        assert_eq!(u64_at(&bytes, 12), u64::MAX);
        let mut data = vec![1, 1, 2, 1, 0, 0, 0x34, 0x12];
        data.extend_from_slice(&3300u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&u64::MAX.to_le_bytes());
        let state = decode_frontend_state(&ok(data.clone()), 1).unwrap();
        assert_eq!(state.measured_vref_mv, None);
        assert_eq!(state.fault, 0x1234);
        assert_eq!(state.config, config);
        assert_eq!(state.config_generation, u64::MAX);
        data[4] = 1;
        assert_eq!(
            decode_frontend_state(&ok(data.clone()), 1)
                .unwrap()
                .measured_vref_mv,
            Some(3300)
        );
        for (offset, value) in [
            (0, 2),
            (1, 0),
            (2, 4),
            (3, 3),
            (4, 2),
            (5, 1),
            (12, 1),
            (15, 1),
        ] {
            let mut bad = data.clone();
            bad[offset] = value;
            assert!(
                decode_frontend_state(&ok(bad), 1).is_err(),
                "offset {offset}"
            );
        }
    }

    #[test]
    fn success_decoders_do_not_accept_error_status_or_nonzero_hpm_status() {
        for (status, hpm_stat) in [(Status::Nack, 0), (Status::Unknown(5), 0), (Status::Ok, 1)] {
            let reply = Reply {
                status,
                hpm_stat,
                data: state_data(),
            };
            assert!(decode_port_state(&reply, 1).is_err());
        }
    }
}
