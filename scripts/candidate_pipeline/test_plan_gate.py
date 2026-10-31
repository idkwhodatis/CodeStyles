import hashlib
from pathlib import Path
import tempfile
import unittest
from candidate_pipeline.build_candidate import load_reviewed_plan


class PlanGateTests(unittest.TestCase):
    def test_empty_malformed_and_wrong_digest_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            plan = Path(temp) / 'plan.json'
            plan.write_bytes(b'{}')
            digest = hashlib.sha256(b'{}').hexdigest()
            self.assertEqual(load_reviewed_plan(plan, digest), {})
            for value in ('', None, 'a' * 63, 'A' * 64, '0' * 64):
                with self.subTest(value=value), self.assertRaises(ValueError):
                    load_reviewed_plan(plan, value)


if __name__ == '__main__':
    unittest.main()
