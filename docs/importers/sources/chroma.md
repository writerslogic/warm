# chroma

`warm-import chroma (--path DIR | --host HOST [--port N --ssl]) [--collection NAME ...]`

Reads every collection (or the named ones) through the real `chromadb` client, paged with `get(limit, offset)`: ids, documents, metadata, stored embeddings and URIs.

Requires `chromadb>=1.5.9`. Local imports explicitly use `RustClient`, so
`CHROMA_API_IMPL` cannot select the legacy Python backend. The importer disables
embedding-function construction and checks each returned collection's tenant,
database and name before reading records. Stored embedding configuration is not
executed; stored URIs remain metadata.

| Chroma | WARM |
|---|---|
| id | `warm_source_id`; document id is `<collection>::<id>` (namespace prefix, see `--id-prefix-namespace`) |
| collection | namespace (`hms_namespace`) |
| document | text |
| embedding | vector (carried by default) |
| metadata | metadata, verbatim; a URI is kept as `chroma_uri` |
| timestamps | Chroma stores none; `--created-at-key` / `--updated-at-key` copy a metadata key into `warm_created_at` / `warm_updated_at` (the key also stays in metadata) |

Credentials: server token from `$CHROMA_API_KEY` (sent as `X-Chroma-Token`), never logged. `--chroma-tenant` / `--chroma-database` select the Chroma tenant and database.

Resume: not seekable (Chroma offsets follow insertion order but are not a keyset); the pipeline re-reads the committed prefix and checks its hash.

Verification covers real local stores (round trip, namespaces, timestamps, resume)
and the real HTTP client with controlled HTTP responses, including mismatched
scope and malicious configuration. The HTTP tests do not run a live Chroma server
or establish its authorization behavior. Chroma 1.5.9 still has upstream
[security advisories](../../SECURITY.md); these client protections do not patch
an independently operated Chroma server.
