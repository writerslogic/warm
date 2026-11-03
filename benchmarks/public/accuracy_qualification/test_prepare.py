# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import tempfile
import unittest
from pathlib import Path

import h5py
import numpy as np

from prepare import digest, extract


class PreparationTests(unittest.TestCase):
    def test_frozen_eligibility_order_and_flushed_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with h5py.File(root / "input.h5", "w") as source:
                data = source.create_dataset("train", data=np.array([[0, 0], [1, -2], [np.nan, 1], [1e300, 1], [3, 4], [5, 6]]))
                for count in [1, 2, 3]:
                    output = root / f"{count}.f32"
                    record = extract(data, count, 2, output)
                    self.assertEqual(record["retainedOrdinals"], [1, 4, 5][:count])
                    self.assertEqual(record["bytes"], count * 2 * 4)
                    self.assertEqual(record["sha256"], digest(output))
                    np.testing.assert_array_equal(np.fromfile(output, dtype="<f4").reshape(-1, 2), [[1, -2], [3, 4], [5, 6]][:count])
                self.assertEqual([item["ordinal"] for item in record["skipped"]], [0, 2, 3])
                with self.assertRaisesRegex(ValueError, "not enough eligible"):
                    extract(data, 4, 2, root / "insufficient.f32")

    def test_shape_and_output_overwrite_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with h5py.File(root / "input.h5", "w") as source:
                data = source.create_dataset("train", data=np.ones((3, 2)))
                for dimensions in [1, 3]:
                    with self.assertRaisesRegex(ValueError, "shape"):
                        extract(data, 2, dimensions, root / f"bad-{dimensions}")
                output = root / "rows.f32"
                extract(data, 2, 2, output)
                before = output.read_bytes()
                with self.assertRaises(FileExistsError):
                    extract(data, 2, 2, output)
                self.assertEqual(output.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
