import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('observations', Path(__file__).with_name('summarize-serving-observations.py'))
observations = importlib.util.module_from_spec(spec)
spec.loader.exec_module(observations)


class ObservationTests(unittest.TestCase):
    def test_cpu_time_formats(self):
        self.assertAlmostEqual(observations.cpu_seconds('2742:31.99'), 164551.99)
        self.assertEqual(observations.cpu_seconds('1-02:03:04'), 93784)

    def test_lifetimes_reset_and_one_core_percent(self):
        samples = [
            {'monotonic': 100, 'observedAt': 'first', 'processes': '1 0 00:10 10 server\n2 1 00:04 1 worker\n'},
            {'monotonic': 105, 'observedAt': 'second', 'processes': '1 0 00:12 20 server\n3 1 00:01 2 new-worker\n'},
            {'monotonic': 110, 'observedAt': 'third', 'processes': '1 0 00:01 15 server\n3 1 00:03 3 new-worker\n'},
        ]
        result = observations.summarize_processes(samples, True)
        processes = {row['pid']: row for row in result['processes']}
        self.assertEqual(processes[1]['sampledCpuSeconds'], 2)
        self.assertEqual(processes[1]['averageSingleCorePercent'], 20)
        self.assertEqual(processes[1]['maximumIntervalSingleCorePercent'], 40)
        self.assertEqual(processes[1]['peakRssBytes'], 20 * 1024)
        self.assertEqual(processes[3]['sampledCpuSeconds'], 3)
        self.assertNotIn(2, processes)
        self.assertEqual(result['cpuCounterResets'], [{'pid': 1, 'command': 'server', 'at': 'third'}])

    def test_boundary_coverage_and_empty_or_reordered_records(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'samples.ndjson'
            def write(seconds):
                rows = [{'observedAt': f'2026-11-03T00:00:{value:02d}+00:00'} for value in seconds]
                path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
            base = observations.timestamp('2026-11-03T00:00:00+00:00')
            write([0, 5, 10, 15])
            for start, finish, count in [(0, 15, 4), (1, 14, 4), (5, 10, 2)]:
                rows, terminal = observations.read_window(path, base + start, base + finish)
                self.assertEqual(len(rows), count)
                self.assertIsNone(terminal)
            for start, finish in [(-1, 15), (0, 16)]:
                with self.assertRaises(ValueError):
                    observations.read_window(path, base + start, base + finish)
            for seconds in [[], [0, 10, 5, 15], [0, 5, 5, 15]]:
                write(seconds)
                with self.assertRaises(ValueError):
                    observations.read_window(path, base, base + 15)


if __name__ == '__main__':
    unittest.main()
