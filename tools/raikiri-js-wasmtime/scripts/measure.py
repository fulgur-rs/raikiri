#!/usr/bin/env python3
"""Run four suites sequentially, with outer timeout and process RSS sampling."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def run_suite(root, executable, stem, allowed_exits):
    result_path = stem.with_suffix('.json')
    result_path.unlink(missing_ok=True)
    command = [str(executable), '--wpt-root', str(root / 'target/wpt'), '--results-json', str(result_path)]
    env = dict(os.environ, RAIKIRI_WPT_METRICS='1')
    samples = []
    timed_out = False
    with stem.with_suffix('.log').open('w') as out, stem.with_suffix('.stderr').open('w') as err:
        start = time.monotonic()
        process = subprocess.Popen(command, cwd=root, env=env, stdout=out, stderr=err)
        deadline = start + 1800
        while process.poll() is None:
            if time.monotonic() > deadline:
                timed_out = True
                process.kill()
                process.wait()
                break
            try:
                lines = Path(f'/proc/{process.pid}/status').read_text().splitlines()
                values = {line.split(':')[0]: int(line.split()[1]) for line in lines if line.startswith(('VmRSS:', 'VmHWM:'))}
                samples.append({'seconds': time.monotonic() - start, **values})
            except (FileNotFoundError, ProcessLookupError):
                pass
            time.sleep(.02)
    record = {
        'command': command, 'sha256': hashlib.sha256(executable.read_bytes()).hexdigest(),
        'seconds': time.monotonic() - start, 'exit': process.returncode,
        'peak_rss_kib': max((s.get('VmRSS', 0) for s in samples), default=0),
        'peak_hwm_kib': max((s.get('VmHWM', 0) for s in samples), default=0),
        'timed_out': timed_out,
    }
    stem.with_suffix('.rss.json').write_text(json.dumps(samples))
    stem.with_suffix('.run.json').write_text(json.dumps(record, indent=2) + '\n')
    if timed_out or process.returncode not in allowed_exits:
        raise RuntimeError(f'runner exit {process.returncode}, timed_out={timed_out}: {executable}')
    try:
        results = json.loads(result_path.read_text())
        if not isinstance(results, list) or not results:
            raise ValueError('expected nonempty result list')
        if any(not {'test_id', 'tests', 'error'} <= row.keys() for row in results):
            raise ValueError('invalid result record')
    except (OSError, ValueError, AttributeError) as error:
        raise RuntimeError(f'fresh results unavailable: {result_path}: {error}') from error
    record['results_sha256'] = hashlib.sha256(result_path.read_bytes()).hexdigest()
    return record


def main(root):
    logs = root / 'target/integration-logs'
    measurements = logs / 'measurements.json'
    measurements.unlink(missing_ok=True)
    output = []
    for backend in ['native', 'wasmtime']:
        for suite, binary in [('parsing', 'run-parsing-invalid'), ('i18n', 'run-css-text-i18n')]:
            executable = logs / f'{backend}-bins' / binary
            stem = logs / f'{backend}-{suite}'
            record = run_suite(root, executable, stem, (0,) if suite == 'parsing' else (0, 1))
            record.update(backend=backend, suite=suite)
            output.append(record)
            measurements.write_text(json.dumps(output, indent=2) + '\n')
            print(json.dumps(record), flush=True)


if __name__ == '__main__':
    main(Path(sys.argv[1]).resolve())
