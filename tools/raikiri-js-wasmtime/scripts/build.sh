#!/usr/bin/env bash
set -euo pipefail
trial_root=$(cd "$(dirname "$0")/.." && pwd)
repo_root=$(cd "$trial_root/../.." && pwd)
# All build stages are explicit. No build.rs invokes external tools.
trial_target=${CARGO_TARGET_DIR:-"$repo_root/target/wasmtime-trial"}
export CARGO_TARGET_DIR="$trial_target"
mkdir -p "$trial_target/aot"
cargo build --offline --locked --release --manifest-path "$trial_root/Cargo.toml" -p raikiri-js-wasmtime-guest --target wasm32-wasip1
trial_guest="$trial_target/wasm32-wasip1/release/raikiri_js_wasmtime_guest.wasm"
trial_aot="$trial_target/aot/guest-fuel.cwasm"
cargo run --offline --locked --release --manifest-path "$trial_root/Cargo.toml" -p raikiri-js-wasmtime-precompiler -- "$trial_guest" "$trial_aot"
python3 - "$trial_guest" "$trial_aot" <<'PY'
import hashlib,json,platform,sys
from pathlib import Path
wasm,aot=map(Path,sys.argv[1:])
meta={'wasmtime':'43.0.2','guest_sha256':hashlib.sha256(wasm.read_bytes()).hexdigest(),'aot_sha256':hashlib.sha256(aot.read_bytes()).hexdigest(),
      'host_target':'x86_64-unknown-linux-gnu','platform':platform.platform(),'page_size':4096,'meter':'fuel','fuel_default':10000000000,
      'memory_limit_default':134217728,'wasm_stack':524288,'memory_reservation':134217728,'guard_size':65536}
aot.with_suffix('.json').write_text(json.dumps(meta,indent=2)+'\n')
PY
export RAIKIRI_JS_AOT="$trial_aot"
cargo build --offline --locked --release --manifest-path "$repo_root/Cargo.toml" -p raikiri-wpt --no-default-features --features js-wasmtime --bin run-parsing-invalid --bin run-css-text-i18n
