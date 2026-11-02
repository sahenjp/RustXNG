#!/usr/bin/env python3
"""Offline release policy: version, notes, exact assets and checksums."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import subprocess
import tomllib
from pathlib import Path

TAG = re.compile(r'v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)\Z')


def version(tag: str) -> str:
    if not TAG.fullmatch(tag) or len(tag) > 64:
        raise ValueError('expected a stable vMAJOR.MINOR.PATCH tag')
    return tag[1:]


def may_promote(tag: str, current: str) -> bool:
    return tuple(map(int, version(tag).split('.'))) >= tuple(map(int, version(current).split('.')))


def validate_version(root: Path, tag: str) -> None:
    expected = version(tag)
    package = tomllib.loads((root / 'Cargo.toml').read_text(encoding="utf-8"))['package']
    locked = tomllib.loads((root / 'Cargo.lock').read_text(encoding="utf-8"))['package']
    if package['version'] != expected or not any(p['name'] == package['name'] and p['version'] == expected for p in locked):
        raise ValueError('tag, Cargo.toml and Cargo.lock versions must agree')


def notes(text: str, tag: str) -> str:
    wanted = version(tag)
    lines = text.splitlines()
    start = next((i for i, line in enumerate(lines) if re.match(r'^## \[' + re.escape(wanted) + r'\](?:\s|\(|$)', line)), None)
    if start is None:
        raise ValueError(f'missing changelog section for {tag}')
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith('## ')), len(lines))
    body = '\n'.join(lines[start + 1:end]).strip()
    if not body:
        raise ValueError('refuse empty release notes')
    return f'# RustXNG {tag}\n\n{body}\n'


def asset_names(tag: str, targets: list[str]) -> list[str]:
    version(tag)
    if not targets or len(targets) != len(set(targets)):
        raise ValueError('empty or duplicate release targets')
    names = ['LICENSE', 'NOTICE']
    for target in targets:
        if not re.fullmatch(r'[a-z0-9_]+(?:-[a-z0-9_]+)+', target):
            raise ValueError('invalid target')
        name = f'rustxng-{tag}-{target}' + ('.exe' if 'windows' in target else '')
        names += [name, name + '.sha256']
    return names


def verify_assets(directory: Path, tag: str, targets: list[str], sha: str = "") -> dict[str, str]:
    names = asset_names(tag, targets)
    actual = {p.name for p in directory.iterdir()}
    if actual - set(names) - {'release-manifest.json'}:
        raise ValueError('unexpected release asset')
    hashes = {}
    for name in names:
        path = directory / name
        if not path.is_file() or path.is_symlink() or not path.stat().st_size:
            raise ValueError(f'missing, empty or symlink asset: {name}')
        with path.open('rb') as stream:
            hashes[name] = hashlib.file_digest(stream, 'sha256').hexdigest()
        if name.endswith('.sha256'):
            binary = name.removesuffix('.sha256')
            expected = re.fullmatch(r'([0-9a-f]{64}) [ *]' + re.escape(binary) + r'\n?', path.read_text(encoding="utf-8"))
            if expected is None or expected[1] != hashes[binary]:
                raise ValueError(f'invalid checksum or filename: {name}')
    manifest_path = directory / 'release-manifest.json'
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
        if manifest.get('tag') != tag or manifest.get('targets') != targets or manifest.get('sha256') != hashes:
            raise ValueError('release manifest disagrees with downloaded assets')
        if not re.fullmatch(r'[0-9a-f]{40}', manifest.get('commit', '')) or (sha and manifest['commit'] != sha):
            raise ValueError('release manifest source commit mismatch')
    return hashes


def check_binary(path: Path, target: str, tag: str, native: bool) -> None:
    with path.open('rb') as stream:
        header = stream.read(64)
        if len(header) < 64:
            raise ValueError('truncated executable header')
        if 'linux' in target:
            machine = struct.unpack('<H', header[18:20])[0]
            expected = 183 if target.startswith('aarch64') else 62
            valid = header[:4] == b'\x7fELF' and header[4:6] == b'\x02\x01' and machine == expected
        elif 'windows' in target:
            offset = struct.unpack('<I', header[60:64])[0]
            stream.seek(offset)
            pe = stream.read(6)
            valid = header[:2] == b'MZ' and pe == b'PE\x00\x00\x64\x86'
        elif 'apple' in target:
            expected = 0x0100000c if target.startswith('aarch64') else 0x01000007
            valid = header[:4] == b'\xcf\xfa\xed\xfe' and struct.unpack('<I', header[4:8])[0] == expected
        else:
            valid = False
    if not valid:
        raise ValueError('binary architecture does not match the release target')
    if native:
        result = subprocess.run([str(path.resolve()), '--version'], check=True, capture_output=True, text=True)
        if not result.stdout.strip().endswith(' ' + version(tag)):
            raise ValueError('binary version differs from release tag')


def ci_gate(runs: list[dict], sha: str, repository: str) -> bool:
    return any(r.get('name') == 'ci' and r.get('head_sha') == sha and r.get('head_branch') == 'main'
               and r.get('event') == 'push' and r.get('conclusion') == 'success'
               and r.get('head_repository', {}).get('full_name') == repository for r in runs)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['validate', 'notes', 'verify', 'manifest', 'ci-gate', 'binary', 'promote'])
    parser.add_argument('--root', type=Path, default=Path('.'))
    parser.add_argument('--tag', required=True)
    parser.add_argument('--dist', type=Path, default=Path('dist'))
    parser.add_argument('--targets', nargs='*', default=[])
    parser.add_argument('--sha', default='')
    parser.add_argument('--runs', type=Path)
    parser.add_argument('--repository', default='')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--target', default='')
    parser.add_argument('--current-tag', default='')
    parser.add_argument('--native', action='store_true')
    args = parser.parse_args()
    version(args.tag)
    if args.command == 'promote':
        print('true' if may_promote(args.tag, args.current_tag) else 'false')
    elif args.command == 'validate':
        validate_version(args.root, args.tag)
    elif args.command == 'binary':
        if not args.binary:
            raise ValueError('missing binary')
        check_binary(args.binary, args.target, args.tag, args.native)
    elif args.command == 'notes':
        print(notes((args.root / 'CHANGELOG.md').read_text(encoding="utf-8"), args.tag), end='')
    elif args.command == 'ci-gate':
        if not args.runs or not ci_gate(json.loads(args.runs.read_text(encoding="utf-8"))['workflow_runs'], args.sha, args.repository):
            raise SystemExit('the exact release commit has no successful main push CI run')
    else:
        hashes = verify_assets(args.dist, args.tag, args.targets, args.sha)
        if args.command == 'manifest':
            if not re.fullmatch(r'[0-9a-f]{40}', args.sha):
                raise ValueError('expected a full source commit SHA')
            manifest = {'tag': args.tag, 'commit': args.sha, 'targets': args.targets, 'sha256': hashes}
            (args.dist / 'release-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8', newline='\n')
        print(f'verified {len(hashes)} release files')


if __name__ == '__main__':
    main()
