#!/usr/bin/env python3
"""Restore the measured Python blobs from an existing local data-layer Git repo."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--repo', required=True, type=Path)
p.add_argument('--out', required=True, type=Path)
a = p.parse_args()
m = json.loads(Path(__file__).with_name('corpus-manifest.json').read_text())
subprocess.run(['git', '-C', str(a.repo), 'cat-file', '-e', m['commit'] + '^{commit}'], check=True)
root = a.out.resolve()
root.mkdir(parents=True, exist_ok=True)
with subprocess.Popen(['git', '-C', str(a.repo), 'cat-file', '--batch'], stdin=subprocess.PIPE, stdout=subprocess.PIPE) as git:
    total = 0
    for entry in m['entries']:
        relative = Path(entry['path'])
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError(f'Unsafe manifest path: {relative}')
        target = root / relative
        if not target.resolve().is_relative_to(root):
            raise ValueError(f'Path escapes output through a symlink: {relative}')
        git.stdin.write((entry['git_blob'] + '\n').encode())
        git.stdin.flush()
        oid, kind, size = git.stdout.readline().decode().split()
        assert oid == entry['git_blob'] and kind == 'blob'
        data = git.stdout.read(int(size))
        assert git.stdout.read(1) == b'\n'
        assert len(data) == entry['bytes']
        assert hashlib.sha256(data).hexdigest() == entry['sha256'], relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists():
            assert target.read_bytes() == data, f'Refusing to overwrite different content: {target}'
        else:
            with target.open('xb') as f:
                f.write(data)
        total += len(data)
    git.stdin.close()
    assert git.wait() == 0
assert len(m['entries']) == m['files'] and total == m['source_bytes']
print(f'Restored {m["files"]} files, {total} bytes from {m["commit"]}')
