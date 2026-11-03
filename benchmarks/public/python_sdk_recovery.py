# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Installed HTTP SDK recovery at acknowledged process-death boundaries."""

import argparse
from dataclasses import asdict
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import time

from writerslogic_warm_sdk import WarmClient


KEY = 'isolated-sdk-recovery-fixture'
VECTOR = [1.0] * 24


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, payload):
    with path.open('x') as stream:
        json.dump(payload, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


class Server:
    def __init__(self, binary, root):
        self.binary, self.root, self.process = binary, root, None
        self.generation = 0

    def start(self):
        if self.process is not None:
            raise RuntimeError('server already owned')
        self.generation += 1
        log = self.root / f'server-{self.generation}.log'
        environment = {key: value for key, value in os.environ.items() if not key.startswith(('WARM_', 'HMS_'))}
        environment.update({'WARM_API_KEY': KEY, 'RAYON_NUM_THREADS': '2'})
        command = [str(self.binary), '--bind', '127.0.0.1:0', '--input-dim', '24', '--dim', '4096',
                   '--data-dir', str(self.root / 'data'), '--log-level', 'warn']
        with log.open('xb', buffering=0) as stream:
            self.process = subprocess.Popen(command, stdout=stream, stderr=stream, env=environment, start_new_session=True)
        started = time.monotonic()
        try:
            while time.monotonic() - started < 30:
                if self.process.poll() is not None:
                    raise RuntimeError('server exited during readiness')
                with log.open('rb') as stream:
                    data = stream.read(65537)
                if len(data) > 65536:
                    raise RuntimeError('unexpectedly large readiness output')
                for line in data.splitlines():
                    if line.startswith(b'warm-server listening on '):
                        address = line.removeprefix(b'warm-server listening on ').decode('ascii')
                        prefix, separator, port = address.rpartition(':')
                        if prefix != '127.0.0.1' or separator != ':' or not port.isdecimal() or not 1 <= int(port) <= 65535:
                            raise RuntimeError('unexpected server listen address')
                        return 'http://' + address, time.monotonic() - started
                time.sleep(.02)
            raise TimeoutError('server readiness exceeded thirty seconds')
        except BaseException:
            self.close()
            raise

    def stop(self, interruption):
        if self.process is None or self.process.poll() is not None:
            raise RuntimeError('server died before the declared boundary')
        self.process.send_signal(interruption)
        result = self.process.wait(timeout=30)
        if (interruption == signal.SIGKILL and result != -signal.SIGKILL) or (
                interruption == signal.SIGTERM and result != 0):
            raise RuntimeError('unexpected server shutdown result')
        self.process = None
        return result

    def close(self):
        if self.process is not None:
            if self.process.poll() is None:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            self.process.wait(timeout=30)
            self.process = None


def facts():
    return [{'id': f'fact-{i}', 'text': f'Batch evidence {i}', 'metadata': {'ordinal': i, 'verified': True}}
            for i in range(32)]


def documents(revision):
    return [{'id': f'doc-{i}', 'text': f'Document {i} revision {revision}', 'vector': VECTOR,
             'metadata': {'ordinal': i, 'revision': revision, 'verified': True}} for i in range(32)]


def city(new=False):
    payload = {'id': 'berlin' if new else 'paris', 'text': 'Alice city Berlin' if new else 'Alice city Paris',
               'subject': 'alice/city', 'valid_from': 2000 if new else 1000,
               'source_uri': 'urn:qualification:berlin' if new else 'urn:qualification:paris'}
    if new:
        payload['supersedes'] = ['paris']
    return payload


def ids(items):
    return {item.id for item in items}


def run_case(binary, output, interruption):
    output.mkdir()
    server = Server(binary, output)
    try:
        url, initial_start = server.start()
        with WarmClient(url=url, api_key=KEY, tenant_id='qualification') as client:
            memory = client.memory
            batch = memory.remember_many(facts())
            assert len(batch) == 32 and all(item.created for item in batch)
            first = memory.remember(**city())
            deadline = time.monotonic() + 2
            while time.time_ns() // 1_000_000 <= first.ingest_time:
                if time.monotonic() > deadline:
                    raise TimeoutError('clock did not advance for distinct known-time observations')
                time.sleep(.001)
            second = memory.remember(**city(True))
            assert second.ingest_time > first.ingest_time
            memory.remember('Must remain forgotten', id='forgotten')
            forgotten = memory.forget(ids=['forgotten'])
            assert forgotten.forgotten == ['forgotten']
            client.add_documents(documents(0))
            client.add_documents(documents(1))
            client.delete_document('doc-7')
            ledger = {'batch': [asdict(item) for item in batch], 'city': [asdict(first), asdict(second)],
                      'forgotten': asdict(forgotten), 'documentRevision': 1, 'deletedDocument': 'doc-7',
                      'boundary': 'Successful public HTTP responses; client receipt fsynced before interruption'}
            write(output / 'acknowledged.json', ledger)
        stopped = server.stop(interruption)
        url, recovery = server.start()
        with WarmClient(url=url, api_key=KEY, tenant_id='qualification') as client:
            memory = client.memory
            for item in facts():
                stored = memory.get(item['id'])
                assert stored is not None and stored.text == item['text']
                assert json.dumps(stored.metadata, sort_keys=True) == json.dumps(item['metadata'], sort_keys=True)
            assert memory.get('forgotten') is None
            assert memory.get('paris').status == 'superseded'
            assert memory.get('berlin').status == 'current'
            assert [item.id for item in memory.history('paris')] == ['paris', 'berlin']
            options = {'subject': 'alice/city', 'k': 10, 'lexical_weight': 1, 'semantic_weight': 0}
            assert ids(memory.recall('Alice city', as_of=1500, **options)) == {'paris'}
            assert ids(memory.recall('Alice city', as_of=2500, **options)) == {'berlin'}
            assert ids(memory.recall('Alice city', as_of=2500, known_at=ledger['city'][0]['ingest_time'], **options)) == {'paris'}
            returned = memory.recall('Alice city', as_of=2500, explain=True, **options)
            assert returned[0].source_uri == city(True)['source_uri']
            assert returned[0].explanation.status == 'current'
            retry = memory.remember_many(facts())
            assert len(retry) == 32 and not any(item.created for item in retry)
            assert memory.remember(**city()).created is False
            assert memory.remember(**city(True)).created is False
            matches = client.query(VECTOR, top_k=64)['matches']
            expected = {item['id']: item for item in documents(1) if item['id'] != 'doc-7'}
            assert len(matches) == len(expected) and {item['id'] for item in matches} == set(expected)
            for item in matches:
                assert item['text'] == expected[item['id']]['text']
                assert json.dumps(item['metadata'], sort_keys=True) == json.dumps(expected[item['id']]['metadata'], sort_keys=True)
            report = memory.forget(ids=['berlin'], include_chain=True, purge=True)
            assert set(report.forgotten) == {'paris', 'berlin'} and report.purged
        with WarmClient(url=url, api_key=KEY, tenant_id='other') as other:
            assert other.memory.get('fact-0') is None
            assert other.query(VECTOR, top_k=64)['matches'] == []
        server.stop(signal.SIGTERM)
        url, final_start = server.start()
        with WarmClient(url=url, api_key=KEY, tenant_id='qualification') as client:
            assert client.memory.get('paris') is None and client.memory.get('berlin') is None
            assert client.memory.recall('Alice city', subject='alice/city', history='all') == []
            assert client.memory.get('fact-0') is not None
        server.stop(signal.SIGTERM)
        result = {'complete': True, 'interruption': interruption.name, 'exitCode': stopped,
                  'initialStartSeconds': initial_start, 'recoverySeconds': recovery, 'postPurgeStartSeconds': final_start,
                  'atomicBatchSize': 32, 'replacedDocumentCount': 32, 'lostAcknowledgedWrites': 0,
                  'duplicateMemoryRetryEffects': 0, 'deletedOrCrossTenantResults': 0,
                  'currentHistoricalKnownTimeAndProvenanceMatched': True, 'purgedChainAbsentAfterReopen': True}
        write(output / 'finished.json', result)
        return result
    finally:
        server.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    write(args.output / 'freeze.json', {'binarySha256': digest(binary), 'harnessSha256': digest(Path(__file__)),
                                       'python': platform.python_version(), 'machine': platform.machine(),
                                       'cases': ['SIGKILL', 'SIGTERM'], 'boundary': 'Acknowledged HTTP writes',
                                       'scope': 'Process death and graceful reopen; not host power loss or all interruption points'})
    cases = [run_case(binary, args.output / method.name.lower(), method) for method in (signal.SIGKILL, signal.SIGTERM)]
    if digest(binary) != json.loads((args.output / 'freeze.json').read_text())['binarySha256']:
        raise RuntimeError('server binary changed during qualification')
    write(args.output / 'finished.json', {'complete': True, 'cases': cases, 'benchmarkClaim': False})


if __name__ == '__main__':
    main()
