#!/usr/bin/env bash
# Run pinned upstream WPT reftests through the external Raikiri product.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WPT_ROOT="$REPO_ROOT/target/wpt"
WPT_VENV="$REPO_ROOT/target/wpt-venv"
BROWSER_BINARY="$REPO_ROOT/target/debug/raikiri-wpt-browser"
PYTHON_VERSION="3.14.7"

cd "$REPO_ROOT"
"$SCRIPT_DIR/fetch.sh"
cargo build --locked -p raikiri-wpt --bin raikiri-wpt-browser

if [ ! -x "$WPT_VENV/bin/python" ]; then
  uv venv --python "$PYTHON_VERSION" "$WPT_VENV"
elif ! "$WPT_VENV/bin/python" -c \
  "import sys; raise SystemExit(sys.version_info[:3] != (3, 14, 7))"
then
  uv venv --clear --python "$PYTHON_VERSION" "$WPT_VENV"
fi

uv pip install \
  --python "$WPT_VENV/bin/python" \
  --requirements "$WPT_ROOT/tools/manifest/requirements.txt" \
  --requirements "$WPT_ROOT/tools/wptrunner/requirements.txt"

uv pip install \
  --python "$WPT_VENV/bin/python" \
  --no-build-isolation \
  --no-deps \
  --editable "$REPO_ROOT/tools/wptrunner-raikiri"

cd "$WPT_ROOT"
exec "$WPT_VENV/bin/python" ./wpt \
  --venv "$WPT_VENV" \
  --skip-venv-setup \
  run raikiri \
  --binary "$BROWSER_BINARY" \
  --ssl-type none \
  "$@" \
  --test-types reftest
