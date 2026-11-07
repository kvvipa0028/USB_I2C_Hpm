//! Explicit real-USB or offline model backend for GUI and CLI.
//!
//! Offline is opt-in, resets on construction and never opens USB. Its two
//! EEPROM tables model functional data only, not timing, electrical behavior,
//! firmware scheduling or evidence that two physical buses run concurrently.

use std::fmt;

use crate::bridge::{BatchRejectedBeforeExecution, Bridge};
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
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DeviceError {}

impl DeviceError {
    fn from_usb(error: anyhow::Error, session_invalidated: bool, has_write: bool) -> Self {
        Self {
            result_unknown: has_write && !error.is::<BatchRejectedBeforeExecution>(),
            message: error.to_string(),
            session_invalidated,
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
}

impl Device {
    /// An absent or broken USB device is an error, never an offline fallback.
    pub fn open_usb(serial: Option<&str>) -> anyhow::Result<Self> {
        let mut bridge = Bridge::open_serial(serial)?;
        let capabilities = bridge.capabilities()?;
        Ok(Self {
            backend: Backend::Usb(bridge),
            capabilities,
        })
    }

    /// Two independent zero-filled 256-byte EEPROMs at address 0x50.
    /// Speed selection is range-checked only; no bus timing is simulated.
    pub fn offline() -> Self {
        Self {
            backend: Backend::Offline(Box::new(Offline {
                tables: [[0; 256]; 2],
                offsets: [0; 2],
            })),
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
            Backend::Offline(model) => Ok(requests
                .iter()
                .map(|request| model.transfer(request))
                .collect()),
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
        })
    }

    fn invalid_request(message: String) -> DeviceError {
        DeviceError {
            message,
            session_invalidated: false,
            result_unknown: false,
        }
    }
}

struct Offline {
    tables: [[u8; 256]; 2],
    offsets: [u8; 2],
}

impl Offline {
    /// The public Device boundary validates port, address, lengths and speed.
    fn transfer(&mut self, request: &TransferRequest) -> TransferOutcome {
        if request.addr != 0x50 {
            return TransferOutcome {
                port: request.port,
                status: Status::Nack,
                hpm_stat: 2003,
                data: vec![],
            };
        }
        let port = request.port as usize;
        let offset = &mut self.offsets[port];
        let table = &mut self.tables[port];
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
