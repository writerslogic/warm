# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check installed OpenSearch components against the pinned official archive."""

import hashlib
import os
from pathlib import Path, PurePosixPath
import tarfile


PINS = {
    'aarch64': ('arm64', '629b6a97104d6f925becd2372d902ea253a6209827435841c2cea9866c83567af776942b488110807370f9f504451bb4512f3bb1afb3c956386914c9474423fc'),
    'x86_64': ('x64', 'a70e52a93ad66d9497dd734f5cc48eb4a8a8c4a92c47491a7ad0f1a898383d11c2948d4ba4159b2361d961bf221efa0fd02084e6389532df960b1ebe0c0a2099'),
}


def digest(path, algorithm='sha256'):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, algorithm).hexdigest()


def relative_member(name, prefix):
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != prefix:
        raise ValueError('archive member escapes its distribution root')
    return Path(*path.parts[1:])


def checked_path(root, relative):
    path = root / relative
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError('installed component resolves outside its distribution root')
    return path


def verify_installed(root, manifest):
    for name, entry in manifest.items():
        relative = Path(name)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('invalid component manifest path')
        path = checked_path(root, relative)
        if entry['kind'] == 'symlink':
            if not path.is_symlink() or os.readlink(path) != entry['target']:
                raise ValueError('installed symlink changed: ' + name)
        elif entry['kind'] == 'file':
            if path.is_symlink() or not path.is_file() or digest(path) != entry['sha256']:
                raise ValueError('installed component changed: ' + name)
        else:
            raise ValueError('unknown component manifest kind')
    for directory, directories, files in os.walk(root, followlinks=False):
        for name in [*files, *(name for name in directories if (Path(directory) / name).is_symlink())]:
            path = Path(directory) / name
            relative = path.relative_to(root)
            if relative.parts[0] in ('logs', 'data', 'tmp'):
                continue
            if relative.as_posix() not in manifest:
                raise ValueError('unsealed installed component: ' + relative.as_posix())


def verify_archive(root, archive, expected_sha512):
    if digest(archive, 'sha512') != expected_sha512:
        raise ValueError('distribution archive checksum mismatch')
    manifest, hardlinks = {}, []
    with tarfile.open(archive, mode='r:*') as stream:
        for member in stream:
            relative = relative_member(member.name, root.name)
            if member.isdir():
                continue
            name = relative.as_posix()
            if name in manifest or any(link[0] == name for link in hardlinks):
                raise ValueError('duplicate archive component')
            if member.isreg():
                handle = stream.extractfile(member)
                if handle is None:
                    raise ValueError('archive file has no data')
                with handle:
                    manifest[name] = {'kind': 'file', 'sha256': hashlib.file_digest(handle, 'sha256').hexdigest()}
            elif member.issym():
                manifest[name] = {'kind': 'symlink', 'target': member.linkname}
            elif member.islnk():
                hardlinks.append((name, relative_member(member.linkname, root.name).as_posix()))
            else:
                raise ValueError('unsupported archive component kind')
    for name, target in hardlinks:
        if target not in manifest or manifest[target]['kind'] != 'file':
            raise ValueError('hard link does not reference a regular archived file')
        manifest[name] = dict(manifest[target])
    verify_installed(root, manifest)
    return manifest
