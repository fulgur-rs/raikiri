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

    def enum_with(self, key: int, name: str, variants: list[int], path: list[str] | None = None) -> int:
        return self._add(key, name, {"enum": {"variants": variants, "impls": []}}, None, path)

    def variant(self, key: int, name: str, attrs: list | None = None) -> int:
        return self._add(key, name, {"variant": {"kind": "plain", "discriminant": None}}, attrs, None)

    def extern_crate(self, key: int, crate: str, as_name: str | None = None, visibility: str = "public") -> int:
        # rustdoc 1.95 writes the name it is published under to `name` and the crate to `rename`.
        body = {"name": as_name, "rename": crate} if as_name else {"name": crate, "rename": None}
        self._add(key, body["name"], {"extern_crate": body}, None, None)
        self.index[str(key)]["visibility"] = visibility
        return key

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

    def test_a_module_and_a_function_of_the_same_name_are_both_listed(self) -> None:
        doc = Doc()
        module = doc.module(1, "cascade", [])
        function = doc.function(2, "cascade", ["x", "cascade"])
        doc.set_items(0, [module, function])
        self.assertEqual(
            [(name.path, name.kind) for name in surface.names(doc.json())],
            [("x::cascade", "function"), ("x::cascade", "module")],
        )

    def test_globs_of_two_foreign_modules_under_one_prefix_are_both_listed(self) -> None:
        doc = Doc()
        first = doc.foreign(8, ["alpha", "api"], kind="module")
        second = doc.foreign(9, ["beta", "api"], kind="module")
        module = doc.module(
            2, "computed", [doc.use(3, "api", first, glob=True), doc.use(4, "api", second, glob=True)]
        )
        doc.set_items(0, [module])
        globs = [name.canonical for name in surface.names(doc.json()) if name.kind == "glob"]
        self.assertEqual(globs, ["alpha::api", "beta::api"])

    def test_a_glob_of_a_local_enum_publishes_its_variants_at_the_use_site(self) -> None:
        doc = Doc()
        enum = doc.enum_with(2, "E", [3, 4], ["x", "private", "E"])
        doc.variant(3, "A")
        doc.variant(4, "B", attrs=HIDDEN)
        doc.set_items(0, [doc.use(1, "E", enum, glob=True)])
        found = listed(doc)
        self.assertEqual(sorted(found), ["x::A", "x::B"])
        self.assertEqual((found["x::A"].kind, found["x::A"].item_id, found["x::A"].hidden), ("variant", 3, False))
        self.assertTrue(found["x::B"].hidden)

    def test_a_variant_and_a_module_of_the_same_name_are_both_listed(self) -> None:
        doc = Doc()
        enum = doc.enum_with(2, "E", [3], ["x", "private", "E"])
        doc.variant(3, "V")
        doc.set_items(0, [doc.use(1, "E", enum, glob=True), doc.module(4, "V", [])])
        self.assertEqual(
            [(name.path, name.kind) for name in surface.names(doc.json())],
            [("x::V", "module"), ("x::V", "variant")],
        )

    def test_a_doc_attribute_with_more_options_can_still_hide(self) -> None:
        def hidden(*attrs: str) -> bool:
            return surface.is_hidden({"attrs": [{"other": attr} for attr in attrs]})

        self.assertTrue(hidden("#[doc(hidden)]"))
        self.assertTrue(hidden('#[doc(hidden, alias = "secret")]'))
        self.assertTrue(hidden('#[doc(alias = "secret", hidden)]'))
        self.assertTrue(hidden("#[inline]", "#[doc(hidden)]"))
        self.assertFalse(hidden('#[doc(alias = "hidden")]'))
        self.assertFalse(hidden('#[doc(alias = "a, hidden, b")]'))
        self.assertFalse(hidden("#[doc(hidden_thing)]"))
        self.assertFalse(hidden("#[must_use]"))
        self.assertFalse(surface.is_hidden({"attrs": ["non_exhaustive"]}))

    def test_a_public_extern_crate_publishes_the_crate_under_its_local_name(self) -> None:
        doc = Doc()
        doc.set_items(0, [doc.extern_crate(1, "dep"), doc.extern_crate(2, "other", as_name="renamed")])
        found = listed(doc)
        self.assertEqual(sorted(found), ["x::dep", "x::renamed"])
        self.assertEqual((found["x::dep"].kind, found["x::dep"].canonical), ("extern_crate", "dep"))
        self.assertEqual(found["x::renamed"].canonical, "other")

    def test_a_private_extern_crate_publishes_nothing(self) -> None:
        doc = Doc()
        doc.set_items(0, [doc.extern_crate(1, "dep", visibility="default")])
        self.assertEqual(listed(doc), {})

    def test_the_same_name_reached_twice_is_listed_once(self) -> None:
        doc = Doc()
        item = doc.struct(4, "Same", ["x", "p", "Same"])
        doc.set_items(0, [doc.use(1, "Same", item), doc.use(2, "Same", item)])
        self.assertEqual([name.path for name in surface.names(doc.json())], ["x::Same"])

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


class Crate(Doc):
    """A crate named `name` whose root module is `name`, with crates of `externals` known by id."""

    def __init__(self, name: str, externals: dict[int, str] | None = None) -> None:
        super().__init__()
        self.name = name
        self.index["0"]["name"] = name
        self.externals = externals or {}

    def item(self, key: int, name: str, refs: list[int] | None = None, attrs: list | None = None) -> int:
        """A struct at `name::<name>` whose signature mentions the items `refs`."""
        inner = {"struct": {"kind": "unit", "impls": []}, "mentions": [{"resolved_path": {"id": r}} for r in refs or []]}
        return self._add(key, name, inner, attrs, [self.name, name])

    def hidden_impl(self, key: int, refs: list[int]) -> int:
        inner = {"impl": {"items": [], "mentions": [{"resolved_path": {"id": r}} for r in refs]}}
        return self._add(key, None, inner, HIDDEN, None)

    def has_impls(self, key: int, impls: list[int]) -> None:
        self.index[str(key)]["inner"]["struct"]["impls"] = impls

    def other(self, key: int, crate_id: int, path: list[str]) -> int:
        self.paths[str(key)] = {"crate_id": crate_id, "path": path, "kind": "struct"}
        return key

    def json(self) -> dict:
        return {
            "index": self.index,
            "paths": self.paths,
            "root": 0,
            "external_crates": {str(key): {"name": name} for key, name in self.externals.items()},
        }


class FirstTierTests(unittest.TestCase):
    def two_crates(self) -> tuple[dict[str, dict], Crate, Crate]:
        # `app` is the consumer. Its `Page` mentions `lib::Shared` in a signature.
        library = Crate("lib")
        shared = library.item(1, "Shared", refs=[2])
        nested = library.item(2, "Nested")
        unused = library.item(3, "Unused")
        direct = library.item(4, "Direct")
        library.set_items(0, [shared, nested, unused, direct])
        app = Crate("app", {1: "lib"})
        page = app.item(1, "Page", refs=[app.other(10, 1, ["lib", "Shared"])])
        app.set_items(0, [page])
        return {"app": app.json(), "lib": library.json()}, app, library

    def outside(self, docs: dict[str, dict], extra: list[str] | None = None) -> dict[str, list[str]]:
        found = surface.outside_first_tier(docs, "app", extra or [])
        return {crate: [name.path for name in names] for crate, names in found.items()}

    def test_what_a_signature_names_in_another_crate_is_in_the_tier_with_what_that_names(self) -> None:
        docs, _, _ = self.two_crates()
        self.assertEqual(self.outside(docs), {"lib": ["lib::Direct", "lib::Unused"]})

    def test_a_listed_name_brings_its_own_signature_with_it(self) -> None:
        docs, _, _ = self.two_crates()
        self.assertEqual(self.outside(docs, ["lib::Direct"]), {"lib": ["lib::Unused"]})

    def test_a_hidden_impl_does_not_make_what_it_names_part_of_the_tier(self) -> None:
        docs, app, library = self.two_crates()
        # `Unused` is named only by a hidden impl of `Shared`.
        library.has_impls(1, [20])
        library.hidden_impl(20, [3])
        docs["lib"] = library.json()
        self.assertEqual(self.outside(docs), {"lib": ["lib::Direct", "lib::Unused"]})

    def test_an_impl_that_is_not_hidden_does(self) -> None:
        docs, app, library = self.two_crates()
        library.has_impls(1, [20])
        library._add(20, None, {"impl": {"items": [], "mentions": [{"resolved_path": {"id": 3}}]}}, None, None)
        docs["lib"] = library.json()
        self.assertEqual(self.outside(docs), {"lib": ["lib::Direct"]})

    def test_hidden_names_are_not_reported(self) -> None:
        docs, app, library = self.two_crates()
        library.index["3"]["attrs"] = HIDDEN
        library.index["4"]["attrs"] = HIDDEN
        docs["lib"] = library.json()
        self.assertEqual(self.outside(docs), {})

    def test_a_module_is_outside_when_nothing_under_it_is_in_the_tier(self) -> None:
        docs, app, library = self.two_crates()
        inner = library.item(5, "Inner")
        library.set_items(0, [1, 2, 3, 4, library.module(6, "m", [inner]), library.module(7, "kept", [2])])
        docs["lib"] = library.json()
        found = self.outside(docs)
        self.assertEqual(found, {"lib": ["lib::Direct", "lib::Unused", "lib::m", "lib::m::Inner"]})

    def test_the_names_between_the_markers_are_read_one_per_line(self) -> None:
        text = (
            "intro lib::NotThis\n"
            f"{surface.FIRST_TIER_BEGIN}\n```text\nlib::A\nlib::m::B\n\n  lib::C  \n```\n"
            f"{surface.FIRST_TIER_END}\nlib::AfterIt\nnot a path\n"
        )
        self.assertEqual(surface.read_first_tier_names(text), ["lib::A", "lib::m::B", "lib::C"])

    def test_a_marker_quoted_in_prose_does_not_cut_the_list(self) -> None:
        text = (
            f"Write `{surface.FIRST_TIER_END}` after the list.\n"
            f"{surface.FIRST_TIER_BEGIN}\n```text\nlib::A\n```\n{surface.FIRST_TIER_END}\n"
        )
        self.assertEqual(surface.read_first_tier_names(text), ["lib::A"])

    def test_the_value_types_are_read_from_their_own_markers(self) -> None:
        text = (
            f"{surface.FIRST_TIER_BEGIN}\n```text\nlib::A\n```\n{surface.FIRST_TIER_END}\n"
            f"{surface.VALUE_TYPES_BEGIN}\n```text\nlib::Id\nlib::m::Color\n```\n{surface.VALUE_TYPES_END}\n"
        )
        self.assertEqual(surface.read_value_types(text), ["lib::Id", "lib::m::Color"])
        self.assertEqual(surface.read_first_tier_names(text), ["lib::A"])

    def test_a_value_type_line_that_is_not_a_path_is_refused_with_its_own_name_for_the_list(self) -> None:
        text = f"{surface.VALUE_TYPES_BEGIN}\nlib::A,\n{surface.VALUE_TYPES_END}\n"
        with self.assertRaisesRegex(ValueError, "value-type markers"):
            surface.read_value_types(text)

    def test_a_line_between_the_markers_that_is_not_a_path_is_refused(self) -> None:
        text = f"{surface.FIRST_TIER_BEGIN}\n```text\nlib::A\nlib::B,\n```\n{surface.FIRST_TIER_END}\n"
        with self.assertRaisesRegex(ValueError, "lib::B,"):
            surface.read_first_tier_names(text)

    def test_a_module_of_another_crate_brings_in_what_it_re_exports_from_elsewhere(self) -> None:
        # `lib::m::Thing` is declared in a private module, so its canonical path is `lib::inner::Thing`.
        library = Crate("lib")
        thing = library.item(1, "Thing")
        library.paths["1"]["path"] = ["lib", "inner", "Thing"]
        module = library.module(3, "m", [library.use(2, "Thing", thing)])
        library.set_items(0, [module])
        app = Crate("app", {1: "lib"})
        reexport = app.other(10, 1, ["lib", "m"])
        app.paths["10"]["kind"] = "module"
        app.set_items(0, [app.use(2, "m", reexport)])
        docs = {"app": app.json(), "lib": library.json()}
        self.assertEqual(self.outside(docs), {})

    def test_a_link_to_a_hidden_item_in_the_docs_of_a_visible_one_is_reported(self) -> None:
        library = Crate("lib")
        shown = library.item(1, "Shown")
        helper = library.item(2, "Helper", attrs=HIDDEN)
        library.index["1"]["links"] = {"`Helper`": helper, "`Shown`": shown}
        library.set_items(0, [shown, helper])
        self.assertEqual(
            surface.hidden_doc_links({"lib": library.json()}),
            [("lib", "Shown", "`Helper`", "lib::Helper")],
        )

    def test_links_among_visible_items_and_from_hidden_ones_are_not_reported(self) -> None:
        library = Crate("lib")
        shown = library.item(1, "Shown")
        other = library.item(2, "Other")
        helper = library.item(3, "Helper", attrs=HIDDEN)
        inner = library.item(4, "Inner", attrs=HIDDEN)
        library.index["1"]["links"] = {"`Other`": other}
        library.index["3"]["links"] = {"`Inner`": inner}
        library.set_items(0, [shown, other, helper, inner])
        self.assertEqual(surface.hidden_doc_links({"lib": library.json()}), [])

    def test_a_link_to_a_member_of_a_hidden_type_is_reported(self) -> None:
        library = Crate("lib")
        shown = library.item(1, "Shown")
        hidden = library.item(2, "Hidden", attrs=HIDDEN)
        method = library._add(5, "run", {"function": {}}, None, None)
        library.index["2"]["inner"]["struct"]["impls"] = [6]
        library._add(6, None, {"impl": {"items": [method]}}, None, None)
        library.index["1"]["links"] = {"`Hidden::run`": method}
        library.set_items(0, [shown, hidden])
        self.assertEqual(
            surface.hidden_doc_links({"lib": library.json()}),
            [("lib", "Shown", "`Hidden::run`", "lib::run")],
        )

    def test_a_document_without_the_markers_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            surface.read_first_tier_names("no markers here")


if __name__ == "__main__":
    unittest.main()
