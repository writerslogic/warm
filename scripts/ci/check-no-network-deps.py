# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Fail when the normal dependency tree of the given feature set holds a network client crate.

usage: check-no-network-deps.py [--features LIST] [--expect-found]

The default feature set must contain none of DENY. `--expect-found` inverts the result and is
the negative control: a feature set that does reach the network (provenance-scitt pulls ureq)
must be reported, which shows the deny list and the cargo tree parsing still work.
"""
import argparse
import subprocess
import sys

DENY = {
    # HTTP clients and servers
    "ureq", "ureq-proto", "reqwest", "hyper", "hyper-util", "hyper-rustls", "hyper-tls", "h2", "h3",
    "isahc", "attohttpc", "surf", "minreq", "curl", "curl-sys", "axum", "warp", "actix-web",
    # TLS stacks
    "rustls", "rustls-webpki", "rustls-native-certs", "webpki-roots", "native-tls", "openssl",
    "openssl-sys", "schannel", "security-framework", "boring", "boring-sys",
    # sockets, async network runtimes, websockets
    "socket2", "mio", "tokio", "async-std", "smol", "tungstenite", "tokio-tungstenite", "quinn",
    "trust-dns-resolver", "hickory-resolver",
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--features", default="")
    parser.add_argument("--expect-found", action="store_true")
    args = parser.parse_args()
    command = ["cargo", "tree", "--locked", "-e", "normal", "--target", "all", "--prefix", "none", "--format", "{p}"]
    if args.features:
        command += ["--features", args.features]
    output = subprocess.run(command, check=True, capture_output=True, text=True).stdout
    crates = {line.split()[0] for line in output.splitlines() if line.strip()}
    found = sorted(crates & DENY)
    print(f"{len(crates)} crates, features {args.features or 'default'}: {'found ' + ', '.join(found) if found else 'no network client crate'}")
    return 0 if bool(found) == args.expect_found else 1


if __name__ == "__main__":
    sys.exit(main())
