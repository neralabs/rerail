"""The crate split, applied: every crate's mounts and manifest, and the edits in the files where they are.

usage: apply.py <tree.tsv> <join out dir> [--write]

<tree.tsv> is `cratify tree native/src/lib.rs`; <join out dir> holds join.py's reads.tsv (with test-hosts.tsv in
place when it ran). Without --write, prints what it would do and writes nothing.

What it writes:
  - each crate's src/lib.rs: mounts.py's tree; an existing crate's own lib.rs keeps its text, the mounts between
    the generated markers;
  - each crate's Cargo.toml (an existing crate's gains the dependencies and features it lacks), the workspace's
    members, and the engine's dependencies on the crates;
  - in the files: the parents' declarations of children of other crates (mounts.py), every module declaration of a
    crate other than the engine `pub` (the crates above path through them), the `test` atom of every cfg predicate
    `any(test, feature = "test-hooks")` but on a test module no other crate reads, and `env!("CARGO_MANIFEST_DIR")`
    as `native/` in the crates under native/crates/ (two directories down).
Items another crate reads are widen.py's.
"""
import collections, json, os, re, subprocess, sys, tomllib
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import mounts
from crate_map import ALLOW, DAG, ROOT
from mounts import DIRS, DSL, ENGINE, GEN_BEGIN, GEN_END, ident, sees, subtree

NATIVE = os.path.join(ROOT, 'native')
TOOL = os.environ.get('CRATIFY_BIN', 'cratify')
EXISTING = {'rerail-prof', 'rerail-rr-syntax', 'rerail-store'}
HOOKS = 'any(test, feature = "test-hooks")'
DESCRIPTIONS = {
    'rerail-lang': 'The language registry and what every crate reads of a language: tapes, trees, lines, JSON, budgets.',
    'rerail-facts': "The engine's facts: the virtual tables, text, strings, producers, SQL profiles.",
    'rerail-rr': 'The .rr front end: parsing to programs, magic sets, lowering, printing, checking.',
    'rerail-logic': 'The Datalog engine: plans, the native executor, SQL, typing, the waves core, the solver device interface.',
    'rerail-sessions': "The SQL executor's sessions.",
    'rerail-w': "W: the matcher, scopes, values, suppressions, demand and the workspace's model.",
    'rerail-transducer': "The transducer: the CPU interpreter, Cranelift, Bitlex, the batch ABI, the matcher device interface.",
    'rerail-device-metal': 'Metal, as the logic\'s and the transducer\'s device (macOS only; registered by the engine).',
    'rerail-lang-python': 'Python: its facts, scopes, values and suppressions.',
    'rerail-lang-ecma': 'JavaScript, TypeScript and Vue: manifests, imports, values.',
    'rerail-lang-php': 'PHP and Blade: scopes, calls, values, the Forte parser.',
    'rerail-lang-rust': 'Rust: format arguments, values, suppressions.',
    'rerail-lang-sql': 'SQL: scopes, values, suppressions.',
    'rerail-lang-json': 'JSON: its Bitlex parser.',
}


# --- file edits -------------------------------------------------------------------------------------------------

class Edits:
    """Byte edits per file, refused when two overlap."""

    def __init__(self):
        self.by_file = collections.defaultdict(list)
        self.why = collections.Counter()

    def add(self, f, s, e, new, why):
        self.by_file[f].append((s, e, new, why))
        self.why[why] += 1

    def apply(self, write):
        bad = []
        for f, es in sorted(self.by_file.items()):
            es.sort(key=lambda x: (x[0], x[1]))
            # an edit inside a whole-declaration replacement is the replacement's (it carries the declaration anew)
            kept = []
            for x in es:
                if kept and x[0] < kept[-1][1]:
                    outer = kept[-1]
                    if outer[0] <= x[0] and x[1] <= outer[1] and outer[3] == 'declaration':
                        continue
                    if x[0] <= outer[0] and outer[1] <= x[1] and x[3] == 'declaration':
                        kept[-1] = x
                        continue
                    bad.append(f'{os.path.relpath(f, ROOT)}: {outer[3]} {outer[:2]} overlaps {x[3]} {x[:2]}')
                    continue
                kept.append(x)
            if write:
                data = open(f, 'rb').read()
                for s, e, new, why in reversed(kept):
                    data = data[:s] + new.encode() + data[e:]
                open(f, 'wb').write(data)
        return bad


def strip_strings(s):
    return re.sub(r'"(?:[^"\\]|\\.)*"', '""', s)


def test_gated(cfg):
    return bool(re.search(r'\btest\b', strip_strings(cfg)))


def read_modules(nodes, join_out):
    """Modules holding an item another crate reads (reads.tsv), each with its ancestors."""
    by_file = {}
    for p, n in nodes.items():
        if not n.inline:
            by_file[n.file] = p
    out = set()
    for line in list(open(os.path.join(join_out, 'reads.tsv')))[1:]:
        f, l, kind, item, users = line.rstrip('\n').split('\t')
        p = by_file.get(os.path.join(ROOT, f))
        if p is None:
            continue
        owner = nodes[p].owner
        if any(u != owner for u in users.split(',')):
            while p != 'crate':
                out.add(p)
                p = p.rsplit('::', 1)[0]
    return out


def module_vis_edits(nodes, edits):
    """Every module declaration a crate other than the engine keeps (its parent's own): `pub`."""
    for p, n in nodes.items():
        if p == 'crate' or not n.decl_file:
            continue
        parent = nodes[p.rsplit('::', 1)[0]]
        if parent.owner == ENGINE or n.owner != parent.owner:
            continue
        data = open(n.decl_file, 'rb').read()
        text = data[n.decl[0]:n.decl[1]].decode()
        m = re.search(r'^([ \t]*)(pub(\s*\([^)]*\))?\s+)?mod\s+' + re.escape(n.name) + r'\b', text, re.M)
        if not m:
            print(f'NO DECL {p}', file=sys.stderr)
            continue
        if (m.group(2) or '').strip() == 'pub':
            continue
        s = n.decl[0] + len(text[:m.start(2) if m.group(2) else m.end(1)].encode())
        e = n.decl[0] + len(text[:m.end(2)].encode()) if m.group(2) else s
        edits.add(n.decl_file, s, e, 'pub ', 'module pub')


def cfg_test_edits(nodes, files, keep_test_mods, edits):
    """The `test` atom of every cfg predicate: `any(test, feature = "test-hooks")`, but on a test module no other
    crate reads (it stays its own crate's test build's)."""
    out = subprocess.run([TOOL, 'cfgtest', *files], capture_output=True, text=True, check=True).stdout
    by_decl = {}
    for p, n in nodes.items():
        if n.decl_file:
            by_decl[(n.decl_file, n.name)] = p
    for line in out.splitlines():
        f, s, e, a, b, gated = line.split('\t')
        data = open(f, 'rb').read()
        if b'test-hooks' in data[int(a):int(b)]:
            continue  # done already
        if gated.startswith('mod '):
            p = by_decl.get((os.path.abspath(f), gated[4:]))
            if p is None or p not in keep_test_mods:
                continue
        edits.add(os.path.abspath(f), int(s), int(e), HOOKS, 'cfg test')


def use_vis_edits(nodes, edits):
    """Every restricted re-export (`pub(crate) use`, `pub(in ...) use`) in a crate other than the engine: `pub`. Other
    crates path through them, and the index names the definition, not the re-export, so widen.py cannot see them."""
    for p, n in nodes.items():
        if n.inline or n.owner == ENGINE:
            continue
        out = subprocess.run([TOOL, 'items', n.file], capture_output=True, text=True, check=True).stdout
        data = open(n.file, 'rb').read()
        for line in out.splitlines():
            f, s, e, k, name, mod, cfg = line.split('\t')
            if k != 'use':
                continue
            text = data[int(s):int(e)].decode()
            m = re.search(r'^[ \t]*(pub\s*\([^)]*\))\s+use\b', text, re.M)
            if m:
                a = int(s) + len(text[:m.start(1)].encode())
                edits.add(n.file, a, a + len(m.group(1).encode()), 'pub', 'use pub')


def hooks(text, modules_too, keep=()):
    """The `test` atoms of the cfg lines of generated text, as cfg_test_edits makes them in files."""
    out = []
    lines = text.split('\n')
    for i, l in enumerate(lines):
        if l.lstrip().startswith('#[cfg(') and 'test-hooks' not in l and re.search(r'\btest\b', strip_strings(l)):
            nxt = next((x for x in lines[i + 1:] if not x.lstrip().startswith('#[')), '')
            m = re.match(r'\s*(pub(\([^)]*\))?\s+)?mod\s+([A-Za-z_0-9]+)', nxt)
            if m is None or modules_too or m.group(3) in keep:
                l = re.sub(r'\btest\b(?![^"]*"\))', HOOKS, l)
        out.append(l)
    return '\n'.join(out)


def manifest_dir_edits(nodes, edits):
    """`env!("CARGO_MANIFEST_DIR")` in a crate under native/crates/: `native/`, as it was in the engine."""
    tok = b'env!("CARGO_MANIFEST_DIR")'
    for p, n in nodes.items():
        if n.inline or n.owner == ENGINE:
            continue
        data = open(n.file, 'rb').read()
        i = data.find(tok)
        while i >= 0:
            if not data[max(0, i - 8):i].endswith(b'concat!('):
                edits.add(n.file, i, i + len(tok), 'concat!(env!("CARGO_MANIFEST_DIR"), "/../..")', 'manifest dir')
            i = data.find(tok, i + 1)


# --- facades: the imports of the modules they stand for ----------------------------------------------------------

_items = {}


def file_uses(path):
    """The file's top-level `use` items (text), as `cratify items` gives them."""
    if path not in _items:
        out = subprocess.run([TOOL, 'items', path], capture_output=True, text=True, check=True).stdout
        data = open(path, 'rb').read()
        _items[path] = [data[int(s):int(e)].decode() for f, s, e, k, n, m, c in (l.split('\t') for l in out.splitlines()) if k == 'use' and not m]
    return _items[path]


def absolute_use(text, mod, nodes):
    """`text` (a use written in module `mod`) with `super::`/`self::` and child-relative paths absolute, or None
    when it is a path the facade cannot repeat."""
    segs = mod.split('::')
    text = re.sub(r'\bsuper::(?:super::)*', lambda m: '::'.join(segs[:len(segs) - m.group(0).count('super::')]) + '::', text)
    text = re.sub(r'\bself::', mod + '::', text)
    m = re.match(r'((?:#\[[^\]]*\]\s*)*)((?:pub(?:\s*\([^)]*\))?\s+)?use\s+)(::)?([A-Za-z_][A-Za-z_0-9]*)', text, re.S)
    if not m:
        return None
    first = m.group(4)
    if m.group(3) is None and first not in ('crate', 'std', 'core', 'alloc') and f'{mod}::{first}' in nodes:
        text = text[:m.start(4)] + f'{mod}::' + text[m.start(4):]
    return text


def use_owner(text, nodes):
    """The crate a `use crate::...` path's deepest known module belongs to (None: an external path)."""
    m = re.search(r'use\s+(crate(?:::[A-Za-z_][A-Za-z_0-9]*)*)', text)
    if not m:
        return None
    segs = m.group(1).split('::')
    for k in range(len(segs), 0, -1):
        p = '::'.join(segs[:k])
        if p in nodes:
            return nodes[p].owner
    return None


class Facades(mounts.Facades):
    def body(self, n, indent, my_dir, lines):
        super().body(n, indent, my_dir, lines)
        if n.inline or n.path == 'crate':
            return
        for u in file_uses(n.file):
            a = absolute_use(u, n.path, self.nodes)
            if a is None:
                continue
            o = use_owner(a, self.nodes)
            if o is not None and o not in sees(self.c):
                continue
            body = '\n'.join(indent + l for l in a.strip().split('\n'))
            lines.append(f'{indent}#[allow(unused_imports)]\n{body}\n')


mounts.Facades = Facades


def root_reexports(c):
    """The engine root's re-exports of the crates below it (`pub use rerail_store::content_store;`) that c sees."""
    out = []
    for line in open(os.path.join(NATIVE, 'src/lib.rs')):
        m = re.match(r'pub use (rerail_[a-z_]+)::', line)
        if m and m.group(1).replace('_', '-') in sees(c) | {'rerail-store'} & sees(c):
            out.append(line)
    return ''.join(out)


# --- manifests ---------------------------------------------------------------------------------------------------

def toml_value(v):
    if isinstance(v, bool):
        return 'true' if v else 'false'
    if isinstance(v, str):
        return json.dumps(v)
    if isinstance(v, list):
        return '[' + ', '.join(toml_value(x) for x in v) + ']'
    if isinstance(v, dict):
        return '{ ' + ', '.join(f'{k} = {toml_value(x)}' for k, x in v.items()) + ' }'
    return str(v)


def rebase(dep, crate_dir):
    if isinstance(dep, dict) and 'path' in dep:
        dep = dict(dep)
        dep['path'] = os.path.relpath(os.path.join(NATIVE, dep['path']), crate_dir)
    return dep


def direct(c):
    return [d for d in DAG.get(c, []) if d != c]


def manifest(c, engine):
    d = os.path.join(ROOT, DIRS[c])
    feats = dict(engine['features'])
    lines = ['# Generated by tools/cratify/apply.py (the crate split): its sources are where they were, mounted in place',
             '# (src/lib.rs).', '[package]', f'name = "{c}"', 'version = "0.1.0"', 'edition = "2024"', 'publish = false',
             f'description = {json.dumps(DESCRIPTIONS.get(c, ""))}', '']
    if c == 'rerail-lang':
        lines[-1:-1] = ['build = "build.rs"']
    lines += ['[features]']
    for f, body in feats.items():
        fwd = [] if f == 'default' else [f'{ident(x).replace("_", "-")}/{f}' for x in direct(c) if x not in EXISTING]
        lines.append(f'{f} = {toml_value(list(body) + fwd)}')
    if 'test-hooks' not in feats:
        lines.append(f'test-hooks = {toml_value([f"{x}/test-hooks" for x in direct(c) if x != "rerail-prof"])}')
    lines += ['', '[dependencies]']
    for k, v in engine['dependencies'].items():
        if k.startswith('rerail-'):
            continue
        lines.append(f'{k} = {toml_value(rebase(v, d))}')
    for x in direct(c):
        dep = {'path': os.path.relpath(os.path.join(ROOT, DIRS[x]), d)}
        if x not in EXISTING:
            dep['default-features'] = False
        lines.append(f'{x} = {toml_value(dep)}')
    for k, v in engine['dependencies'].items():
        if k.startswith('rerail-') and k not in DIRS and k != 'rerail-metal':
            lines.append(f'{k} = {toml_value(rebase(v, d))}')
    for target, t in engine.get('target', {}).items():
        for sect in ('dependencies', 'dev-dependencies'):
            if sect in t:
                lines += ['', f"[target.'{target}'.{sect}]"]
                for k, v in t[sect].items():
                    lines.append(f'{k} = {toml_value(rebase(v, d))}')
    lines += ['', '[dev-dependencies]']
    for k, v in engine.get('dev-dependencies', {}).items():
        if k.startswith('rerail-') and k in DIRS:
            continue
        lines.append(f'{k} = {toml_value(rebase(v, d))}')
    if c == 'rerail-lang':
        lines += ['', '[build-dependencies]', 'serde = { version = "1.0", features = ["derive"] }', 'serde_json = "1.0"']
    if 'lints' in engine:
        lines += ['', '[lints]', 'workspace = true'] if engine['lints'].get('workspace') else []
    return '\n'.join(lines) + '\n'


def merge_manifest(path, c, engine):
    """An existing crate's own Cargo.toml with what its mounted files need: the engine's dependencies it lacks (per
    table, after the table's header, or as a new table), the crates it now stands on, and the engine's features."""
    text = open(path).read()
    have = tomllib.loads(text)
    d = os.path.dirname(path)
    tables = {'dependencies': {k: rebase(v, d) for k, v in engine['dependencies'].items() if not k.startswith('rerail-') or (k not in DIRS and k != 'rerail-metal')}}
    for x in direct(c):
        dep = {'path': os.path.relpath(os.path.join(ROOT, DIRS[x]), d)}
        if x not in EXISTING:
            dep['default-features'] = False
        tables['dependencies'][x] = dep
    for target, t in engine.get('target', {}).items():
        if 'dependencies' in t:
            tables[f"target.'{target}'.dependencies"] = {k: rebase(v, d) for k, v in t['dependencies'].items() if k != 'rerail-metal'}
    def existing(name):
        node = have
        for part in re.findall(r"'[^']*'|[^.]+", name):
            node = node.get(part.strip("'"), {})
        return node
    for name, deps in tables.items():
        lacking = {k: v for k, v in deps.items() if k not in existing(name) and k != c}
        if not lacking:
            continue
        block = ''.join(f'{k} = {toml_value(v)}\n' for k, v in lacking.items())
        header = f'[{name}]\n'
        if header in text:
            i = text.index(header) + len(header)
            text = text[:i] + block + text[i:]
        else:
            text = text.rstrip('\n') + f'\n\n{header}' + block
    feats = have.get('features', {})
    lacking = {f: body for f, body in engine['features'].items() if f not in feats}
    if lacking:
        block = ''.join(f'{f} = {toml_value(list(body) if f == "default" else [])}\n' for f, body in lacking.items())
        if '[features]\n' in text:
            i = text.index('[features]\n') + len('[features]\n')
            text = text[:i] + block + text[i:]
        else:
            i = text.index('\n[', text.index('[package]') + 1) + 1
            text = text[:i] + '[features]\n' + block + '\n' + text[i:]
    return text


def engine_manifest_edits(engine_text, crates):
    """The engine's dependencies on the crates (default features off, its features forwarded) and its own
    dev-dependency with test-hooks (every test build turns them on)."""
    add = ['', '# The engine\'s crates (tools/cratify/apply.py): its sources, mounted in place.']
    for c in crates:
        if c in EXISTING or c == 'rerail-device-metal':
            continue
        add.append(f'{c} = {{ path = "{os.path.relpath(os.path.join(ROOT, DIRS[c]), NATIVE)}", default-features = false }}')
    text = engine_text
    marker = '\n[target.'
    i = text.index(marker)
    text = text[:i] + '\n'.join(add) + '\n' + text[i:]
    old = "[target.'cfg(target_os = \"macos\")'.dependencies]\n"
    text = text.replace(old, old + f'rerail-device-metal = {{ path = "crates/device_metal", default-features = false }}\n', 1)
    old = '[dev-dependencies]\n'
    text = text.replace(old, old + '# Every test build of the engine turns on its crates\' test hooks (`test-hooks`).\nrerail-engine = { path = ".", features = ["test-hooks"] }\n', 1)
    feats = tomllib.loads(engine_text)['features']
    def fwd(m):
        name, body = m.group(1), m.group(2)
        if name == 'default':
            return m.group(0)
        extra = [f'{c}/{name}' for c in crates if c not in EXISTING]
        if name == 'test-hooks':
            extra += ['rerail-store/test-hooks']
        items = [x.strip() for x in body.split(',') if x.strip()] + [json.dumps(x) for x in extra]
        return f'{name} = [{", ".join(items)}]'
    a = text.index('[features]')
    b = text.index('\n[', a + 1)
    sect = re.sub(r'^([a-z0-9-]+) = \[([^\]]*)\]', fwd, text[a:b], flags=re.M)
    return text[:a] + sect + text[b:]


# --- the run -----------------------------------------------------------------------------------------------------

def main(tree_tsv, join_out, *flags):
    write = '--write' in flags
    nodes = mounts.load_tree(tree_tsv)
    mounts.owners(nodes)
    v = mounts.view(nodes)
    crates = sorted({n.owner for n in nodes.values()} - {ENGINE})
    keep = read_modules(nodes, join_out)
    engine_text = open(os.path.join(NATIVE, 'Cargo.toml')).read()
    engine = tomllib.loads(engine_text)
    # lib.rs and Cargo.toml per crate
    for c in crates:
        body = root_reexports(c) + mounts.emit_crate(c, nodes, v)
        body = hooks(body, False, {nodes[p].name for p in keep if nodes[p].owner == c})
        d = os.path.join(ROOT, DIRS[c])
        lib = os.path.join(d, 'src', 'lib.rs')
        if c in EXISTING:
            own = open(lib).read()
            if GEN_BEGIN in own:
                own = own[:own.index(GEN_BEGIN)] + own[own.index(GEN_END) + len(GEN_END):]
            text = own.rstrip('\n') + '\n\n' + GEN_BEGIN + body + GEN_END
        else:
            doc = f'//! {DESCRIPTIONS.get(c, c)}\n//!\n//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.\n'
            text = doc + '#![allow(dead_code, unused_imports)]\n' + GEN_BEGIN + body + GEN_END
        if write:
            os.makedirs(os.path.dirname(lib), exist_ok=True)
            open(lib, 'w').write(text)
            mf = os.path.join(d, 'Cargo.toml')
            open(mf, 'w').write(manifest(c, engine) if c not in EXISTING else merge_manifest(mf, c, engine))
        print(f'{c:22} lib.rs {text.count(chr(10)):5} lines')
    if write:
        open(os.path.join(NATIVE, 'Cargo.toml'), 'w').write(engine_manifest_edits(engine_text, crates))
        ws = os.path.join(ROOT, 'Cargo.toml')
        t = open(ws).read()
        m = re.search(r'members = \[([^\]]*)\]', t)
        have = {x.strip().strip('"') for x in m.group(1).split(',') if x.strip()}
        new = [DIRS[c] for c in crates if DIRS[c] not in have]
        t = t[:m.end(1)] + ''.join(f', "{x}"' for x in new) + t[m.end(1):]
        open(ws, 'w').write(t)
    # the edits in the files
    edits = Edits()
    for f, s, e, old, new in mounts.file_edits(nodes, v):
        edits.add(f, s, e, hooks(new, False, {nodes[p].name for p in keep}), 'declaration')
    module_vis_edits(nodes, edits)
    use_vis_edits(nodes, edits)
    files = sorted({n.file for n in nodes.values() if not n.inline})
    cfg_test_edits(nodes, files, keep, edits)
    manifest_dir_edits(nodes, edits)
    bad = edits.apply(write)
    print('edits:', dict(edits.why))
    for b in bad:
        print('OVERLAP', b)
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(*sys.argv[1:]))
