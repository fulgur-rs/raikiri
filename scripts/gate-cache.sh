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
# restore is what scripts/gate.sh runs before the instrumented build (not on
# its --skip-coverage path, which does not use target/llvm-cov-target). It is
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
# Cargo.lock change or a toolchain bump. cargo never deletes the artifacts of
# a crate version that left Cargo.lock, so entries saved from a checkout that
# was itself restored from an older entry grow with every refresh; to start
# over, build in a target/ that was not restored (RAIKIRI_GATE_CACHE=0 after
# rm -rf target) and save from there.
#
# Entries live in $RAIKIRI_GATE_CACHE_DIR, which defaults to
# /mnt/project-files/cache/raikiri-gate/ when /mnt/project-files exists (the
# directory every cloud thread of the project shares) and to
# ~/.cache/raikiri-gate-target otherwise. An entry is a tar.gz cut into parts
# of 128 MiB (<name>.part.aa, .ab, ...) plus a small <name>.info that lists
# every part with its size and is written last, so a reader never trusts a
# half-written entry. The newest three entries are kept.
#
# Why parts: the shared project mount does not persist large files. Files up
# to 200 MB written to it reached other containers; a 650 MB file never did,
# yet the container that wrote it kept seeing the full file in its local cache.
# Keep every part well below that. Another container's `status` shows whether
# an entry really arrived ("incomplete" means it did not); check it from a
# second container after the first save. RAIKIRI_GATE_CACHE_PART_BYTES
# overrides the part size.
#
# Exit status: 0 on success and when restore has nothing to do, 1 on a
# failure, 2 on a usage error.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

KEEP_ENTRIES=3
PART_BYTES="${RAIKIRI_GATE_CACHE_PART_BYTES:-134217728}"
# The directories of target/ that hold reusable artifacts.
PAYLOAD_DIRS=(llvm-cov-target debug)

log() {
  echo "gate-cache: $*"
}

# Temporary paths of the running command, removed on any exit. Parts that
# belong to an entry whose .info was never written are removed too.
CLEANUP_DIR=""
CLEANUP_FILES=()
CLEANUP_PARTS_PREFIX=""
cleanup() {
  if [[ -n "$CLEANUP_DIR" ]]; then
    rm -rf "$CLEANUP_DIR"
  fi
  if [[ ${#CLEANUP_FILES[@]} -gt 0 ]]; then
    rm -f "${CLEANUP_FILES[@]}"
  fi
  if [[ -n "$CLEANUP_PARTS_PREFIX" ]]; then
    rm -f "$CLEANUP_PARTS_PREFIX"*
  fi
}
trap cleanup EXIT

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
# First 12 hex digits of the SHA-256 of stdin (macOS ships shasum, not sha256sum).
short_hash() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum
  else
    shasum -a 256
  fi | cut -c1-12
}

toolchain_id() {
  {
    rustc -vV
    cargo llvm-cov --version 2>/dev/null || echo "no cargo-llvm-cov"
  } | short_hash
}

lock_id() {
  short_hash <Cargo.lock
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

# The "<part file> <bytes>" lines of an entry's .info, in archive order.
entry_parts() {
  sed -n 's/^part=//p' "$CACHE_DIR/$1.info"
}

# An entry is usable when every part its .info lists exists with the recorded
# size.
entry_is_valid() {
  local name="$1" file bytes actual found=0
  while read -r file bytes; do
    [[ -n "$file" ]] || continue
    case "$file" in
      */*) return 1 ;;
      "$name".part.*) ;;
      *) return 1 ;;
    esac
    found=1
    [[ -f "$CACHE_DIR/$file" ]] || return 1
    actual="$(wc -c <"$CACHE_DIR/$file" | tr -d ' ')"
    [[ "$actual" == "$bytes" ]] || return 1
  done < <(entry_parts "$name")
  [[ "$found" -eq 1 ]]
}

entry_bytes() {
  entry_parts "$1" | awk '{ sum += $2 } END { print sum + 0 }'
}

remove_entry() {
  rm -f "$CACHE_DIR/$1".part.* "$CACHE_DIR/$1.tar.gz" "$CACHE_DIR/$1.info"
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
      size="$(human_size "$(entry_bytes "$name")") in $(entry_parts "$name" | wc -l | tr -d ' ') parts"
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
    log "no usable entry for this toolchain (tc$toolchain) in $CACHE_DIR; building from scratch."
    log "(scripts/gate-cache.sh status lists the entries and flags incomplete ones)"
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
  CLEANUP_DIR="$staging"
  local started="$SECONDS" parts=() file bytes
  while read -r file bytes; do
    parts+=("$CACHE_DIR/$file")
  done < <(entry_parts "$name")
  if ! cat "${parts[@]}" | tar -C "$staging" -xzf -; then
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

  local toolchain lock stamp uniq name probe tmp_info members=() dir
  toolchain="$(toolchain_id)"
  lock="$(lock_id)"
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  # Random id: two saves that start in the same second must not share part
  # files (split reuses one name space per prefix) or an .info.
  uniq="$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')"
  name="gate-target-${stamp}-${uniq}-tc${toolchain}-lock${lock}"
  probe="$CACHE_DIR/.tmp.$$.$uniq.probe"
  tmp_info="$CACHE_DIR/.tmp.$$.$uniq.info"
  CLEANUP_FILES=("$probe" "$tmp_info")
  # Prove the cache is writable before removing anything from the checkout.
  : >"$probe" || die "cannot write to $CACHE_DIR; nothing was removed."

  log "removing the workspace crates' artifacts"
  cargo llvm-cov clean --workspace
  local clean_args=() package
  for package in "${WORKSPACE_PACKAGES[@]}"; do
    clean_args+=(-p "$package")
  done
  if [[ ${#clean_args[@]} -gt 0 ]]; then
    cargo clean --locked "${clean_args[@]}"
  fi

  for dir in "${PAYLOAD_DIRS[@]}"; do
    if [[ -d "$TARGET_DIR/$dir" ]]; then
      members+=("$dir")
    fi
  done
  log "packing ${members[*]} from $TARGET_DIR"
  local started="$SECONDS"
  # Incremental directories only hold workspace-crate state, which was just
  # removed, and are large. The parts are written under their final names (no
  # rename of a file that is still being uploaded); they only count once the
  # .info below exists.
  CLEANUP_PARTS_PREFIX="$CACHE_DIR/$name.part."
  tar -C "$TARGET_DIR" --exclude='debug/incremental' -cf - "${members[@]}" |
    gzip -1 | split -b "$PART_BYTES" - "$CLEANUP_PARTS_PREFIX"
  local part part_files=() bytes total=0
  for part in "$CLEANUP_PARTS_PREFIX"*; do
    if [[ -f "$part" ]]; then
      part_files+=("$part")
    fi
  done
  if [[ ${#part_files[@]} -eq 0 ]]; then
    die "nothing was packed from $TARGET_DIR."
  fi
  for part in "${part_files[@]}"; do
    bytes="$(wc -c <"$part" | tr -d ' ')"
    total=$((total + bytes))
  done
  # The .info file marks the entry complete, so it goes last.
  {
    echo "bytes=$total"
    echo "created=$stamp"
    echo "rustc=$(rustc -vV | sed -n 's/^release: //p')"
    echo "cargo_llvm_cov=$(cargo llvm-cov --version | head -1)"
    for part in "${part_files[@]}"; do
      echo "part=$(basename "$part") $(wc -c <"$part" | tr -d ' ')"
    done
  } >"$tmp_info"
  mv "$tmp_info" "$CACHE_DIR/$name.info"
  CLEANUP_PARTS_PREFIX=""
  log "saved $name ($(human_size "$total") in ${#part_files[@]} parts) in $((SECONDS - started))s"

  prune
}

# Keeps the newest KEEP_ENTRIES usable entries. An entry that fails the check
# may still be uploading, so it is only dropped after a day. Also removes the
# leftovers of savers that died before finishing.
prune() {
  local kept=0 name
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    if entry_is_valid "$name"; then
      kept=$((kept + 1))
      if [[ "$kept" -le "$KEEP_ENTRIES" ]]; then
        continue
      fi
    elif [[ -z "$(find "$CACHE_DIR/$name.info" -mmin +1440 2>/dev/null)" ]]; then
      continue
    fi
    log "pruning $name"
    remove_entry "$name"
  done < <(list_entries)
  find "$CACHE_DIR" -maxdepth 1 -name '.tmp.*' -mmin +180 -exec rm -f {} + 2>/dev/null || true
  # Parts whose .info never arrived (the saver died first) are not an entry.
  local part
  for part in "$CACHE_DIR"/gate-target-*.part.*; do
    [[ -e "$part" ]] || continue
    if [[ ! -e "${part%.part.*}.info" && -n "$(find "$part" -mmin +60 2>/dev/null)" ]]; then
      rm -f "$part"
    fi
  done
}

case "$COMMAND" in
  restore) do_restore ;;
  save) do_save ;;
  status) do_status ;;
esac
