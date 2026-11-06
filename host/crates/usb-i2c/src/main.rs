//! CLI for a USB-I2C bridge or the explicitly selected in-process offline model.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use usb_i2c::bridge::list_devices;
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
        }
    }
}

#[derive(Debug)]
struct CliError {
    code: &'static str,
    message: String,
    session_invalidated: bool,
    result_unknown: bool,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            session_invalidated: false,
            result_unknown: false,
        }
    }

    fn value(&self) -> Value {
        json!({
            "code": self.code,
            "message": self.message,
            "session_invalidated": self.session_invalidated,
            "result_unknown": self.result_unknown,
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

    let mut device = if cli.offline {
        Device::offline()
    } else {
        Device::open_usb(None).map_err(|e| CliError::new("unavailable", format!("{e:#}")))?
    };
    match &cli.cmd {
        Cmd::List => unreachable!("listing does not open a device"),
        Cmd::Ping | Cmd::Info => {
            let line = device.ping_line().map_err(|e| CliError {
                code: "operation_failed",
                message: format!("{e:#}"),
                session_invalidated: !device.is_usable(),
                result_unknown: false,
            })?;
            if matches!(cli.cmd, Cmd::Ping) {
                Ok(Output::success(json!({"firmware": line}), line))
            } else {
                let caps = device.capabilities();
                Ok(Output::success(
                    json!({"firmware": line, "capabilities": capability_value(caps)}),
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
            &mut device,
            request(cli.port, cli.speed_hz, *addr, data.clone(), 0),
        ),
        Cmd::Read { addr, len } => run_transfer(
            &mut device,
            request(cli.port, cli.speed_hz, *addr, Vec::new(), *len),
        ),
        Cmd::Wr { addr, len, data } => run_transfer(
            &mut device,
            request(cli.port, cli.speed_hz, *addr, data.clone(), *len),
        ),
        Cmd::Batch { .. } => {
            let outcomes = device.batch(batch.as_ref().expect("batch was parsed"))?;
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
    match output.error.as_ref().map(|error| error.code) {
        Some("invalid_argument") => ExitCode::from(2),
        Some(_) => ExitCode::FAILURE,
        None => ExitCode::SUCCESS,
    }
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
            }),
        };
        let value = envelope("usb", "write", &output);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["result_unknown"], true);
        assert_eq!(value["error"]["session_invalidated"], true);
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
}
