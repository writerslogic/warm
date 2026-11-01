#!/usr/bin/env bash
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
#
# Regenerates the C header and bundled Go headers. Pass --check to fail if any are stale.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cbindgen="${CBINDGEN:-cbindgen}"
out="$here/include/writerslogic_warm.h"
go_include="$here/../go/include"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
(cd "$here" && "$cbindgen" --config cbindgen.toml --crate writerslogic-warm-c --output "$tmp")
if [[ "${1:-}" == "--check" ]]; then
  diff -u "$out" "$tmp"
  diff -u "$go_include/writerslogic_warm.h" "$tmp"
  diff -u "$here/include/holographic_memory.h" "$go_include/holographic_memory.h"
else
  cp "$tmp" "$go_include/writerslogic_warm.h"
  cp "$here/include/holographic_memory.h" "$go_include/holographic_memory.h"
  mv "$tmp" "$out"
  trap - EXIT
fi
