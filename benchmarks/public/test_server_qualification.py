# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import copy
import unittest

from server_qualification import BATCH, GROUPS, document, validate_matches


class IntegrityOracleTests(unittest.TestCase):
    def rows(self):
        return [{**document("tenant-0", group, 3, member), "score": 0.9}
                for group in range(GROUPS) for member in range(BATCH)]

    def check(self, rows, deleted=(), bucket=None):
        validate_matches(rows, "tenant-0", [3] * GROUPS, [4] * GROUPS, set(deleted), bucket)

    def test_batch_boundary_membership(self):
        rows = self.rows()
        self.check(rows)
        with self.assertRaisesRegex(AssertionError, "partial or mixed"):
            self.check(rows[:-1])
        with self.assertRaisesRegex(AssertionError, "duplicate"):
            self.check(rows + [rows[0]])

    def test_revisions_tenant_and_forgetting(self):
        rows = self.rows()
        for revision, accepted in ((2, False), (3, True), (4, True), (5, False)):
            variant = [{**document("tenant-0", group, revision, member), "score": 0.9}
                       for group in range(GROUPS) for member in range(BATCH)]
            if accepted:
                self.check(variant)
            else:
                with self.assertRaisesRegex(AssertionError, "acknowledged replacement"):
                    self.check(variant)
        mixed = copy.deepcopy(rows)
        mixed[0] = {**document("tenant-0", 0, 4, 0), "score": 0.9}
        with self.assertRaisesRegex(AssertionError, "mixed batch"):
            self.check(mixed)
        wrong_tenant = copy.deepcopy(rows)
        wrong_tenant[0]["metadata"]["tenant"] = "tenant-1"
        with self.assertRaisesRegex(AssertionError, "cross-tenant"):
            self.check(wrong_tenant)
        with self.assertRaisesRegex(AssertionError, "deleted"):
            self.check(rows, deleted=[rows[0]["id"]])

    def test_filter_and_payload_integrity(self):
        rows = [row for row in self.rows() if row["metadata"]["bucket"] == 0]
        self.check(rows, bucket=0)
        with self.assertRaisesRegex(AssertionError, "filter violation"):
            self.check(self.rows(), bucket=0)
        rows[0]["text"] = "corrupt text"
        with self.assertRaisesRegex(AssertionError, "payload"):
            self.check(rows, bucket=0)


if __name__ == "__main__":
    unittest.main()
