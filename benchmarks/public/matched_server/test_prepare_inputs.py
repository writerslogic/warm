# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import unittest

import numpy as np

from prepare_inputs import cohort


class CohortTests(unittest.TestCase):
    def test_exact_source_range_at_length_boundary(self):
        for rows in [1999, 2000, 2001]:
            values = np.arange(1, rows + 1, dtype=np.float64)[:, None]
            if rows < 2000:
                with self.assertRaises(ValueError):
                    cohort(values, 1)
            else:
                result = cohort(values, 1)
                self.assertEqual(result.shape, (1000, 1))
                self.assertEqual(result.dtype, np.dtype('<f4'))
                np.testing.assert_array_equal(result[:, 0], np.arange(1001, 2001))

    def test_invalid_query_is_never_replaced_with_an_adjacent_row(self):
        for value in [0., np.nan, np.inf, 1e100]:
            for ordinal in [999, 1000, 1999, 2000]:
                values = np.ones((2001, 1))
                values[ordinal] = value
                if 1000 <= ordinal < 2000:
                    with self.assertRaisesRegex(ValueError, str(ordinal)):
                        cohort(values, 1)
                else:
                    self.assertEqual(cohort(values, 1).shape, (1000, 1))
        with self.assertRaises(ValueError):
            cohort(np.ones((2000, 2)), 1)


if __name__ == '__main__':
    unittest.main()
