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
from unittest import mock

import public_api_lint as lint

TAFFY_CRATE_ID = 7
TRAITS_CRATE_ID = 8
SERDE_CRATE_ID = 9
ROOT_KINDS = ("function", "struct", "enum", "trait", "constant", "type_alias", "module", "use", "extern_crate")
CORE_CRATE_ID = 10
HIDDEN = [{"other": "#[doc(hidden)]"}]


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
        self.external = {str(TRAITS_CRATE_ID): {"name": "raikiri_traits"}, str(SERDE_CRATE_ID): {"name": "serde"}}
        if with_taffy:
            self.external[str(TAFFY_CRATE_ID)] = {"name": "taffy"}
        self.taffy_node_id = self.foreign(100, TAFFY_CRATE_ID, ["taffy", "tree", "node", "NodeId"])
        self.taffy_size = self.foreign(101, TAFFY_CRATE_ID, ["taffy", "geometry", "Size"])
        self.taffy_trait = self.foreign(102, TAFFY_CRATE_ID, ["taffy", "tree", "traits", "TraverseTree"])
        # A type of the same name in a crate that is allowed.
        self.traits_node_id = self.foreign(103, TRAITS_CRATE_ID, ["raikiri_traits", "NodeId"])
        # A type of a crate outside the workspace, and one of the standard library.
        self.serde_type = self.foreign(104, SERDE_CRATE_ID, ["serde", "Serialize"])
        self.core_type = self.foreign(105, CORE_CRATE_ID, ["core", "option", "Option"])
        self.external[str(CORE_CRATE_ID)] = {"name": "core"}

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

    def doc(self, format_version: int = 56, includes_private: bool = False, root: str | None = None) -> dict:
        index = self.index
        if root is not None:
            # A root module, named `root`, that lists the items declared at the top level.
            members = [item["id"] for item in index.values() if next(iter(item["inner"])) in ROOT_KINDS]
            module = {"module": {"is_crate": True, "items": members, "is_stripped": False}}
            index = {**index, "0": {"id": 0, "crate_id": 0, "name": root, "visibility": "public", "attrs": [], "inner": module}}
        return {
            "format_version": format_version,
            "includes_private": includes_private,
            "index": index,
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


class ThirdPartyTests(unittest.TestCase):
    def named(self, builder: DocBuilder) -> dict[str, list[str]]:
        return lint.third_party_crates(builder.doc(root="raikiri_x"))

    def test_a_crate_outside_the_workspace_in_a_signature_is_named(self) -> None:
        b = DocBuilder()
        b.function(1, "go", [resolved(b.serde_type, "Serialize")], None)
        self.assertEqual(self.named(b), {"serde": ["serde::Serialize"]})

    def test_the_standard_library_and_the_workspace_are_not_third_party(self) -> None:
        b = DocBuilder()
        b.function(1, "go", [resolved(b.traits_node_id, "NodeId")], resolved(b.core_type, "Option"))
        self.assertEqual(self.named(b), {})

    def test_a_hidden_item_does_not_count(self) -> None:
        b = DocBuilder()
        b.function(1, "go", [resolved(b.serde_type, "Serialize")], None, attrs=[{"other": "#[doc(hidden)]"}])
        self.assertEqual(self.named(b), {})

    def test_a_field_of_a_visible_type_counts(self) -> None:
        b = DocBuilder()
        field = b.add(2, "inner", {"struct_field": resolved(b.serde_type, "Serialize")})
        b.add(
            1,
            "Holder",
            {"struct": {"kind": {"plain": {"fields": [field], "has_stripped_fields": False}}, "impls": []}},
            path=["raikiri_x", "Holder"],
        )
        self.assertEqual(self.named(b), {"serde": ["serde::Serialize"]})

    def test_a_blanket_impl_of_a_foreign_trait_is_not_the_api_of_the_crate(self) -> None:
        b = DocBuilder()
        blanket = b.add(
            3,
            None,
            {
                "impl": {
                    "is_synthetic": False,
                    "blanket_impl": resolved(b.serde_type, "T"),
                    "trait": {"path": "Serialize", "id": b.serde_type, "args": None},
                    "for": {"generic": "T"},
                    "items": [],
                }
            },
        )
        b.add(
            1,
            "Holder",
            {"struct": {"kind": "unit", "impls": [blanket]}},
            path=["raikiri_x", "Holder"],
        )
        self.assertEqual(self.named(b), {})

    def test_an_impl_of_a_foreign_trait_for_a_visible_type_counts(self) -> None:
        b = DocBuilder()
        implementation = b.add(
            3,
            None,
            {
                "impl": {
                    "is_synthetic": False,
                    "blanket_impl": None,
                    "trait": {"path": "Serialize", "id": b.serde_type, "args": None},
                    "for": {"generic": "Self"},
                    "items": [],
                }
            },
        )
        b.add(1, "Holder", {"struct": {"kind": "unit", "impls": [implementation]}}, path=["raikiri_x", "Holder"])
        self.assertEqual(self.named(b), {"serde": ["serde::Serialize"]})


class ReexportTests(unittest.TestCase):
    """A `pub use` publishes the type of another crate whether or not a signature mentions it."""

    def named(self, builder: DocBuilder) -> dict[str, list[str]]:
        return lint.third_party_crates(builder.doc(root="raikiri_x"))

    def reexport(self, builder: DocBuilder, key: int, target: int, name: str, *, glob: bool = False) -> int:
        inner = {"use": {"source": name, "name": name, "id": target, "is_glob": glob}}
        return builder.add(key, None, inner)

    def test_a_reexported_type_counts_with_no_signature_that_mentions_it(self) -> None:
        b = DocBuilder()
        self.reexport(b, 1, b.serde_type, "Serialize")
        self.assertEqual(self.named(b), {"serde": ["serde::Serialize"]})

    def test_a_glob_of_a_module_of_another_crate_counts_as_naming_that_crate(self) -> None:
        b = DocBuilder()
        module = b.foreign(106, SERDE_CRATE_ID, ["serde", "de"])
        self.reexport(b, 1, module, "de", glob=True)
        self.assertEqual(self.named(b), {"serde": ["serde::de"]})

    def test_a_reexport_of_a_published_crate_or_a_hidden_one_does_not(self) -> None:
        b = DocBuilder()
        self.reexport(b, 1, b.traits_node_id, "NodeId")
        hidden = self.reexport(b, 2, b.serde_type, "Hidden")
        b.index[str(hidden)]["attrs"] = [{"other": "#[doc(hidden)]"}]
        self.assertEqual(self.named(b), {})

    def test_an_unpublished_workspace_crate_is_third_party_to_a_consumer(self) -> None:
        b = DocBuilder()
        b.external["11"] = {"name": "raikiri_paint"}
        paint = b.foreign(107, 11, ["raikiri_paint", "Thing"])
        b.function(1, "go", [resolved(paint, "Thing")], None)
        self.assertEqual(self.named(b), {"raikiri_paint": ["raikiri_paint::Thing"]})

    def test_a_view_crate_with_no_taffy_in_the_scan_is_a_tool_error(self) -> None:
        def load(crate: str) -> dict:
            b = DocBuilder()
            b.function(1, "plain", [resolved(b.traits_node_id, "NodeId")], None)
            return b.doc(root=crate.replace("-", "_"))

        with self.assertRaisesRegex(lint.ToolError, "view crate"):
            lint.third_party_report(load)


class ReachTests(unittest.TestCase):
    """What the scan counts as part of the API besides the items the names stand for."""

    def test_a_type_that_a_signature_returns_counts_though_no_name_leads_to_it(self) -> None:
        b = DocBuilder()
        field = b.add(6, "field", {"struct_field": resolved(b.serde_type, "Serialize")})
        hidden_away = b.add(
            5,
            "Unnameable",
            {"struct": {"kind": {"plain": {"fields": [field], "has_stripped_fields": False}}, "impls": []}},
            path=["raikiri_x", "private", "Unnameable"],
        )
        b.function(1, "make", [], resolved(hidden_away, "Unnameable"))
        doc = b.doc(root="raikiri_x")
        doc["index"]["0"]["inner"]["module"]["items"].remove(hidden_away)  # in a private module, not re-exported
        self.assertEqual(lint.third_party_crates(doc), {"serde": ["serde::Serialize"]})

    def test_an_impl_of_a_local_trait_for_a_foreign_type_counts(self) -> None:
        b = DocBuilder()
        implementation = b.add(
            8,
            None,
            {
                "impl": {
                    "is_synthetic": False,
                    "blanket_impl": None,
                    "trait": {"path": "Local", "id": 7, "args": None},
                    "for": resolved(b.serde_type, "Serialize"),
                    "items": [],
                }
            },
        )
        b.add(
            7,
            "Local",
            {"trait": {"items": [], "implementations": [implementation]}},
            path=["raikiri_x", "Local"],
        )
        self.assertEqual(lint.third_party_crates(b.doc(root="raikiri_x")), {"serde": ["serde::Serialize"]})

    def test_a_function_is_scanned_though_a_module_has_the_same_path(self) -> None:
        b = DocBuilder()
        b.add(1, "collision", {"module": {"is_crate": False, "items": [], "is_stripped": False}})
        b.function(2, "collision", [], resolved(b.serde_type, "Serialize"))
        self.assertEqual(lint.third_party_crates(b.doc(root="raikiri_x")), {"serde": ["serde::Serialize"]})

    def test_globs_of_two_third_party_modules_under_one_prefix_are_both_reported(self) -> None:
        b = DocBuilder()
        b.foreign(60, SERDE_CRATE_ID, ["serde", "de"])
        b.foreign(61, TAFFY_CRATE_ID, ["taffy", "tree"])
        for key in (60, 61):
            b.paths[str(key)]["kind"] = "module"
        globs = [
            b.add(10 + i, None, {"use": {"source": "api", "name": "api", "id": target, "is_glob": True}})
            for i, target in enumerate((60, 61))
        ]
        b.add(2, "computed", {"module": {"is_crate": False, "items": globs, "is_stripped": False}})
        found = lint.third_party_crates(b.doc(root="raikiri_x"))
        self.assertEqual(sorted(found), ["serde", "taffy"])

    def test_variants_published_by_a_glob_of_a_local_enum_count(self) -> None:
        b = DocBuilder()
        field = b.add(7, "0", {"struct_field": resolved(b.serde_type, "Serialize")})
        variant = b.add(6, "V", {"variant": {"kind": {"tuple": [field]}, "discriminant": None}})
        enum = b.add(5, "E", {"enum": {"variants": [variant], "impls": []}}, path=["raikiri_x", "private", "E"])
        b.add(1, "E", {"use": {"source": "private::E", "name": "E", "id": enum, "is_glob": True}})
        doc = b.doc(root="raikiri_x")
        doc["index"]["0"]["inner"]["module"]["items"].remove(enum)  # in a private module
        self.assertEqual(lint.third_party_crates(doc), {"serde": ["serde::Serialize"]})

    def test_a_variant_survives_a_module_of_the_same_name(self) -> None:
        b = DocBuilder()
        field = b.add(7, "0", {"struct_field": resolved(b.serde_type, "Serialize")})
        variant = b.add(6, "V", {"variant": {"kind": {"tuple": [field]}, "discriminant": None}})
        enum = b.add(5, "E", {"enum": {"variants": [variant], "impls": []}}, path=["raikiri_x", "private", "E"])
        # The module comes first: it must not take the place of the variant.
        b.add(2, "V", {"module": {"is_crate": False, "items": [], "is_stripped": False}})
        b.add(1, "E", {"use": {"source": "private::E", "name": "E", "id": enum, "is_glob": True}})
        doc = b.doc(root="raikiri_x")
        doc["index"]["0"]["inner"]["module"]["items"].remove(enum)
        self.assertEqual(lint.third_party_crates(doc), {"serde": ["serde::Serialize"]})

    def test_an_item_hidden_with_other_doc_options_is_not_scanned(self) -> None:
        b = DocBuilder()
        b.function(
            1,
            "secret",
            [],
            resolved(b.serde_type, "Serialize"),
            attrs=[{"other": '#[doc(hidden, alias = "secret")]'}],
        )
        self.assertEqual(lint.third_party_crates(b.doc(root="raikiri_x")), {})

    def test_a_public_item_counts_even_when_no_name_leads_to_it(self) -> None:
        b = DocBuilder()
        stray = b.function(1, "stray", [], resolved(b.serde_type, "Serialize"))
        doc = b.doc(root="raikiri_x")
        doc["index"]["0"]["inner"]["module"]["items"].remove(stray)  # as if the surface had missed it
        self.assertEqual(lint.third_party_crates(doc), {"serde": ["serde::Serialize"]})

    def test_what_sits_below_a_hidden_module_is_not_a_root(self) -> None:
        b = DocBuilder()
        field = b.add(4, "field", {"struct_field": resolved(b.serde_type, "Serialize")})
        record = b.add(
            3, "Record", {"struct": {"kind": {"plain": {"fields": [field], "has_stripped_fields": False}}, "impls": []}}
        )
        function = b.function(2, "make", [], resolved(b.serde_type, "Serialize"))
        b.add(1, "internals", {"module": {"is_crate": False, "items": [function, record], "is_stripped": False}}, attrs=HIDDEN)
        doc = b.doc(root="raikiri_x")
        # Only the module is declared at the top level.
        doc["index"]["0"]["inner"]["module"]["items"] = [1]
        self.assertEqual(lint.third_party_crates(doc), {})

    def test_a_reexport_in_a_hidden_module_is_not_a_root(self) -> None:
        b = DocBuilder()
        reexport = b.add(3, "Serialize", {"use": {"source": "serde::Serialize", "name": "Serialize", "id": b.serde_type, "is_glob": False}})
        nested = b.add(4, "nested", {"module": {"is_crate": False, "items": [], "is_stripped": False}})
        b.function(5, "deep", [], resolved(b.serde_type, "Serialize"))
        b.index[str(nested)]["inner"]["module"]["items"] = [5]
        b.add(1, "hidden", {"module": {"is_crate": False, "items": [reexport, nested], "is_stripped": False}}, attrs=HIDDEN)
        doc = b.doc(root="raikiri_x")
        doc["index"]["0"]["inner"]["module"]["items"] = [1]
        self.assertEqual(lint.third_party_crates(doc), {})

    def test_a_reexport_below_a_hidden_alias_of_a_private_module_is_not_a_root(self) -> None:
        b = DocBuilder()
        reexport = b.add(3, "Serialize", {"use": {"source": "serde::Serialize", "name": "Serialize", "id": b.serde_type, "is_glob": False}})
        inner = b.add(2, "inner", {"module": {"is_crate": False, "items": [reexport], "is_stripped": False}})
        b.add(1, "alias", {"use": {"source": "inner", "name": "alias", "id": inner, "is_glob": False}}, attrs=HIDDEN)
        doc = b.doc(root="raikiri_x")
        doc["index"]["0"]["inner"]["module"]["items"] = [1]  # `inner` is private
        self.assertEqual(lint.third_party_crates(doc), {})

    def test_a_public_extern_crate_counts_as_naming_the_crate(self) -> None:
        b = DocBuilder()
        # rustdoc 1.91 writes the exported name to `name` and the crate to `rename`.
        b.add(1, "renamed", {"extern_crate": {"name": "renamed", "rename": "serde"}})
        b.add(2, "raikiri_traits", {"extern_crate": {"name": "raikiri_traits", "rename": None}})
        b.add(3, "alias", {"extern_crate": {"name": "alias", "rename": "core"}})
        self.assertEqual(lint.third_party_crates(b.doc(root="raikiri_x")), {"serde": ["serde"]})

    def test_an_extern_crate_cannot_borrow_the_name_of_a_standard_crate(self) -> None:
        b = DocBuilder()
        b.add(1, "std", {"extern_crate": {"name": "std", "rename": "serde"}})  # extern crate serde as std;
        self.assertEqual(lint.third_party_crates(b.doc(root="raikiri_x")), {"serde": ["serde"]})

    def test_a_blanket_impl_listed_by_a_trait_does_not(self) -> None:
        b = DocBuilder()
        blanket = b.add(
            8,
            None,
            {
                "impl": {
                    "is_synthetic": False,
                    "blanket_impl": resolved(b.serde_type, "T"),
                    "trait": {"path": "Local", "id": 7, "args": None},
                    "for": {"generic": "T"},
                    "items": [],
                }
            },
        )
        b.add(7, "Local", {"trait": {"items": [], "implementations": [blanket]}}, path=["raikiri_x", "Local"])
        self.assertEqual(lint.third_party_crates(b.doc(root="raikiri_x")), {})


class MainTests(unittest.TestCase):
    def write_all(
        self,
        directory: Path,
        leaking: str | None = None,
        blind_view: bool = False,
        naming_serde: str | None = None,
    ) -> None:
        """Write the JSON of every crate; `naming_serde` is a crate whose API names a type of serde."""
        for crate in lint.ALL_CRATES:
            b = DocBuilder()
            b.function(1, "plain", [resolved(b.traits_node_id, "NodeId")], None)
            if crate == leaking or (crate in lint.VIEW_CRATES and not blind_view):
                b.function(2, "child_count", [resolved(b.taffy_node_id, "NodeId")], None)
            if crate == naming_serde:
                b.function(3, "outside", [resolved(b.serde_type, "Serialize")], None)
            (directory / lint.json_file_name(crate)).write_text(
                __import__("json").dumps(b.doc(root=crate.replace("-", "_"))), encoding="utf-8"
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
        self.assertIn("raikiri-paint: raikiri_paint::child_count (function): taffy::tree::node::NodeId", out)
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
        # The root module and the one function.
        self.assertEqual(out.count("\n"), 2)
        self.assertIn("raikiri_html::plain [function] ", out)

    def write_published(self, directory: Path) -> None:
        self.write_all(directory)

    def test_surface_prints_the_names_of_one_crate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_published(Path(tmp))
            code, out, _ = self.run_main(Path(tmp), "--surface", "raikiri-net")
        self.assertEqual((code, out), (0, "function   raikiri_net::plain\n"))

    def test_exhaustive_prints_the_types_a_user_can_match_completely(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_published(Path(tmp))
            b = DocBuilder()
            b.add(1, "Kind", {"enum": {"variants": [1, 2, 3], "impls": []}}, path=["raikiri_net", "Kind"])
            b.add(2, "Sealed", {"enum": {"variants": [1]}}, attrs=["non_exhaustive"], path=["raikiri_net", "Sealed"])
            (Path(tmp) / lint.json_file_name("raikiri-net")).write_text(
                __import__("json").dumps(b.doc(root="raikiri_net")), encoding="utf-8"
            )
            code, out, _ = self.run_main(Path(tmp), "--exhaustive", "raikiri-net")
        self.assertEqual((code, out), (0, "enum       raikiri_net::Kind  (3 variants)\n"))

    def test_usage_reports_the_names_that_sources_reach(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_published(Path(tmp))
            sources = Path(tmp) / "src"
            sources.mkdir()
            (sources / "main.rs").write_text("use raikiri_html::plain;\nuse raikiri_dom::missing;\n", encoding="utf-8")
            code, out, _ = self.run_main(Path(tmp), "--usage", str(sources))
        self.assertEqual(code, 0)
        self.assertIn("raikiri_html: 1 names used", out)
        self.assertIn("function   raikiri_html::plain", out)
        self.assertIn("raikiri_dom::missing", out)

    def test_usage_of_something_that_is_not_a_directory_is_a_tool_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_published(Path(tmp))
            code, _, err = self.run_main(Path(tmp), "--usage", str(Path(tmp) / "nowhere"))
        self.assertEqual(code, 2)
        self.assertIn("--usage takes a directory", err)

    def test_fails_when_a_published_crate_names_an_unlisted_third_party_crate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp), naming_serde="raikiri-net")
            code, out, _ = self.run_main(Path(tmp))
        self.assertEqual(code, 1)
        self.assertIn("raikiri-net: serde (serde::Serialize)", out)
        self.assertIn("THIRD_PARTY_ALLOWED", out)

    def test_a_listed_third_party_crate_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp), naming_serde="raikiri-net")
            with mock.patch.dict(lint.THIRD_PARTY_ALLOWED["raikiri-net"], {"serde": "for the test"}):
                code, out, _ = self.run_main(Path(tmp))
        self.assertEqual(code, 0)
        self.assertIn("PASS", out)

    def test_a_listed_crate_that_is_no_longer_named_is_a_note_not_a_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.write_all(Path(tmp))
            code, out, _ = self.run_main(Path(tmp))
        self.assertEqual(code, 0)
        self.assertIn("note: raikiri-html no longer names a type of", out)
        self.assertIn("drop it from THIRD_PARTY_ALLOWED", out)

    def test_every_published_crate_has_an_entry_in_the_table(self) -> None:
        self.assertEqual(set(lint.THIRD_PARTY_ALLOWED), set(lint.PUBLISHED_CRATES))

    def test_dump_of_a_crate_that_is_not_checked_is_a_tool_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            code, _, err = self.run_main(Path(tmp), "--dump", "serde")
        self.assertEqual(code, 2)
        self.assertIn("--dump takes one of", err)


if __name__ == "__main__":
    unittest.main()
