"""One CLI process per operation; no retry and no USB-to-model fallback."""

from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import subprocess
import time


@dataclass
class Invocation:
    argv: list[str]
    returncode: int | None
    stdout: str
    stderr: str
    elapsed_seconds: float
    response: dict | None
    error: str | None

    def as_dict(self) -> dict:
        return vars(self).copy()


def _text(value: str | bytes | None) -> str:
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value or ""


def validate_envelope(value: object, backend: str, returncode: int) -> dict:
    if not isinstance(value, dict):
        raise ValueError("CLI response is not an object")
    if (value.get("schema") != "usb-i2c.cli" or type(value.get("schema_version")) is not int
            or value["schema_version"] != 1):
        raise ValueError("unsupported CLI JSON schema")
    if value.get("backend") != backend:
        raise ValueError("CLI backend does not match the requested test mode")
    if not isinstance(value.get("command"), str) or type(value.get("ok")) is not bool:
        raise ValueError("missing CLI command or boolean outcome")
    if "data" not in value or "error" not in value:
        raise ValueError("incomplete CLI result envelope")
    if value["ok"]:
        if returncode != 0 or value["error"] is not None:
            raise ValueError("successful JSON contradicts process exit/error")
    else:
        error = value["error"]
        if returncode == 0 or not isinstance(error, dict):
            raise ValueError("failed JSON contradicts process exit/error")
        if not isinstance(error.get("code"), str) or not isinstance(error.get("message"), str):
            raise ValueError("missing structured CLI error")
        if any(type(error.get(key)) is not bool for key in ("session_invalidated", "result_unknown")):
            raise ValueError("missing failure/session semantics")
        diagnostics = ("status", "status_code", "hpm_stat")
        if any(key in error for key in diagnostics):
            if not all(key in error for key in diagnostics):
                raise ValueError("incomplete control diagnostics")
            statuses = ("ok", "nack", "timeout", "bad_request", "i2c_error", "busy",
                        "unsupported", "conflict", "wrong_role", "service_deadline")
            status, code, hpm = (error[key] for key in diagnostics)
            if status is None:
                if code is not None:
                    raise ValueError("control status name/code disagree")
            elif (status not in statuses or type(code) is not int
                  or code != statuses.index(status)):
                raise ValueError("control status name/code disagree")
            if hpm is not None and (type(hpm) is not int or not 0 <= hpm <= 0xFFFFFFFF):
                raise ValueError("invalid hpm_stat")
    if value["ok"] and value["command"] in ("scan", "recover", "frontend-status", "frontend-config"):
        data = value["data"]
        if not isinstance(data, dict):
            raise ValueError("control result must contain data")
        if value["command"] == "scan":
            if (data.get("complete") is not True or data.get("cancelled") is not False
                    or data.get("error") is not None):
                raise ValueError("partial scan cannot be successful")
            addresses = data.get("addresses")
            if (not isinstance(addresses, list)
                    or any(type(a) is not int or not 0x08 <= a <= 0x77 for a in addresses)
                    or addresses != sorted(set(addresses))):
                raise ValueError("invalid scan address set")
        elif value["command"] == "recover":
            if (data.get("succeeded") is not True or type(data.get("reason")) is not int
                    or data["reason"] != 0):
                raise ValueError("recovery diagnostic failure cannot be successful")
        else:
            valid, mv = data.get("voltage_valid"), data.get("measured_vref_mv")
            if (type(valid) is not bool or (not valid and mv is not None)
                    or (valid and (type(mv) is not int or not 0 <= mv <= 0xFFFFFFFF))):
                raise ValueError("invalid frontend measurement validity")
    return value


class CliAdapter:
    def __init__(self, binary: Path, mode: str, timeout: float):
        self.binary = binary
        self.mode = mode
        self.timeout = timeout

    def invoke(self, args: list[str]) -> Invocation:
        argv = [str(self.binary), "--json"]
        if self.mode == "offline":
            argv.append("--offline")
        argv.extend(args)
        start = time.monotonic()
        try:
            # Exactly one attempt. A timed-out write may already have executed.
            done = subprocess.run(argv, capture_output=True, text=True, timeout=self.timeout, check=False)
        except subprocess.TimeoutExpired as exc:
            return Invocation(argv, None, _text(exc.stdout), _text(exc.stderr),
                              time.monotonic() - start, None,
                              "CLI deadline exceeded; result unknown; not retried")
        except OSError as exc:
            return Invocation(argv, None, "", "", time.monotonic() - start, None, str(exc))
        try:
            response = validate_envelope(json.loads(done.stdout), self.mode, done.returncode)
            error = None
        except (ValueError, TypeError) as exc:
            response, error = None, str(exc)
        return Invocation(argv, done.returncode, done.stdout, done.stderr,
                          time.monotonic() - start, response, error)
