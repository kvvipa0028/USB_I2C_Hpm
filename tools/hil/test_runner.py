import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import Mock, patch

from adapters.cli import CliAdapter, Invocation, validate_envelope
from run import assert_subset, run_case, run_usb_smoke


def envelope(backend="offline", data=None):
    return {"schema": "usb-i2c.cli", "schema_version": 1, "backend": backend,
            "command": "list", "ok": True, "data": data or {}, "error": None}


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
