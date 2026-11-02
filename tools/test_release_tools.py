import hashlib
import tempfile
import struct
import json
import tomllib
import unittest
from pathlib import Path

from release_tools import check_binary, may_promote, asset_names, ci_gate, notes, validate_version, verify_assets, version


class ReleasePolicyTests(unittest.TestCase):
    def test_old_release_retries_cannot_downgrade_latest(self):
        self.assertFalse(may_promote('v0.26.0', 'v0.27.0'))
        self.assertTrue(may_promote('v0.27.0', 'v0.27.0'))
        self.assertTrue(may_promote('v0.28.0', 'v0.27.9'))

    def test_tag_versions_are_strict(self):
        self.assertEqual(version('v0.27.0'), '0.27.0')
        for tag in ['v01.2.3', 'v1.2', 'v1.2.3/../x', 'main', 'v1.2.3-rc.1']:
            with self.assertRaises(ValueError):
                version(tag)

    def test_changelog_heading_is_literal_and_does_not_consume_next_release(self):
        text = '## [Unreleased]\nfuture\n## [0.27.0](url) (date)\n\nFixed.\n## [0.26.0]\nOld.'
        self.assertEqual(notes(text, 'v0.27.0'), '# RustXNG v0.27.0\n\nFixed.\n')
        with self.assertRaises(ValueError):
            notes(text, 'v0.28.0')
        with self.assertRaises(ValueError):
            notes('## [0.27.0]\n', 'v0.27.0')

    def test_versions_must_match_both_cargo_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'Cargo.toml').write_text('[package]\nname="rustxng"\nversion="0.27.0"\n')
            (root/'Cargo.lock').write_text('[[package]]\nname="rustxng"\nversion="0.26.0"\n')
            with self.assertRaises(ValueError):
                validate_version(root, 'v0.27.0')

    def test_assets_require_complete_set_and_matching_bare_filename_checksums(self):
        targets = ['x86_64-unknown-linux-gnu', 'x86_64-pc-windows-msvc']
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in asset_names('v0.27.0', targets):
                if not name.endswith('.sha256'):
                    (root/name).write_bytes(b'fixture')
                else:
                    binary = name.removesuffix('.sha256')
                    (root/name).write_text(hashlib.sha256(b'fixture').hexdigest() + '  ' + binary + '\n')
            hashes = verify_assets(root, 'v0.27.0', targets)
            self.assertEqual(len(hashes), 6)
            manifest = dict(tag='v0.27.0', targets=targets, sha256=hashes, commit='a'*40)
            (root/'release-manifest.json').write_text(json.dumps(manifest))
            verify_assets(root, 'v0.27.0', targets, 'a'*40)
            with self.assertRaises(ValueError):
                verify_assets(root, 'v0.27.0', targets, 'b'*40)
            (root/'rustxng-v0.27.0-x86_64-unknown-linux-gnu').write_bytes(b'changed')
            with self.assertRaises(ValueError):
                verify_assets(root, 'v0.27.0', targets)

    def test_binary_machine_is_checked_before_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'binary'
            header = bytearray(64)
            header[:6] = b'\x7fELF\x02\x01'
            header[18:20] = struct.pack('<H', 62)
            path.write_bytes(header)
            check_binary(path, 'x86_64-unknown-linux-gnu', 'v0.27.0', False)
            with self.assertRaises(ValueError):
                check_binary(path, 'aarch64-unknown-linux-gnu', 'v0.27.0', False)
            header[:4] = b'\xcf\xfa\xed\xfe'
            header[4:8] = struct.pack('<I', 0x0100000c)
            path.write_bytes(header)
            check_binary(path, 'aarch64-apple-darwin', 'v0.27.0', False)
            with self.assertRaises(ValueError):
                check_binary(path, 'x86_64-apple-darwin', 'v0.27.0', False)

    def test_repository_release_configuration_agrees(self):
        root = Path(__file__).resolve().parents[1]
        cargo_version = tomllib.loads((root/'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
        self.assertEqual(json.loads((root/'.release-please-manifest.json').read_text(encoding='utf-8'))['.'], cargo_version)
        validate_version(root, 'v' + cargo_version)
        self.assertTrue(notes((root/'CHANGELOG.md').read_text(encoding='utf-8'), 'v' + cargo_version))
        targets = json.loads((root/'.github/release-targets.json').read_text(encoding='utf-8'))
        self.assertEqual(len(targets), 6)
        self.assertIn('aarch64-unknown-linux-gnu', [item['target'] for item in targets])
        self.assertEqual(len(asset_names('v'+cargo_version, [item['target'] for item in targets])), 14)

    def test_only_exact_trusted_main_push_ci_can_release(self):
        run = dict(name='ci', head_sha='a', head_branch='main', event='push', conclusion='success', head_repository={'full_name': 'o/r'})
        self.assertTrue(ci_gate([run], 'a', 'o/r'))
        for key, value in [('head_sha', 'b'), ('event', 'pull_request'), ('head_branch', 'other'), ('conclusion', 'failure')]:
            self.assertFalse(ci_gate([{**run, key: value}], 'a', 'o/r'))
        self.assertFalse(ci_gate([run], 'a', 'other/repo'))


if __name__ == '__main__':
    unittest.main()
