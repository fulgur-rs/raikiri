# raikiri-layout-diff

A differential layout checker. It lays out the same inputs with two builds of
the layout engine, typically the merge-base of a pull request and its head,
and reports every input whose result differs, down to the bits of every `f32`.

The WPT conformance run says whether a test still passes. It cannot say that a
box did not move by a hundredth of a pixel, or that a text run kept its
origin, because a layout that is slightly off passes the same tests. This tool
is for changes that claim to preserve layout: a refactoring, a performance
change, or a change of how the style is read. Such a change must report no
difference. A change that is meant to move boxes shows which boxes it moved.

It is the layout counterpart of `raikiri-cascade-diff`, with the same shape:
a tool that dumps one line per case, and a script that builds the tool against
both trees and compares the dumps. It is not part of `scripts/gate.sh`; a run
takes minutes.

## Using it in a pull request

```sh
scripts/layout-diff.sh --wpt-baseline
```

Run it from the checkout of the branch. It compares the branch against the
merge-base with `origin/main` on

- 2,000 generated documents (`--seeds`), which mix out-of-flow boxes,
  vertical writing modes, multi-column containers, tables, flex and grid
  containers, floats, overflow, percentages and page cuts, and
- the WPT tests of `expectations/raikiri-baseline.txt` and
  `expectations/expected-failures.txt` (`--wpt-baseline`), about 4,700 files of
  the pinned WPT checkout (run `scripts/wpt/fetch.sh` once to get it).

The run ends with `PASS: every case has the same canonical layout output` and
exit status 0 when nothing differs. It exits with 1 and lists the differing
cases when something does, and with 2 when the run itself failed: a usage
error, a build that crashed or hung on a case (the case is named), or a
checkout that is missing.

Options:

| Option | Meaning |
| --- | --- |
| `--base <ref>` | The ref whose merge-base with `HEAD` is the baseline (default `origin/main`). |
| `--seeds <n>` | Generated cases use seeds `0..n` (default 2000). `--seeds 0` runs none; then a WPT option is needed. |
| `--wpt-baseline` | Add the baseline and expected-failure tests as cases. |
| `--wpt <dir>` | Add every `.html` file under a directory of the WPT checkout, for example `css/css-flexbox`. Repeatable. |
| `--font <file>` | The font the cases are laid out in, registered as `Ahem`. The default is the Ahem copy of `raikiri-dom`'s test data. |
| `--show <n>` | Print a field-level diff of the first `n` differing cases (default 3). |
| `--timeout <s>` | Stop a build that has not run every case after this many seconds (default 3600). |
| `--no-cache` | Build the merge-base again even when a cached build exists. |

A run of 2,000 generated documents and the WPT baseline (6,740 cases) takes
about two and a half minutes on four cores once the merge-base build is cached;
the first run also builds both trees, which takes longer.

The merge-base build is cached under `~/.cache/raikiri/layout-diff` (override
with `RAIKIRI_LAYOUT_DIFF_CACHE`), so the second run against one base does not
build it again. The two builds run side by side, one process each.

Files of the pinned WPT checkout that the baseline names but the tool cannot
read as the runner does are left out: tests with several variants
(`test.html?variant`) count once, and `.xhtml` tests are skipped because they
are XML documents, which the runner parses with the XML parser and this tool,
which lays out every document through the HTML parser, would misread.

## What is compared

Every case is laid out on one 800 by 600 CSS px page, with print media and the
rules of `@page` the document brings, and the fonts the script passes, so the
result does not depend on the machine. The tool writes a text from the result
and hashes it. The text has two parts, described in `src/dump.rs`:

- the **page API** that a consumer such as fulgur reads, for every page: the
  geometry, the root box, the fragments with their rectangles, the positioned
  glyph runs (origin, advance, ascent, descent, glyphs, decorations, shadows),
  the paint order, the page-margin boxes and the links;
- the **layout of every node** of each laid-out document: `order`, `location`,
  `size`, `scrollable_overflow_rect`, `scrollbar_size`, and the `border`,
  `padding` and `margin` sizes. Pages laid out again after a change of the
  content width have a document of their own.

Numbers are written as the shortest decimal text that reads back as the same
`f32`, so two numbers print the same exactly when their bits are the same.
A change of one ulp, or of the sign of a zero, shows up.

## Reading a report

```
cases      : 6740 (6740 with layout output from both builds)
differing  : 3
new panics : 0
panics fixed by head: 0
panics in head      : 0
errors in head      : 0

first differing cases:
gen:41
gen:977
html:/root/.cache/raikiri/wpt/css/css-grid/...

== diff for gen:41 (base → head) ==
-n12 Element tag=div text="" parent=4 order=0 location=[8.0 20.0] size=[90.0 36.0] ...
+n12 Element tag=div text="" parent=4 order=0 location=[8.0 20.0] size=[90.0 36.001] ...
```

- A case is `gen:SEED` or `html:PATH`. `scripts/layout-diff.sh` shows the first
  few differing cases; to see another one, use the tool directly.
- `raikiri-layout-diff show CASE` prints the full text of a case, and
  `raikiri-layout-diff describe SEED` prints the HTML a seed generates. Both
  take the same `--font` and `--html-root` as `dump`. Build the tool by hand
  with `cargo build --release -p raikiri-layout-diff` to run it on either
  tree.
- A `new panic` is a case that panicked in the head and not in the base. Both
  builds run with debug assertions and overflow checks, so a broken invariant
  shows as a panic. A panic is part of the case's result and is compared like
  any other.
- To turn a difference into a regression test, take the smallest document
  that shows it: `describe` gives the HTML of a generated case.

### Which differences are fine

A pull request that is meant to change layout reports differences. Read them
as the review of that change: every reported case should be one the change is
meant to move, and the fields that differ should be the ones it is meant to
move. A hash that differs only by the order of lines would be a bug of the
tool, not of the change; the dump is repeatable and independent of the order
of the cases, which `src/tests.rs` checks.

## Checking the tool

A comparison that reports nothing is only worth something if the tool does
report a difference when there is one. To check it:

1. **No change.** Run `scripts/layout-diff.sh --wpt-baseline` on a branch whose
   code is the merge-base's. It must report `differing  : 0`.
2. **A change of a thousandth of a pixel.** Make the head differ from the
   merge-base by a size that no test would notice, for example by adding this
   line to `layout_single_page_with_table_projection` in
   `crates/raikiri-dom/src/layout/page_pipeline.rs`, right after the call to
   `document.finalize_oof_layout()`:

   ```rust
   document.nodes[body_id].unrounded_layout.size.width += 0.001;
   ```

   and run the script again. Every case now differs: the field diff shows the
   `<body>` fragment as `rect=[0.0 0.0 800.0 124.0]` in the base and
   `rect=[0.0 0.0 800.001 124.0]` in the head, and the same width in the node
   listing. The run ends with `FAIL` and exit status 1. Remove the line again.

## When the layout API changes

The tool is built from one copy of its sources against both trees, so it can
only use what both trees have. Reading the laid-out box of a node is the one
place that depends on how a tree stores it, and it is in
`src/node_layout.rs`. If a pull request changes how nodes keep their layout,
that file is where to say how to read it in the new tree and in the old one;
`scripts/layout-diff.sh` runs the same file on both.

The manifest the script builds with (`tool_manifest` in
`scripts/layout-diff.sh`) lists the same dependencies as this crate's
`Cargo.toml`. Keep the two in step when adding one.

## Limits

- Cases run in one process each build, one after another. A case that does not
  end holds the run up until `--timeout`, and is named.
- A case that panics is caught and recorded, and the cases after it in the
  same process run on whatever state the panic left behind. A new panic can
  therefore show as more than one difference; look at the first one in the list.
- Only print layout is compared, on one page size. Screen layout is not.
- Files that read other files do so under `--html-root`, which the script sets
  to the WPT checkout; a generated case reads none.
- The fonts of a case are the one font `--font` names and what the document
  declares itself. Documents that rely on other installed fonts lay out in
  that font here.
