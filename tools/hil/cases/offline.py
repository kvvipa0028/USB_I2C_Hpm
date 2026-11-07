"""Contract oracles for a fresh zero-filled 0x50 EEPROM on each model port.

These cases never claim clock accuracy, physical concurrency, or target coverage.
Every subprocess starts a new model, so stateful checks stay in one transaction.
"""


def _case(case_id: str, args: list[str], expected: dict, exit_code: int = 0) -> dict:
    return {"id": case_id, "args": args, "expected": expected, "exit_code": exit_code}


def build_cases() -> list[dict]:
    cases = [
        _case("TC-CLI-01", ["list"], {"ok": True, "data": {"devices": [{"serial": "offline-model", "speed": None}]}}),
        _case("TC-CAP-01", ["info"], {"ok": True, "data": {"capabilities": {
            "port_count": 2, "max_transfer": 256, "fixed_speed_hz": 100_000,
            "min_speed_hz": 10_000, "max_speed_hz": 1_000_000, "supports_custom_speed": True,
            "supports_port_transfer": True, "supports_batch": True, "batch_max_transfer": 240,
        }}}),
        _case("TC-CLI-02", ["ping"], {"ok": True}),
    ]
    for port in (0, 1):
        cases.append(_case(f"TC-MODEL-READ-CH{port}", ["--port", str(port), "read", "0x50", "16"],
                           {"ok": True, "data": {"port": port, "data": [0] * 16}}))
        # Write 255 distinct bytes at offset 0. Current offset is then 255:
        # read sees untouched table[255], then wraps to the first three writes.
        data = ["00"] + [f"{value:02x}" for value in range(1, 256)]
        cases.append(_case(f"TC-MODEL-WRAP-CH{port}", ["--port", str(port), "wr", "0x50", "4", *data],
                           {"ok": True, "data": {"written": 256, "data": [0, 1, 2, 3]}}))
    for hz in (10_000, 50_000, 100_000, 400_000, 800_000, 1_000_000):
        cases.append(_case(f"TC-MODEL-SPEED-{hz}", ["--speed-hz", str(hz), "read", "0x50", "1"],
                           {"ok": True, "data": {"speed_hz": hz, "data": [0]}}))
    for case_id, args in (
        ("TC-MODEL-PORT-REJECT", ["--port", "2", "read", "0x50", "1"]),
        ("TC-MODEL-SPEED-LOW", ["--speed-hz", "9999", "read", "0x50", "1"]),
        ("TC-MODEL-SPEED-HIGH", ["--speed-hz", "1000001", "read", "0x50", "1"]),
        ("TC-MODEL-LENGTH-REJECT", ["read", "0x50", "257"]),
        ("TC-MODEL-EMPTY-REJECT", ["read", "0x50", "0"]),
        ("TC-MODEL-NACK", ["read", "0x51", "1"]),
    ):
        cases.append(_case(case_id, args, {"ok": False, "error": {
            "code": "operation_failed", "session_invalidated": False, "result_unknown": False,
        }}, 1))
    # 17 writes advance each pointer to 17; 240 reads wrap and end at table[0].
    # Different values on the two ports detect accidental shared table storage.
    cases.append(_case("TC-MODEL-BATCH-ISOLATION", [
        "batch", "--ch0-write", "00 " + "aa " * 17, "--ch0-read", "240",
        "--ch1-write", "00 " + "55 " * 17, "--ch1-read", "240",
    ], {"ok": True, "data": {"outcomes": [
        {"port": 0, "status": "ok", "hpm_stat": 0, "data": [0] * 239 + [0xAA]},
        {"port": 1, "status": "ok", "hpm_stat": 0, "data": [0] * 239 + [0x55]},
    ]}}))
    cases.append(_case("TC-MODEL-BATCH-NACK", [
        "batch", "--ch0-addr", "0x51", "--ch0-read", "1", "--ch1-read", "1",
    ], {"ok": False, "error": {"code": "batch_failure", "result_unknown": False},
        "data": {"outcomes": [
            {"port": 0, "status": "nack", "data": []},
            {"port": 1, "status": "ok", "data": [0]},
        ]}}, 1))
    cases.append(_case("TC-MODEL-BATCH-LIMIT", [
        "batch", "--ch0-read", "241", "--ch1-read", "1",
    ], {"ok": False, "error": {"code": "operation_failed", "result_unknown": False}}, 1))
    cases.append(_case("TC-CLI-ARGUMENT-ERROR", ["read", "not-an-address", "1"],
                       {"ok": False, "error": {"code": "invalid_argument", "result_unknown": False}}, 2))
    return cases
