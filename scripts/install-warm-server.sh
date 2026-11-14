#!/bin/sh
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
#
# Installs a prebuilt warm-server from the GitHub release `server-v<version>` and verifies it
# against the release's SHA256SUMS before it is made executable.
#
#   curl -fsSL https://raw.githubusercontent.com/writerslogic/warm/main/scripts/install-warm-server.sh | sh
#   sh install-warm-server.sh 0.6.8            # a specific version
#   WARM_INSTALL_DIR=/usr/local/bin sh install-warm-server.sh
#
# The binary lands in $WARM_INSTALL_DIR (default ~/.local/bin). Linux builds need glibc 2.17 or
# newer; there is no musl (Alpine) or Windows ARM build, and Windows users download the .exe
# from the release page.
set -eu

repo="writerslogic/warm"
version="${1:-${WARM_SERVER_VERSION:-}}"
dest="${WARM_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64 | Linux-arm64) target="aarch64-unknown-linux-gnu" ;;
  *) echo "no prebuilt warm-server for $(uname -s) $(uname -m); build it with: cargo build --release --features server --bin warm-server" >&2; exit 1 ;;
esac
if [ "$target" != "${target%linux-gnu}" ] && [ -f /etc/alpine-release ]; then
  echo "no musl build of warm-server; build it with cargo on Alpine" >&2
  exit 1
fi

if [ -z "$version" ]; then
  version=$(curl -fsSL "https://api.github.com/repos/$repo/releases?per_page=100" |
    sed -n 's/.*"tag_name": *"server-v\([^"]*\)".*/\1/p' | head -1)
  [ -n "$version" ] || { echo "no server-v* release found; pass a version" >&2; exit 1; }
fi
version="${version#server-v}"

base="${WARM_SERVER_BASE_URL:-https://github.com/$repo/releases/download/server-v$version}"
asset="warm-server-$target"
work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT

curl -fsSL -o "$work/$asset" "$base/$asset"
curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS"
want=$(awk -v a="$asset" '$2 == a || $2 == "*" a { print $1 }' "$work/SHA256SUMS")
[ -n "$want" ] || { echo "SHA256SUMS has no entry for $asset" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then
  got=$(sha256sum "$work/$asset" | awk '{print $1}')
else
  got=$(shasum -a 256 "$work/$asset" | awk '{print $1}')
fi
[ "$got" = "$want" ] || { echo "checksum mismatch for $asset: expected $want, got $got" >&2; exit 1; }

mkdir -p "$dest"
chmod +x "$work/$asset"
mv "$work/$asset" "$dest/warm-server"
echo "installed warm-server $version to $dest/warm-server"
case ":$PATH:" in *":$dest:"*) ;; *) echo "add $dest to PATH" ;; esac
