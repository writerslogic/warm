# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Recompute cell evidence from raw responses and an independent cosine reference."""

import hashlib
import json
import math
from pathlib import Path

import numpy as np

from adapters import payload, validate_hits
from measure import audit_scores, partitions, payload_digest, query_order
from oracle import summarize


def read_rows(path):
    with path.open('rb') as stream:
        while line := stream.readline(65537):
            if len(line) > 65536 or not line.endswith(b'\n'):
                raise ValueError('oversized or incomplete raw evidence row')
            yield json.loads(line)


def same_number(left, right):
    return (type(left) in (int, float) and type(right) in (int, float)
            and math.isfinite(left) and math.isfinite(right)
            and math.isclose(left, right, rel_tol=1e-12, abs_tol=1e-12))


def verify_resources(folder, summary, limits):
    with (folder / 'resources.ndjson').open('rb') as stream:
        if hashlib.file_digest(stream, 'sha256').hexdigest() != summary['resourceSha256']:
            raise ValueError('resource evidence digest mismatch')
    if summary.get('resourceAccountingAvailable') is not True:
        return False
    samples = list(read_rows(folder / 'resources.ndjson'))
    if not samples or not any(row.get('cpuBaseline') is True for row in samples):
        raise ValueError('missing resource samples or CPU baseline')
    baseline, maximum = {}, {}
    peaks = {'serverPssBytes': 0, 'clientPssBytes': 0}
    for row in samples:
        if row.get('cpuBaseline') is True:
            baseline, maximum = dict(row['cpuSecondsByIdentity']), {}
        for identity, value in row['cpuSecondsByIdentity'].items():
            if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
                raise ValueError('invalid CPU evidence')
            maximum[identity] = max(value, maximum.get(identity, 0.))
        for name in peaks:
            value = row[name]
            if type(value) is not int or value < 0:
                raise ValueError('invalid PSS evidence')
            peaks[name] = max(peaks[name], value)
    for name, value in peaks.items():
        if summary['resources'][name] != value:
            raise ValueError('sampled memory peak mismatch')
    for group in ('server', 'client'):
        cpu = math.fsum(value - baseline.get(identity, 0.) for identity, value in maximum.items()
                        if identity.startswith(group + ':'))
        if not same_number(cpu, summary['resources']['sampledCpuSeconds'][group]):
            raise ValueError('sampled CPU total mismatch')
    return peaks['serverPssBytes'] <= limits['maxServerPssBytes'] and peaks['clientPssBytes'] <= limits['maxClientPssBytes']


def verify_cell(folder, *, expected, queries, train_path, shape, limits, identity=None):
    folder = Path(folder)
    summary = json.loads((folder / 'finished.json').read_text())
    if summary.get('complete') is not True:
        raise ValueError('incomplete measurement cannot be verified as a complete cell')
    identity_fields = {'engine', 'dataset', 'repetition', 'configuration', 'filter', 'churn', 'clients'}
    if summary.get('developmentOnly') is not True and (identity is None or set(identity) != identity_fields):
        raise ValueError('qualification verification requires its independently supplied cell identity')
    if identity is not None and any(summary[name] != value for name, value in identity.items()):
        raise ValueError('cell identity differs from its frozen job')
    count, clients = len(queries), summary['clients']
    order = query_order(summary['dataset'], summary['repetition'], count)
    slices = partitions(order, clients)
    if summary['order'] != order or summary['clientSlices'] != slices or len(expected) != count:
        raise ValueError('query order, slices or reference count mismatch')
    names = [f'client-{index}.ndjson' for index in range(clients)]
    if set(summary['rawSha256']) != set(names) or {path.name for path in folder.glob('client-*.ndjson')} != set(names):
        raise ValueError('missing or extra client evidence files')
    workers = {row['pid']: row for row in summary['workers']}
    if len(workers) != clients or len(summary['clientProcesses']) != clients:
        raise ValueError('missing or duplicate worker completion')
    if {row['pid'] for row in summary['clientProcesses']} != set(workers):
        raise ValueError('launched and completed worker identities differ')
    for worker in workers.values():
        if not same_number(worker['cpuSeconds'], worker['cpuSeconds']) or worker['cpuSeconds'] < 0:
            raise ValueError('invalid worker CPU total')
    if not same_number(math.fsum(row['cpuSeconds'] for row in workers.values()), summary['workerCpuSeconds']):
        raise ValueError('worker CPU aggregate mismatch')
    train = np.memmap(train_path, dtype='<f4', mode='r', shape=shape)
    rows = []
    for index, name in enumerate(names):
        with (folder / name).open('rb') as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != summary['rawSha256'][name]:
                raise ValueError('raw client digest mismatch')
        records = list(read_rows(folder / name))
        if [row['query'] for row in records] != slices[index]:
            raise ValueError('client query sequence mismatch')
        process = summary['clientProcesses'][index]
        if process['exitcode'] != 0 or process['pid'] not in workers:
            raise ValueError('client did not exit successfully')
        worker = workers[process['pid']]
        completed = 0
        for row in records:
            if type(row['query']) is not int:
                raise ValueError('query ordinal must be an integer')
            if row['expected'] != expected[row['query']]['ids']:
                raise ValueError('recorded reference differs from the independent oracle')
            if not (summary['commonStart'] <= worker['startedAt'] <= row['sentAt'] <= row['receivedAt']
                    <= row['validatedAt'] <= worker['finishedAt']):
                raise ValueError('invalid client timeline')
            if not same_number(row['seconds'], row['receivedAt'] - row['sentAt']):
                raise ValueError('latency boundary mismatch')
            if row['error'] is None:
                returned = row['returned']
                hits = [{**payload(int(hit['id']), int(summary['churn'] and int(hit['id']) < 100)),
                         'id': hit['id'], 'score': hit['score']} for hit in returned]
                identities = validate_hits(hits, shape[0], summary['churn'], summary['filter'])
                if row['hits'] != sorted(identities) or row['payloadSha256'] != payload_digest(hits):
                    raise ValueError('returned membership or canonical payload evidence mismatch')
                error = audit_scores(summary['engine'], hits, queries[row['query']], train, summary['churn'])
                if not same_number(error, row['maximumScoreError']):
                    raise ValueError('score audit differs from the raw returned scores')
                completed += 1
            elif not isinstance(row['error'], dict) or not row['error'].get('type') or not row['error'].get('message'):
                raise ValueError('malformed failure evidence')
        if worker['completed'] != completed or worker['attempted'] != len(records):
            raise ValueError('worker completion count mismatch')
        rows.extend(records)
    recomputed = summarize(rows, count, summary['configuration']['route'] == 'exact')
    for name, value in recomputed.items():
        if isinstance(value, bool):
            matched = summary[name] is value
        else:
            matched = same_number(summary[name], value)
        if not matched:
            raise ValueError('recomputed quality/latency mismatch: ' + name)
    completed = sum(row['completed'] for row in workers.values())
    elapsed = max(row['finishedAt'] for row in workers.values()) - summary['commonStart']
    if (elapsed <= 0 or summary['completed'] != completed or not same_number(summary['elapsedSeconds'], elapsed)
            or not same_number(summary['queriesPerSecond'], completed / elapsed)):
        raise ValueError('throughput boundary or completion count mismatch')
    return {'evidenceVerified': True, 'resourcesPassed': verify_resources(folder, summary, limits),
            'qualityPassed': recomputed['pass'], 'queries': count, 'completed': completed,
            'developmentOnly': summary['developmentOnly'], 'isolationPassed': False,
            'scope': 'Raw cell evidence checked; physical-host isolation and job/source receipts require separate verification.'}
