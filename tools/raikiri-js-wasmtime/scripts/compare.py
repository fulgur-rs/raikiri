#!/usr/bin/env python3
"""Compare every ordered named assertion, preserving duplicate names."""
import json
import sys

def status(test):
    if test['passed']:
        return 'PASS'
    return test['message'].split(':', 1)[0]

def compare(native, guest):
    left = {r['test_id']: r for r in native}
    right = {r['test_id']: r for r in guest}
    differences = []
    messages = []
    if len(left) != len(native) or len(right) != len(guest):
        differences.append({'kind': 'duplicate file identifiers'})
    for path in sorted(left.keys() | right.keys()):
        if path not in left or path not in right:
            differences.append({'file': path, 'kind': 'missing/extra file'})
            continue
        a, b = left[path], right[path]
        if a['error'] != b['error']:
            differences.append({'file': path, 'kind': 'page error', 'native': a['error'], 'guest': b['error']})
        ta, tb = a['tests'], b['tests']
        if len(ta) != len(tb):
            differences.append({'file': path, 'kind': 'assertion count', 'native': len(ta), 'guest': len(tb)})
        for i, (x, y) in enumerate(zip(ta, tb)):
            if (x['name'], status(x)) != (y['name'], status(y)):
                differences.append({'file': path, 'index': i, 'kind': 'assertion name/status', 'native': x, 'guest': y})
            elif x['message'] != y['message']:
                messages.append({'file': path, 'index': i, 'native': x['message'], 'guest': y['message']})
    def counts(rows):
        return {'files': len(rows), 'all_pass': sum(bool(r['tests']) and not r['error'] and all(t['passed'] for t in r['tests']) for r in rows),
                'assertions': sum(len(r['tests']) for r in rows), 'passed': sum(t['passed'] for r in rows for t in r['tests']), 'errors': sum(r['error'] is not None for r in rows)}
    return {'native': counts(native), 'guest': counts(guest), 'differences': differences, 'message_differences': messages}

if __name__ == '__main__':
    if len(sys.argv) != 4:
        raise SystemExit('usage: compare.py native.json guest.json report.json')
    with open(sys.argv[1]) as f: native = json.load(f)
    with open(sys.argv[2]) as f: guest = json.load(f)
    report = compare(native, guest)
    with open(sys.argv[3], 'w') as f: json.dump(report, f, ensure_ascii=False, indent=2)
    print(json.dumps({k: v for k, v in report.items() if not isinstance(v, list)}, ensure_ascii=False))
    print(f"differences={len(report['differences'])} message_differences={len(report['message_differences'])}")
    raise SystemExit(bool(report['differences']))
