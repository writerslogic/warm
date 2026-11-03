# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import io
import os
from pathlib import Path
import tarfile
import tempfile
import unittest

from distribution import digest, verify_archive, verify_installed


class DistributionTests(unittest.TestCase):
    def fixture(self, folder):
        root = folder / 'distribution'
        (root / 'lib').mkdir(parents=True)
        data = b'a pinned implementation\n'
        (root / 'lib/code').write_bytes(data)
        (root / 'link').symlink_to('lib/code')
        (root / 'copy').hardlink_to(root / 'lib/code')
        archive = folder / 'archive.tar'
        with tarfile.open(archive, 'w') as stream:
            stream.add(root, arcname=root.name)
        return root, archive

    def test_files_symlinks_hardlinks_and_changed_components(self):
        with tempfile.TemporaryDirectory() as directory:
            root, archive = self.fixture(Path(directory))
            manifest = verify_archive(root, archive, digest(archive, 'sha512'))
            verify_installed(root, manifest)
            (root / 'lib/code').write_bytes(b'changed')
            with self.assertRaises(ValueError):
                verify_installed(root, manifest)
            with self.assertRaises(ValueError):
                verify_archive(root, archive, '0' * 128)

    def test_extra_executable_component_is_rejected_but_runtime_log_is_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            root, archive = self.fixture(Path(directory))
            manifest = verify_archive(root, archive, digest(archive, 'sha512'))
            (root / 'logs').mkdir()
            (root / 'logs/runtime.log').write_text('runtime')
            verify_installed(root, manifest)
            (root / 'lib/injected.jar').write_text('extra')
            with self.assertRaises(ValueError):
                verify_installed(root, manifest)

    def test_archive_traversal_and_installed_symlink_escape_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            root, archive = self.fixture(folder)
            manifest = verify_archive(root, archive, digest(archive, 'sha512'))
            (folder / 'outside').write_text('outside')
            (root / 'link').unlink()
            (root / 'link').symlink_to('../outside')
            with self.assertRaises(ValueError):
                verify_installed(root, manifest)
            for name in ('../outside', '/absolute/path', 'distribution/../outside'):
                invalid = folder / 'invalid.tar'
                with tarfile.open(invalid, 'w') as stream:
                    member = tarfile.TarInfo(name)
                    member.size = 1
                    stream.addfile(member, io.BytesIO(b'x'))
                with self.assertRaises(ValueError):
                    verify_archive(root, invalid, digest(invalid, 'sha512'))


if __name__ == '__main__':
    unittest.main()
