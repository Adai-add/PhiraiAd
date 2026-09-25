"""Change the existing Android DPI bridge to DisplayMetrics.densityDpi.

patch_density_dpi(bytes) -> bytes is idempotent for the supported Phira shell.
Resolve field/method IDs from the DEX tables, require exactly one known call
sequence, replace its 9 code units without relocating any code, then update
DEX SHA-1 and Adler-32. Unknown shells fail instead of receiving a guessed patch.
The equivalent Java is QuadNative.setDpi(metrics.densityDpi).
"""
import hashlib
import struct
import zlib


def _u32(data, offset):
    return struct.unpack_from('<I', data, offset)[0]


def _uleb(data, offset):
    value = 0
    for shift in range(0, 35, 7):
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
    raise ValueError('Invalid DEX ULEB128')


def _patterns(data):
    if data[:4] != b'dex\n' or _u32(data, 32) != len(data) or _u32(data, 40) != 0x12345678:
        raise ValueError('Unsupported DEX header')
    if data[12:32] != hashlib.sha1(data[32:]).digest() or _u32(data, 8) != zlib.adler32(data[12:]) & 0xffffffff:
        raise ValueError('Invalid DEX checksums')
    strings = []
    count, offset = struct.unpack_from('<II', data, 56)
    for i in range(count):
        _, start = _uleb(data, _u32(data, offset + i * 4))
        strings.append(data[start:data.index(b'\0', start)].decode('utf-8', errors='replace'))
    count, offset = struct.unpack_from('<II', data, 64)
    types = [strings[_u32(data, offset + i * 4)] for i in range(count)]
    count, offset = struct.unpack_from('<II', data, 72)
    protos = []
    for i in range(count):
        _, result, params = struct.unpack_from('<III', data, offset + i * 12)
        args = [] if not params else [types[struct.unpack_from('<H', data, params + 4 + k * 2)[0]] for k in range(_u32(data, params))]
        protos.append('(' + ''.join(args) + ')' + types[result])
    count, offset = struct.unpack_from('<II', data, 80)
    fields = {}
    for i in range(count):
        owner, kind, name = struct.unpack_from('<HHI', data, offset + i * 8)
        fields[(types[owner], strings[name], types[kind])] = i
    count, offset = struct.unpack_from('<II', data, 88)
    methods = {}
    for i in range(count):
        owner, proto, name = struct.unpack_from('<HHI', data, offset + i * 8)
        methods[(types[owner], strings[name], protos[proto])] = i
    metric = 'Landroid/util/DisplayMetrics;'
    try:
        x = fields[(metric, 'xdpi', 'F')]
        y = fields[(metric, 'ydpi', 'F')]
        density = fields[(metric, 'densityDpi', 'I')]
        minimum = methods[('Ljava/lang/Math;', 'min', '(FF)F')]
        setter = methods[('Lquad_native/QuadNative;', 'setDpi', '(I)V')]
    except KeyError as error:
        raise ValueError('Unsupported Android DPI bridge') from error
    u16 = lambda x: struct.pack('<H', x)
    call = b'\x71\x10' + u16(setter) + b'\x05\x00'
    old = b'\x52\x50' + u16(x) + b'\x52\x55' + u16(y) + b'\x71\x20' + u16(minimum) + b'\x50\x00\x0a\x05\x87\x55'
    new = b'\x52\x55' + u16(density) + b'\x00' * 14
    assert len(old) == len(new) == 18
    return old + call, new + call


def patch_density_dpi(data):
    old, new = _patterns(data)
    old_count, new_count = data.count(old), data.count(new)
    if old_count == 0 and new_count == 1:
        return data
    if old_count != 1 or new_count != 0:
        raise ValueError('Expected exactly one supported DPI call sequence')
    offset = data.index(old)
    if offset % 2:
        raise ValueError('Unaligned DPI instructions')
    result = bytearray(data)
    result[offset:offset + len(old)] = new
    result[12:32] = hashlib.sha1(result[32:]).digest()
    struct.pack_into('<I', result, 8, zlib.adler32(result[12:]) & 0xffffffff)
    return bytes(result)


def verify_density_dpi(data):
    old, new = _patterns(data)
    if data.count(old) or data.count(new) != 1:
        raise ValueError('APK still uses an unsupported DPI source')
