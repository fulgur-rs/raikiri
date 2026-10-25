#!/usr/bin/env python3
"""scripts/lib/test_public_api_surface.py — unit tests for public_api_surface.py.

The surface is read from small rustdoc JSON documents built by hand: a module
tree with the shapes that decide what a name is (a declared item, a `pub use`
of a private item, a glob of a module, a re-export of another crate's item, a
hidden item, a cycle). The usage scan is driven on a temporary directory of
Rust sources.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v
"""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import public_api_surface as surface

HIDDEN = [{"other": "#[doc(hidden)]"}]


class Doc:
    """A rustdoc JSON document of a crate named `x` with a root module."""

    def __init__(self) -> None:
        self.index: dict[str, dict] = {}
        self.paths: dict[str, dict] = {}
        self.root = self.module(0, "x", [])

    def _add(self, key: int, name: str | None, inner: dict, attrs: list | None, path: list[str] | None) -> int:
        self.index[str(key)] = {
            "id": key,
            "crate_id": 0,
            "name": name,
            "visibility": "public",
            "attrs": attrs or [],
            "inner": inner,
        }
        if path is not None:
            self.paths[str(key)] = {"crate_id": 0, "path": path, "kind": "struct"}
        return key

    def module(self, key: int, name: str, items: list[int], attrs: list | None = None) -> int:
        inner = {"module": {"is_crate": key == 0, "items": items, "is_stripped": False}}
        return self._add(key, name, inner, attrs, None)

    def struct(self, key: int, name: str, path: list[str] | None = None, attrs: list | None = None) -> int:
        inner = {"struct": {"kind": "unit", "impls": []}}
        return self._add(key, name, inner, attrs, path)

    def plain_struct(
        self, key: int, name: str, public: int, *, stripped: bool = False, attrs: list | None = None
    ) -> int:
        shape = {"plain": {"fields": list(range(100, 100 + public)), "has_stripped_fields": stripped}}
        return self._add(key, name, {"struct": {"kind": shape, "impls": []}}, attrs, ["x", name])

    def tuple_struct(self, key: int, name: str, fields: list[int | None], *, attrs: list | None = None) -> int:
        return self._add(key, name, {"struct": {"kind": {"tuple": fields}, "impls": []}}, attrs, ["x", name])

    def enum(self, key: int, name: str, variants: int, *, attrs: list | None = None) -> int:
        inner = {"enum": {"variants": list(range(200, 200 + variants)), "impls": []}}
        return self._add(key, name, inner, attrs, ["x", name])

    def function(self, key: int, name: str, path: list[str] | None = None) -> int:
        return self._add(key, name, {"function": {}}, None, path)

    def use(self, key: int, name: str, target: int, *, glob: bool = False, attrs: list | None = None) -> int:
        inner = {"use": {"source": name, "name": name, "id": target, "is_glob": glob}}
        return self._add(key, None, inner, attrs, None)

    def foreign(self, key: int, path: list[str], kind: str = "struct") -> int:
        self.paths[str(key)] = {"crate_id": 3, "path": path, "kind": kind}
        return key

    def set_items(self, key: int, items: list[int]) -> None:
        self.index[str(key)]["inner"]["module"]["items"] = items

    def json(self) -> dict:
        return {"index": self.index, "paths": self.paths, "root": 0, "external_crates": {}}


def listed(doc: Doc) -> dict[str, surface.Name]:
    return {name.path: name for name in surface.names(doc.json())}


class NamesTests(unittest.TestCase):
    def test_declared_items_and_modules_are_named_by_the_path_to_them(self) -> None:
        doc = Doc()
        inner = doc.struct(3, "Inner", ["x", "m", "Inner"])
        module = doc.module(2, "m", [inner])
        top = doc.function(1, "top", ["x", "top"])
        doc.set_items(0, [top, module])
        found = listed(doc)
        self.assertEqual(sorted(found), ["x::m", "x::m::Inner", "x::top"])
        self.assertEqual(found["x::m"].kind, "module")
        self.assertEqual(found["x::top"].kind, "function")
        self.assertEqual(found["x::m::Inner"].canonical, "x::m::Inner")

    def test_a_pub_use_of_a_private_item_publishes_it_under_the_new_name(self) -> None:
        doc = Doc()
        page = doc.struct(2, "Page", ["x", "private", "Page"])
        alias = doc.use(1, "Page", page)
        doc.set_items(0, [alias])
        found = listed(doc)
        self.assertEqual(list(found), ["x::Page"])
        self.assertEqual(found["x::Page"].kind, "struct")
        self.assertEqual(found["x::Page"].canonical, "x::private::Page")

    def test_a_glob_of_a_module_of_the_crate_publishes_its_items_at_the_use_site(self) -> None:
        doc = Doc()
        a = doc.struct(3, "A", ["x", "inner", "A"])
        inner = doc.module(2, "inner", [a])
        glob = doc.use(1, "inner", inner, glob=True)
        doc.set_items(0, [glob])
        self.assertEqual(list(listed(doc)), ["x::A"])

    def test_a_reexport_of_another_crates_item_keeps_the_path_it_comes_from(self) -> None:
        doc = Doc()
        foreign = doc.foreign(9, ["other", "deep", "Thing"])
        reexport = doc.use(1, "Thing", foreign)
        doc.set_items(0, [reexport])
        found = listed(doc)
        self.assertEqual(found["x::Thing"].kind, "struct")
        self.assertEqual(found["x::Thing"].canonical, "other::deep::Thing")

    def test_a_glob_of_another_crates_module_is_listed_as_a_glob(self) -> None:
        doc = Doc()
        foreign = doc.foreign(9, ["other", "api"], kind="module")
        glob = doc.use(1, "api", foreign, glob=True)
        module = doc.module(2, "computed", [glob])
        doc.set_items(0, [module])
        found = listed(doc)
        self.assertEqual(found["x::computed::*"].kind, "glob")
        self.assertEqual(found["x::computed::*"].canonical, "other::api")

    def test_hidden_items_are_marked_and_so_is_everything_below_a_hidden_module(self) -> None:
        doc = Doc()
        a = doc.struct(3, "A", ["x", "m", "A"])
        module = doc.module(2, "m", [a], attrs=HIDDEN)
        hidden = doc.function(1, "internal", ["x", "internal"])
        doc.index["1"]["attrs"] = HIDDEN
        doc.set_items(0, [hidden, module])
        found = listed(doc)
        self.assertTrue(all(name.hidden for name in found.values()), found)
        self.assertEqual(surface.format_names(found.values()), "")
        self.assertIn("(hidden)", surface.format_names(found.values(), with_hidden=True))

    def test_a_hidden_reexport_does_not_hide_the_visible_way_to_the_same_name(self) -> None:
        doc = Doc()
        item = doc.struct(4, "Same", ["x", "p", "Same"])
        hidden_use = doc.use(1, "Same", item, attrs=HIDDEN)
        visible_use = doc.use(2, "Same", item)
        doc.set_items(0, [hidden_use, visible_use])
        self.assertFalse(listed(doc)["x::Same"].hidden)

    def test_a_module_that_globs_itself_does_not_loop(self) -> None:
        doc = Doc()
        a = doc.struct(3, "A", ["x", "m", "A"])
        module = doc.module(2, "m", [a])
        self_glob = doc.use(4, "m", module, glob=True)
        doc.set_items(2, [a, self_glob])
        doc.set_items(0, [module])
        self.assertEqual(sorted(listed(doc)), ["x::m", "x::m::A"])

    def test_the_text_form_has_one_line_per_name_and_shows_where_a_reexport_comes_from(self) -> None:
        doc = Doc()
        foreign = doc.foreign(9, ["other", "Thing"])
        doc.set_items(0, [doc.use(1, "Thing", foreign), doc.function(2, "go", ["x", "go"])])
        text = surface.format_names(surface.names(doc.json()))
        self.assertEqual(
            text.splitlines(),
            ["struct     x::Thing  <- other::Thing", "function   x::go"],
        )


class SourceTests(unittest.TestCase):
    def test_comments_and_literals_do_not_name_anything(self) -> None:
        source = (
            'let a = "raikiri_x::A"; // raikiri_y::B\n'
            "/* raikiri_z::C /* nested raikiri_n::N */ still */ let l: &'a str = r#\"raikiri_w::D\"#;\n"
            "let c = 'x'; let e = '\\n'; raikiri_ok::E;\n"
        )
        paths, globs = surface.used_paths(source)
        self.assertEqual(paths, {"raikiri_ok::E"})
        self.assertEqual(globs, set())

    def test_raw_byte_and_c_strings_are_literals_too(self) -> None:
        source = 'let a = br"x\\"; raikiri_a::A; let b = cr#"y\\"#; raikiri_b::B; let c = b"raikiri_z::Z";\n'
        paths, _ = surface.used_paths(source)
        self.assertEqual(paths, {"raikiri_a::A", "raikiri_b::B"})

    def test_blanking_keeps_the_length_even_for_a_literal_that_never_closes(self) -> None:
        for source in ('x "abc', 'x "a\\', "x \"", '"', "r\"abc", "/* open", "'a"):
            self.assertEqual(len(surface.blank_comments_and_strings(source)), len(source), source)

    def test_a_crate_alias_is_followed(self) -> None:
        source = (
            "use raikiri_dom as dom;\nextern crate raikiri_html as html;\n"
            "fn f() { dom::Document::new(); html::layout(); other::dom::Not::x(); }\n"
        )
        paths, _ = surface.used_paths(source)
        self.assertEqual(paths, {"raikiri_dom", "raikiri_dom::Document::new", "raikiri_html::layout"})

    def test_a_use_tree_expands_to_the_paths_it_names(self) -> None:
        tree = "raikiri_html::{Page, Fragment as F, layout, document_layout::{self, Foo}, *}"
        self.assertEqual(
            surface.expand_use_tree(tree),
            [
                "raikiri_html::Page",
                "raikiri_html::Fragment",
                "raikiri_html::layout",
                "raikiri_html::document_layout",
                "raikiri_html::document_layout::Foo",
                "raikiri_html::*",
            ],
        )
        self.assertEqual(surface.expand_use_tree("raikiri_dom::Document"), ["raikiri_dom::Document"])
        self.assertEqual(surface.expand_use_tree(""), [])

    def test_use_declarations_and_inline_paths_are_both_read(self) -> None:
        source = (
            "pub use raikiri_html::{Page,\n    layout};\n"
            "fn f() { raikiri_dom::Document::new(); my_raikiri_dom::Nope::x(); }\n"
            "use raikiri_dom::layout::*;\n"
            "use std::fmt;\n"
        )
        paths, globs = surface.used_paths(source)
        self.assertEqual(paths, {"raikiri_html::Page", "raikiri_html::layout", "raikiri_dom::Document::new"})
        self.assertEqual(globs, {"raikiri_dom::layout::*"})

    def test_a_path_resolves_to_the_longest_name_it_starts_with(self) -> None:
        names = {
            "raikiri_dom::Document": surface.Name("raikiri_dom::Document", "struct", False, None),
            "raikiri_dom::layout": surface.Name("raikiri_dom::layout", "module", False, None),
        }
        resolved = surface.resolve("raikiri_dom::Document::new", names)
        self.assertEqual(resolved.path, "raikiri_dom::Document")
        self.assertEqual(surface.resolve("raikiri_dom::layout::go", names).path, "raikiri_dom::layout")
        self.assertIsNone(surface.resolve("raikiri_dom::Missing", names))
        self.assertIsNone(surface.resolve("raikiri_dom", names))


class UsageTests(unittest.TestCase):
    def write(self, root: Path, relative: str, text: str) -> None:
        target = root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def test_the_report_separates_used_hidden_unknown_and_glob_names(self) -> None:
        surfaces = {
            "raikiri_dom": {
                "raikiri_dom::Document": surface.Name("raikiri_dom::Document", "struct", False, None),
                "raikiri_dom::secret": surface.Name("raikiri_dom::secret", "function", True, None),
            },
            "raikiri_html": {"raikiri_html::layout": surface.Name("raikiri_html::layout", "function", False, None)},
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write(root, "a/lib.rs", "use raikiri_dom::{Document, secret, gone};\nuse raikiri_html::layout;\n")
            self.write(root, "a/more.rs", "fn f() { raikiri_dom::Document::new(); }\nuse raikiri_dom::util::*;\n")
            self.write(root, "target/skipped.rs", "use raikiri_dom::Skipped;\n")
            self.write(root, "a/other.rs", "use serde::Serialize;\n")
            usage = surface.scan_usage(root, surfaces)
        self.assertEqual(usage.files, 3)
        self.assertEqual(sorted(usage.used), ["raikiri_dom::Document", "raikiri_html::layout"])
        self.assertEqual(sorted(usage.hidden), ["raikiri_dom::secret"])
        self.assertEqual(usage.unknown, {"raikiri_dom::gone"})
        self.assertEqual(usage.globs, {"raikiri_dom::util::*"})
        text = surface.format_usage(usage, surfaces)
        self.assertIn("raikiri_dom: 1 names used", text)
        self.assertIn("  struct     raikiri_dom::Document", text)
        self.assertIn("hidden names used", text)
        self.assertIn("match no name of the crate", text)
        self.assertIn("glob imports", text)

    def test_a_bare_crate_name_is_not_reported_as_an_unknown_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write(root, "lib.rs", "use raikiri_dom as dom;\nfn f() { dom::Missing::x(); }\n")
            usage = surface.scan_usage(root, {"raikiri_dom": {}})
        self.assertEqual(usage.unknown, {"raikiri_dom::Missing::x"})

    def test_a_file_that_is_not_utf8_is_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bad.rs").write_bytes(b"\xff\xfe use raikiri_dom::A;")
            usage = surface.scan_usage(root, {"raikiri_dom": {}})
        self.assertEqual(usage.files, 1)
        self.assertEqual(usage.used, {})


class MainTests(unittest.TestCase):
    def run_main(self, *argv: str) -> tuple[int, str]:
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = surface.main(list(argv))
        return code, out.getvalue()

    def test_main_lists_the_names_of_a_crate_and_reports_usage(self) -> None:
        doc = Doc()
        doc.set_items(0, [doc.function(1, "go", ["x", "go"])])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "raikiri_x.json").write_text(json.dumps(doc.json()), encoding="utf-8")
            code, text = self.run_main("--json-dir", str(root), "raikiri-x")
            self.assertEqual((code, text), (0, "function   x::go\n"))
            src = root / "src"
            src.mkdir()
            (src / "main.rs").write_text("use raikiri_x::go;\n", encoding="utf-8")
            # The crate is named `x` inside the document, so its names start with `x::`;
            # a real document names the crate `raikiri_x`.
            doc.index["0"]["name"] = "raikiri_x"
            doc.index["1"]["name"] = "go"
            (root / "raikiri_x.json").write_text(json.dumps(doc.json()), encoding="utf-8")
            code, text = self.run_main("--json-dir", str(root), "--usage", str(src), "raikiri-x")
        self.assertEqual(code, 0)
        self.assertIn("raikiri_x: 1 names used", text)
        self.assertIn("function   raikiri_x::go", text)




class ExhaustiveTests(unittest.TestCase):
    def found(self, doc: Doc) -> dict[str, str]:
        return {name.path: detail for name, detail in surface.exhaustive_types(doc.json())}

    def test_enums_and_all_public_structs_without_the_attribute_are_listed(self) -> None:
        doc = Doc()
        doc.set_items(
            0,
            [
                doc.enum(1, "Kind", 3),
                doc.plain_struct(2, "Options", 2),
                doc.tuple_struct(3, "Id", [101]),
            ],
        )
        self.assertEqual(
            self.found(doc), {"x::Kind": "3 variants", "x::Options": "2 public fields", "x::Id": "1 public fields"}
        )

    def test_the_attribute_is_found_as_a_bare_string_and_as_an_other_entry(self) -> None:
        doc = Doc()
        doc.set_items(
            0,
            [
                doc.enum(1, "Bare", 2, attrs=["non_exhaustive"]),
                doc.plain_struct(2, "Wrapped", 2, attrs=[{"other": "#[non_exhaustive]"}]),
                doc.plain_struct(3, "Other", 2, attrs=[{"other": "#[derive(Debug)]"}]),
            ],
        )
        self.assertEqual(self.found(doc), {"x::Other": "2 public fields"})

    def test_a_struct_a_user_cannot_build_or_destructure_completely_is_left_out(self) -> None:
        doc = Doc()
        doc.set_items(
            0,
            [
                doc.plain_struct(1, "PrivateField", 2, stripped=True),
                doc.plain_struct(2, "NoFields", 0),
                doc.tuple_struct(3, "Opaque", [None]),
                doc.tuple_struct(4, "Mixed", [101, None]),
                doc.struct(5, "Unit", ["x", "Unit"]),
            ],
        )
        self.assertEqual(self.found(doc), {})

    def test_hidden_types_and_types_of_other_crates_are_left_out(self) -> None:
        doc = Doc()
        hidden = doc.enum(1, "Hidden", 2, attrs=HIDDEN)
        foreign = doc.foreign(9, ["other", "Thing"], kind="enum")
        reexport = doc.use(2, "Thing", foreign)
        doc.set_items(0, [hidden, reexport])
        self.assertEqual(self.found(doc), {})

    def test_a_type_reachable_under_two_names_is_listed_once_under_the_shorter(self) -> None:
        doc = Doc()
        kind = doc.enum(2, "Kind", 3)
        module = doc.module(3, "m", [kind])
        alias = doc.use(1, "Kind", kind)
        doc.set_items(0, [alias, module])
        self.assertEqual(self.found(doc), {"x::Kind": "3 variants"})

    def test_format_prints_one_line_per_type(self) -> None:
        doc = Doc()
        doc.set_items(0, [doc.enum(1, "Kind", 3)])
        text = surface.format_exhaustive(surface.exhaustive_types(doc.json()))
        self.assertEqual(text, "enum       x::Kind  (3 variants)\n")
        self.assertEqual(surface.format_exhaustive([]), "")


if __name__ == "__main__":
    unittest.main()
