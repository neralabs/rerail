"""The crate split's last step: each crate's sources move from native/src into the crate's own src/.

usage (from anywhere in the repository):
  move_sources.py plan [PLAN.tsv]  the moves (default tools/cratify/source-moves.tsv): every file under native/src
                                   that a crate other than the engine compiles, to native/crates/<dir>/src/<its path
                                   under native/src>; and, beside the plan (PLAN.sites.json, not kept), every path
                                   the module trees read, as they are before the move
  move_sources.py move [PLAN.tsv]  `git mv` each file of the plan: the moves alone, no file's text changed
  move_sources.py repath [PLAN.tsv]
                                   after `move`: each `#[path]` and `include!`/`include_str!`/`include_bytes!` that
                                   the move changed written again, and a `#[path]` added where a module's file is now
                                   in another crate than its parent's
  move_sources.py where FILE...    for a file another branch adds under native/src: the crates that compile the
                                   modules of its directory, where it belongs

A crate's modules keep their layout: a file at native/src/P moves to <crate>/src/P, so every module path, and every
relative path between two files of one crate, is what it was. What changes is a path between two crates, or from a
crate's root (lib.rs, which mounted native/src as `../../../src`), and those are what `repath` writes again.

Which crate compiles a file is read from the module trees, as rustc resolves them: a `#[path]` (or
`#[cfg_attr(.., path = ..)]`, where the default holds too) is relative to its file's directory at the file's top level
and to its inline module's directory inside one; a non-mod-rs file's own modules sit under its name, and a file loaded
through `#[path]` is a mod-rs file. A file two crates compile goes to the lower one (tools/cratify/crate-order.tsv),
and the other reads it there by path. A file the engine (native/Cargo.toml) compiles stays.

A file no module tree reaches (one that is not Rust, or a file of Rust `include!`d) goes with the crate whose sources
include it, or name its path under native/src (or its directory's), when that is one crate; otherwise it stays, and
the plan says so. A file no source names goes with its directory's files, when they all went to one crate. The
directories read whole (WHOLE: the typing programs, the planner's goldens) stay where they are.
"""
import collections, glob, json, os, re, subprocess, sys, tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
NATIVE = os.path.join(ROOT, 'native')
SRC = os.path.join(NATIVE, 'src')
PLAN = os.path.join(ROOT, 'tools/cratify/source-moves.tsv')
ENGINE = 'rerail-engine'
# Directories under native/src read whole, which never split (`plan`).
WHOLE = ('code_rule_dsl/typing/programs', 'code_rule_dsl/logic/plan/golden')

TOK = re.compile(r'''
    (?P<ws>\s+)
  | (?P<lc>//[^\n]*)
  | (?P<bc>/\*)
  | (?P<rs>r(?P<h>\#*)")
  | (?P<bs>b?"(?:\\.|[^"\\])*")
  | (?P<ch>b?'(?:\\.|[^'\\])'(?!'))
  | (?P<lt>'[A-Za-z_][A-Za-z0-9_]*)
  | (?P<id>[A-Za-z_][A-Za-z0-9_]*)
  | (?P<p>.)
''', re.S | re.X)


def tokens(src):
    """(kind, text, start, end): i an identifier, s a string (text: its contents), p punctuation, c a char."""
    i, n, out = 0, len(src), []
    while i < n:
        m = TOK.match(src, i)
        if m.group('bc'):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith('/*', j):
                    depth += 1
                    j += 2
                elif src.startswith('*/', j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            i = j
            continue
        if m.group('rs'):
            close = '"' + m.group('h')
            j = src.find(close, m.end())
            out.append(('s', src[m.end():j], m.end(), j))
            i = j + len(close)
            continue
        s, i = i, m.end()
        if m.group('ws') or m.group('lc'):
            continue
        if m.group('bs'):
            b = m.group('bs')
            q = s + b.index('"') + 1
            out.append(('s', b[b.index('"') + 1:-1], q, i - 1))
        elif m.group('ch') or m.group('lt'):
            out.append(('c', m.group(0), s, i))
        elif m.group('id'):
            out.append(('i', m.group('id'), s, i))
        else:
            out.append(('p', m.group('p'), s, i))
    return out


def attr_paths(attr):
    """Each `path = "x"` of an attribute: (string token, whether a cfg_attr's)."""
    cond = bool(attr) and attr[0][:2] == ('i', 'cfg_attr')
    return [(attr[k + 2], cond) for k in range(len(attr) - 2)
            if attr[k][:2] == ('i', 'path') and attr[k + 1][:2] == ('p', '=') and attr[k + 2][0] == 's']


class Walk:
    """A crate's files from its roots, with the paths they read.

    sites: [file, start, end, text, base, kind]: a path string at [start, end) of `file`, relative to `base`;
      kind `mod` (a file module's #[path]), `dir` (an inline module's), `include`.
    defaults: [file, offset, name, moddir, base, found]: a `mod name;` that rustc finds beside its parent (`found`),
      with no #[path] that always holds; offset is its line's start, and a #[path] there would be relative to `base`.
    """

    def __init__(self):
        self.files, self.sites, self.defaults, self.seen = [], [], [], set()

    def file(self, path, modrs):
        key = os.path.realpath(path)
        if key in self.seen or not os.path.exists(key):
            return
        self.seen.add(key)
        self.files.append(key)
        src = open(key, encoding='utf-8', errors='replace').read()
        fdir = os.path.dirname(key)
        own = fdir if modrs else os.path.join(fdir, os.path.splitext(os.path.basename(key))[0])
        self.items(key, src, tokens(src), 0, None, fdir, own, True)

    def items(self, f, src, toks, i, end, fdir, moddir, top):
        end = len(toks) if end is None else end
        attrs = []
        while i < end:
            t = toks[i]
            if t[:2] == ('p', '#'):
                j = i + 1
                inner = j < end and toks[j][:2] == ('p', '!')
                if inner:
                    j += 1
                if j < end and toks[j][:2] == ('p', '['):
                    d, k = 0, j
                    while k < end:
                        if toks[k][:2] == ('p', '['):
                            d += 1
                        elif toks[k][:2] == ('p', ']'):
                            d -= 1
                            if d == 0:
                                break
                        k += 1
                    if not inner:
                        attrs.append(toks[j + 1:k])
                    i = k + 1
                    continue
            if t[:2] == ('i', 'mod') and i + 2 < end and toks[i + 1][0] == 'i':
                name = toks[i + 1][1]
                nxt = toks[i + 2]
                paths = [p for a in attrs for p in attr_paths(a)]
                always = any(not cond for _, cond in paths)
                base = fdir if top else moddir
                attrs = []
                if nxt[:2] == ('p', ';'):
                    for tok, _ in paths:
                        self.sites.append([f, tok[2], tok[3], tok[1], base, 'mod'])
                        self.file(os.path.normpath(os.path.join(base, tok[1])), True)
                    if not always:
                        a, b = os.path.join(moddir, name + '.rs'), os.path.join(moddir, name, 'mod.rs')
                        found = a if os.path.exists(a) else b if os.path.exists(b) else None
                        if found:
                            self.defaults.append([f, src.rfind('\n', 0, t[2]) + 1, name, moddir, base, os.path.realpath(found)])
                            self.file(found, found == b)
                    i += 3
                    continue
                if nxt[:2] == ('p', '{'):
                    d, k = 0, i + 2
                    while k < end:
                        if toks[k][:2] == ('p', '{'):
                            d += 1
                        elif toks[k][:2] == ('p', '}'):
                            d -= 1
                            if d == 0:
                                break
                        k += 1
                    subs = []
                    for tok, _ in paths:
                        self.sites.append([f, tok[2], tok[3], tok[1], base, 'dir'])
                        subs.append(os.path.normpath(os.path.join(base, tok[1])))
                    if not always:
                        subs.append(os.path.join(moddir, name))
                    for sub in dict.fromkeys(subs):
                        self.items(f, src, toks, i + 3, k, fdir, sub, False)
                    i = k + 1
                    continue
            if t[0] == 'i' and t[1] in ('include', 'include_str', 'include_bytes') and i + 3 < end \
                    and toks[i + 1][:2] == ('p', '!') and toks[i + 2][:2] == ('p', '(') and toks[i + 3][0] == 's':
                s = toks[i + 3]
                self.sites.append([f, s[2], s[3], s[1], fdir, 'include'])
            if t[:2] in (('p', ';'), ('p', '}'), ('p', '{')):
                attrs = []
            i += 1


def crates():
    """(name, dir, roots): each package of the workspace under native/, with its targets' root files."""
    out = []
    for m in [os.path.join(NATIVE, 'Cargo.toml')] + sorted(glob.glob(os.path.join(NATIVE, 'crates/*/Cargo.toml'))):
        d = os.path.dirname(m)
        t = tomllib.load(open(m, 'rb'))
        roots = [os.path.join(d, t.get('lib', {}).get('path', 'src/lib.rs'))]
        b = t['package'].get('build', 'build.rs')
        if isinstance(b, str):
            roots.append(os.path.join(d, b))
        for kind in ('bin', 'example', 'test', 'bench'):
            roots += [os.path.join(d, e['path']) for e in t.get(kind, []) if 'path' in e]
        for pat in ('src/main.rs', 'src/bin/*.rs', 'tests/*.rs', 'benches/*.rs', 'examples/*.rs'):
            roots += glob.glob(os.path.join(d, pat))
        out.append((t['package']['name'], d, [os.path.realpath(r) for r in dict.fromkeys(roots) if os.path.exists(r)]))
    return out


def walks():
    out = {}
    for name, d, roots in crates():
        w = Walk()
        for r in roots:
            w.file(r, True)
        out[name] = (d, w)
    return out


def rel(p):
    return os.path.relpath(p, ROOT)


def under(p, d):
    return p == d or p.startswith(d + os.sep)


def plan(path):
    ws = walks()
    rows = [l.split('\t')[0] for l in open(os.path.join(ROOT, 'tools/cratify/crate-order.tsv')) if l.strip() and not l.startswith('#')]
    rank = {n: i for i, n in enumerate(rows)}
    owners = collections.defaultdict(set)
    for name, (d, w) in ws.items():
        for f in w.files:
            if under(f, SRC):
                owners[f].add(name)
    moves, multi = {}, {}
    for f, cs in owners.items():
        if ENGINE in cs:
            continue
        home = min(cs, key=lambda c: (rank.get(c, 10 ** 6), c))
        moves[f] = os.path.join(ws[home][0], 'src', os.path.relpath(f, SRC))
        if len(cs) > 1:
            multi[f] = sorted(cs)
    # Files that are not Rust: with the one crate whose sources include them or name them.
    tracked = subprocess.run(['git', 'ls-files', '-z', '--', 'native/src'], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split('\0')
    others = [os.path.join(ROOT, p) for p in tracked if p and (not p.endswith('.rs') or os.path.join(ROOT, p) not in owners)]
    users = collections.defaultdict(set)
    for name, (d, w) in ws.items():
        for f, s, e, text, base, kind in w.sites:
            if kind == 'include':
                users[os.path.normpath(os.path.join(base, text))].add(f)
    texts = {f: open(f, encoding='utf-8', errors='replace').read() for f in owners}
    for o in others:
        if users[o]:
            continue
        names = [os.path.relpath(o, SRC), os.path.relpath(os.path.dirname(o), SRC)]
        for f, t in texts.items():
            if any(re.search(re.escape(n) + r'["/]', t) for n in names if n != '.'):
                users[o].add(f)
    stays = []
    for o in others:
        homes = {moves[u].split('/src/')[0] if u in moves else None for u in users[o]}
        if users[o] and None not in homes and len(homes) == 1:
            moves[o] = os.path.join(homes.pop(), 'src', os.path.relpath(o, SRC))
        else:
            stays.append((o, sorted(rel(u) for u in users[o])))
    # The directories read whole stay whole, where they are: the typing programs, which the package ships as a
    # directory and whose `.rerail` files `#include` each other by name, and the planner's goldens, read by name
    # beside each other.
    for o in [o for o in moves if any(under(o, os.path.join(SRC, d)) for d in WHOLE)]:
        del moves[o]
        stays.append((o, sorted(rel(u) for u in users[o])))
    # A file that is not Rust and that no source names (a directory's UPSTREAM.md) goes with its directory's files,
    # when they all went to one crate and none stayed.
    staying = {o for o, _ in stays}
    for o, us in list(stays):
        if us or o.endswith('.rs'):
            continue
        d = os.path.dirname(o)
        beside = {moves[f].split('/src/')[0] for f in moves if os.path.dirname(f) == d}
        kept = [s for s in staying if s != o and os.path.dirname(s) == d] + [f for f, cs in owners.items() if ENGINE in cs and os.path.dirname(f) == d]
        if len(beside) == 1 and not kept:
            moves[o] = os.path.join(beside.pop(), 'src', os.path.relpath(o, SRC))
            stays.remove((o, us))
    clash = [b for b in moves.values() if os.path.exists(b)]
    if clash:
        sys.exit('already there: ' + ', '.join(rel(b) for b in clash))
    with open(path, 'w') as out:
        out.write("# The crate split's source moves (tools/cratify/move_sources.py plan): from, to.\n")
        for f in sorted(moves):
            out.write(f'{rel(f)}\t{rel(moves[f])}\n')
    sites = {name: {'sites': w.sites, 'defaults': w.defaults} for name, (d, w) in ws.items()}
    json.dump(sites, open(os.path.splitext(path)[0] + '.sites.json', 'w'))
    print(len(moves), 'moves', file=sys.stderr)
    for f, cs in sorted(multi.items()):
        print(f'  two crates compile {rel(f)}: {", ".join(cs)}; it goes to {rel(moves[f]).split("/src/")[0]}', file=sys.stderr)
    for o, us in stays:
        print(f'  stays: {rel(o)}' + (f' (named by {", ".join(us[:3])})' if us else ' (no crate names it)'), file=sys.stderr)


def load(path):
    out = {}
    for l in open(path):
        if l.strip() and not l.startswith('#'):
            a, b = l.rstrip('\n').split('\t')
            out[os.path.join(ROOT, a)] = os.path.join(ROOT, b)
    return out


def move(path):
    pairs = sorted(load(path).items())
    for a, b in pairs:
        os.makedirs(os.path.dirname(b), exist_ok=True)
        subprocess.run(['git', 'mv', rel(a), rel(b)], cwd=ROOT, check=True)
    print(len(pairs), 'files moved', file=sys.stderr)


def repath(path):
    moves = load(path)
    sites = json.load(open(os.path.splitext(path)[0] + '.sites.json'))

    def home(f):
        """The directory a file's modules are now under: its crate's src/ for native/src's, as it was otherwise."""
        g = moves.get(f, f)
        return g.split('/src/')[0] + '/src' if under(g, NATIVE + '/crates') else SRC if under(f, SRC) else None

    def now_dir(d, f):
        """Where directory `d`, as file `f` read it before the move, is now."""
        h = home(f)
        return os.path.join(h, os.path.relpath(d, SRC)) if under(d, SRC) and h and h != SRC else d

    old_dirs = {os.path.dirname(a) for a in moves}
    while True:
        more = {os.path.dirname(d) for d in old_dirs} - old_dirs
        if not more or all(len(d) <= len(ROOT) for d in more):
            break
        old_dirs |= more
    new_paths = set(moves.values())

    def existed(p):
        """Whether `p` was there before the move."""
        return p in moves or p in old_dirs or (os.path.exists(p) and p not in new_paths)

    edits = collections.defaultdict(set)
    for name, s in sites.items():
        for f, start, end, text, base, kind in s['sites']:
            target = os.path.normpath(os.path.join(base, text))
            if not existed(target):
                # The other side of a cfg_attr(path): the walk reads both, rustc one; this one named nothing.
                continue
            target = now_dir(target, f) if kind == 'dir' else moves.get(target, target)
            new_base = now_dir(base, f)
            new = os.path.relpath(target, new_base)
            if new != text:
                edits[moves.get(f, f)].add((start, end, text, new))
        for f, offset, mod, moddir, base, found in s['defaults']:
            want = moves.get(found, found)
            d = now_dir(moddir, f)
            if want not in (os.path.join(d, mod + '.rs'), os.path.join(d, mod, 'mod.rs')):
                edits[moves.get(f, f)].add((offset, offset, None, os.path.relpath(want, now_dir(base, f))))
    n = 0
    for f, es in sorted(edits.items()):
        src = open(f, encoding='utf-8').read()
        for start, end, old, new in sorted(es, reverse=True):
            if old is None:
                indent = re.match(r'[ \t]*', src[start:]).group(0)
                src = src[:start] + f'{indent}#[path = "{new}"]\n' + src[start:]
            else:
                assert src[start:end] == old, (rel(f), src[start:end], old)
                src = src[:start] + new + src[end:]
            n += 1
        open(f, 'w', encoding='utf-8').write(src)
    print(n, 'paths written in', len(edits), 'files', file=sys.stderr)


def where(files):
    dirs = collections.defaultdict(set)
    for name, (d, w) in walks().items():
        for f in w.files:
            dirs[os.path.dirname(f)].add(name)
    for f in files:
        p = os.path.realpath(f)
        print(rel(p), ', '.join(sorted(dirs.get(os.path.dirname(p), {'(no crate compiles its directory)'}))))


if __name__ == '__main__':
    cmd, args = (sys.argv[1], sys.argv[2:]) if len(sys.argv) > 1 else ('', [])
    if cmd in ('plan', 'move', 'repath'):
        globals()[cmd](os.path.abspath(args[0]) if args else PLAN)
    elif cmd == 'where':
        where(args)
    else:
        sys.exit(__doc__)
