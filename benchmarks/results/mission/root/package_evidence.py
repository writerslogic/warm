# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Validate research artifacts and snapshot the canonical mission ledger."""
import argparse
import ast
import csv
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess


def package(args):
    root = Path(__file__).resolve().parent.parent
    commit = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
    counts = {'python_syntax': 0, 'json_documents': 0, 'registered_script_hashes': 0}
    for path in sorted(root.rglob('*')):
        if not path.is_file() or '__pycache__' in path.parts or '.venv' in path.parts:
            continue
        if path.suffix == '.py':
            ast.parse(path.read_text(), filename=str(path))
            counts['python_syntax'] += 1
        if path.suffix == '.json':
            value = json.loads(path.read_text())
            counts['json_documents'] += 1
            if path.name == 'registration.json' and 'script_sha256' in value:
                relative = value.get('script', '').split('/benchmarks/results/mission/', 1)[-1]
                current = root / relative
                if not relative or not current.is_file() or hashlib.sha256(current.read_bytes()).hexdigest() != value['script_sha256']:
                    matches = [script for script in root.rglob('*.py')
                               if '__pycache__' not in script.parts and '.venv' not in script.parts
                               and hashlib.sha256(script.read_bytes()).hexdigest() == value['script_sha256']]
                else:
                    matches = [current]
                if not matches:
                    raise ValueError('registered script not preserved: ' + str(path))
                counts['registered_script_hashes'] += 1
    if len((args.ledger / 'NOW.md').read_text().splitlines()) > 150:
        raise ValueError('NOW exceeds 150 lines')
    filemap_path = args.ledger / 'FILEMAP.tsv'
    rows = list(csv.reader(io.StringIO(filemap_path.read_text()), delimiter='\t'))
    known = {row[0] for row in rows[1:]}
    for path in sorted(root.rglob('*')):
        if not path.is_file() or any(part in ('__pycache__', '.venv', 'ledger') for part in path.relative_to(root).parts):
            continue
        if str(path) not in known:
            relative = path.relative_to(root)
            lane = relative.parts[0] if relative.parts[0] in ('A', 'B', 'C') else 'root'
            rows.append([str(path), lane, 'integrated development evidence; hash packaged', path.stem, commit])
    with filemap_path.open('w', newline='') as stream:
        csv.writer(stream, delimiter='\t', lineterminator='\n').writerows(rows)
    result = {'experiment': args.experiment, 'decision': 'accept-research-package-only',
              'checks': counts, 'engine_gates_rerun': False, 'confirmation_rows_met': 0,
              'scope': 'Syntax, JSON and registered script provenance only; lock validated separately.'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    with (args.ledger / 'LOG.tsv').open('a', newline='') as stream:
        csv.writer(stream, delimiter='\t', lineterminator='\n').writerow([
            args.experiment, 'selection', 'integrated-evidence', 'all-unmet', 1,
            'Are packaged research artifacts parseable and registered scripts unchanged?',
            'All syntax, JSON, source pins and ledger line cap pass; no engine or target claim',
            json.dumps(result, sort_keys=True), 'compose', commit, 'research-only package',
            'owned-local', str(args.output.resolve())])
    canonical = {'MISSION.md', 'NOW.md', 'MATRIX.tsv', 'HYP.md', 'LOG.tsv',
                 'FILEMAP.tsv', 'PENDING.md', 'SPECIALIST.md', 'SEEDS.md'}
    for path in sorted(args.ledger.iterdir()):
        if path.name not in canonical or not path.is_file():
            continue
        destination = root / 'ledger' / path.name
        if path.suffix == '.tsv':
            rows = list(csv.reader(io.StringIO(path.read_text()), delimiter='\t'))
            with destination.open('w', newline='') as stream:
                csv.writer(stream, delimiter='\t', quoting=csv.QUOTE_ALL, lineterminator='\n').writerows(rows)
        else:
            destination.write_text(path.read_text().rstrip() + '\n')
    hashes = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
              for path in sorted(root.rglob('*')) if path.is_file()
              and path.name != 'artifact-sha256.json'
              and '__pycache__' not in path.parts and '.venv' not in path.parts}
    (root / 'artifact-sha256.json').write_text(json.dumps(hashes, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'decision': result['decision'], 'files_hashed': len(hashes), 'checks': counts}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ledger', required=True, type=Path)
    parser.add_argument('--experiment', required=True)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists() or not re.fullmatch(r'E\d+', args.experiment):
        parser.error('new receipt and numeric experiment required')
    try:
        package(args)
    except Exception as error:
        failure = {'experiment': args.experiment, 'decision': 'reject-package-validation',
                   'failure_type': type(error).__name__, 'failure': str(error)}
        if not args.output.exists():
            args.output.parent.mkdir(parents=True, exist_ok=True)
            with args.output.open('x') as stream:
                json.dump(failure, stream, indent=2)
                stream.write('\n')
        with (args.ledger / 'LOG.tsv').open('a', newline='') as stream:
            csv.writer(stream, delimiter='\t', lineterminator='\n').writerow([
                args.experiment, 'selection', 'integrated-evidence', 'all-unmet', 1,
                'Validate research package', 'All syntax and provenance checks pass',
                json.dumps(failure, sort_keys=True), 'reject', 'working-tree',
                'research-only package', 'owned-local', str(args.output.resolve())])
        raise


if __name__ == '__main__':
    main()
