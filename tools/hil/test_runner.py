import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import Mock, patch

from adapters.cli import CliAdapter, Invocation, validate_envelope
from run import assert_subset, run_case, run_usb_smoke


def envelope(backend="offline", data=None, command="list"):
    return {"schema": "usb-i2c.cli", "schema_version": 1, "backend": backend,
            "command": command, "ok": True, "data": data or {}, "error": None}


def failure(command, status="unsupported", code=6, backend="usb"):
    value = envelope(backend, command=command)
    value.update(ok=False, data=None, error={
        "code": "operation_failed", "message": status, "status": status,
        "status_code": code, "hpm_stat": 0,
        "session_invalidated": False, "result_unknown": False,
    })
    return value


def usb_info(role=True, frontend=False):
    return envelope("usb", {
        "source": "device_reply", "capabilities": {"port_count": 2},
        "control_capabilities": [
            {"port": port, "supports_role_config": role, "supports_frontend_get": frontend}
            for port in range(2)],
    }, "info")


def invocation(value, returncode=0):
    return Invocation([], returncode, json.dumps(value), "", 0.0, value, None)


class AdapterTests(unittest.TestCase):
    def test_mode_mismatch_cannot_be_counted_as_hardware(self):
        with self.assertRaisesRegex(ValueError, "backend"):
            validate_envelope(envelope("offline"), "usb", 0)

    def test_process_failure_cannot_be_hidden_by_success_json(self):
        with self.assertRaisesRegex(ValueError, "contradicts"):
            validate_envelope(envelope(), "offline", 1)

    def test_timeout_is_one_attempt_with_unknown_result(self):
        adapter = CliAdapter(Path("/unused/usb-i2c"), "usb", 0.1)
        with patch("adapters.cli.subprocess.run", side_effect=subprocess.TimeoutExpired(["cli"], 0.1)) as run:
            result = adapter.invoke(["write", "0x50", "00", "aa"])
        self.assertEqual(run.call_count, 1)
        self.assertIn("result unknown", result.error)
        self.assertIsNone(result.response)
        self.assertNotIn("--offline", result.argv)

    def test_offline_flag_is_always_explicit(self):
        adapter = CliAdapter(Path("/unused/usb-i2c"), "offline", 1.0)
        completed = subprocess.CompletedProcess([], 0, json.dumps(envelope()), "")
        with patch("adapters.cli.subprocess.run", return_value=completed) as run:
            result = adapter.invoke(["list"])
        self.assertIsNone(result.error)
        self.assertIn("--offline", run.call_args.args[0])

    def test_control_status_and_hpm_diagnostics_are_consistent(self):
        valid = failure("table-write", "conflict", 7)
        self.assertEqual(validate_envelope(valid, "usb", 1)["error"]["status_code"], 7)
        for field, value in (("status_code", 6), ("status_code", True),
                             ("status", "unknown"), ("hpm_stat", -1), ("hpm_stat", True)):
            invalid = failure("table-write", "conflict", 7)
            invalid["error"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                validate_envelope(invalid, "usb", 1)
        del valid["error"]["hpm_stat"]
        with self.assertRaisesRegex(ValueError, "incomplete control"):
            validate_envelope(valid, "usb", 1)

    def test_partial_scan_cannot_claim_success(self):
        valid = envelope(data={"addresses": [0x50], "complete": True,
                               "cancelled": False, "error": None}, command="scan")
        self.assertEqual(validate_envelope(valid, "offline", 0)["data"]["addresses"], [0x50])
        for key, value in (("complete", False), ("cancelled", True), ("error", {}),
                           ("addresses", [0x50, 0x50]), ("addresses", [0x51, 0x50]),
                           ("addresses", [0x07]), ("addresses", [True])):
            invalid = envelope(data={**valid["data"], key: value}, command="scan")
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                validate_envelope(invalid, "offline", 0)

    def test_failed_recovery_cannot_claim_success(self):
        for data in ({"succeeded": False, "reason": 0}, {"succeeded": True, "reason": 1},
                     {"succeeded": True, "reason": False}):
            with self.subTest(data=data), self.assertRaisesRegex(ValueError, "recovery"):
                validate_envelope(envelope(data=data, command="recover"), "offline", 0)

    def test_absent_voltage_cannot_be_presented_as_measurement(self):
        for command in ("frontend-status", "frontend-config"):
            for valid, voltage in ((False, None), (True, 3300)):
                value = envelope(data={"voltage_valid": valid, "measured_vref_mv": voltage}, command=command)
                validate_envelope(value, "offline", 0)
            for valid, voltage in ((False, 0), (True, None), (True, -1), (True, False)):
                with self.subTest(command=command, valid=valid, voltage=voltage), self.assertRaises(ValueError):
                    validate_envelope(envelope(data={"voltage_valid": valid,
                        "measured_vref_mv": voltage}, command=command), "offline", 0)


class RunnerTests(unittest.TestCase):
    def test_missing_hardware_is_blocked_without_connect(self):
        adapter = Mock()
        adapter.invoke.return_value = invocation(envelope("usb", {"devices": []}))
        results, status = run_usb_smoke(adapter)
        self.assertEqual(status, "BLOCKED")
        self.assertEqual(results[-1]["status"], "BLOCKED")
        self.assertEqual(adapter.invoke.call_count, 1)

    def test_multiple_bridges_are_not_selected_implicitly(self):
        adapter = Mock()
        adapter.invoke.return_value = invocation(envelope("usb", {"devices": [{"serial": "A"}, {"serial": "B"}]}))
        _, status = run_usb_smoke(adapter)
        self.assertEqual(status, "BLOCKED")
        self.assertEqual(adapter.invoke.call_count, 1)

    def test_corrupt_data_fails_instead_of_crashing(self):
        adapter = Mock()
        adapter.invoke.return_value = invocation(envelope("usb", ["not", "a", "device-list-object"]))
        results, status = run_usb_smoke(adapter)
        self.assertEqual(status, "FAIL")
        self.assertEqual(results[0]["status"], "FAIL")

    def test_usb_queries_only_advertised_read_only_state(self):
        adapter = Mock()
        responses = [invocation(envelope("usb", {"devices": [{"serial": "A"}]})), invocation(usb_info())]
        for port in range(2):
            responses.extend([
                invocation(envelope("usb", {"port": port, "source": "device_reply",
                    "measured_speed_hz": None, "speed_evidence": "register_calculation"}, "port-state")),
                invocation(failure("frontend-status"), 1),
            ])
        adapter.invoke.side_effect = responses
        results, status = run_usb_smoke(adapter)
        self.assertEqual(status, "USB_SMOKE_PASS")
        self.assertEqual([call.args[0] for call in adapter.invoke.call_args_list], [
            ["list"], ["info"], ["--port", "0", "port-state"], ["--port", "0", "frontend-status"],
            ["--port", "1", "port-state"], ["--port", "1", "frontend-status"],
        ])
        self.assertEqual(sum(r["evidence"] == "usb_protocol" for r in results), 4)
        self.assertEqual(sum(r["evidence"] == "host_capability_gate" for r in results), 2)

    def test_legacy_unsupported_is_a_host_gate_not_hardware_coverage(self):
        adapter = Mock()
        adapter.invoke.side_effect = [
            invocation(envelope("usb", {"devices": [{}]})), invocation(usb_info(role=False)),
            *[invocation(failure(command), 1)
              for _ in range(2) for command in ("port-state", "frontend-status")],
        ]
        results, status = run_usb_smoke(adapter)
        self.assertEqual(status, "USB_SMOKE_PASS")
        self.assertTrue(all(r["evidence"] == "host_capability_gate" for r in results[2:]))

    def test_advertised_frontend_get_requires_device_reply(self):
        adapter = Mock()
        adapter.invoke.side_effect = [
            invocation(envelope("usb", {"devices": [{}]})), invocation(usb_info(frontend=True)),
            invocation(envelope("usb", {"port": 0, "source": "device_reply",
                "measured_speed_hz": None, "speed_evidence": "register_calculation"}, "port-state")),
            invocation(failure("frontend-status"), 1),
        ]
        results, status = run_usb_smoke(adapter)
        self.assertEqual(status, "FAIL")
        self.assertEqual(results[-1]["id"], "TC-USB-FRONTEND-STATUS-CH0")
        self.assertEqual(adapter.invoke.call_count, 4)

    def test_invalid_capability_matrix_stops_before_control_query(self):
        for change in (
            lambda d: d.pop("control_capabilities"),
            lambda d: d["capabilities"].update(port_count=True),
            lambda d: d["control_capabilities"].pop(),
            lambda d: d["control_capabilities"][1].update(port=0),
            lambda d: d["control_capabilities"][0].update(supports_role_config=1),
        ):
            info = usb_info()
            change(info["data"])
            adapter = Mock()
            adapter.invoke.side_effect = [invocation(envelope("usb", {"devices": [{}]})), invocation(info)]
            results, status = run_usb_smoke(adapter)
            self.assertEqual(status, "FAIL")
            self.assertEqual(results[-1]["reason"], "invalid per-port control capability matrix")
            self.assertEqual(adapter.invoke.call_count, 2)

    def test_wrong_command_and_fake_measured_speed_are_rejected(self):
        for data, command in (({"measured_speed_hz": 100000, "speed_evidence": "measured"}, "port-state"),
                              ({"measured_speed_hz": None, "speed_evidence": "register_calculation"}, "recover")):
            adapter = Mock()
            adapter.invoke.side_effect = [
                invocation(envelope("usb", {"devices": [{}]})), invocation(usb_info()),
                invocation(envelope("usb", {"port": 0, "source": "device_reply", **data}, command)),
            ]
            _, status = run_usb_smoke(adapter)
            self.assertEqual(status, "FAIL")
            self.assertEqual(adapter.invoke.call_count, 3)

    def test_missing_readback_byte_fails_oracle(self):
        adapter = Mock()
        adapter.invoke.return_value = invocation(envelope(data={"data": [0, 1, 2]}))
        case = {"id": "readback", "args": ["wr"], "exit_code": 0,
                "expected": {"ok": True, "data": {"data": [0, 1, 2, 3]}}}
        self.assertEqual(run_case(adapter, case, "offline_model")["status"], "FAIL")

    def test_bool_does_not_match_integer_schema(self):
        with self.assertRaises(AssertionError):
            assert_subset({"port": True}, {"port": 1})


if __name__ == "__main__":
    unittest.main()
