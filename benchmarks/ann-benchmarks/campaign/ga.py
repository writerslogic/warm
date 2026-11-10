"""ga.py <cfg.json>: bounded evolutionary search over graph-arm genomes on a held-out dataset, through the
official harness (run_gen.py). cfg: {name, dataset, ef_grid, wheels:[..], builds:[{..}], seeds:[genome..],
max_gens, pop, rounds, noise (relative gain below which a generation counts as no gain), rng}
genome: {wheel, build, env{}, rerank, patience}. Fitness = sqrt(QPS@0.95 * QPS@0.99), log-linear interpolation
on the frontier, median over rounds; 0 when 0.99 is not reached."""
import json, math, os, random, subprocess, sys, statistics as st
from pathlib import Path
from run_gen import atomic_json, json_hash, verified_result
H = os.path.expanduser("~"); RUNS = f"{H}/ab-a/runs"; PY = f"{H}/ab-hms/venv/bin/python"
ENV = {"WARM_QGRAPH_PREFETCH": [0, 1, 2, 3], "WARM_QGRAPH_BATCH_EST": [0, 1], "WARM_QGRAPH_FRONTIER": [1.0, 0.95, 0.9, 0.85],
       "WARM_QGRAPH_RERANK_Z": [0, 1, 2, 3], "WARM_QGRAPH_KERNEL": ["auto", "avx2"]}
RERANK = [16, 24, 32, 48, 64, 96, 128]; PAT = [0, 10, 20, 40, 80]
DEFAULT_ENV = {"WARM_QGRAPH_PREFETCH": 0, "WARM_QGRAPH_BATCH_EST": 0,
               "WARM_QGRAPH_FRONTIER": 1.0, "WARM_QGRAPH_RERANK_Z": 0, "WARM_QGRAPH_KERNEL": "auto"}
key = lambda g: json.dumps(g, sort_keys=True)
def norm(g):
    g = json.loads(key(g))
    g["env"] = {k: v for k, v in g.get("env", {}).items() if k not in DEFAULT_ENV or v != DEFAULT_ENV[k]}
    return g
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
def score(gen, arm, rounds, dataset=None, runs_dir=None, plan_sha=None):
    if type(rounds) is not int or rounds < 1 or not plan_sha:
        raise ValueError("fitness requires positive registered rounds and a bound plan")
    dataset = dataset or cfg["dataset"]
    runs_dir = Path(runs_dir or RUNS)
    all_rows = []
    for r in range(rounds):
        rows, receipt = verified_result(runs_dir / gen / f"{arm}__{dataset}__r{r}")
        identity = receipt["identity"]
        if (identity["plan_sha256"] != plan_sha or identity["dataset"] != dataset
                or identity["arm"] != arm or identity["round"] != r):
            raise ValueError("round receipt does not match the registered arm")
        all_rows.append(rows)
    points = [(at(rows, 0.95), at(rows, 0.99)) for rows in all_rows]
    if any(a is None or b is None for a, b in points):
        return 0.0, None, None
    q95, q99 = (st.median(values) for values in zip(*points))
    return math.sqrt(q95 * q99), q95, q99
def log(msg):
    with open(f"{RUNS}/ga-{cfg['name']}.log", "a") as f: f.write(msg + "\n")
def main(config_path):
    global cfg, rnd
    with open(config_path) as stream:
        cfg = json.load(stream)
    if cfg["max_gens"] < 1 or cfg["rounds"] < 1 or not cfg["seeds"]:
        raise ValueError("evolution requires generations, rounds and seeds")
    rnd = random.Random(cfg.get("rng", 7))
    Path(RUNS).mkdir(parents=True, exist_ok=True)
    done = Path(RUNS) / f"ga-{cfg['name']}.done"
    done.unlink(missing_ok=True)
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
            arms[arm] = {"algo": "warm", "wheel": g["wheel"], "buildtag": "f21de886", "python": PY, "defs": d, "env": env,
                         "expected_rows": len(cfg["ef_grid"])}
        plan = {"gen": gen, "rounds": cfg["rounds"], "cpu": 4, "datasets": [cfg["dataset"]], "arms": arms}
        with open(f"{H}/ab-a/plans/{gen}.json", "w") as fh: json.dump(plan, fh)
        log(f"{gen} start pop={len(pop)}")
        subprocess.run([PY, f"{H}/ab-a/run_gen.py", f"{H}/ab-a/plans/{gen}.json"], check=True)
        res = []
        for i, g in enumerate(pop):
            sc, a, b = score(gen, f"a{i}", cfg["rounds"], plan_sha=json_hash(plan)); res.append((sc, a, b, g))
            log(f"{gen} a{i} fit={sc:.0f} q95={a} q99={b} {key(g)}")
        res.sort(key=lambda x: -x[0]); elites = [r[3] for r in res[:cfg.get("elites", 3)]]
        best = res[0][0]; gain = best / best_prev - 1 if best_prev else 1.0
        log(f"{gen} best={best:.0f} gain_vs_prev={gain:+.3f} elite={key(res[0][3])}")
        stale = stale + 1 if gain < cfg["noise"] else 0
        best_prev = max(best_prev, best)
        if stale >= 2: log("STOP: two generations with no gain above noise"); break
    atomic_json(done, dict(status="complete", configuration_sha256=json_hash(cfg)))
    log("GA_DONE")


if __name__ == "__main__":
    main(sys.argv[1])
