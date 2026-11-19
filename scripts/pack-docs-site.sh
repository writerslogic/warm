#!/usr/bin/env bash
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
#
# Packs a built documentation site (scripts/build-docs-site.sh output, built with the public base
# URL https://writerslogic.com/warm/docs) into one tarball and writes <out>.sha256.
# writerslogic.com consumes the tarball (scripts/sync-warm-docs.mjs there) and serves it at /warm/docs/.
#
# Usage: scripts/pack-docs-site.sh <site-dir> <out.tgz>
set -euo pipefail
site="${1:?site dir}"
out="${2:?output tarball}"
[ -f "$site/latest/index.html" ] && [ -f "$site/versions.json" ] || { echo "not a built docs site: $site" >&2; exit 1; }
tar -C "$site" -czf "$out" .
if command -v sha256sum >/dev/null; then hash="$(sha256sum "$out" | cut -d' ' -f1)"; else hash="$(shasum -a 256 "$out" | cut -d' ' -f1)"; fi
printf '%s  %s\n' "$hash" "$(basename "$out")" > "$out.sha256"
echo "packed $out ($hash)"
