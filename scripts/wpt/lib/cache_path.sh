#!/usr/bin/env bash
# Resolve the single physical WPT cache and link it into a worktree.

wpt_cache_dir() {
  printf '%s/.cache/raikiri/wpt\n' "${HOME:?HOME must be set}"
}

ensure_wpt_cache_link() {
  local repo_root="$1"
  local cache_dir="${2:-$(wpt_cache_dir)}"
  local local_wpt_dir="$repo_root/target/wpt"

  if [[ -e "$local_wpt_dir" && ! -L "$local_wpt_dir" ]]; then
    echo "error: $local_wpt_dir exists and is not a symlink; refusing to replace it." >&2
    echo "       Preserve or migrate any old checkout before linking $cache_dir." >&2
    return 2
  fi

  mkdir -p "$repo_root/target"
  ln -sfn -- "$cache_dir" "$local_wpt_dir"
}
