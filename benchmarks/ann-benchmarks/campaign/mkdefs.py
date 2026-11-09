"""mkdefs.py <outdir> <arm> '<json spec>' : writes <outdir>/<arm>/warm/config.yml
spec: {"build":[{degree,build_ef,code_bits,...}], "ef":[...], "rerank":[...], "patience":[0], "reservoir":[0]}"""
import json, os, sys, yaml
out, arm, spec = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
groups = {}
for i, b in enumerate(spec["build"]):
    b = dict(b, arm=arm)
    groups[f"g{i}"] = {"arg_groups": [b], "args": {},
                       "query_args": [spec["ef"], spec["rerank"], spec.get("patience", [0]), spec.get("reservoir", [0])]}
cfg = {"float": {"any": [{"base_args": ["@metric"], "constructor": "WARM", "disabled": False,
                          "docker_tag": "ann-benchmarks-warm", "module": "ann_benchmarks.algorithms.warm",
                          "name": "warm", "run_groups": groups}]}}
d = os.path.join(out, arm, "warm"); os.makedirs(d, exist_ok=True)
with open(os.path.join(d, "config.yml"), "w") as fh: yaml.safe_dump(cfg, fh)
