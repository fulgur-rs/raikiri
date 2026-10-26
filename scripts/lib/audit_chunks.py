#!/usr/bin/env python3
"""scripts/lib/audit_chunks.py — run audit-vacuous-passes over many ids in isolated chunks.

`scripts/wpt/vacuity-sweep.sh` audits every runnable reftest that no expectations
file lists, about twelve thousand of them. One `audit-vacuous-passes` process
over all of them is lost to the first test that exhausts memory or never
finishes: the report is written only when the whole run ends, so the weekly job
dies without a single row. This driver gives each chunk of ids a process of its
own and watches that process:

  * a process whose resident memory passes the limit is killed;
  * a process that prints no row for the stall time is killed;
  * a process that exits non-zero is treated as failed.

The rows a failed chunk had already printed are kept. The ids it had not
finished are run again, split in halves while the failed run made no progress,
until each failing id stands alone; that id gets a `CRASH` row naming the reason.
Every id therefore ends with a row, and a test that cannot be rendered costs the
sweep that one id instead of the run.

Usage:

    python3 scripts/lib/audit_chunks.py --binary target/release/audit-vacuous-passes \\
      --wpt-root target/wpt --ids candidates.txt --output audit.tsv [--jobs N]

The output has the format of `audit-vacuous-passes --output`: one
`id<TAB>VERDICT<TAB>detail` row per id, in the order of the ids file.

The resident-memory limit needs `/proc`; elsewhere only the stall time and the
exit status apply.

Exit status: 0 when every id has a row, 2 on bad input.
"""

import argparse
import os
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter
from dataclasses import dataclass, field

CRASH = "CRASH"
VERDICT = re.compile(r"[A-Z][A-Z-]*")


@dataclass
class Limits:
    jobs: int
    chunk_size: int = 400
    # A test peaks near 300 MB; this leaves room for every job to hold one.
    max_rss_mb: int = 4096
    # The slowest audit seen takes five minutes on a single id, so a process
    # is only taken for stuck when it has been silent for longer than that.
    stall_seconds: float = 600.0
    poll_seconds: float = 0.25


@dataclass
class ChunkResult:
    """What one process reported: the rows it finished and why it was stopped."""

    rows: dict[str, str] = field(default_factory=dict)
    failure: str | None = None


def parse_rows(lines, wanted):
    """The `id -> "VERDICT<TAB>detail"` rows among `lines`.

    The binary prints one row per finished test to stderr, among progress and
    panic lines. A line counts when it starts with a wanted id, and the second
    field looks like a verdict. A detail that continues on further lines keeps
    only its first line here; the report file holds it whole.
    """
    rows = {}
    for line in lines:
        fields = line.rstrip("\n").split("\t", 2)
        if len(fields) == 3 and fields[0] in wanted and VERDICT.fullmatch(fields[1]):
            rows[fields[0]] = f"{fields[1]}\t{fields[2]}"
    return rows


def rss_mb(pid):
    """Resident memory of `pid` in MB, or None where `/proc` has no such entry."""
    try:
        with open(f"/proc/{pid}/status", encoding="ascii", errors="replace") as status:
            for line in status:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def describe_exit(code):
    if code < 0:
        try:
            return f"killed by {signal.Signals(-code).name}"
        except ValueError:
            return f"killed by signal {-code}"
    return f"exit status {code}"


def run_chunk(command, ids, limits):
    """Audit `ids` in one process built by `command(ids_file, output_file)`."""
    wanted = set(ids)
    result = ChunkResult()
    lock = threading.Lock()
    progress = [time.monotonic()]
    with tempfile.TemporaryDirectory(prefix="audit-chunk-") as directory:
        ids_file = os.path.join(directory, "ids.txt")
        report = os.path.join(directory, "audit.tsv")
        with open(ids_file, "w", encoding="utf-8") as handle:
            handle.writelines(f"{test_id}\n" for test_id in ids)
        process = subprocess.Popen(
            command(ids_file, report),
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
        )

        def forward():
            # Forward the binary's own log, so the job log shows each test.
            for line in process.stderr:
                sys.stderr.write(line)
                sys.stderr.flush()
                row = parse_rows([line], wanted)
                with lock:
                    result.rows.update(row)
                    if row:
                        progress[0] = time.monotonic()

        reader = threading.Thread(target=forward, daemon=True)
        reader.start()
        while process.poll() is None:
            time.sleep(limits.poll_seconds)
            memory = rss_mb(process.pid)
            with lock:
                idle = time.monotonic() - progress[0]
            if memory is not None and memory > limits.max_rss_mb:
                result.failure = (
                    f"memory limit exceeded ({memory:.0f} MB resident, limit {limits.max_rss_mb} MB)"
                )
            elif idle > limits.stall_seconds:
                result.failure = f"no result for {limits.stall_seconds:g} s"
            if result.failure:
                process.kill()
                break
        process.wait()
        reader.join()
        if result.failure is None and process.returncode != 0:
            result.failure = describe_exit(process.returncode)
        if result.failure is None:
            try:
                with open(report, encoding="utf-8") as handle:
                    # The report file holds every detail whole, unlike stderr.
                    result.rows.update(parse_rows(handle, wanted))
            except OSError:
                result.failure = "the audit wrote no report"
    return result


def audit(ids, run, limits):
    """Rows for every id of `ids`, in `limits.chunk_size` chunks run by `run`."""
    rows = {}
    pending = [ids[start : start + limits.chunk_size] for start in range(0, len(ids), limits.chunk_size)]
    while pending:
        chunk = pending.pop(0)
        result = run(chunk)
        rows.update(result.rows)
        unfinished = [test_id for test_id in chunk if test_id not in rows]
        if not unfinished:
            continue
        reason = result.failure or "the audit reported no row"
        if len(unfinished) == 1:
            rows[unfinished[0]] = f"{CRASH}\t{reason}"
            print(f"{unfinished[0]}: {reason}", file=sys.stderr)
        elif result.rows:
            # The ids that were running when the process was stopped come
            # first in the retry, so a failing id is found in a round or two.
            pending.insert(0, unfinished)
        else:
            middle = len(unfinished) // 2
            pending[0:0] = [unfinished[:middle], unfinished[middle:]]
    return rows


def read_ids(path):
    with open(path, encoding="utf-8") as handle:
        return list(dict.fromkeys(line.strip() for line in handle if line.strip()))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", required=True, help="the audit-vacuous-passes executable")
    parser.add_argument("--wpt-root", required=True)
    parser.add_argument("--ids", required=True, help="file with one test id per line")
    parser.add_argument("--output", required=True)
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    parser.add_argument("--chunk-size", type=int, default=Limits.chunk_size)
    parser.add_argument("--max-rss-mb", type=int, default=Limits.max_rss_mb)
    parser.add_argument("--stall-seconds", type=float, default=Limits.stall_seconds)
    parser.add_argument("--poll-seconds", type=float, default=Limits.poll_seconds, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if min(args.jobs, args.chunk_size, args.max_rss_mb) < 1 or min(args.stall_seconds, args.poll_seconds) <= 0:
        parser.error("--jobs, --chunk-size, --max-rss-mb, --stall-seconds must be positive")
    limits = Limits(
        jobs=args.jobs,
        chunk_size=args.chunk_size,
        max_rss_mb=args.max_rss_mb,
        stall_seconds=args.stall_seconds,
        poll_seconds=args.poll_seconds,
    )
    try:
        ids = read_ids(args.ids)
    except OSError as error:
        print(f"error: read {args.ids}: {error}", file=sys.stderr)
        return 2

    def command(ids_file, report):
        return [
            args.binary,
            "--wpt-root", args.wpt_root,
            "--ids", ids_file,
            "--output", report,
            "--jobs", str(limits.jobs),
        ]

    try:
        rows = audit(ids, lambda chunk: run_chunk(command, chunk, limits), limits)
    except OSError as error:
        print(f"error: cannot run {args.binary}: {error}", file=sys.stderr)
        return 2
    os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
    with open(args.output, "w", encoding="utf-8") as handle:
        handle.writelines(f"{test_id}\t{rows[test_id].replace(chr(10), ' ')}\n" for test_id in ids)
    counts = Counter(rows[test_id].split("\t", 1)[0] for test_id in ids)
    summary = ", ".join(f"{verdict} {count}" for verdict, count in sorted(counts.items()))
    print(f"-- {summary} ({len(ids)} total) --", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
