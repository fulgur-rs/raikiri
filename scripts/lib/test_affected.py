#!/usr/bin/env python3
"""scripts/lib/test_affected.py — unit tests for affected.py.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v

Exercises `select()` against a fixture `cargo metadata --no-deps` dict, so
no test here invokes `cargo` or `git`.
"""

import unittest

from affected import is_workspace_wide, select

ROOT = "/repo"


def pkg(name, deps=(), member=True):
    return {
        "id": f"path+file://{ROOT}/crates/{name}#0.1.0",
        "name": name,
        "manifest_path": f"{ROOT}/crates/{name}/Cargo.toml",
        "dependencies": [
            {"name": d, "kind": kind, "path": f"{ROOT}/crates/{d}"}
            for d, kind in deps
        ]
        + [{"name": "serde", "kind": None}],
        "_member": member,
    }


def metadata(*packages):
    return {
        "packages": list(packages),
        "workspace_members": [p["id"] for p in packages if p["_member"]],
    }


# traits <- dom <- html ; paint depends on dom and dev-depends on html ;
# app depends on paint ; bench dev-depends on paint.
META = metadata(
    pkg("traits"),
    pkg("dom", [("traits", None)]),
    pkg("html", [("dom", None)]),
    pkg("paint", [("dom", None), ("html", "dev")]),
    pkg("app", [("paint", None)]),
    pkg("bench", [("paint", "dev")]),
    pkg("leaf"),
)


class SelectTests(unittest.TestCase):
    def test_leaf_change_selects_only_that_crate(self):
        self.assertEqual(select(["crates/leaf/src/lib.rs"], META, ROOT), {"leaf"})

    def test_dev_edge_selects_dependent_without_propagating(self):
        # paint's tests use html, but app links paint's unchanged library.
        self.assertEqual(select(["crates/html/src/lib.rs"], META, ROOT), {"html", "paint"})

    def test_normal_edges_are_transitive(self):
        self.assertEqual(
            select(["crates/traits/src/lib.rs"], META, ROOT),
            {"traits", "dom", "html", "paint", "app", "bench"},
        )

    def test_normal_edge_wins_over_dev_edge_to_same_dependent(self):
        meta = metadata(pkg("a"), pkg("b", [("a", "dev"), ("a", None)]), pkg("c", [("b", None)]))
        self.assertEqual(select(["crates/a/src/lib.rs"], meta, ROOT), {"a", "b", "c"})

    def test_dev_selected_crate_still_propagates_once_its_library_changes(self):
        # html is first reached only via paint's dev edge, then changes itself.
        self.assertEqual(
            select(["crates/html/src/lib.rs", "crates/paint/src/lib.rs"], META, ROOT),
            {"html", "paint", "app", "bench"},
        )

    def test_direct_mode_skips_reverse_deps(self):
        self.assertEqual(
            select(["crates/traits/src/lib.rs"], META, ROOT, with_rdeps=False),
            {"traits"},
        )

    def test_paths_outside_crates_select_nothing(self):
        self.assertEqual(
            select(["scripts/gate.sh", "AGENTS.md", ".github/workflows/ci.yml"], META, ROOT),
            set(),
        )

    def test_crate_manifest_and_fixtures_belong_to_the_crate(self):
        self.assertEqual(
            select(["crates/leaf/Cargo.toml", "crates/leaf/expectations/a.txt"], META, ROOT),
            {"leaf"},
        )

    def test_root_manifest_selects_whole_workspace(self):
        self.assertEqual(
            select(["Cargo.lock"], META, ROOT),
            {"traits", "dom", "html", "paint", "app", "bench", "leaf"},
        )

    def test_prefix_sibling_is_not_mistaken_for_member(self):
        # `crates/dom-extra/` must not be attributed to `crates/dom`.
        self.assertEqual(select(["crates/dom-extra/x.rs"], META, ROOT), set())

    def test_non_member_dependents_are_ignored(self):
        meta = metadata(pkg("traits"), pkg("outside", [("traits", None)], member=False))
        self.assertEqual(select(["crates/traits/src/lib.rs"], meta, ROOT), {"traits"})


class WorkspaceWideTests(unittest.TestCase):
    def test_root_inputs(self):
        for path in ("Cargo.toml", "Cargo.lock", ".cargo/config.toml", "rust-toolchain.toml"):
            self.assertTrue(is_workspace_wide(path), path)

    def test_nested_manifests_are_not_workspace_wide(self):
        for path in ("crates/x/Cargo.toml", "crates/x/rust-toolchain.toml", "README.md"):
            self.assertFalse(is_workspace_wide(path), path)


if __name__ == "__main__":
    unittest.main()
