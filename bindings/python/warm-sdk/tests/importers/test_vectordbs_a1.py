# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""chroma, sqlite-vec, lancedb and qdrant sources.

Verified against REAL local instances (real client libraries writing real stores) except
`test_qdrant_mock_*` and `test_chroma_http_mock_*`, which drive the real client libraries
against HTTP response fixtures for the server code paths.
"""

import sqlite3

import pytest

from writerslogic_warm_sdk.importers.sources.base import ColumnMap

from .conftest import DIM, assert_faithful, assert_resumable, vec
from .mockserver import MockApi

N = 40


@pytest.fixture(autouse=True)
def _state_dir(tmp_path, monkeypatch):
    monkeypatch.setenv("WARM_IMPORT_STATE_DIR", str(tmp_path / "state"))


def warm_docs(server, tenant, vector, **flt):
    sink = server.sink(tenant)
    try:
        return sink.query(vector=vector, top_k=100, filter=flt or None)
    finally:
        sink.close()


# ---- chroma -------------------------------------------------------------------------


def make_chroma(path):
    chromadb = pytest.importorskip("chromadb")
    client = chromadb.PersistentClient(path=str(path))
    col = client.create_collection("docs")
    col.add(
        ids=[f"d{i}" for i in range(N)],
        embeddings=[vec(i) for i in range(N)],
        documents=[f"chroma document {i}" for i in range(N)],
        metadatas=[{"tag": i % 3, "flag": bool(i % 2), "created": f"2024-01-{1 + i % 28:02d}T00:00:00Z"} for i in range(N)],
    )
    other = client.create_collection("other")
    other.add(ids=["x"], embeddings=[vec(500)], documents=["lives elsewhere"], metadatas=[{"k": "v"}])
    return str(path)


def test_chroma_real_roundtrip(tmp_path, run, server):
    from writerslogic_warm_sdk.importers.sources.chroma import ChromaSource

    path = make_chroma(tmp_path / "chroma")
    src = ChromaSource(path=path, created_at_key="created", page_size=7)
    assert src.count() == N + 1
    report, tenant = run(src, batch_size=9)
    assert_faithful(report, N + 1)
    assert report.stats.namespaces == {"docs": N, "other": 1}
    hits = {m["id"]: m for m in warm_docs(server, tenant, vec(3), hms_namespace="docs")}
    meta = hits["docs::d3"]["metadata"]
    assert meta["tag"] == 0 and meta["flag"] is True and meta["warm_source"] == "chroma"
    assert meta["warm_created_at"] == "2024-01-04T00:00:00+00:00" and meta["created"] == "2024-01-04T00:00:00Z"
    assert "docs::x" not in hits and warm_docs(server, tenant, vec(500), hms_namespace="other")[0]["text"] == "lives elsewhere"


def test_chroma_real_resume(tmp_path, server):
    from writerslogic_warm_sdk.importers.sources.chroma import ChromaSource

    path = make_chroma(tmp_path / "chroma")
    assert_resumable(lambda: ChromaSource(path=path, page_size=6), server, tmp_path, N + 1)


def test_chroma_requires_exactly_one_location(tmp_path):
    from writerslogic_warm_sdk.importers.model import ImporterError
    from writerslogic_warm_sdk.importers.sources.chroma import ChromaSource

    with pytest.raises(ImporterError):
        ChromaSource()


@pytest.mark.parametrize("page_size", [N - 1, N, N + 1])
def test_chroma_real_reads_never_execute_collection_configuration(tmp_path, monkeypatch, page_size):
    pytest.importorskip("chromadb")
    from chromadb.api.models.Collection import Collection
    from chromadb.utils.embedding_functions import known_embedding_functions

    from writerslogic_warm_sdk.importers.sources.chroma import ChromaSource

    path = make_chroma(tmp_path / "chroma")
    source = ChromaSource(path=path, page_size=page_size)
    client = source._connect()
    get_collection = client._server.get_collection
    attempts = []

    def forbidden(*args, **kwargs):
        attempts.append("configuration, embedding or mutation")
        raise AssertionError("a read-only import must not execute collection configuration")

    class ForbiddenEmbedding:
        build_from_config = staticmethod(forbidden)

    def poisoned_collection(*args, **kwargs):
        model = get_collection(*args, **kwargs)
        model.configuration_json = {
            **model.configuration_json,
            "embedding_function": {
                "type": "known",
                "name": "warm-import-forbidden",
                "config": {"model_name": "forbidden-import-fixture", "kwargs": {"trust_remote_code": True}},
            },
        }
        return model

    monkeypatch.setitem(known_embedding_functions, "warm-import-forbidden", ForbiddenEmbedding)
    monkeypatch.setattr(client._server, "get_collection", poisoned_collection)
    monkeypatch.setattr(Collection, "configuration", property(forbidden))
    monkeypatch.setattr(Collection, "schema", property(forbidden))
    for method in ("_embed", "add", "delete", "update", "upsert", "modify", "query"):
        monkeypatch.setattr(Collection, method, forbidden)

    try:
        assert all(collection._embedding_function is None for collection in source._collections())
        assert source.count() == N + 1
        records = {(record.namespace, record.id): record for record in source.records()}
        assert len(records) == N + 1
        for i in range(N):
            record = records[("docs", f"d{i}")]
            assert record.vector == pytest.approx(vec(i))
            assert record.text == f"chroma document {i}"
            assert record.metadata["tag"] == i % 3
            assert record.metadata["flag"] is bool(i % 2)
        assert records[("other", "x")].vector == pytest.approx(vec(500))
        assert records[("other", "x")].text == "lives elsewhere"
        assert not attempts
    finally:
        source.close()


@pytest.mark.parametrize("backend", ["chromadb.api.segment.SegmentAPI", "untrusted.module.Backend"])
def test_chroma_local_backend_ignores_untrusted_environment(tmp_path, monkeypatch, backend):
    chromadb = pytest.importorskip("chromadb")
    from chromadb.api.segment import SegmentAPI
    from chromadb.config import Settings

    from writerslogic_warm_sdk.importers.sources.chroma import ChromaSource

    make_chroma(tmp_path / "chroma")
    monkeypatch.setenv("CHROMA_API_IMPL", backend)
    assert Settings().chroma_api_impl == backend

    def forbidden(*args, **kwargs):
        raise AssertionError("the importer must not construct the legacy configuration-loading backend")

    monkeypatch.setattr(SegmentAPI, "__init__", forbidden)
    source = ChromaSource(path=tmp_path / "chroma")
    try:
        assert source._connect().get_settings().chroma_api_impl == "chromadb.api.rust.RustBindingsAPI"
        assert source.count() == N + 1
        assert len(list(source.records())) == N + 1
    finally:
        source.close()


@pytest.mark.parametrize("field,named", [
    (None, False), (None, True), ("tenant", False), ("tenant", True),
    ("database", False), ("database", True), ("name", True),
    ("path", False), ("path", True), ("escaped_path", False), ("requested_path", True),
])
def test_chroma_http_mock_checks_scope_before_reads(monkeypatch, field, named):
    pytest.importorskip("chromadb")
    import json

    import httpx
    from chromadb.api.models.Collection import Collection
    from chromadb.utils.embedding_functions import known_embedding_functions

    from writerslogic_warm_sdk.importers.model import ImporterError
    from writerslogic_warm_sdk.importers.sources.chroma import ChromaSource

    tenant, database = "tenant-a", "database-a"
    prefix = f"/api/v2/tenants/{tenant}/databases/{database}"
    collection_id = "00000000-0000-0000-0000-000000000001"
    model = {
        "id": collection_id, "name": "docs", "tenant": tenant, "database": database,
        "configuration_json": {"embedding_function": {
            "type": "known", "name": "warm-import-forbidden",
            "config": {"model_name": "forbidden-import-fixture", "kwargs": {"trust_remote_code": True}},
        }},
        "schema": {"embedding_function": "must remain opaque"},
        "metadata": None, "dimension": DIM, "version": 0, "log_position": 0,
    }
    if field in ("path", "escaped_path"):
        model["name"] = "../other" if field == "path" else "%2Fother"
    elif field and field != "requested_path":
        model[field] = "wrong-scope"
    attempts, requests, sessions = [], [], []

    def forbidden(*args, **kwargs):
        attempts.append("configuration or embedding")
        raise AssertionError("HTTP collection configuration must remain opaque")

    class ForbiddenEmbedding:
        build_from_config = staticmethod(forbidden)

    def respond(request):
        assert request.url.host == "chroma.invalid"
        assert request.headers["X-Chroma-Token"] == "fixture-token"
        path = request.url.path
        requests.append((request.method, path))
        if request.method == "GET" and path == "/api/v2/auth/identity":
            body = {"user_id": "fixture", "tenant": tenant, "databases": [database]}
        elif request.method == "GET" and path == f"/api/v2/tenants/{tenant}":
            body = {"name": tenant}
        elif request.method == "GET" and path == prefix:
            body = {"id": "00000000-0000-0000-0000-000000000002", "name": database, "tenant": tenant}
        elif request.method == "GET" and path == prefix + "/collections":
            body = [model]
        elif request.method == "GET" and path == prefix + "/collections/docs":
            body = model
        elif request.method == "GET" and path == prefix + f"/collections/{collection_id}/count":
            assert field is None
            body = 1
        elif request.method == "POST" and path == prefix + f"/collections/{collection_id}/get":
            assert field is None
            query = json.loads(request.content)
            assert query["include"] == ["documents", "metadatas", "embeddings", "uris"]
            present = query["offset"] == 0
            body = {"ids": ["one"] if present else [], "documents": ["stored text"] if present else [],
                    "metadatas": [{"tag": "stored"}] if present else [],
                    "embeddings": [vec(0)] if present else [], "uris": ["https://never-fetch.invalid/item"] if present else []}
        else:
            raise AssertionError(f"unexpected Chroma request: {request.method} {path}")
        return httpx.Response(200, json=body)

    real_client = httpx.Client

    def transport_client(*args, **kwargs):
        kwargs["transport"] = httpx.MockTransport(respond)
        client = real_client(*args, **kwargs)
        sessions.append(client)
        return client

    monkeypatch.setenv("ANONYMIZED_TELEMETRY", "False")
    monkeypatch.setattr(httpx, "Client", transport_client)
    monkeypatch.setitem(known_embedding_functions, "warm-import-forbidden", ForbiddenEmbedding)
    monkeypatch.setattr(Collection, "configuration", property(forbidden))
    monkeypatch.setattr(Collection, "schema", property(forbidden))
    monkeypatch.setattr(Collection, "_embed", forbidden)
    names = ["../other" if field == "requested_path" else "docs"] if named else None
    source = ChromaSource(host="chroma.invalid", api_key="fixture-token", tenant=tenant, database=database,
                          collections=names, page_size=1)
    try:
        if field:
            with pytest.raises(ImporterError, match="Chroma returned"):
                source.count()
            with pytest.raises(ImporterError, match="Chroma returned"):
                list(source.records())
            assert not any(path.endswith(("/count", "/get")) for _, path in requests)
        else:
            assert source.count() == 1
            records = list(source.records())
            assert len(records) == 1
            assert (records[0].id, records[0].namespace, records[0].text) == ("one", "docs", "stored text")
            assert records[0].vector == pytest.approx(vec(0))
            assert records[0].metadata == {"tag": "stored", "chroma_uri": "https://never-fetch.invalid/item"}
        assert not attempts
    finally:
        source.close()
        for session in sessions:
            session.close()


# ---- sqlite-vec ---------------------------------------------------------------------


def make_sqlite_vec(path, kind="float"):
    sqlite_vec = pytest.importorskip("sqlite_vec")
    con = sqlite3.connect(path)
    con.enable_load_extension(True)
    sqlite_vec.load(con)
    con.execute(f"CREATE VIRTUAL TABLE v USING vec0(embedding {kind}[{DIM}])")
    con.execute("CREATE TABLE notes(rowid INTEGER PRIMARY KEY, uid TEXT, body TEXT, tag INTEGER, created REAL, space TEXT)")
    for i in range(N):
        data = sqlite_vec.serialize_float32(vec(i)) if kind == "float" else sqlite_vec.serialize_int8(
            [int(x * 100) for x in vec(i)])
        marker = "?" if kind == "float" else "vec_int8(?)"
        con.execute(f"INSERT INTO v(rowid, embedding) VALUES (?, {marker})", (i + 1, data))
        con.execute("INSERT INTO notes VALUES (?,?,?,?,?,?)",
                    (i + 1, f"u{i}", f"sqlite-vec note {i}", i % 5, 1_700_000_000 + i, "a" if i % 2 else "b"))
    con.commit()
    con.close()
    return str(path)


def sqlite_vec_source(db, **kw):
    from writerslogic_warm_sdk.importers.sources.sqlite_vec import SqliteVecSource

    cmap = ColumnMap(["id=uid", "text=body", "created_at=created", "namespace=space"])
    return SqliteVecSource(db, "v", None, "notes", "rowid", cmap, **kw)


def test_sqlite_vec_real_roundtrip(tmp_path, run, server):
    db = make_sqlite_vec(tmp_path / "v.db")
    src = sqlite_vec_source(db, page_size=11)
    assert src.count() == N
    report, tenant = run(src, batch_size=8)
    assert_faithful(report, N)
    assert report.stats.namespaces == {"a": N // 2, "b": N // 2}
    hit = warm_docs(server, tenant, vec(4), warm_source_id="u4")[0]
    assert hit["id"] == "b::u4" and hit["text"] == "sqlite-vec note 4"
    assert hit["metadata"]["tag"] == 4 and hit["metadata"]["warm_created_at"].startswith("2023-11-14T22:13:24")


def test_sqlite_vec_real_resume_uses_rowid_cursor(tmp_path, server):
    db = make_sqlite_vec(tmp_path / "v.db")
    assert sqlite_vec_source(db).seekable
    assert_resumable(lambda: sqlite_vec_source(db, page_size=6), server, tmp_path, N)


def test_sqlite_vec_real_int8_vectors_decode(tmp_path):
    db = make_sqlite_vec(tmp_path / "v8.db", kind="int8")
    recs = list(sqlite_vec_source(db).records())
    assert len(recs) == N and recs[2].vector == [float(int(x * 100)) for x in vec(2)]
    assert [r.cursor for r in recs[:3]] == [1, 2, 3]
    assert [r.id for r in sqlite_vec_source(db).records(cursor=N - 1)] == [f"u{N - 1}"]


def test_sqlite_vec_rejects_a_plain_table(tmp_path):
    from writerslogic_warm_sdk.importers.model import ImporterError
    from writerslogic_warm_sdk.importers.sources.sqlite_vec import SqliteVecSource

    db = make_sqlite_vec(tmp_path / "v.db")
    src = SqliteVecSource(db, "notes", None, None, "rowid", ColumnMap())
    with pytest.raises(ImporterError, match="not a vec0"):
        list(src.records())


# ---- lancedb ------------------------------------------------------------------------


def make_lancedb(path):
    lancedb = pytest.importorskip("lancedb")
    db = lancedb.connect(str(path))
    db.create_table("notes", data=[
        {"id": f"n{i}", "text": f"lance note {i}", "vector": vec(i), "tag": i % 4,
         "created": f"2024-02-{1 + i % 28:02d}T12:00:00Z", "nested": {"a": i, "b": [i, i + 1]}}
        for i in range(N)
    ])
    return str(path)


def lancedb_source(uri, **kw):
    from writerslogic_warm_sdk.importers.sources.lancedb import LanceDbSource

    return LanceDbSource(uri, ["notes"], None, ColumnMap(["created_at=created"]), **kw)


def test_lancedb_real_roundtrip(tmp_path, run, server):
    uri = make_lancedb(tmp_path / "lance")
    src = lancedb_source(uri, page_size=7)
    assert src.count() == N
    report, tenant = run(src, batch_size=10)
    assert_faithful(report, N)
    hit = warm_docs(server, tenant, vec(5), warm_source_id="n5")[0]
    assert hit["id"] == "notes::n5" and hit["metadata"]["nested"] == {"a": 5, "b": [5, 6]}
    assert hit["metadata"]["hms_namespace"] == "notes" and hit["metadata"]["warm_created_at"].startswith("2024-02-06")


def test_lancedb_real_resume(tmp_path, server):
    uri = make_lancedb(tmp_path / "lance")
    assert_resumable(lambda: lancedb_source(uri, page_size=6), server, tmp_path, N)


def test_lancedb_all_tables_by_default(tmp_path):
    from writerslogic_warm_sdk.importers.sources.lancedb import LanceDbSource

    uri = make_lancedb(tmp_path / "lance")
    import lancedb

    lancedb.connect(uri).create_table("more", data=[{"id": "m", "text": "t", "vector": vec(1)}])
    recs = list(LanceDbSource(uri, [], None, ColumnMap()).records())
    assert len(recs) == N + 1 and {r.namespace for r in recs} == {"notes", "more"}


# ---- qdrant -------------------------------------------------------------------------


def make_qdrant(path, named=False):
    qc = pytest.importorskip("qdrant_client")
    from qdrant_client.models import Distance, PointStruct, VectorParams

    client = qc.QdrantClient(path=str(path))
    cfg = {"dense": VectorParams(size=DIM, distance=Distance.COSINE), "alt": VectorParams(size=4, distance=Distance.DOT)} \
        if named else VectorParams(size=DIM, distance=Distance.COSINE)
    client.create_collection("mem", vectors_config=cfg)
    client.upsert("mem", points=[
        PointStruct(
            id=i + 1,
            vector={"dense": vec(i), "alt": [1.0, 0.0, 0.0, float(i)]} if named else vec(i),
            payload={"text": f"qdrant memory {i}", "tag": i % 3, "tenant": "t1" if i % 2 else "t2",
                     "created": 1_700_000_000 + i, "nested": {"k": i}},
        )
        for i in range(N)
    ])
    client.close()
    return str(path)


def qdrant_source(path, **kw):
    from writerslogic_warm_sdk.importers.sources.qdrant import QdrantSource

    return QdrantSource(path=path, page_size=7, created_at_field="created", namespace_field="tenant", **kw)


def test_qdrant_real_roundtrip(tmp_path, run, server):
    path = make_qdrant(tmp_path / "q")
    src = qdrant_source(path)
    assert src.count() == N
    report, tenant = run(src, batch_size=9)
    assert_faithful(report, N)
    assert report.stats.namespaces == {"t1": N // 2, "t2": N // 2}
    hit = warm_docs(server, tenant, vec(6), warm_source_id="7")[0]
    assert hit["id"] == "t2::7" and hit["text"] == "qdrant memory 6" and hit["metadata"]["nested"] == {"k": 6}
    assert hit["metadata"]["warm_created_at"].startswith("2023-11-14T22:13:26")


def test_qdrant_real_resume_by_point_id(tmp_path, server):
    path = make_qdrant(tmp_path / "q")
    assert qdrant_source(path).seekable
    assert_resumable(lambda: qdrant_source(path), server, tmp_path, N)


def test_qdrant_real_cursor_resumes_after_the_given_point(tmp_path):
    path = make_qdrant(tmp_path / "q")
    first = list(qdrant_source(path).records())
    assert len(first) == N
    rest = list(qdrant_source(path).records(cursor=first[19].cursor))
    assert [r.id for r in rest] == [r.id for r in first[20:]]


def test_qdrant_real_named_vectors_need_a_choice(tmp_path):
    from writerslogic_warm_sdk.importers.model import ImporterError

    path = make_qdrant(tmp_path / "q", named=True)
    with pytest.raises(ImporterError, match="--vector-name"):
        list(qdrant_source(path).records())
    recs = list(qdrant_source(path, vector_name="dense").records())
    assert len(recs) == N and len(recs[0].vector) == DIM


KEY = "qd-test-key-123"


def test_qdrant_description_preserves_port_without_credentials():
    from writerslogic_warm_sdk.importers.sources.qdrant import QdrantSource

    url = "http://127.0.0.1:40123"
    with QdrantSource(url=url, api_key=KEY) as source:
        description = source.describe()
    assert description["url"] == url
    assert "api_key" not in description
    assert KEY not in str(description)


def qdrant_mock(points_by_collection):
    api = MockApi(required_auth=("api-key", KEY))

    @api.route("GET", r"/collections")
    def collections(req):
        return {"result": {"collections": [{"name": n} for n in points_by_collection]}, "status": "ok", "time": 0.0}

    @api.route("POST", r"/collections/(?P<c>[^/]+)/points/scroll")
    def scroll(req):
        pts = sorted(points_by_collection[req.match["c"]], key=lambda p: p["id"])
        offset, limit = req.body.get("offset"), req.body["limit"]
        assert req.body["with_payload"] is True and req.body["with_vector"] is True
        pts = [p for p in pts if offset is None or p["id"] >= offset]
        page, rest = pts[:limit], pts[limit:]
        return {"result": {"points": page, "next_page_offset": rest[0]["id"] if rest else None},
                "status": "ok", "time": 0.0}

    @api.route("POST", r"/collections/(?P<c>[^/]+)/points/count")
    def count(req):
        return {"result": {"count": len(points_by_collection[req.match["c"]])}, "status": "ok", "time": 0.0}

    return api


def test_qdrant_mock_server_roundtrip(tmp_path, run, server):
    pytest.importorskip("qdrant_client")
    from writerslogic_warm_sdk.importers.sources.qdrant import QdrantSource

    points = {"mem": [{"id": i + 1, "vector": vec(i), "payload": {"page_content": f"lc doc {i}",
                      "metadata": {"src": f"s{i}", "tag": i % 2}}} for i in range(N)]}
    with qdrant_mock(points) as api:
        src = QdrantSource(url=api.url, api_key=KEY, page_size=9)
        assert "api_key" not in src.describe() and KEY not in str(src.describe())
        assert src.count() == N
        report, tenant = run(src, batch_size=12)
        assert_faithful(report, N)
        api.assert_clean()
    hit = warm_docs(server, tenant, vec(2), warm_source_id="3")[0]
    assert hit["text"] == "lc doc 2" and hit["metadata"]["src"] == "s2" and hit["metadata"]["hms_namespace"] == "mem"


def test_qdrant_mock_server_resume(tmp_path, server):
    pytest.importorskip("qdrant_client")
    from writerslogic_warm_sdk.importers.sources.qdrant import QdrantSource

    points = {"mem": [{"id": i + 1, "vector": vec(i), "payload": {"text": f"t{i}"}} for i in range(N)]}
    with qdrant_mock(points) as api:
        assert_resumable(lambda: QdrantSource(url=api.url, api_key=KEY, page_size=5), server, tmp_path, N)
        api.assert_clean()


def test_qdrant_mock_wrong_key_is_not_leaked_and_fails(tmp_path):
    pytest.importorskip("qdrant_client")
    from writerslogic_warm_sdk.importers.common import scrub
    from writerslogic_warm_sdk.importers.sources.qdrant import QdrantSource

    with qdrant_mock({"mem": []}) as api:
        src = QdrantSource(url=api.url, api_key="wrong-key-value")
        from writerslogic_warm_sdk.importers.common import register_secret

        register_secret("wrong-key-value")
        with pytest.raises(Exception) as err:
            src.count()
        assert "wrong-key-value" not in scrub(str(err.value))
        assert api.auth_failures >= 1
