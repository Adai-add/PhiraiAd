"""Run with the r22.1 APK: python3 tests/verify_android_dpi.py BASE.apk."""
from pathlib import Path
import hashlib
import struct
import sys
import unittest
import zipfile
import zlib
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from dex_density_dpi import patch_density_dpi, verify_density_dpi, _patterns
with zipfile.ZipFile(sys.argv.pop(1)) as apk:
    ORIGINAL = apk.read('classes.dex')

def rehash(data):
    data = bytearray(data)
    struct.pack_into('<I', data, 32, len(data))
    data[12:32] = hashlib.sha1(data[32:]).digest()
    struct.pack_into('<I', data, 8, zlib.adler32(data[12:]) & 0xffffffff)
    return bytes(data)

class DpiPatchTests(unittest.TestCase):
    def test_only_dpi_instructions_and_checksums_change(self):
        old, new = _patterns(ORIGINAL)
        offset = ORIGINAL.index(old)
        patched = patch_density_dpi(ORIGINAL)
        verify_density_dpi(patched)
        self.assertEqual(len(patched), len(ORIGINAL))
        self.assertEqual(patched[32:offset], ORIGINAL[32:offset])
        self.assertEqual(patched[offset + 18:], ORIGINAL[offset + 18:])
        self.assertEqual(patched[offset:offset + len(new)], new)
    def test_idempotent_for_future_releases(self):
        patched = patch_density_dpi(ORIGINAL)
        self.assertEqual(patch_density_dpi(patched), patched)
    def test_corrupt_dex_is_rejected(self):
        corrupt = bytearray(ORIGINAL); corrupt[-1] ^= 1
        with self.assertRaises(ValueError): patch_density_dpi(bytes(corrupt))
    def test_unknown_instruction_sequence_is_rejected(self):
        old, _ = _patterns(ORIGINAL)
        corrupt = bytearray(ORIGINAL); corrupt[ORIGINAL.index(old)+1] = 0x51
        with self.assertRaises(ValueError): patch_density_dpi(rehash(corrupt))
    def test_original_dpi_source_fails_final_verification(self):
        with self.assertRaises(ValueError): verify_density_dpi(ORIGINAL)

unittest.main()
