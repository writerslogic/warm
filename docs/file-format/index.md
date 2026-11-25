# File format

What another tool needs to know to read, verify or migrate a WARM store. The format is derived
from the code and checked by committed golden stores under `tests/golden/`, which
`tests/format_*.rs` read. Integers are little-endian; CRC32 is the IEEE polynomial. The
compatibility promise is in [security](../security.md#stability-and-compatibility).

## Layout

A store is one directory, created on first open.

```
<store>/
  .hms.lock                  exclusive flock; not part of the data
  store.json  store.json.crc32
  vectors_data.bin/          the arena (a directory): format.json, seg_*.bin journal segments,
                             and after a compaction CURRENT and gen-*/ generations
  documents.text             cache, rebuilt if discarded
  indices.json               cache
  nsg_index*.bin ivf_index*.bin   caches
  shard_meta.json            shard count after auto-sharding
  encryption.salt            encrypted stores with a passphrase only
  audit.bin  hms_signing.key  hms_provenance.key  provenance_log.jsonl*  kel.json
```

`.hms.lock`, `hms_signing.key` and other `hms` names are fixed identifiers of the on-disk format and
are not renamed. `warm-server` keeps one store per tenant at `<data-dir>/tenants/<hex(utf8 tenant
name)>/`. A snapshot is a copy of the whole directory taken while no engine holds the lock;
`warm-admin migrate` makes a verified copy.

## Journal

Writes are appended as frames to arena segments. Each frame has a header with a CRC32, optional LZ4
compression and an end marker. A torn tail is truncated on open; corruption before the tail is an
error. The payload is a `HMS-TXN` transaction of JSON mutation records. Compaction writes a new
generation, verifies it, and switches `CURRENT` atomically. The document store can also be
checkpointed to `documents.ckpt` so that replay starts after it.

## Versions

Each artefact has its own version; a store is the combination. The same numbers are constants in
`src/core/format/mod.rs`, and `tests/format_golden.rs` fails if one moves without a golden store.

| Artefact | Where | Current | Readable |
|---|---|---|---|
| Arena manifest | `vectors_data.bin/format.json` `version` | 4 | 2, 3, 4 (no manifest is read as 2) |
| Store schema | `store.json` `version` | 6 by default; 7 with vector records | 2 to 7 |
| Document checkpoint | `documents.ckpt` magic | 2 | 1 to 2 |
| Journal payload | magic `HMS-TXN-3\0` | 3 | 2, 3 |
| Vector updates | `HMSVUP01`, `HMSVRW01`, `HMSVLG01` | 1 | 1 |
| Memory stamp | `mem1:` / `_mem.v` | 1 | 1 |

A store keeps its schema on ordinary open. Enabling `vectorRecords` on schemas 2 to 6 needs
`warm-format upgrade`, which copies into a new destination, verifies it, then publishes it; it never
changes a store in place. Older readers reject schema 7 before replay. Stores without `store.json`
are refused; rebuild them with `warm-admin reencode`.

## Encryption

Requires the `security` feature. With it compiled in, a new store is encrypted unless
`security.plaintext = true` or `WARM_ENCRYPTION=off`. The key scheme is chosen once, at creation,
and recorded in `store.json`.

| Request | Result |
|---|---|
| default | 32 random bytes in `<store>/master.key` (mode 0600), `key_scheme: "master-key"` |
| `security.encryption_enabled` with a passphrase | Argon2id-derived key; `encryption.salt` holds the salt |
| `security.plaintext = true` or `WARM_ENCRYPTION=off` | plaintext store |
| `WARM_ENCRYPTION=on` | encryption demanded; an error without `security` |

AES-256-GCM covers arena frame payloads, persisted index caches and the `documents.text` segment.
It does not cover `store.json`, file names, sizes, topology or salts. `key.check` lets a wrong key
fail on open even for an empty store. A missing key is an error and is never replaced; deleting
the key file destroys the store. A plaintext store written by an earlier release stays plaintext;
to encrypt it, re-encode its documents into a new store.

## Verify

```sh
warm-format verify <store-or-data-dir> [--passphrase-env VAR] [--deep] [--json]
warm-format plan <store>
warm-format upgrade <source> <destination> [--dry-run]
warm-format versions
```

`verify` checks every store under a server data directory without modifying it, reports findings
with codes and exits nonzero on failure. Without `--passphrase-env`, an encrypted store is checked
for framing and CRC32 only. `--deep` also opens a private copy and compares it with the journal.
Build it with `cargo build --release --features cli --bin warm-format`.
