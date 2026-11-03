"""Compare every installed SDK file with the frozen wheel in an isolated interpreter."""
import hashlib
import json
from pathlib import Path
import platform
import sys
import zipfile

import writerslogic_warm_sdk


wheel, receipt, expected_python = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
if platform.python_version() != expected_python:
    raise RuntimeError('unexpected consumer Python version')
package = Path(writerslogic_warm_sdk.__file__).resolve().parent
if not package.is_relative_to(Path(sys.prefix).resolve()):
    raise RuntimeError('SDK import escaped the fresh consumer environment')
with zipfile.ZipFile(wheel) as archive:
    names = [name for name in archive.namelist() if name.startswith('writerslogic_warm_sdk/') and not name.endswith('/')]
    expected_files = set()
    for name in names:
        relative = Path(name).relative_to('writerslogic_warm_sdk')
        expected_files.add(relative.as_posix())
        if (package / relative).read_bytes() != archive.read(name):
            raise RuntimeError('installed SDK differs from wheel: ' + name)
actual_files = {path.relative_to(package).as_posix() for path in package.rglob('*')
                if path.is_file() and '__pycache__' not in path.parts}
if actual_files != expected_files:
    raise RuntimeError('unexpected installed SDK files')
with wheel.open('rb') as stream:
    checksum = hashlib.file_digest(stream, 'sha256').hexdigest()
with receipt.open('x') as stream:
    json.dump({'python': platform.python_version(), 'package': str(package), 'filesVerified': len(names),
               'wheelSha256': checksum}, stream, indent=2)
    stream.write('\n')
