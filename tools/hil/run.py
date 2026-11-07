#!/usr/bin/env python3
"""CLI-based test scaffold. Offline PASS is model evidence, never hardware PASS."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import platform
import subprocess
import sys
import time

from adapters.cli import CliAdapter
from cases.offline import build_cases


ROOT = Path(__file__).resolve().parents[2]
HARDWARE_CASES = [
    ("TC-S1-SLV-100K", "100 kHz continuous table access, 200 rounds and 10 min, zero mismatches"),
    ("TC-S1-SLV-400K", "400 kHz continuous table access, 200 rounds and 10 min, zero mismatches"),
    ("TC-S1-SLV-1M", "1 MHz continuous table access, 200 rounds and 10 min, zero mismatches; ISR latency"),
    ("TC-S1-TIMING", "10k/50k/100k/400k/800k/1M measured SCL error <= 5%; high/low timing"),
    ("TC-S2-COMPAT", "old host/new firmware, new host/old firmware, new host/new firmware"),
    ("TC-S2-MM-OVERLAP", "logic analyzer proves overlapping CH0/CH1 master bus activity"),
    ("TC-S2-FAULT-ISOLATION", "one port NACK/SCL timeout; the other completes independently"),
    ("TC-S2-USB-FAILURE", "FS/HS packet boundaries, disconnect/late reply, no write replay"),
    ("TC-S2-WINDOWS", "clean Windows 10/11 x64 WinUSB enumeration, PING and I2C smoke"),
    ("TC-S3-ROLES", "M/M, M/S, S/M, S/S external peers; busy config rejection and unchanged other port"),
    ("TC-S3-TABLE-CAS", "PC snapshot/atomic fragment CAS against external writes; offset wrap and prefetch-aware oracle"),
    ("TC-S3-TABLE-SERVICE", "two ports at 1 MHz; eligible PC requests <= 2000 us; measured IRQ lock WCET"),
    ("TC-S4-STRETCH", "10 ms allowed stretch, per-low and total deadlines; raw SCL timing and partial execution"),
    ("TC-S4-SCAN", "known address set; address-only START/STOP, NACK, fault, cancellation and partial results"),
    ("TC-S4-RECOVERY", "SCL/SDA stuck, release after 9 clocks, STOP, mux restoration and no active-high drive"),
    ("TC-S5-FRONTEND", "approved circuit/PCB/BOM; 1.8/2.5/3.3/5 V, pullups, ADC validity, protection/ESD/power"),
    ("TC-S6-CONTROL-USB", "config/table/recovery disconnect and late replies; no replay; refreshed state after reconnect"),
    ("TC-S6-GUI-DEVICE", "USB GUI role/table/scan/recovery, cancel and physical disconnect on macOS and Windows"),
]


def assert_subset(actual: object, expected: object, path: str = "$", /) -> None:
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            raise AssertionError(f"{path}: expected object")
        for key, value in expected.items():
            if key not in actual:
                raise AssertionError(f"{path}.{key}: missing")
            assert_subset(actual[key], value, f"{path}.{key}")
    elif isinstance(expected, list):
        if not isinstance(actual, list) or len(actual) != len(expected):
            raise AssertionError(f"{path}: wrong list length/type")
        for index, (a, e) in enumerate(zip(actual, expected)):
            assert_subset(a, e, f"{path}[{index}]")
    elif type(actual) is not type(expected) or actual != expected:
        raise AssertionError(f"{path}: expected {expected!r}, got {actual!r}")


def run_case(adapter: CliAdapter, case: dict, evidence: str) -> dict:
    invocation = adapter.invoke(case["args"])
    result = {"id": case["id"], "evidence": evidence, "status": "FAIL", "invocation": invocation.as_dict()}
    try:
        if invocation.error:
            raise AssertionError(invocation.error)
        if invocation.returncode != case["exit_code"]:
            raise AssertionError(f"expected exit {case['exit_code']}, got {invocation.returncode}")
        assert_subset(invocation.response, case["expected"])
        result["status"] = "PASS"
    except AssertionError as exc:
        result["reason"] = str(exc)
    return result


def run_usb_smoke(adapter: CliAdapter) -> tuple[list[dict], str]:
    # No target I2C transaction is issued by this scaffold in USB mode.
    listed = run_case(adapter, {"id": "TC-USB-ENUM", "args": ["list"],
                               "expected": {"ok": True, "data": {}}, "exit_code": 0}, "usb_protocol")
    results = [listed]
    if listed["status"] != "PASS":
        return results, "FAIL"
    devices = listed["invocation"]["response"]["data"].get("devices")
    if not isinstance(devices, list) or any(not isinstance(d, dict) for d in devices):
        listed.update(status="FAIL", reason="invalid device list")
        return results, "FAIL"
    if len(devices) != 1:
        reason = "no USB bridge connected" if not devices else "USB smoke requires exactly one bridge"
        results.append({"id": "TC-USB-CONNECTION", "evidence": "usb_protocol", "status": "BLOCKED", "reason": reason})
        return results, "BLOCKED"
    info = run_case(adapter, {"id": "TC-USB-INFO", "args": ["info"],
                             "expected": {"ok": True, "command": "info", "data": {
                                 "capabilities": {}, "source": "device_reply"}}, "exit_code": 0}, "usb_protocol")
    results.append(info)
    if info["status"] != "PASS":
        return results, "FAIL"
    data = info["invocation"]["response"]["data"]
    count = data["capabilities"].get("port_count")
    control = data.get("control_capabilities")
    # The CLI produces one entry per negotiated port, including explicit
    # unsupported entries for legacy firmware. Do not infer a missing entry.
    if (type(count) is not int or not 1 <= count <= 2 or not isinstance(control, list)
            or len(control) != count or any(
                not isinstance(cap, dict) or type(cap.get("port")) is not int
                or cap["port"] != port
                or any(type(cap.get(flag)) is not bool for flag in (
                    "supports_role_config", "supports_frontend_get"))
                for port, cap in enumerate(control))):
        info.update(status="FAIL", reason="invalid per-port control capability matrix")
        return results, "FAIL"
    for cap in control:
        port = cap["port"]
        for command, supported in (("port-state", cap["supports_role_config"]),
                                   ("frontend-status", cap["supports_frontend_get"])):
            expected = {"command": command, "ok": supported}
            if supported:
                expected["data"] = {"port": port, "source": "device_reply"}
                if command == "port-state":
                    expected["data"].update(measured_speed_hz=None, speed_evidence="register_calculation")
            else:
                expected["error"] = {
                    "code": "operation_failed", "status": "unsupported", "status_code": 6,
                    "hpm_stat": 0, "session_invalidated": False, "result_unknown": False,
                }
            results.append(run_case(adapter, {
                "id": f"TC-USB-{command.upper()}-CH{port}", "args": ["--port", str(port), command],
                "expected": expected, "exit_code": 0 if supported else 1,
            }, "usb_protocol" if supported else "host_capability_gate"))
            if results[-1]["status"] != "PASS":
                # The operation may have invalidated its session. Stop this
                # smoke run rather than opening more sessions after failure.
                return results, "FAIL"
    return results, "USB_SMOKE_PASS" if all(r["status"] == "PASS" for r in results) else "FAIL"


def revision() -> str | None:
    try:
        return subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                                       text=True, stderr=subprocess.DEVNULL, timeout=5).strip()
    except (OSError, subprocess.SubprocessError):
        return None


def positive_seconds(value: str) -> float:
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("timeout must be a finite positive number")
    return number


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("offline", "usb"), default="offline")
    parser.add_argument("--cli", type=Path, default=ROOT / "host/target/debug/usb-i2c")
    parser.add_argument("--report", type=Path, required=True, help="machine-readable JSON report path")
    parser.add_argument("--timeout", type=positive_seconds, default=10.0, help="per-process deadline in seconds; no retry")
    args = parser.parse_args(argv)
    started = time.monotonic()
    started_at = datetime.now(timezone.utc).isoformat()
    binary = args.cli.resolve()
    adapter = CliAdapter(binary, args.mode, args.timeout)
    if not binary.is_file():
        results = [{"id": "TC-CLI-AVAILABLE", "evidence": "environment", "status": "BLOCKED",
                    "reason": f"build the CLI first: {binary}"}]
        outcome = "BLOCKED"
    elif args.mode == "offline":
        results = [run_case(adapter, case, "offline_model") for case in build_cases()]
        outcome = "MODEL_PASS" if all(r["status"] == "PASS" for r in results) else "FAIL"
    else:
        results, outcome = run_usb_smoke(adapter)
    report = {
        "schema": "usb-i2c.hil", "schema_version": 1,
        "started_at": started_at,
        "mode": args.mode, "status": outcome, "hardware_status": "NOT_RUN",
        "host": platform.platform(), "revision": revision(), "cli": str(binary),
        "cli_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary.is_file() else None,
        "elapsed_seconds": time.monotonic() - started, "retry_count": 0,
        "counts": {
            "model_pass": sum(r["status"] == "PASS" and r["evidence"] == "offline_model" for r in results),
            "usb_protocol_pass": sum(r["status"] == "PASS" and r["evidence"] == "usb_protocol" for r in results),
            "capability_gate_pass": sum(r["status"] == "PASS" and r["evidence"] == "host_capability_gate" for r in results),
            "hardware_pass": 0,
            "fail": sum(r["status"] == "FAIL" for r in results),
            "blocked": sum(r["status"] == "BLOCKED" for r in results),
        },
        "results": results,
        "pending_hardware": [{"id": case_id, "status": "NOT_RUN", "oracle": oracle,
                              "reason": "requires an approved physical setup and raw measurement evidence"}
                             for case_id, oracle in HARDWARE_CASES],
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{outcome}: model_pass={report['counts']['model_pass']} "
          f"usb_protocol_pass={report['counts']['usb_protocol_pass']} hardware_pass=0; report={args.report}")
    print("Physical timing, concurrency, FIFO/ACK behavior and Windows runtime remain unverified.")
    return 0 if outcome in ("MODEL_PASS", "USB_SMOKE_PASS") else (2 if outcome == "BLOCKED" else 1)


if __name__ == "__main__":
    sys.exit(main())
