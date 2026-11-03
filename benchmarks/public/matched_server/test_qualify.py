# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import json
from pathlib import Path
import signal
import tempfile
import time
import unittest

from qualify import job_order, operation_deadline, resource_environment, store_bytes


class QualificationTests(unittest.TestCase):
    def test_order_is_complete_and_rotates_engines_per_repetition(self):
        plan = json.loads((Path(__file__).parent / 'PLAN.json').read_text())
        jobs = job_order(plan)
        self.assertEqual(len(jobs), 18)
        self.assertEqual(len({tuple(row.values()) for row in jobs}), 18)
        for index, dataset in enumerate(plan['qualification']['datasetOrder']):
            for repetition, engines in enumerate(plan['qualification']['engineOrder']):
                group = jobs[index * 9 + repetition * 3:index * 9 + repetition * 3 + 3]
                self.assertEqual([row['engine'] for row in group], engines)
                self.assertEqual({row['dataset'] for row in group}, {dataset})

    def test_effective_parent_memory_and_swap_caps_bind_at_boundary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            proc, group = root / 'proc', root / 'cgroup'
            (proc / 'self').mkdir(parents=True)
            (group / 'run').mkdir(parents=True)
            (proc / 'meminfo').write_text('MemTotal: 16777216 kB\nSwapTotal: 1048576 kB\n')
            (proc / 'self/cgroup').write_text('0::/run\n')
            (group / 'run/memory.swap.max').write_text('0')
            cap = 8 * 2**30
            for limit in (cap - 1, cap, cap + 1):
                (group / 'run/memory.max').write_text(str(limit))
                if limit <= cap:
                    self.assertEqual(resource_environment(cap, proc, group)['effectiveCaps']['memory'], limit)
                else:
                    with self.assertRaises(ValueError):
                        resource_environment(cap, proc, group)
            (group / 'memory.max').write_text(str(cap - 1024))
            self.assertEqual(resource_environment(cap, proc, group)['effectiveCaps']['memory'], cap - 1024)
            (group / 'run/memory.swap.max').write_text('1')
            with self.assertRaises(ValueError):
                resource_environment(cap, proc, group)
            (proc / 'self/cgroup').write_text('0::/../escape\n')
            with self.assertRaises(ValueError):
                resource_environment(cap, proc, group)

    def test_deadline_interrupts_blocking_work_and_restores_signal_handler(self):
        original = signal.getsignal(signal.SIGALRM)
        with self.assertRaises(TimeoutError):
            with operation_deadline(.01):
                time.sleep(.5)
        self.assertEqual(signal.getsignal(signal.SIGALRM), original)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL)[0], 0.)
        signal.setitimer(signal.ITIMER_REAL, 100)
        try:
            with self.assertRaises(RuntimeError):
                with operation_deadline(1):
                    self.fail('an existing timer was replaced')
            self.assertGreater(signal.getitimer(signal.ITIMER_REAL)[0], 90)
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)

    def test_store_accounting_deduplicates_hard_links_and_does_not_follow_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / 'data'
            data.mkdir()
            source = data / 'first'
            source.write_bytes(b'x' * 10000)
            (data / 'second').hardlink_to(source)
            outside = root / 'outside'
            outside.write_bytes(b'x' * 100000)
            link = data / 'link'
            link.symlink_to(outside)
            self.assertEqual(store_bytes(data), (source.stat().st_blocks + link.lstat().st_blocks) * 512)


if __name__ == '__main__':
    unittest.main()
