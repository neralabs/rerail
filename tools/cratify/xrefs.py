"""Who reads whom, by the compiler's own name resolution: rust-analyzer's SCIP index of the workspace joined with each
target's module tree and the syn tool's `sites`, so that a glob, a re-export or a facade never hides a use and an import
never counts as one.

    python3 xrefs.py join <dir>      (on the host xrefs.sh ran on: reads <dir>/{defs,refs,sites,trees,packages}.tsv)

Writes, in <dir>:
  files.tsv   file, crate, lines, role: lib (in a library's tree), test (a module compiled only under `test`), bin,
              other (no target's tree names it: examples, benches, tests/)
  edges.tsv   site file, definition file, class, references: class `code` (a use in compiled code), `test` (in code
              only `test` compiles), `use` (an import or a facade, `use` items, which name an item without using it)
  cross.tsv   site file, definition file, symbol, references: the `code` references across crates, item by item
"""
import collections, os, re, sys


def needs_test(pred):
    """Whether a cfg predicate (`test`, `all(test, ...)`, `any(...)`) holds only where `test` does."""
    pred = pred.strip()
    m = re.fullmatch(r'(all|any|not)\((.*)\)', pred, re.S)
    if not m:
        return pred == 'test'
    parts, depth, cur = [], 0, ''
    for ch in m.group(2):
        if ch == ',' and depth == 0:
            parts.append(cur); cur = ''
            continue
        depth += ch == '('
        depth -= ch == ')'
        cur += ch
    if cur.strip():
        parts.append(cur)
    if m.group(1) == 'all':
        return any(needs_test(p) for p in parts)
    if m.group(1) == 'any':
        return bool(parts) and all(needs_test(p) for p in parts)
    return False


def cfg_needs_test(cfg):
    # The tree's cfg: its `#[cfg(..)]` and `#[cfg_attr(..)]` attributes, spaces removed, one after another.
    return any(needs_test(p) for p in re.findall(r'#\[cfg\((.*?)\)\](?=#\[|$)', cfg))


def join(d):
    packages = {}  # crate -> its directory (relative to the repository)
    for line in open(f'{d}/packages.tsv'):
        name, path = line.rstrip('\n').split('\t')
        packages[name] = path.rstrip('/')
    dirs = sorted(packages.items(), key=lambda kv: -len(kv[1]))

    mounted = {}  # file -> the crate whose target mounts it (a `#[path]` can mount another crate's directory)

    def crate(f):
        if f in mounted:
            return mounted[f]
        if '/vendor/' in f:
            return ''  # a vendored crate: not the workspace's
        for name, p in dirs:
            if f == p or f.startswith(p + '/') or p == '.':
                return name
        return ''

    role, crate_of = {}, {}
    # trees.tsv: crate, target kind, then the syn tool's tree columns.
    test_mods = set()
    rows = [l.rstrip('\n').split('\t') for l in open(f'{d}/trees.tsv')]
    for row in sorted(rows, key=lambda r: r[2].count('::')):
        name, kind, path, file, decl, s, e, vis, cfg, path_attr, inline = row
        key = (name, kind, path)
        parent = (name, kind, path.rsplit('::', 1)[0]) if '::' in path else None
        test = cfg_needs_test(cfg) or (parent in test_mods)
        if test:
            test_mods.add(key)
        if inline == 'inline':
            continue  # an inline module is its file's: `sites` gives its range
        file = os.path.normpath(file)
        r = 'test' if test else ('lib' if kind in ('lib', 'proc-macro', 'rlib') else 'bin')
        # A file two targets mount is the library's when one of them is, else a program's.
        rank = {'test': 0, 'bin': 1, 'lib': 2}
        if rank[r] > rank.get(role.get(file), -1):
            role[file] = r
            mounted[file] = name

    # Each target's root file (the tree lists its modules, not the root).
    for line in open(f'{d}/targets.tsv'):
        name, kind, src = line.rstrip('\n').split('\t')
        r = 'bin' if kind == 'bin' else 'lib'
        if r == 'lib' or os.path.normpath(src) not in role:
            role[os.path.normpath(src)] = r
            mounted[os.path.normpath(src)] = name
    ranges = collections.defaultdict(list)  # file -> [(first, last, kind)]
    for line in open(f'{d}/sites.tsv'):
        file, kind, s, e = line.rstrip('\n').split('\t')
        ranges[os.path.normpath(file)].append((int(s), int(e), kind))
    lines = {}
    for f in ranges.keys() | role.keys():
        try:
            lines[f] = sum(1 for _ in open(f, errors='replace'))
        except OSError:
            lines[f] = 0

    def site_class(file, line):  # line 1-based
        if role.get(file) == 'test':
            return 'test'
        k = 'code'
        for s, e, kind in ranges.get(file, ()):
            if s <= line <= e:
                if kind == 'test':
                    return 'test'
                k = 'use'
        return k

    defs = {}
    for line in open(f'{d}/defs.tsv'):
        sym, file = line.split('\t', 2)[:2]
        file = os.path.normpath(file)
        if not file.startswith('..') and not file.startswith('/'):
            defs.setdefault(sym, file)
    edges = collections.Counter()
    cross = collections.Counter()
    for line in open(f'{d}/refs.tsv'):
        file, l, c, ec, sym = line.rstrip('\n').split('\t')
        df = defs.get(sym)
        # A module named on a path is a namespace the mounts merge, not a use (its items resolve to their own files).
        if df is None or sym.endswith('/'):
            continue
        file = os.path.normpath(file)
        k = site_class(file, int(l) + 1)
        edges[(file, df, k)] += 1
        if k == 'code':
            a, b = crate_of.setdefault(file, crate(file)), crate_of.setdefault(df, crate(df))
            if a != b:
                cross[(file, df, sym)] += 1
    with open(f'{d}/files.tsv', 'w') as o:
        for f in sorted(lines.keys() | {x for k in edges for x in k[:2]}):
            o.write(f'{f}\t{crate(f)}\t{lines.get(f, 0)}\t{role.get(f, "other")}\n')
    with open(f'{d}/edges.tsv', 'w') as o:
        for (a, b, k), n in sorted(edges.items()):
            o.write(f'{a}\t{b}\t{k}\t{n}\n')
    with open(f'{d}/cross.tsv', 'w') as o:
        for (a, b, s), n in sorted(cross.items()):
            o.write(f'{a}\t{b}\t{s}\t{n}\n')
    print(f'{len(lines)} files, {len(edges)} file edges, {len(cross)} cross-crate items')


def ties(d):
    """Files the impl rules keep in one crate (ties.tsv: impl file, the file it is tied to, why): an inherent impl
    is in its type's crate; a trait impl in its trait's or its type's, here its type's when the type is the crate's,
    else its trait's. An impl is read from its items' symbols (`.../impl#[Type][Trait]item().`)."""
    import re
    by_name = collections.defaultdict(list)  # (package, simple name) -> [(symbol, file)]
    items = []
    for line in open(f'{d}/defs.tsv'):
        sym, file = line.split('\t', 2)[:2]
        file = os.path.normpath(file)
        if file.startswith('..') or file.startswith('/'):
            continue
        parts = sym.split(' ')
        if len(parts) < 5:
            continue
        pkg, desc = parts[2], ' '.join(parts[4:])
        m = re.search(r'impl#\[([^\]]*)\](?:\[([^\]]*)\])?', desc)
        if m:
            items.append((pkg, m.group(1), m.group(2), file))
        elif desc.endswith('#'):
            name = re.split(r'[/#]', desc.rstrip('#'))[-1]
            by_name[(pkg, name)].append((desc, file))
    out = set()
    for pkg, ty, tr, file in items:
        ty_name = re.sub(r'<.*', '', ty).split('::')[-1]
        local_ty = by_name.get((pkg, ty_name), [])
        if tr is None or local_ty:
            if len(local_ty) == 1 and local_ty[0][1] != file:
                out.add((file, local_ty[0][1], 'impl ' + ty_name))
            elif len(local_ty) > 1:
                # Several types of the name: the one in the impl's own directory, else every one.
                near = [f for _, f in local_ty if os.path.dirname(f) == os.path.dirname(file)] or [f for _, f in local_ty]
                for f in near:
                    if f != file:
                        out.add((file, f, 'impl ' + ty_name))
        else:
            tr_name = re.sub(r'<.*', '', tr).split('::')[-1]
            for _, f in by_name.get((pkg, tr_name), []):
                if f != file:
                    out.add((file, f, f'impl {tr_name} for {ty_name}'))
    with open(f'{d}/ties.tsv', 'w') as o:
        for a_, b_, why in sorted(out):
            o.write(f'{a_}\t{b_}\t{why}\n')
    print(f'{len(items)} impl items, {len(out)} ties')


if __name__ == '__main__':
    if sys.argv[1] == 'join':
        join(sys.argv[2])
    elif sys.argv[1] == 'ties':
        ties(sys.argv[2])
