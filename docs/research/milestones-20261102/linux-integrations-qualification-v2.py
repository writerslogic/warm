"""Requalify the repaired wheel with the first run's exact dependency profiles."""
import importlib.util
import json
from pathlib import Path
import shutil

ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103/integrations-qualification-v2')
BASIS = ROOT.with_name('integrations-qualification-v1')
spec = importlib.util.spec_from_file_location('qualification', ROOT / 'linux-integrations-qualification-v1.py')
qualification = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualification)
qualification.ROOT = ROOT
original_run = qualification.run


def run(folder, name, args, timeout=1800, require_success=True):
    if name not in ('lock', 'export'):
        return original_run(folder, name, args, timeout, require_success)
    frozen = json.loads((ROOT / 'freeze.json').read_text())['dependencyProfiles'][folder.name]
    previous = BASIS / 'results' / folder.name
    if qualification.digest(previous / 'dependency-freeze.json') != frozen['receiptSha256']:
        raise ValueError('basis dependency receipt changed')
    for filename, expected in frozen['files'].items():
        source = previous / 'dependencies' / filename
        if qualification.digest(source) != expected:
            raise ValueError('basis dependency file changed: ' + filename)
        destination = folder / 'dependencies' / filename
        if destination.exists():
            if qualification.digest(destination) != expected:
                raise ValueError('dependency specification differs from original qualification')
        elif name == 'lock':
            shutil.copy2(source, destination)
        else:
            raise ValueError('missing reused dependency file')
    (folder / (name + '.log')).write_text('Reused the verified v1 dependency lock and hash-bearing requirements unchanged.\n')
    return 0


qualification.run = run

if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = qualification.main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        qualification.write(ROOT / 'finished.json', result)
