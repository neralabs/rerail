#!/usr/bin/env python3
"""What a merge dropped: each side's work since the merge base that the result does not keep.

Usage (from anywhere in the checkout):
  scripts/check/merge-audit.py               # the merge in progress: HEAD, MERGE_HEAD, the working tree
  scripts/check/merge-audit.py <merge>       # a merge commit
  scripts/check/merge-audit.py --since 2026-11-11   # every two-parent merge on HEAD since a date

For each side it reports:
  - lines the side added that the result has nowhere in its tree (a dropped fix, test, check or statement);
  - lines the side deleted that the result has again (an undone removal);
  - files the side added that the result lacks (renames show as a hint when a same-named file exists);
  - decision numbers used by two files in docs/final-shape/decisions.

A line counts as kept if it appears anywhere in the result's tree, so moved code is not reported. What is
left is either a deliberate supersession (say which commit or decision supersedes it in the merge commit) or a
loss (restore it before committing). Exit status 1 when anything is reported.
"""
import collections
import os
import re
import subprocess
import sys

SKIP_SUFFIX = ('.lock', '.json', '.tsv', '.csv', '.svg', '.sqlite', '.bin', '.rir', '.png')
SKIP_PREFIX = ('benchmark/', 'target/', 'gate/data-layer/')
TREES = ['native', 'docs', 'src', 'test', 'scripts', 'tools', 'packs', 'AGENTS.md']


def git(*a, ok=False):
    r = subprocess.run(['git', *a], capture_output=True, text=True)
    if r.returncode and not ok:
        sys.exit(f'git {" ".join(a)}: {r.stderr.strip()}')
    return r.stdout


def keep(path):
    return not path.endswith(SKIP_SUFFIX) and not path.startswith(SKIP_PREFIX)


def blob(rev, path):
    """A file's lines at a revision; rev None is the working tree."""
    if rev is None:
        try:
            with open(path, encoding='utf-8', errors='replace') as f:
                return f.read().split('\n')
        except (FileNotFoundError, IsADirectoryError):
            return None
    global _cat
    if _cat is None:
        _cat = subprocess.Popen(['git', 'cat-file', '--batch'], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    _cat.stdin.write(f'{rev}:{path}\n'.encode())
    _cat.stdin.flush()
    head = _cat.stdout.readline().decode().split()
    if len(head) < 3 or head[1] != 'blob':
        return None
    data = _cat.stdout.read(int(head[2]))
    _cat.stdout.read(1)
    return data.decode('utf-8', 'replace').split('\n')


_cat = None


def changed(a, b):
    out = git('diff', '--no-renames', '--name-status', a, b)
    return [l.split('\t') for l in out.strip().split('\n') if l]


def anywhere(rev, lines):
    """The subset of lines found as a substring of some line in the result's tree."""
    if not lines:
        return set()
    pat = os.path.join(git('rev-parse', '--git-dir').strip(), 'merge-audit-patterns')
    with open(pat, 'w') as f:
        f.write('\n'.join(lines) + '\n')
    args = ['grep', '-h', '-F', '-f', pat] + ([rev] if rev else []) + ['--'] + TREES
    found = {l.strip() for l in git(*args, ok=True).split('\n')}
    os.remove(pat)
    return {t for t in lines if t in found}


def files_at(rev):
    return git('ls-tree', '-r', '--name-only', rev).split('\n') if rev else git('ls-files').split('\n')


def audit(result, p1, p2, title):
    base = git('merge-base', p1, p2).strip()
    names = files_at(result)
    by_name = collections.defaultdict(list)
    for n in names:
        by_name[os.path.basename(n)].append(n)
    report = []
    for side, sp, op in (('ours', p1, p2), ('theirs', p2, p1)):
        candidates = []
        for st in changed(base, sp):
            status, path = st[0], st[-1]
            if not keep(path):
                continue
            res = blob(result, path)
            if status.startswith('A') and res is None:
                hint = by_name.get(os.path.basename(path), [])
                report.append(f'{side}: added {path}, result lacks it' + (f' (same name at {hint[0]})' if hint else ''))
                continue
            if status.startswith('D'):
                if res is not None and blob(op, path) is None:
                    report.append(f'{side}: deleted {path}, result has it again')
                continue
            if res is None:
                if blob(op, path) is not None:
                    report.append(f'{side}: changed {path}, result lacks it')
                continue
            b = collections.Counter(map(str.strip, blob(base, path) or []))
            s = collections.Counter(map(str.strip, blob(sp, path) or []))
            o = collections.Counter(map(str.strip, blob(op, path) or []))
            r = collections.Counter(map(str.strip, res))
            lost = [t for t in s if len(t) > 3 and s[t] > b[t] and r[t] <= b[t]]
            back = [t for t in b if len(t) > 3 and b[t] > s[t] and r[t] > s[t] and o[t] == b[t]]
            candidates.append((path, lost, back))
        kept = anywhere(result, sorted({t for _, lost, _ in candidates for t in lost}))
        for path, lost, back in candidates:
            gone = [t for t in lost if t not in kept]
            for t in gone:
                report.append(f'{side}: {path}: dropped: {t}')
            for t in back:
                report.append(f'{side}: {path}: undone removal: {t}')
    numbers = collections.defaultdict(list)
    for n in names:
        m = re.match(r'docs/final-shape/decisions/(\d{4})-', n)
        if m:
            numbers[m.group(1)].append(n)
    for k, v in sorted(numbers.items()):
        if len(v) > 1:
            report.append(f'decision number {k} used twice: ' + ', '.join(v))
    print(f'== {title}: {len(report)} finding(s)')
    for l in report:
        print('  ' + l)
    return bool(report)


def main():
    os.chdir(git('rev-parse', '--show-toplevel').strip())
    args = sys.argv[1:]
    if not args:
        merge_head = git('rev-parse', '-q', '--verify', 'MERGE_HEAD', ok=True).strip()
        if not merge_head:
            sys.exit('no merge in progress; give a merge commit or --since DATE')
        bad = audit(None, 'HEAD', merge_head, 'the merge in progress')
    elif args[0] == '--since':
        bad = False
        for line in git('log', '--merges', f'--since={args[1]}', '--format=%H %P %s', 'HEAD').strip().split('\n'):
            parts = line.split(' ', 3)
            if len(parts) >= 3 and len(parts[2]) == 40:
                bad |= audit(parts[0], parts[1], parts[2], f'{parts[0][:9]} {parts[3] if len(parts) > 3 else ""}')
    else:
        m = git('rev-parse', args[0]).strip()
        parents = git('rev-list', '--parents', '-n1', m).split()[1:]
        if len(parents) != 2:
            sys.exit(f'{args[0]} is not a two-parent merge')
        bad = audit(m, parents[0], parents[1], args[0])
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
