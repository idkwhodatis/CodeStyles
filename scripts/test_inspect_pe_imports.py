import struct
import unittest
from inspect_pe_imports import inspect_pe


def fixture():
    data = bytearray(1024)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3c, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    struct.pack_into('<HH', data, 0x84, 0xaa64, 1)
    struct.pack_into('<H', data, 0x94, 240)
    struct.pack_into('<H', data, 0x98, 0x20b)
    struct.pack_into('<I', data, 0x98 + 60, 512)
    struct.pack_into('<I', data, 0x98 + 108, 16)
    struct.pack_into('<II', data, 0x98 + 120, 0x1000, 40)
    struct.pack_into('<IIII', data, 0x98 + 240 + 8, 512, 0x1000, 512, 512)
    struct.pack_into('<I', data, 512 + 12, 0x1080)
    data[640:653] = b'KERNEL32.dll\0'
    return data


class PEImportTests(unittest.TestCase):
    def test_arm64_regular_and_delay(self):
        data = fixture()
        struct.pack_into('<II', data, 0x98 + 112 + 13 * 8, 0x1100, 64)
        struct.pack_into('<II', data, 768, 1, 0x1080)
        result = inspect_pe(data)
        self.assertEqual(result['machine'], '0xaa64')
        self.assertEqual(result['imports'], ['KERNEL32.dll'])
        self.assertEqual(result['delay_imports'], ['KERNEL32.dll'])

    def test_invalid_bounds(self):
        for offset, value in ((0x3c, 4096), (512 + 12, 0x3000),
                              (0x98 + 124, 4096), (0x98 + 108, 17)):
            data = fixture()
            struct.pack_into('<I', data, offset, value)
            with self.assertRaises(ValueError):
                inspect_pe(data)

    def test_unterminated_and_va_delay(self):
        data = fixture()
        struct.pack_into('<I', data, 0x98 + 124, 20)
        with self.assertRaises(ValueError):
            inspect_pe(data)
        data = fixture()
        struct.pack_into('<II', data, 0x98 + 112 + 13 * 8, 0x1100, 64)
        struct.pack_into('<II', data, 768, 0, 0x1080)
        with self.assertRaises(ValueError):
            inspect_pe(data)


if __name__ == '__main__':
    unittest.main()
