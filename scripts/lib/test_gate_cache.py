"""Tests for scripts/gate-cache.sh.

The script asks `cargo metadata` where target/ is and which crates belong to
the workspace, and `rustc` / `cargo llvm-cov` for the toolchain identity. These
tests put fake `cargo` and `rustc` first on PATH, so they need neither
cargo-llvm-cov nor a real build, and they keep the cache in a temporary
directory.
"""

import os
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "gate-cache.sh"
REPO = Path(__file__).resolve().parents[2]

FAKE_CARGO = """#!/usr/bin/env bash
echo "$*" >>"$FAKE_CALLS"
case "$1" in
  metadata)
    printf '{"target_directory": "%s", "packages": [{"name": "ws-a"}, {"name": "ws-b"}]}\\n' "$FAKE_TARGET"
    ;;
  llvm-cov)
    case "$2" in
      --version) echo "cargo-llvm-cov 0.0.0" ;;
      clean) rm -rf "$FAKE_TARGET/llvm-cov-target/debug/deps/ws-a" ;;
    esac
    ;;
  clean)
    rm -rf "$FAKE_TARGET/debug/deps/ws-b"
    ;;
esac
"""

FAKE_RUSTC = """#!/usr/bin/env bash
echo "rustc 0.0.0"
echo "release: ${FAKE_RUSTC_RELEASE:-1.0.0}"
"""


def _write_executable(path: Path, text: str) -> None:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class GateCacheTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.target = self.tmp / "target"
        self.cache = self.tmp / "cache"
        self.calls = self.tmp / "calls.log"
        fakebin = self.tmp / "bin"
        fakebin.mkdir()
        _write_executable(fakebin / "cargo", FAKE_CARGO)
        _write_executable(fakebin / "rustc", FAKE_RUSTC)
        self.env = dict(
            os.environ,
            PATH=f"{fakebin}:{os.environ['PATH']}",
            FAKE_TARGET=str(self.target),
            FAKE_CALLS=str(self.calls),
            RAIKIRI_GATE_CACHE_DIR=str(self.cache),
        )
        self.env.pop("CARGO_TARGET_DIR", None)

    def _run(self, *args, **env):
        return subprocess.run(
            [str(SCRIPT), *args],
            capture_output=True,
            text=True,
            env=dict(self.env, **env),
            cwd=REPO,
            check=False,
        )

    def _populate_target(self):
        """Lay out a target/ the way a finished gate run leaves it."""
        for root in ("llvm-cov-target/debug", "debug"):
            deps = self.target / root / "deps"
            deps.mkdir(parents=True)
            (deps / "serde-1.rlib").write_text("third-party")
            (deps / "ws-a" if root != "debug" else deps / "ws-b").write_text("workspace")
            incremental = self.target / root / "incremental" / "ws-a-hash"
            incremental.mkdir(parents=True)
            (incremental / "state").write_text("incremental")

    def _entries(self, suffix):
        return sorted(p.name for p in self.cache.glob(f"gate-target-*{suffix}"))

    def _save_one_entry(self) -> str:
        self._populate_target()
        result = self._run("save")
        self.assertEqual(result.returncode, 0, result.stderr)
        (archive,) = self._entries(".tar.gz")
        return archive[: -len(".tar.gz")]

    def _copy_entry(self, name: str, new_name: str) -> None:
        for suffix in (".tar.gz", ".info"):
            shutil.copy(self.cache / f"{name}{suffix}", self.cache / f"{new_name}{suffix}")

    def test_usage_errors(self):
        for args in ([], ["bogus"], ["save", "extra"]):
            result = self._run(*args)
            self.assertEqual(result.returncode, 2, args)
            self.assertIn("usage", result.stderr)

    def test_save_then_restore_round_trips_third_party_artifacts_only(self):
        self._save_one_entry()
        # save is destructive: the workspace crates' artifacts are gone.
        self.assertTrue((self.target / "llvm-cov-target/debug/deps/serde-1.rlib").exists())
        self.assertFalse((self.target / "llvm-cov-target/debug/deps/ws-a").exists())
        self.assertFalse((self.target / "debug/deps/ws-b").exists())
        calls = self.calls.read_text().splitlines()
        self.assertIn("llvm-cov clean --workspace", calls)
        self.assertIn("clean --locked -p ws-a -p ws-b", calls)

        shutil.rmtree(self.target)
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.target / "llvm-cov-target/debug/deps/serde-1.rlib").exists())
        self.assertTrue((self.target / "debug/deps/serde-1.rlib").exists())
        self.assertFalse((self.target / "llvm-cov-target/debug/incremental").exists())
        self.assertFalse((self.target / "debug/incremental").exists())
        self.assertEqual(
            [p.name for p in self.target.iterdir() if p.name.startswith(".")], []
        )

    def test_archive_holds_no_workspace_artifacts(self):
        name = self._save_one_entry()
        with tarfile.open(self.cache / f"{name}.tar.gz") as archive:
            members = archive.getnames()
        self.assertIn("llvm-cov-target/debug/deps/serde-1.rlib", members)
        self.assertNotIn("llvm-cov-target/debug/deps/ws-a", members)
        self.assertNotIn("debug/deps/ws-b", members)
        self.assertFalse([m for m in members if "incremental" in m])

    def test_restore_without_a_cache_directory_does_nothing(self):
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("building from scratch", result.stdout)
        self.assertFalse(self.target.exists())

    def test_restore_leaves_an_existing_instrumented_build_alone(self):
        self._save_one_entry()
        (self.target / "llvm-cov-target/debug").mkdir(parents=True, exist_ok=True)
        marker = self.target / "llvm-cov-target/debug/marker"
        marker.write_text("mine")
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already exists", result.stdout)
        self.assertEqual(marker.read_text(), "mine")

    def test_restore_keeps_an_existing_debug_directory(self):
        self._save_one_entry()
        shutil.rmtree(self.target / "llvm-cov-target")
        marker = self.target / "debug/marker"
        marker.write_text("mine")
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("kept the existing", result.stdout)
        self.assertEqual(marker.read_text(), "mine")
        self.assertTrue((self.target / "llvm-cov-target/debug/deps/serde-1.rlib").exists())

    def test_restore_prefers_the_entry_for_the_same_lock_file(self):
        exact = self._save_one_entry()
        toolchain = re.search(r"-tc([0-9a-f]+)-", exact).group(1)
        self._copy_entry(exact, f"gate-target-20990101T000000Z-tc{toolchain}-lock000000000000")
        shutil.rmtree(self.target)
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"restoring {exact}", result.stdout)
        self.assertNotIn("different Cargo.lock", result.stdout)

    def test_restore_falls_back_to_the_newest_entry_for_the_same_toolchain(self):
        exact = self._save_one_entry()
        toolchain = re.search(r"-tc([0-9a-f]+)-", exact).group(1)
        older = f"gate-target-20000101T000000Z-tc{toolchain}-lock111111111111"
        newer = f"gate-target-20990101T000000Z-tc{toolchain}-lock222222222222"
        self._copy_entry(exact, older)
        self._copy_entry(exact, newer)
        for suffix in (".tar.gz", ".info"):
            (self.cache / f"{exact}{suffix}").unlink()
        shutil.rmtree(self.target)
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"restoring {newer}", result.stdout)
        self.assertIn("different Cargo.lock", result.stdout)

    def test_restore_ignores_entries_from_another_toolchain(self):
        exact = self._save_one_entry()
        other = "gate-target-20990101T000000Z-tcffffffffffff-lock000000000000"
        self._copy_entry(exact, other)
        for suffix in (".tar.gz", ".info"):
            (self.cache / f"{exact}{suffix}").unlink()
        shutil.rmtree(self.target)
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no entry for this toolchain", result.stdout)
        self.assertFalse(self.target.exists())

    def test_a_different_rustc_does_not_match(self):
        self._save_one_entry()
        shutil.rmtree(self.target)
        result = self._run("restore", FAKE_RUSTC_RELEASE="2.0.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no entry for this toolchain", result.stdout)

    def test_incomplete_entries_are_ignored(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        archive = self.cache / f"{name}.tar.gz"
        archive.write_bytes(archive.read_bytes()[:-8])  # size no longer matches .info
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no entry for this toolchain", result.stdout)
        self.assertFalse(self.target.exists())

    def test_archive_without_an_info_file_is_not_an_entry(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        (self.cache / f"{name}.info").unlink()
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no entry for this toolchain", result.stdout)

    def test_a_corrupt_archive_fails_without_leaving_partial_directories(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        garbage = b"not a gzip archive"
        (self.cache / f"{name}.tar.gz").write_bytes(garbage)
        (self.cache / f"{name}.info").write_text(f"bytes={len(garbage)}\n")
        result = self._run("restore")
        self.assertEqual(result.returncode, 1)
        self.assertIn("failed", result.stderr)
        self.assertEqual(list(self.target.iterdir()), [])

    def test_save_requires_an_instrumented_build(self):
        result = self._run("save")
        self.assertEqual(result.returncode, 1)
        self.assertIn("run scripts/gate.sh first", result.stderr)
        self.assertFalse(self.cache.exists())

    def test_save_keeps_only_the_newest_entries(self):
        self.cache.mkdir()
        for day in range(1, 5):
            stem = f"gate-target-2000010{day}T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb"
            (self.cache / f"{stem}.tar.gz").write_bytes(b"x")
            (self.cache / f"{stem}.info").write_text("bytes=1\n")
        self._populate_target()
        result = self._run("save")
        self.assertEqual(result.returncode, 0, result.stderr)
        remaining = self._entries(".info")
        self.assertEqual(len(remaining), 3)
        self.assertNotIn("gate-target-20000101T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb.info", remaining)
        self.assertNotIn("gate-target-20000102T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb.info", remaining)
        self.assertEqual(len(self._entries(".tar.gz")), 3)

    def test_save_leaves_no_temporary_files_behind(self):
        self._save_one_entry()
        self.assertEqual([p.name for p in self.cache.iterdir() if p.name.startswith(".")], [])

    def test_status_marks_the_entry_restore_would_use(self):
        name = self._save_one_entry()
        result = self._run("status")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"{name}", result.stdout)
        self.assertIn("restore would use this", result.stdout)


if __name__ == "__main__":
    unittest.main()
