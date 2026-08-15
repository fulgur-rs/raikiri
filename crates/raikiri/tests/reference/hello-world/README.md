# hello-world reference fixture

"hello-world VRT" の acceptance criteria 用 fixture。

- **入力**: `<!DOCTYPE html>` + 明示 `<html>` / `<head></head>` / `<body>` skeleton
  内に `<p style="color: red; font-size: 24px; margin: 20px">Hello, world!</p>`
  (v1 の bare fragment から structured HTML5 に更新)
- **PageBox**: `PageBox::A4` = 793.7008 × 1122.5197 CSS px → 794 × 1123 px raster
- **UA CSS**: bundled UA (`p { display: block; }` を含む)
- **Author style**: inline `color: red` + `font-size: 24px` + `margin: 20px`
  (color / font-size / margin の paint pipeline consume を hello-world 段階で
  最低 1 property ずつカバー)
- **Consumer path**: `raikiri::html_to_png_with_fonts(input_bytes, font_ctx)`
  (旧 `html_to_png` 経路は system font 依存で
  cross-machine 非決定的だったため置き換え)
- **Test**: `crates/raikiri/tests/hello_world_vrt.rs::hello_world_renders_pixel_exact`

## Font

hello-world VRT は **cross-machine 決定性**
のため WPT bundled fonts (`target/wpt/fonts/`) 経由の pin `FontContext` を
使う。cascade default `font-family: "serif"` は `raikiri_dom::fonts::build_wpt_font_ctx`
の generic alias remap で **Ahem** に解決される。

- `scripts/wpt/fetch.sh` を先に実行して `target/wpt/fonts/` を準備しておくこと
- 未 fetch なら test は "run scripts/wpt/fetch.sh first" で panic する
- pinned SHA は `scripts/wpt/pinned_sha.txt` (fulgur pin を借用)

Golden PNG の visual は **`Hello, world!` の英字・comma・`!` が全て 24px
em-box の red square、space (U+0020) のみ Ahem font 仕様上 transparent 1em
glyph で描画されず gap として残る**。Ahem は WPT
test font で、`X` が 1em×1em の solid square と定義され、"most other US-ASCII
characters" (英字・数字・comma 含む) が同 glyph を共有、space (U+0020) が
唯一の documented transparent 例外である (Ahem spec
https://web-platform-tests.org/writing-tests/ahem.html)。実際 PNG は 2 つの
solid red block (`Hello,` 6-em + `world!` 6-em) が 20px margin offset + space
1-em gap で並ぶ形。
注: "hello-world" という fixture name は semantic なもので、実際の visual
rendering は WPT-style em-box red square 列 (space のみ gap) であり、認識可能な
letterform ではない。過去の system serif 版とは bitmap が異なる (2026-07-18 に
Ahem 経路へ切替、2026-07-24 に v2 shape へ regenerate)。

## Golden 更新手順

pipeline を意図的に変えた (font glyph tuning / paint semantics 変更等) 場合のみ:

```bash
# 1. golden を再生成 (VRT test は #[ignore] のため --ignored が必須)
RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt -- --ignored

# 2. 通常 test で pass 確認 (同じく --ignored が必須)
cargo test -p raikiri --test hello_world_vrt -- --ignored

# 3. 変更内容を review してから commit
git add crates/raikiri/tests/reference/hello-world/expected/
```

**注意**: 意図せず expected/page-0000.png が変わった場合、determinism
または UA cascade の regression の可能性がある。commit 前に
必ず `git diff --stat` で size 変化を確認し、視覚的にも diff を review すること。
