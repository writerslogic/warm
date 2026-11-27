import yaml, sys, shutil
for n in ("new", "base"):
    p = f"/home/davidcondrey/vs/vibe-{n}/vibe/algorithms/warm/config.yml"
    shutil.copy(p, p + ".full")
    c = yaml.safe_load(open(p))
    ip = c["float"]["ip"][0]["run_groups"]
    for g in list(ip):
        if g in ("learn_mid", "learn_tail"): del ip[g]
    ip["base"]["args"]["degree"] = [32]
    ip["base"]["query_args"]["ef"] = [150, 200, 250, 300, 400, 650, 1000]
    ip["base"]["query_args"]["rerank"] = [150, 200, 400]
    print(n, {g: [len(v) for v in ip[g]["args"].values()] for g in ip})
    yaml.safe_dump(c, open(p, "w"), sort_keys=False)
