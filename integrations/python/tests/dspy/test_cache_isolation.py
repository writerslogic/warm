"""Run the documented offline program without loading DiskCache."""
import os
from pathlib import Path
import subprocess
import sys

import pytest

pytest.importorskip("dspy")


def test_offline_example_never_loads_diskcache(server, tmp_path):
    example = Path(__file__).resolve().parents[2] / "examples" / "dspy_rag.py"
    code = """
import importlib.abc
import os
from pathlib import Path
import runpy
import sys

attempts = []
class RejectDiskCache(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname == 'diskcache' or fullname.startswith('diskcache.'):
            attempts.append(fullname)
            raise ImportError('DiskCache is forbidden in this test')

sys.meta_path.insert(0, RejectDiskCache())
runpy.run_path(sys.argv[1], run_name='__main__')
import dspy
request = {'prompt': 'local cache round trip'}
assert dspy.cache.get(request) is None
dspy.cache.put(request, {'answer': 'memory only'})
assert dspy.cache.get(request) == {'answer': 'memory only'}
assert attempts == [], attempts
assert not Path(os.environ['DSPY_CACHEDIR']).exists()
"""
    env = {**os.environ, "WARM_URL": server.url, "WARM_TENANT": server.tenant(),
           "WARM_INPUT_DIM": str(server.dim), "DSPY_CACHEDIR": str(tmp_path / "cache"),
           "LITELLM_LOCAL_MODEL_COST_MAP": "True", "WARM_API_KEY": ""}
    result = subprocess.run([sys.executable, "-c", code, str(example)], env=env,
                            capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "on your machine" in result.stdout


def test_real_dspy_embedder_never_reads_cache_or_changes_global_policy(server, tmp_path):
    code = """
import asyncio
import importlib.abc
import os
from pathlib import Path
import sys

imports = []
class RejectDiskCache(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname == 'diskcache' or fullname.startswith('diskcache.'):
            imports.append(fullname)
            raise ImportError('DiskCache is forbidden in this test')
sys.meta_path.insert(0, RejectDiskCache())

import dspy
from writerslogic_warm_integrations.core import HashingEmbedder
from writerslogic_warm_integrations.dspy import WarmRetriever
from writerslogic_warm_sdk.dspy import WarmRM

accesses = []
class RejectCache:
    def get(self, *args, **kwargs):
        accesses.append('get')
        raise AssertionError('cache read forbidden')
    def put(self, *args, **kwargs):
        accesses.append('put')
        raise AssertionError('cache write forbidden')
cache = RejectCache()
dspy.cache = cache
def reject_reconfiguration(*args, **kwargs):
    raise AssertionError('global cache policy must be unchanged')
dspy.configure_cache = reject_reconfiguration

base = HashingEmbedder(int(os.environ['WARM_INPUT_DIM']))
model_calls = []
def local_model(texts, *, scale):
    assert scale == 2.0 and 0 < len(texts) <= 2
    model_calls.append(list(texts))
    return [[value * scale for value in row] for row in base.embed_documents(texts)]
embedding = dspy.Embedder(local_model, batch_size=2, caching=True, scale=2.0)
try:
    embedding(['positive cache guard'])
except AssertionError as error:
    assert str(error) == 'cache read forbidden'
else:
    raise AssertionError('the real DSPy cached path did not reach the guard')
assert accesses == ['get']
accesses.clear()

facts = ['Pasta needs ripe tomato and basil.', 'A telescope observed a distant galaxy.']
url, tenant = os.environ['WARM_URL'], os.environ['WARM_TENANT']
retriever = WarmRetriever(url=url, api_key='', tenant_id=tenant, embedder=embedding, corpus=facts)
plain = WarmRetriever(url=url, api_key='', tenant_id=tenant, input_dim=base.dim,
                      embedder=lambda texts: base.embed_documents(texts))
sdk = WarmRM(url=url, api_key='', tenant_id=tenant + '-sdk', embedder=embedding, k=1)
try:
    assert retriever.config.input_dim == base.dim
    assert retriever(facts[0], k=1).passages == [facts[0]]
    assert plain(facts[1], k=1).passages == [facts[1]]
    async def check_async():
        try:
            assert (await retriever.acall(facts[1], k=1)).passages == [facts[1]]
        finally:
            await retriever.aclose()
    asyncio.run(check_async())
    sdk._client.add_documents([
        {'id': str(i), 'text': text, 'vector': base.embed_query(text)}
        for i, text in enumerate(facts)
    ])
    assert sdk(facts).passages == facts
    sdk.embedder = lambda text: base.embed_query(text)
    assert sdk(facts[0]).passages == [facts[0]]
    def failed_model(texts):
        raise ValueError('local embedding failed')
    sdk.embedder = dspy.Embedder(failed_model, caching=True)
    try:
        sdk(facts[0])
    except ValueError as error:
        assert str(error) == 'local embedding failed'
    else:
        raise AssertionError('embedding error was swallowed')
    assert model_calls and accesses == [] and imports == []
    assert dspy.cache is cache and dspy.configure_cache is reject_reconfiguration
    assert embedding.caching is True and embedding.batch_size == 2
    assert not any(name == 'diskcache' or name.startswith('diskcache.') for name in sys.modules)
    assert not Path(os.environ['DSPY_CACHEDIR']).exists()
finally:
    sdk._client.close()
    plain.close()
    retriever.close()
"""
    env = {**os.environ, "WARM_URL": server.url, "WARM_TENANT": server.tenant(),
           "WARM_INPUT_DIM": str(server.dim), "DSPY_CACHEDIR": str(tmp_path / "cache"),
           "LITELLM_LOCAL_MODEL_COST_MAP": "True", "WARM_API_KEY": ""}
    result = subprocess.run([sys.executable, "-c", code], env=env,
                            capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, result.stdout + result.stderr
