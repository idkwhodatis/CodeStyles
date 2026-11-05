"""Fast offline checks for the replay recipe and fixture limits."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import verify_caddy_formatter_dprint as v


class ReplayTests(unittest.TestCase):
    def test_generated_inputs_match_committed_oracle(self):
        oracle = json.loads((v.FIXTURES / 'oracle-cases.json').read_text())
        generated = v.generated_cases()
        self.assertEqual(len(oracle), 560)
        self.assertEqual(sum(row['origin'] == 'upstream' for row in generated), 34)
        self.assertEqual([(r['description'], r['input'], r['origin']) for r in generated],
                         [(r['description'], r['input'], r['origin']) for r in oracle])
        v.validate_cases(oracle)
        for original, expected in zip(generated[:34], oracle[:34]):
            self.assertEqual(original['expect'], expected['expect'])

    def test_fixture_bounds(self):
        for cases in [[], [{'input': 'x'}] * (v.MAX_CASES + 1),
                      [{'description': 'large', 'input': 'x' * (v.MAX_INPUT + 1)}],
                      [{'description': 'large', 'input': '', 'expect': 'x' * (v.MAX_OUTPUT + 1)}]]:
            with self.assertRaises(ValueError):
                v.validate_cases(cases)

    def test_trusted_file_digest_is_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'fixture'
            path.write_bytes(b'fixture')
            self.assertEqual(v.checked_file(path, hashlib.sha256(b'fixture').hexdigest()), path)
            with self.assertRaisesRegex(ValueError, 'checksum'):
                v.checked_file(path, '0' * 64)


if __name__ == '__main__':
    unittest.main()
