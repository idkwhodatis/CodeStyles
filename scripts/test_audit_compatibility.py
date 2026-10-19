#!/usr/bin/env python3
"""Cross-platform binary-policy fixtures; never execute fixture machine code."""
import io
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile

import audit_compatibility as audit

POLICY = json.loads(audit.DEFAULT_POLICY.read_text())
LINUX = 'x86_64-unknown-linux-gnu'
MAC = 'x86_64-apple-darwin'
WIN = 'x86_64-pc-windows-msvc'


def elf(machine=62, versions=('GLIBC_2.28',), needed=('libc.so.6',), rpath=None,
        runpath=None, interpreter='/lib64/ld-linux-x86-64.so.2', extra=b''):
    data = bytearray(4096)
    data[:16] = b'\x7fELF\x02\x01\x01' + b'\0' * 9
    phnum = 3 if interpreter else 2
    struct.pack_into('<HHIQQQIHHHHHH', data, 16, 3, machine, 1, 0, 64, 0, 0, 64, 56, phnum, 0, 0, 0)
    base, dynoff, stroff, veroff = 0x400000, 0x180, 0x400, 0x800
    struct.pack_into('<IIQQQQQQ', data, 64, 1, 5, 0, base, base, len(data), len(data), 4096)
    strings = bytearray(b'\0')
    def string(value):
        offset = len(strings)
        strings.extend(value.encode() + b'\0')
        return offset
    tags = [(1, string(name)) for name in needed]
    if rpath is not None:
        tags.append((15, string(rpath)))
    if runpath is not None:
        tags.append((29, string(runpath)))
    if versions:
        filename = string('libc.so.6')
        struct.pack_into('<HHIII', data, veroff, 1, len(versions), filename, 16, 0)
        for i, version in enumerate(versions):
            struct.pack_into('<IHHII', data, veroff + 16 + 16 * i, 0, 0, 1, string(version), 16 if i + 1 < len(versions) else 0)
        tags += [(0x6ffffffe, base + veroff), (0x6fffffff, 1)]
    tags += [(5, base + stroff), (10, len(strings)), (0, 0)]
    for i, values in enumerate(tags):
        struct.pack_into('<qQ', data, dynoff + i * 16, *values)
    struct.pack_into('<IIQQQQQQ', data, 120, 2, 6, dynoff, base + dynoff, base + dynoff, len(tags) * 16, len(tags) * 16, 8)
    data[stroff:stroff + len(strings)] = strings
    if interpreter:
        raw = interpreter.encode() + b'\0'
        data[0x100:0x100 + len(raw)] = raw
        struct.pack_into('<IIQQQQQQ', data, 176, 3, 4, 0x100, base + 0x100, base + 0x100, len(raw), len(raw), 1)
    return bytes(data) + extra


def mach(cpu=0x1000007, minimum=(10, 15, 7), dylibs=('/usr/lib/libSystem.B.dylib',),
         rpaths=(), old_minimum=False, platform=1, filetype=2):
    commands = []
    if minimum:
        packed = minimum[0] << 16 | minimum[1] << 8 | minimum[2]
        commands.append(struct.pack('<IIII', 0x24, 16, packed, packed) if old_minimum else
                        struct.pack('<IIIIII', 0x32, 24, platform, packed, packed, 0))
    for cmd, names, header in [(0xc, dylibs, 24), (0x8000001c, rpaths, 12)]:
        for name in names:
            raw = name.encode() + b'\0'
            size = (header + len(raw) + 7) // 8 * 8
            commands.append(struct.pack('<III', cmd, size, header) + b'\0' * (header - 12) + raw + b'\0' * (size - header - len(raw)))
    body = b''.join(commands)
    return struct.pack('<IIIIIIII', 0xfeedfacf, cpu, 3, filetype, len(commands), len(body), 0, 0) + body


def fat(slices):
    offset = 8 + 20 * len(slices)
    headers, bodies = [], []
    for cpu, content in slices:
        headers.append(struct.pack('>IIIII', cpu, 3, offset, len(content), 0))
        bodies.append(content)
        offset += len(content)
    return struct.pack('>II', 0xcafebabe, len(slices)) + b''.join(headers + bodies)


def pe(machine=0x8664, subsystem=2, imports=('KERNEL32.dll',), delay=('ucrtbase.dll',), dll=False, is64=True):
    data = bytearray(4096)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3c, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    opt, optsize = 0x98, 240 if is64 else 224
    struct.pack_into('<HHIIIHH', data, 0x84, machine, 1, 0, 0, 0, optsize, 0x2022 if dll else 0x22)
    struct.pack_into('<H', data, opt, 0x20b if is64 else 0x10b)
    struct.pack_into('<Q' if is64 else '<I', data, opt + (24 if is64 else 28), 0x400000)
    struct.pack_into('<HH', data, opt + 40, 6, 0)
    struct.pack_into('<HH', data, opt + 48, 6, 0)
    struct.pack_into('<I', data, opt + 60, 0x400)
    struct.pack_into('<H', data, opt + 68, subsystem)
    dirs = 112 if is64 else 96
    struct.pack_into('<I', data, opt + dirs - 4, 16)
    section = opt + optsize
    data[section:section + 8] = b'.rdata\0\0'
    struct.pack_into('<IIII', data, section + 8, 0xc00, 0x1000, 0xc00, 0x400)
    next_name = 0x900
    for index, names, offset, step in [(1, imports, 0x400, 20), (13, delay, 0x600, 32)]:
        if names:
            struct.pack_into('<II', data, opt + dirs + 8 * index, offset + 0xc00, (len(names) + 1) * step)
            for i, name in enumerate(names):
                raw = name.encode() + b'\0'
                data[next_name:next_name + len(raw)] = raw
                if index == 1:
                    struct.pack_into('<IIIII', data, offset + step * i, 0, 0, 0, next_name + 0xc00, 0)
                else:
                    struct.pack_into('<IIIIIIII', data, offset + step * i, 1, next_name + 0xc00, 0, 0, 0, 0, 0, 0)
                next_name += len(raw)
    return bytes(data)


class AuditTests(unittest.TestCase):
    def run_audit(self, files, target=LINUX, archive=None, policy=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'root'
            root.mkdir()
            for name, content in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            bundle = root
            if archive:
                bundle = Path(tmp) / ('bundle.zip' if archive == 'zip' else 'bundle.tar.gz')
                if archive == 'zip':
                    with zipfile.ZipFile(bundle, 'w') as z:
                        for name, content in files.items():
                            z.writestr(name, content)
                else:
                    with tarfile.open(bundle, 'w:gz') as tar:
                        for name, content in files.items():
                            info = tarfile.TarInfo(name)
                            info.size = len(content)
                            tar.addfile(info, io.BytesIO(content))
            return audit.audit(bundle, target, policy or POLICY)

    def assertPass(self, report):
        self.assertEqual(report['errors'], [])
        self.assertEqual(report['static_audit_status'], 'pass')
        self.assertIsNone(report['validated_minimum_os'])
        self.assertEqual(report['runtime_validation'], 'not-performed')

    def assertError(self, report, message):
        self.assertEqual(report['static_audit_status'], 'fail')
        self.assertTrue(any(message in error for error in report['errors']), report['errors'])

    def test_manifest_six_goals_and_unverified_minimums(self):
        self.assertEqual(len(POLICY['targets']), 6)
        self.assertTrue(all(t['validated_minimum_os'] is None for t in POLICY['targets'].values()))
        self.assertEqual(POLICY['targets']['aarch64-pc-windows-msvc']['desired_minimum_os'], 'Windows 10 22H2')
        self.assertTrue(all('@sha256:' in t['build_image'] for t in POLICY['targets'].values() if 'build_image' in t))

    def test_linux_pass_and_report_every_external_dependency(self):
        report = self.run_audit({'codestyles': elf(), 'resources/dprint': elf(), 'plugin.wasm': b'\0asm'})
        self.assertPass(report)
        self.assertEqual(report['native_binary_count'], 2)
        self.assertEqual(report['binaries'][0]['dependencies'], [{'name': 'libc.so.6', 'status': 'external-system'}])
        self.assertEqual(report['binaries'][0]['maximum_required_symbol_versions'], {'GLIBC': '2.28'})

    def test_zip_and_tar_have_identical_native_inventory(self):
        files = {'codestyles': elf(), 'resources/dprint': elf()}
        direct = self.run_audit(files)
        for kind in ('zip', 'tar'):
            report = self.run_audit(files, archive=kind)
            self.assertPass(report)
            self.assertEqual(report['binaries'], direct['binaries'])

    def test_all_elfs_including_extensionless_sidecars_are_audited(self):
        for path in ('resources/dprint', 'lib/libhidden.so', 'assets/unusual-name'):
            with self.subTest(path=path):
                report = self.run_audit({'codestyles': elf(), path: elf(versions=('GLIBC_2.29',))})
                self.assertError(report, f'{path}: requires GLIBC_2.29')

    def test_glibcxx_cxxabi_and_non_numeric_versions_fail(self):
        for version in ('GLIBCXX_3.4.26', 'CXXABI_1.3.12', 'GLIBC_PRIVATE', 'GLIBC_ABI_DT_RELR'):
            with self.subTest(version=version):
                self.assertError(self.run_audit({'codestyles': elf(versions=(version,))}), version)
        self.assertPass(self.run_audit({'codestyles': elf(versions=('GLIBCXX_3.4.25', 'CXXABI_1.3.11'))}))

    def test_version_strings_outside_requirement_table_do_not_count(self):
        self.assertPass(self.run_audit({'codestyles': elf(extra=b'GLIBC_9.99\0GLIBCXX_99.0\0')}))

    def test_elf_wrong_architecture_and_interpreter_fail(self):
        self.assertError(self.run_audit({'codestyles': elf(machine=183)}), 'unexpected architecture')
        self.assertError(self.run_audit({'codestyles': elf(interpreter='/lib64/custom-loader')}), 'unexpected ELF interpreter')

    def test_elf_external_libraries_and_search_paths_fail_closed(self):
        self.assertError(self.run_audit({'codestyles': elf(needed=('libprivate.so',))}), 'libprivate.so')
        for path in ('/home/builder/lib', '', '$ORIGIN/../../outside', '$LIB'):
            self.assertError(self.run_audit({'codestyles': elf(rpath=path)}), 'search path')
        report = self.run_audit({'bin/codestyles': elf(needed=('libprivate.so',), runpath='$ORIGIN/../lib'),
                                 'lib/libprivate.so': elf()})
        self.assertPass(report)
        self.assertEqual(report['binaries'][0]['dependencies'][0]['resolved_path'], 'lib/libprivate.so')

    def test_bundle_root_origin_and_loader_paths_resolve(self):
        self.assertPass(self.run_audit({'codestyles': elf(needed=('liblocal.so',), rpath='$ORIGIN/lib'),
                                       'lib/liblocal.so': elf()}))
        self.assertPass(self.run_audit({'codestyles': mach(dylibs=('@loader_path/lib/local.dylib',)),
                                       'lib/local.dylib': mach(filetype=6)}, MAC))

    def test_elf_runpath_precedes_rpath(self):
        report = self.run_audit({'codestyles': elf(needed=('libprivate.so',), rpath='$ORIGIN/lib', runpath='$ORIGIN/missing'),
                                 'lib/libprivate.so': elf()})
        self.assertError(report, 'libprivate.so')

    def test_macos_old_and_new_minimum_commands(self):
        for old in (False, True):
            self.assertPass(self.run_audit({'CodeStyles.app/Contents/MacOS/codestyles': mach(old_minimum=old)}, MAC))
        self.assertPass(self.run_audit({'codestyles': mach(cpu=0x100000c, minimum=(11, 0, 0))}, 'aarch64-apple-darwin'))

    def test_macos_high_missing_or_non_mac_minimum_fails(self):
        for content in (mach(minimum=(10, 16, 0)), mach(minimum=None), mach(platform=2)):
            self.assertEqual(self.run_audit({'codestyles': content}, MAC)['static_audit_status'], 'fail')
        self.assertError(self.run_audit({'codestyles': mach(), 'resources/dprint': mach(minimum=(15, 0, 0))}, MAC), 'resources/dprint')

    def test_all_fat_macho_slices_checked(self):
        content = fat([(0x1000007, mach()), (0x100000c, mach(cpu=0x100000c, minimum=(15, 0, 0)))])
        report = self.run_audit({'codestyles': content}, MAC)
        self.assertEqual(len(report['binaries'][0]['slices']), 2)
        self.assertError(report, 'unexpected architecture aarch64')
        self.assertError(report, 'requires macos 15.0.0')

    def test_macos_local_rpath_dependency_and_external_failure(self):
        report = self.run_audit({'CodeStyles.app/Contents/MacOS/codestyles': mach(dylibs=('@rpath/libcustom.dylib',), rpaths=('@executable_path/../Frameworks',)),
                                 'CodeStyles.app/Contents/Frameworks/libcustom.dylib': mach(filetype=6)}, MAC)
        self.assertPass(report)
        for dep in ('/opt/homebrew/lib/libcustom.dylib', '@rpath/missing.dylib', '/usr/lib/../../private.dylib'):
            self.assertError(self.run_audit({'codestyles': mach(dylibs=(dep,))}, MAC), 'unresolved')

    def test_pe_imports_delay_imports_and_crt_reporting(self):
        report = self.run_audit({'codestyles.exe': pe(), 'resources/dprint.exe': pe(subsystem=3)}, WIN)
        self.assertPass(report)
        self.assertEqual(report['binaries'][0]['crt_imports'], ['ucrtbase.dll'])
        self.assertEqual(report['binaries'][0]['delay_imports'], ['ucrtbase.dll'])
        self.assertEqual(report['binaries'][1]['subsystem'], 3)

    def test_documented_windows_system_imports_are_exact_and_sourced(self):
        names = ('pdh.dll', 'icuuc.dll', 'uiautomationcore.dll', 'gdiplus.dll',
                 'mpr.dll', 'windowscodecs.dll', 'shcore.dll')
        for target, machine in [(WIN, 0x8664), ('aarch64-pc-windows-msvc', 0xaa64)]:
            with self.subTest(target=target):
                report = self.run_audit({'codestyles.exe': pe(machine=machine, imports=names, delay=()),
                                         'resources/dprint.exe': pe(machine=machine, subsystem=3,
                                                                    imports=('pdh.dll',), delay=())}, target)
                self.assertPass(report)
                for record in report['binaries']:
                    self.assertTrue(all(dep['status'] == 'external-system' for dep in record['dependencies']))
        for name in names:
            evidence = POLICY['windows']['external_library_evidence'][name]
            self.assertTrue(evidence['source'].startswith('https://learn.microsoft.com/'))
            self.assertTrue(evidence['availability'])
        self.assertIn('1703', POLICY['windows']['external_library_evidence']['icuuc.dll']['availability'])
        self.assertTrue(set(POLICY['windows']['redistributable_libraries']).isdisjoint(
            POLICY['windows']['external_libraries']))
        for name in ('pdh-private.dll', 'icuuc_custom.dll', 'uiautomationcore_extra.dll', 'gdiplus-extension.dll',
                     'mpr-custom.dll', 'windowscodecs-extra.dll', 'shcore_custom.dll'):
            self.assertError(self.run_audit({'codestyles.exe': pe(imports=(name,), delay=())}, WIN), 'unresolved')

    def test_pe_main_console_wrong_arch_dll_or_unknown_import_fails(self):
        for content, expected in [(pe(subsystem=3), 'GUI subsystem'), (pe(machine=0xaa64), 'architecture'),
                                  (pe(dll=True), 'marked as a DLL'), (pe(delay=('custom.dll',)), 'custom.dll')]:
            self.assertError(self.run_audit({'codestyles.exe': content}, WIN), expected)

    def test_windows_header_versions_above_goal_fail_for_all_binaries(self):
        for offset, field in [(48, 'subsystem_version'), (40, 'os_header_version')]:
            data = bytearray(pe(subsystem=3))
            struct.pack_into('<HH', data, 0x98 + offset, 11, 0)
            self.assertError(self.run_audit({'codestyles.exe': pe(), 'resources/dprint.exe': bytes(data)}, WIN), field)

    def test_dprint_must_be_an_executable(self):
        self.assertError(self.run_audit({'codestyles.exe': pe(), 'dprint.exe': pe(dll=True)}, WIN), 'marked as a DLL')
        self.assertError(self.run_audit({'codestyles': mach(), 'dprint': mach(filetype=6)}, MAC), 'must be MH_EXECUTE')

    def test_pe_bundled_dll_is_audited_and_case_insensitive(self):
        report = self.run_audit({'codestyles.exe': pe(imports=('CUSTOM.dll',), delay=()),
                                 'custom.DLL': pe(dll=True)}, WIN)
        self.assertPass(report)
        self.assertEqual(report['binaries'][0]['dependencies'][0]['status'], 'bundled')
        self.assertError(self.run_audit({'codestyles.exe': pe(), 'custom.dll': pe(machine=0xaa64, dll=True)}, WIN), 'architecture')

    def test_vc_runtime_must_be_bundled_and_matching_architecture(self):
        main = pe(delay=('VCRUNTIME140.dll',))
        self.assertError(self.run_audit({'codestyles.exe': main}, WIN), 'missing bundled Visual C++ runtime')
        self.assertPass(self.run_audit({'codestyles.exe': main, 'VCRUNTIME140.dll': pe(dll=True, delay=())}, WIN))
        self.assertError(self.run_audit({'codestyles.exe': main, 'VCRUNTIME140.dll': pe(machine=0xaa64, dll=True, delay=())}, WIN), 'architecture')

    def test_pe_dprint_cannot_borrow_main_executable_dll_directory(self):
        files = {'codestyles.exe': pe(delay=()),
                 'resources/bin/dprint.exe': pe(subsystem=3, delay=('VCRUNTIME140.dll',)),
                 'VCRUNTIME140.dll': pe(dll=True, delay=())}
        self.assertError(self.run_audit(files, WIN), 'resources/bin/dprint.exe: missing bundled Visual C++ runtime')
        files['resources/bin/VCRUNTIME140.dll'] = files.pop('VCRUNTIME140.dll')
        self.assertPass(self.run_audit(files, WIN))

    def test_macho_dprint_has_own_executable_path_and_inherited_rpaths(self):
        files = {'CodeStyles.app/Contents/MacOS/codestyles': mach(),
                 'CodeStyles.app/Contents/Resources/dprint': mach(dylibs=('@executable_path/libcustom.dylib',)),
                 'CodeStyles.app/Contents/MacOS/libcustom.dylib': mach(filetype=6)}
        self.assertError(self.run_audit(files, MAC), 'unresolved external dependency @executable_path/libcustom.dylib')
        files['CodeStyles.app/Contents/Resources/libcustom.dylib'] = files.pop('CodeStyles.app/Contents/MacOS/libcustom.dylib')
        self.assertPass(self.run_audit(files, MAC))
        files['CodeStyles.app/Contents/Resources/dprint'] = mach(dylibs=('@rpath/libcustom.dylib',), rpaths=('@executable_path',))
        files['CodeStyles.app/Contents/Resources/libcustom.dylib'] = mach(filetype=6, dylibs=('@rpath/libsecond.dylib',))
        files['CodeStyles.app/Contents/Resources/libsecond.dylib'] = mach(filetype=6)
        self.assertPass(self.run_audit(files, MAC))

    def test_dependency_must_be_shared_library_not_executable(self):
        self.assertError(self.run_audit({'codestyles.exe': pe(delay=('VCRUNTIME140.dll',)),
                                        'VCRUNTIME140.dll': pe(dll=False, delay=())}, WIN), 'not a DLL')
        self.assertError(self.run_audit({'codestyles': mach(dylibs=('@loader_path/local.dylib',)),
                                        'local.dylib': mach(filetype=2)}, MAC), 'not a matching-architecture dylib')
        executable = bytearray(elf())
        struct.pack_into('<H', executable, 16, 2)
        self.assertError(self.run_audit({'codestyles': elf(needed=('local.so',), rpath='$ORIGIN'),
                                        'local.so': bytes(executable)}), 'not a loadable shared object')

    def test_pe_exact_uninstaller_exception_is_recorded_not_skipped(self):
        helper = pe(machine=0x14c, is64=False)
        report = self.run_audit({'codestyles.exe': pe(), 'unins000.exe': helper}, WIN)
        self.assertPass(report)
        exception = report['binaries'][1]['installer_helper_exception']
        self.assertTrue(exception['emulation_required'])
        self.assertEqual(report['binaries'][1]['architecture'], 'x86')
        self.assertError(self.run_audit({'codestyles.exe': pe(), 'tools/unins000.exe': helper}, WIN), 'architecture')
        self.assertError(self.run_audit({'codestyles.exe': pe(), 'unins000.exe': pe(machine=0x14c, is64=False, subsystem=3)}, WIN), 'helper has unexpected subsystem')

    def test_installed_uninstaller_system_imports_remain_audited(self):
        helper = pe(machine=0x14c, is64=False,
                    imports=('mpr.dll', 'windowscodecs.dll', 'Shcore.dll'), delay=())
        for target, machine in [(WIN, 0x8664), ('aarch64-pc-windows-msvc', 0xaa64)]:
            with self.subTest(target=target):
                report = self.run_audit({'codestyles.exe': pe(machine=machine), 'unins000.exe': helper}, target)
                self.assertPass(report)
                record = next(item for item in report['binaries'] if item['path'] == 'unins000.exe')
                self.assertTrue(record['installer_helper_exception']['emulation_required'])
                self.assertEqual({dep['name'] for dep in record['dependencies']},
                                 {'mpr.dll', 'windowscodecs.dll', 'Shcore.dll'})
                self.assertTrue(all(dep['status'] == 'external-system' for dep in record['dependencies']))
                self.assertTrue(all(dep['entrypoint'] == 'unins000.exe' for dep in record['dependencies']))

    def test_uninstaller_cannot_load_mismatched_architecture_dll(self):
        self.assertError(self.run_audit({'codestyles.exe': pe(),
                                        'unins000.exe': pe(machine=0x14c, is64=False, delay=('custom.dll',)),
                                        'custom.dll': pe(dll=True)}, WIN), 'does not match importer')

    def test_duplicate_macho_architecture_cannot_hide_a_slice(self):
        content = fat([(0x1000007, mach(dylibs=('/private/hidden.dylib',))), (0x1000007, mach())])
        self.assertError(self.run_audit({'codestyles': content}, MAC), 'multiple Mach-O fat slices')

    def test_wrong_format_truncated_and_disguised_native_files_fail(self):
        self.assertError(self.run_audit({'codestyles': elf(), 'foreign': pe()}), 'expected ELF')
        self.assertError(self.run_audit({'codestyles': elf(), 'foreign': mach()}), 'expected ELF')
        self.assertError(self.run_audit({'codestyles.exe': pe(), 'foreign': elf()}, WIN), 'expected PE')
        for content in (b'\x7fELF', b'MZ', b'\xcf\xfa\xed\xfe'):
            self.assertError(self.run_audit({'codestyles': content}), 'malformed binary')
        self.assertError(self.run_audit({'codestyles': elf(), 'lib/broken.so': b'not a binary'}), 'unknown file format')

    def test_missing_and_ambiguous_main_fail(self):
        self.assertError(self.run_audit({'dprint': elf()}), 'exactly one main executable')
        self.assertError(self.run_audit({'a/codestyles': elf(), 'b/codestyles': elf()}), 'exactly one main executable')

    def test_elf_string_table_can_cross_contiguous_load_segments(self):
        data = bytearray(elf(interpreter=None))
        # Split the original LOAD through the dynamic string table, as found
        # in real post-link-optimized Python builds. File/virtual mapping stays
        # contiguous even though neither segment contains the whole table.
        struct.pack_into('<H', data, 56, 3)
        struct.pack_into('<QQ', data, 64 + 32, 0x410, 0x410)
        struct.pack_into('<IIQQQQQQ', data, 176, 1, 4, 0x410, 0x400410, 0x400410,
                         len(data) - 0x410, len(data) - 0x410, 1)
        self.assertEqual(audit.parse_elf(bytes(data))['needed'], ['libc.so.6'])
        # A virtual gap must still fail rather than reading unrelated bytes.
        struct.pack_into('<Q', data, 176 + 16, 0x400420)
        with self.assertRaisesRegex(audit.AuditError, 'not file-backed'):
            audit.parse_elf(bytes(data))

    def test_malformed_elf_version_chain_fails(self):
        data = bytearray(elf(versions=('GLIBC_2.28', 'GLIBC_2.29')))
        struct.pack_into('<I', data, 0x810 + 12, 0)
        self.assertError(self.run_audit({'codestyles': bytes(data)}), 'short ELF version auxiliary chain')

    def test_malformed_macho_command_and_fat_slice_fail(self):
        data = bytearray(mach())
        struct.pack_into('<I', data, 36, 4)
        self.assertError(self.run_audit({'codestyles': bytes(data)}, MAC), 'invalid Mach-O load command size')
        data = bytearray(fat([(0x1000007, mach())]))
        struct.pack_into('>I', data, 16, 8)
        self.assertError(self.run_audit({'codestyles': bytes(data)}, MAC), 'overlapping Mach-O fat slices')

    def test_malformed_pe_import_table_fails(self):
        data = bytearray(pe())
        struct.pack_into('<I', data, 0x98 + 112 + 8 + 4, 20)
        self.assertError(self.run_audit({'codestyles.exe': bytes(data)}, WIN), 'lacks null terminator')

    def test_archive_path_traversal_duplicates_and_symlink_escapes_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            bundle = Path(tmp) / 'bad.zip'
            with zipfile.ZipFile(bundle, 'w') as z:
                z.writestr('../escape', elf())
            with self.assertRaisesRegex(audit.AuditError, 'unsafe bundle member'):
                audit.read_bundle(bundle)
            with tarfile.open(Path(tmp) / 'bad.tar', 'w') as tar:
                info = tarfile.TarInfo('link')
                info.type, info.linkname = tarfile.SYMTYPE, '/etc/passwd'
                tar.addfile(info)
            with self.assertRaisesRegex(audit.AuditError, 'absolute archive link'):
                audit.read_bundle(Path(tmp) / 'bad.tar')

    def test_tar_internal_symbolic_and_hard_links_audited(self):
        with tempfile.TemporaryDirectory() as tmp:
            bundle = Path(tmp) / 'valid.tar.gz'
            data = elf()
            with tarfile.open(bundle, 'w:gz') as tar:
                info = tarfile.TarInfo('codestyles')
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
                for name, kind in [('linked', tarfile.SYMTYPE), ('hard', tarfile.LNKTYPE)]:
                    info = tarfile.TarInfo(name)
                    info.type, info.linkname = kind, 'codestyles'
                    tar.addfile(info)
            report = audit.audit(bundle, LINUX, POLICY)
            self.assertPass(report)
            self.assertEqual(report['native_binary_count'], 3)

    def test_cli_writes_failed_report_and_nonzero_exit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, report = Path(tmp) / 'bundle', Path(tmp) / 'report.json'
            root.mkdir()
            (root / 'codestyles').write_bytes(elf(versions=('GLIBC_99.0',)))
            run = subprocess.run([sys.executable, str(Path(audit.__file__)), '--target', LINUX,
                                  '--bundle', str(root), '--report', str(report)], capture_output=True, text=True)
            self.assertEqual(run.returncode, 1)
            self.assertEqual(json.loads(report.read_text())['static_audit_status'], 'fail')

    @unittest.skipUnless(sys.platform.startswith('linux') and shutil.which('readelf'), 'requires native Linux readelf')
    def test_real_native_elf_matches_readelf_dynamic_dependencies(self):
        # Python/coreutils paths can be scripts or launcher shims in build
        # images. Resolve /proc/self/exe in this process before invoking readelf:
        # passing that proc path to readelf would inspect readelf itself.
        candidates = [Path('/proc/self/exe'), Path(sys.executable),
                      Path('/bin/true'), Path('/bin/ls'), Path(shutil.which('readelf'))]
        path = None
        for candidate in candidates:
            try:
                resolved = candidate.resolve(strict=True)
                with resolved.open('rb') as source:
                    magic = source.read(4)
                if magic == b'\x7fELF':
                    path = resolved
                    break
            except OSError:
                continue
        self.assertIsNotNone(path, 'Linux test environment has no readable real ELF candidate')
        record = audit.parse_elf(path.read_bytes())
        output = subprocess.check_output(['readelf', '-d', str(path)], text=True)
        import re
        self.assertEqual(record['needed'], re.findall(r'\(NEEDED\).*?\[(.*?)\]', output))
        versions = subprocess.check_output(['readelf', '--version-info', str(path)], text=True)
        for version in record['required_symbol_versions']:
            self.assertIn(version['name'], versions)


if __name__ == '__main__':
    unittest.main()
