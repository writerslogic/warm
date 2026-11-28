"""Sample VmHWM (kernel peak RSS) of every VIBE run_algorithm.py worker; append one CSV row per worker at exit.

Each worker process is one index build plus all its query groups, so the peak covers
dataset load, build and search. Poll interval 0.25 s; the last sample before exit is reported.
"""
import csv, os, sys, time, json
out = sys.argv[1]
os.sched_setaffinity(0, {0})
seen = {}
def status(pid):
    d = {}
    with open("/proc/%d/status" % pid) as fh:
        for line in fh:
            k, _, v = line.partition(":")
            if k in ("VmHWM", "VmRSS"):
                d[k] = int(v.split()[0])
    return d
def flush(pid, rec):
    with open(out, "a", newline="") as fh:
        csv.writer(fh).writerow([rec["tag"], rec["dataset"], rec["algorithm"], rec["args"], rec["hwm_kb"], rec["first"], time.time()])
while True:
    live = set()
    for p in os.listdir("/proc"):
        if not p.isdigit():
            continue
        pid = int(p)
        try:
            with open("/proc/%d/cmdline" % pid, "rb") as fh:
                c = fh.read().decode(errors="replace").split("\0")
            if not any(x.endswith("run_algorithm.py") for x in c) or "python3" not in c[0] + c[1]:
                continue
            st = status(pid)
            cwd = os.readlink("/proc/%d/cwd" % pid)
        except (OSError, IndexError):
            continue
        live.add(pid)
        i = c.index("--count")
        rec = seen.setdefault(pid, dict(tag=os.path.basename(cwd), dataset=c[c.index("--dataset") + 1], algorithm=c[c.index("--algorithm") + 1],
                                        args=c[i + 2], hwm_kb=0, first=time.time()))
        rec["hwm_kb"] = max(rec["hwm_kb"], st.get("VmHWM", 0))
    for pid in [x for x in seen if x not in live]:
        flush(pid, seen.pop(pid))
    time.sleep(0.25)
