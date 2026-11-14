# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).resolve().with_name("check-doc-figures.py")
spec = importlib.util.spec_from_file_location("check_doc_figures", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class FigureTests(unittest.TestCase):
    def test_a_figure_matches_at_the_precision_it_is_written(self) -> None:
        values = sorted([3787.4774, 0.85938, 2091.98])
        self.assertTrue(module.present(values, "3,787", False))
        self.assertTrue(module.present(values, "0.859", False))
        self.assertTrue(module.present(values, "2,092", False))
        self.assertFalse(module.present(values, "3,788", False))
        self.assertFalse(module.present(values, "0.860", False))

    def test_only_units_and_metrics_make_a_number_a_figure(self) -> None:
        text = "v0.6.8 in 2026: 12,345 QPS, recall@10 0.859, 2.93x and 1.5 GiB; 7 files"
        found = [written for written, _, _ in module.figures(text)]
        self.assertEqual(sorted(found), ["0.859", "1.5", "12,345", "2.93"])

    def test_multiplication_sign_and_thousands_shorthand_are_figures(self) -> None:
        found = [written for written, _, _ in module.figures("a 2.93\u00d7 lead and 3.4k QPS")]
        self.assertEqual(sorted(found), ["2.93", "3400"])


if __name__ == "__main__":
    unittest.main()
