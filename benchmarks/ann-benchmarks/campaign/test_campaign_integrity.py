# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Offline regression checks; no native index, dataset, benchmark or cloud run."""
import concurrent.futures
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import types
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import ga
import run_gen


def exported(recall="0.995", qps="200"):
    return ("algorithm,parameters,dataset,count,k-nn,qps\n"
            f"warm,configuration,synthetic,10,{recall},{qps}\n")


def identity(round_number=0):
    return dict(dataset="synthetic", algorithm="warm", arm="ctrl", round=round_number,
                plan_sha256="frozen-plan", expected_rows=1)


class CampaignReceipts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def execute(self, directory, text=None, run_code=None, export_code=None, unit_identity=None):
        directory.mkdir(parents=True, exist_ok=True)
        run_code = run_code or "from pathlib import Path; Path('executed').write_text('yes')"
        export_code = export_code or ("from pathlib import Path; Path('export.partial.csv').write_text("
                                      + repr(exported() if text is None else text) + ")")
        run_gen.execute_unit(directory, unit_identity or identity(),
                             [sys.executable, "-c", run_code],
                             [sys.executable, "-c", export_code], dict(os.environ))

    def test_failed_runner_cannot_publish_success_or_export(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.execute(self.root, run_code="raise SystemExit(7)")
        self.assertFalse((self.root / "success.json").exists())
        self.assertFalse((self.root / "export.partial.csv").exists())
        self.assertEqual(json.loads((self.root / "failure.json").read_text())["status"], "failed")

    def test_failed_export_retains_partial_without_success(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.execute(self.root, export_code="from pathlib import Path; Path('export.partial.csv').write_text('partial'); raise SystemExit(9)")
        self.assertEqual((self.root / "export.partial.csv").read_text(), "partial")
        self.assertFalse((self.root / "export.csv").exists())
        self.assertFalse((self.root / "success.json").exists())

    def test_empty_truncated_nonfinite_and_duplicate_exports_fail(self):
        bad = ["", "algorithm,parameters,dataset,count,k-nn,qps\n",
               "algorithm,parameters,dataset,count,k-nn,qps\nwarm,configuration,synthetic,10,0.99\n",
               exported(qps="nan"), exported(recall="1.1"), exported() + exported().splitlines()[1] + "\n"]
        for number, text in enumerate(bad):
            with self.subTest(number=number):
                directory = self.root / str(number)
                with self.assertRaises(ValueError):
                    self.execute(directory, text=text)
                self.assertFalse((directory / "success.json").exists())

    def test_complete_receipt_resumes_without_child_execution(self):
        self.execute(self.root)
        rows, receipt = run_gen.verified_result(self.root, identity())
        self.assertEqual(len(rows), 1)
        self.assertEqual(receipt["rows"], 1)
        self.execute(self.root, run_code="raise SystemExit(99)")
        self.assertFalse((self.root / "failure.json").exists())

    def test_successful_process_with_partial_grid_cannot_publish_success(self):
        with self.assertRaises(ValueError):
            self.execute(self.root, unit_identity=dict(identity(), expected_rows=2))
        self.assertFalse((self.root / "success.json").exists())

    def test_stale_or_modified_export_is_not_reused(self):
        self.execute(self.root)
        with self.assertRaises(ValueError):
            self.execute(self.root, unit_identity=dict(identity(), plan_sha256="changed"))
        (self.root / "export.csv").write_text(exported(qps="201"))
        with self.assertRaises(ValueError):
            run_gen.verified_result(self.root, identity())

    def test_legacy_csv_and_unpinned_runtime_cannot_resume(self):
        (self.root / "export.csv").write_text(exported())
        with self.assertRaises(FileNotFoundError):
            self.execute(self.root)
        (self.root / "export.csv").unlink()
        self.execute(self.root)
        with self.assertRaises(ValueError):
            run_gen.execute_unit(self.root, identity(), [], [], {}, allow_resume=False)

    def test_fitness_requires_every_round_and_recall_floor(self):
        first = self.root / "generation/ctrl__synthetic__r0"
        second = self.root / "generation/ctrl__synthetic__r1"
        self.execute(first, text=exported(qps="100"), unit_identity=identity(0))
        with self.assertRaises(FileNotFoundError):
            ga.score("generation", "ctrl", 2, dataset="synthetic", runs_dir=self.root, plan_sha="frozen-plan")
        self.execute(second, text=exported(qps="300"), unit_identity=identity(1))
        self.assertEqual(ga.score("generation", "ctrl", 2, dataset="synthetic",
                                 runs_dir=self.root, plan_sha="frozen-plan"), (200.0, 200.0, 200.0))
        with self.assertRaises(ValueError):
            ga.score("generation", "ctrl", 2, dataset="synthetic", runs_dir=self.root, plan_sha="changed")
        low = self.root / "below/ctrl__synthetic__r0"
        self.execute(low, text=exported(recall="0.98"), unit_identity=identity(0))
        self.assertEqual(ga.score("below", "ctrl", 1, dataset="synthetic",
                                 runs_dir=self.root, plan_sha="frozen-plan"), (0.0, None, None))

    def test_per_key_defaults_preserve_explicit_enabled_values(self):
        for key in ("WARM_QGRAPH_PREFETCH", "WARM_QGRAPH_BATCH_EST", "WARM_QGRAPH_RERANK_Z"):
            with self.subTest(key=key):
                self.assertEqual(ga.norm({"env": {key: 1}})["env"], {key: 1})
                self.assertNotEqual(ga.norm({"env": {key: 1}}), ga.norm({"env": {key: 0}}))
        self.assertEqual(ga.norm({"env": dict(ga.DEFAULT_ENV)})["env"], {})
        self.assertEqual(ga.norm({"env": {"UNKNOWN": 0}})["env"], {"UNKNOWN": 0})


class CacheIntegrity(unittest.TestCase):
    def setUp(self):
        import numpy as np
        self.np = np
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.native = self.root / ("fixture" + importlib.machinery.EXTENSION_SUFFIXES[0])
        self.native.write_bytes(b"runtime-one")
        package_names = ("_campaign_check", "_campaign_check.algorithms", "_campaign_check.algorithms.base",
                         "_campaign_check.algorithms.warm")
        modules = {name: types.ModuleType(name) for name in package_names}
        for module in modules.values():
            module.__path__ = []
        base = types.ModuleType("_campaign_check.algorithms.base.module")
        base.BaseANN = object
        native = types.ModuleType("writerslogic_warm")
        native.VectorIndex = object
        native.__file__ = str(self.native)
        modules.update({"_campaign_check.algorithms.base.module": base, "writerslogic_warm": native})
        self.module_patch = patch.dict(sys.modules, modules)
        self.module_patch.start()
        spec = importlib.util.spec_from_file_location("_campaign_check.algorithms.warm.module", HERE / "warm_module_campaign.py")
        self.cache = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.cache)
        self.cache.CACHE = str(self.root)
        self.X = np.arange(6000, dtype=np.float32).reshape(3000, 2)

    def tearDown(self):
        self.module_patch.stop()
        self.temporary.cleanup()

    def key(self, values=None, degree=32):
        return self.cache.cache_file(self.X if values is None else values, "euclidean", degree, 500, 8, 8, False)

    def test_full_data_runtime_and_build_identity(self):
        original = self.key()
        changed = self.X.copy()
        changed[1, 0] += 0.5
        self.assertNotEqual(original, self.key(changed))
        self.assertNotEqual(original, self.key(degree=64))
        self.native.write_bytes(b"runtime-two")
        self.assertNotEqual(original, self.key())

    def test_concurrent_publishers_share_one_complete_file(self):
        calls, guard, barrier = [], threading.Lock(), threading.Barrier(2)
        class Index:
            def __init__(self, state):
                self.state = state
            def fit(self, values):
                with guard:
                    calls.append("fit")
                barrier.wait(timeout=5)
            def save(self, path):
                Path(path).write_bytes(self.state)
            def load(self, path):
                self.state = Path(path).read_bytes()
                if self.state not in (b"complete-one", b"complete-two"):
                    raise ValueError("partial index observed")
        path = self.root / "concurrent.warm"
        indexes = [Index(b"complete-one"), Index(b"complete-two")]
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            futures = [executor.submit(self.cache.cache_load_or_build, path, index, self.X) for index in indexes]
            outcomes = [future.result(timeout=10) for future in futures]
        self.assertEqual(sum(outcome["raced"] for outcome in outcomes), 1)
        self.assertTrue(all(outcome["built"] and not outcome["hit"] for outcome in outcomes))
        self.assertEqual(calls, ["fit", "fit"])
        self.assertEqual(indexes[0].state, path.read_bytes())
        self.assertEqual(indexes[1].state, path.read_bytes())
        self.assertEqual(list(self.root.glob("concurrent.warm.*")), [])

    def test_failed_publication_leaves_no_loadable_index(self):
        class Index:
            def fit(self, values):
                pass
            def save(self, path):
                Path(path).write_bytes(b"incomplete")
                raise OSError("fixture save failure")
        path = self.root / "failure.warm"
        with self.assertRaises(OSError):
            self.cache.cache_load_or_build(path, Index(), self.X)
        self.assertFalse(path.exists())
        self.assertEqual(list(self.root.glob("failure.warm.*")), [])


if __name__ == "__main__":
    unittest.main()
