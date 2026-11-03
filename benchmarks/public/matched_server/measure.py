# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Separate-process clients with a common start and retained per-request evidence."""

import hashlib
import json
import math
import multiprocessing as mp
from multiprocessing.connection import wait
import os
from pathlib import Path
import time
import traceback

import numpy as np

from adapters import CLIENTS, payload, validate_hits
from oracle import normalize, summarize
from server_proc import tree_pids


def query_order(dataset, repetition, count, source_offset=1000):
    if type(repetition) is not int or repetition not in range(3):
        raise ValueError('repetition must be 0, 1 or 2')
    if type(count) is not int or not 1 <= count <= 1000:
        raise ValueError('query count must be within 1..1000')
    return sorted(range(count), key=lambda i: hashlib.sha256(
        f'{dataset}/{repetition}/{source_offset + i}'.encode()).digest())


def partitions(order, clients):
    if type(clients) is not int or clients not in (1, 8):
        raise ValueError('client count must be one or eight')
    if len(order) < clients or len(set(order)) != len(order):
        raise ValueError('query slices require distinct ordinals and a nonempty slice per client')
    return [order[len(order) * i // clients:len(order) * (i + 1) // clients]
            for i in range(clients)]


def audit_scores(engine, hits, query, train, churn):
    if not hits:
        return 0.
    ids = [int(hit['id']) for hit in hits]
    originals = [i + 1 if churn and i < 100 else i for i in ids]
    scores = normalize(train[originals]) @ normalize(query[None, :])[0]
    actual = np.array([hit['score'] for hit in hits], dtype=np.float64)
    if engine == 'opensearch':
        actual = 2 * actual - 1
    error = float(np.max(np.abs(actual - scores)))
    if not math.isfinite(error) or error > 1e-4:
        raise ValueError(f'returned cosine score mismatch: maximum absolute error {error}')
    if np.any(actual[1:] > actual[:-1] + 1e-4):
        raise ValueError('returned scores are not descending')
    return error


def payload_digest(hits):
    values = [{key: hit[key] for key in ('id', 'text', 'metadata')} for hit in hits]
    canonical = json.dumps(values, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()
    return hashlib.sha256(canonical).hexdigest()


def proc_sample(pid):
    root = Path('/proc') / str(pid)
    try:
        fields = (root / 'stat').read_text().rsplit(')', 1)[1].split()
        identity = f'{pid}:{fields[19]}'
        cpu = (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK')
        pss = next(int(line.split()[1]) * 1024 for line in
                   (root / 'smaps_rollup').read_text().splitlines() if line.startswith('Pss:'))
        return identity, cpu, pss
    except (FileNotFoundError, ProcessLookupError):
        return None


class Resources:
    def __init__(self, server_pgid, clients, limits, log):
        self.server_pgid, self.clients = server_pgid, clients
        self.limits, self.log = limits, log
        self.initial, self.maximum = {}, {}
        self.peaks = {'serverPssBytes': 0, 'clientPssBytes': 0}
        self.sample(initial=True)

    def sample(self, initial=False):
        if not Path('/proc/self/smaps_rollup').exists():
            if self.server_pgid is not None:
                raise RuntimeError('qualification resource accounting requires Linux procfs')
            return
        groups = {'client': [os.getpid(), *self.clients], 'server':
                  tree_pids(self.server_pgid) if self.server_pgid is not None else []}
        row = {'monotonic': time.perf_counter(), 'load': os.getloadavg(), 'cpuSecondsByIdentity': {},
               'cpuBaseline': initial}
        for group, pids in groups.items():
            memory = 0
            for pid in pids:
                sample = proc_sample(pid)
                if sample is None:
                    continue
                identity, cpu, pss = sample
                key = group + ':' + identity
                row['cpuSecondsByIdentity'][key] = cpu
                memory += pss
                if initial:
                    self.initial[key] = cpu
                self.maximum[key] = max(cpu, self.maximum.get(key, 0.))
            key = group + 'PssBytes'
            row[key] = memory
            self.peaks[key] = max(self.peaks[key], memory)
        self.log.write(json.dumps(row) + '\n')
        for group in ('server', 'client'):
            if row[group + 'PssBytes'] > self.limits['max' + group.title() + 'PssBytes']:
                raise RuntimeError(group + ' memory ceiling exceeded')

    def result(self):
        return {**self.peaks, 'sampledCpuSeconds': {
            group: math.fsum(value - self.initial.get(key, 0.) for key, value in self.maximum.items()
                             if key.startswith(group + ':')) for group in ('server', 'client')},
            'cpuLimit': 'Short-lived server children between samples can be missed; sampled CPU is a lower bound.',
            'memoryLimit': 'Sampled PSS peaks are lower bounds; parent and client processes are included.'}


def worker(engine, port, configuration, filter_name, ordinals, queries, warmup,
           expected, train_path, shape, churn, destination, connection, start_event):
    client = None
    try:
        train = np.memmap(train_path, dtype='<f4', mode='r', shape=shape)
        client = CLIENTS[engine](port, configuration, filter_name)
        if engine in ('warm', 'opensearch'):
            client.http.conn.connect()
        prepared = [(ordinal, client.prepare(queries[ordinal])) for ordinal in ordinals]
        for query in warmup:
            hits = client.query(client.prepare(query))
            validate_hits(hits, shape[0], churn, filter_name)
            audit_scores(engine, hits, query, train, churn)
        with destination.open('x', buffering=1) as log:
            connection.send({'stage': 'ready', 'pid': os.getpid()})
            if not start_event.wait(600):
                raise TimeoutError('coordinator did not release client start')
            cpu_start, first = time.process_time(), time.perf_counter()
            completed = 0
            for ordinal, request in prepared:
                hits, identities, payload_hash, score_error, error = [], [], None, None, None
                received = None
                sent = time.perf_counter()
                try:
                    hits = client.query(request)
                    received = time.perf_counter()
                    identities = sorted(validate_hits(hits, shape[0], churn, filter_name))
                    score_error = audit_scores(engine, hits, queries[ordinal], train, churn)
                    payload_hash = payload_digest(hits)
                    expected_payload = [payload(int(hit['id']), int(churn and int(hit['id']) < 100)) for hit in hits]
                    if payload_hash != payload_digest(expected_payload):
                        raise ValueError('canonical payload or metadata type mismatch')
                except Exception as failure:
                    if received is None:
                        received = time.perf_counter()
                    error = {'type': type(failure).__name__, 'message': str(failure)}
                validated = time.perf_counter()
                completed += error is None
                returned = [{'id': hit.get('id'), 'score': hit.get('score')
                             if type(hit.get('score')) in (int, float) and math.isfinite(hit['score'])
                             else repr(hit.get('score'))} for hit in hits if isinstance(hit, dict)] if isinstance(hits, list) else []
                record = {'query': ordinal, 'sentAt': sent, 'receivedAt': received,
                          'validatedAt': validated, 'seconds': received - sent,
                          'hits': identities, 'returned': returned, 'payloadSha256': payload_hash,
                          'maximumScoreError': score_error, 'expected': expected[ordinal]['ids'], 'error': error}
                log.write(json.dumps(record, allow_nan=False) + '\n')
            end = time.perf_counter()
            cpu = time.process_time() - cpu_start
        connection.send({'stage': 'done', 'pid': os.getpid(), 'startedAt': first,
                         'finishedAt': end, 'cpuSeconds': cpu, 'completed': completed,
                         'attempted': len(ordinals)})
    except BaseException as failure:
        connection.send({'stage': 'error', 'pid': os.getpid(),
                         'error': {'type': type(failure).__name__, 'message': str(failure),
                                   'traceback': traceback.format_exc()}})
    finally:
        if client is not None:
            client.close()
        connection.close()


def run_cell(*, engine, port, configuration, filter_name, dataset, repetition,
             clients, queries, warmup, expected, train_path, shape, churn, output,
             limits, server_pgid=None, qualification=False):
    if engine not in CLIENTS or len(expected) != len(queries):
        raise ValueError('unknown engine or inconsistent query/reference count')
    if qualification and (len(queries) != 1000 or shape[0] != 100000 or server_pgid is None
                          or not Path('/proc/self/smaps_rollup').exists()):
        raise ValueError('qualification requires the complete corpus, query count and Linux resource accounting')
    order = query_order(dataset, repetition, len(queries))
    slices = partitions(order, clients)
    output.mkdir()
    context = mp.get_context('spawn')
    event = context.Event()
    processes, receivers, done = [], [], []
    started, resource, failure = None, None, None
    controller_cpu = time.process_time()
    rows = []
    try:
        for index, ordinals in enumerate(slices):
            receiver, sender = context.Pipe(duplex=False)
            process = context.Process(target=worker, args=(engine, port, configuration, filter_name,
                ordinals, queries, warmup[len(warmup) * index // clients:len(warmup) * (index + 1) // clients],
                expected, str(train_path), shape, churn, output / f'client-{index}.ndjson', sender, event))
            try:
                process.start()
            except BaseException:
                receiver.close()
                sender.close()
                raise
            sender.close()
            processes.append(process)
            receivers.append(receiver)
        with (output / 'resources.ndjson').open('x', buffering=1) as log:
            resource = Resources(server_pgid, [p.pid for p in processes], limits, log)
            ready, active = set(), set(receivers)
            deadline = time.monotonic() + limits['startupSeconds']
            while active:
                if time.monotonic() > deadline:
                    raise TimeoutError('client phase deadline exceeded')
                for receiver in wait(list(active), timeout=limits['sampleSeconds']):
                    try:
                        message = receiver.recv()
                    except EOFError as error:
                        raise RuntimeError('client exited without a terminal result') from error
                    stage = message['stage']
                    if stage == 'error':
                        raise RuntimeError(json.dumps(message['error']))
                    if stage == 'ready':
                        if receiver in ready or started is not None:
                            raise RuntimeError('invalid duplicate or late readiness message')
                        ready.add(receiver)
                        if len(ready) == clients:
                            resource.initial.clear()
                            resource.maximum.clear()
                            resource.sample(initial=True)
                            controller_cpu = time.process_time()
                            started = time.perf_counter()
                            event.set()
                            deadline = time.monotonic() + math.ceil(len(queries) / clients) * limits['querySeconds'] + 60
                    elif stage == 'done':
                        if receiver not in ready or started is None:
                            raise RuntimeError('client completed before common start')
                        done.append(message)
                        active.remove(receiver)
                    else:
                        raise RuntimeError('unknown client message')
                resource.sample()
    except Exception as error:
        failure = {'type': type(error).__name__, 'message': str(error), 'traceback': traceback.format_exc()}
    finally:
        for process in processes:
            if process.is_alive() and failure is not None:
                process.terminate()
        for process in processes:
            process.join(timeout=5)
            if process.is_alive():
                process.kill()
                process.join(timeout=5)
            if process.exitcode != 0 and failure is None:
                failure = {'type': 'ChildProcessError', 'message': f'client {process.pid} exited {process.exitcode}'}
        for receiver in receivers:
            receiver.close()
    cpu = time.process_time() - controller_cpu
    for path in sorted(output.glob('client-*.ndjson')):
        with path.open() as stream:
            for line in stream:
                try:
                    rows.append(json.loads(line))
                except json.JSONDecodeError:
                    failure = failure or {'type': 'ValueError', 'message': 'truncated client evidence'}
    result = {'complete': failure is None, 'failure': failure, 'engine': engine,
              'dataset': dataset, 'repetition': repetition, 'clients': clients,
              'configuration': configuration, 'filter': filter_name, 'churn': churn,
              'order': order, 'clientSlices': slices, 'workers': done,
              'clientProcesses': [{'pid': process.pid, 'exitcode': process.exitcode} for process in processes],
              'controllerCpuSeconds': cpu, 'workerCpuSeconds': math.fsum(r['cpuSeconds'] for r in done),
              'resources': resource.result() if resource is not None else None,
              'resourceAccountingAvailable': Path('/proc/self/smaps_rollup').exists(),
              'payloadDigestFormat': 'SHA256 of UTF-8 JSON, sorted object keys, compact separators, result order preserved',
              'developmentOnly': not qualification, 'attempted': len(rows)}
    result['rawSha256'] = {}
    for path in sorted(output.glob('client-*.ndjson')):
        with path.open('rb') as stream:
            result['rawSha256'][path.name] = hashlib.file_digest(stream, 'sha256').hexdigest()
    result['resourceSha256'] = None
    if (output / 'resources.ndjson').exists():
        with (output / 'resources.ndjson').open('rb') as stream:
            result['resourceSha256'] = hashlib.file_digest(stream, 'sha256').hexdigest()
    if failure is None:
        try:
            result.update(summarize(rows, len(queries), configuration['route'] == 'exact'))
            elapsed = max(row['finishedAt'] for row in done) - started
            completed = sum(row['completed'] for row in done)
            result.update({'commonStart': started, 'elapsedSeconds': elapsed,
                           'completed': completed, 'queriesPerSecond': completed / elapsed})
        except Exception as error:
            result.update({'complete': False, 'failure': {'type': type(error).__name__, 'message': str(error)}})
    result['pass'] = result.get('pass', False) and result['complete']
    with (output / 'finished.json').open('x') as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
        stream.write('\n')
    return result
