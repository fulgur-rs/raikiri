"""Integration checks for the persistent shared WPT cache and worktree links."""
from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class FetchCacheTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="raikiri-wpt-cache-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "home"
        self.project = self.root / "project"
        self.linked = self.root / "linked-worktree"
        self.wpt_source = self.root / "wpt-source"
        self.wpt_remote = self.root / "wpt-remote.git"
        self.cache = self.home / ".cache" / "raikiri" / "wpt"
        self._make_wpt_remote()
        self._make_project()
        self.env = os.environ.copy()
        self.env.update({"HOME": str(self.home), "WPT_REMOTE_URL": str(self.wpt_remote)})
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE"):
            self.env.pop(key, None)

    def _git(self, cwd: Path, *args: str) -> str:
        result = subprocess.run(
            ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
        )
        return result.stdout.strip()

    def _write(self, root: Path, relative: str, contents: str) -> None:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding="utf-8")

    def _init_git(self, root: Path) -> None:
        root.mkdir(parents=True, exist_ok=True)
        self._git(root, "init", "-q")
        self._git(root, "config", "user.name", "WPT cache test")
        self._git(root, "config", "user.email", "wpt-cache@example.invalid")

    def _make_wpt_remote(self) -> None:
        self._init_git(self.wpt_source)
        self._write(self.wpt_source, "acid/test.html", "acid root\n")
        self._write(self.wpt_source, "css/test.html", "css root\n")
        self._write(self.wpt_source, "fonts/ahem.txt", "font root\n")
        self._write(self.wpt_source, "images/ref.txt", "image root\n")
        self._write(self.wpt_source, "resources/testharness.js", "resources root\n")
        self._write(self.wpt_source, "tools/wptrunner/README.rst", "wptrunner root\n")
        self._write(self.wpt_source, "wpt", "wpt CLI\n")
        self._write(self.wpt_source, "docs/commands.json", "{}\n")
        self._write(self.wpt_source, "outside/not-sparse.txt", "outside roots\n")
        self._write(
            self.wpt_source, "html/dom/resources/nested.js", "nested per-test resources\n"
        )
        self._git(self.wpt_source, "add", "-A")
        self._git(self.wpt_source, "commit", "-qm", "fake pinned WPT")
        self.pin = self._git(self.wpt_source, "rev-parse", "HEAD")
        subprocess.run(
            ["git", "clone", "--bare", "-q", str(self.wpt_source), str(self.wpt_remote)],
            check=True,
            capture_output=True,
            text=True,
        )

    def _make_project(self) -> None:
        source_files = (
            "scripts/wpt/fetch.sh",
            "scripts/wpt/subset.txt",
            "scripts/wpt/lib/shared_sparse.sh",
            "scripts/wpt/lib/cache_path.sh",
            "scripts/lib/repo_root.sh",
            ".gitignore",
        )
        for relative in source_files:
            source = REPO_ROOT / relative
            destination = self.project / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        self._write(self.project, "scripts/wpt/pinned_sha.txt", self.pin + "\n")
        self._init_git(self.project)
        self._git(self.project, "add", "-A")
        self._git(self.project, "commit", "-qm", "minimal project fixture")

    def _fetch(self, cwd: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", "scripts/wpt/fetch.sh"],
            cwd=cwd,
            env=self.env,
            capture_output=True,
            text=True,
        )

    def _run_cache_link_helper(self, repo_root: Path) -> subprocess.CompletedProcess[str]:
        helper = repo_root / "scripts/wpt/lib/cache_path.sh"
        script = 'source "$1"; ensure_wpt_cache_link "$2" "$(wpt_cache_dir)"'
        return subprocess.run(
            ["bash", "-c", script, "bash", str(helper), str(repo_root)],
            cwd=repo_root,
            env=self.env,
            capture_output=True,
            text=True,
        )

    def test_cache_survives_target_cleanup_and_links_main_and_linked_worktrees(self) -> None:
        first = self._fetch(self.project)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(self.cache.resolve(), self.cache)
        self.assertEqual((self.cache / ".git" / "info" / "sparse-checkout").read_text(),
                         "acid\ncss\nfonts\nimages\n/resources\n/tools\n/wpt\n/docs/commands.json\n")
        self.assertEqual(
            (self.cache / ".git" / "info" / "sparse-checkout").stat().st_mode & 0o222,
            0,
        )
        self.assertEqual(self._git(self.cache, "rev-parse", "HEAD"), self.pin)
        self.assertFalse((self.cache / "outside/not-sparse.txt").exists())
        self.assertTrue((self.cache / "resources/testharness.js").exists())
        self.assertTrue((self.cache / "tools/wptrunner/README.rst").exists())
        self.assertTrue((self.cache / "wpt").exists())
        self.assertTrue((self.cache / "docs/commands.json").exists())
        self.assertFalse((self.cache / "html/dom/resources/nested.js").exists())
        main_link = self.project / "target/wpt"
        self.assertTrue(main_link.is_symlink())
        self.assertEqual(main_link.resolve(), self.cache)

        local_fixture = self.cache / "css/local-user-fixture.html"
        local_fixture.write_text("keep me\n", encoding="utf-8")
        shutil.rmtree(self.project / "target")
        self.assertTrue(self.cache.is_dir())
        restored = self._fetch(self.project)
        self.assertEqual(restored.returncode, 0, restored.stderr)
        self.assertEqual(main_link.resolve(), self.cache)
        self.assertEqual(local_fixture.read_text(encoding="utf-8"), "keep me\n")

        self._git(
            self.project,
            "worktree",
            "add",
            "--quiet",
            "-b",
            "cache-link-test",
            str(self.linked),
            "HEAD",
        )
        linked_fetch = self._fetch(self.linked)
        self.assertEqual(linked_fetch.returncode, 0, linked_fetch.stderr)
        linked_link = self.linked / "target/wpt"
        self.assertTrue(linked_link.is_symlink())
        self.assertEqual(linked_link.resolve(), self.cache)
        self.assertEqual(self._git(self.cache, "rev-parse", "HEAD"), self.pin)

        shutil.rmtree(self.linked / "target")
        repaired = self._run_cache_link_helper(self.linked)
        self.assertEqual(repaired.returncode, 0, repaired.stderr)
        self.assertEqual(linked_link.resolve(), self.cache)

    def test_fetch_migrates_the_locked_legacy_sparse_root_set(self) -> None:
        first = self._fetch(self.project)
        self.assertEqual(first.returncode, 0, first.stderr)
        sparse_file = self.cache / ".git" / "info" / "sparse-checkout"
        sparse_file.chmod(0o644)
        sparse_file.write_text(
            "acid\ncss\nfonts\nimages\n/resources\n",
            encoding="utf-8",
        )
        sparse_file.chmod(0o444)

        migrated = self._fetch(self.project)

        self.assertEqual(migrated.returncode, 0, migrated.stderr)
        self.assertEqual(
            sparse_file.read_text(encoding="utf-8"),
            "acid\ncss\nfonts\nimages\n/resources\n/tools\n/wpt\n/docs/commands.json\n",
        )
        self.assertEqual(sparse_file.stat().st_mode & 0o222, 0)

    def test_fetch_refuses_legacy_physical_checkout_without_creating_duplicate(self) -> None:
        legacy_git = self.project / "target/wpt/.git"
        legacy_git.mkdir(parents=True)
        result = self._fetch(self.project)
        self.assertEqual(result.returncode, 2)
        self.assertIn("legacy shared WPT checkout still exists", result.stderr)
        self.assertFalse(self.cache.exists())


if __name__ == "__main__":
    unittest.main()
