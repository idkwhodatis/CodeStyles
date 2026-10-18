"""Harness regression tests; fake payloads here do not constitute app GUI evidence."""
import io
import json
import os
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
import zipfile

import smoke_package as smoke


class ExtractionTests(unittest.TestCase):
    def test_zip_preserves_execute_permission_and_unicode(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "bundle.zip"
            with zipfile.ZipFile(archive, "w") as out:
                info = zipfile.ZipInfo("安装 with spaces/codestyles")
                info.external_attr = 0o100755 << 16
                out.writestr(info, b"binary")
            smoke.extract_archive(archive, root / "extracted")
            executable = smoke.find_binary(root / "extracted")
            self.assertEqual(executable.read_bytes(), b"binary")
            if os.name != "nt":
                self.assertTrue(executable.stat().st_mode & stat.S_IXUSR)

    def test_tar_preserves_execute_permission(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "bundle.tar.gz"
            with tarfile.open(archive, "w:gz") as out:
                info = tarfile.TarInfo("bundle/codestyles")
                info.mode = 0o755
                info.size = 6
                out.addfile(info, io.BytesIO(b"binary"))
            smoke.extract_archive(archive, root / "extracted")
            self.assertEqual(smoke.find_binary(root / "extracted").read_bytes(), b"binary")

    def test_archive_traversal_duplicates_and_symlinks_rejected(self):
        for name in ("../escape", "/absolute", "C:/drive", "a\\b"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                archive = root / "bad.zip"
                with zipfile.ZipFile(archive, "w") as out:
                    out.writestr(name, b"bad")
                with self.assertRaises(ValueError):
                    smoke.extract_archive(archive, root / "extracted")
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "link.tar"
            with tarfile.open(archive, "w") as out:
                info = tarfile.TarInfo("link")
                info.type = tarfile.SYMTYPE
                info.linkname = "../escape"
                out.addfile(info)
            with self.assertRaises(ValueError):
                smoke.extract_archive(archive, root / "extracted")


class SmokeTests(unittest.TestCase):
    def test_clean_environment_uses_cold_external_cache(self):
        with tempfile.TemporaryDirectory() as temp:
            env = smoke.isolated_environment(Path(temp))
            self.assertNotIn("CODESTYLES_RESOURCES", env)
            self.assertFalse(Path(env["DPRINT_CACHE_DIR"]).exists())
            self.assertEqual(env["HTTPS_PROXY"], "http://127.0.0.1:9")
            self.assertEqual(env["NO_PROXY"], "")
            self.assertNotIn("DISPLAY", env)

    def test_validation_rejects_checkout_fallback_and_missing_languages(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            cache = root / "cache"
            report = {"status": "passed", "mode": "headless_standalone_preview",
                      "resources": {"root": str(root / "resources"),
                                    "discovery": "development_checkout", "checksums_verified": True},
                      "dprint_executable": str(root / "resources/bin/dprint"),
                      "cache_directory": str(cache), "checks": []}
            with self.assertRaisesRegex(ValueError, "not discovered"):
                smoke.validate_backend_report(report, root, cache)
            report["resources"]["discovery"] = "beside_executable"
            with self.assertRaisesRegex(ValueError, "eight languages"):
                smoke.validate_backend_report(report, root, cache)

    @unittest.skipIf(os.name == "nt", "POSIX fake executable fixture")
    def test_directory_payload_is_copied_and_original_is_unchanged(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            payload = root / "original"
            payload.mkdir()
            binary = payload / "codestyles"
            # This fixture verifies the harness only. Actual package evidence
            # must use the compiled Rust application and its real WASM plugins.
            binary.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(__file__).resolve().parent
cache = pathlib.Path(os.environ['DPRINT_CACHE_DIR'])
assert not cache.exists()
assert 'CODESTYLES_RESOURCES' not in os.environ
assert pathlib.Path.cwd() != root
cache.mkdir()
(cache / 'compiled-test-fixture').write_text('fixture')
checks = []
for ext, plugin in [('ts','typescript'), ('js','typescript'), ('tsx','typescript'), ('jsx','typescript'), ('json','json'), ('jsonc','json'), ('md','markdown'), ('toml','toml')]:
    checks.append({'extension': ext, 'plugin': plugin, 'status': 'passed', 'evidence': {'changed': True, 'idempotent': True}})
pathlib.Path(sys.argv[2]).write_text(json.dumps({'status': 'passed', 'mode': 'headless_standalone_preview', 'resources': {'root': str(root / 'resources'), 'discovery': 'beside_executable', 'checksums_verified': True}, 'dprint_executable': str(root / 'resources/bin/dprint'), 'cache_directory': str(cache), 'checks': checks}))
''', encoding="utf-8")
            binary.chmod(0o755)
            before = smoke.snapshot(payload)
            report = root / "report.json"
            status = smoke.main(["--directory", str(payload), "--report", str(report)])
            self.assertEqual(status, 0)
            result = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(result["status"], "passed")
            self.assertTrue(result["read_only_install"]["write_denied"])
            self.assertTrue(result["install_tree_unchanged"])
            self.assertFalse(result["network"]["isolation_enforced_by_script"])
            self.assertEqual(smoke.snapshot(payload), before)

    def test_invalid_package_produces_failed_json_and_nonzero(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            payload = root / "empty"
            payload.mkdir()
            report = root / "report.json"
            self.assertEqual(smoke.main(["--directory", str(payload), "--report", str(report)]), 1)
            self.assertEqual(json.loads(report.read_text())["status"], "failed")


if __name__ == "__main__":
    unittest.main()
