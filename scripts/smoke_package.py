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
import tarfile
import tempfile
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


def protect_install(root):
    """Return evidence plus cleanup data. Windows attributes alone are not ACLs."""
    acl_sid = None
    evidence = {"method": "unix_mode_bits", "write_denied": False}
    for path in sorted(root.rglob("*"), reverse=True) + [root]:
        if not path.is_symlink():
            path.chmod(stat.S_IMODE(path.stat().st_mode) & ~0o222)
    if os.name == "nt":
        evidence["method"] = "windows_readonly_attributes"
        evidence["limitation"] = "Windows read-only attributes do not prohibit directory writes."
        # Try an actual deny-write ACL on our disposable copy. This never changes
        # the source installation and is removed before temporary-dir cleanup.
        try:
            identity = subprocess.run(["whoami", "/user", "/fo", "csv", "/nh"],
                                      capture_output=True, text=True, check=True)
            sid = next(csv.reader(identity.stdout.strip().splitlines()))[1]
            acl_sid = sid  # Remove any partial ACL changes even if icacls fails.
            subprocess.run(["icacls", str(root), "/deny", f"*{sid}:(OI)(CI)(W)", "/T", "/Q"],
                           check=True, capture_output=True, text=True)
            evidence.update(method="windows_deny_write_acl", limitation=None)
        except (OSError, subprocess.SubprocessError, IndexError, StopIteration) as error:
            evidence["acl_error"] = str(error)
    # Probe directory creation instead of trusting chmod/administrator identity.
    probe = root / ".codestyles-write-probe"
    try:
        probe.write_bytes(b"write protection probe")
    except PermissionError:
        evidence["write_denied"] = True
    else:
        probe.unlink()
        evidence["limitation"] = "Current process can still write to installation; snapshot detects persistent changes only."
    return evidence, acl_sid


def restore_install(root, acl_sid):
    if acl_sid:
        subprocess.run(["icacls", str(root), "/remove:d", f"*{acl_sid}", "/T", "/Q"],
                       check=True, capture_output=True, text=True)
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
    with tempfile.TemporaryDirectory(prefix="CodeStyles package 验证 ") as temp:
        workspace = Path(temp).resolve()
        install = workspace / "安装 CodeStyles read only"
        install.mkdir()
        if args.archive:
            extract_archive(args.archive, install)
        else:
            # Preserve relative symlinks from native package layouts; executable
            # discovery ignores aliases, and the source directory is untouched.
            shutil.copytree(args.directory, install, dirs_exist_ok=True, symlinks=True)
        executable = find_binary(install)
        env = isolated_environment(workspace)
        cache = Path(env["DPRINT_CACHE_DIR"])
        cwd = workspace / "unrelated working directory 工作"
        cwd.mkdir()
        report_path = workspace / "backend self-test.json"
        result.update(executable=str(executable), install_directory=str(install),
                      working_directory=str(cwd), cache_directory=str(cache),
                      cache_initially_absent=not cache.exists(), resource_override_removed=True)
        protection, acl_sid = protect_install(install)
        result["read_only_install"] = protection
        try:
            if os.name != "nt" and not protection["write_denied"]:
                raise ValueError("Unix install-tree write protection is ineffective; run as a non-root user")
            if args.require_readonly and not protection["write_denied"]:
                raise ValueError("Installation is not protected against writes")
            before = snapshot(install)
            process = subprocess.run([str(executable), "--self-test", str(report_path)],
                                     cwd=cwd, env=env, capture_output=True, text=True,
                                     encoding="utf-8", errors="replace", timeout=args.timeout)
            result.update(exit_code=process.returncode, stdout=process.stdout, stderr=process.stderr)
            result["install_tree_unchanged"] = before == snapshot(install)
            if report_path.is_file():
                result["backend"] = json.loads(report_path.read_text(encoding="utf-8"))
            if process.returncode != 0:
                raise ValueError(f"Packaged executable exited {process.returncode}: {process.stderr}")
            if not result["install_tree_unchanged"]:
                raise ValueError("Packaged executable modified its installation")
            validate_backend_report(result.get("backend", {}), install, cache)
            result["cache_populated"] = cache.is_dir() and any(path.is_file() for path in cache.rglob("*"))
            if not result["cache_populated"]:
                raise ValueError("Formatter did not populate the fresh external cache")
        finally:
            restore_install(install, acl_sid)


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
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"Package smoke {result['status']}: {args.report}")
    if result.get("error"):
        print(result["error"])
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
