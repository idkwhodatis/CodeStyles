#!/usr/bin/env python3
"""Test a distributed payload through its own headless standalone-preview mode.

Extracts/copies into a Unicode/space path, with unrelated cwd and a fresh cache.
Unreachable proxy settings are defense in depth, NOT genuine network isolation.
Use a network-disabled container/VM for that evidence. No GUI is initialized.
"""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import traceback
import zipfile

EXPECTED_EXTENSIONS = {"ts", "js", "tsx", "jsx", "json", "jsonc", "md", "toml"}
EXPECTED_PLUGINS = {"typescript", "json", "markdown", "toml"}


def safe_member(root, name):
    path = PurePosixPath(name)
    if (not name or "\\" in name or path.is_absolute()
            or any(part in ("..", ".") or ":" in part for part in path.parts)):
        raise ValueError(f"Unsafe archive member: {name!r}")
    result = root.joinpath(*path.parts)
    if result == root:
        raise ValueError(f"Empty archive member: {name!r}")
    return result


def extract_archive(archive_path, destination):
    """Extract only regular files/directories; preserve Unix executable bits."""
    seen = set()

    def target(name):
        path = safe_member(destination, name)
        if path in seen:
            raise ValueError(f"Duplicate archive member: {name}")
        seen.add(path)
        return path

    if zipfile.is_zipfile(archive_path):
        with zipfile.ZipFile(archive_path) as archive:
            for member in archive.infolist():
                # ZipInfo.filename normalizes backslashes on Windows. Check the
                # original stored name so malformed members cannot be hidden.
                path = target(member.orig_filename)
                mode = member.external_attr >> 16
                if stat.S_ISLNK(mode):
                    raise ValueError(f"Archive symlinks are unsupported: {member.filename}")
                if member.is_dir():
                    path.mkdir(parents=True, exist_ok=True)
                    continue
                if stat.S_IFMT(mode) not in (0, stat.S_IFREG):
                    raise ValueError(f"Non-regular archive member: {member.filename}")
                path.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(member) as source, path.open("wb") as output:
                    shutil.copyfileobj(source, output)
                path.chmod((mode & 0o777) or 0o644)
    else:
        with tarfile.open(archive_path, "r:*") as archive:
            for member in archive:
                path = target(member.name)
                if member.isdir():
                    path.mkdir(parents=True, exist_ok=True)
                elif member.isfile():
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with archive.extractfile(member) as source, path.open("wb") as output:
                        shutil.copyfileobj(source, output)
                    path.chmod(member.mode & 0o777)
                else:
                    raise ValueError(f"Non-regular archive member: {member.name}")


def snapshot(root):
    result = {}
    for path in [root, *sorted(root.rglob("*"))]:
        info = path.lstat()
        value = {"mode": stat.S_IMODE(info.st_mode)}
        if path.is_symlink():
            value["symlink"] = os.readlink(path)
        elif path.is_file():
            value.update(bytes=info.st_size, mtime_ns=info.st_mtime_ns,
                         sha256=hashlib.sha256(path.read_bytes()).hexdigest())
        else:
            value["directory"] = True
        result[path.relative_to(root).as_posix()] = value
    return result


def windows_deny_write_command(root, sid):
    # Generic W/GW also denies SYNCHRONIZE, which generic read/execute request.
    # Deny only mutation rights so icacls traversal and application loading work.
    # https://learn.microsoft.com/windows/win32/fileio/file-security-and-access-rights
    return ["icacls", str(root), "/deny", f"*{sid}:(OI)(CI)(WD,AD,WEA,WA)", "/T", "/Q"]


def remove_windows_write_deny(root, sid):
    subprocess.run(["icacls", str(root), "/remove:d", f"*{sid}", "/T", "/Q"],
                   check=True, capture_output=True, text=True)


def protect_install(root, evidence=None):
    """Return evidence plus cleanup data; unwind partially applied protection."""
    acl_sid = None
    if evidence is None:
        evidence = {}
    evidence.update(method="unix_mode_bits", write_denied=False)
    try:
        for path in sorted(root.rglob("*"), reverse=True) + [root]:
            if not path.is_symlink():
                path.chmod(stat.S_IMODE(path.stat().st_mode) & ~0o222)
        if os.name == "nt":
            evidence["method"] = "windows_readonly_attributes"
            evidence["limitation"] = "Windows read-only attributes do not prohibit directory writes."
            # Only our disposable copy is changed. If icacls applies part of an
            # ACL before failing, remove it before falling back to attributes.
            try:
                identity = subprocess.run(["whoami", "/user", "/fo", "csv", "/nh"],
                                          capture_output=True, text=True, check=True)
                sid = next(csv.reader(identity.stdout.strip().splitlines()))[1]
                acl_sid = sid
                command = windows_deny_write_command(root, sid)
                evidence["deny_rights"] = ["WD", "AD", "WEA", "WA"]
                subprocess.run(command, check=True, capture_output=True, text=True)
                evidence.update(method="windows_deny_write_acl", limitation=None)
            except (OSError, subprocess.SubprocessError, IndexError, StopIteration) as error:
                evidence["acl_error"] = str(error)
                if acl_sid:
                    remove_windows_write_deny(root, acl_sid)
                    acl_sid = None
                    evidence["partial_acl_removed"] = True
        # Probe directory creation instead of trusting attributes or identity.
        probe = root / ".codestyles-write-probe"
        try:
            probe.write_bytes(b"write protection probe")
        except PermissionError:
            evidence["write_denied"] = True
        else:
            probe.unlink()
            evidence["limitation"] = "Current process can still write to installation; snapshot detects persistent changes only."
        return evidence, acl_sid
    except BaseException:
        # The caller has no cleanup token until this function returns. Undo
        # mode changes and even a partially installed ACL on exceptional exits.
        try:
            restore_install(root, acl_sid)
        except (OSError, subprocess.SubprocessError) as error:
            evidence["cleanup_error"] = str(error)
        raise


def restore_install(root, acl_sid):
    if acl_sid:
        remove_windows_write_deny(root, acl_sid)
    for path in [root, *root.rglob("*")]:
        if not path.is_symlink():
            path.chmod(stat.S_IMODE(path.stat().st_mode) | (0o700 if path.is_dir() else 0o600))


def find_binary(root):
    candidates = [path for path in root.rglob("*")
                  if path.name in ("codestyles", "codestyles.exe")
                  and path.is_file() and not path.is_symlink()]
    if len(candidates) != 1:
        raise ValueError(f"Expected one packaged CodeStyles executable, found {len(candidates)}")
    return candidates[0]


def isolated_environment(workspace):
    env = dict(os.environ)
    for key in list(env):
        if key.upper() in {"CODESTYLES_RESOURCES", "DPRINT_CACHE_DIR", "DISPLAY", "WAYLAND_DISPLAY"}:
            del env[key]
    directories = {"HOME": "user home", "USERPROFILE": "user home", "APPDATA": "user data",
                   "LOCALAPPDATA": "local data", "XDG_CACHE_HOME": "user cache",
                   "TMPDIR": "temporary files", "TMP": "temporary files", "TEMP": "temporary files"}
    for key, name in directories.items():
        path = workspace / name
        path.mkdir(exist_ok=True)
        env[key] = str(path)
    # This path must not exist before the packaged executable starts.
    env["DPRINT_CACHE_DIR"] = str(workspace / "fresh dprint cache 缓存")
    for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
        env[key] = "http://127.0.0.1:9"
    env["NO_PROXY"] = env["no_proxy"] = ""
    return env


def windows_comparison_path(resolved_path):
    """Normalize an already resolved Windows path for lexical comparison.

    Rust canonicalize emits extended-length drive/UNC paths; Python's resolve
    can emit ordinary paths. Keep filesystem resolution separate so symlink
    containment remains checked before normalizing the equivalent prefixes.
    """
    value = str(resolved_path)
    if value[:8].upper() == "\\\\?\\UNC\\":
        value = "\\\\" + value[8:]
    elif (value.startswith("\\\\?\\") and len(value) >= 7
          and value[4].lower() in "abcdefghijklmnopqrstuvwxyz" and value[5:7] == ":\\"):
        value = value[4:]
    return PureWindowsPath(value)


def resolved_comparison_path(path):
    resolved = Path(path).resolve()
    return windows_comparison_path(resolved) if os.name == "nt" else resolved


def is_within(path, root):
    try:
        resolved_comparison_path(path).relative_to(resolved_comparison_path(root))
        return True
    except ValueError:
        return False


def validate_backend_report(report, install, cache):
    if report.get("status") != "passed" or report.get("mode") != "headless_standalone_preview":
        raise ValueError(f"Packaged self-test failed: {report.get('error', 'invalid report')}")
    resources = report.get("resources", {})
    if resources.get("discovery") not in {"beside_executable", "macos_app_bundle"}:
        raise ValueError("Resources were not discovered from the packaged installation")
    if not resources.get("checksums_verified") or not is_within(resources.get("root", ""), install):
        raise ValueError("Verified resources are not inside the extracted installation")
    if not is_within(report.get("dprint_executable", ""), install):
        raise ValueError("Formatter executable is outside the installation")
    if resolved_comparison_path(report.get("cache_directory", "")) != resolved_comparison_path(cache):
        raise ValueError("Self-test did not use the fresh external cache")
    checks = report.get("checks", [])
    if len(checks) != 8 or {item.get("extension") for item in checks} != EXPECTED_EXTENSIONS:
        raise ValueError("Self-test did not cover all eight languages")
    if {item.get("plugin") for item in checks} != EXPECTED_PLUGINS:
        raise ValueError("Self-test did not cover all four WASM plugins")
    for item in checks:
        evidence = item.get("evidence", {})
        if item.get("status") != "passed" or not evidence.get("changed") or not evidence.get("idempotent"):
            raise ValueError(f"Formatter check failed: {item}")


def smoke(args, result):
    result["stage"] = "create_workspace"
    with tempfile.TemporaryDirectory(prefix="CodeStyles package 验证 ") as temp:
        workspace = Path(temp).resolve()
        install = workspace / "安装 CodeStyles read only"
        install.mkdir()
        if args.archive:
            result["stage"] = "extract_payload"
            extract_archive(args.archive, install)
        else:
            result["stage"] = "copy_payload"
            # Preserve relative symlinks from native package layouts; executable
            # discovery ignores aliases, and the source directory is untouched.
            shutil.copytree(args.directory, install, dirs_exist_ok=True, symlinks=True)
        result["stage"] = "locate_executable"
        executable = find_binary(install)
        result["stage"] = "prepare_environment"
        env = isolated_environment(workspace)
        cache = Path(env["DPRINT_CACHE_DIR"])
        cwd = workspace / "unrelated working directory 工作"
        cwd.mkdir()
        report_path = workspace / "backend self-test.json"
        result.update(executable=str(executable), install_directory=str(install),
                      working_directory=str(cwd), cache_directory=str(cache),
                      cache_initially_absent=not cache.exists(), resource_override_removed=True)
        result["stage"] = "protect_install"
        result["read_only_install"] = {}
        protection, acl_sid = protect_install(install, result["read_only_install"])
        try:
            if os.name != "nt" and not protection["write_denied"]:
                raise ValueError("Unix install-tree write protection is ineffective; run as a non-root user")
            if args.require_readonly and not protection["write_denied"]:
                raise ValueError("Installation is not protected against writes")
            result["stage"] = "snapshot_before"
            before = snapshot(install)
            result["stage"] = "execute_self_test"
            process = subprocess.run([str(executable), "--self-test", str(report_path)],
                                     cwd=cwd, env=env, capture_output=True, text=True,
                                     encoding="utf-8", errors="replace", timeout=args.timeout)
            result.update(exit_code=process.returncode, stdout=process.stdout, stderr=process.stderr)
            result["stage"] = "snapshot_after"
            result["install_tree_unchanged"] = before == snapshot(install)
            result["stage"] = "read_backend_report"
            if report_path.is_file():
                result["backend"] = json.loads(report_path.read_text(encoding="utf-8"))
            if process.returncode != 0:
                raise ValueError(f"Packaged executable exited {process.returncode}: {process.stderr}")
            if not result["install_tree_unchanged"]:
                raise ValueError("Packaged executable modified its installation")
            result["stage"] = "validate_backend_report"
            validate_backend_report(result.get("backend", {}), install, cache)
            result["stage"] = "verify_cache"
            result["cache_populated"] = cache.is_dir() and any(path.is_file() for path in cache.rglob("*"))
            if not result["cache_populated"]:
                raise ValueError("Formatter did not populate the fresh external cache")
        finally:
            failed = sys.exc_info()[0] is not None
            if not failed:
                result["stage"] = "restore_install"
            try:
                restore_install(install, acl_sid)
            except (OSError, subprocess.SubprocessError) as error:
                result["cleanup_error"] = str(error)
                if not failed:
                    raise
        result["stage"] = "cleanup_workspace"
    result["stage"] = "complete"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--archive", type=Path)
    source.add_argument("--directory", type=Path, help="Copy a mounted/installed payload; never alter its source")
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--require-readonly", action="store_true", help="Also reject Windows attribute-only fallback")
    parser.add_argument("--network-isolation", choices=["proxy-only", "external"], default="proxy-only",
                        help="external records the caller's network-disabled container/VM guarantee")
    args = parser.parse_args(argv)
    result = {"format_version": 1, "status": "failed", "input": str(args.archive or args.directory),
              "network": {"mode": args.network_isolation, "unreachable_proxies": True,
                          "isolation_enforced_by_script": False,
                          "note": "Caller asserts external network isolation." if args.network_isolation == "external"
                          else "Unreachable proxies are not genuine network isolation."},
              "gui": "not_tested"}
    try:
        smoke(args, result)
        result["status"] = "passed"
    except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        result["error"] = str(error)
        result["error_type"] = type(error).__name__
        result["error_stage"] = result.get("stage", "unknown")
        result["traceback"] = traceback.format_exc()
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"Package smoke {result['status']}: {args.report}")
    if result.get("error"):
        print(f"{result['error_stage']}: {result['error']}")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
