# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Audit selected calibration responses without opening held-out query vectors."""

import argparse
import hashlib
from itertools import permutations
import json
import math
from pathlib import Path
import sys

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent / 'matched_server'))
from adapters import FILTERS, encode, payload, validate_hits
from calibrate import configurations, select_configuration, sha, write
from measure import audit_scores
from oracle import summarize, truth
from seal_qualification import quality_cell
from verify_evidence import read_rows, same_number


def legacy_digest(hits, order):
    values = []
    for hit in hits:
        values.append({'id': hit['id'], 'text': hit['text'],
                       'metadata': {key: hit['metadata'][key] for key in order}})
    return hashlib.sha256(encode(values)).hexdigest()


def same_structure(left, right):
    if isinstance(left, dict):
        return isinstance(right, dict) and left.keys() == right.keys() and all(same_structure(value, right[key]) for key, value in left.items())
    if isinstance(left, list):
        return isinstance(right, list) and len(left) == len(right) and all(same_structure(a, b) for a, b in zip(left, right))
    if isinstance(left, float):
        return same_number(left, right)
    return type(left) is type(right) and left == right


def audit_reference(expected, recorded):
    if set(expected) != set(recorded):
        raise ValueError('calibration oracle state coverage mismatch')
    maximum = 0.
    for state, filters in expected.items():
        if set(filters) != set(recorded[state]):
            raise ValueError('calibration oracle filter coverage mismatch')
        for name, rows in filters.items():
            previous = recorded[state][name]
            if len(rows) != len(previous):
                raise ValueError('calibration oracle query count mismatch')
            for actual, original in zip(rows, previous):
                if actual['ids'] != original['ids'] or actual['eligible'] != original['eligible']:
                    raise ValueError('calibration oracle membership mismatch')
                if len(actual['scores']) != len(original['scores']):
                    raise ValueError('calibration oracle score count mismatch')
                pairs = list(zip(actual['scores'], original['scores']))
                if actual['cutoff'] is None or original['cutoff'] is None:
                    if actual['cutoff'] is not original['cutoff']:
                        raise ValueError('calibration oracle cutoff absence mismatch')
                else:
                    pairs.append((actual['cutoff'], original['cutoff']))
                for left, right in pairs:
                    if type(right) not in (int, float) or not math.isfinite(right) or abs(left - right) > 1e-12:
                        raise ValueError('calibration oracle score mismatch')
                    maximum = max(maximum, abs(left - right))
    return maximum


def audit_cell(path, summary, *, engine, train, queries, expected):
    records, metadata_order, maximum_error = [], None, 0.
    churn, name = summary['state'] != 'baseline', summary['filter']
    for ordinal, row in enumerate(read_rows(path)):
        if type(row.get('query')) is not int or row['query'] != ordinal or ordinal >= len(queries):
            raise ValueError('calibration query sequence mismatch')
        if row['error'] is not None or row['expected'] != expected[ordinal]['ids']:
            raise ValueError('selected calibration error or independent oracle mismatch')
        returned = row['returned']
        if not isinstance(returned, list) or len(returned) > 10:
            raise ValueError('invalid returned calibration result')
        hits = []
        for hit in returned:
            identifier = hit['id']
            if (not isinstance(identifier, str) or not identifier.isascii() or not identifier.isdigit()
                    or len(identifier) > 12 or str(int(identifier)) != identifier):
                raise ValueError('invalid returned calibration identity')
            index = int(identifier)
            hits.append({**payload(index, int(churn and index < 100)), 'score': hit['score']})
        identities = validate_hits(hits, len(train), churn, name)
        if row['hits'] != sorted(identities):
            raise ValueError('calibration hit evidence differs from returned identities')
        maximum_error = max(maximum_error, audit_scores(engine, hits, queries[ordinal], train, churn))
        if metadata_order is None and hits:
            matches = [order for order in permutations(hits[0]['metadata']) if legacy_digest(hits, order) == row['payloadSha256']]
            if len(matches) != 1:
                raise ValueError('legacy payload digest has no unique valid metadata order')
            metadata_order = matches[0]
        if legacy_digest(hits, metadata_order or ()) != row['payloadSha256']:
            raise ValueError('legacy payload digest mismatch')
        records.append(row)
    result = summarize(records, len(queries), summary['configuration']['route'] == 'exact')
    for key, value in result.items():
        if isinstance(value, bool):
            equal = summary.get(key) is value
        else:
            equal = same_number(summary.get(key), value)
        if not equal:
            raise ValueError('calibration summary mismatch: ' + key)
    if result['pass'] is not True:
        raise ValueError('selected calibration cell failed recomputed quality')
    return {'rawSha256': sha(path), 'queries': len(records), 'metadataOrder': metadata_order,
            'maximumScoreError': maximum_error, 'recomputed': result}


def raw_cell_present(path, summary):
    if path.is_file():
        return True
    error = summary.get('error')
    if (summary.get('pass') is not False or not isinstance(error, dict)
            or set(error) != {'type', 'message'}
            or any(not isinstance(error[key], str) or not error[key] for key in error)):
        raise ValueError('calibration raw cell is missing without a recorded rejection')
    return False


def audit_build(folder, build, plan, engine):
    finished = json.loads((folder / 'finished.json').read_text())
    if finished.get('complete') is not True or finished.get('failure') is not None or finished.get('build') != build:
        raise ValueError('incomplete or mismatched calibration build')
    cells, paths, rejected_without_raw = [], {}, []
    for state in plan['states']:
        for name in FILTERS:
            for index, config in enumerate(configurations(engine, build, name, plan)):
                stem = f'{state}-{name}-{index}'
                summary = json.loads((folder / (stem + '-summary.json')).read_text())
                if (summary.get('state') != state or summary.get('filter') != name
                        or summary.get('configuration') != config):
                    raise ValueError('calibration cell identity differs from declared grid')
                if summary.get('pass') is True and not quality_cell(summary, plan):
                    raise ValueError('calibration summary claims an invalid quality pass')
                if not raw_cell_present(folder / (stem + '.ndjson'), summary):
                    rejected_without_raw.append({'summary': summary,
                                                 'summarySha256': sha(folder / (stem + '-summary.json')),
                                                 'rawPath': str(folder / (stem + '.ndjson'))})
                cells.append(summary)
                paths[(state, name, json.dumps(config, sort_keys=True))] = folder / (stem + '.ndjson')
    if cells != finished['cells']:
        raise ValueError('build receipt differs from cell summaries')
    chosen = {}
    for name in FILTERS:
        selected = select_configuration([row for row in cells if row['filter'] == name],
                                        configurations(engine, build, name, plan), plan['states'])
        if selected is not None:
            chosen[name] = selected
    if not same_structure(chosen, finished['selected']):
        raise ValueError('build did not select its declared fastest eligible configurations')
    candidate = None
    if len(chosen) == len(FILTERS):
        candidate = {'build': build, 'directory': str(folder), 'filters': chosen,
                     'objective': math.exp(math.fsum(math.log(row['median']) for row in chosen.values()) / len(chosen))}
    return candidate, paths, rejected_without_raw


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--calibration', type=Path, required=True)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if any(not path.is_absolute() for path in (args.calibration, args.inputs, args.output)):
        parser.error('absolute paths required')
    selection = json.loads((args.calibration / 'selection.json').read_text())
    freeze = json.loads((args.calibration / 'freeze.json').read_text())
    if (selection.get('complete') is not True or selection.get('eligible') is not True
            or selection.get('failures') != [] or sha(args.calibration / 'freeze.json') != selection['freezeSha256']):
        raise ValueError('incomplete or mismatched calibration receipt chain')
    plan, engine, dataset = freeze['plan'], freeze['engine'], freeze['dataset']
    if plan != json.loads((Path(__file__).parent / 'matched_server/PLAN.json').read_text()):
        raise ValueError('calibration plan differs from declared policy')
    for name in ('adapters.py', 'oracle.py'):
        matches = [value for path, value in freeze['sources'].items() if Path(path).name == name]
        if matches != [sha(Path(__file__).parent / 'matched_server' / name)]:
            raise ValueError('serving/reference source differs from calibration')
    receipt_path = args.inputs / dataset / 'inputs.json'
    if sha(receipt_path) != freeze['inputReceiptSha256']:
        raise ValueError('calibration input receipt mismatch')
    inputs = json.loads(receipt_path.read_text())
    for name in ('train.f32', 'calibration.f32'):
        if sha(args.inputs / dataset / name) != inputs['files'][name]['sha256']:
            raise ValueError('calibration data digest mismatch')
    train = np.memmap(args.inputs / dataset / 'train.f32', dtype='<f4', mode='r',
                      shape=(plan['rows'], plan['datasets'][dataset]))
    queries = np.fromfile(args.inputs / dataset / 'calibration.f32', dtype='<f4').reshape(plan['queries'], plan['datasets'][dataset])
    expected = {'baseline': truth(train, queries), 'churn': truth(train, queries, True)}
    oracle_difference = audit_reference(expected, json.loads((args.calibration / 'oracle.json').read_text()))
    args.output.mkdir()
    audited, candidates, rejected_without_raw = [], [], []
    for number, build in enumerate(plan['calibration'][engine]['builds']):
        folder = args.calibration / f'build-{number}'
        candidate, paths, rejected = audit_build(folder, build, plan, engine)
        rejected_without_raw.extend(rejected)
        if candidate is None:
            continue
        candidates.append(candidate)
        for name, chosen in candidate['filters'].items():
            for summary in chosen['states']:
                path = paths[(summary['state'], name, json.dumps(summary['configuration'], sort_keys=True))]
                result = audit_cell(path, summary, engine=engine, train=train, queries=queries,
                                    expected=expected['baseline' if summary['state'] == 'baseline' else 'churn'][name])
                row = {'build': number, 'filter': name, 'state': summary['state'],
                       'configuration': summary['configuration'], **result}
                write(args.output / f'{number}-{summary["state"]}-{name}.json', row)
                audited.append(row)
    normalized = lambda rows: [{**row, 'directory': Path(row['directory']).name} for row in rows]
    if not same_structure(normalized(candidates), normalized(selection['candidates'])):
        raise ValueError('selection omitted or changed an eligible build')
    best = min(candidates, key=lambda row: (row['objective'], json.dumps(row['build'], sort_keys=True)))
    if not same_structure(normalized([best]), normalized([selection['selected']])):
        raise ValueError('selection did not choose the fastest eligible build')
    write(args.output / 'finished.json', {'complete': True, 'engine': engine, 'dataset': dataset,
          'calibrationFreezeSha256': selection['freezeSha256'], 'selectionSha256': sha(args.calibration / 'selection.json'),
          'auditorSha256': sha(Path(__file__)), 'cells': audited, 'heldOutVectorsOpened': False,
          'rejectedWithoutRaw': rejected_without_raw,
          'oracleMaximumNumericalDifference': oracle_difference,
          'scope': 'Every declared cell summary and build selection checked; raw responses verified for the selected filter configuration in every eligible build/state. Pre-recording rejections without raw responses are listed explicitly. Rejected configurations, host isolation and resource ceilings are not independently audited here.'})


if __name__ == '__main__':
    main()
