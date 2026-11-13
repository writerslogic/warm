"""Run one local campaign generation with declared per-arm expected_rows and bound receipts."""
import csv
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

H = Path.home() / "ab-a/harness"
RUNS = Path.home() / "ab-a/runs"


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def json_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def tree_hash(path):
    path = Path(path)
    if path.is_file():
        return file_hash(path)
    files = sorted(p for p in path.rglob("*") if p.is_file() and "__pycache__" not in p.parts)
    if not files:
        raise ValueError(f"missing or empty input directory: {path}")
    return json_hash({str(p.relative_to(path)): file_hash(p) for p in files})


def atomic_json(path, value):
    path = Path(path)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=path.name + ".", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, sort_keys=True, allow_nan=False)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def export_rows(path, dataset, algorithm):
    with Path(path).open(newline="") as stream:
        reader = csv.DictReader(stream)
        required = {"algorithm", "parameters", "dataset", "count", "k-nn", "qps"}
        if not required.issubset(reader.fieldnames or []):
            raise ValueError("incomplete export header")
        rows = list(reader)
    if not rows:
        raise ValueError("empty export")
    parameters = set()
    for row in rows:
        if None in row or any(row.get(key) is None for key in required):
            raise ValueError("truncated or malformed export")
        recall, qps = float(row["k-nn"]), float(row["qps"])
        if (row["dataset"] != dataset or row["algorithm"] != algorithm
                or not row["parameters"] or row["parameters"] in parameters
                or int(row["count"]) <= 0 or not math.isfinite(recall)
                or not 0 <= recall <= 1 or not math.isfinite(qps) or qps <= 0):
            raise ValueError("invalid export row or identity")
        parameters.add(row["parameters"])
    return rows


def verified_result(directory, identity=None):
    directory = Path(directory)
    with (directory / "success.json").open() as stream:
        receipt = json.load(stream)
    if receipt["status"] != "complete" or (identity is not None and receipt["identity"] != identity):
        raise ValueError("stale or unsuccessful campaign receipt")
    source = receipt["identity"]
    rows = export_rows(directory / "export.csv", source["dataset"], source["algorithm"])
    if (type(source["expected_rows"]) is not int or source["expected_rows"] < 1
            or len(rows) != source["expected_rows"] or receipt["rows"] != len(rows)
            or receipt["export_sha256"] != file_hash(directory / "export.csv")):
        raise ValueError("export differs from success receipt")
    return rows, receipt


def execute_unit(directory, identity, command, export_command, env, allow_resume=True):
    directory = Path(directory)
    if (directory / "success.json").exists() or (directory / "export.csv").exists():
        if not allow_resume:
            raise ValueError("installed competitor environment is not pinned for result reuse")
        verified_result(directory, identity)
        return
    partial = directory / "export.partial.csv"
    partial.unlink(missing_ok=True)
    try:
        with (directory / "run.log").open("w") as log:
            subprocess.run(command, cwd=directory, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        with (directory / "export.log").open("w") as log:
            subprocess.run(export_command, cwd=directory, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        rows = export_rows(partial, identity["dataset"], identity["algorithm"])
        if type(identity["expected_rows"]) is not int or len(rows) != identity["expected_rows"]:
            raise ValueError("export does not cover the registered configuration count")
        receipt = dict(status="complete", identity=identity, export_sha256=file_hash(partial), rows=len(rows))
        os.replace(partial, directory / "export.csv")
        atomic_json(directory / "success.json", receipt)
    except BaseException as error:
        atomic_json(directory / "failure.json", dict(status="failed", identity=identity,
                    error_type=type(error).__name__, error=str(error)))
        raise


def main(plan_path):
    with Path(plan_path).open() as stream:
        plan = json.load(stream)
    gen, names = plan["gen"], list(plan["arms"])
    if not names or type(plan["rounds"]) is not int or plan["rounds"] < 1:
        raise ValueError("campaign requires arms and positive rounds")
    for name in [gen, *names, *plan["datasets"]]:
        if not isinstance(name, str) or not name or name in (".", "..") or "/" in name or "\\" in name or "__" in name:
            raise ValueError("invalid campaign path component")
    expected_counts = {}
    for arm, settings in plan["arms"].items():
        rounds = settings.get("rounds", plan["rounds"])
        if type(rounds) is not int or not 1 <= rounds <= plan["rounds"]:
            raise ValueError("arm rounds must be within the generation's registered rounds")
        datasets = settings.get("datasets", plan["datasets"])
        if not datasets or not set(datasets).issubset(plan["datasets"]):
            raise ValueError("arm datasets must be a nonempty subset of the registered datasets")
        for dataset in datasets:
            expected = settings.get("expected_rows")
            expected = expected.get(dataset) if isinstance(expected, dict) else expected
            if type(expected) is not int or expected < 1:
                raise ValueError(f"register a positive expected_rows count for {arm}/{dataset} before running")
            expected_counts[arm, dataset] = expected
    RUNS.mkdir(parents=True, exist_ok=True)
    done = RUNS / f"{gen}.done"
    done.unlink(missing_ok=True)
    def say(*values):
        with (RUNS / f"{gen}.log").open("a") as log:
            print(time.strftime("%H:%M:%S"), *values, file=log, flush=True)
    plan_digest = json_hash(plan)
    common = dict(plan_sha256=plan_digest, runner_sha256=file_hash(__file__),
                  harness_sha256=tree_hash(H / "ann_benchmarks"),
                  run_py_sha256=file_hash(H / "run.py"), export_py_sha256=file_hash(H / "data_export.py"))
    data = {ds: file_hash(H / "data" / f"{ds}.hdf5") for ds in plan["datasets"]}
    arms = {name: dict(definitions_sha256=tree_hash(arm["defs"]),
                      python_sha256=file_hash(arm["python"]),
                      wheel_sha256=tree_hash(Path.home() / "ab-a/wheels" / arm["wheel"]) if arm.get("wheel") else None)
            for name, arm in plan["arms"].items()}
    completed = 0
    with (Path.home() / "hms-timing.lock").open("a") as lock:
        waited = time.time()
        fcntl.flock(lock, fcntl.LOCK_EX)
        say("lock acquired after", f"{time.time()-waited:.0f}s")
        for r in range(plan["rounds"]):
            order = names[r % len(names):] + names[:r % len(names)]
            for ds in plan["datasets"]:
                for arm in order:
                    a = plan["arms"][arm]
                    if ("datasets" in a and ds not in a["datasets"]) or r >= a.get("rounds", plan["rounds"]):
                        continue
                    expected = expected_counts[arm, ds]
                    directory = RUNS / gen / f"{arm}__{ds}__r{r}"
                    directory.mkdir(parents=True, exist_ok=True)
                    for name in ("ann_benchmarks", "run.py", "data_export.py", "data", "logging.conf"):
                        link = directory / name
                        if not os.path.lexists(link):
                            link.symlink_to(H / name)
                        elif link.resolve() != (H / name).resolve():
                            raise ValueError("existing harness link differs")
                    env = {k: v for k, v in os.environ.items() if not k.startswith("WARM_QGRAPH_")}
                    env.update(OMP_NUM_THREADS="1", MKL_NUM_THREADS="1", OPENBLAS_NUM_THREADS="1",
                               RAYON_NUM_THREADS="1", PYTHONHASHSEED="0", WARM_BUILD_CACHE=str(Path.home() / "ab-a/cache"))
                    if a.get("wheel"):
                        env["PYTHONPATH"] = str(Path.home() / "ab-a/wheels" / a["wheel"])
                        env["WARM_BUILD_TAG"] = a.get("buildtag", a["wheel"])
                    else:
                        env.pop("PYTHONPATH", None)
                    env.update({k: str(v) for k, v in a.get("env", {}).items()})
                    identity = dict(common, **arms[arm], dataset_sha256=data[ds], dataset=ds,
                                    algorithm=a["algo"], arm=arm, round=r, expected_rows=expected)
                    command = ["taskset", "-c", str(plan["cpu"]), a["python"], "-u", "run.py", "--local",
                               "--dataset", ds, "--definitions", a["defs"], "--runs", "2", "--algorithm", a["algo"], "--force"]
                    export_command = [a["python"], "data_export.py", "--output", "export.partial.csv"]
                    started = time.time()
                    say("start", arm, ds, "r", r, "load", os.getloadavg())
                    execute_unit(directory, identity, command, export_command, env, allow_resume=bool(a.get("wheel")))
                    completed += 1
                    say("done", arm, ds, "r", r, f"{time.time()-started:.0f}s")
    if not completed:
        raise ValueError("plan scheduled no units")
    atomic_json(done, dict(status="complete", plan_sha256=plan_digest, completed_units=completed))
    say("GEN_DONE")


if __name__ == "__main__":
    main(sys.argv[1])
