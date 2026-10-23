#!/usr/bin/env bash
# Share the gate's third-party build artifacts between containers and
# worktrees, so a cold scripts/gate.sh does not compile the dependency graph
# again.
#
# Usage:
#   scripts/gate-cache.sh restore   unpack the newest matching cache into target/
#   scripts/gate-cache.sh save      pack target/ into the cache (destructive, see below)
#   scripts/gate-cache.sh status    show the cache location and its entries
#
# On a cold 4-vCPU container nearly all of the gate's wall-clock time is
# compiling third-party crates (stylo, boa, shodo, blitz, ...), and none of
# that depends on the commit under test. The cache holds exactly those
# artifacts: target/llvm-cov-target (the cargo-llvm-cov build, which also
# serves the doctests and validate-expectations through
# scripts/lib/instrumented-run.sh) and target/debug (clippy and rustdoc).
# Restoring it leaves only the workspace crates to compile.
#
# restore is what scripts/gate.sh runs before the instrumented build. It is
# best effort: it does nothing when target/ already has an instrumented build
# or when no cache entry exists, and the gate then builds as it always did.
# cargo decides what is still fresh, so an entry saved for an older
# Cargo.lock is still useful; only crates whose version changed recompile.
# An entry saved by a different rustc or cargo-llvm-cov is never restored.
#
# save is destructive: it first removes the workspace crates' artifacts
# (cargo llvm-cov clean --workspace and cargo clean -p <member>) and the
# incremental directories, so that the entry holds third-party artifacts
# only. Run it when you are done with the gate in this checkout; the next gate
# run recompiles the workspace crates. Refresh the cache after a large
# Cargo.lock change or a toolchain bump.
#
# Entries live in $RAIKIRI_GATE_CACHE_DIR, which defaults to
# /mnt/project-files/cache/raikiri-gate/ when /mnt/project-files exists (the
# directory every cloud thread of the project shares) and to
# ~/.cache/raikiri-gate-target otherwise. An entry is a tar.gz plus a small
# .info file that is written last, so a reader never trusts a half-written
# archive. The newest three entries are kept.
#
# Exit status: 0 on success and when restore has nothing to do, 1 on a
# failure, 2 on a usage error.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

KEEP_ENTRIES=3
# The directories of target/ that hold reusable artifacts.
PAYLOAD_DIRS=(llvm-cov-target debug)

log() {
  echo "gate-cache: $*"
}

die() {
  echo "gate-cache: $*" >&2
  exit 1
}

usage() {
  echo "usage: scripts/gate-cache.sh restore | save | status" >&2
  exit 2
}

if [[ $# -ne 1 ]]; then
  usage
fi
COMMAND="$1"
case "$COMMAND" in
  restore | save | status) ;;
  -h | --help)
    awk 'NR==1 { next } /^#/ { print; next } { exit }' "${BASH_SOURCE[0]}"
    exit 0
    ;;
  *) usage ;;
esac

cd "$REPO_ROOT"

if [[ -n "${RAIKIRI_GATE_CACHE_DIR:-}" ]]; then
  CACHE_DIR="$RAIKIRI_GATE_CACHE_DIR"
elif [[ -d /mnt/project-files ]]; then
  CACHE_DIR="/mnt/project-files/cache/raikiri-gate"
else
  CACHE_DIR="${HOME:?HOME must be set}/.cache/raikiri-gate-target"
fi

# One cargo call gives the target directory (line 1) and the workspace member
# names (the other lines).
if ! METADATA="$(cargo metadata --no-deps --format-version 1 | python3 -c '
import json, sys
metadata = json.load(sys.stdin)
print(metadata["target_directory"])
for package in metadata["packages"]:
    print(package["name"])
')"; then
  die "cargo metadata failed; cannot locate the target directory."
fi
TARGET_DIR="$(printf '%s\n' "$METADATA" | sed -n '1p')"
WORKSPACE_PACKAGES=()
while IFS= read -r package; do
  WORKSPACE_PACKAGES+=("$package")
done < <(printf '%s\n' "$METADATA" | sed '1d')

# An entry is only reusable by the same compiler and the same cargo-llvm-cov
# (which decides the instrumentation flags); Cargo.lock only picks the best
# match among those.
toolchain_id() {
  {
    rustc -vV
    cargo llvm-cov --version 2>/dev/null || echo "no cargo-llvm-cov"
  } | sha256sum | cut -c1-12
}

lock_id() {
  sha256sum Cargo.lock | cut -c1-12
}

human_size() {
  awk -v bytes="$1" 'BEGIN { printf "%.0f MiB", bytes / 1048576 }'
}

# Names of the complete entries, newest first (the UTC timestamp leads the
# name, so a reverse sort is chronological).
list_entries() {
  [[ -d "$CACHE_DIR" ]] || return 0
  ls -1 "$CACHE_DIR" | { grep -E '^gate-target-.*\.info$' || true; } |
    sed 's/\.info$//' | LC_ALL=C sort -r
}

# An entry is usable when its archive exists with the size the .info recorded.
entry_is_valid() {
  local name="$1" expected actual
  expected="$(sed -n 's/^bytes=//p' "$CACHE_DIR/$name.info")"
  [[ -n "$expected" && -f "$CACHE_DIR/$name.tar.gz" ]] || return 1
  actual="$(wc -c <"$CACHE_DIR/$name.tar.gz" | tr -d ' ')"
  [[ "$actual" == "$expected" ]]
}

# Prints the entry to restore: the newest one saved for the same Cargo.lock,
# else the newest one for the same toolchain. Prints nothing when none fits.
pick_entry() {
  local toolchain="$1" lock="$2" name same_toolchain=""
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    entry_is_valid "$name" || continue
    if [[ "$name" == *"-tc${toolchain}-lock${lock}" ]]; then
      echo "$name"
      return 0
    fi
    if [[ -z "$same_toolchain" && "$name" == *"-tc${toolchain}-lock"* ]]; then
      same_toolchain="$name"
    fi
  done < <(list_entries)
  [[ -z "$same_toolchain" ]] || echo "$same_toolchain"
}

do_status() {
  local toolchain lock picked name
  toolchain="$(toolchain_id)"
  lock="$(lock_id)"
  log "cache dir  : $CACHE_DIR"
  log "target dir : $TARGET_DIR"
  log "toolchain  : tc$toolchain (rustc and cargo-llvm-cov), Cargo.lock: lock$lock"
  if [[ -d "$TARGET_DIR/llvm-cov-target/debug" ]]; then
    log "target/llvm-cov-target/debug exists: restore would do nothing"
  fi
  picked="$(pick_entry "$toolchain" "$lock")"
  local found=0
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    found=1
    local note="" size=""
    if entry_is_valid "$name"; then
      size="$(human_size "$(wc -c <"$CACHE_DIR/$name.tar.gz" | tr -d ' ')")"
    else
      note=" (incomplete, ignored)"
    fi
    [[ "$name" != "$picked" ]] || note=" <- restore would use this"
    log "entry      : $name ${size}${note}"
  done < <(list_entries)
  if [[ "$found" -eq 0 ]]; then
    log "no entries"
  fi
}

do_restore() {
  if [[ -d "$TARGET_DIR/llvm-cov-target/debug" ]]; then
    log "$TARGET_DIR/llvm-cov-target/debug already exists; leaving target/ alone."
    return 0
  fi
  if [[ ! -d "$CACHE_DIR" ]]; then
    log "no cache directory at $CACHE_DIR; building from scratch."
    return 0
  fi
  local toolchain lock name
  toolchain="$(toolchain_id)"
  lock="$(lock_id)"
  name="$(pick_entry "$toolchain" "$lock")"
  if [[ -z "$name" ]]; then
    log "no entry for this toolchain (tc$toolchain) in $CACHE_DIR; building from scratch."
    return 0
  fi
  log "restoring $name"
  if [[ "$name" != *"-lock${lock}" ]]; then
    log "(saved for a different Cargo.lock; cargo recompiles the crates that changed)"
  fi

  # Unpack beside the destination, then move each directory into place: a
  # failed or interrupted extraction never leaves half a directory that cargo
  # could mistake for a finished build.
  mkdir -p "$TARGET_DIR"
  local staging
  staging="$(mktemp -d "$TARGET_DIR/.gate-cache-restore.XXXXXX")"
  # shellcheck disable=SC2064
  trap "rm -rf '$staging'" EXIT
  local started="$SECONDS"
  if ! tar -C "$staging" -xzf "$CACHE_DIR/$name.tar.gz"; then
    echo "gate-cache: unpacking $name failed; building from scratch." >&2
    return 1
  fi
  local dir
  for dir in "${PAYLOAD_DIRS[@]}"; do
    [[ -d "$staging/$dir" ]] || continue
    if [[ -e "$TARGET_DIR/$dir" ]]; then
      log "kept the existing $TARGET_DIR/$dir"
    else
      mv "$staging/$dir" "$TARGET_DIR/$dir"
    fi
  done
  log "restored in $((SECONDS - started))s"
}

do_save() {
  cargo llvm-cov --version >/dev/null 2>&1 ||
    die "cargo-llvm-cov is not installed (cargo install cargo-llvm-cov)."
  [[ -d "$TARGET_DIR/llvm-cov-target/debug" ]] ||
    die "$TARGET_DIR/llvm-cov-target/debug does not exist; run scripts/gate.sh first."
  [[ -d "$CACHE_DIR" ]] || mkdir -p "$CACHE_DIR" ||
    die "cannot create $CACHE_DIR."

  log "removing the workspace crates' artifacts"
  cargo llvm-cov clean --workspace
  local clean_args=() package
  for package in "${WORKSPACE_PACKAGES[@]}"; do
    clean_args+=(-p "$package")
  done
  if [[ ${#clean_args[@]} -gt 0 ]]; then
    cargo clean --locked "${clean_args[@]}"
  fi

  local toolchain lock stamp name members=() dir
  toolchain="$(toolchain_id)"
  lock="$(lock_id)"
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  name="gate-target-${stamp}-tc${toolchain}-lock${lock}"
  for dir in "${PAYLOAD_DIRS[@]}"; do
    if [[ -d "$TARGET_DIR/$dir" ]]; then
      members+=("$dir")
    fi
  done

  local tmp_tar="$CACHE_DIR/.tmp.$$.$stamp.tar.gz"
  # shellcheck disable=SC2064
  trap "rm -f '$tmp_tar' '$tmp_tar.info'" EXIT
  log "packing ${members[*]} from $TARGET_DIR"
  local started="$SECONDS"
  # Incremental directories only hold workspace-crate state, which was just
  # removed, and are large.
  tar -C "$TARGET_DIR" --exclude='debug/incremental' -cf - "${members[@]}" |
    gzip -1 >"$tmp_tar"
  local bytes
  bytes="$(wc -c <"$tmp_tar" | tr -d ' ')"
  mv "$tmp_tar" "$CACHE_DIR/$name.tar.gz"
  # The .info file marks the entry complete, so it goes last.
  {
    echo "bytes=$bytes"
    echo "created=$stamp"
    echo "rustc=$(rustc -vV | sed -n 's/^release: //p')"
    echo "cargo_llvm_cov=$(cargo llvm-cov --version | head -1)"
  } >"$tmp_tar.info"
  mv "$tmp_tar.info" "$CACHE_DIR/$name.info"
  log "saved $name ($(human_size "$bytes")) in $((SECONDS - started))s"

  prune
}

# Drops entries beyond the newest KEEP_ENTRIES and leftovers of savers that
# died before finishing.
prune() {
  local index=0 name
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    index=$((index + 1))
    if [[ "$index" -gt "$KEEP_ENTRIES" ]]; then
      log "pruning $name"
      rm -f "$CACHE_DIR/$name.info" "$CACHE_DIR/$name.tar.gz"
    fi
  done < <(list_entries)
  find "$CACHE_DIR" -maxdepth 1 -name '.tmp.*' -mmin +180 -exec rm -f {} + 2>/dev/null || true
  # An archive whose .info never arrived (the saver died between the two
  # renames) is not an entry.
  local archive
  for archive in "$CACHE_DIR"/gate-target-*.tar.gz; do
    [[ -e "$archive" ]] || continue
    [[ ! -e "${archive%.tar.gz}.info" ]] || continue
    if [[ -n "$(find "$archive" -mmin +60 2>/dev/null)" ]]; then
      rm -f "$archive"
    fi
  done
}

case "$COMMAND" in
  restore) do_restore ;;
  save) do_save ;;
  status) do_status ;;
esac
