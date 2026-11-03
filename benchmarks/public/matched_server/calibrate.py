# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Freeze and calibrate the declared grid using exposed queries only."""

import argparse
import gc
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import signal
import threading
import time
import traceback

import numpy as np

from adapters import CLIENTS, FILTERS, SYSTEMS, encode, validate_hits
from oracle import summarize, truth


HERE = Path(__file__).resolve().parent


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def configurations(engine, build, name, plan):
    spec = plan['calibration'][engine]
    yield {'route': 'exact'}
    if engine == 'warm':
        if name == 'all':
            for ef in spec['ef']:
                yield {'route': 'graph', 'ef': ef}
    elif engine == 'opensearch':
        for ef in spec['ef']:
            for count in spec['candidateK']:
                yield {'route': 'knn', 'ef': ef, 'candidateK': count}
    else:
        for route in ['planner', 'iterative']:
            if build['index'] == 'hnsw':
                for ef in spec['ef']:
                    yield {'route': route, 'ef': ef}
            else:
                for probes in spec['probes']:
                    if probes <= build['lists']:
                        yield {'route': route, 'probes': probes, 'lists': build['lists']}


class Guard:
    def __init__(self, system, output, limits):
        self.system, self.output, self.limits = system, output, limits
        self.deadline = time.monotonic() + limits['buildSeconds']
        self.stop_event, self.failure = threading.Event(), None
        self.peak = 0
        self.thread = threading.Thread(target=self.run, daemon=True)

    def run(self):
        with (self.output / 'resources.ndjson').open('x', buffering=1) as log:
            while not self.stop_event.wait(self.limits['sampleSeconds']):
                rss = self.system.server.sample() if self.system.server is not None else 0
                self.peak = max(self.peak, rss)
                log.write(json.dumps({'monotonic': time.monotonic(), 'serverPssBytes': rss,
                                      'load': os.getloadavg()}) + '\n')
                if rss > self.limits['maxServerPssBytes'] or time.monotonic() > self.deadline:
                    self.failure = 'server memory or per-build calibration deadline exceeded'
                    if self.system.server is not None and self.system.server.alive():
                        try:
                            os.killpg(self.system.server.proc.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    return

    def check(self):
        if self.failure:
            raise RuntimeError(self.failure)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *args):
        self.stop_event.set()
        self.thread.join()


def select_configuration(cells, candidates, states):
    eligible = []
    for configuration in candidates:
        matching = [row for row in cells if row['configuration'] == configuration]
        if len(matching) == len(states) and {row['state'] for row in matching} == set(states) and all(row['pass'] for row in matching):
            objective = math.exp(math.fsum(math.log(row['median']) for row in matching) / len(matching))
            eligible.append({'configuration': configuration, 'median': objective, 'states': matching})
    return min(eligible, key=lambda row: (row['median'], json.dumps(row['configuration'], sort_keys=True))) if eligible else None


def cell(engine, port, config, name, queries, expected, destination, plan, churn):
    client = CLIENTS[engine](port, config, name)
    records = []
    try:
        prepared = [client.prepare(query) for query in queries]
        for request in prepared[:plan['calibration']['warmQueries']]:
            validate_hits(client.query(request), plan['rows'], churn, name)
        with destination.open('x', buffering=1) as log:
            for ordinal, request in enumerate(prepared):
                start = time.perf_counter()
                error = None
                hits, identities, payload_hash = [], [], None
                try:
                    hits = client.query(request)
                    elapsed = time.perf_counter() - start
                    identities = sorted(validate_hits(hits, plan['rows'], churn, name))
                    payload_hash = hashlib.sha256(encode([{key: hit[key] for key in ('id', 'text', 'metadata')}
                                                          for hit in hits])).hexdigest()
                except Exception as failure:
                    elapsed = time.perf_counter() - start
                    error = {'type': type(failure).__name__, 'message': str(failure)}
                record = {'query': ordinal, 'seconds': elapsed, 'hits': identities,
                          'returned': [{'id': hit.get('id'), 'score': hit.get('score')} for hit in hits],
                          'payloadSha256': payload_hash, 'expected': expected[ordinal]['ids'], 'error': error}
                records.append(record)
                log.write(json.dumps(record, allow_nan=False) + '\n')
                if error:
                    raise RuntimeError('calibration cell rejected at first request/payload error: ' + json.dumps(error))
    finally:
        client.close()
    return summarize(records, plan['queries'], config['route'] == 'exact')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--engine', choices=SYSTEMS, required=True)
    parser.add_argument('--dataset', required=True)
    parser.add_argument('--installation-evidence', type=Path, required=True)
    args = parser.parse_args()
    plan = json.loads((HERE / 'PLAN.json').read_text())
    if platform.system() != 'Linux' or args.dataset not in plan['datasets']:
        parser.error('requires Linux and a declared corpus')
    if any(not path.is_absolute() for path in [args.inputs, args.output, args.installation_evidence]):
        parser.error('paths must be absolute')
    if len(os.sched_getaffinity(0)) != 4:
        parser.error('calibration requires the declared four-CPU affinity')
    preparation = json.loads((args.inputs / 'finished.json').read_text())
    if preparation.get('complete') is not True or sha(args.inputs / 'freeze.json') != plan['inputFreezeSha256']:
        raise ValueError('input seal mismatch')
    record = json.loads((args.inputs / args.dataset / 'inputs.json').read_text())
    expected_input = next(item['inputsSha256'] for item in preparation['completed'] if item['name'] == args.dataset)
    if sha(args.inputs / args.dataset / 'inputs.json') != expected_input:
        raise ValueError('dataset receipt mismatch')
    for filename in ['train.f32', 'calibration.f32']:
        if sha(args.inputs / args.dataset / filename) != record['files'][filename]['sha256']:
            raise ValueError('training/calibration input digest mismatch')
    args.output.mkdir()
    source_files = sorted(HERE.glob('*.py')) + [HERE / 'PLAN.json', HERE / 'pyproject.toml', HERE / 'uv.lock']
    source_files += [HERE.parents[1] / 'competitive/warmbench' / name
                     for name in ['server_proc.py', 'server_worker.py', 'servers.py', 'ann_worker.py', 'text_worker.py']]
    source_hashes = {str(path): sha(path) for path in source_files}
    installation = {path.name: sha(path) for path in sorted(args.installation_evidence.iterdir()) if path.is_file()}
    freeze = {'plan': plan, 'sources': source_hashes, 'inputReceiptSha256': expected_input,
              'engine': args.engine, 'dataset': args.dataset, 'installationEvidence': installation,
              'warmBinarySha256': sha(Path(os.environ['WARM_BENCH_WARM_SERVER'])),
              'postgresBinarySha256': sha(Path(os.environ['WARM_BENCH_PG_BIN']) / 'postgres'),
              'pgvectorBinarySha256': sha(Path(os.environ['WARM_BENCH_PG_BIN']).parent / 'lib/vector.so'),
              'machine': {'platform': platform.platform(), 'affinity': sorted(os.sched_getaffinity(0)),
                          'cpuinfo': Path('/proc/cpuinfo').read_text(), 'meminfo': Path('/proc/meminfo').read_text(),
                          'swaps': Path('/proc/swaps').read_text()},
              'python': platform.python_version(), 'numpy': np.__version__,
              'started': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'claim': 'Calibration only; no held-out query loaded; no qualified performance claim.'}
    write(args.output / 'freeze.json', freeze)
    dimension = plan['datasets'][args.dataset]
    train = np.fromfile(args.inputs / args.dataset / 'train.f32', dtype='<f4').reshape(plan['rows'], dimension)
    queries = np.fromfile(args.inputs / args.dataset / 'calibration.f32', dtype='<f4').reshape(plan['queries'], dimension)
    expected = {'baseline': truth(train, queries), 'churn': truth(train, queries, True)}
    write(args.output / 'oracle.json', expected)
    gc.collect()
    candidates, failures = [], []
    for number, build in enumerate(plan['calibration'][args.engine]['builds']):
        folder = args.output / f'build-{number}'
        folder.mkdir()
        system = SYSTEMS[args.engine](folder / 'server', 4, 'calibration', dim=dimension)
        cells, selected = [], {}
        failure = None
        try:
            with Guard(system, folder, plan['resources']) as guard:
                start = time.monotonic()
                system.start()
                system.create(build)
                system.ingest(train, build)
                system.wait_queryable(build)
                build_seconds = time.monotonic() - start
                guard.check()
                write(folder / 'built.json', {'seconds': build_seconds, 'settings': system.inspect()})
                for state in plan['states']:
                    churn = state != 'baseline'
                    if state == 'churn-before-maintenance':
                        system.close()
                        system.connect()
                        start = time.monotonic()
                        for ordinal in range(100):
                            system.replace_one(ordinal, train[ordinal + 1])
                        for ordinal in range(100, 200):
                            system.delete_one(ordinal)
                        write(folder / 'churn.json', {'seconds': time.monotonic() - start, 'settings': system.inspect()})
                    elif state == 'churn-after-maintenance':
                        system.close()
                        system.connect()
                        start = time.monotonic()
                        system.maintain(build)
                        write(folder / 'maintenance.json', {'seconds': time.monotonic() - start, 'settings': system.inspect()})
                    elif state == 'reopen':
                        system.stop()
                        start = time.monotonic()
                        system.start()
                        write(folder / 'reopen.json', {'readySeconds': time.monotonic() - start, 'settings': system.inspect()})
                    for name in FILTERS:
                        for index, configuration in enumerate(configurations(args.engine, build, name, plan)):
                            guard.check()
                            details = {'filter': name, 'state': state, 'configuration': configuration}
                            try:
                                details.update(cell(args.engine, system.port, configuration, name, queries,
                                                    expected['churn' if churn else 'baseline'][name],
                                                    folder / f'{state}-{name}-{index}.ndjson', plan, churn))
                            except Exception as error:
                                details.update({'pass': False, 'error': {'type': type(error).__name__, 'message': str(error)}})
                            cells.append(details)
                            write(folder / f'{state}-{name}-{index}-summary.json', details)
                            print(json.dumps({'build': number, **details}), flush=True)
                            if not system.server.alive():
                                raise RuntimeError('server exited during calibration')
                for name in FILTERS:
                    chosen = select_configuration([row for row in cells if row['filter'] == name],
                                                  configurations(args.engine, build, name, plan), plan['states'])
                    if chosen is not None:
                        selected[name] = chosen
                guard.check()
                if len(selected) == len(FILTERS):
                    candidates.append({'build': build, 'directory': str(folder), 'filters': selected,
                        'objective': math.exp(math.fsum(math.log(row['median']) for row in selected.values()) / len(selected))})
        except Exception as error:
            failure = {'type': type(error).__name__, 'message': str(error), 'traceback': traceback.format_exc()}
            failures.append({'build': number, 'failure': failure})
            print(json.dumps({'build': number, 'failure': failure}), flush=True)
        finally:
            try:
                system.stop()
            finally:
                write(folder / 'finished.json', {'complete': failure is None, 'failure': failure,
                      'build': build, 'cells': cells, 'selected': selected,
                      'peakServerPssBytes': guard.peak})
    if {str(path): sha(path) for path in source_files} != source_hashes:
        raise RuntimeError('driver changed during calibration')
    chosen = min(candidates, key=lambda row: (row['objective'], json.dumps(row['build'], sort_keys=True))) if candidates else None
    write(args.output / 'selection.json', {'complete': not failures, 'eligible': chosen is not None,
          'selected': chosen, 'candidates': candidates, 'failures': failures, 'freezeSha256': sha(args.output / 'freeze.json'),
          'selectedBeforeHeldOutQueries': True})
    if failures or chosen is None:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
