"""Widen what another crate reads to `pub` (the crate split): every item of widen.tsv (join.py, from the SCIP index).

usage: widen.py <widen.tsv> [--apply]

A restricted visibility (`pub(crate)`, `pub(super)`, `pub(in ...)`) becomes `pub`; a private item gets `pub ` before
its keyword (a field before its name). A method of a trait or of a trait's impl carries no visibility and is left
(its trait is what is widened). Modules are mounts.py's (it widens the declarations it writes). Every edit checks
the text it replaces, and refuses the file otherwise. Positions are the index's (1-based lines, character columns).
"""
import collections, os, re, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
TOOL = os.environ.get('CRATIFY_BIN', 'cratify')
KEYWORD = re.compile(r'((?:default\s+|async\s+|const\s+|unsafe\s+|extern\s+"[^"]*"\s+)*(?:fn|struct|enum|union|type|const|static|trait|macro_rules!?|mod)\s+)$')


def trait_spans(path):
    """Byte spans of the file's traits and trait impls (their methods take no visibility)."""
    out = subprocess.run([TOOL, 'items', path], capture_output=True, text=True, check=True).stdout
    spans = []
    for line in out.splitlines():
        f, s, e, kind, name, mod, cfg = line.split('\t')
        if kind == 'trait' or (kind == 'impl' and ' for ' in name):
            spans.append((int(s), int(e)))
    return spans


def main(tsv, *flags):
    rows = collections.defaultdict(list)
    for line in list(open(tsv))[1:]:
        f, l, c, kind, now, at, item, users = line.rstrip('\n').split('\t')
        if kind == 'module':
            continue
        rows[f].append((int(l), int(c), kind, now, at, item))
    done = collections.Counter()
    refused = []
    for f, items in sorted(rows.items()):
        p = os.path.join(ROOT, f)
        lines = open(p, encoding='utf-8').read().split('\n')
        byte_start = [0]
        for x in lines:
            byte_start.append(byte_start[-1] + len(x.encode()) + 1)
        traits = trait_spans(p)
        edits = []  # (line index, col start, col end, text)
        for l, c, kind, now, at, item in items:
            if now == 'private':
                i = l - 1
                if i >= len(lines):
                    refused.append(f'{f}:{l} {item}: past the file\'s end (a stale index)')
                    continue
                b = byte_start[i] + len(lines[i][:c].encode())
                if kind == 'method' and any(s <= b < e for s, e in traits):
                    done['trait method'] += 1
                    continue
                before = lines[i][:c]
                if kind == 'field':
                    edits.append((i, c, c, 'pub '))
                else:
                    m = KEYWORD.search(before)
                    if not m:
                        refused.append(f'{f}:{l}:{c} {kind} {item}: no keyword before the name: {lines[i].strip()[:80]}')
                        continue
                    edits.append((i, m.start(), m.start(), 'pub '))
            else:
                pos, span = at.split(':')
                i = int(pos) - 1
                s, e = (int(x) for x in span.split('-'))
                if i >= len(lines) or lines[i][s:e].replace(' ', '') != now:
                    refused.append(f'{f}:{l} {item}: expected `{now}` at {at}, found `{lines[i][s:e] if i < len(lines) else "(past the end)"}`')
                    continue
                edits.append((i, s, e, 'pub'))
            done[kind] += 1
        for i, s, e, t in sorted(set(edits), reverse=True):
            lines[i] = lines[i][:s] + t + lines[i][e:]
        if '--apply' in flags:
            open(p, 'w', encoding='utf-8').write('\n'.join(lines))
    print(f'widened: {dict(done)}')
    for r in refused:
        print('REFUSED', r)
    return 1 if refused else 0


if __name__ == '__main__':
    sys.exit(main(*sys.argv[1:]))
