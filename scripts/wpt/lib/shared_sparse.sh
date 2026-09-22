#!/usr/bin/env bash
# Shared sparse-checkout helpers for scripts/wpt/fetch.sh.

validate_shared_wpt_subset() {
  local subset_file="$1"
  local -a expected=(css fonts images)
  local -a actual=()

  if [ ! -r "$subset_file" ]; then
    echo "error: cannot read shared WPT subset: $subset_file" >&2
    return 2
  fi
  mapfile -t actual < <(
    sed -E 's/^[[:space:]]*#.*$//; /^[[:space:]]*$/d' "$subset_file"
  )

  if [ "${#actual[@]}" -ne "${#expected[@]}" ]; then
    echo "error: $subset_file must contain exactly: css, fonts, images" >&2
    return 2
  fi
  local index
  for index in "${!expected[@]}"; do
    if [ "${actual[$index]}" != "${expected[$index]}" ]; then
      echo "error: $subset_file must contain exactly: css, fonts, images" >&2
      return 2
    fi
  done
  printf '%s\n' "${expected[@]}"
}

install_locked_shared_sparse_file() {
  local sparse_file="$1"
  shift
  local -a roots=("$@")
  local expected_content current_content directory temporary_file
  expected_content="$(printf '%s\n' "${roots[@]}")"
  current_content="$(cat "$sparse_file" 2>/dev/null || true)"

  if [ -e "$sparse_file" ] &&
    [ "$current_content" != "$expected_content" ] &&
    [ ! -w "$sparse_file" ]; then
    echo "error: $sparse_file is locked with unexpected roots; inspect it manually" >&2
    return 2
  fi

  directory="$(dirname "$sparse_file")"
  mkdir -p "$directory"
  temporary_file="$(mktemp "$directory/sparse-checkout.XXXXXX")"
  printf '%s\n' "${roots[@]}" > "$temporary_file"
  chmod 444 "$temporary_file"
  # Replace the inode atomically. A stale process that already opened the old
  # file can only write to the detached inode; reapply reads the locked path.
  mv -f -- "$temporary_file" "$sparse_file"
}
