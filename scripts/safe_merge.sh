#!/usr/bin/env bash
# scripts/safe_merge.sh — refuse `git merge` unless the Merge 直前 checklist
# (rules/gate.md §Gate 通過条件 (合成)) is already recorded, before the merge
# runs.
#
# gate.md's checklist (accepted via bd raikiri-spike-j2p6) asks whoever runs
# the merge to recall, immediately before running `git merge`, 4 dedicated
# lines covering §8.1 / §8.2 / §8.3 / §8.1.1 evidence. Measured across
# several sprints after that text landed, the exact 4-line form appeared in
# 1 of 22+ merges checked (the rest either had none of the 4 lines, or folded
# §8.1.1's patch-coverage disposition into the §8.1 line's own text instead
# of itemizing it separately) — a prose reminder to "remember to write 4
# lines" does not reliably produce the 4 lines. This script does not replace
# gate.md's checklist text; it makes the checklist a precondition the merge
# command itself enforces: `git merge` never runs unless the message already
# has all 4 lines, checked *before* the `git merge` command executes, not
# after.
#
# This script does NOT re-run the §8.1 checks themselves (that is
# scripts/gate.sh's job, already covering §8.1 (a)(b)(c) plus the §8.1.4
# applicability predicate) — it only verifies that the checklist recording
# gate.md requires exists and is non-vacuous. Composing the two: run
# scripts/gate.sh first to actually pass §8.1, then use this script's
# validated message to record that (and §8.2/§8.3/§8.1.1) and perform the
# merge in one step.
#
# Usage:
#   scripts/safe_merge.sh <branch> -F <message-file> [--evidence-file <file>] [--check-only] [-- <extra `git merge` args>]
#   scripts/safe_merge.sh <branch> -m <message>       [--evidence-file <file>] [--check-only] [-- <extra `git merge` args>]
#
#   <branch>              Branch (or ref) to merge into the current branch —
#                         same first positional argument `git merge` itself
#                         takes.
#   -F <message-file>     Path to a file containing the full merge commit
#                         message (`-` reads the message from stdin). Same
#                         semantics as `git commit -F`.
#   -m <message>          Merge commit message given directly on the command
#                         line. Same semantics as `git commit -m`.
#
#                         `-F -` (stdin, e.g. via a heredoc) and `-m` are the
#                         forms that avoid ever staging a gate-evidence
#                         commit message through a scratchpad file first —
#                         `-F <path>` is provided for callers that already
#                         have the message in a file for another reason, not
#                         as an invitation to write one out first.
#   --evidence-file <f>   Optional. Validate the checklist against this
#                         file's content INSTEAD OF the merge message —
#                         for gate.md's alternate "issue comment" evidence
#                         location (e.g. `bd comments <id> --json` output,
#                         with the relevant comment's body extracted, saved
#                         to a file). Use the --json form, not a pretty-
#                         printed/terminal capture such as `bd show <id>
#                         --long` redirected to a file: bd's terminal
#                         renderer wraps long lines for display, and each
#                         checklist item's required content must stay on
#                         one physical line (see check_item below) --
#                         a wrap that lands between e.g. a §8.3 job id and
#                         its GATE PASS verdict would make a genuinely
#                         passing item look invalid. When --evidence-file
#                         is omitted, the merge message itself (-F/-m) is
#                         what gets validated, which is the common case:
#                         one text serves as both the evidence and the
#                         merge commit's own message.
#   --check-only          Validate and report, but do not run `git merge`.
#                         Exit status still reflects pass/fail. Useful to
#                         test a draft message before committing to it.
#   -- <extra args>       Only specific `git merge` options are forwarded to
#                         the underlying `git merge --no-ff` call; anything
#                         else after a literal `--` is refused. This is an
#                         allow-list, not a raw passthrough — see "About
#                         `-- <extra args>`" below for what is admitted and
#                         why.
#
# What "machine-checkable evidence" means here: the validated text must
# contain, each as its own dedicated line, all 4 checklist markers below —
# not folded into another item's line (this exact failure mode — patch-
# coverage detail present in the §8.1 line's own text but never itemized on
# its own §8.1.1 line — is what the sprints after j2p6 landed actually did).
# A marker may appear on exactly one line; if it appears on more than one,
# the item is invalid rather than picking one arbitrarily (checking two
# candidate lines independently and accepting a pass on either would let a
# stray earlier mention rescue a line that actually fails). Each marker
# line must also carry a minimal content signal so an empty "- §8.3:"
# cannot pass, and that content must be the first thing after the marker —
# a free-form description may follow it, but the disposition itself cannot
# be buried later in the line (a substring search anywhere in the line
# would let e.g. "- §8.1: FAIL, not pass yet" match on "pass"):
#
#   - §8.1:   PASS, GREEN, or non-applicable/N/A (the §8.1.4 skip
#             disposition), immediately after the marker
#   - §8.2:   a convergence mode marker ("(i)" / "(ii)") or "skip" (the
#             lens-matrix.md §発火 skip exception), immediately after the
#             marker
#   - §8.3:   a Codex job id (task-<id>-<id>) and a GATE PASS verdict
#             specifically, adjacent to each other in either order
#             immediately after the marker — a recorded GATE FAIL means
#             §8.3 is not yet satisfied (it is not accepted, the same way
#             a recorded "§8.1: FAIL" is not accepted by the §8.1 check
#             above), and a GATE PASS mentioned elsewhere in the line
#             (e.g. citing an earlier iteration) does not count
#   - §8.1.1: covered, cov:ignore, or non-applicable/N/A, immediately
#             after the marker
#
# About `-- <extra args>`: this is an allow-list, not a raw passthrough.
# Two `git merge` options were CONFIRMED (Round 3 review) to defeat the
# validation above when forwarded unchanged: `-- --ff` overrides the fixed
# `--no-ff` below, producing a fast-forward with no merge commit at all
# (the validated message is simply never used, since a fast-forward has no
# commit to attach it to); `-- --edit` reopens $GIT_EDITOR after
# validation, letting the checklist be edited or deleted from the message
# that actually lands. Only options that can neither suppress merge-commit
# creation, nor let the validated message be replaced post-validation, nor
# cause the merged branch's changes to not land in the tree, are admitted:
# `-S` / `--gpg-sign[=<keyid>]` / `--no-gpg-sign` (commit signing) and
# `--strategy=<s>` / `--strategy-option=<o>` (merge strategy selection).
# Each token in the extra args is checked independently, so value-taking
# options must use their `--flag=value` form (e.g. `--strategy=ort`, not
# `-s ort`) — accepting a bare `-s` would require deciding whether the
# next token is its argument or a new flag, and that ambiguity is refused
# here rather than guessed. Anything not on this list — including but not
# limited to `--ff`, `--ff-only`, `--squash`, `--no-commit`, `--edit`,
# `--no-edit`, `--abort`, `--continue`, `--quit`, `--log`, `--cleanup` —
# is refused before `git merge` ever runs.
#
# `--strategy=<s>` is a closed enumeration (`ort`, `recursive`, `resolve`,
# `octopus`, `subtree`), not `--strategy=.+`: git's own `ours` strategy
# (distinct from `-X ours`, a `--strategy-option` that only affects
# genuinely conflicting hunks under a real 3-way merge) discards the
# merged branch's tree changes entirely while still producing a 2-parent
# merge commit carrying the validated, fully-passing checklist message —
# satisfying every check above while the reviewed diff silently never
# lands. `ours` is deliberately excluded from the enumeration for exactly
# this reason.
#
# The admitted categories are deliberately minimal — only what callers
# actually needed at the time this allow-list was written — not an
# exhaustive enumeration of every `git merge` option that happens to satisfy
# the criteria above. Other options may also satisfy them (e.g.
# `--allow-unrelated-histories` neither suppresses commit creation, touches
# the message, nor drops tree changes); they are refused here simply
# because nothing has needed them yet. Adding an option later must be
# argued against the criteria above (does it suppress merge-commit
# creation, let the validated message be replaced/appended-to
# post-validation, or cause the merged branch's changes to not land?), not
# added on convenience — and routing around this script instead of
# extending it is never the answer.
#
# This is a floor, not the full record: it verifies the checklist's 4 lines
# exist and are non-vacuous, not that every sub-clause gate.md's §Gate 通過
# 条件 (合成) requires is satisfied in full detail (per-remit citations,
# convergence-mode rationale for case (ii), etc. remain a human/coordinator
# judgment call, same as before this script existed — the same limitation
# scripts/gate.sh's own header states about its own §8.1.4 condition (2)).
#
# Exit status: 0, and `git merge --no-ff` runs (unless --check-only), only
# if all 4 markers are present and pass their content check. Non-zero, and
# no `git merge` command is ever invoked, otherwise.

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
ALLOWED_EXTRA_ARG_RE='^(-S|--gpg-sign|--gpg-sign=.+|--no-gpg-sign|--strategy=(ort|recursive|resolve|octopus|subtree)|--strategy-option=.+)$'
for arg in "${EXTRA_ARGS[@]}"; do
  if ! [[ "$arg" =~ $ALLOWED_EXTRA_ARG_RE ]]; then
    echo "safe_merge.sh: extra arg not on the allow-list: $arg" >&2
    echo "  Allowed: -S, --gpg-sign[=<keyid>], --no-gpg-sign, --strategy=(ort|recursive|resolve|octopus|subtree), --strategy-option=<o>." >&2
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
# LINE_REGEX picks out the dedicated checklist line for one item (anchored
# so e.g. "§8.1:" cannot accidentally match a "§8.1.1:" line). VALUE_REGEX
# is matched, case-insensitively, against the text after the marker on
# that line, leading whitespace trimmed, anchored to the start of that
# text. See "What machine-checkable evidence means here" in the file
# header above for why a match must land on exactly one line and be
# anchored at the start rather than found as a substring anywhere in the
# line.
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

echo "-- Merge 直前 checklist validation (rules/gate.md §Gate通過条件(合成)) --"
check_item "§8.1" \
  '^[[:space:]]*- §8\.1: ?' \
  "expected PASS/GREEN or non-applicable/N/A immediately after the marker" \
  '(pass\b|green\b|non-applicable\b|n/a\b)'
check_item "§8.2" \
  '^[[:space:]]*- §8\.2: ?' \
  "expected convergence mode (i)/(ii) or skip immediately after the marker" \
  '(\(i\)|\(ii\)|skip\b)'
# §8.3 requires a job id and a GATE PASS verdict adjacent to each other,
# right after the marker (see the file header above for why "GATE FAIL"
# doesn't count and why the two tokens must be adjacent rather than each
# independently matched anywhere in the line) — e.g. "task-abc-123 GATE
# FAIL (see task-abc-124 GATE PASS in the retry log)" contains both an id
# pattern and the literal text "gate pass" somewhere, but is not itself a
# passing verdict.
check_item "§8.3" \
  '^[[:space:]]*- §8\.3: ?' \
  "expected a Codex job id (task-<id>-<id>) and a GATE PASS verdict adjacent to each other, immediately after the marker" \
  '(task-[a-z0-9]+-[a-z0-9]+[[:space:]]+gate[[:space:]]+pass\b|gate[[:space:]]+pass[[:space:]]+task-[a-z0-9]+-[a-z0-9]+\b)'
check_item "§8.1.1" \
  '^[[:space:]]*- §8\.1\.1: ?' \
  "expected covered/cov:ignore or non-applicable/N/A immediately after the marker" \
  '(covered\b|cov:ignore\b|non-applicable\b|n/a\b)'

echo
if [[ "${#MISSING[@]}" -gt 0 ]]; then
  echo "== safe_merge.sh: REFUSING to merge =="
  echo "Missing/invalid Merge 直前 checklist items:"
  for m in "${MISSING[@]}"; do
    echo "  - $m"
  done
  echo
  echo "Add the missing line(s) to $EVIDENCE_SOURCE and re-run."
  echo "No git merge command has been executed."
  exit 1
fi

echo "== safe_merge.sh: all 4 checklist items present =="

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
