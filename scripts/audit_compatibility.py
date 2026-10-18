#!/usr/bin/env python3
"""Fail-closed, non-executing binary-policy audit of a complete distribution.

Accepts directories, ZIPs and tar archives. Parses ELF, Mach-O (including fat
slices) and PE headers itself; no target executable is run and no host library
resolution is mistaken for target compatibility. A pass is static evidence,
never a claim that the desired minimum OS or graphics stack was tested.
"""
import argparse
import hashlib
import json
import posixpath
import re
import stat
import struct
import sys
import tarfile
import zipfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = ROOT / 'resources' / 'compatibility.json'
MACH_MAGICS = {b'\xce\xfa\xed\xfe': ('<', False), b'\xcf\xfa\xed\xfe': ('<', True),
               b'\xfe\xed\xfa\xce': ('>', False), b'\xfe\xed\xfa\xcf': ('>', True)}
FAT_MAGICS = {b'\xca\xfe\xba\xbe': ('>', False), b'\xbe\xba\xfe\xca': ('<', False),
              b'\xca\xfe\xba\xbf': ('>', True), b'\xbf\xba\xfe\xca': ('<', True)}


class AuditError(ValueError):
    """Malformed binary/archive, or an unsupported structure that cannot pass."""


class Reader:
    def __init__(self, data, endian='<'):
        self.data, self.endian = data, endian

    def bytes(self, offset, size):
        if offset < 0 or size < 0 or offset + size > len(self.data):
            raise AuditError(f'truncated data at offset {offset}, length {size}')
        return self.data[offset:offset + size]

    def unpack(self, fmt, offset):
        return struct.unpack(self.endian + fmt, self.bytes(offset, struct.calcsize(self.endian + fmt)))

    def string(self, offset, end=None):
        end = len(self.data) if end is None else end
        if not 0 <= offset < end <= len(self.data):
            raise AuditError('string offset outside its table')
        zero = self.data.find(b'\0', offset, end)
        if zero < 0:
            raise AuditError('unterminated binary string')
        try:
            return self.data[offset:zero].decode('utf-8')
        except UnicodeDecodeError as exc:
            raise AuditError('non-UTF-8 binary dependency name') from exc


def parse_elf(data):
    r = Reader(data)
    ident = r.bytes(0, 16)
    if ident[4] not in (1, 2) or ident[5] not in (1, 2) or ident[6] != 1:
        raise AuditError('invalid ELF identification')
    r.endian = '<' if ident[5] == 1 else '>'
    bits = 64 if ident[4] == 2 else 32
    fmt = 'HHIQQQIHHHHHH' if bits == 64 else 'HHIIIIIHHHHHH'
    hdr = r.unpack(fmt, 16)
    machine, phoff, phentsize, phnum = hdr[1], hdr[4], hdr[8], hdr[9]
    if phnum == 0xffff:
        raise AuditError('extended ELF program-header counts are unsupported')
    pfmt = 'IIQQQQQQ' if bits == 64 else 'IIIIIIII'
    if phnum and phentsize < struct.calcsize(r.endian + pfmt):
        raise AuditError('invalid ELF program-header size')
    segments, dynamic, interpreter = [], None, None
    for i in range(phnum):
        ph = r.unpack(pfmt, phoff + phentsize * i)
        kind, off, addr, size = (ph[0], ph[2], ph[3], ph[5]) if bits == 64 else (ph[0], ph[1], ph[2], ph[4])
        r.bytes(off, size)
        if kind == 1:
            segments.append((addr, size, off))
        elif kind == 2:
            if dynamic is not None:
                raise AuditError('multiple ELF dynamic segments')
            dynamic = (off, size)
        elif kind == 3:
            interpreter = r.string(off, off + size)

    def mapped(addr, length=1):
        for base, size, off in segments:
            if base <= addr and addr + length <= base + size:
                return off + addr - base
        raise AuditError(f'ELF virtual address is not file-backed: {addr:#x}')

    tags = {}
    if dynamic:
        fmt, step = ('qQ', 16) if bits == 64 else ('iI', 8)
        terminated = False
        for off in range(dynamic[0], dynamic[0] + dynamic[1] - step + 1, step):
            tag, value = r.unpack(fmt, off)
            if tag == 0:
                terminated = True
                break
            tags.setdefault(tag, []).append(value)
        if not terminated:
            raise AuditError('ELF dynamic table lacks DT_NULL')
    strings = None
    if tags:
        if len(tags.get(5, [])) != 1 or len(tags.get(10, [])) != 1:
            raise AuditError('ELF dynamic string table is missing or ambiguous')
        strings = (mapped(tags[5][0], tags[10][0]), tags[10][0])

    def dynstr(index):
        if not strings or index >= strings[1]:
            raise AuditError('ELF string index outside DT_STRTAB')
        return r.string(strings[0] + index, strings[0] + strings[1])

    versions = []
    if 0x6ffffffe in tags or 0x6fffffff in tags:
        if len(tags.get(0x6ffffffe, [])) != 1 or len(tags.get(0x6fffffff, [])) != 1:
            raise AuditError('ELF version requirements table is missing or ambiguous')
        address, count = tags[0x6ffffffe][0], tags[0x6fffffff][0]
        if count > len(data) // 16:
            raise AuditError('invalid ELF version requirement count')
        visited = set()
        for i in range(count):
            if address in visited:
                raise AuditError('cyclic ELF version requirements')
            visited.add(address)
            version, aux_count, filename, aux, nxt = r.unpack('HHIII', mapped(address, 16))
            if version != 1 or aux_count > len(data) // 16:
                raise AuditError('unsupported ELF version requirement structure')
            library = dynstr(filename)
            aux_address, aux_seen = address + aux, set()
            for j in range(aux_count):
                if aux_address in aux_seen:
                    raise AuditError('cyclic ELF version auxiliaries')
                aux_seen.add(aux_address)
                _, flags, _, name, next_aux = r.unpack('IHHII', mapped(aux_address, 16))
                versions.append({'library': library, 'name': dynstr(name), 'weak': bool(flags & 2)})
                if j + 1 < aux_count and not next_aux:
                    raise AuditError('short ELF version auxiliary chain')
                aux_address += next_aux
            if i + 1 < count and not nxt:
                raise AuditError('short ELF version requirement chain')
            address += nxt
    return {'format': 'ELF', 'architecture': {62: 'x86_64', 183: 'aarch64'}.get(machine, f'elf-machine-{machine}'),
            'bits': bits, 'endian': r.endian, 'elf_type': hdr[0], 'interpreter': interpreter,
            'needed': [dynstr(v) for v in tags.get(1, [])],
            'rpath': [p for v in tags.get(15, []) for p in dynstr(v).split(':')],
            'runpath': [p for v in tags.get(29, []) for p in dynstr(v).split(':')],
            'soname': [dynstr(v) for v in tags.get(14, [])],
            'required_symbol_versions': versions}


def packed_version(value):
    return f'{value >> 16}.{(value >> 8) & 255}.{value & 255}'


def parse_mach_slice(data):
    endian, is64 = MACH_MAGICS[data[:4]]
    r = Reader(data, endian)
    hdr = r.unpack('IIIIIIII' if is64 else 'IIIIIII', 0)
    cpu, filetype, ncmds, cmdbytes = hdr[1], hdr[3], hdr[4], hdr[5]
    offset = 32 if is64 else 28
    limit = offset + cmdbytes
    r.bytes(offset, cmdbytes)
    minimums, dylibs, rpaths, ids = [], [], [], []
    if ncmds > cmdbytes // 8:
        raise AuditError('Mach-O load command count exceeds table')
    for _ in range(ncmds):
        cmd, size = r.unpack('II', offset)
        if size < 8 or offset + size > limit or size % 4:
            raise AuditError('invalid Mach-O load command size')
        if cmd == 0x24:
            if size < 16:
                raise AuditError('truncated LC_VERSION_MIN_MACOSX')
            minimums.append({'platform': 'macos', 'version': packed_version(r.unpack('I', offset + 8)[0])})
        elif cmd == 0x32:
            if size < 24:
                raise AuditError('truncated LC_BUILD_VERSION')
            platform, minimum, _, ntools = r.unpack('IIII', offset + 8)
            if 24 + ntools * 8 > size:
                raise AuditError('truncated LC_BUILD_VERSION tool table')
            minimums.append({'platform': 'macos' if platform == 1 else f'apple-platform-{platform}',
                             'version': packed_version(minimum)})
        elif cmd in (0xc, 0xd, 0x80000018, 0x8000001f, 0x20, 0x80000023, 0x8000001c):
            header_size = 12 if cmd == 0x8000001c else 24
            if size < header_size:
                raise AuditError('truncated Mach-O path command')
            string_offset = r.unpack('I', offset + 8)[0]
            if string_offset < header_size:
                raise AuditError('invalid Mach-O path offset')
            name = r.string(offset + string_offset, offset + size)
            if cmd == 0x8000001c:
                rpaths.append(name)
            elif cmd == 0xd:
                ids.append(name)
            else:
                dylibs.append({'name': name, 'weak': cmd == 0x80000018})
        offset += size
    if offset != limit:
        raise AuditError('Mach-O load command table length mismatch')
    return {'architecture': {0x1000007: 'x86_64', 0x100000c: 'aarch64'}.get(cpu, f'mach-cpu-{cpu}'),
            'bits': 64 if is64 else 32, 'endian': endian, 'file_type': filetype,
            'minimum_os': minimums, 'dylibs': dylibs, 'rpath': rpaths, 'install_names': ids}


def parse_mach(data):
    if data[:4] in MACH_MAGICS:
        return {'format': 'Mach-O', 'slices': [parse_mach_slice(data)]}
    endian, is64 = FAT_MAGICS[data[:4]]
    r = Reader(data, endian)
    count = r.unpack('I', 4)[0]
    step = 32 if is64 else 20
    if not count or count > (len(data) - 8) // step:
        raise AuditError('invalid Mach-O fat architecture count')
    slices, ranges = [], []
    for i in range(count):
        values = r.unpack('IIQQII' if is64 else 'IIIII', 8 + i * step)
        cpu, off, size = values[0], values[2], values[3]
        if off < 8 + count * step or any(off < end and start < off + size for start, end in ranges):
            raise AuditError('overlapping Mach-O fat slices')
        ranges.append((off, off + size))
        content = r.bytes(off, size)
        if content[:4] not in MACH_MAGICS:
            raise AuditError('non-Mach-O fat slice')
        record = parse_mach_slice(content)
        if record['architecture'] != {0x1000007: 'x86_64', 0x100000c: 'aarch64'}.get(cpu, f'mach-cpu-{cpu}'):
            raise AuditError('fat architecture disagrees with Mach-O slice')
        slices.append(record)
    return {'format': 'Mach-O', 'slices': slices}


def parse_pe(data):
    r = Reader(data)
    peoff = r.unpack('I', 0x3c)[0]
    if r.bytes(peoff, 4) != b'PE\0\0':
        raise AuditError('DOS executable is not PE')
    machine, sections, _, _, _, optsize, characteristics = r.unpack('HHIIIHH', peoff + 4)
    opt = peoff + 24
    magic = r.unpack('H', opt)[0]
    if magic not in (0x10b, 0x20b):
        raise AuditError('unsupported PE optional header')
    is64 = magic == 0x20b
    if (machine == 0x14c and is64) or (machine in (0x8664, 0xaa64) and not is64):
        raise AuditError('PE machine and optional-header architecture disagree')
    dirs_offset = 112 if is64 else 96
    if optsize < dirs_offset:
        raise AuditError('truncated PE optional header')
    r.bytes(opt, optsize)
    dircount = r.unpack('I', opt + dirs_offset - 4)[0]
    if dircount > (optsize - dirs_offset) // 8:
        raise AuditError('truncated PE data directories')
    imagebase = r.unpack('Q' if is64 else 'I', opt + (24 if is64 else 28))[0]
    headersize = r.unpack('I', opt + 60)[0]
    mapped_sections = []
    for i in range(sections):
        start = opt + optsize + i * 40
        r.bytes(start, 40)
        _, addr, rawsize, rawoff = r.unpack('IIII', start + 8)
        r.bytes(rawoff, rawsize)
        mapped_sections.append((addr, rawsize, rawoff))

    def mapped(addr, length=1):
        if addr < headersize and addr + length <= min(headersize, len(data)):
            return addr
        for base, size, offset in mapped_sections:
            if base <= addr and addr + length <= base + size:
                return offset + addr - base
        raise AuditError(f'PE RVA is not file-backed: {addr:#x}')

    def name_at(addr):
        start = mapped(addr)
        # Require the entire name to remain in the same file-backed mapping.
        name = r.string(start)
        mapped(addr, len(name.encode('utf-8')) + 1)
        return name

    def imports(index, delay=False):
        if dircount <= index:
            return []
        address, size = r.unpack('II', opt + dirs_offset + 8 * index)
        if not address and not size:
            return []
        step = 32 if delay else 20
        if not address or size < step:
            raise AuditError('invalid PE import directory')
        values = []
        for delta in range(0, size - step + 1, step):
            entry = r.unpack('IIIIIIII' if delay else 'IIIII', mapped(address + delta, step))
            if not any(entry):
                return values
            name_rva = entry[1] if delay else entry[3]
            if delay:
                if entry[0] & ~1:
                    raise AuditError('unsupported PE delay-import attributes')
                if not entry[0] & 1:
                    name_rva -= imagebase
            values.append(name_at(name_rva))
        raise AuditError('PE import directory lacks null terminator')

    normal, delayed = imports(1), imports(13, True)
    all_names = normal + delayed
    return {'format': 'PE', 'architecture': {0x14c: 'x86', 0x8664: 'x86_64', 0xaa64: 'aarch64'}.get(machine, f'pe-machine-{machine}'),
            'bits': 64 if is64 else 32, 'is_dll': bool(characteristics & 0x2000),
            'subsystem': r.unpack('H', opt + 68)[0],
            'subsystem_version': '.'.join(map(str, r.unpack('HH', opt + 48))),
            'os_header_version': '.'.join(map(str, r.unpack('HH', opt + 40))),
            'imports': normal, 'delay_imports': delayed,
            'crt_imports': sorted({name for name in all_names if re.match(
                r'(?i)(?:api-ms-win-crt-|ucrtbase|vcruntime|msvcp|msvcr|concrt)', name)}),
            'crt_note': 'No CRT import does not prove every library is statically linked or every API exists on the desired OS.'}


def binary_format(data):
    if data[:4] == b'\x7fELF':
        return 'ELF'
    if data[:4] in MACH_MAGICS or data[:4] in FAT_MAGICS:
        return 'Mach-O'
    if data[:2] == b'MZ':
        return 'PE'
    return None


def safe_name(name):
    name = name.replace('\\', '/')
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or not path.parts or ':' in path.parts[0]:
        raise AuditError(f'unsafe bundle member: {name!r}')
    return str(path)


def read_bundle(path):
    """Read without extracting archive paths; reject escaping/ambiguous links."""
    entries, links = {}, {}

    def add(name, data=None, link=None):
        name = safe_name(name)
        if name in entries or name in links:
            raise AuditError(f'duplicate bundle member: {name}')
        if link is not None:
            links[name] = link
        else:
            entries[name] = data

    if path.is_dir():
        root = path.resolve()
        for member in sorted(path.rglob('*')):
            if member.is_symlink():
                # Directory links are deliberately rejected: rglob does not walk them.
                if member.is_dir() or not member.resolve().is_relative_to(root):
                    raise AuditError(f'unsupported or escaping directory symlink: {member}')
            if member.is_file():
                add(member.relative_to(path).as_posix(), member.read_bytes())
            elif not member.is_dir():
                raise AuditError(f'unsupported bundle entry: {member}')
    elif zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as archive:
            for item in archive.infolist():
                if item.is_dir():
                    continue
                content = archive.read(item)
                if stat.S_ISLNK(item.external_attr >> 16):
                    add(item.filename, link=(content.decode('utf-8'), False))
                else:
                    add(item.filename, content)
    elif tarfile.is_tarfile(path):
        with tarfile.open(path) as archive:
            for item in archive:
                if item.isdir():
                    continue
                if item.issym():
                    add(item.name, link=(item.linkname, False))
                elif item.islnk():
                    # Tar hard-link targets are archive-root relative.
                    add(item.name, link=(safe_name(item.linkname), True))
                elif item.isfile():
                    add(item.name, archive.extractfile(item).read())
                else:
                    raise AuditError(f'unsupported archive member: {item.name}')
    else:
        raise AuditError('bundle must be a directory, ZIP, or tar archive')
    for name in links:
        seen, current = set(), name
        while current in links:
            if current in seen:
                raise AuditError(f'cyclic archive symlink: {name}')
            seen.add(current)
            target, hard = links[current]
            if target.startswith('/'):
                raise AuditError(f'absolute archive link unsupported: {name}')
            current = safe_name(target if hard else posixpath.normpath(posixpath.join(posixpath.dirname(current), target)))
        if current not in entries:
            raise AuditError(f'dangling archive symlink: {name}')
        entries[name] = entries[current]
    return entries


def version_tuple(value):
    parts = value.split('.')
    if not parts or any(not part.isdecimal() for part in parts):
        raise AuditError(f'not a numeric version: {value}')
    return tuple(map(int, parts)) + (0,) * max(0, 3 - len(parts))


def audit(bundle, target, policy, main_executable=None):
    if target not in policy['targets']:
        raise AuditError(f'unsupported target: {target}')
    selected = policy['targets'][target]
    expected = selected['format']
    entries = read_bundle(Path(bundle))
    records, errors, warnings = [], [], []
    main_name = main_executable or ('codestyles.exe' if expected == 'PE' else 'codestyles')
    mains = [name for name in entries if name == main_name or (not main_executable and PurePosixPath(name).name == main_name)]
    if len(mains) != 1:
        errors.append(f'expected exactly one main executable {main_name}, found {len(mains)}')
    main = mains[0] if len(mains) == 1 else None
    parsers = {'ELF': parse_elf, 'Mach-O': parse_mach, 'PE': parse_pe}
    for name, content in sorted(entries.items()):
        kind = binary_format(content)
        if not kind:
            if name == main or re.search(r'(?i)(?:\.exe|\.dll|\.dylib|\.so(?:\.\d+)*)$', name) or PurePosixPath(name).name == 'dprint':
                errors.append(f'{name}: expected a native binary, found unknown file format')
            continue
        try:
            record = parsers[kind](content)
        except (AuditError, struct.error, KeyError, OverflowError) as exc:
            errors.append(f'{name}: malformed binary: {exc}')
            continue
        record.update(path=name, sha256=hashlib.sha256(content).hexdigest(), bytes=len(content))
        records.append(record)
        if kind != expected:
            errors.append(f'{name}: expected {expected}, found {kind}')
        slices = record['slices'] if kind == 'Mach-O' else [record]
        for part in slices:
            helper = policy['windows'].get('installer_helpers', {}).get(name) if kind == 'PE' else None
            if helper and part['architecture'] in helper['architectures']:
                record['installer_helper_exception'] = {**helper, 'emulation_required': part['architecture'] != selected['architecture']}
                if record['subsystem'] != helper['subsystem']:
                    errors.append(f'{name}: installer helper has unexpected subsystem {record["subsystem"]}')
            elif part['architecture'] != selected['architecture'] or part['bits'] != 64:
                errors.append(f'{name}: unexpected architecture {part["architecture"]}/{part["bits"]}')
    by_name = {record['path']: record for record in records}
    lower_names = {}
    for name in entries:
        if name.lower() in lower_names and expected == 'PE':
            errors.append(f'case-insensitive package path collision: {name}')
        lower_names[name.lower()] = name

    def dependency(record, name, candidates, system=False, **extra):
        bundled = next((candidate for candidate in candidates if candidate in by_name), None)
        status = 'bundled' if bundled else 'external-system' if system else 'unresolved'
        result = {'name': name, 'status': status, **extra}
        if bundled:
            result['resolved_path'] = bundled
        record.setdefault('dependencies', []).append(result)
        if status == 'unresolved':
            errors.append(f'{record["path"]}: undeclared or unresolved external dependency {name}')

    for record in records:
        name = record['path']
        if record['format'] != expected:
            continue
        parent = posixpath.dirname(name)
        if record['format'] == 'ELF':
            config = policy['linux']
            if name == main and record['elf_type'] not in (2, 3):
                errors.append(f'{name}: main ELF must be an executable or PIE')
            if record['endian'] != '<':
                errors.append(f'{name}: expected little-endian ELF')
            interpreter = record['interpreter']
            if interpreter and interpreter not in selected['allowed_interpreters']:
                errors.append(f'{name}: unexpected ELF interpreter {interpreter}')
            maxima = {}
            for required in record['required_symbol_versions']:
                value = required['name']
                namespace = value.split('_', 1)[0]
                if namespace not in config['symbol_version_limits']:
                    continue
                suffix = value[len(namespace) + 1:]
                try:
                    numbers = version_tuple(suffix)
                except AuditError:
                    errors.append(f'{name}: unsupported required symbol version {value}')
                    continue
                if namespace not in maxima or numbers > version_tuple(maxima[namespace]):
                    maxima[namespace] = suffix
                if numbers > version_tuple(config['symbol_version_limits'][namespace]):
                    errors.append(f'{name}: requires {value}, exceeds {namespace}_{config["symbol_version_limits"][namespace]}')
            record['maximum_required_symbol_versions'] = maxima
            search_paths = []
            for path in record['rpath'] + record['runpath']:
                if path == '$ORIGIN' or path.startswith('$ORIGIN/') or path == '${ORIGIN}' or path.startswith('${ORIGIN}/'):
                    resolved = path.replace('${ORIGIN}', parent or '.').replace('$ORIGIN', parent or '.')
                    resolved = posixpath.normpath(resolved)
                    if resolved == '..' or resolved.startswith('../') or resolved.startswith('/') or '$' in resolved:
                        errors.append(f'{name}: unsafe ELF search path {path}')
                    else:
                        if path in (record['runpath'] or record['rpath']):
                            search_paths.append(resolved)
                else:
                    errors.append(f'{name}: non-relocatable ELF search path {path!r}')
            for needed in record['needed']:
                candidates = [posixpath.normpath(posixpath.join(path, needed)) for path in search_paths]
                dependency(record, needed, candidates, needed in config['external_libraries'])
            record['external_runtime_prerequisites'] = config['runtime_prerequisites']
        elif record['format'] == 'Mach-O':
            main_parent = posixpath.dirname(main or '')
            def expand(path):
                return posixpath.normpath(path.replace('@loader_path', parent or '.').replace('@executable_path', main_parent or '.'))
            for part in record['slices']:
                if name == main and part['file_type'] != 2:
                    errors.append(f'{name}: main Mach-O must be MH_EXECUTE')
                if not part['minimum_os']:
                    errors.append(f'{name}: Mach-O slice has no minimum-OS load command')
                for minimum in part['minimum_os']:
                    if minimum['platform'] != 'macos' or version_tuple(minimum['version']) > version_tuple(selected['desired_minimum_os']):
                        errors.append(f'{name}: {part["architecture"]} requires {minimum["platform"]} {minimum["version"]}, target ceiling {selected["desired_minimum_os"]}')
                rpaths = list(part['rpath'])
                if main in by_name and by_name[main]['format'] == 'Mach-O' and main != name:
                    for main_slice in by_name[main]['slices']:
                        rpaths.extend(p.replace('@loader_path', '@executable_path') for p in main_slice['rpath'])
                for path in part['rpath']:
                    resolved = expand(path)
                    if not path.startswith(('@loader_path', '@executable_path')) or resolved.startswith(('../', '/')) or resolved == '..':
                        errors.append(f'{name}: non-relocatable Mach-O rpath {path}')
                for dylib in part['dylibs']:
                    dep = dylib['name']
                    system = dep.startswith(tuple(policy['macos']['system_dylib_prefixes'])) and posixpath.normpath(dep) == dep
                    candidates = []
                    if dep.startswith(('@loader_path/', '@executable_path/')):
                        candidates = [expand(dep)]
                    elif dep.startswith('@rpath/'):
                        candidates = [posixpath.normpath(posixpath.join(expand(p), dep[len('@rpath/'):])) for p in rpaths]
                    dependency(record, dep, candidates, system, architecture=part['architecture'], weak=dylib['weak'])
        else:
            config = policy['windows']
            if name == main and record['is_dll']:
                errors.append(f'{name}: main executable is marked as a DLL')
            if name == main and record['subsystem'] != config['main_subsystem']:
                errors.append(f'{name}: main executable must use Windows GUI subsystem (2), found {record["subsystem"]}')
            for imported in record['imports'] + record['delay_imports']:
                dep = imported.lower()
                candidates = [lower_names.get(posixpath.join(parent, dep).lower()),
                              lower_names.get(posixpath.join(posixpath.dirname(main or ''), dep).lower())]
                system = ('/' not in dep and '\\' not in dep) and (dep in config['external_libraries'] or dep.startswith(tuple(config['system_import_prefixes'])))
                dependency(record, imported, candidates, system)
                if record['dependencies'][-1]['status'] == 'unresolved' and re.match(r'(?:vcruntime|msvcp|msvcr(?!t)|concrt)', dep):
                    errors.append(f'{name}: missing bundled Visual C++ runtime DLL {imported}; a clean Windows installation does not guarantee the Redistributable')
            record['external_crt_requirements'] = [dep['name'] for dep in record.get('dependencies', [])
                                                   if dep['status'] != 'bundled' and dep['name'] in record['crt_imports']]
    if not records:
        errors.append('no native binaries found')
    if main and main not in by_name:
        errors.append(f'{main}: main executable could not be audited')
    warnings.extend(policy['audit_limitations'])
    return {'schema_version': 1, 'target': target, 'bundle': str(bundle), 'main_executable': main,
            'policy_sha256': hashlib.sha256(json.dumps(policy, sort_keys=True).encode()).hexdigest(),
            'desired_minimum_os': selected['desired_minimum_os'],
            'validated_minimum_os': None, 'runtime_validation': 'not-performed',
            'static_audit_status': 'pass' if not errors else 'fail',
            'native_binary_count': len(records), 'binaries': records, 'errors': errors, 'limitations': warnings}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--policy', type=Path, default=DEFAULT_POLICY)
    parser.add_argument('--main-executable', help='Exact bundle-relative path (otherwise find one codestyles executable)')
    args = parser.parse_args()
    try:
        policy = json.loads(args.policy.read_text(encoding='utf-8'))
        report = audit(args.bundle, args.target, policy, args.main_executable)
    except (AuditError, OSError, ValueError, KeyError, zipfile.BadZipFile, tarfile.TarError) as exc:
        report = {'schema_version': 1, 'target': args.target, 'static_audit_status': 'fail',
                  'validated_minimum_os': None, 'runtime_validation': 'not-performed', 'errors': [str(exc)]}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(f'Static compatibility audit: {report["static_audit_status"]}; report: {args.report}')
    for error in report['errors']:
        print(f'ERROR: {error}', file=sys.stderr)
    print('This audit does not certify minimum-OS, display-server, GPU, or driver compatibility.')
    return 0 if report['static_audit_status'] == 'pass' else 1


if __name__ == '__main__':
    raise SystemExit(main())
