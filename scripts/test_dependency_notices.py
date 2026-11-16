"""Offline checks for the exact audited notice bytes and source availability."""
import hashlib
import json
import tarfile
import unittest

import bundle_resources as bundle


class DependencyNoticeTests(unittest.TestCase):
    def test_required_notice_policy_has_independent_expected_names(self):
        app = {
            'licenses/gpui-kit-0.7.3-APACHE-2.0.txt',
            'licenses/gpui-kit-assets-0.7.3-LICENSE-LUCIDE.txt',
            'licenses/gpui-kit-assets-0.7.3-SOURCE.txt',
        }
        dprint = {
            'licenses/wasmtime-49.0.2-cranelift-0.136.2.txt',
            'licenses/dprint-0.63.0-dependencies-SOURCE.txt',
        }
        mpl = {
            'licenses/option-ext-0.2.0-MPL-2.0.txt',
            'licenses/option-ext-0.2.0-SOURCE.txt',
            'licenses/option-ext-0.2.0.crate',
        }
        for arch in ('x86_64', 'aarch64'):
            for os in ('unknown-linux-gnu', 'apple-darwin', 'pc-windows-msvc'):
                target = f'{arch}-{os}'
                for component, expected in [('app', app), ('dprint', dprint)]:
                    with self.subTest(target=target, component=component):
                        actual = bundle.dependency_notices(component, target)
                        self.assertEqual(len(actual), len(set(actual)))
                        self.assertEqual(set(actual), expected | (mpl if 'linux' in os else set()))

    def test_exact_upstream_notices_and_source_are_pinned(self):
        expected = {
            'wasmtime-49.0.2-cranelift-0.136.2.txt': '268872b9816f90fd8e85db5a28d33f8150ebb8dd016653fb39ef1f94f2686bc5',
            'gpui-kit-assets-0.7.3-LICENSE-LUCIDE.txt': 'b495047bd93a9b06913511076f504daba17d5bbeb3e0650f3bb53a4220329c57',
            'gpui-kit-0.7.3-APACHE-2.0.txt': 'd1b0449e5478c574ba4f686c2656df7fe77d66821a61f8b6ed3378a58ed9a811',
            'option-ext-0.2.0-MPL-2.0.txt': '66a3107d5ad6a058aab753eaac2047ccb2ed0e39465dd0fe5844da3e300d5172',
            'option-ext-0.2.0.crate': '04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d',
        }
        manifest = json.loads((bundle.RESOURCES / 'manifest.json').read_text())
        names = set(bundle.dependency_notices('app', 'x86_64-unknown-linux-gnu'))
        names.update(bundle.dependency_notices('dprint', 'x86_64-unknown-linux-gnu'))
        for entry in bundle.pinned_notices(manifest, names):
            bundle.verify(bundle.RESOURCES / entry['path'], entry)
        for name, digest in expected.items():
            with self.subTest(name=name):
                self.assertEqual(hashlib.sha256((bundle.RESOURCES / 'licenses' / name).read_bytes()).hexdigest(), digest)

    def test_mpl_source_is_available_offline_and_keeps_original_license(self):
        licenses = bundle.RESOURCES / 'licenses'
        with tarfile.open(licenses / 'option-ext-0.2.0.crate') as source:
            self.assertTrue({'option-ext-0.2.0/src/lib.rs', 'option-ext-0.2.0/Cargo.toml',
                             'option-ext-0.2.0/LICENSE.txt'} <= set(source.getnames()))
            self.assertEqual(source.extractfile('option-ext-0.2.0/LICENSE.txt').read(),
                             (licenses / 'option-ext-0.2.0-MPL-2.0.txt').read_bytes())
        instructions = (licenses / 'option-ext-0.2.0-SOURCE.txt').read_text()
        self.assertIn('option-ext-0.2.0.crate', instructions)
        self.assertIn('not a claim of executable inclusion', instructions)

    def test_duplicate_pins_are_rejected_and_component_is_explicit(self):
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            bundle.pinned_notices({'files': [{'path': 'same'}, {'path': 'same'}]}, ['same'])
        with self.assertRaisesRegex(ValueError, 'Unknown notice component'):
            bundle.dependency_notices('unknown', 'x86_64-unknown-linux-gnu')


if __name__ == '__main__':
    unittest.main()
