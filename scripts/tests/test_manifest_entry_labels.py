"""Synthetic binary-AXML coverage; no Android tools or actual APK required."""
import importlib.util
from pathlib import Path
import struct
import sys
import unittest

scripts = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(scripts))
spec = importlib.util.spec_from_file_location('package_android_release', scripts / 'package_android_release.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def fixture(resource_labels=False, missing_alias_label=False, utf8=True, unknown=False, filter_labels=False, import_activity=False):
    strings = ['http://schemas.android.com/apk/res/android', 'label', 'name', 'application', 'activity', 'activity-alias',
               'intent-filter', 'action', 'category', 'data', 'meta-data', 'value', 'mimeType',
               'android.intent.action.MAIN', 'android.intent.category.LAUNCHER', 'android.intent.action.VIEW',
               'application/zip', 'org.flos.phira.MainActivity', 'org.flos.phira.ImportActivity' if import_activity else 'org.flos.phira.ChartActivity',
               'org.flos.phira.ResPackActivity', 'PhiraiAD', 'org.example.UnrelatedActivity', 'Unrelated',
               'org.example.ZipActivity', 'android']
    payload = bytearray(); offsets = []
    for value in strings:
        offsets.append(len(payload)); payload.extend(m._encode_pool_string(value, utf8))
    payload.extend(bytes(-len(payload) % 4))
    pool = struct.pack('<HHI5I', 1, 28, 28 + 4 * len(strings) + len(payload), len(strings), 0, 0x100 if utf8 else 0, 28 + 4 * len(strings), 0)
    pool += struct.pack(f'<{len(offsets)}I', *offsets) + payload
    ids = [0] * len(strings)
    ids[1] = m.ANDROID_ATTR_LABEL; ids[2] = 0x01010003; ids[11] = 0x01010024; ids[12] = 0x01010026
    mapping = struct.pack('<HHI', 0x0180, 8, 8 + 4 * len(ids)) + struct.pack(f'<{len(ids)}I', *ids)
    def attr(name, value, reference=False):
        return struct.pack('<IIIHBBI', 0, strings.index(name), 0xffffffff if reference else strings.index(value), 8, 0, 1 if reference else 3, 0x7f010001 if reference else strings.index(value))
    def node(tag, attributes, children=b''):
        start = struct.pack('<HHIII', 0x0102, 16, 36 + len(attributes) * 20, 1, 0xffffffff)
        start += struct.pack('<II6H', 0xffffffff, strings.index(tag), 20, 20, len(attributes), 0, 0, 0) + b''.join(attributes)
        end = struct.pack('<HHIIIII', 0x0103, 16, 24, 1, 0xffffffff, 0xffffffff, strings.index(tag))
        return start + children + end
    def intent(launcher=False):
        return node('intent-filter', [attr('label', 'PhiraiAD')] if filter_labels else [], node('action', [attr('name', 'android.intent.action.MAIN' if launcher else 'android.intent.action.VIEW')]) +
                    (node('category', [attr('name', 'android.intent.category.LAUNCHER')]) if launcher else node('data', [attr('mimeType', 'application/zip')])))
    def entry(name, launcher=False):
        attrs = [attr('name', name)]
        if launcher or not missing_alias_label: attrs.append(attr('label', 'PhiraiAD', resource_labels))
        return node('activity' if launcher else 'activity-alias', attrs, intent(launcher))
    body = node('application', [attr('label', 'PhiraiAD', resource_labels)],
                entry('org.flos.phira.MainActivity', True) + entry('org.flos.phira.ImportActivity' if import_activity else 'org.flos.phira.ChartActivity') +
                entry('org.example.ZipActivity' if unknown else 'org.flos.phira.ResPackActivity') +
                node('activity', [attr('name', 'org.example.UnrelatedActivity'), attr('label', 'Unrelated')]))
    ns_start = struct.pack('<HHIIIII', 0x0100, 16, 24, 1, 0xffffffff, strings.index('android'), 0)
    ns_end = struct.pack('<HHIIIII', 0x0101, 16, 24, 1, 0xffffffff, strings.index('android'), 0)
    xml = pool + mapping + ns_start + body + ns_end
    return struct.pack('<HHI', 3, 8, len(xml) + 8) + xml


def labels(data):
    strings = None; found = {}
    for pos, kind, header, size in m._manifest_chunks(data):
        if kind == 1: strings = m._decode_string_pool(data[pos:pos + size])[0]
        if kind != 0x0102: continue
        ext = pos + header; element = strings[struct.unpack_from('<I', data, ext + 4)[0]]
        if element not in {'application', 'activity', 'activity-alias'}: continue
        start, stride, count = struct.unpack_from('<HHH', data, ext + 8)
        values = {}
        for i in range(count):
            a = ext + start + i * stride
            name = strings[struct.unpack_from('<I', data, a + 4)[0]]
            value = struct.unpack_from('<I', data, a + 16)[0]
            if data[a + 15] == 3: values[name] = strings[value]
        found[values.get('name', 'application')] = values.get('label')
    assert struct.unpack_from('<I', data, 4)[0] == len(data)
    return found


class LabelsTest(unittest.TestCase):
    def check(self, **options):
        patched = m.patch_manifest_label(fixture(**options), 'PhiraiAd')
        values = labels(patched)
        self.assertEqual(values['application'], 'PhiraiAd')
        self.assertEqual(values['org.flos.phira.MainActivity'], 'PhiraiAd')
        self.assertEqual(values['org.flos.phira.ImportActivity' if options.get('import_activity') else 'org.flos.phira.ChartActivity'], '导入到PhiraiAd(谱面)')
        self.assertEqual(values['org.flos.phira.ResPackActivity'], '导入到PhiraiAd(资源包)')
        self.assertEqual(values['org.example.UnrelatedActivity'], 'Unrelated')
        return patched
    def test_flattened_existing_labels(self): self.check()
    def test_resource_reference_labels(self): self.check(resource_labels=True)
    def test_insert_missing_alias_label(self): self.check(missing_alias_label=True)
    def test_utf16_pool(self): self.check(utf8=False)
    def test_explicit_intent_filter_labels(self):
        patched = self.check(filter_labels=True, missing_alias_label=True)
        strings = next(m._decode_string_pool(patched[pos:pos+size])[0] for pos, kind, header, size in m._manifest_chunks(patched) if kind == 1)
        values = []
        for pos, kind, header, size in m._manifest_chunks(patched):
            if kind != 0x0102: continue
            ext = pos + header
            if strings[struct.unpack_from('<I', patched, ext+4)[0]] != 'intent-filter': continue
            start, stride, count = struct.unpack_from('<HHH', patched, ext+8)
            for i in range(count):
                attr = ext + start + i * stride
                if strings[struct.unpack_from('<I', patched, attr+4)[0]] == 'label':
                    values.append(strings[struct.unpack_from('<I', patched, attr+16)[0]])
        self.assertEqual(values, ['PhiraiAd', '导入到PhiraiAd(谱面)', '导入到PhiraiAd(资源包)'])
    def test_repeated_repack(self):
        patched = self.check()
        self.assertEqual(labels(patched), labels(m.patch_manifest_label(patched, 'PhiraiAd')))
    def test_unknown_zip_handler_fails_explicitly(self):
        with self.assertRaisesRegex(ValueError, 'Cannot identify ZIP entry'):
            m.patch_manifest_label(fixture(unknown=True), 'PhiraiAd')
    def test_known_generic_import_activity(self): self.check(import_activity=True)
    def test_metadata_roles(self):
        self.assertEqual(m._component_role('org.example.ImportActivity', ['_import_respack']), 'respack')
        self.assertEqual(m._component_role('org.example.ImportActivity', ['chart']), 'chart')

if __name__ == '__main__': unittest.main()
