"""DSPy retrieval contracts against a local authenticated WARM server."""
import uuid

import pytest

dspy = pytest.importorskip("dspy")

from writerslogic_warm_sdk import WarmError
from writerslogic_warm_sdk.dspy import WarmRM
from test_live_server import API_KEY, BIN, embedding, server

pytestmark = pytest.mark.skipif(not BIN, reason="WARM_SERVER_BIN not set")


@pytest.fixture
def retriever(server):
    clients = []

    def make(**kwargs):
        value = WarmRM(url=server, api_key=API_KEY, tenant_id=uuid.uuid4().hex,
                       embedder=lambda query: embedding(int(query)), **kwargs)
        clients.append(value._client)
        return value

    yield make
    for client in clients:
        client.close()


def test_prediction_default_limit_override_and_multiple_queries(retriever):
    value = retriever(k=3)
    value._client.add_documents([
        {"id": str(i), "text": f"passage {i}", "vector": embedding(i)} for i in range(4)
    ])
    prediction = value("0")
    assert isinstance(prediction, dspy.Prediction)
    assert len(prediction.passages) == 3 and prediction.passages[0] == "passage 0"
    first, second = value("0", k=2).passages, value("1", k=2).passages
    assert first[0] == "passage 0" and second[0] == "passage 1"
    assert value(["0", "1"], k=2).passages == first + second
    assert value([], k=2).passages == []


@pytest.mark.parametrize("limit", [0, 1, 2, 99, 100, 101])
def test_server_limits_are_preserved(retriever, limit):
    value = retriever()
    value._client.add_documents([{"id": "one", "text": "one", "vector": embedding(0)}])
    if limit in (0, 101):
        with pytest.raises(WarmError) as error:
            value("0", k=limit)
        assert error.value.status_code == 422
    else:
        assert value("0", k=limit).passages == ["one"]


def test_replacements_deletions_and_tenant_isolation(retriever):
    value = retriever()
    other = retriever()
    value._client.add_documents([{"id": "note", "text": "old", "vector": embedding(0)}])
    assert value("0").passages == ["old"]
    assert other("0").passages == []
    value._client.add_documents([{"id": "note", "text": "new", "vector": embedding(1)}])
    assert value("1").passages == ["new"]
    value._client.delete_document("note")
    assert value("1").passages == []


def test_authentication_and_embedder_errors_propagate(retriever):
    value = retriever()
    value._client.client.headers["Authorization"] = "Bearer invalid"
    with pytest.raises(WarmError) as error:
        value("0")
    assert error.value.status_code == 401
    value.embedder = None
    with pytest.raises(ValueError, match="embedder must be provided"):
        value("0")

    def failed_embedder(query):
        raise ValueError("local embedding failure")

    value.embedder = failed_embedder
    with pytest.raises(ValueError, match="local embedding failure"):
        value("0")
