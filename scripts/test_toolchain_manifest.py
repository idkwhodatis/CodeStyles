"""Pure maintainer manifest-selection tests; no network or native execution."""
import unittest
from generate_toolchain_manifest import selected_path


class ToolchainSelectionTests(unittest.TestCase):
    def test_rustfmt_only_no_cargo(self):
        self.assertEqual(selected_path('rustfmt-version/rustfmt-preview/bin/rustfmt', 'rustfmt-preview'), 'bin/rustfmt')
        self.assertEqual(selected_path('rustfmt-version/rustfmt-preview/bin/rustfmt.exe', 'rustfmt-preview'), 'bin/rustfmt.exe')
        self.assertIsNone(selected_path('rustfmt-version/rustfmt-preview/bin/cargo-fmt', 'rustfmt-preview'))

    def test_shared_library_closure_not_compiler_or_std(self):
        for name in ('lib/librustc_driver-hash.so', 'lib/libLLVM.so.23.1', 'lib/librustc_driver.dylib', 'bin/rustc_driver.dll'):
            self.assertEqual(selected_path('rustc-version/rustc/' + name, 'rustc'), name)
        for name in ('bin/rustc', 'bin/rustdoc', 'lib/rustlib/x86_64/bin/rust-lld', 'lib/rustlib/x86_64/lib/libstd.rlib',
                     'lib/libserde_derive-deadbeef.dylib', 'lib/librustc-stable_rt.asan.dylib',
                     'bin/unreviewed.dll', 'lib/unreviewed.so'):
            self.assertIsNone(selected_path('rustc-version/rustc/' + name, 'rustc'))

    def test_complete_notices_and_no_traversal(self):
        self.assertEqual(selected_path('rustc-version/LICENSE-MIT', 'rustc'), 'licenses/rustc/LICENSE-MIT')
        self.assertEqual(selected_path('rustc-version/rustc/share/doc/rust/COPYRIGHT.html', 'rustc'), 'licenses/rustc/COPYRIGHT.html')
        self.assertIsNone(selected_path('rustc-version/rustc/../bin/rustc', 'rustc'))


if __name__ == '__main__':
    unittest.main()
