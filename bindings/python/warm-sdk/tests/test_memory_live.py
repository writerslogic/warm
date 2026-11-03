# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Temporal memory contracts exercised through the installed HTTP SDK."""

import asyncio
import time
import uuid

import pytest

from writerslogic_warm_sdk import AsyncWarmClient, WarmClient
from writerslogic_warm_sdk.memory import MemoryError
from test_live_server import API_KEY, BIN, server

pytestmark = pytest.mark.skipif(not BIN, reason="WARM_SERVER_BIN not set")


def identifiers(results):
    return {item.id for item in results}


def test_temporal_recall_preserves_source_and_both_time_axes(server):
    with WarmClient(url=server, api_key=API_KEY, tenant_id=uuid.uuid4().hex) as client:
        memory = client.memory
        capabilities = memory.capabilities()
        assert capabilities["contractVersion"] == 1 and capabilities["temporalQueries"] is True
        first = memory.remember("Alice city Paris", id="paris", subject="alice/city", valid_from=1000,
                                source_uri="urn:qualification:paris", metadata={"verified": True})
        deadline = time.monotonic() + 2
        while time.time_ns() // 1_000_000 <= first.ingest_time:
            assert time.monotonic() < deadline
            time.sleep(.001)
        second = memory.remember("Alice city Berlin", id="berlin", subject="alice/city", valid_from=2000,
                                 supersedes=["paris"], source_uri="urn:qualification:berlin",
                                 metadata={"verified": True})
        assert second.ingest_time > first.ingest_time
        assert second.supersedes == ["paris"]
        options = dict(k=10, subject="alice/city", lexical_weight=1, semantic_weight=0)
        assert identifiers(memory.recall("Alice city", as_of=1500, **options)) == {"paris"}
        current = memory.recall("Alice city", as_of=2500, explain=True, **options)
        assert identifiers(current) == {"berlin"}
        assert current[0].source_uri == "urn:qualification:berlin"
        assert current[0].metadata == {"verified": True}
        assert current[0].explanation.status == "current"
        assert identifiers(memory.recall("Alice city", as_of=2500, known_at=first.ingest_time, **options)) == {"paris"}
        assert memory.recall("Alice city", as_of=2500, known_at=first.ingest_time - 1, **options) == []
        history = memory.recall("Alice city", as_of=2500, history="all", **options)
        assert {item.id: item.status for item in history} == {"paris": "superseded", "berlin": "current"}
        assert [item.id for item in memory.history("berlin")] == ["paris", "berlin"]
        explanation = memory.explain("Alice city", "paris", as_of=2500)
        assert explanation.status == "superseded"
        assert any(link.id == "berlin" and link.relation == "successor" for link in explanation.chain)


def test_conflicting_sources_coexist_until_explicit_forgetting(server):
    tenant = uuid.uuid4().hex
    with WarmClient(url=server, api_key=API_KEY, tenant_id=tenant) as client:
        memory = client.memory
        memory.remember_many([
            {"id": "yes", "text": "Alice city Paris", "subject": "alice/city", "source_uri": "urn:source:yes"},
            {"id": "no", "text": "Alice city not Paris", "subject": "alice/city", "source_uri": "urn:source:no"},
        ])
        results = memory.recall("Alice city", subject="alice/city", k=10)
        assert identifiers(results) == {"yes", "no"}
        assert all(item.status == "current" for item in results)
        assert {item.source_uri for item in results} == {"urn:source:yes", "urn:source:no"}
        dry = memory.forget(ids=["yes"], dry_run=True)
        assert dry.forgotten == ["yes"] and dry.dry_run
        assert memory.get("yes") is not None
        removed = memory.forget(ids=["yes"], purge=True)
        assert removed.forgotten == ["yes"] and removed.purged
        assert memory.get("yes") is None
        assert identifiers(memory.recall("Alice city", subject="alice/city", k=10, history="all")) == {"no"}
        assert memory.forget(ids=["yes"]).forgotten == []
    with WarmClient(url=server, api_key=API_KEY, tenant_id=uuid.uuid4().hex) as other:
        assert other.memory.get("no") is None
        assert other.memory.recall("Alice city", history="all") == []
    with WarmClient(url=server, tenant_id=tenant) as anonymous:
        with pytest.raises(MemoryError) as error:
            anonymous.memory.get("no")
        assert error.value.status == 401


def test_atomic_batch_conflict_retry_and_validity_boundaries(server):
    with WarmClient(url=server, api_key=API_KEY, tenant_id=uuid.uuid4().hex) as client:
        memory = client.memory
        facts = [{"id": f"batch-{i}", "text": f"Batch evidence {i}", "metadata": {"ordinal": i}}
                 for i in range(32)]
        written = memory.remember_many(facts)
        assert len(written) == 32 and all(item.created for item in written)
        retried = memory.remember_many(facts)
        assert len(retried) == 32 and not any(item.created for item in retried)
        with pytest.raises(MemoryError) as error:
            memory.remember_many([{"id": "uncommitted", "text": "Must not be partially stored"},
                                  {"id": "batch-0", "text": "Conflicting replacement"}])
        assert error.value.status == 409
        assert memory.get("uncommitted") is None
        assert memory.get("batch-0").text == facts[0]["text"]
        memory.remember("Window evidence", id="window", subject="window", valid_from=1000, valid_to=2000)
        for point, expected in ((999, set()), (1000, {"window"}), (1999, {"window"}), (2000, set()), (2001, set())):
            assert identifiers(memory.recall("Window", subject="window", as_of=point)) == expected
        assert memory.apply_forgetting().forgotten == []
        assert memory.consolidate().created == []


def test_async_memory_chain_forgetting_and_error_parsing(server):
    async def run():
        async with AsyncWarmClient(url=server, api_key=API_KEY, tenant_id=uuid.uuid4().hex) as client:
            memory = client.memory
            assert (await memory.capabilities())["contractVersion"] == 1
            facts = [{"id": "old", "text": "Alice city Paris", "subject": "alice/city", "valid_from": 1000},
                     {"id": "new", "text": "Alice city Berlin", "subject": "alice/city", "valid_from": 2000,
                      "supersedes": ["old"]}]
            assert len(await memory.remember_many(facts)) == 2
            assert identifiers(await memory.recall("Alice city", subject="alice/city", as_of=2500)) == {"new"}
            assert [item.id for item in await memory.history("old")] == ["old", "new"]
            report = await memory.forget(ids=["new"], include_chain=True, dry_run=True)
            assert set(report.forgotten) == {"old", "new"} and report.dry_run
            assert await memory.get("old") is not None
            report = await memory.forget(ids=["new"], include_chain=True, purge=True)
            assert set(report.forgotten) == {"old", "new"} and report.purged
            assert await memory.get("old") is None and await memory.get("new") is None
            assert await memory.recall("Alice city", history="all") == []
            with pytest.raises(MemoryError) as error:
                await memory.capabilities(contract_version=2)
            assert error.value.status == 422
    asyncio.run(run())
