"""Reproduce the frozen deletion-witness result from a portable bundle."""

import argparse
import json
from pathlib import Path
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'temporal_trace'))
from reproduce import digest, run, verify_bundle


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if sys.version_info < (3, 11) or not __debug__:
        parser.error('Python 3.11 or newer with assertions enabled is required')
    root = args.bundle.resolve(strict=True)
    manifest = json.loads((root / 'manifest.json').read_text())
    runners = {'warm.witness-reproduction/1': 'run_witness.py',
               'warm.matrix-witness-reproduction/1': 'run_matrix_witness.py',
               'warm.ambient-witness-reproduction/1': 'run_ambient_witness.py'}
    schema = manifest.get('schema')
    if not isinstance(schema, str) or schema not in runners:
        raise ValueError('unsupported reproduction manifest')
    verify_bundle(root, manifest)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'complete': False, 'independenceVerified': False, 'benchmarkClaim': False}
    try:
        shutil.copyfile(root / 'original/freeze.json', output / 'freeze.json')
        run([sys.executable, '-u', 'benchmarks/research/source_prediction/' + runners[schema],
             '--output', str(output)], output / 'run.log', root)
        result = json.loads((output / 'finished.json').read_text())
        for name, expected in manifest['expected']['result'].items():
            if type(result.get(name)) is not type(expected) or result[name] != expected:
                raise ValueError('research result differs: ' + name)
        for name, expected in manifest['expected']['outputs'].items():
            relative = Path(name)
            if relative.name != name or digest(output / relative) != expected:
                raise ValueError('deterministic evidence differs: ' + name)
        verify_bundle(root, manifest)
        receipt.update({'complete': True, 'scientificResultReproduced': True,
                        'python': sys.version, 'manifestSha256': digest(root / 'manifest.json'),
                        'result': manifest['expected']['result'],
                        'matchedOutputs': len(manifest['expected']['outputs']),
                        'scope': 'Positive-evidence deletion-bound construction; no system advantage'})
    except Exception as error:
        receipt['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        with (output / 'reproduction.json').open('x') as stream:
            json.dump(receipt, stream, indent=2, sort_keys=True)
            stream.write('\n')
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
