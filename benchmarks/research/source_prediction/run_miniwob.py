import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import urllib.request

from miniwob import MAX_COMPRESSED, decode, encoded, parse, summarize


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('xb') as stream:
        stream.write(encoded(value) + b'\n')


def fetch(root, manifest, item):
    path = PurePosixPath(item['path'])
    if (len(path.parts) != 3 or path.is_absolute() or any(part in ('.', '..') for part in path.parts)
            or not path.name.endswith('.json.gz') or path.parts[1] != item['task']
            or not re.fullmatch('[a-f0-9]{40}', item['sha'])
            or type(item['size']) is not int or not 0 < item['size'] <= MAX_COMPRESSED):
        raise ValueError('invalid pinned input entry')
    destination = root / 'inputs' / path
    if destination.exists():
        data = destination.read_bytes()
    else:
        url = f"https://raw.githubusercontent.com/{manifest['repository']}/{manifest['revision']}/{path}"
        with urllib.request.urlopen(url, timeout=45) as response:
            data = response.read(item['size'] + 1)
    blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
    if len(data) != item['size'] or blob != item['sha']:
        raise ValueError('pinned input size or git blob hash mismatch')
    if not destination.exists():
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open('xb') as stream:
            stream.write(data)
    return {'path': item['path'], 'bytes': len(data), 'gitBlobSha1': blob,
            'sha256': hashlib.sha256(data).hexdigest()}


def run(root, output):
    freeze = json.loads((root / 'freeze.json').read_text())
    for name, expected in freeze['files'].items():
        if digest(root / 'source' / name) != expected:
            raise ValueError('frozen analysis source mismatch: ' + name)
    if digest(root / 'input-manifest.json') != freeze['inputManifestSha256']:
        raise ValueError('input manifest changed')
    manifest = json.loads((root / 'input-manifest.json').read_text())
    if manifest['repository'] != 'stanfordnlp/miniwob-plusplus-demos' or not re.fullmatch('[a-f0-9]{40}', manifest['revision']):
        raise ValueError('unexpected input repository/revision')
    if len(manifest['files']) != 384 or len({item['path'] for item in manifest['files']}) != 384:
        raise ValueError('unexpected demonstration cohort')
    with ThreadPoolExecutor(max_workers=4) as pool:
        receipts = list(pool.map(lambda item: fetch(root, manifest, item), manifest['files']))
    write(output / 'input-receipts.json', receipts)
    failures, results = [], []
    for task in manifest['tasks']:
        selected = [item for item in manifest['files'] if item['task'] == task]
        if len(selected) != 128:
            raise ValueError('unexpected task cohort size')
        parsed, audits = [], []
        for source, item in enumerate(selected):
            try:
                document = parse(decode((root / 'inputs' / item['path']).read_bytes()), task)
                parsed.append(document)
                audits.append({'source': source, 'path': item['path'],
                               **{name: value for name, value in document.items() if name not in ('states', 'pairs')},
                               'recordedStates': len(document['states']), 'pairedTransitions': len(document['pairs']),
                               'nonUniqueTargetPairs': sum(pair['targetCount'] != 1 for pair in document['pairs'])})
            except (ValueError, TypeError, KeyError, RecursionError) as error:
                failures.append({'path': item['path'], 'error': type(error).__name__ + ': ' + str(error)})
        write(output / (task + '-events.json'), audits)
        if len(parsed) != 128:
            continue
        for projection in ('raw', 'target-path'):
            summary, packet = summarize(parsed, projection)
            write(output / (task + '-' + projection + '-transitions.json'), packet)
            results.append({'task': task, 'projection': projection, 'demonstrations': 128,
                            'unmatchedEvents': sum(len(row['unmatched']) for row in parsed),
                            'pairedEvents': sum(len(row['pairs']) for row in parsed), **summary})
    write(output / 'parser-failures.json', failures)
    if failures:
        raise ValueError('cohort contains parser failures; no demonstration exclusion is allowed')
    write(output / 'summary.json', results)
    for name, expected in freeze['files'].items():
        if digest(root / 'source' / name) != expected:
            raise ValueError('frozen source changed during run')
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'cohort': len(receipts), 'cells': len(results), 'freezeSha256': digest(root / 'freeze.json'),
            'raw': {path.name: digest(path) for path in sorted(output.iterdir()) if path.is_file()}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    arguments = parser.parse_args()
    arguments.output.mkdir()
    try:
        result = run(arguments.root, arguments.output)
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(arguments.output / 'finished.json', result)
        print(json.dumps(result, sort_keys=True), flush=True)
