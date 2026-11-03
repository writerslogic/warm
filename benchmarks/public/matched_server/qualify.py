# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Execute one frozen serving job; performance claims require later evidence review."""

import argparse
from contextlib import contextmanager
import gc
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import time
import traceback

import numpy as np

from adapters import CLIENTS, FILTERS, SYSTEMS, payload, validate_hits
from calibrate import Guard, sha, write
from distribution import verify_installed
from measure import audit_scores, payload_digest, query_order, run_cell
from oracle import truth
from seal_qualification import current_binaries, current_sources


HERE = Path(__file__).parent


def job_order(plan):
    return [{'dataset': dataset, 'repetition': repetition, 'engine': engine}
            for dataset in plan['qualification']['datasetOrder']
            for repetition, engines in enumerate(plan['qualification']['engineOrder'])
            for engine in engines]


def resource_environment(limit, proc=Path('/proc'), cgroup=Path('/sys/fs/cgroup')):
    memory = {}
    for line in (proc / 'meminfo').read_text().splitlines():
        if line.startswith(('MemTotal:', 'SwapTotal:')):
            key, value, units = line.split()
            if units != 'kB':
                raise ValueError('unexpected kernel memory units')
            memory[key[:-1]] = int(value) * 1024
    caps = {'memory': memory['MemTotal'], 'swap': memory['SwapTotal']}
    records = []
    for line in (proc / 'self/cgroup').read_text().splitlines():
        if not line.startswith('0::'):
            continue
        relative = Path(line[3:].lstrip('/'))
        if '..' in relative.parts:
            raise ValueError('unexpected cgroup path')
        folder = cgroup / relative
        while folder == cgroup or cgroup in folder.parents:
            entry = {'directory': str(folder)}
            for kind, name in [('memory', 'memory.max'), ('swap', 'memory.swap.max')]:
                path = folder / name
                if path.exists():
                    value = path.read_text().strip()
                    entry[name] = value
                    if value != 'max':
                        caps[kind] = min(caps[kind], int(value))
            records.append(entry)
            if folder == cgroup:
                break
            folder = folder.parent
    if caps['memory'] > limit or caps['swap'] != 0:
        raise ValueError('run requires an effective memory cap within eight GiB and disabled swap')
    return {'hostMemory': memory, 'effectiveCaps': caps, 'cgroups': records}


def store_bytes(root):
    total, seen = 0, set()
    for folder, directories, files in os.walk(root, followlinks=False):
        for name in files:
            path = Path(folder) / name
            try:
                stat = path.lstat()
            except FileNotFoundError:
                continue
            identity = stat.st_dev, stat.st_ino
            if identity not in seen:
                seen.add(identity)
                total += stat.st_blocks * 512
    return total


@contextmanager
def operation_deadline(seconds):
    def expired(signum, frame):
        raise TimeoutError('operation exceeded its declared deadline')

    if signal.getitimer(signal.ITIMER_REAL)[0] != 0:
        raise RuntimeError('cannot replace an existing operation timer')
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def verify_changed_documents(system):
    system.close()
    system.connect()
    for ordinal in range(100):
        actual, expected = system.fetch(ordinal), payload(ordinal, 1)
        if actual is None or any(actual.get(name) != expected[name] for name in expected):
            raise RuntimeError(f'acknowledged replacement payload mismatch: {ordinal}')
    for ordinal in range(100, 200):
        if system.fetch(ordinal) is not None:
            raise RuntimeError(f'acknowledged deletion still visible: {ordinal}')


def cold_open(system, engine, configuration, query, train, expected, limits):
    preparer = CLIENTS[engine](system.port, configuration, 'all')
    try:
        request = preparer.prepare(query)
    finally:
        preparer.close()
    system.stop()
    started = time.perf_counter()
    system.start()
    ready = time.perf_counter()
    client = CLIENTS[engine](system.port, configuration, 'all')
    try:
        hits = client.query(request)
        received = time.perf_counter()
        identities = validate_hits(hits, len(train), True, 'all')
        score_error = audit_scores(engine, hits, query, train, True)
        payload_hash = payload_digest(hits)
        if payload_hash != payload_digest([payload(int(hit['id']), int(int(hit['id']) < 100)) for hit in hits]):
            raise ValueError('cold-open payload type mismatch')
        validated = time.perf_counter()
    finally:
        client.close()
    recall = len(identities & set(expected['ids'])) / len(expected['ids'])
    exact_error = configuration['route'] == 'exact' and identities != set(expected['ids'])
    return {'startedAt': started, 'readyAt': ready, 'receivedAt': received, 'validatedAt': validated,
            'readySeconds': ready - started, 'firstValidatedResponseSeconds': validated - started,
            'recall': recall, 'maximumScoreError': score_error, 'hits': sorted(identities),
            'returned': [{'id': hit['id'], 'score': hit['score']} for hit in hits], 'payloadSha256': payload_hash,
            'expected': expected['ids'], 'unresolvedExactMembership': int(exact_error),
            'pass': recall >= .9 and not exact_error and ready - started <= limits['startupSeconds']
                    and received - ready <= limits['querySeconds'],
            'cache': 'Fresh server process; filesystem page cache retained. One request prepared before restart.'}


def run_job(*, engine, dataset, repetition, selected, plan, train_path, queries,
            warmup, expected, output, hardware, qualification=False):
    rows, dimensions = plan['rows'], plan['datasets'][dataset]
    output.mkdir()
    train = np.memmap(train_path, dtype='<f4', mode='r', shape=(rows, dimensions))
    system = SYSTEMS[engine](output / 'server', 4, 'serving-qualification' if qualification else 'serving-development', dim=dimensions)
    cells, phases, failure, cold = [], [], None, None
    build = selected['build']
    limits = plan['resources']
    guard = Guard(system, output, limits)
    try:
        with guard:
            start = time.perf_counter()
            system.start()
            system.create(build)
            system.ingest(train, build)
            system.wait_queryable(build)
            phases.append({'phase': 'build', 'seconds': time.perf_counter() - start,
                           'settings': system.inspect(), 'allocatedStoreBytes': store_bytes(system.data)})
            for state in plan['states']:
                churn = state != 'baseline'
                if state == 'churn-before-maintenance':
                    system.close()
                    system.connect()
                    operations = []
                    for kind, ordinals in [('replace', range(100)), ('delete', range(100, 200))]:
                        for ordinal in ordinals:
                            start = time.perf_counter()
                            with operation_deadline(limits['mutationSeconds']):
                                if kind == 'replace':
                                    system.replace_one(ordinal, train[ordinal + 1])
                                else:
                                    system.delete_one(ordinal)
                            elapsed = time.perf_counter() - start
                            operations.append({'operation': kind, 'id': ordinal, 'seconds': elapsed})
                            write(output / f'{kind}-{ordinal}.json', operations[-1])
                            if elapsed > limits['mutationSeconds']:
                                raise TimeoutError('acknowledged mutation exceeded declared latency limit')
                    verify_changed_documents(system)
                    phases.append({'phase': 'churn', 'operations': operations,
                                   'acknowledgedReplacementsChecked': 100, 'acknowledgedDeletionsChecked': 100})
                elif state == 'churn-after-maintenance':
                    system.close()
                    system.connect()
                    start = time.perf_counter()
                    system.maintain(build)
                    phases.append({'phase': 'maintenance', 'seconds': time.perf_counter() - start,
                                   'settings': system.inspect()})
                elif state == 'reopen':
                    ordinal = query_order(dataset, repetition, len(queries))[0]
                    cold = cold_open(system, engine, selected['filters']['all'], queries[ordinal], train,
                                     expected['churn']['all'][ordinal], limits)
                    cold['query'] = ordinal
                    write(output / 'cold-open.json', cold)
                    verify_changed_documents(system)
                allocated = store_bytes(system.data)
                phases.append({'phase': state, 'allocatedStoreBytes': allocated})
                if allocated > limits['maxStoreBytes']:
                    raise RuntimeError('allocated store exceeds declared ceiling')
                for filter_name in FILTERS:
                    for clients in plan['qualification']['concurrency']:
                        guard.check()
                        system.server.sampling(False)
                        try:
                            result = run_cell(engine=engine, port=system.port, configuration=selected['filters'][filter_name],
                                filter_name=filter_name, dataset=dataset, repetition=repetition, clients=clients,
                                queries=queries, warmup=warmup[:plan['qualification']['warmQueries']],
                                expected=expected['churn' if churn else 'baseline'][filter_name],
                                train_path=train_path, shape=train.shape, churn=churn,
                                output=output / f'{state}-{filter_name}-{clients}', limits=limits,
                                server_pgid=system.server.proc.pid, qualification=qualification)
                        finally:
                            system.server.sampling(True)
                        cells.append({**result, 'state': state, 'hardware': hardware,
                                      'evidenceVerified': False, 'isolationPassed': False})
                        print(json.dumps({'state': state, 'filter': filter_name, 'clients': clients,
                                          'complete': result['complete'], 'pass': result['pass']}), flush=True)
                        guard.check()
                        if not system.server.alive() or not result['complete']:
                            raise RuntimeError('server or measurement client failed during serving job')
            guard.check()
            allocated = store_bytes(system.data)
            if allocated > limits['maxStoreBytes']:
                raise RuntimeError('final store exceeds declared ceiling')
            phases.append({'phase': 'final', 'allocatedStoreBytes': allocated})
    except Exception as error:
        failure = {'type': type(error).__name__, 'message': str(error), 'traceback': traceback.format_exc()}
    finally:
        try:
            system.stop()
        except Exception as error:
            failure = failure or {'type': type(error).__name__, 'message': str(error), 'phase': 'server shutdown'}
        result = {'complete': failure is None, 'failure': failure, 'cells': cells, 'phases': phases,
                  'cold': cold, 'engine': engine, 'dataset': dataset, 'repetition': repetition,
                  'hardware': hardware, 'developmentOnly': not qualification,
                  'peakServerPssBytes': guard.peak,
                  'qualityPassed': failure is None and all(row['pass'] for row in cells) and cold is not None and cold['pass'],
                  'claimQualified': False,
                  'pending': ['Independent raw-evidence verification', 'Physical-host isolation evidence',
                              'Complete matrix and comparison gates', 'Installed-component/recovery evidence review']}
        write(output / 'finished.json', result)
    return result


def prepare_reference(input_folder, output, rows, dimensions, queries):
    import resource
    train = np.memmap(input_folder / 'train.f32', dtype='<f4', mode='r', shape=(rows, dimensions))
    questions = np.fromfile(input_folder / 'test.f32', dtype='<f4').reshape(queries, dimensions)
    expected = {'baseline': truth(train, questions), 'churn': truth(train, questions, True)}
    write(output, expected)
    write(output.with_suffix('.preparation.json'), {
        'oracleSha256': sha(output), 'peakPreparationRssBytes': resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024,
        'scope': 'Offline exact-reference preparation in an exiting process, before any serving measurements.'})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--seal', type=Path, required=True)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--job', type=int, required=True)
    args = parser.parse_args()
    if platform.system() != 'Linux' or len(os.sched_getaffinity(0)) != 4:
        parser.error('requires four-CPU Linux affinity')
    if any(os.environ.get(name) != '1' for name in ('OPENBLAS_NUM_THREADS', 'OMP_NUM_THREADS')):
        parser.error('set OPENBLAS_NUM_THREADS=1 and OMP_NUM_THREADS=1 before launching')
    if any(not path.is_absolute() for path in (args.seal, args.inputs, args.output)):
        parser.error('paths must be absolute')
    seal = json.loads(args.seal.read_text())
    if seal.get('schema') != 'warm.qualification-configuration-seal/1' or seal.get('heldOutVectorsOpenedBySealer') is not False:
        raise ValueError('missing pre-query configuration seal')
    plan = json.loads((HERE / 'PLAN.json').read_text())
    sources = {path.name: sha(path) for path in sorted(HERE.glob('*.py'))}
    if seal['plan'] != plan or seal['sources'] != current_sources() or seal['qualificationSourceHashes'] != sources:
        raise ValueError('qualification source changed after selection seal')
    binaries = current_binaries()
    if binaries != seal['binaries'] or platform.node() != seal['host']['hostname']:
        raise ValueError('calibrated host or executable changed')
    verify_installed(Path(os.environ['WARM_MATCHED_OPENSEARCH_HOME']), seal['opensearchDistribution']['files'])
    jobs = job_order(plan)
    if not 0 <= args.job < len(jobs):
        parser.error('job index is outside the frozen sequence')
    environment = resource_environment(plan['resources']['guestOrCgroupBytes'])
    if not args.output.exists():
        args.output.mkdir()
    sequence = args.output / 'sequence.json'
    if not sequence.exists():
        if args.job != 0:
            raise ValueError('qualification must start at job zero')
        write(sequence, {'sealSha256': sha(args.seal), 'jobs': jobs})
    if json.loads(sequence.read_text()) != {'sealSha256': sha(args.seal), 'jobs': jobs}:
        raise ValueError('qualification sequence or seal changed')
    if args.job:
        previous = json.loads((args.output / f'job-{args.job-1:02d}' / 'finished.json').read_text())
        verification = json.loads((args.output / f'job-{args.job-1:02d}-source-verification.json').read_text())
        if not previous['complete'] or verification.get('pass') is not True:
            raise ValueError('prior job failed before completing; preserve it and resolve the failure first')
    if shutil.disk_usage(args.output).free < plan['resources']['maxStoreBytes'] + 2**30:
        raise RuntimeError('insufficient free disk for the declared store ceiling')
    job = jobs[args.job]
    dataset, engine, repetition = (job[name] for name in ('dataset', 'engine', 'repetition'))
    folder = args.inputs / dataset
    receipt = seal['calibrationEvidence'][dataset]['inputReceipt']
    for name in ('train.f32', 'calibration.f32', 'test.f32'):
        if sha(folder / name) != receipt['files'][name]['sha256']:
            raise ValueError('frozen input digest mismatch: ' + name)
    reference = args.output / (dataset + '-reference.json')
    if not reference.exists():
        with (args.output / (dataset + '-reference.log')).open('x') as log:
            subprocess.run([sys.executable, str(Path(__file__).resolve()), '--prepare-reference', str(folder), str(reference),
                            str(plan['rows']), str(plan['datasets'][dataset]), str(plan['queries'])],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=7200)
    reference_receipt = json.loads(reference.with_suffix('.preparation.json').read_text())
    if sha(reference) != reference_receipt['oracleSha256']:
        raise ValueError('offline oracle changed after preparation')
    queries = np.fromfile(folder / 'test.f32', dtype='<f4').reshape(plan['queries'], plan['datasets'][dataset])
    warmup = np.fromfile(folder / 'calibration.f32', dtype='<f4').reshape(plan['queries'], plan['datasets'][dataset])
    expected = json.loads(reference.read_text())
    write(args.output / f'job-{args.job:02d}-freeze.json', {
        'job': job, 'configurationSealSha256': sha(args.seal), 'oracleSha256': sha(reference),
        'environment': environment, 'affinity': sorted(os.sched_getaffinity(0)),
        'started': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'scope': 'Serving measurements; offline reference preparation is recorded separately.'})
    gc.collect()
    result = run_job(engine=engine, dataset=dataset, repetition=repetition,
        selected=seal['selected'][dataset][engine], plan=plan, train_path=folder / 'train.f32',
        queries=queries, warmup=warmup, expected=expected, output=args.output / f'job-{args.job:02d}',
        hardware=seal['hardware'], qualification=True)
    matched = ({path.name: sha(path) for path in sorted(HERE.glob('*.py'))} == sources
               and current_sources() == seal['sources'] and current_binaries() == binaries)
    try:
        verify_installed(Path(os.environ['WARM_MATCHED_OPENSEARCH_HOME']), seal['opensearchDistribution']['files'])
    except (OSError, ValueError):
        matched = False
    write(args.output / f'job-{args.job:02d}-source-verification.json', {'pass': matched,
          'configurationSealSha256': sha(args.seal), 'finishedSha256': sha(args.output / f'job-{args.job:02d}/finished.json')})
    if not matched:
        raise RuntimeError('frozen serving source changed during measurements')
    if not result['complete'] or not result['qualityPassed']:
        raise SystemExit(1)


if __name__ == '__main__':
    if len(sys.argv) == 7 and sys.argv[1] == '--prepare-reference':
        prepare_reference(Path(sys.argv[2]), Path(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6]))
    else:
        main()
