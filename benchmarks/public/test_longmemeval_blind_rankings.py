import copy
import hashlib
import json
import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import longmemeval_pipeline as P  # noqa: E402

CFG = json.loads((Path(__file__).resolve().parent / "longmemeval_config.json").read_text())
KINDS = ["turn", "turn_exp", "session"]


def h(*parts: str) -> int:
    return int(hashlib.sha256("|".join(parts).encode()).hexdigest()[:8], 16)


def dataset() -> list[dict]:
    """Three questions with two evidence sessions and six fillers; texts repeat so many scores tie."""
    out = []
    for n in range(3):
        sids, sessions, dates = [], [], []
        for i in range(8):
            sid = f"answer_q{n}_{i}" if i < 2 else f"sharegpt_q{n}x{i}"
            turns = []
            for t in range(3):
                words = f"topic{(n + i + t) % 3} item{(i * 3 + t) % 4}"
                turns.append({"role": "user", "content": f"user says {words}", "has_answer": i < 2 and t == i})
                turns.append({"role": "assistant", "content": f"assistant replies {words}"})
            sids.append(sid)
            sessions.append(turns)
            dates.append(f"2023/05/{10 + i} (Wed) 09:00")
        out.append({"question_id": f"q{n}", "question_type": "multi-session", "question": f"what about topic{n}",
                    "answer": "x", "question_date": "2023/06/01 (Thu) 09:00", "haystack_session_ids": sids,
                    "haystack_dates": dates, "haystack_sessions": sessions, "answer_session_ids": sids[:2]})
    return out


def flipped(data: list[dict], strip_prefix: bool = False) -> list[dict]:
    """Same retrieval inputs, every label-bearing field changed."""
    out = copy.deepcopy(data)
    for e in out:
        e["question_id"] += "_abs"
        e["question_type"] = "temporal-reasoning"
        e["answer"] = "something else entirely"
        e["answer_session_ids"] = []
        for s in e["haystack_sessions"]:
            for t in s:
                if t["role"] == "user":
                    t["has_answer"] = not t["has_answer"]
        if strip_prefix:
            e["haystack_session_ids"] = [re.sub(r"^answer_", "", s) for s in e["haystack_session_ids"]]
    return out


def retrieve(data: list[dict], tmp: Path):
    """Content-only synthetic scores, targets and rerank scores; no model."""
    f = tmp / "data.json"
    f.write_text(json.dumps(data))
    qs = P.load(f)
    scores, targets, rerank = {}, {}, {"session": {}, "turn": {}}
    for q in qs:
        keys = P.build_keys(q, KINDS)
        targets[q.qid] = {f"{k}|{n}": (tid, sid) for n, (k, _, tid, sid) in enumerate(keys)}
        rows = [[f"{k}|{n}", (h("lex", text) % 3) / 2, (h("sem", text) % 4) / 4] for n, (k, text, _, _) in enumerate(keys)]
        scores[q.qid] = {"orig": rows}
        rerank["turn"][q.qid] = {tid: (h("rr", u) % 5) / 5 for s in q.sessions for tid, u, _ in s.turns}
        rerank["session"][q.qid] = {s.sid: (h("rr", s.user_text) % 5) / 5 for s in q.sessions}
    return qs, scores, targets, rerank


def blind_bytes(data, tmp):
    qs, scores, targets, rerank = retrieve(data, tmp)
    r = P.blinded_rankings(qs, scores, targets, CFG["turn"], CFG["session"], None, "", rerank)
    return P.rankings_bytes(P.export_rankings(r["turn"])), qs, scores, targets, rerank


class BlindRankings(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.tmp = Path(tempfile.mkdtemp())

    def test_export_ignores_labels(self):
        base, *_ = blind_bytes(dataset(), self.tmp)
        flip, *_ = blind_bytes(flipped(dataset()), self.tmp)
        self.assertEqual(base, flip.replace(b"_abs", b""))

    def test_export_ignores_id_marker(self):
        norm = lambda b: re.sub(rb"(?<=[\"\[,])(?:answer|noans)_", b"", b)  # noqa: E731
        base, *_ = blind_bytes(dataset(), self.tmp)
        flip, *_ = blind_bytes(flipped(dataset(), strip_prefix=True), self.tmp)
        self.assertEqual(norm(base), norm(flip.replace(b"_abs", b"")))

    def test_export_shape(self):
        raw, qs, *_ = blind_bytes(dataset(), self.tmp)
        out = json.loads(raw)
        self.assertEqual(sorted(out), [q.qid for q in qs])
        for q in qs:
            ids = {tid.replace("noans_", "answer_") for s in q.sessions for tid, _, _ in s.turns}
            self.assertEqual(len(out[q.qid]), len(set(out[q.qid])))
            self.assertTrue(set(out[q.qid]) <= ids)
            self.assertGreaterEqual(len(out[q.qid]), 10)

    def test_control_unblinded_run_does_depend_on_labels(self):
        """The data has ties, and the existing tie-break by id string does see the labels."""
        outs = []
        saved, P.evaluate = P.evaluate, lambda *args, **kw: {}  # only the rankings matter here
        self.addCleanup(setattr, P, "evaluate", saved)
        for d in (dataset(), flipped(dataset())):
            qs, scores, targets, rerank = retrieve(d, self.tmp)
            r, _ = P.run(qs, scores, targets, CFG["turn"], CFG["session"], None, None, "", rerank)
            outs.append([[t.replace("noans_", "answer_") for t in r["turn"][q.qid]] for q in qs])
        self.assertNotEqual(outs[0], outs[1])


if __name__ == "__main__":
    unittest.main()


def test_export_rankings_drops_repeated_turns_keeping_first_position():
    from longmemeval_pipeline import export_rankings
    out = export_rankings({"q": ["noans_a_1_3", "b_2_1", "answer_a_1_3", "b_2_1", "c_1_1"]}, depth=3)
    assert out == {"q": ["answer_a_1_3", "b_2_1", "c_1_1"]}
