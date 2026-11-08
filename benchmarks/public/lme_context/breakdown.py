import json, os, collections
D = {e["question_id"]: e for e in json.load(open(os.path.expanduser("~/.cache/warm-bench/downloads/longmemeval_s_cleaned.json")))}
split = json.load(open("../frozen/longmemeval_split.json")); dev = set(split["dev"])
R = json.load(open("../out/lme500.rankings.json"))
def labels(p):
    out = {}
    for l in open(p):
        r = json.loads(l); out[r["question_id"]] = bool(r["autoeval_label"]["label"])
    return out
runs = {f"5mini_k30pre_r{i}": labels(f"../l01/out/l01-gpt-5-mini-k30-preopen-r{i}.hypotheses.jsonl.eval-results-gpt-4o-mini") for i in (1,2,3)}
runs["4o_k10"] = labels("../out/lme500.hypotheses.jsonl.eval-results-gpt-4o")
def evidence_in_k(q, k):
    e = D[q]; need = set(); 
    for sid, s in zip(e["haystack_session_ids"], e["haystack_sessions"]):
        for i, t in enumerate(s):
            if t.get("has_answer"): need.add((sid, i))
    got = set()
    for c in R[q][:k]:
        c = c.replace("noans_", "answer_"); sid = "_".join(c.split("_")[:-1]); t = int(c.split("_")[-1]) - 1
        got.add((sid, t)); got.add((sid, t + 1))
    return len(need), len(need & got)
qs = sorted(D)
def typ(q): return D[q]["question_type"] + ("_abs" if q.endswith("_abs") else "")
tot = collections.Counter(); 
rep = {}
for name, lab in runs.items():
    wrong = [q for q in qs if not lab[q]]
    bt = collections.Counter(typ(q) for q in wrong)
    rep[name] = {"wrong": len(wrong), "by_type": dict(bt)}
five = [runs[f"5mini_k30pre_r{i}"] for i in (1,2,3)]
allw = [q for q in qs if all(not r[q] for r in five)]
anyw = [q for q in qs if any(not r[q] for r in five)]
split_ret = collections.Counter()
for q in allw:
    n, g = evidence_in_k(q, 30)
    split_ret["abstain" if n == 0 else ("evidence_in_context" if g == n else ("partial" if g else "none"))] += 1
rep["k30pre_wrong_in_all3"] = {"n": len(allw), "by_type": dict(collections.Counter(typ(q) for q in allw)), "retrieval_split": dict(split_ret), "dev": sum(q in dev for q in allw)}
rep["k30pre_wrong_in_any"] = len(anyw)
four = runs["4o_k10"]; w4 = [q for q in qs if not four[q]]
sp = collections.Counter()
for q in w4:
    n, g = evidence_in_k(q, 10)
    sp["abstain" if n == 0 else ("evidence_in_context" if g == n else ("partial" if g else "none"))] += 1
rep["4o_k10_retrieval_split"] = dict(sp)
tsz = collections.Counter(typ(q) for q in qs); rep["type_sizes"] = dict(tsz)
rep["dev_wrong_4o_k10"] = [q for q in w4 if q in dev]
rep["dev_types_wrong_4o_k10"] = dict(collections.Counter(typ(q) for q in w4 if q in dev))
json.dump(rep, open("breakdown.json", "w"), indent=1)
print(json.dumps(rep, indent=1)[:3500])
