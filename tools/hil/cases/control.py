"""S3-S6 CLI oracles; each invocation gets a fresh master-role model.

Cross-request table/CAS persistence is tested by the Rust CLI dispatch tests,
not by pretending these independent processes share their model state.
"""

from __future__ import annotations


def _case(case_id: str, args: list[str], expected: dict, exit_code: int = 0) -> dict:
    return {"id": case_id, "args": args, "expected": expected, "exit_code": exit_code}


def _error(status: str | None, status_code: int | None, *, invalid=False) -> dict:
    return {"ok": False, "error": {
        "code": "invalid_argument" if invalid else "operation_failed",
        "status": status, "status_code": status_code,
        "hpm_stat": None if status is None else 0,
        "session_invalidated": False, "result_unknown": False,
    }}


def build_control_cases() -> list[dict]:
    control_caps = [{
        "version": 1, "port": port, "supports_role_config": True,
        "supports_table": True, "supports_table_cas": True, "supports_timeouts": True,
        "supports_probe": True, "supports_recover": True,
        "supports_frontend_get": False, "supports_frontend_set": False,
        "table_size": 256, "max_table_len": 256, "table_service_us": 2000,
        "stretch_min_us": 1000, "stretch_max_us": 100000, "stretch_default_us": 25000,
        "transaction_min_us": 1000, "transaction_max_us": 2000000,
        "transaction_default_us": 500000, "control_deadline_us": 1500000,
        "frontend_options": 0, "timing_evidence": "advertised_contract_not_measurement",
    } for port in (0, 1)]
    cases = [_case("TC-CONTROL-CAPS", ["info"], {"ok": True, "data": {
        "control_capabilities": control_caps, "source": "offline_model",
    }})]
    for port in (0, 1):
        prefix = ["--port", str(port)]
        cases.append(_case(f"TC-CONTROL-STATE-CH{port}", [*prefix, "port-state"], {
            "ok": True, "data": {
                "port": port, "activity": "idle", "config": {
                    "role": "master", "slave_address": 0x50, "speed_hz": 100000,
                    "stretch_timeout_us": 25000, "transaction_timeout_us": 500000,
                }, "config_generation": 0, "table_generation": 0,
                "calculated_speed_hz": 100000, "measured_speed_hz": None,
                "speed_evidence": "offline_model", "source": "offline_model",
            },
        }))
        cases.append(_case(f"TC-CONTROL-CONFIG-CH{port}", [
            *prefix, "--speed-hz", "400000", "port-config", "--role", "slave",
            "--slave-address", "0x51", "--stretch-timeout-us", "30000",
            "--transaction-timeout-us", "600000", "--expected-gen", "0",
        ], {"ok": True, "data": {
            "port": port, "activity": "idle", "config": {
                "role": "slave", "slave_address": 0x51, "speed_hz": 400000,
                "stretch_timeout_us": 30000, "transaction_timeout_us": 600000,
            }, "config_generation": 1, "table_generation": 0,
            "calculated_speed_hz": 400000, "measured_speed_hz": None,
            "speed_evidence": "offline_model", "source": "offline_model",
        }}))
        cases.append(_case(f"TC-CONTROL-SCAN-CH{port}", [*prefix, "scan"], {
            "ok": True, "data": {"port": port, "addresses": [0x50],
                "address_range": {"first": 0x08, "last": 0x77},
                "complete": True, "cancelled": False, "error": None,
                "probe_only": True, "source": "offline_model"},
        }))
        cases.append(_case(f"TC-CONTROL-RECOVER-CH{port}", [*prefix, "recover"], {
            "ok": True, "data": {"port": port, "command_completed": True,
                "succeeded": True, "reason": 0, "reason_name": "success", "clocks_sent": 9,
                "initial_lines": 3, "final_lines": 3, "line_state_source": "offline_model",
                "timing_verified": False, "source": "offline_model"},
        }))
        cases.append(_case(f"TC-CONTROL-TABLE-ROLE-CH{port}",
                           [*prefix, "table-read", "0", "1"], _error("wrong_role", 8), 1))
    cases.extend([
        _case("TC-CONTROL-CONFIG-CAS-REJECT", ["port-config", "--role", "slave", "--expected-gen", "1"], _error("conflict", 7), 1),
        _case("TC-CONTROL-TABLE-WRITE-ROLE", ["table-write", "0xff", "aa", "--expected-gen", "0"], _error("wrong_role", 8), 1),
        _case("TC-CONTROL-FRONTEND-GET", ["frontend-status"], _error("unsupported", 6), 1),
        _case("TC-CONTROL-FRONTEND-SET", ["frontend-config", "--source", "internal3v3", "--pullup", "4k7", "--expected-gen", "0"], _error("unsupported", 6), 1),
    ])
    for case_id, args in (
        ("SLAVE-SPEED", ["port-config", "--role", "slave", "--speed-hz", "200000"]),
        ("ADDRESS", ["port-config", "--role", "master", "--slave-address", "0x07"]),
        ("STRETCH", ["port-config", "--role", "master", "--stretch-timeout-us", "999"]),
        ("DEADLINE", ["port-config", "--role", "master", "--transaction-timeout-us", "1000"]),
        ("TABLE-EMPTY", ["table-read", "0", "0"]),
        ("TABLE-LENGTH", ["table-read", "0", "257"]),
        ("TABLE-RANGE", ["table-read", "255", "2"]),
        ("TABLE-OFFSET", ["table-read", "65535", "1"]),
        ("TABLE-WRITE-RANGE", ["table-write", "255", "aa", "bb"]),
        ("TABLE-HEX", ["table-write", "0", "not-hex"]),
        ("ROLE", ["port-config", "--role", "unknown"]),
        ("FRONTEND-SOURCE", ["frontend-config", "--source", "unknown", "--pullup", "off", "--expected-gen", "0"]),
        ("FRONTEND-GENERATION", ["frontend-config", "--source", "external", "--pullup", "off"]),
    ):
        cases.append(_case(f"TC-CONTROL-INVALID-{case_id}", args, _error(None, None, invalid=True), 2))
    return cases
