"""Harness regression tests; fake payloads here do not constitute app GUI evidence."""
import io
import json
import os
from pathlib import Path, PureWindowsPath
import stat
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock
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
                    # Assign after construction to retain literal backslashes
                    # even on Windows, where ZipInfo normally normalizes them.
                    info = zipfile.ZipInfo("placeholder")
                    info.filename = info.orig_filename = name
                    out.writestr(info, b"bad")
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


class ProtectionTests(unittest.TestCase):
    def test_windows_acl_denies_only_mutations_not_shared_read_execute_rights(self):
        command = smoke.windows_deny_write_command("installation", "S-1-5-21-123")
        self.assertEqual(command, ["icacls", "installation", "/deny",
                                   "*S-1-5-21-123:(OI)(CI)(WD,AD,WEA,WA)", "/T", "/Q"])

    def test_partial_windows_acl_failure_removes_denial_before_fallback(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "sample").write_bytes(b"sample")
            calls = [SimpleNamespace(stdout='"user","S-1-5-21-123"\n'),
                     subprocess.CalledProcessError(5, ["icacls", "/deny"]),
                     SimpleNamespace(returncode=0)]
            try:
                with mock.patch.object(smoke, "os", SimpleNamespace(name="nt")), \
                        mock.patch.object(smoke.subprocess, "run", side_effect=calls) as run:
                    evidence, sid = smoke.protect_install(root)
                self.assertIsNone(sid)
                self.assertTrue(evidence["partial_acl_removed"])
                self.assertIn("exit status 5", evidence["acl_error"])
                self.assertEqual(run.call_args_list[2].args[0],
                                 ["icacls", str(root), "/remove:d", "*S-1-5-21-123", "/T", "/Q"])
            finally:
                smoke.restore_install(root, None)

    def test_protection_exception_restores_modes_and_partial_acl(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            sample = root / "sample"
            sample.write_bytes(b"sample")
            calls = [SimpleNamespace(stdout='"user","S-1-5-21-123"\n'),
                     SimpleNamespace(returncode=0), SimpleNamespace(returncode=0)]
            evidence = {}
            with mock.patch.object(smoke, "os", SimpleNamespace(name="nt")), \
                    mock.patch.object(smoke.subprocess, "run", side_effect=calls) as run, \
                    mock.patch.object(Path, "write_bytes", side_effect=OSError("probe failure")):
                with self.assertRaisesRegex(OSError, "probe failure"):
                    smoke.protect_install(root, evidence)
            self.assertEqual(run.call_args_list[-1].args[0][2], "/remove:d")
            self.assertTrue(sample.stat().st_mode & stat.S_IWUSR)
            self.assertTrue(root.stat().st_mode & stat.S_IWUSR)
            self.assertNotIn("cleanup_error", evidence)

    @unittest.skipUnless(os.name == "nt", "Requires native Windows ACL and executable semantics")
    def test_native_windows_readonly_acl_preserves_reads_and_execution(self):
        with tempfile.TemporaryDirectory(prefix="CodeStyles ACL 安装 ") as temp:
            root = Path(temp)
            nested = root / "nested resources"
            nested.mkdir()
            sample = nested / "sample.txt"
            sample.write_bytes(b"readable resource")
            executable = nested / "cmd.exe"
            shutil.copyfile(Path(os.environ["SystemRoot"]) / "System32/cmd.exe", executable)
            evidence, sid = smoke.protect_install(root)
            try:
                self.assertEqual(evidence["method"], "windows_deny_write_acl", evidence)
                self.assertTrue(evidence["write_denied"], evidence)
                self.assertEqual(sample.read_bytes(), b"readable resource")
                self.assertEqual(executable.read_bytes()[:2], b"MZ")
                self.assertTrue(smoke.snapshot(root))
                subprocess.run([str(executable), "/d", "/c", "exit", "0"],
                               check=True, capture_output=True, timeout=10)
                with self.assertRaises(PermissionError):
                    sample.write_bytes(b"changed")
                with self.assertRaises(PermissionError):
                    (nested / "new-file").write_bytes(b"changed")
                with self.assertRaises(PermissionError):
                    (nested / "new-directory").mkdir()
            finally:
                smoke.restore_install(root, sid)
            sample.write_bytes(b"cleanup restored write access")


class SmokeTests(unittest.TestCase):

    def test_windows_extended_drive_and_unc_prefixes_compare_equally(self):
        for extended, ordinary in [
            (r"\\?\C:\CodeStyles 安装\resources", r"c:\codestyles 安装\resources"),
            (r"\\?\UNC\server\share\安装\resources", r"\\SERVER\share\安装\resources"),
        ]:
            with self.subTest(path=extended):
                self.assertEqual(smoke.windows_comparison_path(extended), PureWindowsPath(ordinary))
        # Volume/device paths are not interchangeable with ordinary drive paths.
        volume = r"\\?\Volume{123}\resources"
        self.assertEqual(smoke.windows_comparison_path(volume), PureWindowsPath(volume))

    def test_windows_prefixed_resource_containment_and_cache_equality(self):
        # Pure path fixtures run on Linux/macOS too; real filesystem resolve is
        # exercised by the native Windows package smoke lane.
        with mock.patch.object(smoke, "resolved_comparison_path", side_effect=smoke.windows_comparison_path):
            for install, resources, cache, reported_cache in [
                (r"C:\安装", r"\\?\C:\安装\resources", r"C:\cache", r"\\?\C:\cache"),
                (r"\\server\share\安装", r"\\?\UNC\server\share\安装\resources",
                 r"\\server\share\cache", r"\\?\UNC\server\share\cache"),
            ]:
                with self.subTest(install=install):
                    self.assertTrue(smoke.is_within(resources, install))
                    self.assertFalse(smoke.is_within(install + " sibling", install))
                    report = {"status": "passed", "mode": "headless_standalone_preview",
                              "resources": {"root": resources, "discovery": "beside_executable",
                                            "checksums_verified": True},
                              "dprint_executable": resources + r"\bin\dprint.exe",
                              "cache_directory": reported_cache, "checks": []}
                    # Reaching language validation proves both containment and
                    # the differently prefixed cache paths were accepted.
                    with self.assertRaisesRegex(ValueError, "eight languages"):
                        smoke.validate_backend_report(report, install, cache)
                    report["cache_directory"] += " sibling"
                    with self.assertRaisesRegex(ValueError, "fresh external cache"):
                        smoke.validate_backend_report(report, install, cache)

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

    def test_report_retains_failed_stage_and_traceback_after_cleanup(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            payload = root / "payload"
            payload.mkdir()
            (payload / "codestyles").write_bytes(b"fixture")
            report = root / "report.json"
            with mock.patch.object(smoke, "protect_install", return_value=({"write_denied": True}, None)), \
                    mock.patch.object(smoke, "snapshot", side_effect=PermissionError("denied file read")):
                self.assertEqual(smoke.main(["--directory", str(payload), "--report", str(report)]), 1)
            result = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(result["error_stage"], "snapshot_before")
            self.assertEqual(result["error_type"], "PermissionError")
            self.assertIn("denied file read", result["traceback"])
            self.assertIn("snapshot(install)", result["traceback"])

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
