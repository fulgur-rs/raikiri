"""Tests for scripts/lib/instrumented-run.sh.

The script asks `cargo llvm-cov show-env` where the instrumented build lives.
These tests put a fake `cargo` first on PATH so they need neither
cargo-llvm-cov nor a real build.
"""

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "instrumented-run.sh"
REPO = Path(__file__).resolve().parents[2]


def _write_executable(path: Path, text: str) -> None:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class InstrumentedRunTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.target = self.tmp / "target"
        self.debug = self.target / "llvm-cov-target" / "debug"
        self.fakebin = self.tmp / "bin"
        self.fakebin.mkdir()

    def _fake_cargo(self, body: str) -> None:
        _write_executable(self.fakebin / "cargo", "#!/usr/bin/env bash\n" + body)

    def _fake_show_env(self) -> None:
        self._fake_cargo(
            'if [[ "$1 $2" == "llvm-cov show-env" ]]; then\n'
            f"  echo \"export CARGO_LLVM_COV_TARGET_DIR='{self.target}'\"\n"
            "  exit 0\n"
            "fi\n"
            "exit 99\n"
        )

    def _run(self, *args):
        env = dict(os.environ, PATH=f"{self.fakebin}:{os.environ['PATH']}")
        env.pop("RUSTDOCFLAGS", None)
        return subprocess.run(
            [str(SCRIPT), *args],
            capture_output=True,
            text=True,
            env=env,
            cwd=REPO,
            check=False,
        )

    def test_without_arguments_is_a_usage_error(self):
        result = self._run()
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage", result.stderr)

    def test_bin_without_a_name_is_a_usage_error(self):
        result = self._run("--bin")
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage", result.stderr)

    def test_show_env_failure_stops_before_running_anything(self):
        self._fake_cargo("exit 1\n")
        result = self._run("true")
        self.assertEqual(result.returncode, 2)
        self.assertIn("show-env failed", result.stderr)

    def test_missing_instrumented_directory_is_reported(self):
        self._fake_show_env()
        result = self._run("true")
        self.assertEqual(result.returncode, 2)
        self.assertIn("does not exist", result.stderr)

    def test_bin_runs_the_binary_the_instrumented_build_produced(self):
        self._fake_show_env()
        self.debug.mkdir(parents=True)
        _write_executable(
            self.debug / "tool",
            '#!/usr/bin/env bash\necho "args=$* profile=$LLVM_PROFILE_FILE"\n',
        )
        result = self._run("--bin", "tool", "a", "b")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "args=a b profile=/dev/null")

    def test_bin_missing_from_the_instrumented_build_is_reported(self):
        self._fake_show_env()
        self.debug.mkdir(parents=True)
        result = self._run("--bin", "tool")
        self.assertEqual(result.returncode, 2)
        self.assertIn("was not built", result.stderr)

    def test_command_runs_in_the_instrumented_target_directory(self):
        self._fake_show_env()
        self.debug.mkdir(parents=True)
        result = self._run(
            "bash", "-c", 'echo "$CARGO_TARGET_DIR|$RUSTDOCFLAGS|$LLVM_PROFILE_FILE"'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.strip(),
            f"{self.target}/llvm-cov-target|-C instrument-coverage --cfg=coverage|/dev/null",
        )

    def test_command_keeps_the_callers_rustdocflags(self):
        self._fake_show_env()
        self.debug.mkdir(parents=True)
        env = dict(
            os.environ,
            PATH=f"{self.fakebin}:{os.environ['PATH']}",
            RUSTDOCFLAGS="-D warnings",
        )
        result = subprocess.run(
            [str(SCRIPT), "bash", "-c", 'echo "$RUSTDOCFLAGS"'],
            capture_output=True,
            text=True,
            env=env,
            cwd=REPO,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.strip(), "-D warnings -C instrument-coverage --cfg=coverage"
        )

    def test_command_exit_status_is_passed_through(self):
        self._fake_show_env()
        self.debug.mkdir(parents=True)
        result = self._run("bash", "-c", "exit 7")
        self.assertEqual(result.returncode, 7)


if __name__ == "__main__":
    unittest.main()
