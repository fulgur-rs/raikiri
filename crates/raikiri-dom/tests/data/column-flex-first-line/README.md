# Column flex first-piece print measurements

`chromium.json` records 57 cases, 146 printed pages, 427 colored border boxes,
and 255 character origins from Chromium 152.0.7977.82 on 2026-11-05.
Each `body` is independent. The page is 200×200 CSS pixels, with zero margin;
Ahem is 16px with a 50px line height unless a case overrides it.

To reproduce a case, place its `body` after:

```html
<!doctype html><style>
@page { size: 200px 200px; margin: 0 }
@font-face { font-family: Ahem; src: url("file:///absolute/path/to/text-autospace/Ahem.ttf") }
</style>
```

Print with:

```sh
TMPDIR="$HOME/tmp" chromium --headless --no-sandbox --disable-gpu \
  --no-first-run --allow-file-access-from-files --no-pdf-header-footer \
  --print-to-pdf=case.pdf file:///absolute/path/to/case.html
```

Confirm every PDF page is 150×150 points. Read filled drawing rectangles with
PyMuPDF, intersect them with the page rectangle, and convert points to CSS
pixels by dividing by 0.75. The union of shapes with each identifier's color
is its visible border box: `f` is `#333333`, `tail` is `#cc0000`, and `i1`
through `i5` use `#11aabb` through `#55aabb`. Borders use their box's color.
Text origins come from each character's `origin` in `page.get_text("rawdict")`,
with the same unit conversion. The native comparison allows 0.02 CSS pixels
for PDF coordinate quantization and checks every recorded box and character.

The original four-item case leaves six pixels at the first page's foot.
Before the fix the last item's box occupied those six pixels, with only
44 pixels on the second page. Chromium moves that item's first piece and box
to the second page. Five-item and block controls distinguish the existing
non-last and block paths. Further cases cover first-piece fit, fixed boxes
shorter than their line, multiple pages, widows/orphans, split text nodes,
contents wrappers, absolute descendants, negative margins, gap, and insets.
All existing case expectations were preserved as the matrix expanded. Full
size containment cases cover both block and flex owners, empty content,
forced breaks and named descendants. Inline-size containment is a separate
control. [CSS Containment 2 section 3.1](https://www.w3.org/TR/css-contain-2/#containment-size)
requires size containment boxes to be monolithic. The WPT060 test/reference
print identically in Chromium; fixing the flex test exposed their formerly
shared contained-block bug in the reference, which is fixed through the same
monolithic classification rather than by changing WPT expectations.

```sh
RUSTUP_TOOLCHAIN=1.95.0 TMPDIR="$HOME/tmp" cargo test -p raikiri-dom --test column_flex_first_line
```


Oversized text/empty boxes, page-height boundaries, first-item background
ownership, and slice/clone holders are recorded independently. A plain
next-page-size cutoff was rejected: Chromium also moves oversized contained
boxes in several cases. Cloned leading-edge ownership is recognized only for
a static margin-free ancestor chain whose per-segment offset equals its
leading padding/border, with every nonzero edge cloned. The fixture also
includes a zero-edge body above the cloned holder, matching WPT031's shape.
