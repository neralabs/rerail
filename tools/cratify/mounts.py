"""The crate split's mounts: every crate's module tree over the files where they are.

usage: mounts.py <tree.tsv> <out dir> [--apply]

<tree.tsv> is `cratify tree native/src/lib.rs` (the syn tool in rs/): every module of the engine as it is now, with
how its parent declares it. Each module goes to the crate crate-map.tsv names (crate_map.py), an inline module to
its file's crate. Then, per crate C:

  - C's lib.rs holds an inline module for every module on the way to one of C's files that C does not own, its
    files mounted where its parent declared them, and what C can read of the rest: a module of a crate C depends
    on is re-exported (`pub use rerail_x::...::m;`), or, where C sees more of it than that crate does, an inline
    module that globs that crate's and adds the rest;
  - in a file C owns, a child another crate owns is re-exported in place of its `mod` (`pub use`), or merged as
    above, when C depends on that crate, and dropped when it does not (that crate mounts it itself).

Writes, under <out dir>: crates/<dir>/src/lib.rs for each crate (the engine's root stays native/src/lib.rs), and
edits.tsv, the declaration edits in the files (file, start, end, replacement). --apply writes both into the tree.
"""
import collections, json, os, sys
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from crate_map import ALLOW, DAG, ROOT, load_map

DSL = 'crate::code_rule_dsl'
ENGINE = 'rerail-engine'
# Where each crate lives; the existing ones keep their directories.
DIRS = {
    'rerail-prof': 'native/crates/prof', 'rerail-rr-syntax': 'native/crates/rr_syntax', 'rerail-store': 'native/crates/store',
    'rerail-lang': 'native/crates/lang', 'rerail-facts': 'native/crates/facts', 'rerail-rr': 'native/crates/rr',
    'rerail-logic': 'native/crates/logic', 'rerail-sessions': 'native/crates/sessions', 'rerail-transducer': 'native/crates/transducer',
    'rerail-w': 'native/crates/w', 'rerail-device-metal': 'native/crates/device_metal',
    'rerail-lang-python': 'native/crates/lang_python', 'rerail-lang-ecma': 'native/crates/lang_ecma', 'rerail-lang-php': 'native/crates/lang_php',
    'rerail-lang-rust': 'native/crates/lang_rust', 'rerail-lang-sql': 'native/crates/lang_sql', 'rerail-lang-json': 'native/crates/lang_json',
}
GEN_BEGIN = '// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin\n'
GEN_END = '// cratify: mounts end\n'


def ident(crate):
    return crate.replace('-', '_')


class Node:
    def __init__(self, path, file, decl_file, start, end, vis, cfg, path_attr, inline):
        self.path, self.file, self.decl_file = path, file, decl_file
        self.decl = (start, end)
        self.vis, self.cfg, self.path_attr, self.inline = vis, cfg, path_attr, inline
        self.children = []
        self.owner = None

    @property
    def name(self):
        return self.path.rsplit('::', 1)[-1]


def load_tree(tsv):
    nodes = {}
    root = Node('crate', os.path.join(ROOT, 'native/src/lib.rs'), None, 0, 0, '', '', '', False)
    nodes['crate'] = root
    for line in open(tsv):
        path, file, decl_file, s, e, vis, cfg, path_attr, kind = line.rstrip('\n').split('\t')
        file, decl_file = os.path.join(ROOT, file), os.path.join(ROOT, decl_file) if decl_file else decl_file
        nodes[path] = Node(path, file, decl_file, int(s), int(e), vis, cfg, path_attr, kind == 'inline')
    for path, n in nodes.items():
        if path != 'crate':
            nodes[path.rsplit('::', 1)[0]].children.append(n)
    return nodes


def owners(nodes):
    """Each module's crate: the map's longest prefix; macOS-only packed modules are Metal's; an inline module is its
    file's; a module the map does not name is its parent's."""
    M = load_map()
    def mapped(path):
        if path == 'crate':
            return ENGINE
        s = path[len(DSL):].lstrip(':') or 'dsl_root' if path.startswith(DSL) else path
        segs = s.split('::')
        for k in range(len(segs), 0, -1):
            key = '::'.join(segs[:k])
            if key in M:
                return M[key]
        return None
    def walk(n, parent_owner):
        if n.inline:
            n.owner = parent_owner
        else:
            o = mapped(n.path)
            if 'target_os="macos"' in n.cfg and n.path.startswith(DSL + '::transducer::packed::'):
                o = 'rerail-device-metal'
            n.owner = o or parent_owner
        for k in n.children:
            walk(k, n.owner if n.inline or not k.inline else n.owner)
    walk(nodes['crate'], ENGINE)


def sees(c):
    return ALLOW.get(c, set()) | {c}


def view(nodes):
    """V[(X, path)]: the modules X's own tree holds under `path` when X owns `path`'s file (its children of crates it
    depends on re-exported, the others dropped)."""
    memo = {}
    def v(x, n):
        key = (x, n.path)
        if key in memo:
            return memo[key]
        out = {n.path}
        for k in n.children:
            if k.owner == x:
                out |= v(x, k)
            elif k.owner in ALLOW.get(x, ()):
                out |= v(k.owner, k)
        memo[key] = out
        return out
    return v


def subtree(n):
    out = [n]
    for k in n.children:
        out += subtree(k)
    return out


def rel(target, base_dir):
    return os.path.relpath(target, base_dir)


def child_dir(n):
    """The directory a plain `mod x;` in module n is looked for in, as the compiler does."""
    if n.path == 'crate':
        return os.path.dirname(n.file)
    if n.inline:
        return None  # the caller tracks inline directories
    base, fname = os.path.split(n.file)
    stem = os.path.splitext(fname)[0]
    mod_rs = fname == 'mod.rs' or bool(n.path_attr)
    return base if mod_rs else os.path.join(base, stem)


def cfg_lines(n, indent):
    return ''.join(f'{indent}{c}\n' for c in split_attrs(n.cfg) if c.startswith('#[cfg('))


def split_attrs(s):
    out, depth, cur = [], 0, ''
    for ch in s:
        cur += ch
        if ch == '[':
            depth += 1
        elif ch == ']':
            depth -= 1
            if depth == 0:
                out.append(cur)
                cur = ''
    return out


class Facades:
    """What crate `c` writes for modules it does not own: inline modules over the original's directories."""

    def __init__(self, c, nodes, v):
        self.c, self.nodes, self.v = c, nodes, v
        self.mine = {p for p, n in nodes.items() if n.owner == c and not n.inline}
        self.want = {p for p, n in nodes.items() if n.owner in sees(c)}

    def need_inline(self, n):
        """Whether `c` writes n as its own inline module: it holds one of c's files, or c sees more under it than
        its owner's tree does."""
        sub = {k.path for k in subtree(n)}
        if sub & self.mine:
            return True
        if n.owner not in ALLOW.get(self.c, ()):
            return bool(sub & self.want)
        return bool((sub & self.want) - self.v(n.owner, n))

    def body(self, n, indent, my_dir, lines):
        """Inside n's inline module: its owner's module globbed (when c depends on it), then each child c sees.
        `my_dir`: where the inline module's plain `mod x;` children are (the original's directory for them)."""
        c = self.c
        if n.owner in ALLOW.get(c, ()):
            lines.append(f'{indent}pub use {ident(n.owner)}::{n.path[len("crate::"):]}::*;\n')
        globbed = self.v(n.owner, n) if n.owner in ALLOW.get(c, ()) else set()
        for k in sorted(n.children, key=lambda k: k.name):
            self.child(k, indent, my_dir, lines, globbed=globbed)

    def child(self, k, indent, my_dir, lines, vis='pub ', globbed=frozenset(), attr_dir=None):
        """`attr_dir`: what k's own `#[path]` is relative to, when not `my_dir` (a declaration at the top of a file:
        the file's directory, which for a non-mod.rs file is not its children's)."""
        c = self.c
        attr_dir = attr_dir or my_dir
        sub = {x.path for x in subtree(k)}
        if not (sub & self.want):
            return
        if k.owner != c and 'cfg(test)' in k.cfg.replace(' ', ''):
            return  # another crate's test module exists only in that crate's own test build
        if k.path in globbed and not self.need_inline(k):
            return  # the parent's glob brings it
        cfg = cfg_lines(k, indent)
        if k.owner == c and not k.inline:
            # c's file, declared as its parent declared it: a plain declaration stays plain (this inline module's
            # directory is the original's); a `#[path]` one is that file again, relative to this directory.
            attr = f'{indent}#[path = "{rel(k.file, attr_dir)}"]\n' if k.path_attr else ''
            lines.append(f'{cfg}{attr}{indent}{vis}mod {k.name};\n')
        elif self.need_inline(k):
            kd = child_dir(k) if not k.inline else os.path.join(my_dir, k.name)
            lines.append(f'{cfg}{indent}#[path = "{rel(kd, attr_dir)}"]\n{indent}{vis}mod {k.name} {{\n')
            self.body(k, indent + '    ', kd, lines)
            lines.append(f'{indent}}}\n')
        elif k.owner in ALLOW.get(c, ()):
            # pub: the module's glob in the crates above carries it
            lines.append(f'{cfg}{indent}pub use {ident(k.owner)}::{k.path[len("crate::"):]};\n')


def emit_crate(c, nodes, v):
    """c's lib.rs mounts (the text between the generated markers)."""
    f = Facades(c, nodes, v)
    lines = [f'extern crate self as {ident(c)};\n']
    lib_dir = os.path.join(ROOT, DIRS[c], 'src')
    for k in sorted(nodes['crate'].children, key=lambda k: k.name):
        if k.owner == c and not k.inline:
            if not k.path_attr and not k.file.endswith('/mod.rs') and any(not x.inline for x in k.children):
                print(f'  {c}: {k.path} is a non-mod-rs file with children, mounted by #[path]: its children move', file=sys.stderr)
            if {x.path for x in subtree(k)} & f.want:
                lines.append(f'{cfg_lines(k, "")}#[path = "{rel(k.file, lib_dir)}"]\npub mod {k.name};\n')
            continue
        if f.need_inline(k):
            kd = child_dir(k)
            lines.append(f'{cfg_lines(k, "")}#[path = "{rel(kd, lib_dir)}"]\npub mod {k.name} {{\n')
            f.body(k, '    ', kd, lines)
            lines.append('}\n')
        else:
            f.child(k, '', lib_dir, lines)
    return ''.join(lines)


def file_edits(nodes, v):
    """The declaration edits in owned files: per child of another crate, a re-export, a merged inline module, or
    nothing."""
    edits = []
    facades = {}
    for p, n in nodes.items():
        for k in n.children:
            if k.inline or k.owner == n.owner:
                continue
            x = n.owner
            text = open(k.decl_file, 'rb').read()[k.decl[0]:k.decl[1]].decode()
            if k.owner in ALLOW.get(x, ()) or x == ENGINE:
                f = facades.setdefault(x, Facades(x, nodes, v))
                lines = []
                # the parent's directory for its plain children (the declaration is in the parent's file)
                pdir = child_dir(n) if not n.inline else None
                # outside the engine a module is pub: the crates above path through it
                vis = (k.vis + ' ') if k.vis and x == ENGINE else '' if x == ENGINE and not k.vis else 'pub '
                f.child(k, '', pdir, lines, vis=vis, attr_dir=os.path.dirname(k.decl_file))
                new = ''.join(lines).rstrip('\n') or f'// cratify: `{k.name}` holds nothing {x} reads'
            else:
                new = f'// cratify: `{k.name}` is {k.owner}\'s (it mounts it)'
            edits.append((k.decl_file, k.decl[0], k.decl[1], text, new))
    return edits


def main(tsv, out, *flags):
    nodes = load_tree(tsv)
    owners(nodes)
    v = view(nodes)
    os.makedirs(out, exist_ok=True)
    crates = sorted({n.owner for n in nodes.values()} - {ENGINE})
    for c in crates:
        body = emit_crate(c, nodes, v)
        d = os.path.join(out, DIRS[c], 'src')
        os.makedirs(d, exist_ok=True)
        open(os.path.join(d, 'lib.rs'), 'w').write(GEN_BEGIN + body + GEN_END)
        print(f'{c:22} {sum(1 for l in body.splitlines() if l.strip().startswith("pub mod ") and not l.endswith("{")):4} files, {body.count("pub use "):4} re-exports, {body.count("{"):4} inline')
    edits = file_edits(nodes, v)
    with open(os.path.join(out, 'edits.tsv'), 'w') as w:
        for f, s, e, old, new in sorted(edits):
            w.write(f'{os.path.relpath(f, ROOT)}\t{s}\t{e}\t{json.dumps(old)}\t{json.dumps(new)}\n')
    kinds = collections.Counter('drop' if n.startswith('// cratify') else 'merge' if '{' in n else 're-export' for *_, n in edits)
    print(f'declaration edits: {len(edits)} ({dict(kinds)})')
    stats = collections.Counter(n.owner for n in nodes.values() if not n.inline)
    print('files per crate:', dict(sorted(stats.items(), key=lambda kv: -kv[1])))


if __name__ == '__main__':
    main(*sys.argv[1:])
