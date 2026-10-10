#!/usr/bin/env python3
"""Measure identical, checked workloads in fresh native engine processes."""

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


def workload(name, count):
    bodies = {
        "js": "for (let i = 0; i < n; i++) checksum = (checksum + (i & 1023)) | 0;",
        "getter": "for (let i = 0; i < n; i++) checksum += element.nodeType;",
        "attributes": '''for (let i = 0; i < n; i++) {
            element.setAttribute("data-final", "value");
            if (element.getAttribute("data-final") === "value") checksum++;
        }''',
        "coercion": '''const value = {toString() { return "value"; }};
        for (let i = 0; i < n; i++) {
            element.setAttribute("data-final", value);
            if (element.getAttribute("data-final") === "value") checksum++;
        }''',
        "identity": '''for (let i = 0; i < n; i++) {
            if (element.parentNode === documentNode) checksum++;
            if (element.isSameNode(element)) checksum++;
        }''',
    }
    source = '''(() => {
        function work(n) {
            let checksum = 0;
            BODY
            return checksum;
        }
        for (let i = 0; i < 3; i++) work(1000);
        const started = Date.now();
        const checksum = work(COUNT);
        return JSON.stringify({checksum, loop_ms: Date.now() - started});
    })();'''.replace("BODY", bodies[name]).replace("COUNT", str(count))
    if name == "js":
        remainder = count % 1024
        expected = ((count // 1024) * 523776 + remainder * (remainder - 1) // 2) & 0xFFFFFFFF
        if expected >= 0x80000000:
            expected -= 0x100000000
    else:
        expected = count * (2 if name == "identity" else 1)
    mutations = count + 3000 if name in ("attributes", "coercion") else 0
    return source, expected, mutations


def invoke(binary, engine, script, cpu):
    command = [str(binary), "--backend", engine]
    if script is not None:
        command.extend(["--script", str(script)])
    if cpu is not None:
        command = ["taskset", "--cpu-list", str(cpu), *command]
    started = time.perf_counter_ns()
    result = subprocess.run(command, capture_output=True, text=True, timeout=120, check=True)
    wall_ms = (time.perf_counter_ns() - started) / 1e6
    payload = json.loads(result.stdout)
    return payload, wall_ms


def check_result(name, payload, expected, mutations):
    if name == "contract":
        rows = payload["results"]
        if len(rows) != 42 or any(row["status"] != "PASS" for row in rows):
            raise RuntimeError(f"contract failure: {payload}")
        if payload["dom_attribute"] != "from-js":
            raise RuntimeError("contract did not mutate the real Document")
    elif payload["results"]["checksum"] != expected or payload["mutation_count"] != mutations:
        raise RuntimeError(f"workload failure: {payload}")
    elif name in ("attributes", "coercion") and payload["dom_attribute"] != "value":
        raise RuntimeError("attribute workload did not mutate the real Document")


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
    cases = [("empty", 0), ("contract", 42), ("js", 1_000_000), ("js", 10_000_000),
             ("getter", 100_000), ("getter", 1_000_000),
             ("attributes", 25_000), ("attributes", 250_000),
             ("coercion", 25_000), ("coercion", 250_000),
             ("identity", 100_000), ("identity", 1_000_000)]
    report = {
        "environment": {"platform": platform.platform(), "python": platform.python_version(),
                        "cpu_affinity": args.cpu, "samples": args.samples,
                        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                        "load_before": os.getloadavg()},
        "method": "Fresh process each sample; OS-cache priming pair discarded; engine order alternates. Loop workloads call the same function 3x1000 before measurement. elapsed_ms includes engine initialization, compile, warmup, work, JSON parse and engine destruction; wall_ms also includes subprocess/taskset/file IO/output. loop_ms uses Date.now (1ms granularity).",
        "cases": [],
    }
    tmp_root = Path(os.environ.get("TMPDIR", str(Path.home() / "tmp")))
    tmp_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="webidl-perf-", dir=tmp_root) as scratch:
        for name, count in cases:
            source, expected, mutations = ('JSON.stringify({checksum:0})', 0, 0) if name == "empty" else (None, None, None)
            if name not in ("empty", "contract"):
                source, expected, mutations = workload(name, count)
            path = None
            if source is not None:
                path = Path(scratch) / f"{name}-{count}.js"
                path.write_text(source, encoding="utf-8")
            case = {"name": name, "iterations": count, "engines": {"boa": [], "v8": []}}
            for sample in range(args.samples + 1):
                engines = ["boa", "v8"] if sample % 2 == 0 else ["v8", "boa"]
                for engine in engines:
                    payload, wall_ms = invoke(binary, engine, path, args.cpu)
                    check_result(name, payload, expected, mutations)
                    if sample:
                        case["engines"][engine].append({"elapsed_ms": payload["elapsed_ms"],
                                                       "wall_ms": wall_ms,
                                                       "loop_ms": payload["results"].get("loop_ms") if name != "contract" else None})
            for engine, samples in case["engines"].items():
                values = [sample["elapsed_ms"] for sample in samples]
                print(f"{name:12} {count:10} {engine:3} median={statistics.median(values):9.3f} ms range={min(values):.3f}..{max(values):.3f}", flush=True)
            report["cases"].append(case)
    report["environment"]["load_after"] = os.getloadavg()
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
