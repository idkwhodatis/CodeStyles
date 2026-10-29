#!/usr/bin/env python3
"""Read PE32+ import names without loading/executing binaries (stdlib only).

Static loader metadata is not proof of runtime behavior: dynamically constructed
LoadLibrary calls and clean/minimum-OS execution require separate native tests.
"""
import argparse
import json
import mmap
import struct


def inspect_pe(data):
    def read(offset, size):
        if offset < 0 or size < 0 or offset + size > len(data):
            raise ValueError('PE read outside file')
        return data[offset:offset + size]

    def unpack(fmt, offset):
        return struct.unpack(fmt, read(offset, struct.calcsize(fmt)))

    if read(0, 2) != b'MZ':
        raise ValueError('Missing DOS signature')
    pe, = unpack('<I', 0x3c)
    if read(pe, 4) != b'PE\0\0':
        raise ValueError('Missing PE signature')
    machine, sections = unpack('<HH', pe + 4)
    optional_size, = unpack('<H', pe + 20)
    optional = pe + 24
    read(optional, optional_size)
    magic, = unpack('<H', optional)
    if magic != 0x20b or optional_size < 112:
        raise ValueError('Expected PE32+ optional header')
    count, = unpack('<I', optional + 108)
    if count > 16 or 112 + count * 8 > optional_size:
        raise ValueError('Invalid PE data directories')
    headers_size, = unpack('<I', optional + 60)
    if headers_size > len(data) or headers_size < optional + optional_size + sections * 40:
        raise ValueError('Invalid PE header size')
    table = []
    for index in range(sections):
        section = optional + optional_size + index * 40
        virtual_size, rva, raw_size, raw = unpack('<IIII', section + 8)
        read(raw, raw_size)
        span = max(virtual_size, raw_size)
        if rva < headers_size or rva + span > 0x100000000:
            raise ValueError('Invalid section RVA')
        if any(rva < old_rva + old_span and old_rva < rva + span
               for old_rva, old_span, _, _ in table):
            raise ValueError('Overlapping PE sections')
        table.append((rva, span, raw, raw_size))

    def mapped(rva, size):
        if 0 <= rva and rva + size <= headers_size:
            return rva
        for base, _, raw, raw_size in table:
            if base <= rva and rva + size <= base + raw_size:
                return raw + rva - base
        raise ValueError('RVA outside file-backed section')

    def name(rva):
        value = bytearray()
        for index in range(4096):
            char = read(mapped(rva + index, 1), 1)
            if char == b'\0':
                if not value:
                    raise ValueError('Empty DLL name')
                return value.decode('ascii')
            value.extend(char)
        raise ValueError('Unterminated DLL name')

    def imports(directory, width, name_offset, delay=False):
        if count <= directory:
            return []
        rva, size = unpack('<II', optional + 112 + directory * 8)
        if not rva and not size:
            return []
        if not rva or size < width:
            raise ValueError('Invalid import directory')
        mapped(rva, size)
        result = []
        for index in range(min(size // width, 65536)):
            descriptor = read(mapped(rva + index * width, width), width)
            if not any(descriptor):
                return result
            if delay and struct.unpack_from('<I', descriptor)[0] != 1:
                raise ValueError('Delay import uses unsupported VA addressing')
            result.append(name(struct.unpack_from('<I', descriptor, name_offset)[0]))
        raise ValueError('Unterminated import directory')

    return {'machine': hex(machine), 'format': 'PE32+',
            'imports': imports(1, 20, 12),
            'delay_imports': imports(13, 32, 4, delay=True)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('file')
    args = parser.parse_args()
    with open(args.file, 'rb') as file, mmap.mmap(file.fileno(), 0, access=mmap.ACCESS_READ) as data:
        print(json.dumps(inspect_pe(data), indent=2))


if __name__ == '__main__':
    main()
