#!/usr/bin/env python3
"""scripts/lib/test_public_api_lint.py — unit tests for public_api_lint.py.

The tests build small rustdoc JSON documents by hand: one item per shape of
signature that can name a type (a parameter, a generic argument, a field, a
trait impl, a re-export), so that a change in what the walker looks at shows as
a failing case rather than as a lint that quietly stops seeing. `main()` is
driven end to end on a temporary directory of such documents, because the
failure the lint exists for is a fact about the documents together.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v
"""

from __future__ import annotations

import contextlib
import io
import tempfile
import unittest
from pathlib import Path

import public_api_lint as lint

TAFFY_CRATE_ID = 7
TRAITS_CRATE_ID = 8


def resolved(item_id: int, path: str, args: list | None = None) -> dict:
    return {
        "resolved_path": {
            "path": path,
            "id": item_id,
            "args": {"angle_bracketed": {"args": args, "constraints": []}} if args is not None else None,
        }
    }


def type_arg(inner: dict) -> dict:
    return {"type": inner}


class DocBuilder:
    """A rustdoc JSON document with a few items of the crate itself."""

    def __init__(self, *, with_taffy: bool = True, shift: int = 0) -> None:
        self.shift = shift
        self.index: dict[str, dict] = {}
        self.paths: dict[str, dict] = {}
        self.external = {str(TRAITS_CRATE_ID): {"name": "raikiri_traits"}}
        if with_taffy:
            self.external[str(TAFFY_CRATE_ID)] = {"name": "taffy"}
        self.taffy_node_id = self.foreign(100, TAFFY_CRATE_ID, ["taffy", "tree", "node", "NodeId"])
        self.taffy_size = self.foreign(101, TAFFY_CRATE_ID, ["taffy", "geometry", "Size"])
        self.taffy_trait = self.foreign(102, TAFFY_CRATE_ID, ["taffy", "tree", "traits", "TraverseTree"])
        # A type of the same name in a crate that is allowed.
        self.traits_node_id = self.foreign(103, TRAITS_CRATE_ID, ["raikiri_traits", "NodeId"])

    def foreign(self, key: int, crate_id: int, path: list[str]) -> int:
        key += self.shift
        self.paths[str(key)] = {"crate_id": crate_id, "path": path, "kind": "struct"}
        return key

    def add(
        self,
        key: int,
        name: str | None,
        inner: dict,
        *,
        visibility: str = "public",
        attrs: list | None = None,
        path: list[str] | None = None,
    ) -> int:
        key += self.shift
        self.index[str(key)] = {
            "id": key,
            "crate_id": 0,
            "name": name,
            "visibility": visibility,
            "attrs": attrs or [],
            "inner": inner,
        }
        if path is not None:
            self.paths[str(key)] = {"crate_id": 0, "path": path, "kind": "struct"}
        return key

    def function(self, key: int, name: str, inputs: list[dict], output: dict | None, **extra: object) -> int:
        sig = {
            "inputs": [[f"arg{i}", entry] for i, entry in enumerate(inputs)],
            "output": output,
            "is_c_variadic": False,
        }
        return self.add(key, name, {"function": {"sig": sig, "generics": {"params": [], "where_predicates": []}}}, **extra)

    def doc(self, format_version: int = 56, includes_private: bool = False) -> dict:
        return {
            "format_version": format_version,
            "includes_private": includes_private,
            "index": self.index,
            "paths": self.paths,
            "external_crates": self.external,
            "root": 0,
        }


def leaks_of(builder: DocBuilder) -> list[lint.Leak]:
    return lint.find_leaks("raikiri-x", builder.doc())


class WalkerTests(unittest.TestCase):
    def test_a_parameter_of_a_taffy_type_is_a_leak(self) -> None:
        b = DocBuilder()
        b.function(1, "child_count", [resolved(b.taffy_node_id, "NodeId")], None)
        (leak,) = leaks_of(b)
        self.assertEqual(leak.path, "child_count")
        self.assertEqual(leak.kind, "function")
        self.assertEqual(leak.names, ("taffy::tree::node::NodeId",))

    def test_a_generic_argument_of_a_return_type_is_a_leak(self) -> None:
        b = DocBuilder()
        vec = resolved(50, "Vec", [type_arg(resolved(b.taffy_size, "Size"))])
        b.function(1, "sizes", [], vec)
        (leak,) = leaks_of(b)
        self.assertEqual(leak.names, ("taffy::geometry::Size",))

    def test_a_struct_field_of_a_taffy_type_is_a_leak(self) -> None:
        b = DocBuilder()
        field = b.add(2, "origin", {"struct_field": resolved(b.taffy_size, "Size")})
        b.add(1, "Node", {"struct": {"kind": {"plain": {"fields": [field], "has_stripped_fields": False}}, "impls": []}}, path=["x", "Node"])
        (leak,) = leaks_of(b)
        self.assertEqual(leak.path, "x::Node::origin")
        self.assertEqual(leak.kind, "struct_field")

    def test_an_impl_of_a_taffy_trait_is_a_leak_with_its_methods(self) -> None:
        b = DocBuilder()
        method = b.function(3, "child_count", [resolved(b.taffy_node_id, "NodeId")], None)
        trait = {"path": "TraverseTree", "id": b.taffy_trait, "args": None}
        subject = b.add(4, "Doc", {"struct": {"kind": "unit", "impls": []}}, path=["x", "Doc"])
        b.add(
            2,
            None,
            {"impl": {"trait": trait, "for": resolved(subject, "Doc"), "items": [method], "blanket_impl": None}},
        )
        b.index[str(subject)]["inner"]["struct"]["impls"] = [2 + b.shift]
        paths = {leak.path: leak for leak in leaks_of(b)}
        self.assertEqual(
            sorted(paths),
            [
                "x::Doc::<impl taffy::tree::traits::TraverseTree for x::Doc>",
                "x::Doc::<impl taffy::tree::traits::TraverseTree for x::Doc>::child_count",
            ],
        )

    def test_a_reexport_of_a_taffy_item_is_a_leak(self) -> None:
        b = DocBuilder()
        b.add(1, "Size", {"use": {"source": "taffy::Size", "name": "Size", "id": b.taffy_size, "is_glob": False}})
        (leak,) = leaks_of(b)
        self.assertEqual(leak.kind, "use")

    def test_a_type_of_the_same_name_from_another_crate_is_not_a_leak(self) -> None:
        b = DocBuilder()
        b.function(1, "child_count", [resolved(b.traits_node_id, "NodeId")], resolved(b.traits_node_id, "NodeId"))
        self.assertEqual(leaks_of(b), [])

    def test_a_document_that_does_not_link_taffy_has_no_leak(self) -> None:
        b = DocBuilder(with_taffy=False)
        b.function(1, "f", [resolved(b.traits_node_id, "NodeId")], None)
        self.assertEqual(lint.find_leaks("raikiri-x", b.doc()), [])

    def test_items_of_other_crates_are_not_inspected(self) -> None:
        b = DocBuilder()
        b.index["900"] = {
            "id": 900,
            "crate_id": TAFFY_CRATE_ID,
            "name": "inherited",
            "visibility": "public",
            "attrs": [],
            "inner": {"struct_field": resolved(b.taffy_size, "Size")},
        }
        self.assertEqual(leaks_of(b), [])

    def test_unknown_format_version_stops_the_run(self) -> None:
        b = DocBuilder()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "x.json"
            path.write_text(__import__("json").dumps(b.doc(format_version=999)), encoding="utf-8")
            with self.assertRaises(lint.ToolError) as raised:
                lint.load_doc(path)
        self.assertIn("999", str(raised.exception))

    def test_json_with_private_items_stops_the_run(self) -> None:
        b = DocBuilder()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "x.json"
            path.write_text(__import__("json").dumps(b.doc(includes_private=True)), encoding="utf-8")
            with self.assertRaises(lint.ToolError):
                lint.load_doc(path)

    def test_unreadable_json_stops_the_run(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(lint.ToolError):
                lint.load_doc(Path(tmp) / "missing.json")
            broken = Path(tmp) / "broken.json"
            broken.write_text("{", encoding="utf-8")
            with self.assertRaises(lint.ToolError):
                lint.load_doc(broken)


class DumpTests(unittest.TestCase):
    def build(self, shift: int) -> DocBuilder:
        b = DocBuilder(shift=shift)
        field = b.add(2, "width", {"struct_field": resolved(b.traits_node_id, "NodeId")})
        b.add(
            1,
            "Boxed",
            {"struct": {"kind": {"plain": {"fields": [field], "has_stripped_fields": False}}, "impls": []}},
            path=["x", "Boxed"],
        )
        b.function(3, "helper", [resolved(b.traits_node_id, "NodeId")], None, attrs=[{"other": "#[doc(hidden)]"}])
        return b

    def test_the_dump_does_not_depend_on_how_items_are_numbered(self) -> None:
        first = lint.api_lines(self.build(0).doc())
        second = lint.api_lines(self.build(5000).doc())
        self.assertEqual(first, second)
        self.assertEqual(len(first), 3)

    def test_the_dump_names_items_by_path_and_marks_hidden_ones(self) -> None:
        lines = lint.api_lines(self.build(0).doc())
        self.assertTrue(any(line.startswith("helper [function hidden] ") for line in lines), lines)
        self.assertTrue(any(line.startswith("x::Boxed::width [struct_field] ") for line in lines), lines)
        self.assertTrue(all("raikiri_traits::NodeId" in line for line in lines if "width" in line or "helper" in line))

    def test_a_changed_signature_changes_the_dump(self) -> None:
        before = self.build(0)
        after = self.build(0)
        after.index["3"]["inner"]["function"]["sig"]["output"] = resolved(after.traits_node_id, "NodeId")
        self.assertNotEqual(lint.api_lines(before.doc()), lint.api_lines(after.doc()))


class MainTests(unittest.TestCase):
    def write_all(self, directory: Path, leaking: str | None = None, blind_view: bool = False) -> None:
        for crate in (*lint.CHECKED_CRATES, *lint.VIEW_CRATES):
            b = DocBuilder()
            b.function(1, "plain", [resolved(b.traits_node_id, "NodeId")], None)
            if crate == leaking or (crate in lint.VIEW_CRATES and not blind_view):
                b.function(2, "child_count", [resolved(b.taffy_node_id, "NodeId")], None)
            (directory / lint.json_file_name(crate)).write_text(
                __import__("json").dumps(b.doc()), encoding="utf-8"
            )

    def run_main(self, directory: Path, *extra: str) -> tuple[int, str, str]:
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = lint.main(["--repo-root", str(directory), "--json-dir", str(directory), *extra])
        return code, out.getvalue(), err.getvalue()

    def test_passes_when_only_the_view_crate_names_taffy(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp))
            code, out, _ = self.run_main(Path(tmp), "-v")
        self.assertEqual(code, 0, out)
        self.assertIn("PASS", out)
        self.assertIn("raikiri-layout-style: 1 items name Taffy (view crate, allowed)", out)

    def test_fails_and_names_the_item_when_a_checked_crate_leaks(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp), leaking="raikiri-paint")
            code, out, _ = self.run_main(Path(tmp))
        self.assertEqual(code, 1)
        self.assertIn("raikiri-paint: child_count (function): taffy::tree::node::NodeId", out)
        self.assertNotIn("raikiri-dom:", out)

    def test_a_view_crate_the_walker_sees_no_taffy_in_is_a_tool_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp), blind_view=True)
            code, _, err = self.run_main(Path(tmp))
        self.assertEqual(code, 2)
        self.assertIn("view crate", err)

    def test_a_missing_json_file_is_a_tool_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp))
            (Path(tmp) / lint.json_file_name("raikiri-html")).unlink()
            code, _, err = self.run_main(Path(tmp))
        self.assertEqual(code, 2)
        self.assertIn("raikiri_html.json", err)

    def test_a_crate_without_public_items_is_a_tool_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp))
            empty = DocBuilder()
            (Path(tmp) / lint.json_file_name("raikiri-traits")).write_text(
                __import__("json").dumps(empty.doc()), encoding="utf-8"
            )
            code, _, err = self.run_main(Path(tmp))
        self.assertEqual(code, 2)
        self.assertIn("raikiri-traits", err)

    def test_dump_prints_the_lines_of_one_crate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp))
            code, out, _ = self.run_main(Path(tmp), "--dump", "raikiri-html")
        self.assertEqual(code, 0)
        self.assertEqual(out.count("\n"), 1)
        self.assertTrue(out.startswith("plain [function] "), out)

    def test_dump_of_a_crate_that_is_not_checked_is_a_tool_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            code, _, err = self.run_main(Path(tmp), "--dump", "serde")
        self.assertEqual(code, 2)
        self.assertIn("--dump takes one of", err)


if __name__ == "__main__":
    unittest.main()
