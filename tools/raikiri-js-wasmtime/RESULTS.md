# Wasmtime WPT integration trial — 実測結果

2026-09-27。Linux x86_64 / 4KiB pages、Rust 1.91.0、Boa 0.22.0、Wasmtime 43.0.2。
WPT: `97ea26e26a2aac3eec7e770650b25e7049ed4a4e`。検証はローカル試験として実施。実装完了後のユーザー指示によりdraft PRとして公開する。mergeは未実施。

## 結果

実際の raikiri-js DomRuntime / Boa を WASI reactor にし、燃料計測付き AOT を WPT runner に埋め込んだ。
ゲストが論理 DOM と JS を持ち、既存の native CSS / layout / script loader に bounded JSON RPC する。
native は既定、Wasmtime は `--no-default-features --features js-wasmtime`。実行時切替・fallback はない。

948ページ・13,751 assertion の名前、順序、成功／失敗、エラー、メッセージが両backendで完全一致。
両suiteとも結果JSONのSHA256も一致した。既存の失敗とimportWorklet実行エラーは維持し、期待値を変更していない。

| Suite | 全assertion成功のfile | assertion成功 | 実行エラー | 結果差分 |
|---|---:|---:|---:|---:|
| parsing | 438/751 | 6920/12195 | 1 | 0 |
| i18n | 113/197 | 452/1556 | 0 | 0 |

## 時間とRSS

fresh release binariesを別ディレクトリに保存し、native parsing → native i18n → Wasmtime parsing → Wasmtime i18n の順で各1回実行した。
同時にbuild / testを走らせていない。各runに30分の外側timeout。全runでmetricsを有効にした値であり、計測処理のコストを別に引いていない。
繰り返し試行による分散推定はしていない。RSSはfont / CSS / layoutを含むWPTプロセス全体で、純粋なエンジンのbaselineではない。

| Suite | native秒 | Wasmtime秒 | 比率 | native観測最大RSS MiB | Wasmtime観測最大RSS MiB |
|---|---:|---:|---:|---:|---:|
| parsing | 51.73 | 80.97 | 1.57× | 82.3 | 64.8 |
| i18n | 24.54 | 30.76 | 1.25× | 45.8 | 55.7 |

観測最大RSSは20ms間隔の外側VmRSSサンプルと、各ページの初期化／実行後に採ったVmRSSの最大を使う。
短いピークは取り逃がし得る。元のサンプル値・VmHWM・各ページ値はlogsに保存した。
各ページの初期／実行後RSS中央値はプロセス内の既存allocation / Module / native cachesを含む。

| Backend / Suite | ページ初期RSS中央値 MiB | 実行後RSS中央値 MiB |
|---|---:|---:|
| native-parsing | 73.6 | 73.6 |
| native-i18n | 36.2 | 36.7 |
| wasmtime-parsing | 47.7 | 52.1 |
| wasmtime-i18n | 40.3 | 44.6 |

## RPC / fuel / linear memory

bridgeはguest→native service呼び出し、guest ABIはhost→guest export呼び出し。bytesは各方向の累計、memoryはページ別最大の全ページ最大。

| Suite | bridge calls | bridge request MiB | bridge response MiB | guest ABI request MiB | guest ABI response MiB | 最大fuel | 最大linear memory MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| parsing | 2292 | 0.55 | 152.93 | 22.41 | 25.61 | 8,072,877,730 | 19.44 |
| i18n | 3335 | 8.58 | 43.26 | 6.30 | 8.41 | 763,339,073 | 9.81 |

通信量だけでCPU時間の支配要因は断定していない。fuel計測、BoaのWASM実行、snapshot / JSON処理の分離profilingは未実施。

## Artifact / 再現

guest wasm → trusted AOT → embedded runner の順で明示的にbuildする。build.rsからCargo / compilerを呼ばない。
runtime依存treeに `wasmtime-internal-cranelift` / `cranelift-codegen` / frontend はない。共有metadata型のbitset / entity / bforestは含まれる。
単一のprocess-wide Engine / Moduleを独立Storeで再利用する。runnerの隣にWASM / AOTファイルを置く必要はない。

| Artifact | bytes | MiB | SHA256 |
|---|---:|---:|---|
| raikiri_js_wasmtime_guest.wasm | 10,061,166 | 9.60 | `ac77ddc6806f78d86bc2d2e1cccb0f4e361aa568c813c136718286b2262ba5e7` |
| guest-fuel.cwasm | 51,684,904 | 49.29 | `397d533f7ad155f02b0aec5afaf297166a385a17934c4df83cbe3b3668ff2e3e` |
| native-bins/run-parsing-invalid | 28,108,088 | 26.81 | `9aa13955f5ac6422dfee31bbdcdf3b7d3f0a2c6fb447ddf6e4dbc62d7b0c3f34` |
| native-bins/run-css-text-i18n | 28,103,744 | 26.80 | `4df0a8b788a2ce9a98f667b62864ed5b863698d0f937993c73094ec74623faea` |
| wasmtime-bins/run-parsing-invalid | 66,854,944 | 63.76 | `b2ceb6c9249cd83b165b182a9384957d8a0946409a14d937946a068b969e6c84` |
| wasmtime-bins/run-css-text-i18n | 66,848,160 | 63.75 | `f4c233234cb0b38389ccd43b4a2cdfe93fb8c59c8d3fd2863a185613b4c4eea6` |

AOT manifest: `/home/mitz/Work/oss/raikiri-spike/target/aot/guest-fuel.json`。compiler / runtimeはfuel、512KiB stack、128MiB reservation、64KiB guardを共有する。
既定budgetは1ページあたりfuel 10,000,000,000、linear memory 128MiB。初期化・構文解析・callbacks・getter・結果変換・teardownの間にfuelを補充しない。

```bash
bash tools/raikiri-js-wasmtime/scripts/build.sh
# runner build時に指定したtargetディレクトリ配下のrelease binaryを使う。
target/wasmtime-trial/release/run-parsing-invalid --wpt-root target/wpt --results-json target/parsing.json
target/wasmtime-trial/release/run-css-text-i18n --wpt-root target/wpt --results-json target/i18n.json
```

## 検証

- DOM snapshot: 700 unit成功、12 integration成功、既存unit ignore 2。
- 既存raikiri-js: 361成功。
- WPT Rust全test: native 277成功、Wasmtime 278成功、両方0失敗・既存ignore187。
- 実際のresources/testharness.jsを使う既存ignore7件を明示実行し7成功。
- trial workspace: guest / protocol / hostの14 test成功。並列Store隔離、metering不一致artifact拒否も確認。
- builtin長時間loop、regex backtracking、call tree、深いparser入力、allocation、結果getterを停止し、次document生存を確認。
- malformed / detached / template / namespaced arenaと反復CSS pseudo flush、短いresponse bufferでexactly-once RPCを確認。
- 両featureのclippy、guest WASM clippy、trial clippy、両workspace fmt、native default workspace doc、feature別private docs、trial docs、orphan lint成功。
- doc-pointer lintのplain-comment違反3件は変更前baselineと同じ。ratchetは5 <= 21。全面greenとは主張しない。
- --cfg test補助rustdocは変更前後ともtempfile E0432/E0433で停止し、link検証の証拠には使っていない。通常private-item補助docで追加link errorなし。
- fresh-context最終reviewはCritical0 / Important2 / Minor0。2件ともRED→GREENで修正。
  非有限harness statusとclean Abort優先順位を実WPT adapterで再現し修正。計測異常終了＋古いJSON利用もCLIで再現し修正。
- 初回比較で見つかったgeometry f64 1ULPのmessage差10件はfloat_roundtripで修正。最終比較はmessage差も0。

## 判断と制約

- 実行ledgerにはbd commentsを使った。repoのbd一元管理規約を優先。誤判断ならskillのmarkdown automationを失うがdurable evidenceは残る。
- owned DTO変換は既存raikiri-js型に依存し、公開型にserdeを追加しなかった。誤判断ならnative JS型へのcompile依存が残る。JsValueは境界を跨がない。
- Cargo自動enrollを防ぐためrootにnested workspace directoryのexcludeと各trial packageのworkspace指定を追加。13 main / 5 trial membersをlocked metadataで検証。誤判断ならworkspace解決への影響。wildcard excludeの失敗案は置換済み。
- 本番採用、macOS / Windows対応、native host全体のCPU / memory containmentを判断対象外とした。承認済みlocal scopeを維持。誤判断ならtrialを本番／platform安全性の根拠と誤認する。
- performance / named parityはreview時未完だったため、ここで新しい全runを評価した。merge判断はしない。誤判断なら途中結果で完了を宣言するか、未承認の共有変更になる。
- ローカル試験完了時はuserのno-commit / no-pushをskillのcommit / cleanupより優先し、uncommitted worktreeを保持した。後続のユーザー指示でdraft PR公開（commit / push）を許可された。誤判断なら履歴化されない成果を誤って削除する。
- 軽微なreview指摘の保留はない。
- native CSS / layout / font / fragment / fetch処理はguest fuelとlinear-memory capの対象外。外側process timeoutを維持する。
- clean guest Abortは最終論理DOMの同期を試みる。後続の結果転送／DOM同期が失敗しても既知のAbort理由を優先し、転送失敗と最後のnative checkpointのみが残ることをstderrの`WASM_DIAGNOSTIC`に記録する。engine trapは停止Storeを破棄し、最後のnative checkpointだけ残す。
- Linux x86_64 / 4KiB pages限定。元のCPU / parser / memory security issueは閉じていない。merge gateは実行していない。

## Evidence

worktree: `/home/mitz/Work/oss/raikiri-spike/.worktrees/wasmtime-wpt-integration`
branch: `spike/wasmtime-wpt-integration`。base: `ab7e619a8f321f03de8b8c8b9342954868e044c8`。
machine-local raw logs / named results / RSS samples / hashes: `/home/mitz/Work/oss/raikiri-spike/.worktrees/wasmtime-wpt-integration/target/integration-logs`。
主要fileはmeasurements.json、metrics-summary.json、*-comparison.json、wpt-*-final.log、trial-final-contract-tests.log。
最終reviewのimmutable patchとmanifestは `target/integration-logs/review/` に保持。初回計測はinitial-measurement/に保存した。
