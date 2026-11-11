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


def enum_spans(path):
    """Byte spans of the file's enums (their variants' fields take no visibility)."""
    out = subprocess.run([TOOL, 'items', path], capture_output=True, text=True, check=True).stdout
    return [(int(s), int(e)) for f, s, e, kind, name, mod, cfg in (l.split('\t') for l in out.splitlines()) if kind == 'enum']


def struct_spans(path):
    """Byte spans of the file's structs and unions (where a field is declared)."""
    out = subprocess.run([TOOL, 'items', path], capture_output=True, text=True, check=True).stdout
    return [(int(s), int(e)) for f, s, e, kind, name, mod, cfg in (l.split('\t') for l in out.splitlines()) if kind in ('struct', 'union')]


def item_name(item):
    """The name an index symbol ends in (`a/b/Name#`, `a/b/f().`, `a/B#field.`)."""
    m = re.findall(r'[A-Za-z_][A-Za-z_0-9]*', item.split('/')[-1])
    return m[-1] if m else None


def locate(lines, i, c, name, kind, byte_start=None, structs=()):
    """The (line, column) of the item's name: the index's, when the file still has the name there, else the line
    nearest to it where the name follows a keyword (a field: begins a `name:`). None: not found (refused)."""
    if name is None:
        return None
    def ok(j, col):
        if lines[j][col:col + len(name)] != name or re.match(r'[A-Za-z_0-9]', lines[j][col + len(name):col + len(name) + 1] or ' '):
            return False
        before = lines[j][:col]
        if kind == 'field':
            b = byte_start[j] + len(lines[j][:col].encode())
            if not any(s <= b < e for s, e in structs):
                return False  # a struct literal's field, a pattern's: not a declaration
            return re.fullmatch(r'\s*(?:pub(?:\s*\([^)]*\))?\s+)?', before) is not None and lines[j][col + len(name):].lstrip().startswith(':')
        return KEYWORD.search(before) is not None
    if i < len(lines) and ok(i, c):
        return i, c
    for d in range(1, 400):
        for j in (i - d, i + d):
            if 0 <= j < len(lines):
                for m in re.finditer(r'\b' + re.escape(name) + r'\b', lines[j]):
                    if ok(j, m.start()):
                        return j, m.start()
    return None


def main(tsv, *flags):
    rows = collections.defaultdict(list)
    for line in list(open(tsv))[1:]:
        f, l, c, kind, now, at, item, users = line.rstrip('\n').split('\t')
        if kind in ('module', 'macro'):
            continue  # modules are apply.py's; a macro crosses crates by #[macro_export] or a `pub use`, never `pub`
        if kind == '70':
            kind = 'method'  # SCIP's kind for a method a trait declares
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
        enums = enum_spans(p)
        structs = struct_spans(p)
        edits = []  # (line index, col start, col end, text)
        for l, c, kind, now, at, item in items:
            if now == 'private':
                if kind == 'field' and l - 1 < len(lines):
                    b0 = byte_start[l - 1] + len(lines[l - 1][:c].encode())
                    if any(s <= b0 < e for s, e in enums) and not any(s <= b0 < e for s, e in structs):
                        done['variant field'] += 1
                        continue
                found = locate(lines, l - 1, c, item_name(item), kind, byte_start, structs)
                if found is None:
                    refused.append(f'{f}:{l} {item}: not at the index\'s place nor near it (a stale index)')
                    continue
                i, c = found
                b = byte_start[i] + len(lines[i][:c].encode())
                # anything in a trait or a trait's impl (SCIP's `static` is an associated fn without self, too)
                if kind != 'field' and any(s <= b < e for s, e in traits):
                    done['trait method'] += 1
                    continue
                if kind == 'field' and any(s <= b < e for s, e in enums):
                    done['variant field'] += 1
                    continue
                before = lines[i][:c]
                if kind == 'field':
                    at_ = c
                else:
                    m = KEYWORD.search(before)
                    if not m:
                        refused.append(f'{f}:{l}:{c} {kind} {item}: no keyword before the name: {lines[i].strip()[:80]}')
                        continue
                    at_ = m.start()
                # the file already gives it a visibility (the index is older): that one is replaced, `pub` kept
                v = re.search(r'(pub(?:\s*\([^)]*\))?)\s+$', lines[i][:at_])
                if v and v.group(1) == 'pub':
                    done[kind] += 1
                    continue
                if v:
                    edits.append((i, v.start(1), v.end(1), 'pub'))
                else:
                    edits.append((i, at_, at_, 'pub '))
            else:
                pos, span = at.split(':')
                i = int(pos) - 1
                s, e = (int(x) for x in span.split('-'))
                if i >= len(lines) or lines[i][s:e].replace(' ', '') != now:
                    # moved: the nearest line with the name after this visibility
                    name = item_name(item)
                    found = None
                    for d in range(0, 400):
                        for j in (i - d, i + d):
                            if 0 <= j < len(lines) and name:
                                m = re.search(r'(pub\s*\([^)]*\))\s+(?:(?:unsafe|async|const|extern\s+"C")\s+)*(?:fn|struct|enum|trait|type|const|static|union|mod|use)?\s*' + re.escape(name) + r'\b', lines[j])
                                if m and m.group(1).replace(' ', '') == now:
                                    found = (j, m.start(1), m.end(1))
                                    break
                        if found:
                            break
                    if found:
                        i, s, e = found
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
