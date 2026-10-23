"""Tests for scripts/gate-cache.sh.

The script asks `cargo metadata` where target/ is and which crates belong to
the workspace, and `rustc` / `cargo llvm-cov` for the toolchain identity. These
tests put fake `cargo` and `rustc` first on PATH, so they need neither
cargo-llvm-cov nor a real build, and they keep the cache in a temporary
directory.
"""

import io
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

    def _entries(self, suffix=".info"):
        return sorted(p.name for p in self.cache.glob(f"gate-target-*{suffix}"))

    def _parts(self, name):
        return sorted(self.cache.glob(f"{name}.part.*"))

    def _part_owners(self):
        """Names of the entries that still have part files on disk."""
        return {p.name.split(".part.")[0] for p in self.cache.glob("gate-target-*.part.*")}

    def _write_entry(self, name, parts):
        """Write a synthetic entry: one file per element of `parts`, then the .info."""
        self.cache.mkdir(exist_ok=True)
        lines = [f"bytes={sum(len(p) for p in parts)}"]
        for index, data in enumerate(parts):
            part = f"{name}.part.a{chr(ord('a') + index)}"
            (self.cache / part).write_bytes(data)
            lines.append(f"part={part} {len(data)}")
        (self.cache / f"{name}.info").write_text("\n".join(lines) + "\n")

    def _remove_entry(self, name):
        for part in self._parts(name):
            part.unlink()
        (self.cache / f"{name}.info").unlink()

    def _save_one_entry(self, **env) -> str:
        self._populate_target()
        result = self._run("save", **env)
        self.assertEqual(result.returncode, 0, result.stderr)
        (info,) = self._entries(".info")
        return info[: -len(".info")]

    def _copy_entry(self, name: str, new_name: str) -> None:
        """Copy an entry under another name (parts renamed, .info rewritten)."""
        for part in self._parts(name):
            shutil.copy(part, self.cache / part.name.replace(name, new_name))
        info = (self.cache / f"{name}.info").read_text().replace(name, new_name)
        (self.cache / f"{new_name}.info").write_text(info)

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
        data = b"".join(part.read_bytes() for part in self._parts(name))
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
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
        self._remove_entry(exact)
        shutil.rmtree(self.target)
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"restoring {newer}", result.stdout)
        self.assertIn("different Cargo.lock", result.stdout)

    def test_restore_ignores_entries_from_another_toolchain(self):
        exact = self._save_one_entry()
        other = "gate-target-20990101T000000Z-tcffffffffffff-lock000000000000"
        self._copy_entry(exact, other)
        self._remove_entry(exact)
        shutil.rmtree(self.target)
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no usable entry for this toolchain", result.stdout)
        self.assertFalse(self.target.exists())

    def test_a_different_rustc_does_not_match(self):
        self._save_one_entry()
        shutil.rmtree(self.target)
        result = self._run("restore", FAKE_RUSTC_RELEASE="2.0.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no usable entry for this toolchain", result.stdout)

    def test_incomplete_entries_are_ignored(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        part = self._parts(name)[-1]
        part.write_bytes(part.read_bytes()[:-8])  # size no longer matches .info
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no usable entry for this toolchain", result.stdout)
        self.assertFalse(self.target.exists())

    def test_archive_without_an_info_file_is_not_an_entry(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        (self.cache / f"{name}.info").unlink()
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no usable entry for this toolchain", result.stdout)

    def test_a_corrupt_archive_fails_without_leaving_partial_directories(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        self._remove_entry(name)
        self._write_entry(name, [b"not a gzip archive"])
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
        for day in range(1, 5):
            stem = f"gate-target-2000010{day}T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb"
            self._write_entry(stem, [b"x", b"y"])
        self._populate_target()
        result = self._run("save")
        self.assertEqual(result.returncode, 0, result.stderr)
        remaining = self._entries(".info")
        self.assertEqual(len(remaining), 3)
        self.assertNotIn("gate-target-20000101T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb.info", remaining)
        self.assertNotIn("gate-target-20000102T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb.info", remaining)
        self.assertEqual(len(self._part_owners()), 3)  # no parts of pruned entries left

    def test_broken_entries_do_not_push_usable_ones_out(self):
        for day in (1, 2):
            stem = f"gate-target-2000010{day}T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb"
            self._write_entry(stem, [b"x"])
        for day in (1, 2, 3):  # newer than any real entry, but the parts are missing
            stem = f"gate-target-209001{day:02d}T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb"
            (self.cache / f"{stem}.info").write_text(f"bytes=1\npart={stem}.part.aa 1\n")
        self._populate_target()
        result = self._run("save")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self._part_owners()), 3)  # the new entry and both old ones
        self.assertEqual(len(self._entries(".info")), 6)  # fresh broken ones stay

    def test_broken_entries_are_dropped_after_a_day(self):
        self.cache.mkdir()
        stem = "gate-target-20000101T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb"
        info = self.cache / f"{stem}.info"
        info.write_text(f"bytes=1\npart={stem}.part.aa 1\n")
        two_days_ago = info.stat().st_mtime - 2 * 86400
        os.utime(info, (two_days_ago, two_days_ago))
        self._populate_target()
        result = self._run("save")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(f"{stem}.info", self._entries(".info"))

    def test_save_that_cannot_write_the_cache_leaves_the_checkout_alone(self):
        self._populate_target()
        self.cache.mkdir()
        self.cache.chmod(0o500)
        self.addCleanup(self.cache.chmod, 0o700)
        if os.access(self.cache, os.W_OK):  # running as root: permissions do not apply
            self.skipTest("cache directory is writable despite chmod")
        result = self._run("save")
        self.assertEqual(result.returncode, 1)
        self.assertIn("nothing was removed", result.stderr)
        self.assertTrue((self.target / "llvm-cov-target/debug/deps/ws-a").exists())
        self.assertNotIn("clean", self.calls.read_text())

    def test_save_leaves_no_temporary_files_behind(self):
        self._save_one_entry()
        self.assertEqual([p.name for p in self.cache.iterdir() if p.name.startswith(".")], [])

    def test_a_large_archive_is_split_into_parts_that_restore_in_order(self):
        name = self._save_one_entry(RAIKIRI_GATE_CACHE_PART_BYTES="100")
        parts = self._parts(name)
        self.assertGreater(len(parts), 3)
        self.assertTrue(all(p.stat().st_size <= 100 for p in parts))
        self.assertEqual(
            [line.split()[0] for line in (self.cache / f"{name}.info").read_text().splitlines() if line.startswith("part=")],
            [f"part={p.name}" for p in parts],
        )
        shutil.rmtree(self.target)
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.target / "llvm-cov-target/debug/deps/serde-1.rlib").exists())
        self.assertTrue((self.target / "debug/deps/serde-1.rlib").exists())

    def test_an_entry_with_a_missing_part_is_ignored(self):
        name = self._save_one_entry(RAIKIRI_GATE_CACHE_PART_BYTES="100")
        shutil.rmtree(self.target)
        self._parts(name)[1].unlink()
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no usable entry for this toolchain", result.stdout)
        self.assertFalse(self.target.exists())
        status = self._run("status")
        self.assertIn("incomplete, ignored", status.stdout)

    def test_an_entry_in_the_old_single_archive_format_is_ignored(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        archive = self.cache / f"{name}.tar.gz"
        archive.write_bytes(b"".join(p.read_bytes() for p in self._parts(name)))
        (self.cache / f"{name}.info").write_text(f"bytes={archive.stat().st_size}\n")
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no usable entry for this toolchain", result.stdout)

    def test_an_info_file_cannot_point_outside_its_own_parts(self):
        name = self._save_one_entry()
        shutil.rmtree(self.target)
        outside = self.tmp / "outside"
        outside.write_bytes(b"x")
        (self.cache / f"{name}.info").write_text(f"bytes=1\npart=../outside 1\n")
        result = self._run("restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no usable entry for this toolchain", result.stdout)

    def test_parts_of_an_entry_that_never_got_its_info_file_are_dropped_after_an_hour(self):
        self.cache.mkdir()
        old = self.cache / "gate-target-20000101T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb.part.aa"
        fresh = self.cache / "gate-target-20260101T000000Z-tcaaaaaaaaaaaa-lockbbbbbbbbbbbb.part.aa"
        old.write_bytes(b"x")
        fresh.write_bytes(b"x")
        two_hours_ago = old.stat().st_mtime - 7200
        os.utime(old, (two_hours_ago, two_hours_ago))
        self._populate_target()
        result = self._run("save")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(old.exists())
        self.assertTrue(fresh.exists())  # its saver may still be running

    def test_status_marks_the_entry_restore_would_use(self):
        name = self._save_one_entry()
        result = self._run("status")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"{name}", result.stdout)
        self.assertIn("restore would use this", result.stdout)


if __name__ == "__main__":
    unittest.main()
