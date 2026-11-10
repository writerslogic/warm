"""run_gen.py <plan.json>: one unattended generation. Plan: {"gen","rounds","cpu","datasets":[..],
"arms":{name:{"algo":"warm|faiss|scann","wheel":tag|null,"python":path,"defs":dir,"env":{..}}}}
Per round the arm order rotates; each (arm,dataset) is one official run.py --local under flock on cpu <cpu>."""
import fcntl, json, os, subprocess, sys, time
H = os.path.expanduser("~/ab-a/harness"); RUNS = os.path.expanduser("~/ab-a/runs")
with open(sys.argv[1]) as fh: plan = json.load(fh)
gen = plan["gen"]
names = list(plan["arms"])
def say(*a):
    with open(f"{RUNS}/{gen}.log", "a") as log:
        print(time.strftime("%H:%M:%S"), *a, file=log, flush=True)
lk = open(os.path.expanduser("~/hms-timing.lock"), "a"); t_w = time.time(); fcntl.flock(lk, fcntl.LOCK_EX); say("lock acquired after", f"{time.time()-t_w:.0f}s")
for r in range(plan["rounds"]):
    order = names[r % len(names):] + names[:r % len(names)]
    for ds in plan["datasets"]:
        for arm in order:
            a = plan["arms"][arm]
            if ("datasets" in a and ds not in a["datasets"]) or r >= a.get("rounds", plan["rounds"]): continue
            wd = f"{RUNS}/{gen}/{arm}__{ds}__r{r}"
            if os.path.exists(wd + "/export.csv"):
                continue
            os.makedirs(wd, exist_ok=True)
            for n in ("ann_benchmarks", "run.py", "data_export.py", "data", "logging.conf"):
                if not os.path.lexists(f"{wd}/{n}"):
                    os.symlink(f"{H}/{n}", f"{wd}/{n}")
            env = dict(os.environ, OMP_NUM_THREADS="1", MKL_NUM_THREADS="1", OPENBLAS_NUM_THREADS="1",
                       RAYON_NUM_THREADS="1", PYTHONHASHSEED="0",
                       WARM_BUILD_CACHE=os.path.expanduser("~/ab-a/cache"))
            if a.get("wheel"):
                env["PYTHONPATH"] = os.path.expanduser(f"~/ab-a/wheels/{a['wheel']}")
                env["WARM_BUILD_TAG"] = a.get("buildtag", a["wheel"])
            env.update({k: str(v) for k, v in a.get("env", {}).items()})
            cmd = ["taskset", "-c", str(plan["cpu"]),
                   a["python"], "-u", "run.py", "--local", "--dataset", ds, "--definitions", a["defs"],
                   "--runs", "2", "--algorithm", a["algo"], "--force"]
            t0 = time.time(); say("start", arm, ds, "r", r, "load", os.getloadavg())
            with open(f"{wd}/run.log", "w") as lf:
                rc = subprocess.run(cmd, cwd=wd, env=env, stdout=lf, stderr=subprocess.STDOUT).returncode
            subprocess.run([a["python"], "data_export.py", "--output", "export.csv"], cwd=wd, env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            say("done", arm, ds, "r", r, "rc", rc, f"{time.time()-t0:.0f}s")
fcntl.flock(lk, fcntl.LOCK_UN); lk.close()
say("GEN_DONE")
with open(f"{RUNS}/{gen}.done", "w") as fh: fh.write("ok")
