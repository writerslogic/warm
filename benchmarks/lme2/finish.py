#!/usr/bin/env python3
"""Pull reader caches, judge the cached subset (resumable), report metrics. Key via env only."""
import json, subprocess, sys, os
from pathlib import Path
ROOT = Path("/Volumes/A/.hms-lb/lme2")
PY = str(ROOT / "venv/bin/python")
systems = sys.argv[1:]
q = {json.loads(l)["id"]: json.loads(l) for l in open(ROOT / "data/questions.jsonl")}
for s in systems:
    cache = json.loads((ROOT / f"reader_cache_{s}.json").read_text())
    for dom in ("enterprise", "web"):
        ids = [i for i in cache if q[i]["domain"] == dom]
        if not ids:
            continue
        sub = {i: cache[i] for i in ids}
        cpath = ROOT / f"scratch_cache_{s}_{dom}.json"
        cpath.write_text(json.dumps(sub))
        out = ROOT / "runs_judged" / f"{s}_{dom}_subset{len(ids)}"
        if out.exists():
            subprocess.run(["mv", str(out), str(out) + f".old{int(__import__('time').time())}"], check=True)
        cmd = [PY, "-I", str(ROOT / "warm_memory/run_lme2.py"), "full", "--system", s, "--domain", dom, "--out", str(out),
               "--reader-cache", str(cpath), "--judge-cache", str(ROOT / "judge_cache.jsonl"), "--question-ids", *ids]
        r = subprocess.run(cmd, env=os.environ, capture_output=True, text=True)
        (ROOT / "runs_judged" / f"{s}_{dom}.log").write_text(r.stdout[-3000:] + r.stderr[-3000:])
        print(s, dom, len(ids), "rc", r.returncode, flush=True)
