"""Score one context arm on a question subset with the official reader prompt and the official judge script.
usage: run_arm.py ARM_NAME SPEC_JSON [--subset dev|heldout|all] [--reader gpt-4o-2024-08-06] [--repeat 0] [--extract FILE ...]
Reader+judge calls go through an OpenRouter proxy on 127.0.0.1:8788 (ledger ctx/proxy-ledger.jsonl); NOT started here."""
import sys; sys.path.insert(0, "/Volumes/A/.hms-lb/longmemeval/ctx")
import argparse, hashlib, json, os, subprocess, time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import render
from openai import OpenAI
HERE = Path(__file__).resolve().parent
DL = Path.home() / ".cache/warm-bench/downloads"
ap = argparse.ArgumentParser(); ap.add_argument("arm"); ap.add_argument("spec")
ap.add_argument("--subset", default="dev"); ap.add_argument("--reader", default="gpt-4o-2024-08-06")
ap.add_argument("--repeat", type=int, default=0); ap.add_argument("--extract", nargs="*", default=[str(HERE / "ext_out_dev.jsonl")])
ap.add_argument("--judges", default="gpt-4o,gpt-4o-mini"); ap.add_argument("--dry", action="store_true"); ap.add_argument("--qids")
a = ap.parse_args()
split = json.load(open(HERE.parent / "frozen/longmemeval_split.json"))
ids = {"dev": split["dev"], "heldout": split["heldout"], "all": split["dev"] + split["heldout"]}[a.subset]
if a.qids: ids = a.qids.split(",")
store = render.Store(*[p for p in a.extract if Path(p).exists()])
spec = json.loads(a.spec) if a.spec.startswith("{") else json.load(open(a.spec))
prompts = {q: render.build(q, store, spec) for q in ids}
tok = {q: len(render.qa.ENC.encode(p, disallowed_special=())) for q, p in prompts.items()}
est = sum(tok.values()) * 2.5e-6 + len(ids) * 250 * 10e-6 + len(ids) * 400 * 2.5e-6
print(f"[{a.arm}] n={len(ids)} mean_tokens={sum(tok.values())/len(ids):.0f} est_usd_reader+judge={est:.2f}", flush=True)
if a.dry: sys.exit(0)
cli = OpenAI(base_url="http://127.0.0.1:8788/v1", api_key="x", timeout=300)
cache = HERE / "calls"; cache.mkdir(exist_ok=True)
def ask(q):
    body = {"model": a.reader, "messages": [{"role": "user", "content": prompts[q]}], "n": 1, "temperature": 0, "max_tokens": 800}
    k = hashlib.sha256(json.dumps({**body, "rep": a.repeat}, sort_keys=True).encode()).hexdigest()
    p = cache / f"{k}.json"
    if p.exists(): return q, json.loads(p.read_text())["text"]
    for i in range(4):
        try:
            r = cli.chat.completions.create(**body); t = r.choices[0].message.content or ""; break
        except Exception as e:
            if i == 3: raise
            time.sleep(3 * (i + 1))
    p.write_text(json.dumps({"text": t})); return q, t
with ThreadPoolExecutor(8) as ex: hyp = dict(ex.map(ask, ids))
out = HERE / "runs"; out.mkdir(exist_ok=True)
hp = out / f"{a.arm}.r{a.repeat}.{a.reader}.hyps.jsonl"
hp.write_text("".join(json.dumps({"question_id": q, "hypothesis": hyp[q]}) + "\n" for q in ids))
res = {}
for j in a.judges.split(","):
    env = {**os.environ, "OPENAI_BASE_URL": "http://127.0.0.1:8788/v1", "OPENAI_API_KEY": "x"}
    code = f"import sys,runpy; sys.path.insert(0,{str(DL)!r}); sys.argv=['e',{j!r},{str(hp)!r},{str(DL/'longmemeval_s_cleaned.json')!r}]; runpy.run_path({str(DL/'longmemeval_evaluate_qa_9e0b455f4ef0.py')!r}, run_name='__main__')"
    subprocess.run(["uv", "run", "--no-project", "--python", "3.11", "--with", "openai==2.8.1", "--with", "tqdm", "--with", "backoff", "--with", "numpy", "python", "-I", "-c", code], check=True, env=env, stdout=subprocess.DEVNULL)
    lab = {}
    for l in open(str(hp) + f".eval-results-{j}"):
        r = json.loads(l); lab[r["question_id"]] = bool(r["autoeval_label"]["label"])
    res[j] = {"correct": sum(lab.values()), "n": len(lab), "labels": lab}
D = render.data(); bytype = {}
for q in ids:
    t = D[q]["question_type"]; bytype.setdefault(t, [0, 0]); bytype[t][1] += 1; bytype[t][0] += res[a.judges.split(",")[0]]["labels"][q]
summ = {"arm": a.arm, "spec": spec, "reader": a.reader, "repeat": a.repeat, "n": len(ids), "mean_tokens": sum(tok.values()) / len(ids),
        "scores": {j: res[j]["correct"] for j in res}, "by_type_first_judge": bytype, "labels": {j: res[j]["labels"] for j in res}}
(out / f"{a.arm}.r{a.repeat}.{a.reader}.json").write_text(json.dumps(summ, indent=1))
print(json.dumps({k: summ[k] for k in ("arm", "n", "mean_tokens", "scores", "by_type_first_judge")}))
