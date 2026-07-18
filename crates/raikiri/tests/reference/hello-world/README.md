# hello-world reference fixture

spec §M1 acceptance criteria の "hello-world VRT" 用 fixture。

- **入力**: `<p style="color:red">Hi</p>` (DOCTYPE / html / body なし、html5ever が
  自動 synthesize)
- **PageBox**: `PageBox::A4` = 793.7008 × 1122.5197 CSS px → 794 × 1123 px raster
- **UA CSS**: M1.4a bundled UA (`p { display: block; }` を含む)
- **Author style**: inline `style="color:red"` のみ
- **Consumer path**: `raikiri::html_to_png_with_fonts(input_bytes, font_ctx)`
  (raikiri-spike-e93 以降。旧 `html_to_png` 経路は system font 依存で
  cross-machine 非決定的だったため置き換え)
- **Test**: `crates/raikiri/tests/hello_world_vrt.rs::hello_world_renders_pixel_exact`

## Font

M1.15 (raikiri-spike-e93) 以降、hello-world VRT は **cross-machine 決定性**
のため WPT bundled fonts (`target/wpt/fonts/`) 経由の pin `FontContext` を
使う。cascade default `font-family: "serif"` は `raikiri_dom::fonts::build_wpt_font_ctx`
の generic alias remap で **Ahem** に解決される。

- `scripts/wpt/fetch.sh` を先に実行して `target/wpt/fonts/` を準備しておくこと
- 未 fetch なら test は "run scripts/wpt/fetch.sh first" で panic する
- pinned SHA は `scripts/wpt/pinned_sha.txt` (fulgur pin を借用)

Golden PNG の visual は Ahem で描画した "Hi" (real text)。過去 (M1.14) の
system serif 版とは bitmap が異なる (2026-07-18 の raikiri-spike-e93 で切替)。

## Golden 更新手順

pipeline を意図的に変えた (font glyph tuning / paint semantics 変更等) 場合のみ:

```bash
# 1. golden を再生成
RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt

# 2. 通常 test で pass 確認
cargo test -p raikiri --test hello_world_vrt

# 3. 変更内容を review してから commit
git add crates/raikiri/tests/reference/hello-world/expected/
```

**注意**: 意図せず expected/page-0000.png が変わった場合、determinism (m1.13)
または UA cascade (m1.22 / m1.23) の regression の可能性がある。commit 前に
必ず `git diff --stat` で size 変化を確認し、視覚的にも diff を review すること。
