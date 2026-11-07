//! Explicit real-USB or offline model backend for GUI and CLI.
//!
//! Offline is opt-in, resets on construction and never opens USB. Its two
//! EEPROM tables model functional data only, not timing, electrical behavior,
//! firmware scheduling or evidence that two physical buses run concurrently.

use std::fmt;
use std::time::{Duration, Instant};

use crate::control::*;

use crate::bridge::{BatchRejectedBeforeExecution, Bridge, ControlReplyError};
use crate::proto::{
    Capabilities, Status, TransferOutcome, TransferRequest, BATCH_MAX_XFER, EXTENSION_VERSION,
    MAX_XFER,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Usb,
    Offline,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usb => "usb",
            Self::Offline => "offline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceError {
    pub message: String,
    pub session_invalidated: bool,
    /// A submitted USB write may have partially or fully executed. This is not
    /// cleared by reconnecting, and is never a request to retry automatically.
    pub result_unknown: bool,
    pub status: Option<ControlStatus>,
    pub hpm_stat: Option<u32>,
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DeviceError {}

impl DeviceError {
    fn from_usb(error: anyhow::Error, session_invalidated: bool, has_write: bool) -> Self {
        let control = error.downcast_ref::<ControlReplyError>();
        Self {
            result_unknown: has_write
                && !error.is::<BatchRejectedBeforeExecution>()
                && !control.is_some_and(|e| e.status.rejected_before_execution()),
            message: error.to_string(),
            session_invalidated,
            status: control.map(|e| e.status),
            hpm_stat: control.map(|e| e.hpm_stat),
        }
    }
}

enum Backend {
    Usb(Bridge),
    Offline(Box<Offline>),
}

pub struct Device {
    backend: Backend,
    capabilities: Capabilities,
    control_capabilities: Vec<ControlCapabilities>,
}

impl Device {
    /// An absent or broken USB device is an error, never an offline fallback.
    pub fn open_usb(serial: Option<&str>) -> anyhow::Result<Self> {
        let mut bridge = Bridge::open_serial(serial)?;
        let capabilities = bridge.capabilities()?;
        let control_capabilities = (0..capabilities.port_count)
            .map(|port| bridge.control_capabilities(port))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            backend: Backend::Usb(bridge),
            capabilities,
            control_capabilities,
        })
    }

    /// Two independent zero-filled 256-byte EEPROMs at address 0x50.
    /// Speed selection is range-checked only; no bus timing is simulated.
    pub fn offline() -> Self {
        Self {
            backend: Backend::Offline(Box::new(Offline {
                external_tables: [[0; 256]; 2],
                external_offsets: [0; 2],
                slave_tables: [[0; 256]; 2],
                states: std::array::from_fn(|port| PortState {
                    port: port as u8,
                    config: PortConfig::default(),
                    activity: PortActivity::Idle,
                    calculated_speed_hz: 100_000,
                    config_generation: 0,
                    table_generation: 0,
                }),
            })),
            control_capabilities: (0..2).map(offline_control_capabilities).collect(),
            capabilities: Capabilities {
                extension_version: Some(EXTENSION_VERSION),
                port_count: 2,
                max_transfer: MAX_XFER,
                fixed_speed_hz: 100_000,
                min_speed_hz: 10_000,
                max_speed_hz: 1_000_000,
                supports_custom_speed: true,
                supports_port_transfer: true,
                supports_batch: true,
                batch_max_transfer: BATCH_MAX_XFER,
            },
        }
    }

    pub fn backend(&self) -> BackendKind {
        match self.backend {
            Backend::Usb(_) => BackendKind::Usb,
            Backend::Offline(_) => BackendKind::Offline,
        }
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn is_usable(&self) -> bool {
        match &self.backend {
            Backend::Usb(bridge) => bridge.is_usable(),
            Backend::Offline(_) => true,
        }
    }

    pub fn ping_line(&mut self) -> anyhow::Result<String> {
        match &mut self.backend {
            Backend::Usb(bridge) => bridge.ping_line(),
            Backend::Offline(_) => {
                Ok("offline model: two 256-byte EEPROMs at 0x50; no USB or hardware timing".into())
            }
        }
    }

    pub fn i2c(&mut self, request: &TransferRequest) -> Result<Vec<u8>, DeviceError> {
        self.ready()?;
        self.capabilities
            .validate_transfer(request)
            .map_err(Self::invalid_request)?;
        match &mut self.backend {
            Backend::Usb(bridge) => bridge.i2c_port(request).map_err(|error| {
                DeviceError::from_usb(error, !bridge.is_usable(), !request.wdata.is_empty())
            }),
            Backend::Offline(model) => {
                let result = model.transfer(request);
                if result.status == Status::Ok {
                    Ok(result.data)
                } else {
                    Err(DeviceError {
                        message: format!(
                            "offline CH{} I2C 0x{:02x}: {:?} (hpm_stat {})",
                            request.port, request.addr, result.status, result.hpm_stat
                        ),
                        session_invalidated: false,
                        result_unknown: false,
                        status: None,
                        hpm_stat: Some(result.hpm_stat),
                    })
                }
            }
        }
    }

    pub fn batch(
        &mut self,
        requests: &[TransferRequest],
    ) -> Result<Vec<TransferOutcome>, DeviceError> {
        self.ready()?;
        // Validate both before either model or device can change a byte.
        self.capabilities
            .validate_batch(requests)
            .map_err(Self::invalid_request)?;
        match &mut self.backend {
            Backend::Usb(bridge) => bridge.batch(requests).map_err(|error| {
                DeviceError::from_usb(
                    error,
                    !bridge.is_usable(),
                    requests.iter().any(|r| !r.wdata.is_empty()),
                )
            }),
            // Sequential model evaluation is intentional: it is not a timing
            // simulation or evidence of concurrent execution on hardware.
            Backend::Offline(model) => {
                for request in requests {
                    if model.states[usize::from(request.port)].config.role != PortRole::Master {
                        return Err(DeviceError::control(
                            ControlStatus::BadRequest,
                            "batch rejected before execution: a port is in slave role",
                        ));
                    }
                }
                Ok(requests
                    .iter()
                    .map(|request| model.transfer(request))
                    .collect())
            }
        }
    }

    fn ready(&self) -> Result<(), DeviceError> {
        if self.is_usable() {
            return Ok(());
        }
        Err(DeviceError {
            message: "bridge session is unusable; reconnect before another request".into(),
            session_invalidated: true,
            result_unknown: false, // This new request was not submitted.
            status: None,
            hpm_stat: None,
        })
    }

    fn invalid_request(message: String) -> DeviceError {
        DeviceError {
            message,
            session_invalidated: false,
            result_unknown: false,
            status: None,
            hpm_stat: None,
        }
    }
}

impl DeviceError {
    fn control(status: ControlStatus, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            session_invalidated: false,
            result_unknown: false,
            status: Some(status),
            hpm_stat: Some(0),
        }
    }

    fn from_control_usb(error: anyhow::Error, invalidated: bool, mutating: bool) -> Self {
        let diagnostic = error.downcast_ref::<ControlReplyError>();
        Self {
            message: error.to_string(),
            session_invalidated: invalidated,
            result_unknown: mutating && invalidated,
            status: diagnostic.map(|e| e.status),
            hpm_stat: diagnostic.map(|e| e.hpm_stat),
        }
    }
}

impl Device {
    pub fn control_capabilities(&self) -> &[ControlCapabilities] {
        &self.control_capabilities
    }

    fn control_ready(
        &self,
        port: u8,
        supports: impl FnOnce(&ControlCapabilities) -> bool,
    ) -> Result<&ControlCapabilities, DeviceError> {
        self.ready()?;
        let caps = self
            .control_capabilities
            .get(usize::from(port))
            .ok_or_else(|| Self::invalid_request(format!("port {port} is unavailable")))?;
        if !supports(caps) {
            return Err(DeviceError::control(
                ControlStatus::Unsupported,
                "control operation is unsupported",
            ));
        }
        Ok(caps)
    }

    pub fn port_state(&mut self, port: u8) -> Result<PortState, DeviceError> {
        self.control_ready(port, |c| c.supports_role_config)?;
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .port_state(port)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), false)),
            Backend::Offline(model) => Ok(model.states[usize::from(port)].clone()),
        }
    }

    pub fn configure_port(
        &mut self,
        port: u8,
        config: &PortConfig,
        expected_generation: Option<u64>,
    ) -> Result<PortState, DeviceError> {
        let caps = self
            .control_ready(port, |c| c.supports_role_config)?
            .clone();
        encode_config(0, port, config, expected_generation).map_err(Self::invalid_request)?;
        if config.speed_hz < self.capabilities.min_speed_hz
            || config.speed_hz > self.capabilities.max_speed_hz
        {
            return Err(Self::invalid_request(
                "speed is outside device capabilities".into(),
            ));
        }
        if caps.supports_timeouts {
            if !(caps.stretch_min_us..=caps.stretch_max_us).contains(&config.stretch_timeout_us)
                || !(caps.transaction_min_us..=caps.transaction_max_us)
                    .contains(&config.transaction_timeout_us)
            {
                return Err(Self::invalid_request(
                    "timeouts are outside device capabilities".into(),
                ));
            }
        } else {
            let current = self.port_state(port)?;
            if config.stretch_timeout_us != current.config.stretch_timeout_us
                || config.transaction_timeout_us != current.config.transaction_timeout_us
            {
                return Err(DeviceError::control(
                    ControlStatus::Unsupported,
                    "timeout configuration is unsupported",
                ));
            }
        }
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .configure_port(port, config, expected_generation)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), true)),
            Backend::Offline(model) => {
                let state = &mut model.states[usize::from(port)];
                if expected_generation.is_some_and(|g| g != state.config_generation) {
                    return Err(DeviceError::control(
                        ControlStatus::Conflict,
                        "configuration generation conflict",
                    ));
                }
                state.config = config.clone();
                // Model value only: this backend does not emulate HPM timing registers.
                state.calculated_speed_hz = config.speed_hz;
                state.config_generation = state.config_generation.wrapping_add(1);
                Ok(state.clone())
            }
        }
    }

    pub fn table_read(
        &mut self,
        port: u8,
        offset: u16,
        len: u16,
    ) -> Result<TableSnapshot, DeviceError> {
        let caps = self.control_ready(port, |c| c.supports_table)?;
        encode_table_read(0, port, offset, len).map_err(Self::invalid_request)?;
        if len > caps.max_table_len
            || u32::from(offset) + u32::from(len) > u32::from(caps.table_size)
        {
            return Err(Self::invalid_request(
                "table range exceeds device capabilities".into(),
            ));
        }
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .table_read(port, offset, len)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), false)),
            Backend::Offline(model) => {
                model.require_role(port, PortRole::Slave)?;
                Ok(TableSnapshot {
                    offset,
                    data: model.slave_tables[usize::from(port)]
                        [usize::from(offset)..usize::from(offset + len)]
                        .to_vec(),
                    generation: model.states[usize::from(port)].table_generation,
                })
            }
        }
    }

    pub fn table_write(
        &mut self,
        port: u8,
        offset: u16,
        data: &[u8],
        expected_generation: Option<u64>,
    ) -> Result<TableCommit, DeviceError> {
        let caps = self.control_ready(port, |c| {
            c.supports_table && (expected_generation.is_none() || c.supports_table_cas)
        })?;
        encode_table_write(0, port, offset, data, expected_generation)
            .map_err(Self::invalid_request)?;
        if data.len() > usize::from(caps.max_table_len)
            || usize::from(offset) + data.len() > usize::from(caps.table_size)
        {
            return Err(Self::invalid_request(
                "table range exceeds device capabilities".into(),
            ));
        }
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .table_write(port, offset, data, expected_generation)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), true)),
            Backend::Offline(model) => {
                model.require_role(port, PortRole::Slave)?;
                let state = &mut model.states[usize::from(port)];
                if expected_generation.is_some_and(|g| g != state.table_generation) {
                    return Err(DeviceError::control(
                        ControlStatus::Conflict,
                        "table generation conflict",
                    ));
                }
                model.slave_tables[usize::from(port)]
                    [usize::from(offset)..usize::from(offset) + data.len()]
                    .copy_from_slice(data);
                state.table_generation = state.table_generation.wrapping_add(1);
                Ok(TableCommit {
                    offset,
                    len: data.len() as u16,
                    generation: state.table_generation,
                })
            }
        }
    }

    pub fn probe(&mut self, port: u8, address: u8) -> Result<bool, DeviceError> {
        self.control_ready(port, |c| c.supports_probe)?;
        encode_probe(0, port, address).map_err(Self::invalid_request)?;
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .probe(port, address)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), false)),
            Backend::Offline(model) => {
                model.require_role(port, PortRole::Master)?;
                Ok(address == 0x50)
            }
        }
    }

    pub fn scan(
        &mut self,
        port: u8,
        cancelled: impl FnMut() -> bool,
    ) -> Result<ScanResult, DeviceError> {
        let start = Instant::now();
        self.scan_with_clock(port, cancelled, || start.elapsed())
    }

    fn scan_with_clock(
        &mut self,
        port: u8,
        cancelled: impl FnMut() -> bool,
        elapsed: impl FnMut() -> Duration,
    ) -> Result<ScanResult, DeviceError> {
        self.control_ready(port, |c| c.supports_probe)?;
        Ok(scan_addresses(
            |address| self.probe(port, address),
            cancelled,
            elapsed,
        ))
    }

    pub fn recover(&mut self, port: u8) -> Result<RecoveryResult, DeviceError> {
        self.control_ready(port, |c| c.supports_recover)?;
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .recover(port)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), true)),
            Backend::Offline(model) => {
                model.require_role(port, PortRole::Master)?;
                // Explicit functional model; no GPIO, line sampling or timing occurs.
                Ok(RecoveryResult {
                    port,
                    clocks_sent: 9,
                    initial_lines: 3,
                    final_lines: 3,
                    reason: 0,
                })
            }
        }
    }

    pub fn frontend_get(&mut self, port: u8) -> Result<FrontendState, DeviceError> {
        self.control_ready(port, |c| c.supports_frontend_get)?;
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .frontend_get(port)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), false)),
            Backend::Offline(_) => Err(DeviceError::control(
                ControlStatus::Unsupported,
                "offline model has no frontend measurements",
            )),
        }
    }

    pub fn frontend_set(
        &mut self,
        port: u8,
        config: &FrontendConfig,
        expected_generation: u64,
    ) -> Result<FrontendState, DeviceError> {
        self.control_ready(port, |c| c.supports_frontend_set)?;
        match &mut self.backend {
            Backend::Usb(bridge) => bridge
                .frontend_set(port, config, expected_generation)
                .map_err(|e| DeviceError::from_control_usb(e, !bridge.is_usable(), true)),
            Backend::Offline(_) => Err(DeviceError::control(
                ControlStatus::Unsupported,
                "offline model has no frontend",
            )),
        }
    }
}

fn scan_addresses(
    mut probe: impl FnMut(u8) -> Result<bool, DeviceError>,
    mut cancelled: impl FnMut() -> bool,
    mut elapsed: impl FnMut() -> Duration,
) -> ScanResult {
    let mut result = ScanResult {
        addresses: vec![],
        complete: false,
        cancelled: false,
        error: None,
    };
    for address in 0x08..=0x77 {
        if cancelled() {
            result.cancelled = true;
            return result;
        }
        if elapsed() >= Duration::from_secs(5) {
            result.error = Some(DeviceError::control(
                ControlStatus::ServiceDeadline,
                "scan reached its 5 second deadline",
            ));
            return result;
        }
        match probe(address) {
            Ok(true) => result.addresses.push(address),
            Ok(false) => {}
            Err(error) => {
                result.error = Some(error);
                return result;
            }
        }
    }
    result.complete = true;
    result
}

fn offline_control_capabilities(port: u8) -> ControlCapabilities {
    ControlCapabilities {
        version: Some(1),
        port,
        supports_role_config: true,
        supports_table: true,
        supports_timeouts: true,
        supports_probe: true,
        supports_recover: true,
        supports_frontend_get: false,
        supports_frontend_set: false,
        supports_table_cas: true,
        table_size: 256,
        max_table_len: 256,
        table_service_us: 2000,
        stretch_min_us: 1000,
        stretch_max_us: 100_000,
        stretch_default_us: 25_000,
        transaction_min_us: 1000,
        transaction_max_us: 2_000_000,
        transaction_default_us: 500_000,
        control_deadline_us: 1_500_000,
        frontend_options: 0,
    }
}

struct Offline {
    external_tables: [[u8; 256]; 2],
    external_offsets: [u8; 2],
    slave_tables: [[u8; 256]; 2],
    states: [PortState; 2],
}

impl Offline {
    fn require_role(&self, port: u8, role: PortRole) -> Result<(), DeviceError> {
        if self.states[usize::from(port)].config.role == role {
            Ok(())
        } else {
            Err(DeviceError::control(
                ControlStatus::WrongRole,
                "operation is unavailable in the current port role",
            ))
        }
    }

    /// The public Device boundary validates port, address, lengths and speed.
    fn transfer(&mut self, request: &TransferRequest) -> TransferOutcome {
        if self.states[usize::from(request.port)].config.role != PortRole::Master {
            return TransferOutcome {
                port: request.port,
                status: Status::BadRequest,
                hpm_stat: 0,
                data: vec![],
            };
        }
        if request.addr != 0x50 {
            return TransferOutcome {
                port: request.port,
                status: Status::Nack,
                hpm_stat: 2003,
                data: vec![],
            };
        }
        let port = request.port as usize;
        let offset = &mut self.external_offsets[port];
        let table = &mut self.external_tables[port];
        if let Some((&start, values)) = request.wdata.split_first() {
            *offset = start;
            for &value in values {
                table[*offset as usize] = value;
                *offset = offset.wrapping_add(1);
            }
        }
        let mut data = Vec::with_capacity(request.rlen);
        for _ in 0..request.rlen {
            data.push(table[*offset as usize]);
            *offset = offset.wrapping_add(1);
        }
        TransferOutcome {
            port: request.port,
            status: Status::Ok,
            hpm_stat: 0,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_batch_rejection_has_known_outcome_but_usb_write_failure_does_not() {
        let rejected = DeviceError::from_usb(BatchRejectedBeforeExecution.into(), false, true);
        assert!(!rejected.result_unknown);
        assert!(!rejected.session_invalidated);
        assert!(rejected.message.contains("before execution"));

        let timeout = DeviceError::from_usb(
            std::io::Error::from(std::io::ErrorKind::TimedOut).into(),
            true,
            true,
        );
        assert!(timeout.result_unknown);
        assert!(timeout.session_invalidated);

        // Preserve conservative handling of confirmed I2C write failures.
        let nack = DeviceError::from_usb(anyhow::anyhow!("I2C Nack"), false, true);
        assert!(nack.result_unknown);
        assert!(!nack.session_invalidated);
    }

    fn request(port: u8, wdata: &[u8], rlen: usize) -> TransferRequest {
        TransferRequest {
            port,
            addr: 0x50,
            wdata: wdata.to_vec(),
            rlen,
            speed_hz: 100_000,
        }
    }

    #[test]
    fn offline_is_explicit_zero_initialized_and_recreated_fresh() {
        let mut device = Device::offline();
        assert_eq!(device.backend(), BackendKind::Offline);
        assert!(device.ping_line().unwrap().contains("no USB"));
        assert_eq!(device.i2c(&request(0, &[], 256)).unwrap(), [0; 256]);
        device.i2c(&request(0, &[0, 0xab], 0)).unwrap();
        assert_eq!(device.i2c(&request(0, &[0], 1)).unwrap(), [0xab]);
        assert_eq!(Device::offline().i2c(&request(0, &[0], 1)).unwrap(), [0]);
    }

    #[test]
    fn register_table_wrap_random_and_current_reads_are_independent_per_port() {
        let mut device = Device::offline();
        device
            .i2c(&request(0, &[254, 0xa1, 0xb2, 0xc3, 0xd4], 0))
            .unwrap();
        assert_eq!(
            device.i2c(&request(0, &[254], 3)).unwrap(),
            [0xa1, 0xb2, 0xc3]
        );
        assert_eq!(device.i2c(&request(0, &[], 1)).unwrap(), [0xd4]);
        assert_eq!(device.i2c(&request(1, &[254], 4)).unwrap(), [0; 4]);
        device.i2c(&request(1, &[0, 0x77], 0)).unwrap();
        assert_eq!(device.i2c(&request(0, &[0], 2)).unwrap(), [0xc3, 0xd4]);
    }

    #[test]
    fn absent_address_is_known_nack_and_does_not_move_offset() {
        let mut device = Device::offline();
        device.i2c(&request(0, &[5, 0x42], 0)).unwrap();
        device.i2c(&request(0, &[5], 0)).unwrap();
        let error = device
            .i2c(&TransferRequest {
                addr: 0x51,
                ..request(0, &[99, 1], 1)
            })
            .unwrap_err();
        assert!(error.message.contains("Nack"));
        assert!(!error.session_invalidated && !error.result_unknown);
        assert!(device.is_usable());
        assert_eq!(device.i2c(&request(0, &[], 1)).unwrap(), [0x42]);
    }

    #[test]
    fn invalid_second_batch_entry_cannot_apply_first_write() {
        let mut device = Device::offline();
        for invalid in [
            request(1, &[0; 241], 0),
            request(0, &[0], 1),
            request(1, &[], 0),
            request(2, &[0], 1),
        ] {
            let error = device
                .batch(&[request(0, &[0, 0x99], 0), invalid])
                .unwrap_err();
            assert!(!error.result_unknown && !error.session_invalidated);
            assert_eq!(device.i2c(&request(0, &[0], 1)).unwrap(), [0]);
        }
    }

    #[test]
    fn batch_returns_confirmed_independent_results_even_when_one_nacks() {
        let mut device = Device::offline();
        let outcomes = device
            .batch(&[
                TransferRequest {
                    addr: 0x51,
                    ..request(1, &[0, 0xaa], 0)
                },
                request(0, &[0, 0xbb], 0),
            ])
            .unwrap();
        assert_eq!((outcomes[0].port, outcomes[0].status), (1, Status::Nack));
        assert_eq!((outcomes[1].port, outcomes[1].status), (0, Status::Ok));
        assert_eq!(device.i2c(&request(0, &[0], 1)).unwrap(), [0xbb]);
        assert_eq!(device.i2c(&request(1, &[0], 1)).unwrap(), [0]);
    }

    #[test]
    fn offline_checks_speed_bounds_without_claiming_timing() {
        let mut device = Device::offline();
        for speed_hz in [10_000, 50_000, 100_000, 400_000, 800_000, 1_000_000] {
            assert!(device
                .i2c(&TransferRequest {
                    speed_hz,
                    ..request(0, &[0], 1)
                })
                .is_ok());
        }
        for speed_hz in [9_999, 1_000_001] {
            assert!(device
                .i2c(&TransferRequest {
                    speed_hz,
                    ..request(0, &[0], 1)
                })
                .is_err());
        }
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;

    fn slave() -> PortConfig {
        PortConfig {
            role: PortRole::Slave,
            ..PortConfig::default()
        }
    }

    fn transfer(port: u8, data: &[u8], len: usize) -> TransferRequest {
        TransferRequest {
            port,
            addr: 0x50,
            wdata: data.to_vec(),
            rlen: len,
            speed_hz: 100_000,
        }
    }

    #[test]
    fn slave_table_is_distinct_from_external_eeprom_and_survives_role_changes() {
        let mut device = Device::offline();
        device.i2c(&transfer(0, &[0, 0xee], 0)).unwrap();
        assert_eq!(
            device.table_read(0, 0, 1).unwrap_err().status,
            Some(ControlStatus::WrongRole)
        );
        device.configure_port(0, &slave(), Some(0)).unwrap();
        assert_eq!(device.table_read(0, 0, 256).unwrap().data, [0; 256]);
        let commit = device.table_write(0, 0, &[0xaa], Some(0)).unwrap();
        assert_eq!(commit.generation, 1);
        assert!(device.i2c(&transfer(0, &[0], 1)).is_err());
        assert_eq!(
            device.probe(0, 0x50).unwrap_err().status,
            Some(ControlStatus::WrongRole)
        );
        assert_eq!(
            device.recover(0).unwrap_err().status,
            Some(ControlStatus::WrongRole)
        );
        device.configure_port(1, &slave(), None).unwrap();
        assert_eq!(device.table_read(1, 0, 1).unwrap().data, [0]);
        device
            .configure_port(0, &PortConfig::default(), Some(1))
            .unwrap();
        assert_eq!(device.i2c(&transfer(0, &[0], 1)).unwrap(), [0xee]);
        device.configure_port(0, &slave(), Some(2)).unwrap();
        let snapshot = device.table_read(0, 0, 1).unwrap();
        assert_eq!((snapshot.data, snapshot.generation), (vec![0xaa], 1));
    }

    #[test]
    fn table_lengths_offsets_and_cas_are_atomic_and_port_scoped() {
        let mut device = Device::offline();
        device.configure_port(0, &slave(), None).unwrap();
        device.configure_port(1, &slave(), None).unwrap();
        let original = vec![0x55; 256];
        assert_eq!(
            device
                .table_write(0, 0, &original, Some(0))
                .unwrap()
                .generation,
            1
        );
        for (offset, len) in [(0, 1), (255, 1), (1, 255), (0, 256)] {
            assert_eq!(
                device.table_read(0, offset, len).unwrap().data,
                vec![0x55; usize::from(len)]
            );
        }
        for (offset, len) in [(0, 0), (256, 1), (255, 2), (1, 256), (u16::MAX, 1)] {
            assert!(device.table_read(0, offset, len).is_err());
            assert!(device
                .table_write(0, offset, &vec![0x99; usize::from(len)], None)
                .is_err());
        }
        let rejected = device.table_write(0, 255, &[0x99], Some(0)).unwrap_err();
        assert_eq!(rejected.status, Some(ControlStatus::Conflict));
        assert!(!rejected.result_unknown && !rejected.session_invalidated);
        assert_eq!(device.table_read(0, 0, 256).unwrap().data, original);
        assert_eq!(
            device
                .table_write(0, 255, &[0x99], Some(1))
                .unwrap()
                .generation,
            2
        );
        assert_eq!(device.table_read(0, 254, 2).unwrap().data, [0x55, 0x99]);
        assert_eq!(device.table_read(1, 0, 256).unwrap().data, [0; 256]);
        assert_eq!(device.port_state(1).unwrap().table_generation, 0);
    }

    #[test]
    fn configuration_conflict_and_invalid_fields_leave_both_ports_unchanged() {
        let mut device = Device::offline();
        let initial0 = device.port_state(0).unwrap();
        let initial1 = device.port_state(1).unwrap();
        assert_eq!(
            device
                .configure_port(0, &slave(), Some(1))
                .unwrap_err()
                .status,
            Some(ControlStatus::Conflict)
        );
        for config in [
            PortConfig {
                slave_address: 0x07,
                ..slave()
            },
            PortConfig {
                speed_hz: 200_000,
                ..slave()
            },
            PortConfig {
                speed_hz: 9_999,
                ..PortConfig::default()
            },
            PortConfig {
                stretch_timeout_us: 999,
                ..PortConfig::default()
            },
            PortConfig {
                transaction_timeout_us: 1_000,
                ..PortConfig::default()
            },
            PortConfig {
                transaction_timeout_us: 2_000_001,
                ..PortConfig::default()
            },
        ] {
            assert!(device.configure_port(0, &config, None).is_err());
        }
        assert_eq!(device.port_state(0).unwrap(), initial0);
        assert_eq!(device.port_state(1).unwrap(), initial1);
        let state = device.configure_port(0, &slave(), Some(0)).unwrap();
        assert_eq!(state.config_generation, 1);
        assert_eq!(device.port_state(1).unwrap(), initial1);
        assert!(device.port_state(2).is_err());
    }

    #[test]
    fn slave_in_second_batch_slot_rejects_before_first_external_write() {
        let mut device = Device::offline();
        device.configure_port(1, &slave(), None).unwrap();
        let err = device
            .batch(&[transfer(0, &[0, 0x99], 0), transfer(1, &[0], 1)])
            .unwrap_err();
        assert_eq!(err.status, Some(ControlStatus::BadRequest));
        assert!(!err.result_unknown);
        assert_eq!(device.i2c(&transfer(0, &[0], 1)).unwrap(), [0]);
    }

    #[test]
    fn scan_does_not_move_eeprom_offset_and_preserves_partial_cancellation() {
        let mut device = Device::offline();
        device.i2c(&transfer(0, &[7, 0xab], 0)).unwrap();
        device.i2c(&transfer(0, &[7], 0)).unwrap();
        let full = device.scan(0, || false).unwrap();
        assert_eq!(full.addresses, [0x50]);
        assert!(full.complete && !full.cancelled && full.error.is_none());
        assert_eq!(device.i2c(&transfer(0, &[], 1)).unwrap(), [0xab]);
        let mut boundaries = 0;
        let partial = device
            .scan(0, || {
                boundaries += 1;
                boundaries > 80
            })
            .unwrap();
        assert_eq!(partial.addresses, [0x50]);
        assert!(!partial.complete && partial.cancelled && partial.error.is_none());
        let cancelled = device.scan(0, || true).unwrap();
        assert!(cancelled.cancelled && cancelled.addresses.is_empty());
        assert!(device.scan(2, || false).is_err());
    }

    #[test]
    fn scan_deadline_and_transport_error_preserve_partial_result_and_certainty() {
        let mut visited = Vec::new();
        let mut elapsed_calls = 0;
        let result = scan_addresses(
            |address| {
                visited.push(address);
                Ok(address == 0x50)
            },
            || false,
            || {
                elapsed_calls += 1;
                if elapsed_calls > 80 {
                    Duration::from_secs(5)
                } else {
                    Duration::ZERO
                }
            },
        );
        assert_eq!(result.addresses, [0x50]);
        assert!(!result.complete && !result.cancelled);
        assert_eq!(
            result.error.unwrap().status,
            Some(ControlStatus::ServiceDeadline)
        );
        assert_eq!(visited.len(), 80);
        let expected = DeviceError {
            message: "USB lost".into(),
            session_invalidated: true,
            result_unknown: false,
            status: None,
            hpm_stat: None,
        };
        let result = scan_addresses(
            |address| {
                if address == 0x52 {
                    Err(expected.clone())
                } else {
                    Ok(address == 0x50)
                }
            },
            || false,
            || Duration::ZERO,
        );
        assert_eq!(result.addresses, [0x50]);
        assert!(!result.complete && !result.cancelled);
        assert_eq!(result.error, Some(expected));
    }

    #[test]
    fn frontend_is_unsupported_and_control_errors_keep_machine_readable_certainty() {
        let mut device = Device::offline();
        assert!(device
            .control_capabilities()
            .iter()
            .all(|c| !c.supports_frontend_get && !c.supports_frontend_set));
        let error = device.frontend_get(0).unwrap_err();
        assert_eq!(error.status, Some(ControlStatus::Unsupported));
        assert!(!error.result_unknown && !error.session_invalidated);
        let config = FrontendConfig {
            source: FrontendSource::Internal3V3,
            pullup: PullupSetting::Ohm4K7,
        };
        assert_eq!(
            device.frontend_set(0, &config, 0).unwrap_err().status,
            Some(ControlStatus::Unsupported)
        );
        assert!(device.recover(0).unwrap().succeeded());
        for status in [
            ControlStatus::Busy,
            ControlStatus::Conflict,
            ControlStatus::WrongRole,
            ControlStatus::Unsupported,
        ] {
            let error = DeviceError::from_control_usb(
                ControlReplyError {
                    status,
                    hpm_stat: 0,
                }
                .into(),
                false,
                true,
            );
            assert_eq!(error.status, Some(status));
            assert!(!error.result_unknown && !error.session_invalidated);
        }
        for mutating in [false, true] {
            let error =
                DeviceError::from_control_usb(anyhow::anyhow!("USB timeout"), true, mutating);
            assert_eq!(error.result_unknown, mutating);
            assert!(error.session_invalidated);
        }
    }
}
