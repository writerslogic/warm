"""mkdefs_ivf.py <outdir> <arm> '<json spec>': spec {"build":[{lists,pq_m,reorder,threads}], "nprobe":[...]}"""
import json, os, sys, yaml
out, arm, spec = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
groups = {f"g{i}": {"arg_groups": [dict(b, arm=arm)], "args": {}, "query_args": [spec["nprobe"]]} for i, b in enumerate(spec["build"])}
cfg = {"float": {"any": [{"base_args": ["@metric"], "constructor": "WARMIVF", "disabled": False,
                          "docker_tag": "ann-benchmarks-warm", "module": "ann_benchmarks.algorithms.warmivf",
                          "name": "warmivf", "run_groups": groups}]}}
d = os.path.join(out, arm, "warmivf"); os.makedirs(d, exist_ok=True)
yaml.safe_dump(cfg, open(os.path.join(d, "config.yml"), "w"))
