# WebIDL / Boa / V8 試作

レビュー用の standalone spike。既存の `raikiri-js` / WPT runner は置き換えていない。
基準 revision: `302d3e88f40fba467fa23562ba1b041baaeee4e5`。
課題と結果の正本: `raikiri-spike-qknxc`（bindings）、`raikiri-spike-gr655`（worker 再利用）。

## 構成

`idl/dom.webidl` を weedle で解析し、型付きの中間表現へ変換する。
`build.rs` / `codegen.rs` が次の 3 ファイルを Cargo の `OUT_DIR` へ生成する。

- `common.rs`: receiver 検査、必須引数数、引数変換順序、共通 Rust 操作の呼び出し、戻り値変換。
- `boa.rs`: Boa の callback と interface / member 登録。
- `v8.rs`: V8 の callback と interface / member 登録。

`src/shared.rs` の Rust 操作は実際の `raikiri_dom::Document` を操作する。
JS 値 / GC / wrapper cache / prototype / 例外の保持は各 backend に置く。
JS の文字列変換ではユーザーのコードが再入するため、DOM の借用は変換後に取る。
wrapper は各ページの realm / context の実行中に保持し、ページ終了時に host 側の参照を解除する。
GC 後の同一性は確認するが、
weak wrapper cache や DOM と JS の循環参照回収はこの spike に含まない。

`Worker` は Boa の `Context` / V8 の `Isolate` を保持する。
`run_page` ごとに Boa の `Realm` / V8 の `Context` と新しい `Document` を作る。
interface、prototype、wrapper cache もページごとに作り直す。
V8 の platform 初期化はプロセス内で1回。snapshot / compiled script cache は使っていない。
エンジン内部の interner / JIT / allocator 等は再利用されるため、ページ終了時の完全なメモリ解放は保証しない。

Promise の処理は試作用の `checkpoint()` で明示的に進める。
Boa の executor / V8 の microtask queue はページごとに分離し、残った job は終了時に破棄する。
`markJob()` は worker 共通のカウンタを増やし、古いページの callback 実行を検出するテスト専用 hook。
成功・JS 例外・結果変換エラー・JSON parse エラーの後も host 参照と job を片付け、
WeakRef の kept object を解除する。解除前後の実 GC をテストする。
V8 の queue を戻す操作は `ContextScope` を抜けてから行う。
入ったまま変更すると V8 が fatal abort することを試作中に再現し、順序を修正した。
Context の host slot は `clear_all_slots()` で内部の弱参照管理も含めて解除する。
`remove_slot()` だけでは、CPU2固定の release で空500ページ実行後の isolate 破棄時に
弱参照 callback の abort を再現した。解除方法を変更後、同じ条件のテストと測定で正常終了した。

この IDL は `Node` / `Element` の一部だけを表す。Document fixture は `Node` として公開する。
native 操作名は IDL の camelCase 名から snake_case へ変換し、Rust の型検査で接続を確認する。
未対応の optional / variadic / overload / dictionary / 拡張属性などは黙って省略せず拒否する。

## 再現

リポジトリ直下から実行する。既存 pin の Rust 1.91.0 を使用。

```bash
TMPDIR="$HOME/tmp" cargo test --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml -j 4
TMPDIR="$HOME/tmp" cargo clippy --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --all-targets -j 4 -- -D warnings

TMPDIR="$HOME/tmp" cargo test --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --no-default-features --features boa -j 4
TMPDIR="$HOME/tmp" cargo test --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --no-default-features --features v8 -j 4

TMPDIR="$HOME/tmp" cargo check --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --no-default-features --features wasm --target wasm32-unknown-unknown -j 4

TMPDIR="$HOME/tmp" cargo run --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --bin webidl-dual-engine-spike -- --backend boa
TMPDIR="$HOME/tmp" cargo run --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --bin webidl-dual-engine-spike -- --backend v8

TMPDIR="$HOME/tmp" cargo run --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --bin webidl-dual-engine-spike -- --backend v8 --repeat 100 --worker reuse
TMPDIR="$HOME/tmp" cargo run --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --bin webidl-dual-engine-spike -- --backend v8 --repeat 100 --worker fresh
```

`--worker` の既定値は `reuse`。`--script file.js --repeat N` で同じ script を新しいページで繰り返せる。
`--batch file.json` は JS ソース文字列の配列を読み、順に実行する。ファイル名の配列ではない。
batch / repeat は各ページの結果またはエラーを報告し、エラーの後も次のページへ進む。
`--batch` と `--repeat` は併用できない。

生成結果を別途読む場合は、試作ディレクトリから次を実行する。

```bash
TMPDIR="$HOME/tmp" cargo run --locked --bin idl-generate -- idl/dom.webidl target/generated-preview
```

依存 pin は Boa / boa_gc 0.22.0、v8 152.2.0、weedle 0.13.1。
`Cargo.lock` はこの standalone workspace 専用。本体の manifest / lockfile は変更しない。
WASM 構成の normal dependency graph に `v8` は存在しない。

このマシンの kache が V8 build script の出力だけを復元して native archive を復元せず、
clean 後に `could not find native static library rusty_v8` になった場合は、
同じ pin の archive を新しい一時パスへ取得し、build script を実行し直す。
同じ URL を環境変数に指定するだけでは、その設定の古い出力もキャッシュから復元される場合がある。
依存バージョンは変えない。

```bash
mkdir -p "$HOME/tmp"
v8_archive_dir="$(mktemp -d -p "$HOME/tmp" webidl-v8.XXXXXX)"
curl --fail --location --retry 2 --output "$v8_archive_dir/v8.a.gz" https://github.com/denoland/rusty_v8/releases/download/v152.2.0/librusty_v8_release_x86_64-unknown-linux-gnu.a.gz
TMPDIR="$HOME/tmp" RUSTY_V8_ARCHIVE="$v8_archive_dir/v8.a.gz" cargo test --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml -j 4
rm -rf "$v8_archive_dir"
```

release build でも同じ環境変数を使用できる。この URL はこのマシンの x86_64 Linux 用。

## 検証範囲と判断

両 backend が同じ `tests/contract.js` を実行する。42 ケース:
継承・prototype、Node 型 / 同一性 / nullable 引数、GC 後の同一性、属性操作、
HTML の属性名 case folding、DOMString の null / undefined / number / object / Symbol、
変換順序・再入・例外同一性、引数不足 / 余分な引数、receiver brand / 偽装 / Proxy、
descriptor / readonly / toStringTag、正常な Unicode、native エラーを確認する。
レビューで見つかった例外名への継承 setter 呼び出しは、own data property の定義に修正した。
throwing setter と継承された readonly name の回帰ケースも両 backend で実行する。
Rust の integration test が結果を照合し、`data-final=from-js` を実際の Document から読み取る。

最終確認: 両 backend の42ケースがすべて PASS。default 構成の Rust テスト12件、
Boa 単独 / V8 単独の各10件、clippy `-D warnings`、fmt、Boa の WASM check は PASS。
private item を含む rustdoc と `--cfg test` を加えた rustdoc の両方も `-D warnings` で PASS。
WPT 本体での成績や速度比較は、この結果には含まない。

ページ間の global / lexical binding / JS と DOM の prototype / DOM 属性 / wrapper の分離、
古い Promise job の破棄、次ページの job 実行、例外後の復旧を両 backend で検証する。
同じ worker で20ページ連続して共通42ケースを実行する integration test も含む。
強制 GC をしない空500ページの終了処理も検証し、CPU2固定の release 構成でも12件 PASS。

IDL の `Element : Node` の継承だけを一時的に外すと、両 backend で inheritance ケースが失敗した。
変更は復元した。共通定義が実際の JS 公開構造を制御することを確認できた。

WebIDL → 共通 Rust 操作 + エンジン別 bindings の構成は、この範囲で実現できた。
既存の全 bindings を抽象化する前に、1 interface ずつ移す方針が適している。

## 制約

- DOMString の単独サロゲートは本来保存できる必要があるが、現行 Document は UTF-8 の文字列を保持する。
  試作は変換時に TypeError にしており、仕様適合ではない。両エンジンで診断する fixture を残した。

  ```bash
  TMPDIR="$HOME/tmp" cargo run --locked --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --bin webidl-dual-engine-spike -- --backend v8 --script tools/webidl-dual-engine-spike/tests/surrogate.js
  ```

  `results` は `{"roundtrip":false,"error":"TypeError"}`。Boa でも同じ。
  本格導入には DOMString / USVString の区別と DOM 側の保存方針が必要。
- native DOM エラーは Error に `InvalidCharacterError` の name を付けるだけで、完全な DOMException ではない。
- CSSOM / layout flush / 完全な event loop / timer / legacy collection / cross-realm は未試作。
  Promise は上記の明示 checkpoint とページ間の queue 分離だけを検証し、
  WPT の async testharness や unhandled rejection の報告は接続していない。
- WPT 本体の testharness / idlharness / 全体 baseline は実行していない。
- WASM はコンパイル確認だけで、ブラウザや Wasmtime 上の実行確認ではない。
- 通常の contract 実行の `elapsed_ms` は初期化・GC・テストを含む診断値。
  単発の値から速度比較はしない。下の性能測定では同じ workload と条件で繰り返す。
- sandbox / resource limit / プロセス隔離は未実装。既存の Boa-in-Wasmtime 経路の代替ではない。

## 既存チェックの結果

`scripts/orphan-tests-lint.sh` は PASS。
`scripts/doc-pointer-lint.sh` は変更していない `crates/raikiri-style/src/property/tests/content_tests.rs:589`
の plain comment 内の角括弧で FAIL。基準 commit に同じ記述があることを確認した。
既存不具合として `raikiri-spike-b72gv` に記録した。
試作ディレクトリはこの lint の対象外で、試作の適合性をこの PASS/FAIL から判断しない。

レビューと次段の検討のため、ソースと lockfile は専用 worktree に保持する。
ビルド生成物・生ログ・一時バイナリは、結果を Beads に記録した後に削除する。

## 初回の性能測定（2026-10-10、worker 再利用前）

課題と測定条件の正本: `raikiri-spike-5liya`。`bench.py` で再現できる。
Ryzen 5 5600G、Linux 7.2.5、CPU governor=performance、論理 CPU 2 に固定。
Rust 1.91.0 の通常の release build（追加の LTO / native CPU 最適化なし）。
依存 pin と共通 DOM は上記と同じ。CPU の SMT sibling は 8。CPU は占有していない。

各サンプルを新しいプロセスで実行し、Boa / V8 の順序を交互にする。
workload ごとに最初の各1回を捨て、次の各7回の中央値を取る。
ループは同じ関数を1000回×3呼んでから本測定する。これは完全な JIT warmup の保証ではない。
すべての実行で checksum、属性の実値、mutation count を検査する。
contract は42ケースの全 PASS と実 Document の変更を検査する。

下表の単位は ms。エンジン初期化、compile、warmup、実行、結果 JSON parse、
エンジン破棄を含む `elapsed_ms` の中央値。プロセス起動、script のファイル読み取り、
標準出力は含まない。`bench.py` はそれらを含む `wall_ms` と、
`Date.now()` によるループ内時間（1msの粒度）も記録する。

| workload | 反復回数 | Boa | V8 | Boa/V8 |
| --- | ---: | ---: | ---: | ---: |
| 空の script + JSON 結果 | 0 | 0.617 | 3.412 | 0.18 |
| 共通 contract（GC ケース込み） | 42 ケース | 2.823 | 5.943 | 0.48 |
| JS 整数ループ | 1,000,000 | 49.870 | 5.386 | 9.26 |
| JS 整数ループ | 10,000,000 | 469.157 | 7.830 | 59.91 |
| nodeType getter | 100,000 | 34.678 | 9.610 | 3.61 |
| nodeType getter | 1,000,000 | 321.472 | 48.787 | 6.59 |
| setAttribute + getAttribute | 25,000 組 | 20.885 | 18.153 | 1.15 |
| setAttribute + getAttribute | 250,000 組 | 171.372 | 108.338 | 1.58 |
| object → DOMString + set/get | 25,000 組 | 32.233 | 21.227 | 1.52 |
| object → DOMString + set/get | 250,000 組 | 284.154 | 137.574 | 2.07 |
| parentNode 同一性 + isSameNode | 100,000 組 | 79.476 | 18.505 | 4.29 |
| parentNode 同一性 + isSameNode | 1,000,000 組 | 758.557 | 126.676 | 5.99 |

OS からのプロセス起動を含む中央値: 空の script は Boa 4.282 / V8 7.069 ms、
contract は Boa 6.932 / V8 9.658 ms。JS 1000万回は Boa 473.649 / V8 11.585 ms。
全ケースのばらつき等は Beads に記録済み。生データは結果を記録後に削除する。

この native 試作では、長い JS ループと DOM getter の反復は V8 が速い。
共通 Rust DOM で属性を変更・取得する処理では差が縮まり、
新しいエンジンで短いテストを実行する場合は Boa が速い。
WPT 全体への効果は、エンジンの初期化を使い回す実際の runner で測る必要がある。
既存の Boa-in-Wasmtime 経路との比較、WPT 本体、layout、メモリ使用量は未測定。

```bash
TMPDIR="$HOME/tmp" cargo build --locked --release --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --bin webidl-dual-engine-spike -j 4
TMPDIR="$HOME/tmp" python3 -B tools/webidl-dual-engine-spike/bench.py --cpu 2 --samples 7 --output "$HOME/tmp/webidl-perf.json"
```

一時 JS は `TMPDIR` 内で自動削除される。出力 JSON は確認・記録後に削除する。

## worker を再利用した性能測定（2026-10-10）

課題: `raikiri-spike-gr655`。`bench-workers.py` で同じ revision の `fresh` / `reuse` を比較する。
CPU、release、依存 pin は上記と同じ。各 batch は新しいプロセスで起動し、
Boa/V8 × fresh/reuse の4通りの実行順を毎回ずらす。
最初の1巡を捨て、次の7巡の中央値を取る。全160 batch / 21,120ページを検査した。
42ケースの試験は全134,400ケースが PASS。loop は各ページで1000回×3 warmup する。

両 mode でページの Realm / Context / Document / bindings は毎回新しくする。
V8 platform は両 mode ともプロセス内で1回だけ初期化される。
`fresh` はページごとに worker を作って破棄し、`reuse` は batch に1つだけ作る。
全ページの checksum / 実 Document の属性 / mutation count / callback count、
worker の実際の初期化回数も検査する。

単位は ms/ページ。`elapsed_ms / ページ数` の各 batch の値から中央値を取る。
worker 初期化、各ページの作成・実行・片付け、Rust側の結果収集、worker 破棄を含む。
プロセス起動・入力読み取り・標準出力は除く。ページ単独の `elapsed_ms` は worker 初期化・
破棄と結果収集を含まない。生データのばらつき、初期化時間、外部 wall time は Beads に記録済み。

| workload | batch のページ数 | Boa fresh | Boa reuse | V8 fresh | V8 reuse |
| --- | ---: | ---: | ---: | ---: | ---: |
| 空の script + JSON | 500 | 0.445 | 0.529 | 1.221 | 0.336 |
| 共通42ケース（GC込み） | 100 | 2.300 | 5.632 | 3.395 | 1.437 |
| JS 整数ループ100万回 | 20 | 49.458 | 48.489 | 2.801 | 1.785 |
| nodeType getter 10万回 | 20 | 31.333 | 31.078 | 8.828 | 7.746 |
| setAttribute + getAttribute 2.5万組 | 20 | 19.940 | 19.789 | 15.447 | 14.586 |

V8 は空の script で約3.6倍、42ケースで約2.4倍の短縮になった。
この42ケースでは、V8 reuse は Boa fresh より約1.6倍速い。
属性操作など実行自体が長い workload では、初期化を省く割合は小さくなる。
snapshot や script cache を追加する前に、worker の使い回しだけでも効果がある。

Boa の `Context::create_realm()` による長寿命 worker は、この試作では改善しなかった。
42ケースの reuse では、先頭10ページのページ時間中央値2.196msから末尾10ページ10.129msへ増えた。
native finalizer の追加診断では、閉じた10個の Realm の Probe が Context 生存中は0個、
Context 破棄後に10個回収された。`Realm::create` の RootShape だけを変える対照でも、
共有RootShapeでは0個、新しいRootShapeでは10個が Context 生存中に回収された。
共有RootShapeを変える対照で保持の差を確認した。正確な内部経路・対処は未解明で、
再現と今後の調査を `raikiri-spike-lry65` に記録した。
Boa の再利用を本体に採用する前に解決が必要。新しいRootShapeを渡すだけの置換は
default global bindings の初期化を欠くため採用していない。

この結果は native の限定した試作で、WPT 全体・Wasmtime・layout の成績や総メモリ使用量を示さない。

```bash
TMPDIR="$HOME/tmp" cargo build --locked --release --manifest-path tools/webidl-dual-engine-spike/Cargo.toml --bin webidl-dual-engine-spike -j 4
TMPDIR="$HOME/tmp" python3 -B tools/webidl-dual-engine-spike/bench-workers.py --cpu 2 --samples 7 --output "$HOME/tmp/webidl-worker-perf.json"
```
