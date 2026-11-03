"""Summarize recorded host and guest observations without asserting isolation."""
import argparse
from collections import defaultdict
from datetime import datetime
import hashlib
import json
from pathlib import Path
import re


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def timestamp(value):
    return datetime.fromisoformat(value).timestamp()


def cpu_seconds(value):
    days, clock = value.split('-', 1) if '-' in value else ('0', value)
    parts = [float(part) for part in clock.split(':')]
    if len(parts) not in (2, 3) or any(part < 0 for part in parts):
        raise ValueError('invalid process CPU time')
    result = 0.0
    for part in parts:
        result = result * 60 + part
    return int(days) * 86400 + result


def process_rows(value):
    rows = {}
    for line in value.splitlines():
        pid, parent, clock, rss, command = line.split(None, 4)
        key = (int(pid), command)
        if key in rows:
            raise ValueError('duplicate process identity')
        rows[key] = {'cpu': cpu_seconds(clock), 'parent': int(parent), 'rssBytes': int(rss) * 1024}
    return rows


def read_window(path, start, finish):
    samples, terminal = [], None
    with path.open('rb') as stream:
        for line in stream:
            if len(line) > 8 * 2**20 or not line.endswith(b'\n'):
                raise ValueError('invalid observation record size or boundary')
            row = json.loads(line)
            if row.get('type') == 'terminal':
                terminal = row
            else:
                samples.append(row)
    times = [timestamp(row['observedAt']) for row in samples]
    if not times or times != sorted(set(times)) or not times[0] <= start <= finish <= times[-1]:
        raise ValueError('observation window is unordered or does not cover execution')
    first = max(index for index, value in enumerate(times) if value <= start)
    last = next(index for index, value in enumerate(times) if value >= finish)
    return samples[first:last + 1], terminal


def summarize_processes(samples, guest):
    totals = defaultdict(float)
    peaks = defaultdict(float)
    rss_peaks = defaultdict(int)
    parents = defaultdict(set)
    resets = []
    previous = None
    gaps = []
    for sample in samples:
        value = sample['processes'] if guest else sample['commands']['processes']['stdout']
        current = process_rows(value)
        for key, row in current.items():
            rss_peaks[key] = max(rss_peaks[key], row['rssBytes'])
            parents[key].add(row['parent'])
        if previous is not None:
            old, before = previous
            gap = sample['monotonic'] - before
            if gap <= 0:
                raise ValueError('nonpositive observation interval')
            gaps.append(gap)
            for key, row in current.items():
                delta = row['cpu'] - old.get(key, {'cpu': 0.0})['cpu']
                if delta < 0:
                    resets.append({'pid': key[0], 'command': key[1], 'at': sample['observedAt']})
                    continue
                totals[key] += delta
                peaks[key] = max(peaks[key], 100 * delta / gap)
        previous = current, sample['monotonic']
    duration = samples[-1]['monotonic'] - samples[0]['monotonic']
    return {'coveredSeconds': duration, 'maximumSampleGapSeconds': max(gaps),
            'cpuCounterResets': resets,
            'processes': [{'pid': key[0], 'command': key[1], 'parents': sorted(parents[key]),
                           'sampledCpuSeconds': total, 'averageSingleCorePercent': 100 * total / duration,
                           'maximumIntervalSingleCorePercent': peaks[key], 'peakRssBytes': rss_peaks[key]}
                          for key, total in sorted(totals.items(), key=lambda item: (-item[1], item[0]))],
            'limitation': 'PID/command identities lack start times; resets are flagged. Processes wholly between samples and CPU after their last sample are absent. CPU percentages use one core as 100%.'}


def vm_counts(value):
    return {key.strip(): int(number) for key, number in re.findall(r'^([^:\n]+):\s+(\d+)\.', value, re.M)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--serving', required=True, type=Path)
    parser.add_argument('--host', required=True, type=Path)
    parser.add_argument('--guest', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    start = json.loads((args.serving / 'execution-started.json').read_text())
    finish = json.loads((args.serving / 'finished.json').read_text())
    host_terminal = json.loads((args.host / 'finished.json').read_text())
    if finish.get('complete') is not True or host_terminal.get('complete') is not True:
        raise ValueError('execution or observation did not complete')
    host_path, guest_path = args.host / 'samples.ndjson', args.guest / 'samples.ndjson'
    if digest(host_path) != host_terminal['samplesSha256']:
        raise ValueError('host observations changed')
    window = timestamp(start['startedAt']), timestamp(finish['completedAt'])
    host, _ = read_window(host_path, *window)
    guest, guest_terminal = read_window(guest_path, *window)
    if guest_terminal is None or guest_terminal.get('complete') is not True:
        raise ValueError('guest observations did not complete')
    commands_failed = [{'at': row['observedAt'], 'command': name, 'exitCode': value['exitCode']}
                       for row in host for name, value in row['commands'].items() if value['exitCode'] != 0]
    free = [int(re.search(r'System-wide memory free percentage: (\d+)%', row['commands']['memoryPressure']['stdout'])[1]) for row in host]
    before, after = [vm_counts(row['commands']['virtualMemory']['stdout']) for row in (host[0], host[-1])]
    available = [int(re.search(r'^MemAvailable:\s+(\d+) kB', row['proc']['meminfo'], re.M)[1]) * 1024 for row in guest]
    result = {'complete': True, 'executionWindow': [start['startedAt'], finish['completedAt']],
              'inputs': {str(path): digest(path) for path in (host_path, guest_path, args.serving / 'execution-started.json', args.serving / 'finished.json')},
              'summarizerSha256': digest(Path(__file__)),
              'host': {'samples': len(host), 'commandFailures': commands_failed,
                       'minimumFreePercent': min(free), 'maximumFreePercent': max(free),
                       'virtualMemoryDeltas': {key: after[key] - before[key] for key in ('Pageins', 'Pageouts', 'Swapins', 'Swapouts', 'Compressions', 'Decompressions')},
                       'thermalOutputs': sorted({row['commands']['thermal']['stdout'] for row in host}),
                       'powerOutputs': sorted({row['commands']['power']['stdout'] for row in host}),
                       **summarize_processes(host, False)},
              'guest': {'samples': len(guest), 'minimumAvailableBytes': min(available),
                        'pressureFirstLast': {kind: [row['proc']['pressure/' + kind] for row in (guest[0], guest[-1])] for kind in ('cpu', 'io', 'memory')},
                        **summarize_processes(guest, True)},
              'isolationPassed': False, 'claimQualified': False,
              'pending': ['Manual attribution of observed processes and interference', 'Per-job evidence and actual settings review']}
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
    print(json.dumps({'complete': True, 'hostSamples': len(host), 'guestSamples': len(guest),
                      'hostCommandFailures': len(commands_failed), 'isolationPassed': False}))


if __name__ == '__main__':
    main()
