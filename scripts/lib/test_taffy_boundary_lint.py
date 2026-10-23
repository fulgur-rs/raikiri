#!/usr/bin/env python3
"""scripts/lib/test_taffy_boundary_lint.py — unit tests for taffy_boundary_lint.py.

The scanner tests feed small Rust snippets to `scan_source`; the ratchet tests
write a temporary `crates/*/src/*.rs` tree with a baseline and an exempt list
and drive `main()` end to end, because the failure the lint exists for (a file
growing past its pin, or a pin left behind) is a fact about the tree plus the
baseline together.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v
"""

from __future__ import annotations

import contextlib
import io
import re
import tempfile
import unittest
from pathlib import Path

from taffy_boundary_lint import (
    OUTPUT_NAMES,
    TokenizeError,
    main,
    scan_source,
    strip_comments_and_strings,
)

REPO_ROOT = Path(__file__).resolve().parents[2]


def _names(src: str) -> list[str]:
    return [o.name for o in scan_source("x.rs", src).counted]


class StripTests(unittest.TestCase):
    def test_comments_and_strings_are_blanked_and_lines_kept(self) -> None:
        src = 'let a = "taffy::Size"; // taffy::Layout\n/* taffy::Point\n */ let b = 1;\n'
        out = strip_comments_and_strings(src)
        self.assertEqual(len(out), len(src))
        self.assertEqual(out.count("\n"), src.count("\n"))
        self.assertNotIn("taffy", out)
        self.assertIn("let b = 1;", out)

    def test_nested_block_comment(self) -> None:
        self.assertNotIn("taffy", strip_comments_and_strings("/* a /* taffy::Size */ taffy::Point */ x"))

    def test_raw_and_byte_strings(self) -> None:
        for lit in ('r"taffy::Size"', 'r#"a "taffy::Size" b"#', 'b"taffy::Size"', 'br#"taffy::Size"#', 'c"taffy::Size"'):
            self.assertNotIn("taffy", strip_comments_and_strings(f"let a = {lit};"), lit)

    def test_escaped_quote_does_not_end_a_string(self) -> None:
        self.assertNotIn("taffy", strip_comments_and_strings('let a = "q\\" taffy::Size";'))

    def test_char_literals_and_lifetimes(self) -> None:
        out = strip_comments_and_strings("fn f<'a>(x: &'a str) { let q = '\\''; let r = 'x'; taffy::Size }")
        self.assertIn("<'a>", out)
        self.assertIn("&'a str", out)
        self.assertIn("taffy::Size", out)
        self.assertNotIn("'x'", out)
        self.assertNotIn("\\'", out)

    def test_raw_identifier_is_not_a_raw_string(self) -> None:
        self.assertIn("taffy::Size", strip_comments_and_strings("let r#type = 1; taffy::Size"))

    def test_unterminated_literals_are_errors(self) -> None:
        for bad in ('let a = "oops;', "/* never closed", 'r#"never closed"'):
            with self.assertRaises(TokenizeError, msg=bad):
                strip_comments_and_strings(bad)


class UseTreeTests(unittest.TestCase):
    def test_flat_and_nested_groups(self) -> None:
        src = "use taffy::{Size, tree::{LayoutInput, LayoutOutput}, style::AvailableSpace};\n"
        self.assertEqual(_names(src), ["Size", "LayoutInput", "LayoutOutput", "AvailableSpace"])

    def test_alias_counts_the_original_name(self) -> None:
        self.assertEqual(_names("use taffy::NodeId as TaffyNodeId;\n"), ["NodeId"])
        self.assertEqual(_names("use taffy::{Layout as TaffyLayout, Line as TaffyLine};\n"), ["Layout", "Line"])

    def test_input_side_names_are_not_counted(self) -> None:
        src = "use taffy::{CoreStyle, Dimension, Display, LengthPercentage, Style, compute_leaf_layout};\n"
        self.assertEqual(_names(src), [])

    def test_enum_variant_import_counts_once_per_leaf(self) -> None:
        self.assertEqual(_names("use taffy::AvailableSpace::{Definite, MaxContent};\n"), ["AvailableSpace"] * 2)

    def test_multi_line_import_is_counted_per_item_with_its_own_line(self) -> None:
        src = "use taffy::{\n    Size,\n    Dimension,\n    Point,\n};\n"
        scan = scan_source("x.rs", src)
        self.assertEqual([(o.name, o.line) for o in scan.counted], [("Size", 2), ("Point", 4)])

    def test_other_crates_with_the_same_names_are_not_counted(self) -> None:
        src = "use crate::taffy::Size;\nuse raikiri_layout_geometry::{Size, Point};\nuse std::collections::HashMap;\n"
        self.assertEqual(_names(src), [])

    def test_leading_double_colon_and_pub_use(self) -> None:
        self.assertEqual(_names("pub use ::taffy::Rect;\n"), ["Rect"])
        self.assertEqual(_names("pub(crate) use taffy::Layout;\n"), ["Layout"])

    def test_glob_import_of_a_taffy_module_is_counted(self) -> None:
        scan = scan_source("x.rs", "use taffy::tree::*;\n")
        self.assertEqual([(o.name, o.text) for o in scan.counted], [("*", "use taffy::tree::*")])

    def test_raw_identifier_in_an_import_is_read(self) -> None:
        self.assertEqual(_names("use taffy::{r#Line, Size};\n"), ["Line", "Size"])

    def test_enum_variant_with_an_output_type_name_is_not_a_type(self) -> None:
        # GridPlacement::Line is the input-side grid placement variant.
        self.assertEqual(_names("use taffy::GridPlacement::Line;\nlet p = taffy::GridPlacement::Line(1);\n"), [])
        self.assertEqual(_names("let q = taffy::style::GridPlacement::Line(2);\n"), [])

    def test_importing_the_crate_or_a_module_is_counted(self) -> None:
        for src in (
            "use taffy as t;\n",
            "use taffy::{self as t};\n",
            "use taffy::tree;\n",
            "use taffy::geometry::{self};\n",
            "use taffy::{style, Dimension};\n",
        ):
            names = _names(src)
            self.assertEqual(len(names), 1, src)
            self.assertEqual(names[0], "<module>", src)

    def test_importing_a_function_or_a_trait_is_not_a_module_import(self) -> None:
        self.assertEqual(_names("use taffy::{compute_leaf_layout, TraversePartialTree};\nuse taffy::prelude::TaffyAuto;\n"), [])

    def test_use_inside_a_function_body(self) -> None:
        self.assertEqual(_names("fn f() {\n    use taffy::geometry::Size;\n}\n"), ["Size"])

    def test_use_in_a_comment_or_string_is_ignored(self) -> None:
        self.assertEqual(_names('// use taffy::Size;\nlet s = "use taffy::Size;";\n'), [])

    def test_macro_fragment_use_that_is_not_taffy_is_skipped(self) -> None:
        self.assertEqual(_names("macro_rules! m { () => { use $crate::inner::Size; } }\n"), [])

    def test_precise_capturing_is_not_a_use_item(self) -> None:
        self.assertEqual(_names("fn f<'a>() -> impl Sized + use<'a> { taffy::Size::<f32>::ZERO }\n"), ["Size"])


class InlinePathTests(unittest.TestCase):
    def test_inline_paths(self) -> None:
        src = "let a: taffy::Size<f32> = taffy::Size::ZERO;\nlet b = taffy::NodeId::from(1usize);\n"
        self.assertEqual(_names(src), ["Size", "Size", "NodeId"])

    def test_generic_arguments_are_separate_occurrences(self) -> None:
        self.assertEqual(_names("let a: taffy::Size<taffy::Rect<f32>>;\n"), ["Size", "Rect"])

    def test_module_qualified_path(self) -> None:
        self.assertEqual(_names("fn f(i: taffy::tree::LayoutInput) -> taffy::tree::LayoutOutput {}\n"), ["LayoutInput", "LayoutOutput"])

    def test_algorithm_calls_and_traits_are_not_counted(self) -> None:
        src = "taffy::compute_root_layout(tree, id, space);\nimpl taffy::LayoutPartialTree for T {}\n"
        self.assertEqual(_names(src), [])

    def test_turbofish_stops_the_path(self) -> None:
        self.assertEqual(_names("let z = taffy::Size::<f32>::ZERO;\n"), ["Size"])

    def test_nested_crate_paths_are_not_taffy_paths(self) -> None:
        self.assertEqual(_names("let a = crate::taffy::Size::ZERO;\nlet b = other::taffy::Layout::default();\n"), [])

    def test_comments_and_strings_are_not_counted(self) -> None:
        src = '/// Built from `taffy::Layout`.\nlet s = "taffy::Size";\n'
        self.assertEqual(_names(src), [])

    def test_raw_identifier_segment(self) -> None:
        self.assertEqual(_names("let a: taffy::r#Size<f32>;\n"), ["Size"])

    def test_word_boundary(self) -> None:
        self.assertEqual(_names("let a = not_taffy::Size::ZERO;\nlet b = taffy_impl::Size::ZERO;\n"), [])

    def test_inline_path_inside_use_statement_is_not_double_counted(self) -> None:
        self.assertEqual(_names("use taffy::Size;\nlet a: Size<f32>;\n"), ["Size"])


class IgnoreMarkerTests(unittest.TestCase):
    def test_marker_with_reason_excludes_the_occurrence(self) -> None:
        src = "use taffy::Size as StyleSize; // taffy-boundary-lint:ignore: Size<Dimension> is style input\nuse taffy::Point;\n"
        scan = scan_source("x.rs", src)
        self.assertEqual([o.name for o in scan.counted], ["Point"])
        self.assertEqual([o.name for o in scan.ignored], ["Size"])

    def test_marker_without_reason_is_not_honoured_and_is_reported(self) -> None:
        scan = scan_source("x.rs", "use taffy::Size; // taffy-boundary-lint:ignore:\n")
        self.assertEqual([o.name for o in scan.counted], ["Size"])
        self.assertEqual(scan.unreasoned_markers, (1,))

    def test_marker_applies_to_its_own_line_only(self) -> None:
        src = "use taffy::{\n    Size, // taffy-boundary-lint:ignore: style input\n    Point,\n};\n"
        scan = scan_source("x.rs", src)
        self.assertEqual([o.name for o in scan.counted], ["Point"])
        self.assertEqual([o.name for o in scan.ignored], ["Size"])

    def test_marker_on_the_previous_line_is_not_honoured(self) -> None:
        src = "// taffy-boundary-lint:ignore: not on the same line\nuse taffy::Size;\n"
        self.assertEqual(_names(src), ["Size"])


def _write(root: Path, rel: str, content: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def _run(root: Path, *extra: str) -> tuple[int, str]:
    out = io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
        code = main(
            [
                "--repo-root",
                str(root),
                "--baseline-file",
                str(root / "baseline.txt"),
                "--exempt-file",
                str(root / "exempt.txt"),
                *extra,
            ]
        )
    return code, out.getvalue()


class RatchetTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        _write(self.root, "exempt.txt", "# exempt\ncrates/view/  # the view crate\ncrates/dom/src/protocol.rs\n")
        _write(self.root, "crates/dom/src/a.rs", "use taffy::{Size, Point};\nfn f(_: taffy::Layout) {}\n")
        _write(self.root, "crates/dom/src/b.rs", "use taffy::Dimension;\n")
        _write(self.root, "crates/dom/src/protocol.rs", "pub use taffy::{LayoutInput, LayoutOutput};\n")
        _write(self.root, "crates/view/src/lib.rs", "use taffy::{Size, Layout};\n")
        _write(self.root, "baseline.txt", "# pins\n3 crates/dom/src/a.rs\n")

    def test_matching_baseline_passes_and_exempt_files_are_not_counted(self) -> None:
        code, out = _run(self.root)
        self.assertEqual(code, 0, out)
        self.assertIn("PASS", out)

    def test_new_occurrence_in_a_pinned_file_fails(self) -> None:
        _write(self.root, "crates/dom/src/a.rs", "use taffy::{Size, Point};\nfn f(_: taffy::Layout, _: taffy::NodeId) {}\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("crates/dom/src/a.rs: 4 occurrence(s), baseline 3 (+1)", out)
        self.assertIn("2: taffy::Layout  -> raikiri_layout_geometry::BoxLayout", out)

    def test_new_file_without_an_entry_fails(self) -> None:
        _write(self.root, "crates/dom/src/c.rs", "use taffy::Rect;\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("crates/dom/src/c.rs: 1 occurrence(s), no baseline entry (+1)", out)
        self.assertIn("raikiri_layout_geometry::Edges", out)

    def test_removed_occurrence_without_lowering_the_baseline_fails(self) -> None:
        _write(self.root, "crates/dom/src/a.rs", "use taffy::Size;\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("baseline 3 is stale", out)

    def test_baseline_entry_without_occurrences_fails(self) -> None:
        _write(self.root, "crates/dom/src/a.rs", "use raikiri_layout_geometry::Size;\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("no counted occurrences left", out)

    def test_baseline_entry_for_an_exempt_file_fails(self) -> None:
        _write(self.root, "baseline.txt", "3 crates/dom/src/a.rs\n2 crates/dom/src/protocol.rs\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("crates/dom/src/protocol.rs", out)

    def test_ignore_marker_keeps_the_file_at_its_pin(self) -> None:
        _write(
            self.root,
            "crates/dom/src/a.rs",
            "use taffy::{Size, Point}; // taffy-boundary-lint:ignore: style input\nfn f(_: taffy::Layout) {}\n",
        )
        _write(self.root, "baseline.txt", "1 crates/dom/src/a.rs\n")
        code, out = _run(self.root)
        self.assertEqual(code, 0, out)
        self.assertIn("ignored by marker        : 2", out)

    def test_failure_lists_at_most_a_dozen_occurrences_per_file(self) -> None:
        body = "".join(f"let a{i}: taffy::Size<f32>;\n" for i in range(20))
        _write(self.root, "crates/dom/src/c.rs", body)
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("... 8 more", out)

    def test_print_counts_does_not_need_a_baseline(self) -> None:
        (self.root / "baseline.txt").unlink()
        code, out = _run(self.root, "--print-counts")
        self.assertEqual(code, 0, out)
        self.assertEqual(out.strip(), "3 crates/dom/src/a.rs")

    def test_update_baseline_lowers_and_drops_zero_entries(self) -> None:
        _write(self.root, "crates/dom/src/a.rs", "use taffy::Size;\n")
        _write(self.root, "crates/dom/src/c.rs", "use raikiri_layout_geometry::Size;\n")
        code, out = _run(self.root, "--update-baseline")
        self.assertEqual(code, 0, out)
        self.assertEqual((self.root / "baseline.txt").read_text(), "# pins\n\n1 crates/dom/src/a.rs\n")
        code, out = _run(self.root)
        self.assertEqual(code, 0, out)

    def test_update_baseline_refuses_when_one_file_rose_even_if_the_total_fell(self) -> None:
        _write(self.root, "crates/dom/src/a.rs", "use raikiri_layout_geometry::Size;\n")
        _write(self.root, "crates/dom/src/moved.rs", "use taffy::{Size, Point};\n")
        before = (self.root / "baseline.txt").read_text()
        code, out = _run(self.root, "--update-baseline")
        self.assertEqual(code, 1, out)
        self.assertIn("crates/dom/src/moved.rs: 2 (pinned 0)", out)
        self.assertEqual((self.root / "baseline.txt").read_text(), before)

    def test_update_baseline_refuses_to_raise_the_total(self) -> None:
        _write(self.root, "crates/dom/src/c.rs", "use taffy::Rect;\n")
        before = (self.root / "baseline.txt").read_text()
        code, out = _run(self.root, "--update-baseline")
        self.assertEqual(code, 1, out)
        self.assertIn("refusing to raise", out)
        self.assertEqual((self.root / "baseline.txt").read_text(), before)

    def test_unreadable_baseline_is_a_tooling_error(self) -> None:
        _write(self.root, "baseline.txt", "three crates/dom/src/a.rs\n")
        code, out = _run(self.root)
        self.assertEqual(code, 2, out)

    def test_zero_count_baseline_line_is_a_tooling_error(self) -> None:
        _write(self.root, "baseline.txt", "0 crates/dom/src/a.rs\n")
        code, out = _run(self.root)
        self.assertEqual(code, 2, out)

    def test_source_that_is_not_utf8_is_a_tooling_error(self) -> None:
        path = self.root / "crates/dom/src/c.rs"
        path.write_bytes(b"// \xff\xfe\nuse taffy::Size;\n")
        code, out = _run(self.root)
        self.assertEqual(code, 2, out)
        self.assertIn("crates/dom/src/c.rs", out)

    def test_unscannable_source_is_a_tooling_error(self) -> None:
        _write(self.root, "crates/dom/src/c.rs", 'let a = "oops;\n')
        code, out = _run(self.root)
        self.assertEqual(code, 2, out)
        self.assertIn("crates/dom/src/c.rs", out)

    def test_a_crates_build_output_is_not_scanned(self) -> None:
        _write(self.root, "crates/dom/target/debug/build/x/out.rs", "use taffy::Size;\n")
        code, out = _run(self.root)
        self.assertEqual(code, 0, out)

    def test_a_module_directory_named_target_is_scanned(self) -> None:
        _write(self.root, "crates/dom/src/layout/target/mod.rs", "use taffy::Rect;\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("crates/dom/src/layout/target/mod.rs", out)

    def test_hint_inside_raikiri_dom_names_the_protocol_module(self) -> None:
        _write(self.root, "crates/raikiri-dom/src/c.rs", "use taffy::{NodeId, Layout};\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("-> crate::layout::protocol::NodeId", out)

    def test_hint_outside_raikiri_dom_does_not_name_the_crate_private_module(self) -> None:
        _write(self.root, "crates/other/src/lib.rs", "use taffy::{NodeId, Layout};\n")
        code, out = _run(self.root)
        self.assertEqual(code, 1, out)
        self.assertIn("no owned name outside raikiri-dom yet", out)
        self.assertIn("raikiri_layout_geometry::BoxLayout", out)


class NameTableTests(unittest.TestCase):
    def _reexports(self, rel: str, pattern: str) -> dict[str, str]:
        """Exported name -> Taffy source name from a `use taffy::{...}` re-export list."""
        text = (REPO_ROOT / rel).read_text(encoding="utf-8")
        body = re.search(pattern, text, re.S)
        self.assertIsNotNone(body, f"{rel}: re-export list not found")
        out = {}
        for item in body.group(1).split(","):
            item = item.strip()
            if not item:
                continue
            source, _, alias = item.partition(" as ")
            out[alias.strip() or source.strip()] = source.strip()
        return out

    def test_owned_names_match_the_real_reexports(self) -> None:
        geometry = self._reexports("crates/raikiri-layout-geometry/src/lib.rs", r"pub use taffy::\{(.*?)\};")
        protocol = self._reexports("crates/raikiri-dom/src/layout/protocol.rs", r"pub\(crate\) use taffy::\{(.*?)\};")
        owned = {}
        for name, hint in OUTPUT_NAMES.items():
            if hint.startswith("raikiri_layout_geometry::"):
                owned[hint.removeprefix("raikiri_layout_geometry::")] = name
                self.assertIn(hint.removeprefix("raikiri_layout_geometry::"), geometry, name)
                self.assertEqual(geometry[hint.removeprefix("raikiri_layout_geometry::")], name)
            elif hint.startswith("crate::layout::protocol::"):
                self.assertEqual(hint.removeprefix("crate::layout::protocol::"), name)
                self.assertEqual(protocol.get(name), name, f"{name} is not re-exported by the protocol module")
        # Every re-export has a hint, so nothing is exported without being counted.
        for exported, source in geometry.items():
            self.assertEqual(owned.get(exported), source, f"geometry crate exports {exported} but the lint does not count {source}")
        for exported in protocol:
            self.assertEqual(OUTPUT_NAMES.get(exported), f"crate::layout::protocol::{exported}")

    def test_every_counted_name_has_a_replacement_hint(self) -> None:
        for name, hint in OUTPUT_NAMES.items():
            self.assertTrue(hint, name)

    def test_rect_and_layout_move_to_the_edges_and_box_layout_names(self) -> None:
        self.assertTrue(OUTPUT_NAMES["Rect"].endswith("::Edges"))
        self.assertTrue(OUTPUT_NAMES["Layout"].endswith("::BoxLayout"))


if __name__ == "__main__":
    unittest.main()
