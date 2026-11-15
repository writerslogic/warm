# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Head-to-head adapters: mem0ai (open source) and Graphiti (the open-source core of Zep) on LoCoMo.

Both go through the same reader, judge, prompt, memory rendering and o200k token accounting as
the WARM arm in locomo_bench.py. They call an LLM and an embedder during ingestion, so the model,
endpoint and key are parameters. `--llm-base-url` points both at any OpenAI-compatible server,
including stub_llm.py, which proves the plumbing for free.

mem0ai ingest follows the paper-era harness the WARM arm matches (mem0ai/mem0 evaluation/src/memzero/add.py
at 393a4fd5): one user per speaker, each session sent in batches of 2 messages, with the
speaker's own turns as role user and the other speaker's as role assistant, metadata
{"timestamp": session date}. Search is top_k=30 per speaker, rendered "{timestamp}: {memory}".
Graphiti follows getzep/zep-papers locomo_eval: every turn is one episode `"{speaker}: {text}"`
with the session date as reference time, one group per conversation; the context is facts (edge
search, 20) plus entities (node search, 20) in Zep's own template, placed in the first speaker's
memory slot of the shared reader prompt. Settings and versions are written to the run summary.
"""

import asyncio
import json
import os
import tempfile
from datetime import datetime, timezone

ZEP_TEMPLATE = """FACTS and ENTITIES represent relevant context to the current conversation.

# These are the most relevant facts for the conversation along with the datetime of the event that the fact refers to.
If a fact mentions something happening a week ago, then the datetime will be the date time of last week and not the datetime
of when the fact was stated.
Timestamps in memories represent the actual time the event occurred, not the time the event was mentioned in a message.

<FACTS>
{facts}
</FACTS>

# These are the most relevant entities
# ENTITY_NAME: entity summary
<ENTITIES>
{entities}
</ENTITIES>"""


def session_dates(conv):
    """(session date string, [turns]) in order."""
    n = 1
    while f'session_{n}' in conv:
        yield conv[f'session_{n}_date_time'], conv[f'session_{n}']
        n += 1


def parse_date(date):
    return datetime.strptime(date, '%I:%M %p on %d %B, %Y').replace(tzinfo=timezone.utc)


class Mem0System:
    """mem0ai open-source `Memory`, one user per speaker."""
    name = 'mem0ai'

    def __init__(self, base_url, api_key, model, embed_model, embed_dim, workdir=None, threshold=None):
        self.threshold = threshold
        os.environ.setdefault('MEM0_TELEMETRY', 'False')
        # mem0ai routes every chat call to OpenRouter when this variable is set, ignoring openai_base_url.
        os.environ.pop('OPENROUTER_API_KEY', None)
        from mem0 import Memory
        import mem0
        self.version = getattr(mem0, '__version__', None) or _dist_version('mem0ai')
        self.workdir = workdir or tempfile.mkdtemp(prefix='locomo-mem0-')
        self.config = {
            'llm': {'provider': 'openai', 'config': {'model': model, 'api_key': api_key, 'openai_base_url': base_url, 'temperature': 0.0}},
            'embedder': {'provider': 'openai', 'config': {'model': embed_model, 'api_key': api_key, 'openai_base_url': base_url,
                                                          'embedding_dims': embed_dim}},
            'vector_store': {'provider': 'qdrant', 'config': {'path': os.path.join(self.workdir, 'qdrant'), 'collection_name': 'locomo',
                                                              'embedding_model_dims': embed_dim, 'on_disk': True}},
            'history_db_path': os.path.join(self.workdir, 'history.db'),
        }
        self.memory = Memory.from_config(self.config)

    def settings(self):
        redacted = json.loads(json.dumps(self.config))
        redacted['llm']['config']['api_key'] = redacted['embedder']['config']['api_key'] = '<redacted>'
        return {'package': 'mem0ai', 'version': self.version, 'config': redacted, 'ingest': 'paper-era add.py, batches of 2',
                'search': {'top_k': 30, 'threshold': self.threshold if self.threshold is not None else 'package default (0.1)'}}

    def ingest(self, sample):
        conv = sample['conversation']
        users = {'a': f"{conv['speaker_a']}_{sample['sample_id']}", 'b': f"{conv['speaker_b']}_{sample['sample_id']}"}
        self.users = getattr(self, 'users', {})
        self.users[sample['sample_id']] = users
        for date, turns in session_dates(conv):
            msgs = {'a': [], 'b': []}
            for t in turns:
                text = f"{t['speaker']}: {t['text']}"
                own = 'a' if t['speaker'] == conv['speaker_a'] else 'b'
                other = 'b' if own == 'a' else 'a'
                msgs[own].append({'role': 'user', 'content': text})
                msgs[other].append({'role': 'assistant', 'content': text})
            for side, items in msgs.items():
                for i in range(0, len(items), 2):
                    self.memory.add(items[i:i + 2], user_id=users[side], metadata={'timestamp': date})

    def search(self, sample, side, question, k):
        extra = {} if self.threshold is None else {'threshold': self.threshold}
        res = self.memory.search(question, filters={'user_id': self.users[sample['sample_id']][side]}, top_k=k, **extra)
        return [f"{r.get('metadata', {}).get('timestamp', '')}: {r['memory']}" for r in res['results']]


class GraphitiSystem:
    """Graphiti core, one embedded Kuzu database (one Graphiti instance) per conversation.

    graphiti-core 0.31.2 raises AttributeError (`KuzuDriver._database`) when `group_id` is given with the
    Kuzu driver, so the conversation boundary is the database and the default group is used.
    """
    name = 'graphiti'

    def __init__(self, base_url, api_key, model, embed_model, embed_dim, reranker='rrf', workdir=None):
        os.environ.setdefault('GRAPHITI_TELEMETRY_ENABLED', 'false')
        self.workdir = workdir or tempfile.mkdtemp(prefix='locomo-graphiti-')
        self.args = (base_url, api_key, model, embed_model, embed_dim)
        self.reranker = reranker
        self.version = _dist_version('graphiti-core')
        self.model, self.embed_model, self.embed_dim = model, embed_model, embed_dim
        self.loop = asyncio.new_event_loop()
        self.graphs = {}

    def run(self, coro):
        return self.loop.run_until_complete(coro)

    def _open(self, sample_id):
        from graphiti_core import Graphiti
        from graphiti_core.cross_encoder.openai_reranker_client import OpenAIRerankerClient
        from graphiti_core.driver.kuzu_driver import KuzuDriver
        from graphiti_core.embedder.openai import OpenAIEmbedder, OpenAIEmbedderConfig
        from graphiti_core.llm_client import LLMConfig, OpenAIClient
        base_url, api_key, model, embed_model, embed_dim = self.args
        cfg = LLMConfig(api_key=api_key, model=model, small_model=model, base_url=base_url, temperature=0.0)
        g = Graphiti(
            graph_driver=KuzuDriver(db=os.path.join(self.workdir, f'{sample_id}.kuzu')), llm_client=OpenAIClient(config=cfg),
            embedder=OpenAIEmbedder(OpenAIEmbedderConfig(api_key=api_key, base_url=base_url, embedding_model=embed_model, embedding_dim=embed_dim)),
            cross_encoder=OpenAIRerankerClient(config=cfg))
        self.run(g.build_indices_and_constraints())
        self.graphs[sample_id] = g
        return g

    def settings(self):
        return {'package': 'graphiti-core', 'version': self.version, 'driver': 'kuzu (embedded), one database per conversation',
                'model': self.model, 'embedder': [self.embed_model, self.embed_dim],
                'edge_search': {'num_results': 20, 'reranker': self.reranker},
                'node_search': {'config': 'NODE_HYBRID_SEARCH_RRF', 'limit': 20},
                'ingest': 'one message episode per turn in conversation order, zep-papers locomo_eval', 'context': 'Zep template, facts + entities'}

    def ingest(self, sample):
        from graphiti_core.nodes import EpisodeType
        conv = sample['conversation']
        g = self._open(sample['sample_id'])

        async def go():
            for date, turns in session_dates(conv):
                ref = parse_date(date)
                for t in turns:
                    await g.add_episode(name=t['dia_id'], episode_body=f"{t['speaker']}: {t['text']}", source=EpisodeType.message,
                                        source_description='conversation', reference_time=ref)
        self.run(go())

    def search(self, sample, side, question, k):
        """The shared reader prompt has two memory slots; the graph context is one list and goes in slot 'a'."""
        if side == 'b':
            return []
        from graphiti_core.search.search_config_recipes import EDGE_HYBRID_SEARCH_CROSS_ENCODER, NODE_HYBRID_SEARCH_RRF
        g = self.graphs[sample['sample_id']]

        async def go():
            if self.reranker == 'cross_encoder':
                cfg = EDGE_HYBRID_SEARCH_CROSS_ENCODER.model_copy(update={'limit': 20})
                edges = (await g.search_(question, config=cfg)).edges
            else:
                edges = await g.search(question, num_results=20)
            nodes = (await g.search_(question, config=NODE_HYBRID_SEARCH_RRF.model_copy(update={'limit': 20}))).nodes
            return edges, nodes
        edges, nodes = self.run(go())
        facts = '\n'.join(f'  - {e.fact} (event_time: {e.valid_at})' for e in edges)
        entities = '\n'.join(f'  - {n.name}: {n.summary}' for n in nodes)
        return [ZEP_TEMPLATE.format(facts=facts, entities=entities)]

    def close(self):
        for g in self.graphs.values():
            self.run(g.close())


def _dist_version(name):
    from importlib.metadata import version
    return version(name)
