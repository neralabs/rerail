"""Move items between files by name, byte-exact (spans from `cratify items`, the syn tool in rs/).

usage: move_items.py <moves.tsv> [--check <edges.json> <index dir>...] [--apply <edges.json>]

moves.tsv: one move per line, tab-separated:
    <source file>  <destination file>  <item>; <item>; ...
an item being `kind name` as `cratify items` prints it (`struct Program`, `impl fmt::Display for Term`, `fn join`).
Paths are relative to native/src/code_rule_dsl/. The destination is a child module of the source's module (its
file under the source's directory), so the source declares it and re-exports what moved: no caller changes.

--check: every reference inside a moved span to an item of the source file that stays (it would need the source's
         scope), and to a crate the destination's crate may not read.
--apply: cut the spans, append them to the destination (creating it with the source's `use` lines), and add
         `mod <dest>;` and `pub(in crate::code_rule_dsl) use <dest>::{...};` to the source.
"""
import collections, json, os, re, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
DSL = 'native/src/code_rule_dsl/'
TOOL = os.environ.get('CRATIFY_BIN', 'cratify')

def items(path):
    out = subprocess.run([TOOL, 'items', os.path.join(ROOT, path)], capture_output=True, text=True, check=True).stdout
    rows = []
    for line in out.splitlines():
        f, s, e, kind, name, mod, cfg = line.split('\t')
        rows.append((int(s), int(e), kind, name, mod, cfg))
    return rows

def spec(path):
    for line in open(path):
        line = line.split('#', 1)[0].rstrip('\n')
        if not line.strip():
            continue
        src, dst, what = line.split('\t')
        # `=dst`: the destination is declared by hand and nothing is re-exported (the source keeps stubs)
        bare = dst.strip().startswith('=')
        yield DSL + src.strip(), ('=' if bare else '') + DSL + dst.strip().lstrip('='), [w.strip() for w in what.split(';') if w.strip()]

def pick(src, wanted):
    rows = [r for r in items(src) if not r[4]]  # top level only
    got, missing = [], []
    for w in wanted:
        # `kind name`, or `kind name @macos`: only the item whose cfg names macOS (several impls of one type)
        macos = w.endswith(' @macos')
        kind, name = w.removesuffix(' @macos').split(' ', 1)
        hit = [r for r in rows if r[2] == kind and r[3] in (name, name.replace(' ', '')) and (not macos or 'target_os="macos"' in r[5])]
        if not hit:
            missing.append(w)
        got.extend(hit)
    return sorted(set(got)), missing, rows

def line_of(data):
    """Byte offset of each line's start (`data` is bytes, as the syn tool's offsets are)."""
    starts = [0]
    i = data.find(b'\n')
    while i >= 0:
        starts.append(i + 1)
        i = data.find(b'\n', i + 1)
    return starts

def check(moves, edges_json, indexes):
    sys.path.insert(0, HERE)
    from crate_map import ALLOW, file_crates, crate_of_module, module_of_file
    F, D = file_crates(edges_json)
    defs = {}
    for idx in indexes:
        for row in open(os.path.join(idx, 'defs.tsv')):
            c = row.rstrip('\n').split('\t')
            if c[1] in F and c[0] not in defs and c[10] != '29':
                defs[c[0]] = (c[1], int(c[2]))
    refs = collections.defaultdict(list)
    files = {m[0] for m in moves}
    for idx in indexes:
        for row in open(os.path.join(idx, 'refs.tsv')):
            f, l, c, ec, sym = row.rstrip('\n').split('\t')
            if f in files:
                refs[f].append((int(l), int(c), sym))
    bad = 0
    plan = []
    for src, dst, wanted in moves:
        got, missing, rows = pick(src, wanted)
        for m in missing:
            print(f'MISSING {src}: {m}'); bad += 1
        L = line_of(open(os.path.join(ROOT, src), 'rb').read())
        dst = dst.lstrip('=')
        dcrate = F.get(dst) or crate_of_module(module_of_file(dst, D))
        plan.append((src, dst, got, rows, L, dcrate))
    def new_home(f, line):
        """(destination file, crate) of a definition at (f, line) that a move takes, else None."""
        for src, dst, got, rows, L, dcrate in plan:
            if src == f and any(s <= L[line] < e for s, e, *_ in got):
                return dst, dcrate
        return None
    for src, dst, got, rows, L, dcrate in plan:
        print(f'{src.replace(DSL, "")} -> {dst.replace(DSL, "")} [{dcrate}]: {len(got)} items')
        spans = [(s, e) for s, e, *_ in got]
        inside = lambda b: any(s <= b < e for s, e in spans)
        staying = [(s, e, k, n) for s, e, k, n, *_ in rows if (s, e) not in spans and k not in ('use', 'mod')]
        need = collections.defaultdict(set)
        for l, c, sym in refs[src]:
            if not inside(L[l] + c) or sym not in defs:
                continue
            f, dl = defs[sym]
            home = new_home(f, dl)
            if home and home[0] == dst:
                continue
            if f == src and not home:
                stay = next((f'{k} {n}' for s, e, k, n in staying if s <= L[dl] < e), sym)
                user = next((f'{k} {n}' for s, e, k, n, *_ in got if s <= L[l] + c < e), '?')
                need['stays in ' + src.replace(DSL, '')].add(f'{stay} (from {user})')
                continue
            to = home[1] if home else F.get(f)
            if to and to != dcrate and to not in ALLOW.get(dcrate, ()):
                need[f'{dcrate} may not read {to}'].add(sym.split('code_rule_dsl/')[-1])
        for why, what in sorted(need.items()):
            print(f'  {why}: ' + ', '.join(sorted(what)))
            bad += 1
    return bad

def module_path(f):
    """crate::code_rule_dsl::a::b for native/src/code_rule_dsl/a/b.rs (or a/b/mod.rs)."""
    rel = os.path.splitext(f[len(DSL):])[0]
    if rel.endswith('/mod'):
        rel = rel[:-4]
    return 'crate::code_rule_dsl::' + rel.replace('/', '::')

def absolute(text, mod):
    """`super::` chains in `text` (written in module `mod`) as absolute paths, so the text reads the same anywhere."""
    segs = mod.split('::')
    return re.sub(r'\bsuper::(?:super::)*', lambda m: '::'.join(segs[:len(segs) - m.group(0).count('super::')]) + '::', text)

ITEM = re.compile(r'^(?P<ind>[ \t]*)(?P<vis>pub(?:\s*\([^)]*\))?\s+)?(?P<kw>(?:unsafe\s+|async\s+|const\s+)*(?:fn|struct|enum|type|const|static|trait|mod|union)\b)', re.M)
def widen(item):
    """The item at least `pub(in crate::code_rule_dsl)`: its source re-exports it at that visibility. An impl, or an
    item already as visible, is left as it is."""
    lines = item.split('\n')
    for i, line in enumerate(lines):
        stripped = line.lstrip()
        if not stripped or stripped.startswith(('//', '#[', '#!')):
            continue
        m = ITEM.match(line)
        if m and (m.group('vis') or '').replace(' ', '') in ('', 'pub(super)', 'pub(self)', 'pub(crate)'[:0]):
            lines[i] = line[:m.start('kw')].replace(m.group('vis') or '', '') + 'pub(in crate::code_rule_dsl) ' + line[m.start('kw'):]
        break
    return '\n'.join(lines)

def apply(moves, edges_json):
    sys.path.insert(0, HERE)
    from crate_map import module_of_file
    D = json.load(open(edges_json))
    created_mod = {}
    landed = collections.defaultdict(list)  # destination -> (source module, names)
    for src, dst, wanted in moves:
        bare = dst.startswith('=')
        dst = dst.lstrip('=')
        got, missing, rows = pick(src, wanted)
        if missing:
            sys.exit(f'{src}: no {missing}')
        p = os.path.join(ROOT, src)
        text = open(p, 'rb').read()
        first = min(s for s, *_ in got)
        last = max((e for s, e, k, n, m, c in rows if k in ('use', 'mod') and not m and e <= first), default=None)
        if last:
            at = text.find(b'\n', last) + 1
        else:
            at = 0
            while text.startswith(b'//!', at) or text.startswith(b'\n', at):
                at = text.find(b'\n', at) + 1
        assert at <= first, (src, at, first)
        moved = []
        for s, e, *_ in sorted(got, reverse=True):
            # take the whole lines the item covers, and one blank line after it
            ls = text.rfind(b'\n', 0, s) + 1
            le = text.find(b'\n', e)
            le = len(text) if le < 0 else le + 1
            if text[le:le + 1] == b'\n':
                le += 1
            moved.append(text[ls:le].rstrip(b'\n').decode() + '\n')
            text = text[:ls] + text[le:]
        moved.reverse()
        smod = module_of_file(src, D)
        assert smod, f'no module for {src}'
        moved = [widen(absolute(m, smod)) for m in moved]
        stem = os.path.splitext(os.path.basename(dst))[0]
        child = os.path.dirname(dst) in (src[:-3], os.path.dirname(src) if src.endswith('mod.rs') else None) or bare
        uses = ['use super::*;'] if child else [absolute(text[s:e].decode(), smod) for s, e, k, n, m, c in rows if k == 'use' and not m]
        names = sorted({n.split(' for ')[-1] if k == 'impl' else n for s, e, k, n, m, c in got if k not in ('impl', 'use')})
        dp = os.path.join(ROOT, dst)
        if os.path.exists(dp):
            body = open(dp).read().rstrip('\n') + '\n\n'
        else:
            os.makedirs(os.path.dirname(dp), exist_ok=True)
            body = f'//! Moved from `{os.path.basename(src)}` by the crate split (tools/cratify/moves.tsv).\n\n' + ''.join(u + '\n' for u in uses) + '\n'
        open(dp, 'w').write(body + '\n'.join(moved))
        landed[dst].append((smod, names))
        if bare:
            open(p, 'wb').write(text)
            print(f'{src.replace(DSL, "")} -> {dst.replace(DSL, "")}: {len(got)} items (declared by hand)')
            continue
        if child:
            decl = f'pub(in crate::code_rule_dsl) use {stem}::{{{", ".join(names)}}};\n'
            if f'mod {stem};'.encode() not in text and not created_mod.get(dst):
                decl = f'pub(in crate::code_rule_dsl) mod {stem};\n' + decl
            created_mod[dst] = True
        else:
            # a destination elsewhere: the full module path, from the destination's own directory
            mod = module_of_file(dst, D)
            decl = f'pub(in crate::code_rule_dsl) use {mod}::{{{", ".join(names)}}};\n'
        # after the last top-level `use`/`mod` before the first moved item (computed before the cuts), else after
        # the inner doc comment
        text = text[:at] + decl.encode() + text[at:]
        open(p, 'wb').write(text)
        print(f'{src.replace(DSL, "")} -> {dst.replace(DSL, "")}: {len(got)} items')
    # A path to an item that now lives in the same file is its bare name.
    for dst, groups in landed.items():
        dp = os.path.join(ROOT, dst)
        text = open(dp).read()
        for mod, names in groups:
            for n in names:
                text = text.replace(f'{mod}::{n}', n)
        open(dp, 'w').write(text)

if __name__ == '__main__':
    args = sys.argv[1:]
    moves = list(spec(args[0]))
    if '--check' in args:
        i = args.index('--check')
        rest = [a for a in args[i + 1:] if not a.startswith('--')]
        sys.exit(1 if check(moves, rest[0], rest[1:]) else 0)
    if '--apply' in args:
        apply(moves, args[args.index('--apply') + 1])
