# Vertical container ratio measurements

`chromium.json` records 158 cases and 166 border boxes from Chromium
152.0.7977.82 on 2026-11-05. Each `body` is an independent document body.
The default box sizing is content-box; individual cases override it.

Reproduce a case with:

```css
@font-face { font-family: Ahem; src: url("../text-autospace/Ahem.ttf"); }
body { margin: 0; font: 10px/10px Ahem; }
body > div { position: absolute; left: 200px; top: 150px; }
```

Use an 800×600 viewport, confirming both `innerWidth/innerHeight` and
`document.documentElement.clientWidth/clientHeight`. On the recording machine,
headless `--window-size=800,743 --hide-scrollbars` supplied the initial
viewport; the later width-bound recordings used `800,687`. Confirm the
reported viewport rather than relying on the outer window size.
Wait for window load and `document.fonts.ready` before reading
`getBoundingClientRect()` for `#i` and, where present, `#p`. Subtract `(200,150)`
from their coordinates. Images embed a 30×120 SVG; the native test resolver
provides those same natural dimensions.

The cases cover grid item measurement and normal/start/stretch alignment,
auto margins, intrinsic parent width versus a flexed item's cross size,
both vertical writing directions, wrapping in both flex directions, preferred
sizes versus flex basis, child bounds, cyclic percentage edges, physical width
bases, natural text/image sizes, box sizing, and container/child insets.

The native test uses an 800×600 page with zero page margin and the same offset,
so the measured boxes stay clear of page cuts.

```sh
RUSTUP_TOOLCHAIN=1.95.0 TMPDIR="$HOME/tmp" cargo test -p raikiri-dom --test vertical_container_ratio
```
