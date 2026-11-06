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
    results.append(run_case(adapter, {"id": "TC-USB-INFO", "args": ["info"],
                                     "expected": {"ok": True, "data": {"capabilities": {}}}, "exit_code": 0}, "usb_protocol"))
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
