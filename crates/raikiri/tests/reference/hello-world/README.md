# hello-world reference fixture

spec §M1 acceptance criteria の "hello-world VRT" 用 fixture。

- **入力**: `<p style="color:red">Hi</p>` (DOCTYPE / html / body なし、html5ever が
  自動 synthesize)
- **PageBox**: `PageBox::A4` = 793.7008 × 1122.5197 CSS px → 794 × 1123 px raster
- **UA CSS**: M1.4a bundled UA (`p { display: block; }` を含む)
- **Author style**: inline `style="color:red"` のみ
- **Consumer path**: `raikiri::html_to_png(input_bytes)`
- **Test**: `crates/raikiri/tests/hello_world_vrt.rs::hello_world_renders_pixel_exact`

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
