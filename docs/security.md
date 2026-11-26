# Security and compatibility

Report vulnerabilities privately through
[GitHub Security Advisories](https://github.com/writerslogic/warm/security/advisories/new) or the
address in [SECURITY.md](https://github.com/writerslogic/warm/blob/main/SECURITY.md). Do not open a
public issue.

## What WARM does and does not protect

WARM runs locally. The core engine makes no network calls and sends no telemetry; CI checks that
the default dependency tree has no HTTP client. Features that touch a network are opt-in:
`server` listens on the address you give it, and `provenance-scitt` submits signed statements
to a service you configure. `local-models` and `local-embed` read weights from disk; only
`local-embed-download` downloads, once, a sha256-pinned model pack.

| Claim | Reality |
|---|---|
| Sketches and term counts protect text | They do not. The encoder is deterministic and public, so anyone holding a store can test candidate texts. |
| `storeText: false` is private | It drops passages but keeps term counts, metadata, source URIs, IDs and supplied embeddings. |
| Embeddings are safe to share | Inversion attacks exist. Treat embeddings as sensitive as the text. |
| Queries are protected | Queries and model inference run in plaintext in process memory. |
| Encryption protects everything | See below. |
| Deleting erases | Removed records can persist in obsolete segments, snapshots and backups. Compaction is not secure erasure, and CRC32 detects accidents only. |
| Vector masking hides vectors from a remote store | `VectorMask` (Python SDK `zero_trust_key`) is obfuscation. Pairwise similarity is preserved, and known plaintext and masked pairs reveal the permutation progressively. |
| Differential privacy covers the engine | It applies only to the configured bundle operation, is not audited as a finite-precision mechanism, and WARM keeps no persistent budget accountant. |

Not implemented: search over encrypted data, query privacy against the server, PoSME receipts, RATS
attestation verification and `did:web` resolution for access credentials. The experimental
`private-search` feature (LWE, SimplePIR-style) hides the query vector from the scoring server
only, for an index someone else holds; it does nothing for a local store.

## Encryption at rest

With the `security` feature a new store is encrypted by default (AES-256-GCM, 12-byte random nonce
per record). The default key is 32 random bytes in `<store>/master.key` with mode 0600; set
`security.master_key_file` to keep it elsewhere, because a key next to the data does not protect
against a copy of the whole directory. A passphrase scheme derives the key with Argon2id and a
16-byte salt in `encryption.salt`. Plaintext is an explicit opt-out (`security.plaintext = true` or
`WARM_ENCRYPTION=off`). Key schemes and the on-disk envelope are in [file format](file-format/index.md#encryption).

Covered: arena frame payloads, persisted index caches and the passage text segment
(`documents.text`). Not covered: schema metadata, file names, sizes, topology, salts, access
patterns, process memory, swap and crash dumps. A wrong passphrase currently surfaces as
`cannot replay arena frame at 0`, which is an authenticated-decryption failure wrapped in replay
context.

## Signing and audit

- Audit log entries are signed with Ed25519 (`ed25519-dalek`, constant-time) using a raw 32-byte key
  at `hms_signing.key`. Audit entries store SHA-256 hashes of IDs, never raw IDs.
- There is no automatic audit-key rotation: keep the old public keys that you trust. A compromised
  signing key can forge entries, and per-entry signatures do not detect deletion, truncation or
  reordering of otherwise valid records.
- Without `security`, arena integrity relies on CRC32, which detects accidental corruption only.
- The embedded library has no user or role model; the caller controls access. `warm-server` adds
  scoped keys and tenant isolation ([server](server/index.md#authentication)).
- Meaning-memory IDF values are clipped proportionally so a few poisoned documents cannot dominate
  term weights.

## Standards and provenance

The `provenance` feature (experimental, implies `security`) issues COSE_Sign1 signed statements,
DID identities, status-list credentials and sidecars.

| Specification | What is integrated | Limit |
|---|---|---|
| W3C VC Data Model 2.0, Data Integrity EdDSA | `eddsa-jcs-2022` proofs over RFC 8785 canonical JSON | Local checks only |
| Bitstring Status List 1.0 | Signed GZIP and base64url lists with rollback rejection and bounded decoding | No production status endpoint |
| DID Core | Ed25519 `did:key`, `did:jwk` and `did:web` documents | No general resolver; the host authenticates DIDs |
| CAWG Identity and Training-and-Data-Mining assertions 1.1 | COSE-signed assertions | No conformance result claimed |
| C2PA | WARM manifests and sidecars use some C2PA-style labels | They are not C2PA Content Credentials |
| SCITT (RFC 9943), COSE receipts (RFC 9942) | Issuer-signed fact statements; opt-in submission | Receipts are returned unverified |

The [provenance profile](reference/provenance-profile-v1.md) is the draft interoperability
specification. No certification is claimed. To reproduce the checks:
`cargo test --locked --lib --no-default-features --features provenance core::provenance:: -- --test-threads=1`
and `bash scripts/check-w3c-interop.sh`.

## Stability and compatibility

WARM follows semantic versioning. The crate is 0.x: until 1.0 a minor release may break any API
surface. Additive changes are minor, behavior changes are major, and a security fix may break an
unsafe behavior in a patch release and says so in the changelog.

| Surface | Versioned as | Not covered |
|---|---|---|
| Rust crate `warm-rust` | crate version | the `experimental` feature, `doc(hidden)` and `pub(crate)` items |
| Python `warm-python`, `warm-sdk` | PyPI version | underscore names; the phase substrate is experimental |
| JavaScript client `warm-sdk` | `sdk-vX.Y.Z` | undocumented properties |
| Embedded Node `warm-memory` | engine version | undocumented properties |
| HTTP `warm-server` | `/api/v1` | new optional fields and endpoints may appear |
| C ABI | `hms1_*` symbol prefix per ABI major | Rust layouts and internal symbols |
| Go, Java, .NET | binding version over the C ABI | package availability |
| Swift, Kotlin, WebAssembly | binding version | experimental until a stable contract is declared |

The file-format promise: a store written by any 1.x opens read-write in every later 1.x without a
migration, and released 0.x stores (v0.6.1 is in `tests/golden/`) open in the current code. The
engine never converts a store silently; `warm-format upgrade` writes a new store and leaves the
old one untouched. An older reader that meets a version it does not know refuses to open it.

## Verify a release

Every release asset carries a GitHub build attestation and a line in `SHA256SUMS`.

```sh
gh attestation verify <downloaded wheel or .tgz> --repo writerslogic/warm
npm audit signatures        # in a project that depends on the npm package
sha256sum -c SHA256SUMS     # next to the downloaded release assets
```

`reproducible.yml` builds `warm-eval`, `warm-server`, the napi library, the Python wheel and the
crate on two runners from different checkout paths and fails unless the hashes match. The wheel
may differ only in the SBOM maturin embeds, which records the absolute checkout path. The
cross-compiled npm bindings and wheels are attested but not compared.
