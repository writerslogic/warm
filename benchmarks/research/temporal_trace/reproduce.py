# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Reproduce the frozen temporal research result from a portable source bundle."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def verify_bundle(root, manifest):
    for name, expected in manifest['files'].items():
        relative = Path(name)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('invalid source manifest path')
        path = root / relative
        if not path.resolve().is_relative_to(root.resolve()) or path.is_symlink() or digest(path) != expected:
            raise ValueError('source bundle differs from its manifest: ' + name)


def run(command, output, root):
    with output.open('xb') as stream:
        subprocess.run(command, cwd=root, stdout=stream, stderr=subprocess.STDOUT, check=True, timeout=600)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = args.bundle.resolve(strict=True)
    if sys.version_info < (3, 11):
        parser.error('Python 3.11 or newer is required')
    manifest = json.loads((root / 'manifest.json').read_text())
    verify_bundle(root, manifest)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    relative = Path('benchmarks/research/temporal_trace')
    run([sys.executable, '-m', 'unittest', 'discover', '-s', str(relative), '-p', 'test_*.py', '-v'],
        args.output / 'tests.log', root)
    run([sys.executable, '-u', str(relative / 'run.py'), '--output', str(args.output / 'trace')],
        args.output / 'trace.log', root)
    result = json.loads((args.output / 'trace/finished.json').read_text())
    for name, expected in manifest['expected']['traceSummary'].items():
        if type(result[name]) is not type(expected) or result[name] != expected:
            raise ValueError('research result changed: ' + name)
    for case, expected in manifest['expected']['cohortPackets'].items():
        for method, checksum in expected.items():
            if digest(args.output / 'trace' / case / (method + '.json')) != checksum:
                raise ValueError('certificate differs from original experiment: ' + case + '/' + method)
    run([sys.executable, '-u', str(relative / 'audit_symbolic.py'), '--original', str(args.output / 'trace'),
         '--output', str(args.output / 'symbolic')], args.output / 'symbolic.log', root)
    symbolic = json.loads((args.output / 'symbolic/finished.json').read_text())
    if symbolic.get('complete') is not True or len(symbolic['cases']) != 12 or not all(
            item.get('bothPacketsIdentical') is True for item in symbolic['cases']):
        raise ValueError('symbolic control did not reproduce every identical packet')
    verify_bundle(root, manifest)
    receipt = {'complete': True, 'scientificResultReproduced': True, 'benchmarkClaim': False,
               'independenceVerified': False, 'python': sys.version, 'manifestSha256': digest(root / 'manifest.json'),
               'traceSummary': {name: result[name] for name in manifest['expected']['traceSummary']},
               'symbolicPacketsMatched': 12, 'timingPolicy': 'Recorded, not required to match across machines',
               'scope': 'Temporal research correctness, failed size screen and matching established control; not the eight-milestone system claim'}
    with (args.output / 'finished.json').open('x') as stream:
        json.dump(receipt, stream, indent=2)
        stream.write('\n')
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
