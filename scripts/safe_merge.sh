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
# has all 4 lines, checked *before* any git command executes, not after.
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
#                         location (e.g. `bd show <id> --long` output saved
#                         to a file). When omitted, the merge message itself
#                         (-F/-m) is what gets validated, which is the common
#                         case: one text serves as both the evidence and the
#                         merge commit's own message.
#   --check-only          Validate and report, but do not run `git merge`.
#                         Exit status still reflects pass/fail. Useful to
#                         test a draft message before committing to it.
#   -- <extra args>       Anything after a literal `--` is passed through to
#                         the underlying `git merge --no-ff` call unchanged.
#
# What "machine-checkable evidence" means here: the validated text must
# contain, each as its own dedicated line, all 4 checklist markers below —
# not folded into another item's line (this exact failure mode — patch-
# coverage detail present in the §8.1 line's own text but never itemized on
# its own §8.1.1 line — is what the sprints after j2p6 landed actually did).
# Each marker line must also carry a minimal content signal so an empty
# "- §8.3:" cannot pass:
#
#   - §8.1:   PASS, GREEN, or non-applicable/N/A (the §8.1.4 skip
#             disposition)
#   - §8.2:   a convergence mode marker ("(i)" / "(ii)") or "skip" (the
#             lens-matrix.md §発火 skip exception)
#   - §8.3:   a Codex job id (task-<id>-<id>) AND a GATE PASS verdict
#             specifically — a recorded GATE FAIL means §8.3 is not yet
#             satisfied (it is not accepted, the same way a recorded
#             "§8.1: FAIL" is not accepted by the §8.1 check above)
#   - §8.1.1: covered, cov:ignore, or non-applicable/N/A
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
# no git command is ever invoked, otherwise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve the repo root from the caller's shell, not from this script's own
# on-disk location — see scripts/gate.sh's header for the full rationale
# (running one checkout's absolute scripts/safe_merge.sh path while the
# shell's cwd is a different git worktree must not silently merge branches
# in the wrong tree).
REPO_ROOT="$(git rev-parse --show-toplevel)"

SCRIPT_TREE="$(cd "$SCRIPT_DIR/.." && git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -n "$SCRIPT_TREE" && "$SCRIPT_TREE" != "$REPO_ROOT" ]]; then
  echo "safe_merge.sh: refusing to run." >&2
  echo "  The script file you invoked lives in a different git tree than" >&2
  echo "  your shell's current directory:" >&2
  echo "    invoked script's tree     : $SCRIPT_TREE" >&2
  echo "    current directory's tree  : $REPO_ROOT" >&2
  echo "  Invoke the safe_merge.sh that belongs to the tree you want" >&2
  echo "  checked (e.g. cd there and run ./scripts/safe_merge.sh) instead of" >&2
  echo "  another checkout's absolute path." >&2
  exit 2
fi

cd "$REPO_ROOT"

# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

usage() {
  sed -n '2,86p' "${BASH_SOURCE[0]}"
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

# check_item LABEL LINE_REGEX HINT CONTENT_REGEX...
# LINE_REGEX picks out the dedicated checklist line (anchored so e.g.
# "§8.1:" cannot accidentally match a "§8.1.1:" line — the exact folding
# failure this script exists to catch). Every CONTENT_REGEX after HINT is
# applied independently to that line, case-insensitively, and ALL of them
# must match (AND, not OR) for the item to pass — e.g. §8.3 requires both a
# job id pattern and a GATE PASS verdict on the same line, not either one.
check_item() {
  local label="$1" line_re="$2" hint="$3"
  shift 3
  local content_res=("$@")
  local line
  line="$(printf '%s\n' "$EVIDENCE_TEXT" | grep -E "$line_re" || true)"
  if [[ -z "$line" ]]; then
    echo "[MISSING] $label: no line matching /$line_re/ found."
    MISSING+=("$label -- no dedicated line found")
    return
  fi
  local req
  for req in "${content_res[@]}"; do
    if ! printf '%s\n' "$line" | grep -qiE "$req"; then
      echo "[INVALID] $label: line found but missing required content ($hint):"
      echo "    $line"
      MISSING+=("$label -- line present but content check failed ($hint)")
      return
    fi
  done
  echo "[OK] $label:"
  echo "    $line"
}

echo "-- Merge 直前 checklist validation (rules/gate.md §Gate通過条件(合成)) --"
check_item "§8.1" \
  '^[[:space:]]*- §8\.1: ?' \
  "expected PASS/GREEN or non-applicable/N/A" \
  '(pass|green|non-applicable|n/a)'
check_item "§8.2" \
  '^[[:space:]]*- §8\.2: ?' \
  "expected convergence mode (i)/(ii) or skip" \
  '(\(i\)|\(ii\)|skip)'
# §8.3 requires BOTH a job id and a GATE PASS verdict specifically — a
# recorded GATE FAIL means §8.3 (gate.md §Gate 通過条件 (合成) condition 3,
# "§8.3 codex 最終レビュー完了") is not yet satisfied, so it must not be
# accepted as passing evidence any more than a recorded "§8.1: FAIL" is.
check_item "§8.3" \
  '^[[:space:]]*- §8\.3: ?' \
  "expected a Codex job id (task-<id>-<id>) and a GATE PASS verdict" \
  'task-[a-z0-9]+-[a-z0-9]+' \
  'gate pass'
check_item "§8.1.1" \
  '^[[:space:]]*- §8\.1\.1: ?' \
  "expected covered/cov:ignore or non-applicable/N/A" \
  '(covered|cov:ignore|non-applicable|n/a)'

echo
if [[ "${#MISSING[@]}" -gt 0 ]]; then
  echo "== safe_merge.sh: REFUSING to merge =="
  echo "Missing/invalid Merge 直前 checklist items:"
  for m in "${MISSING[@]}"; do
    echo "  - $m"
  done
  echo
  echo "Add the missing line(s) to $EVIDENCE_SOURCE and re-run."
  echo "No git command has been executed."
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
