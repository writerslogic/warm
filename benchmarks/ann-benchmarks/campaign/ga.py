"""ga.py <cfg.json>: bounded evolutionary search over graph-arm genomes on a held-out dataset, through the
official harness (run_gen.py). cfg: {name, dataset, ef_grid, wheels:[..], builds:[{..}], seeds:[genome..],
max_gens, pop, rounds, noise (relative gain below which a generation counts as no gain), rng}
genome: {wheel, build, env{}, rerank, patience}. Fitness = sqrt(QPS@0.95 * QPS@0.99), log-linear interpolation
on the frontier, median over rounds; 0 when 0.99 is not reached."""
import json, math, os, random, subprocess, sys, statistics as st, csv, glob
H = os.path.expanduser("~"); RUNS = f"{H}/ab-a/runs"; PY = f"{H}/ab-hms/venv/bin/python"
cfg = json.load(open(sys.argv[1])); rnd = random.Random(cfg.get("rng", 7))
ENV = {"WARM_QGRAPH_PREFETCH": [0, 1, 2, 3], "WARM_QGRAPH_BATCH_EST": [0, 1], "WARM_QGRAPH_FRONTIER": [1.0, 0.95, 0.9, 0.85],
       "WARM_QGRAPH_RERANK_Z": [0, 1, 2, 3], "WARM_QGRAPH_KERNEL": ["auto", "avx2"]}
RERANK = [16, 24, 32, 48, 64, 96, 128]; PAT = [0, 10, 20, 40, 80]
key = lambda g: json.dumps(g, sort_keys=True)
def norm(g):
    g = json.loads(key(g)); g["env"] = {k: v for k, v in g.get("env", {}).items() if v not in (0, 1.0, "auto")}; return g
def mutate(g):
    g = norm(g); n = rnd.choice([1, 1, 2])
    for _ in range(n):
        gene = rnd.choice(["wheel", "build", "env", "env", "rerank", "patience"])
        if gene == "wheel": g["wheel"] = rnd.choice(cfg["wheels"])
        elif gene == "build": g["build"] = rnd.choice(cfg["builds"])
        elif gene == "env":
            k = rnd.choice(list(ENV)); g.setdefault("env", {})[k] = rnd.choice(ENV[k])
        elif gene == "rerank": g["rerank"] = rnd.choice(RERANK)
        else: g["patience"] = rnd.choice(PAT)
    return norm(g)
def cross(a, b):
    c = {"wheel": rnd.choice([a["wheel"], b["wheel"]]), "build": rnd.choice([a["build"], b["build"]]),
         "rerank": rnd.choice([a["rerank"], b["rerank"]]), "patience": rnd.choice([a["patience"], b["patience"]]),
         "env": {**a.get("env", {}), **b.get("env", {})}}   # union of the parents' env components
    return norm(c)
def defs(arm, g):
    spec = {"build": [g["build"]], "ef": cfg["ef_grid"], "rerank": [g["rerank"]], "patience": [g["patience"]]}
    subprocess.run([PY, f"{H}/ab-a/in/mkdefs.py", f"{H}/ab-a/defs", arm, json.dumps(spec)], check=True)
    return f"{H}/ab-a/defs/{arm}"
def frontier(rows):
    pts = sorted(((float(r["k-nn"]), float(r["qps"])) for r in rows), key=lambda x: -x[1]); out = []; best = -1
    for rec, q in pts:
        if rec > best: out.append((rec, q)); best = rec
    return sorted(out)
def at(rows, thr):
    f = frontier(rows)
    for (r0, q0), (r1, q1) in zip(f, f[1:]):
        if r0 < thr <= r1: return math.exp(math.log(q0) + (thr - r0) / (r1 - r0) * (math.log(q1) - math.log(q0)))
    return f[0][1] if f and f[0][0] >= thr else None
def score(gen, arm, rounds):
    s95, s99 = [], []
    for r in range(rounds):
        p = f"{RUNS}/{gen}/{arm}__{cfg['dataset']}__r{r}/export.csv"
        if not os.path.exists(p): continue
        rows = list(csv.DictReader(open(p)))
        if not rows: continue
        a, b = at(rows, 0.95), at(rows, 0.99)
        if a: s95.append(a)
        if b: s99.append(b)
    if not s95 or not s99: return 0.0, None, None
    q95, q99 = st.median(s95), st.median(s99)
    return math.sqrt(q95 * q99), q95, q99
def log(msg):
    with open(f"{RUNS}/ga-{cfg['name']}.log", "a") as f: f.write(msg + "\n")
pop = [norm(g) for g in cfg["seeds"]]; hist = []; best_prev = 0; stale = 0; elites = []
for gi in range(cfg["max_gens"]):
    # next population: elites + mutants + crossovers (deduped), then seeds fill gen 0
    if gi > 0:
        pop = list(elites); parents = list(elites)
        tries = 0
        while len(pop) < cfg["pop"] and tries < 200:
            tries += 1
            c = cross(*rnd.sample(parents, 2)) if len(parents) > 1 and rnd.random() < 0.4 else mutate(rnd.choice(parents))
            if all(key(c) != key(x) for x in pop): pop.append(c)
    gen = f"{cfg['name']}-g{gi}"
    arms = {}
    for i, g in enumerate(pop):
        arm = f"a{i}"; d = defs(f"{gen}-{arm}", g)
        env = dict(g.get("env", {}))
        arms[arm] = {"algo": "warm", "wheel": g["wheel"], "buildtag": "f21de886", "python": PY, "defs": d, "env": env}
    plan = {"gen": gen, "rounds": cfg["rounds"], "cpu": 4, "datasets": [cfg["dataset"]], "arms": arms}
    json.dump(plan, open(f"{H}/ab-a/plans/{gen}.json", "w"))
    log(f"{gen} start pop={len(pop)}")
    subprocess.run([PY, f"{H}/ab-a/run_gen.py", f"{H}/ab-a/plans/{gen}.json"])
    res = []
    for i, g in enumerate(pop):
        sc, a, b = score(gen, f"a{i}", cfg["rounds"]); res.append((sc, a, b, g))
        log(f"{gen} a{i} fit={sc:.0f} q95={a} q99={b} {key(g)}")
    res.sort(key=lambda x: -x[0]); elites = [r[3] for r in res[:cfg.get("elites", 3)]]
    best = res[0][0]; gain = best / best_prev - 1 if best_prev else 1.0
    log(f"{gen} best={best:.0f} gain_vs_prev={gain:+.3f} elite={key(res[0][3])}")
    stale = stale + 1 if gain < cfg["noise"] else 0
    best_prev = max(best_prev, best)
    if stale >= 2: log("STOP: two generations with no gain above noise"); break
log("GA_DONE"); open(f"{RUNS}/ga-{cfg['name']}.done", "w").write("ok")
