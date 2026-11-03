"""Record actual collection, including inherited framework conformance cases."""
import json
from pathlib import Path
import sys

import pytest


class Collection:
    def __init__(self, output):
        self.output = output

    def pytest_collection_finish(self, session):
        rows = [{'nodeid': item.nodeid, 'path': str(item.path.resolve()),
                 'originalName': item.originalname or item.name} for item in session.items]
        with self.output.open('x') as stream:
            json.dump(rows, stream, indent=2, sort_keys=True)
            stream.write('\n')


if __name__ == '__main__':
    raise SystemExit(pytest.main(sys.argv[2:], plugins=[Collection(Path(sys.argv[1]))]))
