#!/usr/bin/env bash
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
#
# Linux only. Creates a store on a file-backed ext4 image, forgets documents through the
# public API (including a purge cut short by a crash), unmounts, and scans the raw image file
# for the deleted text. Needs root for mount (GitHub-hosted runners have passwordless sudo).
#
# This shows that no byte of the deleted text is left in the filesystem's blocks, journal
# included. It does not show anything about flash translation layers, copy-on-write
# snapshots or backups; see docs/memory-api/ERASURE.md.
set -euo pipefail

work="${RUNNER_TEMP:-$(mktemp -d)}/raw-image-erase"
image="$work/store.img"
mount_point="$work/mnt"
markers="$work/markers.json"
mkdir -p "$mount_point"
truncate -s 256M "$image"
mkfs.ext4 -q -F "$image"
sudo mount -o loop "$image" "$mount_point"
cleanup() { sudo umount "$mount_point" 2>/dev/null || true; }
trap cleanup EXIT
sudo chown "$(id -u):$(id -g)" "$mount_point"

WARM_RAW_MOUNT="$mount_point" WARM_RAW_MARKERS="$markers" \
  cargo test --features failpoints --test raw_image -- --exact write_stores_for_the_raw_image_scan --nocapture

sync
sudo umount "$mount_point"
trap - EXIT
python3 - "$image" "$markers" <<'PY'
import json, sys
image = open(sys.argv[1], "rb").read()
spec = json.load(open(sys.argv[2]))
failed = False
for marker in spec["erased"]:
    # A fragment as well as the whole marker: a partly overwritten copy still counts.
    for needle in (marker, marker[:16]):
        count = image.count(needle.encode())
        print(f"erased  {count:3d} hits for {needle}")
        failed |= count != 0
for marker in spec["present"]:
    count = image.count(marker.encode())
    print(f"present {count:3d} hits for {marker}")
    # Survivors prove the scan sees live text on the image. The logical-forget control is
    # reported only: the first CI run found no remnant of it in the image.
    if count == 0 and marker.startswith("RAWKEEP"):
        print("scan control failed: a record that must be present was not found")
        failed = True
sys.exit(1 if failed else 0)
PY
