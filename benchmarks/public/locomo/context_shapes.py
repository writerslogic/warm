# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""How the WARM backend for Mem0's harness shapes the items it returns from a recall.

The harness asks the memory system for at most `top_k` items (200) and prints them to the answerer sorted by `created_at`.
Every shape keeps that contract: at most `top_k` items, each a {id, memory, score, created_at} dict.

  turns   one item per recalled turn, as ingested (the original run's behaviour).
  ordered the same recalled turns, one item per turn, with the turn's index inside its session added to `created_at` as
          milliseconds, so the reader sees each session in dialogue order instead of in descending score order.
  merged  consecutive turns of one session become one item (a one-turn gap between two recalled turns is filled), so the
          items cover more turns. Hits are taken in rank order until the next one would exceed `max_items` items or
          `max_turns` turns.
"""
from datetime import datetime, timezone


def stamp(epoch, index):
    """ISO time of a turn: the session epoch plus `index` + 1 milliseconds. The date part never changes for a real session, and the
    non-zero fraction keeps every item in the one format the harness parses into a human-readable date (isoformat drops a zero fraction)."""
    return datetime.fromtimestamp(epoch + (index + 1) / 1000.0, tz=timezone.utc).isoformat()


class TenantIndex:
    """Turns of one tenant in ingestion order, so adjacency inside a session is known on the client."""

    def __init__(self):
        self.ids, self.texts, self.sessions, self.indices, self.epochs, self.seq = [], [], [], [], [], {}
        self._n_sessions = 0
        self._last_epoch = None
        self._within = 0

    def add(self, fid, text, epoch):
        if epoch != self._last_epoch or not self.ids:
            self._n_sessions += 1
            self._last_epoch = epoch
            self._within = 0
        if fid in self.seq:
            return
        self.seq[fid] = len(self.ids)
        self.ids.append(fid)
        self.texts.append(text)
        self.sessions.append(self._n_sessions)
        self.indices.append(self._within)
        self.epochs.append(epoch)
        self._within += 1

    def created_at(self, p):
        return stamp(self.epochs[p], self.indices[p])


def ordered_items(hits, index, max_items):
    out = []
    for h in hits[:max_items]:
        p = index.seq.get(h['id'])
        out.append({'id': h['id'], 'memory': h['text'], 'score': h.get('score', 0.0),
                    'created_at': index.created_at(p) if p is not None else ''})
    return out


def layout(selected, index, gap):
    """Maximal runs of consecutive same-session turns (sorted positions); with gap, a single missing turn between two selected ones is filled."""
    s = set(selected)
    if gap:
        for p in selected:
            if p + 2 in s and p + 1 not in s and p + 2 < len(index.ids) and index.sessions[p] == index.sessions[p + 1] == index.sessions[p + 2]:
                s.add(p + 1)
    runs = []
    for p in sorted(s):
        if runs and p == runs[-1][-1] + 1 and index.sessions[p] == index.sessions[runs[-1][-1]]:
            runs[-1].append(p)
        else:
            runs.append([p])
    return runs


def select_runs(hits, index, max_items, max_turns, gap=1):
    """Runs built from rank-ordered hits; stops at the first hit that would exceed max_items runs or max_turns turns."""
    selected, runs = [], []
    for h in hits:
        p = index.seq.get(h['id'])
        if p is None or p in selected:
            continue
        trial_runs = layout(selected + [p], index, gap)
        if len(trial_runs) > max_items or sum(len(r) for r in trial_runs) > max_turns:
            break
        selected.append(p)
        runs = trial_runs
    return runs


def merged_items(hits, index, max_items, max_turns, gap=1):
    runs = select_runs(hits, index, max_items, max_turns, gap)
    score = {index.seq[h['id']]: h.get('score', 0.0) for h in hits if h['id'] in index.seq}
    items = []
    for r in runs:
        best = max((score[p] for p in r if p in score), default=0.0)
        items.append({'id': index.ids[r[0]] + (f'+{len(r) - 1}' if len(r) > 1 else ''), 'memory': '\n'.join(index.texts[p] for p in r),
                      'score': best, 'created_at': index.created_at(r[0])})
    return items
