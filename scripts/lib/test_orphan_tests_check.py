#!/usr/bin/env python3
"""scripts/lib/test_orphan_tests_check.py — unit tests for orphan_tests_check.py.

Every test in `MainExitCodeTests` writes a small temporary `crates/*/src/*.rs`
tree to disk and drives `main()` end-to-end (not just a single function's
pure logic), because the bug this checker exists to catch — a `tests.rs`
file with no `mod tests;` anywhere in its parent — has no unit-level analog:
it's a fact about *two files together*, so a fixture-based check is the only
kind that can actually reproduce it. Per this task's acceptance criteria,
`test_missing_mod_decl_is_reported_as_orphan` is the "deliberately broken"
case: it builds a tree exactly like a botched migration (content moved to
`tests.rs`, `mod tests;` never added to the parent) and asserts the checker
catches it non-zero.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v
"""

from __future__ import annotations

import contextlib
import io
import tempfile
import unittest
from pathlib import Path

from orphan_tests_check import find_orphans, main


def _write(root: Path, rel_path: str, content: str) -> None:
    path = root / rel_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


class FindOrphansTests(unittest.TestCase):
    def test_sibling_tests_rs_with_mod_decl_is_not_orphan(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget.rs", "pub struct Widget;\n\n#[cfg(test)]\nmod tests;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            self.assertEqual(find_orphans(root), [])

    def test_sibling_tests_rs_without_mod_decl_is_orphan(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # widget.rs never declares `mod tests;` — the exact shape a
            # migration that forgot to add the declaration produces.
            _write(root, "crates/foo/src/widget.rs", "pub struct Widget;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            orphans = find_orphans(root)
            self.assertEqual(len(orphans), 1)
            self.assertEqual(orphans[0].path, root / "crates/foo/src/widget/tests.rs")
            self.assertEqual(orphans[0].mod_name, "tests")

    def test_mod_rs_directory_form_also_satisfies_parent(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget/mod.rs", "pub struct Widget;\n\n#[cfg(test)]\nmod tests;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            self.assertEqual(find_orphans(root), [])

    def test_crate_root_level_underscore_tests_file(self) -> None:
        # crates/raikiri-html/src/document_tests.rs's real shape: a
        # `*_tests.rs` file directly under src/, declared from lib.rs.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/lib.rs", "#[cfg(test)]\nmod document_tests;\n")
            _write(root, "crates/foo/src/document_tests.rs", "#[test]\nfn it_works() {}\n")
            self.assertEqual(find_orphans(root), [])

    def test_crate_root_level_underscore_tests_file_without_mod_decl_is_orphan(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/lib.rs", "// forgot to declare it\n")
            _write(root, "crates/foo/src/document_tests.rs", "#[test]\nfn it_works() {}\n")
            orphans = find_orphans(root)
            self.assertEqual(len(orphans), 1)
            self.assertEqual(orphans[0].mod_name, "document_tests")

    def test_nested_hub_and_leaf_both_checked_independently(self) -> None:
        # property/tests.rs (hub) declares box_model_tests, and is itself
        # declared from property.rs — mirrors
        # crates/raikiri-style/src/property/{tests.rs,tests/box_model_tests.rs}.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/property.rs", "#[cfg(test)]\nmod tests;\n")
            _write(root, "crates/foo/src/property/tests.rs", "use super::*;\n\nmod box_model_tests;\n")
            _write(root, "crates/foo/src/property/tests/box_model_tests.rs", "#[test]\nfn it_works() {}\n")
            self.assertEqual(find_orphans(root), [])

    def test_hub_missing_leaf_declaration_is_orphan_even_though_hub_itself_is_reachable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/property.rs", "#[cfg(test)]\nmod tests;\n")
            # tests.rs is reachable, but forgets to declare box_model_tests.
            _write(root, "crates/foo/src/property/tests.rs", "use super::*;\n")
            _write(root, "crates/foo/src/property/tests/box_model_tests.rs", "#[test]\nfn it_works() {}\n")
            orphans = find_orphans(root)
            self.assertEqual(len(orphans), 1)
            self.assertEqual(orphans[0].mod_name, "box_model_tests")

    def test_hub_itself_unreachable_is_orphan_independent_of_its_own_leaf_declarations(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # property.rs never declares `mod tests;` — the hub file is
            # itself orphaned, even though it correctly declares its leaf.
            _write(root, "crates/foo/src/property.rs", "pub struct Property;\n")
            _write(root, "crates/foo/src/property/tests.rs", "use super::*;\n\nmod box_model_tests;\n")
            _write(root, "crates/foo/src/property/tests/box_model_tests.rs", "#[test]\nfn it_works() {}\n")
            orphans = find_orphans(root)
            self.assertEqual(len(orphans), 1)
            self.assertEqual(orphans[0].path, root / "crates/foo/src/property/tests.rs")
            self.assertEqual(orphans[0].mod_name, "tests")

    def test_mod_mentioned_only_in_comment_does_not_count(self) -> None:
        # A commented-out or prose mention of `mod tests;` must not be
        # mistaken for a real declaration — that would make the checker
        # blind to exactly the mistake it exists to catch.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget.rs", "// TODO: mod tests;\npub struct Widget;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            orphans = find_orphans(root)
            self.assertEqual(len(orphans), 1)

    def test_pub_mod_decl_counts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget.rs", "pub struct Widget;\n\n#[cfg(test)]\npub(crate) mod tests;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            self.assertEqual(find_orphans(root), [])


class MainExitCodeTests(unittest.TestCase):
    def test_missing_mod_decl_is_reported_as_orphan(self) -> None:
        """The deliberately-broken case: a tests.rs split that forgot its
        `mod tests;` line. main() must exit non-zero and name the file."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget.rs", "pub struct Widget;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                exit_code = main(["--repo-root", str(root)])
            self.assertEqual(exit_code, 1)
            self.assertIn("widget/tests.rs", buf.getvalue())

    def test_clean_tree_exits_zero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget.rs", "pub struct Widget;\n\n#[cfg(test)]\nmod tests;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                exit_code = main(["--repo-root", str(root)])
            self.assertEqual(exit_code, 0)

    def test_verbose_flag_lists_scanned_candidates(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget.rs", "pub struct Widget;\n\n#[cfg(test)]\nmod tests;\n")
            _write(root, "crates/foo/src/widget/tests.rs", "use super::*;\n\n#[test]\nfn it_works() {}\n")
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                main(["--repo-root", str(root), "-v"])
            self.assertIn("scanned:", buf.getvalue())

    def test_no_candidates_exits_zero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write(root, "crates/foo/src/widget.rs", "pub struct Widget;\n")
            exit_code = main(["--repo-root", str(root)])
            self.assertEqual(exit_code, 0)


if __name__ == "__main__":
    unittest.main()
