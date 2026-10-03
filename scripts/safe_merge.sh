#!/usr/bin/env bash
# Refuse to merge unless validation, independent review, and patch-coverage
# evidence are recorded in a machine-checkable form.
#
# Usage:
#   scripts/safe_merge.sh <branch> -F <message-file> [--evidence-file <file>] [--check-only] [-- <extra git merge args>]
#   scripts/safe_merge.sh <branch> -m <message>       [--evidence-file <file>] [--check-only] [-- <extra git merge args>]
#
# Required evidence lines, each appearing exactly once:
#   - Validation: PASS
#   - Independent review: PASS
#   - Patch coverage: PASS or N/A
#
# A disposition must appear immediately after its marker. Additional context
# may follow on that line. The evidence can be included in the merge message,
# or supplied separately with --evidence-file.
#
#   <branch>            Branch or ref to merge into the current branch.
#   -F <message-file>   Read the full merge message from a file; `-` reads stdin.
#   -m <message>        Use the supplied merge message.
#   --evidence-file     Validate a separate plain-text evidence record instead
#                       of the merge message.
#   --check-only        Validate without running git merge.
#   -- <extra args>     Forward only the options documented below.
#
# Extra git merge options are allow-listed. This prevents disabling merge
# commit creation, changing the validated message after validation, or
# selecting a strategy that drops the incoming tree changes. Value-taking
# options must use their `--flag=value` form. The accepted options are -S,
# --gpg-sign[=<keyid>], --no-gpg-sign, the enumerated --strategy values, and
# the enumerated --strategy-option values handled by ALLOWED_EXTRA_ARG_RE.
#
# Exit status is 0 only when all three evidence lines pass validation and,
# unless --check-only is set, git merge --no-ff succeeds.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve REPO_ROOT from the caller's shell cwd (not from this script's own
# on-disk location) and hard-fail on a cross-tree mismatch — see
# scripts/lib/repo_root.sh for the full rationale. A prior hand-rolled
# version of this guard here used `git rev-parse --show-toplevel ... ||
# true` and treated a resolution failure the same as "no mismatch", which
# let an unresolvable tree silently pass instead of refusing to run. The
# shared lib's primary check (REPO_ROOT itself, from the caller's cwd) fixes
# that for this script's actual working directory; its secondary check
# (whether the invoked script *file* sits in a different tree) still has an
# analogous fail-open gap of its own when that tree is unresolvable rather
# than merely different — a pre-existing property of the shared lib, not
# something this switch introduces or closes.
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

cd "$REPO_ROOT"

# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

usage() {
  # Print the header comment block (lines 2..N, i.e. everything up to but
  # not including the first non-comment line) rather than a hand-maintained
  # line-number range: the header has already needed its end bumped by hand
  # twice as it grew, and a stale range fails silently in both directions
  # (too small truncates the very options docs; too large leaks shell code
  # like `set -euo pipefail` into --help output).
  awk 'NR==1 { next } /^#/ { print; next } { exit }' "${BASH_SOURCE[0]}"
}

if [[ $# -eq 0 ]]; then
  usage
  exit 2
fi

if [[ "$1" == "-h" || "$1" == "--help" ]]; then
  usage
  exit 0
fi

BRANCH="$1"
shift

# Reject a branch argument that could be parsed as a `git merge` option
# instead of a ref name. $BRANCH reaches the eventual `git merge --no-ff
# "$BRANCH" ...` call without an intervening `--`, so a value starting with
# `-` (e.g. a typo'd or attacker-controlled ref) would otherwise be handed
# to git as a flag rather than a branch name. `-- <extra args>` remains the
# one sanctioned way to pass option-shaped values through to `git merge`.
if [[ "$BRANCH" == -* ]]; then
  echo "safe_merge.sh: branch name must not start with '-': $BRANCH" >&2
  echo "  git merge would parse this as an option, not a ref name." >&2
  exit 2
fi

MSG_FILE=""
MSG_TEXT=""
EVIDENCE_FILE=""
CHECK_ONLY=0
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -F)
      MSG_FILE="${2:?safe_merge.sh: -F requires a file argument}"
      shift 2
      ;;
    -m)
      MSG_TEXT="${2:?safe_merge.sh: -m requires a message argument}"
      shift 2
      ;;
    --evidence-file)
      EVIDENCE_FILE="${2:?safe_merge.sh: --evidence-file requires a file argument}"
      shift 2
      ;;
    --check-only)
      CHECK_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      EXTRA_ARGS=("$@")
      break
      ;;
    *)
      echo "safe_merge.sh: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# `-- <extra args>` allow-list (see "About `-- <extra args>`" in the file
# header for the CONFIRMED --ff / --edit bypasses this closes, and why
# value-taking options require their `--flag=value` form here).
ALLOWED_EXTRA_ARG_RE='^(-S|--gpg-sign|--gpg-sign=.+|--no-gpg-sign|--strategy=(ort|recursive|resolve|octopus|subtree)|--strategy-option=(ours|theirs|patience|histogram|diff-algorithm=(patience|histogram|minimal|myers)|renormalize|no-renormalize|find-renames|find-renames=[0-9]+|no-renames|subtree|subtree=.+|rename-threshold=[0-9]+))$'
for arg in "${EXTRA_ARGS[@]}"; do
  if ! [[ "$arg" =~ $ALLOWED_EXTRA_ARG_RE ]]; then
    echo "safe_merge.sh: extra arg not on the allow-list: $arg" >&2
    echo "  Allowed: -S, --gpg-sign[=<keyid>], --no-gpg-sign, --strategy=(ort|recursive|resolve|octopus|subtree), --strategy-option=(see --help for the full enumeration)." >&2
    echo "  Anything that could suppress the merge commit or replace the validated message is refused (see --help)." >&2
    exit 2
  fi
done

if [[ -z "$MSG_FILE" && -z "$MSG_TEXT" ]]; then
  echo "safe_merge.sh: one of -F <file> or -m <message> is required." >&2
  exit 2
fi
if [[ -n "$MSG_FILE" && -n "$MSG_TEXT" ]]; then
  echo "safe_merge.sh: pass only one of -F or -m, not both." >&2
  exit 2
fi

read_file_or_stdin() {
  local path="$1"
  if [[ "$path" == "-" ]]; then
    cat
  else
    if [[ ! -f "$path" ]]; then
      echo "safe_merge.sh: file not found: $path" >&2
      exit 2
    fi
    cat "$path"
  fi
}

if [[ -n "$MSG_FILE" ]]; then
  MESSAGE="$(read_file_or_stdin "$MSG_FILE")"
else
  MESSAGE="$MSG_TEXT"
fi

if [[ -n "$EVIDENCE_FILE" ]]; then
  EVIDENCE_TEXT="$(read_file_or_stdin "$EVIDENCE_FILE")"
  EVIDENCE_SOURCE="$EVIDENCE_FILE"
else
  EVIDENCE_TEXT="$MESSAGE"
  EVIDENCE_SOURCE="(merge commit message)"
fi

echo "== scripts/safe_merge.sh =="
echo "repo root       : $REPO_ROOT"
echo "branch          : $BRANCH"
echo "evidence source : $EVIDENCE_SOURCE"
echo

MISSING=()

# check_item LABEL LINE_REGEX HINT VALUE_REGEX
#
# LINE_REGEX picks out the dedicated evidence line for one item. VALUE_REGEX
# is matched, case-insensitively, against the text after the marker on
# that line, leading whitespace trimmed, anchored to the start of that
# text. A match must land on exactly one line and be anchored at the start
# rather than found as a substring anywhere in the line.
check_item() {
  local label="$1" line_re="$2" hint="$3" value_re="$4"
  local matches
  matches="$(printf '%s\n' "$EVIDENCE_TEXT" | grep -E "$line_re" || true)"
  if [[ -z "$matches" ]]; then
    echo "[MISSING] $label: no line matching /$line_re/ found."
    MISSING+=("$label -- no dedicated line found")
    return
  fi
  local match_count
  match_count="$(printf '%s\n' "$matches" | grep -c '^')"
  if [[ "$match_count" -gt 1 ]]; then
    echo "[INVALID] $label: multiple lines matched /$line_re/ -- exactly one dedicated line is required:"
    printf '%s\n' "$matches" | sed 's/^/    /'
    MISSING+=("$label -- multiple matching lines for one checklist item")
    return
  fi
  local line="$matches"
  local value
  value="$(printf '%s\n' "$line" | sed -E "s#$line_re##")"
  if ! printf '%s\n' "$value" | grep -qiE "^[[:space:]]*($value_re)"; then
    echo "[INVALID] $label: line found but value does not start with required content ($hint):"
    echo "    $line"
    MISSING+=("$label -- line present but content check failed ($hint)")
    return
  fi
  echo "[OK] $label:"
  echo "    $line"
}

echo "-- pre-merge evidence validation --"
check_item "Validation" \
  '^[[:space:]]*- Validation: ?' \
  "expected PASS immediately after the marker" \
  '(pass\b)'
check_item "Independent review" \
  '^[[:space:]]*- Independent review: ?' \
  "expected PASS immediately after the marker" \
  '(pass\b)'
check_item "Patch coverage" \
  '^[[:space:]]*- Patch coverage: ?' \
  "expected PASS or N/A immediately after the marker" \
  '(pass\b|n/a\b)'

echo
if [[ "${#MISSING[@]}" -gt 0 ]]; then
  echo "== safe_merge.sh: REFUSING to merge =="
  echo "Missing or invalid pre-merge evidence items:"
  for m in "${MISSING[@]}"; do
    echo "  - $m"
  done
  echo
  echo "Add the missing line(s) to $EVIDENCE_SOURCE and re-run."
  echo "No git merge command has been executed."
  exit 1
fi

echo "== safe_merge.sh: all 3 evidence items present =="

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  echo "--check-only passed: not running git merge."
  exit 0
fi

TMPMSG="$(mktemp "$TMPDIR/safe_merge_msg.XXXXXX")"
trap 'rm -f "$TMPMSG"' EXIT
printf '%s\n' "$MESSAGE" > "$TMPMSG"

if [[ "${#EXTRA_ARGS[@]}" -gt 0 ]]; then
  echo "Running: git merge --no-ff $BRANCH -F <message> ${EXTRA_ARGS[*]}"
else
  echo "Running: git merge --no-ff $BRANCH -F <message>"
fi
git merge --no-ff "$BRANCH" -F "$TMPMSG" "${EXTRA_ARGS[@]}"
