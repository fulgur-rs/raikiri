#!/usr/bin/env python3
"""Compare fresh and reused workers with a fresh realm and Document per page."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time

from bench import check_result, workload


def invoke(binary, engine, mode, script, pages, cpu):
    command = [str(binary), "--backend", engine, "--worker", mode, "--repeat", str(pages)]
    if script is not None:
        command.extend(["--script", str(script)])
    if cpu is not None:
        command = ["taskset", "--cpu-list", str(cpu), *command]
    started = time.perf_counter_ns()
    result = subprocess.run(command, capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise RuntimeError(f"{engine}/{mode} exited {result.returncode}: {result.stderr}")
    return json.loads(result.stdout), (time.perf_counter_ns() - started) / 1e6


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).parent / "target/release/webidl-dual-engine-spike")
    parser.add_argument("--samples", type=int, default=7)
    parser.add_argument("--cpu", type=int)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.samples < 3:
        parser.error("at least three samples are required")
    binary = args.binary.resolve()
    cases = [("empty", 0, 500), ("contract", 42, 100), ("js", 1_000_000, 20),
             ("getter", 100_000, 20), ("attributes", 25_000, 20)]
    report = {
        "environment": {"platform": platform.platform(), "python": platform.python_version(),
                        "cpu_affinity": args.cpu, "samples": args.samples,
                        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                        "load_before": os.getloadavg()},
        "method": "Fresh process per batch. Each page gets a fresh realm and Document in both modes. Rotate the four engine/mode combinations each sample; first round discarded. Loops warm 3x1000 per page. elapsed_ms includes worker initialization, all pages, report collection and worker destruction, excluding file IO/process startup/stdout. page elapsed excludes worker initialization/destruction and report collection. wall includes process/taskset/IO/stdout. All pages checked.",
        "cases": [],
    }
    combinations = [("boa", "fresh"), ("v8", "fresh"), ("boa", "reuse"), ("v8", "reuse")]
    tmp_root = Path(os.environ.get("TMPDIR", str(Path.home() / "tmp")))
    tmp_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="webidl-worker-perf-", dir=tmp_root) as scratch:
        for name, count, pages in cases:
            source, expected, mutations = ('JSON.stringify({checksum:0})', 0, 0) if name == "empty" else (None, None, None)
            if name not in ("empty", "contract"):
                source, expected, mutations = workload(name, count)
            path = None
            if source is not None:
                path = Path(scratch) / f"{name}-{count}.js"
                path.write_text(source, encoding="utf-8")
            case = {"name": name, "iterations": count, "pages": pages,
                    "runs": {f"{engine}/{mode}": [] for engine, mode in combinations}}
            contract_mutations = None
            for sample in range(args.samples + 1):
                offset = sample % len(combinations)
                for engine, mode in combinations[offset:] + combinations[:offset]:
                    payload, wall_ms = invoke(binary, engine, mode, path, pages, args.cpu)
                    if (payload["engine"] != engine or payload["worker"] != mode or
                            payload["worker_initializations"] != (1 if mode == "reuse" else pages) or
                            len(payload["pages"]) != pages):
                        raise RuntimeError("incorrect batch shape or worker count")
                    for page in payload["pages"]:
                        if "error" in page:
                            raise RuntimeError(f"page failure: {page}")
                        check_result(name, page, expected, mutations)
                        if page["job_callbacks"] != 0:
                            raise RuntimeError("unexpected job callback")
                        if name == "contract":
                            if contract_mutations is None:
                                contract_mutations = page["mutation_count"]
                            if page["mutation_count"] != contract_mutations:
                                raise RuntimeError("contract mutation count changed between pages")
                    if sample:
                        case["runs"][f"{engine}/{mode}"].append({
                            "elapsed_ms": payload["elapsed_ms"], "wall_ms": wall_ms,
                            "worker_init_ms": payload["worker_init_ms"],
                            "page_elapsed_ms": [page["elapsed_ms"] for page in payload["pages"]]})
            for label, samples in case["runs"].items():
                values = [sample["elapsed_ms"] / pages for sample in samples]
                print(f"{name:12} {pages:4} pages {label:9} median={statistics.median(values):8.3f} ms/page range={min(values):.3f}..{max(values):.3f}", flush=True)
            report["cases"].append(case)
    report["environment"]["load_after"] = os.getloadavg()
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
