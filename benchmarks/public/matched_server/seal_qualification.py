# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Seal all serving selections before a qualification reader opens held-out vectors."""

import argparse
import json
import math
import os
from pathlib import Path
import platform
import time

from adapters import FILTERS, SYSTEMS
from calibrate import configurations, sha, write
from distribution import PINS, verify_archive


HERE = Path(__file__).parent
CRITICAL_LOCAL = ('adapters.py', 'oracle.py', 'PLAN.json', 'pyproject.toml', 'uv.lock')
CRITICAL_LIFECYCLE = ('server_proc.py', 'server_worker.py', 'servers.py', 'ann_worker.py', 'text_worker.py')
ENGINE_BINARIES = {'warm': ('warmBinarySha256',),
                   'pgvector': ('postgresBinarySha256', 'pgvectorBinarySha256'),
                   'opensearch': ()}


def quality_cell(row, plan):
    if row.get('queries') != plan['queries'] or type(row.get('queries')) is not int:
        return False
    if any(type(row.get(name)) not in (int, float) or not math.isfinite(row[name])
           for name in ('meanRecall', 'worstRecall', 'median', 'p95', 'p99')):
        return False
    if any(type(row.get(name)) is not int for name in ('errors', 'unresolvedExactMembership')):
        return False
    if row.get('configuration', {}).get('route') == 'exact' and (row['meanRecall'] != 1 or row['worstRecall'] != 1):
        return False
    return (row.get('pass') is True and row.get('errors') == 0
            and row.get('unresolvedExactMembership') == 0
            and .99 <= row['meanRecall'] <= 1 and .9 <= row['worstRecall'] <= row['meanRecall']
            and 0 < row['median'] <= row['p95'] <= row['p99'])


def validate_selection(selection, freeze, plan, engine, dataset, source_hashes, binary_hashes):
    if (selection.get('complete') is not True or selection.get('eligible') is not True
            or selection.get('selectedBeforeHeldOutQueries') is not True or selection.get('failures') != []):
        raise ValueError('calibration is incomplete, ineligible or has failed builds')
    if freeze.get('plan') != plan or freeze.get('engine') != engine or freeze.get('dataset') != dataset:
        raise ValueError('calibration policy, engine or corpus mismatch')
    for name, expected in source_hashes.items():
        matches = [value for path, value in freeze['sources'].items() if Path(path).name == name]
        if matches != [expected]:
            raise ValueError('calibrated serving source mismatch: ' + name)
    if engine not in ENGINE_BINARIES:
        raise ValueError('unknown calibrated engine')
    if any(name not in binary_hashes or freeze.get(name) != binary_hashes[name]
           for name in ENGINE_BINARIES[engine]):
        raise ValueError('calibrated executable mismatch')
    candidates = selection.get('candidates')
    if not isinstance(candidates, list) or not candidates:
        raise ValueError('no eligible calibrated build')
    seen_builds = set()
    for candidate in candidates:
        build = candidate['build']
        identity = json.dumps(build, sort_keys=True)
        if build not in plan['calibration'][engine]['builds'] or identity in seen_builds:
            raise ValueError('undeclared or duplicate calibrated build')
        seen_builds.add(identity)
        if set(candidate['filters']) != set(FILTERS):
            raise ValueError('incomplete calibrated filter coverage')
        for name, chosen in candidate['filters'].items():
            configuration = chosen['configuration']
            if configuration not in list(configurations(engine, build, name, plan)):
                raise ValueError('undeclared selected query configuration')
            states = chosen['states']
            if len(states) != len(plan['states']) or {row['state'] for row in states} != set(plan['states']):
                raise ValueError('missing or duplicate selected calibration state')
            for row in states:
                if row.get('filter') != name or row.get('configuration') != configuration or not quality_cell(row, plan):
                    raise ValueError('selected calibration cell does not satisfy its gate')
            median = math.exp(math.fsum(math.log(row['median']) for row in states) / len(states))
            if not math.isclose(chosen['median'], median, rel_tol=1e-12, abs_tol=0.):
                raise ValueError('selected filter objective mismatch')
        objective = math.exp(math.fsum(math.log(row['median']) for row in candidate['filters'].values()) / len(FILTERS))
        if not math.isclose(candidate['objective'], objective, rel_tol=1e-12, abs_tol=0.):
            raise ValueError('selected build objective mismatch')
    chosen = min(candidates, key=lambda row: (row['objective'], json.dumps(row['build'], sort_keys=True)))
    if selection.get('selected') != chosen:
        raise ValueError('selection is not the declared fastest eligible build')
    return chosen


def current_sources():
    paths = [HERE / name for name in CRITICAL_LOCAL]
    paths += [HERE.parents[1] / 'competitive/warmbench' / name for name in CRITICAL_LIFECYCLE]
    return {path.name: sha(path) for path in paths}


def current_binaries():
    return {'warmBinarySha256': sha(Path(os.environ['WARM_BENCH_WARM_SERVER'])),
            'postgresBinarySha256': sha(Path(os.environ['WARM_BENCH_PG_BIN']) / 'postgres'),
            'pgvectorBinarySha256': sha(Path(os.environ['WARM_BENCH_PG_BIN']).parent / 'lib/vector.so')}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--calibrations', type=Path, required=True,
                        help='JSON object mapping every corpus and engine to its calibration directory')
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--hardware', required=True)
    args = parser.parse_args()
    plan = json.loads((HERE / 'PLAN.json').read_text())
    if platform.system() != 'Linux' or args.hardware not in plan['resources']['physicalHosts']:
        parser.error('requires Linux and a declared physical configuration')
    if any(not path.is_absolute() for path in (args.calibrations, args.inputs, args.output)):
        parser.error('paths must be absolute')
    mapping = json.loads(args.calibrations.read_text())
    if set(mapping) != set(plan['datasets']) or any(set(engines) != set(SYSTEMS) for engines in mapping.values()):
        raise ValueError('all declared corpora and engines must be calibrated before sealing')
    preparation = json.loads((args.inputs / 'finished.json').read_text())
    if preparation.get('complete') is not True or sha(args.inputs / 'freeze.json') != plan['inputFreezeSha256']:
        raise ValueError('qualification input preparation mismatch')
    binaries = current_binaries()
    source_hashes = current_sources()
    records, selected = {}, {}
    for dataset, engines in mapping.items():
        input_record_path = args.inputs / dataset / 'inputs.json'
        input_digest = next(row['inputsSha256'] for row in preparation['completed'] if row['name'] == dataset)
        if sha(input_record_path) != input_digest:
            raise ValueError('input receipt mismatch')
        input_record = json.loads(input_record_path.read_text())
        if input_record['queryOrdinals'] != list(range(1000, 2000)):
            raise ValueError('qualification query ordinals changed')
        selected[dataset], records[dataset] = {}, {'inputReceipt': input_record, 'engines': {}}
        for engine, directory in engines.items():
            folder = Path(directory)
            if not folder.is_absolute():
                raise ValueError('calibration directories must be absolute')
            selection = json.loads((folder / 'selection.json').read_text())
            freeze = json.loads((folder / 'freeze.json').read_text())
            if sha(folder / 'freeze.json') != selection['freezeSha256'] or freeze['inputReceiptSha256'] != input_digest:
                raise ValueError('calibration receipt chain mismatch')
            chosen = validate_selection(selection, freeze, plan, engine, dataset, source_hashes, binaries)
            selected[dataset][engine] = {'build': chosen['build'],
                                        'filters': {name: row['configuration'] for name, row in chosen['filters'].items()}}
            records[dataset]['engines'][engine] = {'directory': str(folder), 'selection': selection,
                'selectionSha256': sha(folder / 'selection.json'), 'freeze': freeze}
    archive_arch, archive_digest = PINS[platform.machine()]
    distribution = verify_archive(Path(os.environ['WARM_MATCHED_OPENSEARCH_HOME']),
                                  Path(os.environ['WARM_MATCHED_OPENSEARCH_ARCHIVE']), archive_digest)
    args.output.mkdir()
    write(args.output / 'configuration-seal.json', {
        'schema': 'warm.qualification-configuration-seal/1', 'hardware': args.hardware,
        'sealedAt': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'plan': plan, 'sources': source_hashes, 'binaries': binaries,
        'host': {'hostname': platform.node(), 'platform': platform.platform(), 'machine': platform.machine()},
        'selected': selected, 'calibrationEvidence': records,
        'opensearchDistribution': {'files': distribution, 'archiveSha512': archive_digest,
            'officialChecksumUrl': f'https://artifacts.opensearch.org/releases/bundle/opensearch/3.9.0/opensearch-3.9.0-linux-{archive_arch}.tar.gz.sha512'},
        'qualificationSourceHashes': {path.name: sha(path) for path in sorted(HERE.glob('*.py'))},
        'heldOutVectorsOpenedBySealer': False,
        'scope': 'Configuration and component seal only; raw calibration validation, resource/isolation checks, serving measurements and independent reproduction remain separate obligations.'})
    print(json.dumps({'sealed': True, 'sha256': sha(args.output / 'configuration-seal.json'),
                      'heldOutVectorsOpened': False}))


if __name__ == '__main__':
    main()
