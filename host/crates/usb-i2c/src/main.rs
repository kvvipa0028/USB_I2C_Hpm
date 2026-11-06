//! CLI for a USB-I2C bridge or the explicitly selected in-process offline model.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use usb_i2c::bridge::list_devices;
use usb_i2c::control::{
    ControlCapabilities, ControlStatus, FrontendConfig, FrontendSource, FrontendState,
    PortActivity, PortConfig, PortRole, PortState, PullupSetting, RecoveryResult, ScanResult,
};
use usb_i2c::device::{Device, DeviceError};
use usb_i2c::proto::{Capabilities, Status, TransferRequest};

#[derive(Parser, Debug)]
#[command(
    name = "usb-i2c",
    about = "USB-to-I2C bridge client (address: 0x50 or 80; data bytes: hex)"
)]
struct Cli {
    /// Use a fresh two-port EEPROM model. Never opens USB; data lasts one process.
    #[arg(long, global = true)]
    offline: bool,
    /// I2C port (legacy firmware only supports port 0).
    #[arg(long, global = true, default_value_t = 0)]
    port: u8,
    /// Requested SCL frequency; unsupported settings are rejected, not substituted.
    #[arg(long, global = true, default_value_t = 100_000)]
    speed_hz: u32,
    /// Emit one schema-versioned JSON object, including operational errors.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List connected bridges (offline lists only the model).
    List,
    /// Check the link and print the firmware id.
    Ping,
    /// Read the firmware identity and supported capabilities.
    Info,
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
    /// One bounded transaction on each port; no sequence, loop, or retry.
    Batch {
        #[arg(long, value_parser = parse_addr, default_value = "0x50")]
        ch0_addr: u8,
        /// Space-separated hex bytes, quoted as one argument (e.g. "00 aa").
        #[arg(long, default_value = "")]
        ch0_write: String,
        #[arg(long, default_value_t = 0)]
        ch0_read: usize,
        #[arg(long, value_parser = parse_addr, default_value = "0x50")]
        ch1_addr: u8,
        #[arg(long, default_value = "")]
        ch1_write: String,
        #[arg(long, default_value_t = 0)]
        ch1_read: usize,
    },
    /// Read the selected port's applied configuration and generations.
    PortState,
    /// Apply a complete port configuration (not a partial update).
    PortConfig {
        /// Port role: master or slave.
        #[arg(long, value_parser = parse_role)]
        role: PortRole,
        #[arg(long, value_parser = parse_addr, default_value = "0x50")]
        slave_address: u8,
        #[arg(long, default_value_t = 25_000)]
        stretch_timeout_us: u32,
        #[arg(long, default_value_t = 500_000)]
        transaction_timeout_us: u32,
        /// Compare the current configuration generation before applying.
        #[arg(long)]
        expected_gen: Option<u64>,
    },
    /// Read a non-wrapping fragment of this device's slave table; requires slave role.
    TableRead {
        #[arg(value_parser = parse_offset)]
        offset: u16,
        len: u16,
    },
    /// Commit HEX bytes to the selected slave table, optionally using generation CAS.
    TableWrite {
        #[arg(value_parser = parse_offset)]
        offset: u16,
        #[arg(required = true, value_parser = parse_hex_byte)]
        data: Vec<u8>,
        #[arg(long)]
        expected_gen: Option<u64>,
    },
    /// Probe 0x08..0x77 without sending data; partial scans are failures.
    Scan,
    /// Attempt bounded bus recovery once; a diagnostic failure returns nonzero.
    Recover,
    /// Read frontend state; unsupported hardware is reported explicitly.
    FrontendStatus,
    /// Configure a supported frontend using an explicit configuration generation.
    FrontendConfig {
        /// external, internal1v8, internal3v3, or internal5v.
        #[arg(long, value_parser = parse_frontend_source)]
        source: FrontendSource,
        /// off, 4k7, or 2k2.
        #[arg(long, value_parser = parse_pullup)]
        pullup: PullupSetting,
        #[arg(long)]
        expected_gen: u64,
    },
}

impl Cmd {
    fn name(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Ping => "ping",
            Self::Info => "info",
            Self::Write { .. } => "write",
            Self::Read { .. } => "read",
            Self::Wr { .. } => "wr",
            Self::Batch { .. } => "batch",
            Self::PortState => "port-state",
            Self::PortConfig { .. } => "port-config",
            Self::TableRead { .. } => "table-read",
            Self::TableWrite { .. } => "table-write",
            Self::Scan => "scan",
            Self::Recover => "recover",
            Self::FrontendStatus => "frontend-status",
            Self::FrontendConfig { .. } => "frontend-config",
        }
    }
}

#[derive(Debug)]
struct CliError {
    code: &'static str,
    message: String,
    session_invalidated: bool,
    result_unknown: bool,
    status: Option<ControlStatus>,
    hpm_stat: Option<u32>,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            session_invalidated: false,
            result_unknown: false,
            status: None,
            hpm_stat: None,
        }
    }

    fn value(&self) -> Value {
        json!({
            "code": self.code,
            "message": self.message,
            "session_invalidated": self.session_invalidated,
            "result_unknown": self.result_unknown,
            "status": self.status.map(control_status_name),
            "status_code": self.status.map(|status| status as u8),
            "hpm_stat": self.hpm_stat,
        })
    }
}

impl From<DeviceError> for CliError {
    fn from(error: DeviceError) -> Self {
        Self {
            code: "operation_failed",
            message: error.message,
            session_invalidated: error.session_invalidated,
            result_unknown: error.result_unknown,
            status: error.status,
            hpm_stat: error.hpm_stat,
        }
    }
}

struct Output {
    data: Value,
    text: String,
    error: Option<CliError>,
}

impl Output {
    fn success(data: Value, text: String) -> Self {
        Self {
            data,
            text,
            error: None,
        }
    }
}

fn parse_addr(s: &str) -> Result<u8, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u8::from_str_radix(h, 16),
        None => s.parse::<u8>(),
    };
    r.map_err(|e| format!("bad address '{s}': {e}"))
}

fn parse_hex_byte(s: &str) -> Result<u8, String> {
    let h = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u8::from_str_radix(h, 16).map_err(|e| format!("bad hex byte '{s}': {e}"))
}

fn parse_offset(s: &str) -> Result<u16, String> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16),
        None => s.parse(),
    }
    .map_err(|e| format!("bad table offset '{s}': {e}"))
}

fn parse_role(s: &str) -> Result<PortRole, String> {
    match s {
        "master" => Ok(PortRole::Master),
        "slave" => Ok(PortRole::Slave),
        _ => Err("role must be master or slave".into()),
    }
}

fn parse_frontend_source(s: &str) -> Result<FrontendSource, String> {
    match s {
        "external" => Ok(FrontendSource::External),
        "internal1v8" => Ok(FrontendSource::Internal1V8),
        "internal3v3" => Ok(FrontendSource::Internal3V3),
        "internal5v" => Ok(FrontendSource::Internal5V),
        _ => Err("source must be external, internal1v8, internal3v3, or internal5v".into()),
    }
}

fn parse_pullup(s: &str) -> Result<PullupSetting, String> {
    match s {
        "off" => Ok(PullupSetting::Off),
        "4k7" => Ok(PullupSetting::Ohm4K7),
        "2k2" => Ok(PullupSetting::Ohm2K2),
        _ => Err("pullup must be off, 4k7, or 2k2".into()),
    }
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, CliError> {
    s.split_whitespace()
        .map(parse_hex_byte)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| CliError::new("invalid_argument", e))
}

fn hex_dump(data: &[u8]) -> String {
    data.chunks(16)
        .enumerate()
        .map(|(i, row)| {
            let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
            format!("{:04x}: {}", i * 16, hex.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn capability_value(caps: &Capabilities) -> Value {
    json!({
        "extension_version": caps.extension_version,
        "port_count": caps.port_count,
        "max_transfer": caps.max_transfer,
        "fixed_speed_hz": caps.fixed_speed_hz,
        "min_speed_hz": caps.min_speed_hz,
        "max_speed_hz": caps.max_speed_hz,
        "supports_custom_speed": caps.supports_custom_speed,
        "supports_port_transfer": caps.supports_port_transfer,
        "supports_batch": caps.supports_batch,
        "batch_max_transfer": caps.batch_max_transfer,
    })
}

fn control_capability_value(c: &ControlCapabilities) -> Value {
    json!({
        "version": c.version, "port": c.port,
        "supports_role_config": c.supports_role_config,
        "supports_table": c.supports_table,
        "supports_timeouts": c.supports_timeouts,
        "supports_probe": c.supports_probe,
        "supports_recover": c.supports_recover,
        "supports_frontend_get": c.supports_frontend_get,
        "supports_frontend_set": c.supports_frontend_set,
        "supports_table_cas": c.supports_table_cas,
        "table_size": c.table_size, "max_table_len": c.max_table_len,
        "table_service_us": c.table_service_us,
        "stretch_min_us": c.stretch_min_us, "stretch_max_us": c.stretch_max_us,
        "stretch_default_us": c.stretch_default_us,
        "transaction_min_us": c.transaction_min_us, "transaction_max_us": c.transaction_max_us,
        "transaction_default_us": c.transaction_default_us,
        "control_deadline_us": c.control_deadline_us,
        "frontend_options": c.frontend_options,
        "timing_evidence": "advertised_contract_not_measurement",
    })
}

fn control_status_name(status: ControlStatus) -> &'static str {
    match status {
        ControlStatus::Ok => "ok",
        ControlStatus::Nack => "nack",
        ControlStatus::Timeout => "timeout",
        ControlStatus::BadRequest => "bad_request",
        ControlStatus::I2cError => "i2c_error",
        ControlStatus::Busy => "busy",
        ControlStatus::Unsupported => "unsupported",
        ControlStatus::Conflict => "conflict",
        ControlStatus::WrongRole => "wrong_role",
        ControlStatus::ServiceDeadline => "service_deadline",
    }
}

fn source(offline: bool) -> &'static str {
    if offline {
        "offline_model"
    } else {
        "device_reply"
    }
}

fn port_config(cli: &Cli) -> Option<PortConfig> {
    if let Cmd::PortConfig {
        role,
        slave_address,
        stretch_timeout_us,
        transaction_timeout_us,
        ..
    } = cli.cmd
    {
        Some(PortConfig {
            role,
            slave_address,
            speed_hz: cli.speed_hz,
            stretch_timeout_us,
            transaction_timeout_us,
        })
    } else {
        None
    }
}

fn port_state_output(state: PortState, offline: bool) -> Output {
    let role = match state.config.role {
        PortRole::Master => "master",
        PortRole::Slave => "slave",
    };
    let activity = match state.activity {
        PortActivity::Idle => "idle",
        PortActivity::Busy => "busy",
        PortActivity::Fault => "fault",
    };
    Output::success(json!({
        "port": state.port, "activity": activity,
        "config": {
            "role": role, "slave_address": state.config.slave_address,
            "speed_hz": state.config.speed_hz,
            "stretch_timeout_us": state.config.stretch_timeout_us,
            "transaction_timeout_us": state.config.transaction_timeout_us,
        },
        "calculated_speed_hz": state.calculated_speed_hz,
        "speed_evidence": if offline { "offline_model" } else { "register_calculation" },
        "measured_speed_hz": null,
        "config_generation": state.config_generation,
        "table_generation": state.table_generation,
        "source": source(offline),
    }), format!(
        "CH{} {role} {activity}: applied={} Hz calculated={} Hz (not measured); slave_address=0x{:02x} stretch={} us transaction={} us config_generation={} table_generation={}",
        state.port, state.config.speed_hz, state.calculated_speed_hz, state.config.slave_address,
        state.config.stretch_timeout_us, state.config.transaction_timeout_us,
        state.config_generation, state.table_generation,
    ))
}

fn scan_output(port: u8, result: ScanResult, offline: bool) -> Output {
    let failure = result.error.map(CliError::from);
    let data = json!({
        "port": port, "addresses": result.addresses,
        "address_range": {"first": 0x08, "last": 0x77},
        "complete": result.complete, "cancelled": result.cancelled,
        "error": failure.as_ref().map(CliError::value),
        "source": source(offline), "probe_only": true,
    });
    let error = failure.or_else(|| {
        (!result.complete || result.cancelled).then(|| {
            CliError::new(
                "operation_failed",
                if result.cancelled {
                    "scan cancelled; address set is partial"
                } else {
                    "scan incomplete; address set is partial"
                },
            )
        })
    });
    let addresses = result
        .addresses
        .iter()
        .map(|a| format!("0x{a:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    Output {
        data,
        text: format!(
            "CH{port}: {addresses}\ncomplete={} cancelled={}",
            result.complete, result.cancelled
        ),
        error,
    }
}

fn recovery_output(result: RecoveryResult, offline: bool) -> Output {
    let reason = match result.reason {
        0 => "success",
        1 => "scl_stuck_low",
        2 => "sda_stuck_low",
        3 => "deadline",
        4 => "reinitialization_failed",
        _ => "unknown",
    };
    Output {
        data: json!({
            "port": result.port, "command_completed": true, "succeeded": result.succeeded(),
            "clocks_sent": result.clocks_sent, "initial_lines": result.initial_lines,
            "final_lines": result.final_lines, "reason": result.reason, "reason_name": reason,
            "source": source(offline), "line_state_source": source(offline), "timing_verified": false,
        }),
        text: format!(
            "CH{} recovery {reason}: clocks={} initial_lines={} final_lines={} ({})",
            result.port,
            result.clocks_sent,
            result.initial_lines,
            result.final_lines,
            source(offline)
        ),
        error: (!result.succeeded()).then(|| {
            CliError::new(
                "operation_failed",
                format!("recovery failed: {reason} (reason {})", result.reason),
            )
        }),
    }
}

fn frontend_output(state: FrontendState, offline: bool) -> Output {
    let frontend_source = match state.config.source {
        FrontendSource::External => "external",
        FrontendSource::Internal1V8 => "internal1v8",
        FrontendSource::Internal3V3 => "internal3v3",
        FrontendSource::Internal5V => "internal5v",
    };
    let pullup = match state.config.pullup {
        PullupSetting::Off => "off",
        PullupSetting::Ohm4K7 => "4k7",
        PullupSetting::Ohm2K2 => "2k2",
    };
    Output::success(json!({
        "port": state.port, "config": {"source": frontend_source, "pullup": pullup},
        "config_generation": state.config_generation, "fault": state.fault,
        "voltage_valid": state.measured_vref_mv.is_some(),
        "measured_vref_mv": state.measured_vref_mv,
        "source": source(offline),
    }), format!("CH{} source={frontend_source} pullup={pullup} config_generation={} fault={} measured_vref_mv={}",
        state.port, state.config_generation, state.fault,
        state.measured_vref_mv.map_or_else(|| "unavailable".into(), |mv| mv.to_string())))
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Ok => "ok",
        Status::Nack => "nack",
        Status::Timeout => "timeout",
        Status::BadRequest => "bad_request",
        Status::I2cError => "i2c_error",
        Status::Unknown(_) => "unknown",
    }
}

fn request(port: u8, speed_hz: u32, addr: u8, wdata: Vec<u8>, rlen: usize) -> TransferRequest {
    TransferRequest {
        port,
        addr,
        wdata,
        rlen,
        speed_hz,
    }
}

fn run_transfer(device: &mut Device, req: TransferRequest) -> Result<Output, CliError> {
    let data = device.i2c(&req)?;
    let text = if req.rlen == 0 {
        format!("OK wrote {} bytes to 0x{:02x}", req.wdata.len(), req.addr)
    } else {
        hex_dump(&data)
    };
    Ok(Output::success(
        json!({
            "port": req.port,
            "address": req.addr,
            "speed_hz": req.speed_hz,
            "written": req.wdata.len(),
            "data": data,
        }),
        text,
    ))
}

fn validate_control_arguments(cli: &Cli) -> Result<(), CliError> {
    if let Some(config) = port_config(cli) {
        config
            .validate()
            .map_err(|e| CliError::new("invalid_argument", e))?;
    }
    let range = match &cli.cmd {
        Cmd::TableRead { offset, len } => Some((*offset, usize::from(*len))),
        Cmd::TableWrite { offset, data, .. } => Some((*offset, data.len())),
        _ => None,
    };
    if let Some((offset, len)) = range {
        if !(1..=256).contains(&len) || usize::from(offset) + len > 256 {
            return Err(CliError::new(
                "invalid_argument",
                "table range must be 1..256 bytes within the 256-byte table",
            ));
        }
    }
    Ok(())
}

fn execute(cli: &Cli) -> Result<Output, CliError> {
    if matches!(cli.cmd, Cmd::List) {
        if cli.offline {
            return Ok(Output::success(
                json!({"devices": [{"serial": "offline-model", "speed": null}]}),
                "offline EEPROM model (no USB device)".into(),
            ));
        }
        let devs = list_devices().map_err(|e| CliError::new("unavailable", e.to_string()))?;
        let rows: Vec<Value> = devs
            .iter()
            .map(|d| json!({"serial": d.serial, "speed": d.speed}))
            .collect();
        let text = if devs.is_empty() {
            "no usb_i2c device found".into()
        } else {
            devs.iter()
                .map(|d| format!("usb_i2c serial={} speed={}", d.serial, d.speed))
                .collect::<Vec<_>>()
                .join("\n")
        };
        return Ok(Output::success(json!({"devices": rows}), text));
    }

    // Parse both batch inputs before opening USB or executing either transaction.
    let batch = if let Cmd::Batch {
        ch0_addr,
        ch0_write,
        ch0_read,
        ch1_addr,
        ch1_write,
        ch1_read,
    } = &cli.cmd
    {
        Some([
            request(
                0,
                cli.speed_hz,
                *ch0_addr,
                parse_hex_bytes(ch0_write)?,
                *ch0_read,
            ),
            request(
                1,
                cli.speed_hz,
                *ch1_addr,
                parse_hex_bytes(ch1_write)?,
                *ch1_read,
            ),
        ])
    } else {
        None
    };

    validate_control_arguments(cli)?;
    let mut device = if cli.offline {
        Device::offline()
    } else {
        Device::open_usb(None).map_err(|e| CliError::new("unavailable", format!("{e:#}")))?
    };
    run_on_device(cli, &mut device, batch.as_ref())
}

/// Shared command dispatch. Tests can retain one explicitly created model across
/// requests; public CLI invocations still create a fresh model every process.
fn run_on_device(
    cli: &Cli,
    device: &mut Device,
    batch: Option<&[TransferRequest; 2]>,
) -> Result<Output, CliError> {
    validate_control_arguments(cli)?;
    match &cli.cmd {
        Cmd::List => unreachable!("listing does not open a device"),
        Cmd::Ping | Cmd::Info => {
            let line = device.ping_line().map_err(|e| CliError {
                code: "operation_failed",
                message: format!("{e:#}"),
                session_invalidated: !device.is_usable(),
                result_unknown: false,
                status: None,
                hpm_stat: None,
            })?;
            if matches!(cli.cmd, Cmd::Ping) {
                Ok(Output::success(json!({"firmware": line}), line))
            } else {
                let caps = device.capabilities();
                Ok(Output::success(
                    json!({
                        "firmware": line, "capabilities": capability_value(caps),
                        "control_capabilities": device.control_capabilities().iter().map(control_capability_value).collect::<Vec<_>>(),
                        "source": source(cli.offline),
                    }),
                    format!(
                        "{line}\nports={} speed_hz={}..={} default={} max_transfer={} port_transfer={} batch={} batch_max_transfer={}",
                        caps.port_count, caps.min_speed_hz, caps.max_speed_hz,
                        caps.fixed_speed_hz, caps.max_transfer,
                        caps.supports_port_transfer, caps.supports_batch, caps.batch_max_transfer,
                    ),
                ))
            }
        }
        Cmd::Write { addr, data } => run_transfer(
            device,
            request(cli.port, cli.speed_hz, *addr, data.clone(), 0),
        ),
        Cmd::Read { addr, len } => run_transfer(
            device,
            request(cli.port, cli.speed_hz, *addr, Vec::new(), *len),
        ),
        Cmd::Wr { addr, len, data } => run_transfer(
            device,
            request(cli.port, cli.speed_hz, *addr, data.clone(), *len),
        ),
        Cmd::Batch { .. } => {
            let outcomes = device.batch(batch.expect("batch was parsed"))?;
            let all_ok = outcomes.iter().all(|r| r.status == Status::Ok);
            let rows: Vec<Value> = outcomes
                .iter()
                .map(|r| {
                    json!({
                        "port": r.port,
                        "status": status_name(r.status),
                        "hpm_stat": r.hpm_stat,
                        "data": r.data,
                    })
                })
                .collect();
            let text = outcomes
                .iter()
                .map(|r| {
                    format!(
                        "CH{} {} (hpm_stat {})\n{}",
                        r.port,
                        status_name(r.status),
                        r.hpm_stat,
                        hex_dump(&r.data)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Output {
                data: json!({"outcomes": rows}),
                text,
                error: (!all_ok).then(|| {
                    CliError::new(
                        "batch_failure",
                        "one or more ports failed; inspect each outcome",
                    )
                }),
            })
        }
        Cmd::PortState => Ok(port_state_output(device.port_state(cli.port)?, cli.offline)),
        Cmd::PortConfig { expected_gen, .. } => {
            let config = port_config(cli).expect("configuration command");
            Ok(port_state_output(
                device.configure_port(cli.port, &config, *expected_gen)?,
                cli.offline,
            ))
        }
        Cmd::TableRead { offset, len } => {
            let snapshot = device.table_read(cli.port, *offset, *len)?;
            Ok(Output::success(
                json!({
                    "port": cli.port, "offset": snapshot.offset, "len": snapshot.data.len(),
                    "data": snapshot.data, "generation": snapshot.generation, "source": source(cli.offline),
                }),
                format!(
                    "CH{} table offset={} len={} generation={}\n{}",
                    cli.port,
                    snapshot.offset,
                    snapshot.data.len(),
                    snapshot.generation,
                    hex_dump(&snapshot.data)
                ),
            ))
        }
        Cmd::TableWrite {
            offset,
            data,
            expected_gen,
        } => {
            let commit = device.table_write(cli.port, *offset, data, *expected_gen)?;
            Ok(Output::success(
                json!({
                    "port": cli.port, "offset": commit.offset, "len": commit.len,
                    "generation": commit.generation, "source": source(cli.offline),
                }),
                format!(
                    "CH{} table committed offset={} len={} generation={}",
                    cli.port, commit.offset, commit.len, commit.generation
                ),
            ))
        }
        Cmd::Scan => Ok(scan_output(
            cli.port,
            device.scan(cli.port, || false)?,
            cli.offline,
        )),
        Cmd::Recover => Ok(recovery_output(device.recover(cli.port)?, cli.offline)),
        Cmd::FrontendStatus => Ok(frontend_output(device.frontend_get(cli.port)?, cli.offline)),
        Cmd::FrontendConfig {
            source,
            pullup,
            expected_gen,
        } => Ok(frontend_output(
            device.frontend_set(
                cli.port,
                &FrontendConfig {
                    source: *source,
                    pullup: *pullup,
                },
                *expected_gen,
            )?,
            cli.offline,
        )),
    }
}

fn envelope(backend: &str, command: &str, output: &Output) -> Value {
    json!({
        "schema": "usb-i2c.cli",
        "schema_version": 1,
        "backend": backend,
        "command": command,
        "ok": output.error.is_none(),
        "data": output.data,
        "error": output.error.as_ref().map(CliError::value),
    })
}

fn output_exit_code(output: &Output) -> u8 {
    match output.error.as_ref().map(|error| error.code) {
        Some("invalid_argument") => 2,
        Some(_) => 1,
        None => 0,
    }
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) if e.use_stderr() && args.iter().any(|s| s == "--json") => {
            let backend = if args.iter().any(|s| s == "--offline") {
                "offline"
            } else {
                "usb"
            };
            let output = Output {
                data: Value::Null,
                text: String::new(),
                error: Some(CliError::new("invalid_argument", e.to_string())),
            };
            println!("{}", envelope(backend, "invalid", &output));
            return ExitCode::from(2);
        }
        Err(e) => {
            let code = e.exit_code() as u8;
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    let backend = if cli.offline { "offline" } else { "usb" };
    let output = execute(&cli).unwrap_or_else(|e| Output {
        data: Value::Null,
        text: String::new(),
        error: Some(e),
    });
    if cli.json {
        println!("{}", envelope(backend, cli.cmd.name(), &output));
    } else {
        if cli.offline {
            println!("[offline model — no USB or hardware verification]");
        }
        if !output.text.is_empty() {
            println!("{}", output.text);
        }
        if let Some(error) = &output.error {
            eprintln!("{}: {}", error.code, error.message);
        }
    }
    ExitCode::from(output_exit_code(&output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_syntax_and_global_options_parse() {
        let cli = Cli::try_parse_from(["usb-i2c", "wr", "0x50", "16", "80"]).unwrap();
        assert!(!cli.offline);
        assert_eq!((cli.port, cli.speed_hz), (0, 100_000));
        assert!(matches!(cli.cmd, Cmd::Wr { addr: 0x50, len: 16, data } if data == [0x80]));
        let cli = Cli::try_parse_from([
            "usb-i2c",
            "read",
            "80",
            "2",
            "--offline",
            "--json",
            "--port",
            "1",
        ])
        .unwrap();
        assert!(cli.offline && cli.json);
        assert_eq!(cli.port, 1);
    }

    #[test]
    fn json_failure_preserves_unknown_outcome() {
        let output = Output {
            data: Value::Null,
            text: String::new(),
            error: Some(CliError {
                code: "operation_failed",
                message: "lost confirmation".into(),
                session_invalidated: true,
                result_unknown: true,
                status: Some(ControlStatus::Timeout),
                hpm_stat: Some(2004),
            }),
        };
        let value = envelope("usb", "write", &output);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["result_unknown"], true);
        assert_eq!(value["error"]["session_invalidated"], true);
        assert_eq!(value["error"]["status"], "timeout");
        assert_eq!(value["error"]["status_code"], 2);
        assert_eq!(value["error"]["hpm_stat"], 2004);
    }

    #[test]
    fn bad_batch_hex_is_rejected_before_device_open() {
        let cli = Cli::try_parse_from([
            "usb-i2c",
            "batch",
            "--ch0-write",
            "00 11",
            "--ch1-write",
            "not-hex",
        ])
        .unwrap();
        let error = execute(&cli).err().expect("must reject bad input");
        assert_eq!(error.code, "invalid_argument");
        assert!(!error.result_unknown);
    }

    fn model_command(device: &mut Device, args: &[&str]) -> Result<Output, CliError> {
        let mut argv = vec!["usb-i2c", "--offline", "--json"];
        argv.extend_from_slice(args);
        let cli = Cli::try_parse_from(argv).unwrap();
        run_on_device(&cli, device, None)
    }

    #[test]
    fn control_config_readback_and_table_cas_use_one_explicit_model_session() {
        let mut device = Device::offline();
        let untouched = device.port_state(1).unwrap();
        let configured = model_command(
            &mut device,
            &[
                "--speed-hz",
                "400000",
                "port-config",
                "--role",
                "slave",
                "--slave-address",
                "0x51",
                "--stretch-timeout-us",
                "30000",
                "--transaction-timeout-us",
                "600000",
                "--expected-gen",
                "0",
            ],
        )
        .unwrap();
        assert_eq!(
            configured.data["config"],
            json!({
                "role": "slave", "slave_address": 0x51, "speed_hz": 400000,
                "stretch_timeout_us": 30000, "transaction_timeout_us": 600000,
            })
        );
        assert_eq!(configured.data["config_generation"], 1);
        assert_eq!(configured.data["source"], "offline_model");
        assert_eq!(configured.data["speed_evidence"], "offline_model");
        assert!(configured.data["measured_speed_hz"].is_null());
        assert_eq!(
            model_command(&mut device, &["port-state"]).unwrap().data,
            configured.data
        );
        assert_eq!(device.port_state(1).unwrap(), untouched);

        let committed = model_command(
            &mut device,
            &["table-write", "0xff", "aa", "--expected-gen", "0"],
        )
        .unwrap();
        assert_eq!(committed.data["generation"], 1);
        assert_eq!(committed.data["offset"], 255);
        let before = model_command(&mut device, &["table-read", "255", "1"])
            .unwrap()
            .data;
        assert_eq!(before["data"], json!([0xaa]));
        let stale = model_command(
            &mut device,
            &["table-write", "255", "55", "--expected-gen", "0"],
        )
        .err()
        .unwrap();
        assert_eq!(stale.status, Some(ControlStatus::Conflict));
        assert!(!stale.result_unknown && !stale.session_invalidated);
        assert_eq!(
            model_command(&mut device, &["table-read", "255", "1"])
                .unwrap()
                .data,
            before
        );
        model_command(
            &mut device,
            &["--port", "1", "port-config", "--role", "slave"],
        )
        .unwrap();
        assert_eq!(
            model_command(&mut device, &["--port", "1", "table-read", "255", "1"])
                .unwrap()
                .data["data"],
            json!([0])
        );
        model_command(
            &mut device,
            &["port-config", "--role", "master", "--expected-gen", "1"],
        )
        .unwrap();
        model_command(
            &mut device,
            &["port-config", "--role", "slave", "--expected-gen", "2"],
        )
        .unwrap();
        assert_eq!(
            model_command(&mut device, &["table-read", "255", "1"])
                .unwrap()
                .data,
            before
        );
    }

    #[test]
    fn invalid_control_input_has_no_effect_and_is_rejected_before_usb_open() {
        for args in [
            vec!["port-config", "--role", "slave", "--speed-hz", "200000"],
            vec!["port-config", "--role", "master", "--slave-address", "0x07"],
            vec![
                "port-config",
                "--role",
                "master",
                "--transaction-timeout-us",
                "1000",
            ],
            vec!["table-read", "65535", "1"],
            vec!["table-read", "255", "2"],
            vec!["table-write", "255", "aa", "bb"],
        ] {
            let mut argv = vec!["usb-i2c"];
            argv.extend(args);
            let cli = Cli::try_parse_from(argv).unwrap();
            let error = execute(&cli).err().unwrap();
            assert_eq!(error.code, "invalid_argument");
            assert!(!error.result_unknown);
        }
        let mut device = Device::offline();
        model_command(&mut device, &["port-config", "--role", "slave"]).unwrap();
        model_command(&mut device, &["table-write", "0", "ab"]).unwrap();
        let state = device.port_state(0).unwrap();
        let table = device.table_read(0, 0, 256).unwrap();
        assert!(model_command(&mut device, &["table-write", "255", "aa", "bb"]).is_err());
        assert!(model_command(
            &mut device,
            &["port-config", "--role", "slave", "--speed-hz", "200000"]
        )
        .is_err());
        assert_eq!(device.port_state(0).unwrap(), state);
        assert_eq!(device.table_read(0, 0, 256).unwrap(), table);
    }

    #[test]
    fn wrong_role_and_unsupported_frontend_retain_machine_readable_errors() {
        let mut device = Device::offline();
        assert_eq!(
            model_command(&mut device, &["table-read", "0", "1"])
                .err()
                .unwrap()
                .status,
            Some(ControlStatus::WrongRole)
        );
        let no_frontend = model_command(&mut device, &["frontend-status"])
            .err()
            .unwrap();
        assert_eq!(no_frontend.value()["status"], "unsupported");
        assert_eq!(no_frontend.value()["hpm_stat"], 0);
        assert_eq!(
            model_command(
                &mut device,
                &[
                    "frontend-config",
                    "--source",
                    "internal3v3",
                    "--pullup",
                    "4k7",
                    "--expected-gen",
                    "0"
                ]
            )
            .err()
            .unwrap()
            .status,
            Some(ControlStatus::Unsupported)
        );
        model_command(&mut device, &["port-config", "--role", "slave"]).unwrap();
        assert!(model_command(&mut device, &["read", "0x50", "1"]).is_err());
        assert_eq!(
            model_command(&mut device, &["recover"])
                .err()
                .unwrap()
                .status,
            Some(ControlStatus::WrongRole)
        );
        let scan = model_command(&mut device, &["scan"]).unwrap();
        assert_eq!(output_exit_code(&scan), 1);
        assert_eq!(scan.data["complete"], false);
        assert_eq!(scan.data["error"]["status"], "wrong_role");
    }

    #[test]
    fn partial_scan_and_cancel_never_return_success_or_discard_certainty() {
        let lost = DeviceError {
            message: "lost reply".into(),
            session_invalidated: true,
            result_unknown: false,
            status: Some(ControlStatus::ServiceDeadline),
            hpm_stat: Some(123),
        };
        let partial = scan_output(
            1,
            ScanResult {
                addresses: vec![0x50],
                complete: false,
                cancelled: false,
                error: Some(lost),
            },
            false,
        );
        assert_eq!(output_exit_code(&partial), 1);
        assert_eq!(partial.data["addresses"], json!([0x50]));
        assert_eq!(partial.data["error"]["status_code"], 9);
        assert_eq!(
            partial.error.as_ref().unwrap().value()["session_invalidated"],
            true
        );
        for cancelled in [false, true] {
            let partial = scan_output(
                0,
                ScanResult {
                    addresses: vec![],
                    complete: false,
                    cancelled,
                    error: None,
                },
                true,
            );
            assert_eq!(output_exit_code(&partial), 1);
            assert_eq!(partial.data["cancelled"], cancelled);
        }
    }

    #[test]
    fn recovery_diagnostic_failure_and_absent_voltage_are_not_success_measurements() {
        for reason in 0..=4 {
            let result = recovery_output(
                RecoveryResult {
                    port: 1,
                    clocks_sent: 9,
                    initial_lines: 3,
                    final_lines: 3,
                    reason,
                },
                true,
            );
            assert_eq!(output_exit_code(&result), u8::from(reason != 0));
            assert_eq!(result.data["command_completed"], true);
            assert_eq!(result.data["succeeded"], reason == 0);
            assert_eq!(result.data["line_state_source"], "offline_model");
            assert_eq!(result.data["timing_verified"], false);
        }
        let frontend = frontend_output(
            FrontendState {
                port: 0,
                config: FrontendConfig {
                    source: FrontendSource::External,
                    pullup: PullupSetting::Off,
                },
                measured_vref_mv: None,
                fault: 0,
                config_generation: 7,
            },
            false,
        );
        assert_eq!(frontend.data["voltage_valid"], false);
        assert!(frontend.data["measured_vref_mv"].is_null());
        assert!(frontend.text.contains("unavailable"));
    }
}
