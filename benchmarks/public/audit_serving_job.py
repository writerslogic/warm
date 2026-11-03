# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Audit one complete serving lifecycle without promoting it to a system claim."""

import argparse
import json
import math
from pathlib import Path
import sys

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent / 'matched_server'))
from adapters import payload, validate_hits
from calibrate import sha, write
from measure import audit_scores, payload_digest, query_order
from oracle import truth
from qualify import job_order
from seal_qualification import current_sources
from verify_evidence import read_rows, same_number, verify_cell
from audit_calibration import audit_reference, same_structure


def load(path, limit=64 * 1024**2):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        raise ValueError('missing, linked or oversized evidence file: ' + path.name)
    return json.loads(path.read_text())


def nonnegative(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def bounded_integer(value, maximum):
    return type(value) is int and 0 <= value <= maximum


def validate_cells(result, plan, selected, identity, development):
    if result.get('complete') is not True or result.get('failure') is not None:
        raise ValueError('incomplete serving job')
    if result.get('developmentOnly') is not development or any(not same_structure(value, result.get(name)) for name, value in identity.items()):
        raise ValueError('serving job identity or mode mismatch')
    expected = [(state, name, clients) for state in plan['states'] for name in plan['filters']
                for clients in plan['qualification']['concurrency']]
    actual = [[row.get('state'), row.get('filter'), row.get('clients')] for row in result['cells']]
    if not same_structure(list(map(list, expected)), actual):
        raise ValueError('missing, repeated or reordered serving cells')
    for row in result['cells']:
        cell_identity = {name: identity[name] for name in ('engine', 'dataset', 'repetition', 'hardware')}
        cell_identity.update({'configuration': selected['filters'][row['filter']],
                              'churn': row['state'] != 'baseline', 'developmentOnly': development})
        if any(not same_structure(value, row.get(name)) for name, value in cell_identity.items()):
            raise ValueError('cell differs from its frozen serving job')
        if row.get('evidenceVerified') is not False or row.get('isolationPassed') is not False:
            raise ValueError('unreviewed serving output was already promoted')
    return expected


def verify_phases(folder, result, plan):
    phases = result['phases']
    labels = ['build', 'baseline', 'churn', 'churn-before-maintenance', 'maintenance',
              'churn-after-maintenance', 'reopen', 'final']
    if [phase.get('phase') for phase in phases] != labels:
        raise ValueError('serving lifecycle is missing, repeated or reordered')
    limits = plan['resources']
    if (not nonnegative(phases[0].get('seconds')) or phases[0]['seconds'] > limits['buildSeconds']
            or not nonnegative(phases[4].get('seconds'))):
        raise ValueError('invalid build or maintenance duration')
    for index in (0, 1, 3, 5, 6, 7):
        if not bounded_integer(phases[index].get('allocatedStoreBytes'), limits['maxStoreBytes']):
            raise ValueError('recorded store size violates the declared bound')
    churn = phases[2]
    if churn.get('acknowledgedReplacementsChecked') != 100 or churn.get('acknowledgedDeletionsChecked') != 100:
        raise ValueError('incomplete acknowledged mutation verification')
    mutations = [('replace', index) for index in range(100)] + [('delete', index) for index in range(100, 200)]
    operations = churn['operations']
    if [(item.get('operation'), item.get('id')) for item in operations] != mutations:
        raise ValueError('mutation receipt sequence mismatch')
    for (kind, ordinal), item in zip(mutations, operations):
        if type(item['id']) is not int or not nonnegative(item.get('seconds')) or item['seconds'] > limits['mutationSeconds']:
            raise ValueError('invalid mutation identity or duration')
        if not same_structure(item, load(folder / f'{kind}-{ordinal}.json')):
            raise ValueError('mutation receipt differs from the lifecycle summary')
    samples = list(read_rows(folder / 'resources.ndjson'))
    if not samples:
        raise ValueError('missing lifecycle resource samples')
    last, peak = None, 0
    for sample in samples:
        if not nonnegative(sample.get('monotonic')) or (last is not None and sample['monotonic'] < last):
            raise ValueError('invalid lifecycle resource timeline')
        if not bounded_integer(sample.get('serverPssBytes'), limits['maxServerPssBytes']):
            raise ValueError('lifecycle server memory bound failed')
        if not isinstance(sample.get('load'), list) or len(sample['load']) != 3 or not all(map(nonnegative, sample['load'])):
            raise ValueError('invalid recorded guest load')
        last, peak = sample['monotonic'], max(peak, sample['serverPssBytes'])
    if type(result.get('peakServerPssBytes')) is not int or result['peakServerPssBytes'] != peak:
        raise ValueError('lifecycle server memory peak mismatch')
    return {'resourceSamples': len(samples), 'peakServerPssBytes': peak,
            'mutations': len(mutations), 'storeSampling': 'Declared lifecycle boundaries, not continuous peak',
            'settingsReceipts': [phases[index]['settings'] for index in (0, 4)]}


def verify_cold(cold, *, engine, dataset, repetition, configuration, queries, train, expected, limits):
    ordinal = query_order(dataset, repetition, len(queries))[0]
    if type(cold.get('query')) is not int or cold['query'] != ordinal or cold['expected'] != expected[ordinal]['ids']:
        raise ValueError('cold-open query or independent oracle mismatch')
    times = [cold.get(name) for name in ('startedAt', 'readyAt', 'receivedAt', 'validatedAt')]
    if not all(map(nonnegative, times)) or times != sorted(times):
        raise ValueError('invalid cold-open timeline')
    if (not same_number(cold['readySeconds'], times[1] - times[0])
            or not same_number(cold['firstValidatedResponseSeconds'], times[3] - times[0])):
        raise ValueError('cold-open duration boundary mismatch')
    hits = []
    if not isinstance(cold['returned'], list) or len(cold['returned']) > 10:
        raise ValueError('invalid cold-open result count')
    for item in cold['returned']:
        identifier = item['id']
        if (not isinstance(identifier, str) or not identifier.isascii() or not identifier.isdigit()
                or len(identifier) > 12 or str(int(identifier)) != identifier):
            raise ValueError('invalid cold-open result identity')
        index = int(identifier)
        hits.append({**payload(index, int(index < 100)), 'score': item['score']})
    identities = validate_hits(hits, len(train), True, 'all')
    if cold['hits'] != sorted(identities) or cold['payloadSha256'] != payload_digest(hits):
        raise ValueError('cold-open payload or identity evidence mismatch')
    score_error = audit_scores(engine, hits, queries[ordinal], train, True)
    if not same_number(cold['maximumScoreError'], score_error):
        raise ValueError('cold-open raw score mismatch')
    gold = set(expected[ordinal]['ids'])
    recall = len(identities & gold) / len(gold)
    exact_error = int(configuration['route'] == 'exact' and identities != gold)
    passed = recall >= .9 and not exact_error and times[1] - times[0] <= limits['startupSeconds'] and times[2] - times[1] <= limits['querySeconds']
    if (not same_number(cold['recall'], recall) or type(cold['unresolvedExactMembership']) is not int
            or cold['unresolvedExactMembership'] != exact_error or cold.get('pass') is not passed):
        raise ValueError('cold-open quality summary mismatch')
    return {'qualityPassed': passed, 'query': ordinal, 'maximumScoreError': score_error,
            'cache': 'Fresh server process; filesystem page cache retained'}


def verify_job(folder, *, plan, selected, identity, development, train_path, queries, expected):
    result = load(folder / 'finished.json')
    cells = validate_cells(result, plan, selected, identity, development)
    shape = (plan['rows'], plan['datasets'][identity['dataset']])
    train = np.memmap(train_path, dtype='<f4', mode='r', shape=shape)
    lifecycle = verify_phases(folder, result, plan)
    cold = load(folder / 'cold-open.json')
    if not same_structure(cold, result['cold']):
        raise ValueError('cold-open receipt differs from job summary')
    cold_check = verify_cold(cold, engine=identity['engine'], dataset=identity['dataset'],
                            repetition=identity['repetition'], configuration=selected['filters']['all'],
                            queries=queries, train=train, expected=expected['churn']['all'], limits=plan['resources'])
    checked = []
    for position, (state, name, clients) in enumerate(cells):
        cell_folder = folder / f'{state}-{name}-{clients}'
        cell = load(cell_folder / 'finished.json')
        embedded = {key: value for key, value in result['cells'][position].items()
                    if key not in ('state', 'hardware', 'evidenceVerified', 'isolationPassed')}
        if not same_structure(cell, embedded):
            raise ValueError('standalone and embedded cell receipts differ')
        cell_identity = {key: identity[key] for key in ('engine', 'dataset', 'repetition')}
        cell_identity.update({'configuration': selected['filters'][name], 'filter': name,
                              'churn': state != 'baseline', 'clients': clients})
        evidence = verify_cell(cell_folder, expected=expected['churn' if state != 'baseline' else 'baseline'][name],
                               queries=queries, train_path=train_path, shape=shape, limits=plan['resources'], identity=cell_identity)
        checked.append({**cell, **evidence, 'state': state, 'hardware': identity['hardware']})
    quality = cold_check['qualityPassed'] and all(item['qualityPassed'] for item in checked)
    if result.get('qualityPassed') is not quality:
        raise ValueError('job quality summary differs from its raw evidence')
    return {'complete': True, 'rawJobEvidenceVerified': True, 'qualityPassed': quality,
            'resourcesPassed': all(item['resourcesPassed'] for item in checked), 'cells': checked,
            'lifecycle': lifecycle, 'cold': cold_check, 'finishedSha256': sha(folder / 'finished.json'),
            'developmentOnly': development, 'claimQualified': False,
            'pending': ['Physical-host isolation evidence', 'Actual engine settings/durability review',
                        'Complete two-host comparison matrix', 'Independent evaluator reproduction']}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--synthetic', action='store_true')
    parser.add_argument('--seal', type=Path)
    parser.add_argument('--inputs', type=Path)
    parser.add_argument('--job', type=int)
    args = parser.parse_args()
    if not args.root.is_absolute() or not args.output.is_absolute():
        parser.error('absolute evidence paths required')
    source_evidence = None
    if args.synthetic:
        if args.seal is not None or args.inputs is not None or args.job is not None:
            parser.error('synthetic audit cannot consume qualification input arguments')
        freeze = load(args.root / 'freeze.json')
        if freeze.get('synthetic') is not True or freeze.get('benchmarkClaim') is not False:
            raise ValueError('missing synthetic development freeze')
        plan, selected = freeze['plan'], freeze['selected']
        folder, train_path, query_path = args.root / 'job', args.root / 'train.f32', args.root / 'queries.f32'
        if sha(train_path) != freeze['trainSha256'] or sha(query_path) != freeze['queriesSha256']:
            raise ValueError('synthetic input hash mismatch')
        summary = load(folder / 'finished.json')
        identity = {name: summary[name] for name in ('engine', 'dataset', 'repetition', 'hardware')}
        reference = args.root / 'reference.json'
    else:
        if args.seal is None or args.inputs is None or args.job is None or args.job < 0:
            parser.error('qualification audit requires seal, inputs and a nonnegative job index')
        seal = load(args.seal)
        if seal.get('schema') != 'warm.qualification-configuration-seal/1' or seal.get('heldOutVectorsOpenedBySealer') is not False:
            raise ValueError('invalid pre-query qualification seal')
        plan, jobs = seal['plan'], job_order(seal['plan'])
        serving_source = Path(__file__).parent / 'matched_server'
        if (plan != load(serving_source / 'PLAN.json') or seal['sources'] != current_sources()
                or seal['qualificationSourceHashes'] != {path.name: sha(path) for path in sorted(serving_source.glob('*.py'))}
                or seal['hardware'] not in plan['resources']['physicalHosts']):
            raise ValueError('sealed policy, hardware or audited implementation differs')
        if args.job >= len(jobs) or load(args.root / 'sequence.json') != {'sealSha256': sha(args.seal), 'jobs': jobs}:
            raise ValueError('qualification job order or seal changed')
        identity = {**jobs[args.job], 'hardware': seal['hardware']}
        selected = seal['selected'][identity['dataset']][identity['engine']]
        folder = args.root / f'job-{args.job:02d}'
        source_evidence = load(args.root / f'job-{args.job:02d}-source-verification.json')
        freeze = load(args.root / f'job-{args.job:02d}-freeze.json')
        if (source_evidence.get('pass') is not True or source_evidence['configurationSealSha256'] != sha(args.seal)
                or source_evidence['finishedSha256'] != sha(folder / 'finished.json')
                or freeze['configurationSealSha256'] != sha(args.seal) or freeze['job'] != jobs[args.job]):
            raise ValueError('qualification source/receipt chain mismatch')
        caps = freeze['environment']['effectiveCaps']
        if (not bounded_integer(caps['memory'], plan['resources']['guestOrCgroupBytes']) or caps['memory'] == 0
                or type(caps['swap']) is not int or caps['swap'] != 0
                or len(freeze['affinity']) != 4 or len(set(freeze['affinity'])) != 4
                or any(type(cpu) is not int or cpu < 0 for cpu in freeze['affinity'])):
            raise ValueError('qualification resource cap or affinity mismatch')
        train_path = args.inputs / identity['dataset'] / 'train.f32'
        query_path = args.inputs / identity['dataset'] / 'test.f32'
        for path in (train_path, query_path):
            expected_hash = seal['calibrationEvidence'][identity['dataset']]['inputReceipt']['files'][path.name]['sha256']
            if sha(path) != expected_hash:
                raise ValueError('qualification input hash mismatch')
        reference = args.root / (identity['dataset'] + '-reference.json')
        if freeze['oracleSha256'] != sha(reference) or load(reference.with_suffix('.preparation.json'))['oracleSha256'] != sha(reference):
            raise ValueError('qualification oracle receipt mismatch')
    shape = (plan['rows'], plan['datasets'][identity['dataset']])
    if (not bounded_integer(shape[0], 100000) or shape[0] < 201 or not bounded_integer(shape[1], 512) or shape[1] == 0
            or not bounded_integer(plan['queries'], 1000) or plan['queries'] == 0):
        raise ValueError('cohort exceeds the audited allocation bounds')
    if (train_path.stat().st_size != math.prod(shape) * 4
            or query_path.stat().st_size != plan['queries'] * shape[1] * 4):
        raise ValueError('input byte dimensions differ from the declared cohort')
    train = np.memmap(train_path, dtype='<f4', mode='r', shape=shape)
    queries = np.fromfile(query_path, dtype='<f4').reshape(plan['queries'], shape[1])
    expected = {'baseline': truth(train, queries), 'churn': truth(train, queries, True)}
    maximum = audit_reference(expected, load(reference))
    result = verify_job(folder, plan=plan, selected=selected, identity=identity, development=args.synthetic,
                        train_path=train_path, queries=queries, expected=expected)
    result.update({'oracleMaximumRoundingDifference': maximum, 'sourceReceipt': source_evidence,
                   'auditorSha256': sha(Path(__file__)), 'sourceReceiptScope': 'Recorded post-run source checks; no off-host executable reinspection'})
    args.output.mkdir(parents=True, exist_ok=False)
    write(args.output / 'finished.json', result)
    print(json.dumps({'complete': True, 'cells': len(result['cells']), 'qualityPassed': result['qualityPassed'],
                      'resourcesPassed': result['resourcesPassed'], 'claimQualified': False}))


if __name__ == '__main__':
    main()
