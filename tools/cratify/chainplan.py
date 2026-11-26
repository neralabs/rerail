"""The longest crate chain, read from what the code uses (xrefs.py's tables), and what would shorten it.

    python3 chainplan.py <xrefs dir> [--sim N] [--path]

- The precedence graph: crate A is above crate B when compiled code of A (not imports, not test-only code) names an
  item defined in B, by the compiler's resolution. Its longest path is the floor the cargo graph can be cut to with
  no code moved: every other cargo edge on a chain is a facade.
- `--path`: the floor's chain, each step with the files and items that make it.
- `--sim N`: moves of files between crates (whole units: a cycle of files, or files the impl rules tie together),
  greedily, each the move that shortens the chain most for the fewest lines, until the chain is N or no move helps.
  A move keeps the graph acyclic. Each step prints the chain, the lines moved and the rebuild cost (each crate's
  lines times the crates its edit rebuilds).
"""
import collections, fnmatch, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)


def load(d):
    files = {}
    for line in open(f'{d}/files.tsv'):
        f, c, n, role = line.rstrip('\n').split('\t')
        files[f] = (c, int(n), role)
    fe = collections.Counter()  # (site file, def file) -> code references
    te = collections.Counter()  # the same, from code only `test` compiles
    for line in open(f'{d}/edges.tsv'):
        a, b, k, n = line.rstrip('\n').split('\t')
        if a == b or a not in files or b not in files:
            continue
        if k == 'code' and files[a][2] in ('lib', 'bin'):
            fe[(a, b)] += int(n)
        elif k == 'test' or (k == 'code' and files[a][2] == 'test'):
            te[(a, b)] += int(n)
    cross = collections.defaultdict(collections.Counter)  # (site file, def file) -> {symbol: n}
    for line in open(f'{d}/cross.tsv'):
        a, b, s, n = line.rstrip('\n').split('\t')
        cross[(a, b)][s] += int(n)
    ties = []
    if os.path.exists(f'{d}/ties.tsv'):
        ties = [tuple(l.rstrip('\n').split('\t')[:2]) for l in open(f'{d}/ties.tsv')]
    return files, fe, te, cross, ties


def cargo_deps():
    import fanout as F
    meta = F.metadata('test-hooks')
    deps, _, _, _ = F.graph(meta, None, dev=False)
    return deps


def longest(g, nodes):
    d = {}

    def go(c, stack=()):
        if c in d:
            return d[c]
        if c in stack:
            raise ValueError('cycle at ' + c)
        d[c] = 1 + max((go(x, stack + (c,)) for x in g.get(c, ()) if x != c), default=0)
        return d[c]
    for c in nodes:
        go(c)
    top = max(nodes, key=lambda c: d[c])
    path = [top]
    while [x for x in g.get(path[-1], ()) if x != path[-1]]:
        path.append(max((x for x in g[path[-1]] if x != path[-1]), key=lambda c: d[c]))
    return d, path


def profile(d, top):
    """A placement's score, lowest best: the chain, then how many crates are at each depth from `top` down."""
    t = max(d.values())
    return (t,) + tuple(sum(1 for v in d.values() if v == k) for k in range(top, top - 6, -1))


def height(g):
    """Each node's longest path up to a root (1 at a root)."""
    rev = collections.defaultdict(set)
    nodes = set(g)
    for a, bs in g.items():
        for b in bs:
            rev[b].add(a)
            nodes.add(b)
    h = {}

    def go(c):
        if c not in h:
            h[c] = 1 + max((go(x) for x in rev[c]), default=0)
        return h[c]
    for c in nodes:
        go(c)
    return h


def short(c):
    return c[7:] if c.startswith('rerail-') else c


def main(d, args):
    files, fe, te, cross, ties = load(d)
    deps = cargo_deps()
    crates = sorted(deps)
    lib = {f for f, (c, n, r) in files.items() if r in ('lib', 'bin') and c in deps}
    # The precedence graph from the code.
    prec = collections.defaultdict(set)
    for (a, b), n in fe.items():
        ca, cb = files[a][0], files[b][0]
        if ca != cb and ca in deps and cb in deps:
            prec[ca].add(cb)

    def below(c, seen=None):
        seen = set() if seen is None else seen
        for x in deps.get(c, ()):
            if x not in seen:
                seen.add(x)
                below(x, seen)
        return seen
    odd = [(a, b) for a in prec for b in prec[a] if b not in below(a)]
    if odd:
        print(f'{len(odd)} uses of a crate not below the user (cfgs the index does not see, or the index wrong):')
        for a, b in odd[:20]:
            print('  ', short(a), '->', short(b))
            for (x, y), n in sorted(fe.items(), key=lambda kv: -kv[1]):
                if files[x][0] == a and files[y][0] == b:
                    print('      ', x, '->', y, n)
                    break
        for a, b in odd:
            prec[a].discard(b)
    cd, cpath = longest({c: deps[c] for c in crates}, crates)
    pd, ppath = longest({c: prec.get(c, set()) for c in crates}, crates)
    print('cargo chain', cd[cpath[0]], ':', ' > '.join(map(short, cpath)))
    print('code chain (the floor with no code moved)', pd[ppath[0]], ':', ' > '.join(map(short, ppath)))
    # The build's chain: each crate's test target too, over what its tests name (and its own library).
    tg = {c: set(prec.get(c, ())) for c in crates}
    for (a, b), n in te.items():
        ca, cb = files[a][0], files[b][0]
        if ca in deps and cb in deps and ca != cb:
            tg.setdefault(ca + ' [tests]', {ca}).add(cb)
    for c in crates:
        tg.setdefault(c + ' [tests]', {c})
    td, tpath = longest(tg, sorted(tg))
    print('build chain with test targets (the floor with no code moved)', td[tpath[0]], ':', ' > '.join(map(short, tpath)))
    facade = [(a, b) for a, b in zip(cpath, cpath[1:]) if b not in prec.get(a, ())]
    print('facade steps on the cargo chain:', ', '.join(f'{short(a)}->{short(b)}' for a, b in facade) or 'none')
    if '--path' in args:
        for a, b in zip(ppath, ppath[1:]):
            pairs = [((x, y), n) for (x, y), n in fe.items() if files[x][0] == a and files[y][0] == b]
            used = collections.Counter()
            users = collections.Counter()
            for (x, y), n in pairs:
                used[y] += n
                users[x] += n
            print(f'\n{short(a)} -> {short(b)}: {len(users)} files of {short(a)} use {len(used)} of {short(b)}')
            for y, n in used.most_common(6):
                syms = collections.Counter()
                for (x, y2), _ in pairs:
                    if y2 == y:
                        syms.update(cross.get((x, y2), {}))
                names = ', '.join(s.rstrip('#.').rsplit('/', 1)[-1] for s, _ in syms.most_common(5))
                print(f'   {y} ({files[y][1]} lines): {n} uses; {names}')
            for x, n in users.most_common(4):
                print(f'   used by {x} ({files[x][1]} lines): {n}')
    if '--plan' in args:
        fe, files = apply_plan(files, fe, args[args.index('--plan') + 1])
    if '--sim' in args:
        simulate(files, fe, ties, deps, prec, int(args[args.index('--sim') + 1]))


def apply_plan(files, fe, path):
    """A plan's steps, one a line (`#` comments):
      move PREFIX CRATE    the files under PREFIX go to CRATE (a new crate is any name not yet one)
      cut SITE DEF         the uses from files under SITE of files under DEF go (an interface: a slot the crate
                           above registers at load, or a parameter)
      merge A B            crate B's files go to crate A
    Prints, per step, the code chain after it."""
    files = dict(files)
    fe = collections.Counter(fe)
    for raw in open(path):
        line = raw.split('#')[0].split()
        if not line:
            continue
        op = line[0]
        if op == 'move':
            prefix, to = line[1], 'rerail-' + line[2]
            hit = [f for f in files if (fnmatch.fnmatch(f, prefix) if '*' in prefix else f.startswith(prefix))]
            assert hit, prefix
            for f in hit:
                files[f] = (to,) + files[f][1:]
        elif op == 'cut':
            m = lambda f, p: fnmatch.fnmatch(f, p) if '*' in p else f.startswith(p)
            gone = [k for k in fe if m(k[0], line[1]) and m(k[1], line[2])]
            assert gone, line
            for k in gone:
                del fe[k]
        elif op == 'merge':
            a, b = 'rerail-' + line[1], 'rerail-' + line[2]
            for f, v in list(files.items()):
                if v[0] == b:
                    files[f] = (a,) + v[1:]
        g = collections.defaultdict(set)
        for (x, y) in fe:
            cx, cy = files[x][0], files[y][0]
            if cx != cy and files[x][2] in ('lib', 'bin') and cx.startswith('rerail-') and cy.startswith('rerail-'):
                g[cx].add(cy)
        nodes = sorted(set(g) | {y for v in g.values() for y in v})
        try:
            d, p = longest(g, nodes)
            top = d[p[0]]
            print(f'{" ".join(line):70} chain {top}: {" > ".join(map(short, p))}')
        except ValueError as e:
            print(f'{" ".join(line):70} makes a cycle: {e}')
            raise SystemExit(1)
    h = height(g)
    crit = collections.defaultdict(list)
    for a in g:
        for b in g[a]:
            if d[a] == d[b] + 1 and h[a] + d[a] - 1 == top:
                n = sum(k for (x, y), k in fe.items() if files[x][0] == a and files[y][0] == b and files[x][2] in ('lib', 'bin'))
                crit[d[a]].append(f'{short(a)}->{short(b)} ({n})')
    print('every step on a longest path, by depth:')
    for k in sorted(crit, reverse=True):
        print(f'  {k}: ' + ', '.join(sorted(crit[k])))
    return fe, files


def simulate(files, fe, ties, deps, prec, target):
    # Units: the strongly connected files of each crate, joined by the ties (impl rules).
    lib = sorted(f for f, (c, n, r) in files.items() if r in ('lib', 'bin') and c in deps)
    idx = {f: i for i, f in enumerate(lib)}
    parent = list(range(len(lib)))

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    def union(a, b):
        parent[find(a)] = find(b)
    g = collections.defaultdict(set)
    for (a, b) in fe:
        if a in idx and b in idx:
            g[idx[a]].add(idx[b])
    # Tarjan over the file graph within crates (a cycle across crates cannot be).
    sys.setrecursionlimit(100000)
    index, low, on, st, n_ = {}, {}, set(), [], [0]

    def strong(v):
        index[v] = low[v] = n_[0]; n_[0] += 1
        st.append(v); on.add(v)
        for w in g[v]:
            if w not in index:
                strong(w)
                low[v] = min(low[v], low[w])
            elif w in on:
                low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            while True:
                w = st.pop(); on.discard(w)
                union(w, v)
                if w == v:
                    break
    for v in range(len(lib)):
        if v not in index:
            strong(v)
    for a, b in ties:
        if a in idx and b in idx:
            union(idx[a], idx[b])
    # A crate's root (its lib.rs, main) stays.
    pinned = set()
    for i, f in enumerate(lib):
        if f.endswith('/lib.rs') or f.endswith('/main.rs') or '/bin/' in f or f.endswith('build.rs'):
            pinned.add(find(i))
    units = collections.defaultdict(list)
    for i in range(len(lib)):
        units[find(i)].append(i)
    uid = {u: k for k, u in enumerate(units)}
    members = [units[u] for u in units]
    of = [0] * len(lib)
    for k, ms in enumerate(members):
        for i in ms:
            of[i] = k
    ucrate = [files[lib[ms[0]]][0] for ms in members]
    ulines = [sum(files[lib[i]][1] for i in ms) for ms in members]
    upin = [find(ms[0]) in pinned for ms in members]
    uout = [collections.Counter() for _ in members]
    for (a, b), n in fe.items():
        if a in idx and b in idx and of[idx[a]] != of[idx[b]]:
            uout[of[idx[a]]][of[idx[b]]] += n
    uin = [set() for _ in members]
    for u, outs in enumerate(uout):
        for w in outs:
            uin[w].add(u)
    # Crate edges kept by what no unit carries: the cargo edges no file's code makes are facades and go; external
    # crates keep theirs.
    crates = sorted(deps)
    cnt = collections.Counter()

    def touch(u, sign):
        for w in uout[u]:
            if ucrate[u] != ucrate[w]:
                cnt[(ucrate[u], ucrate[w])] += sign
        for w in uin[u]:
            if ucrate[u] != ucrate[w]:
                cnt[(ucrate[w], ucrate[u])] += sign
    for u in range(len(members)):
        for w in uout[u]:
            if ucrate[u] != ucrate[w]:
                cnt[(ucrate[u], ucrate[w])] += 1

    def graph():
        gg = collections.defaultdict(set)
        for (a, b), k in cnt.items():
            if k > 0:
                gg[a].add(b)
        return gg

    def depth():
        try:
            d, p = longest(graph(), crates)
        except ValueError:
            return None, None
        return d, p

    def move(group, t):
        gs = set(group)
        old = [ucrate[u] for u in group]
        for u in group:
            touch(u, -1)
        # touch counted an edge between two group members twice; add those back once
        for u in group:
            for w in uout[u]:
                if w in gs and ucrate[u] != ucrate[w]:
                    cnt[(ucrate[u], ucrate[w])] += 1
        for u in group:
            ucrate[u] = t
        for u in group:
            touch(u, 1)
        for u in group:
            for w in uout[u]:
                if w in gs and ucrate[u] != ucrate[w]:
                    cnt[(ucrate[u], ucrate[w])] -= 1
        return old

    def unmove(group, old):
        gs = set(group)
        for u in group:
            touch(u, -1)
        for u in group:
            for w in uout[u]:
                if w in gs and ucrate[u] != ucrate[w]:
                    cnt[(ucrate[u], ucrate[w])] += 1
        for u, o in zip(group, old):
            ucrate[u] = o
        for u in group:
            touch(u, 1)
        for u in group:
            for w in uout[u]:
                if w in gs and ucrate[u] != ucrate[w]:
                    cnt[(ucrate[u], ucrate[w])] -= 1

    def cost():
        gg = graph()
        rev = collections.defaultdict(set)
        for a, bs in gg.items():
            for b in bs:
                rev[b].add(a)
        lines = collections.Counter()
        for u, c in enumerate(ucrate):
            lines[c] += ulines[u]
        tot = 0
        for c in crates:
            seen, todo = set(), [c]
            while todo:
                x = todo.pop()
                for y in rev[x]:
                    if y not in seen:
                        seen.add(y); todo.append(y)
            tot += lines[c] * len(seen)
        return tot

    def closure(start, step, within):
        seen, todo = set(start), list(start)
        while todo:
            u = todo.pop()
            for w in step(u):
                if w not in seen and ucrate[w] == within:
                    seen.add(w); todo.append(w)
        return sorted(seen)

    # A move goes to an engine crate (crate-order.tsv: core or provider) or a new one; never a test crate's.
    movable = set()
    for line in open(os.path.join(HERE, 'crate-order.tsv')):
        f = line.split()
        if len(f) == 2 and not line.startswith('#') and f[1] in ('core', 'provider') and f[0] in deps:
            movable.add(f[0])
    d, p = depth()
    c0 = cost()
    print(f'\nunits: {len(members)} ({sum(upin)} pinned); the code chain {d[p[0]]}; rebuild cost {c0 // 1000}k')
    moved = 0
    for _ in range(60):
        d, p = depth()
        top = d[p[0]]
        if top <= target:
            break
        cur = profile(d, top)
        best = None
        gg = graph()
        h = height(gg)
        critical = sorted({(a, b) for a in gg for b in gg[a] if d[a] == d[b] + 1 and h[a] + d[a] - 1 == top})
        fresh = f'new-{len(crates)}'
        crates.append(fresh)
        for a, b in critical:
            used = sorted({w for u in range(len(members)) if ucrate[u] == a for w in uout[u] if ucrate[w] == b})
            using = sorted({u for u in range(len(members)) if ucrate[u] == a and any(ucrate[w] == b for w in uout[u])})
            down = closure(used, lambda u: uout[u], b)  # what the used files read in their crate goes with them
            up = closure(using, lambda u: uin[u], a)  # what reads the users in their crate goes with them
            for group, kind in ((down, 'down'), (up, 'up')):
                if any(upin[u] for u in group):
                    continue
                lines = sum(ulines[u] for u in group)
                for t in crates:
                    if t == ucrate[group[0]] or (t != fresh and t not in movable):
                        continue
                    old = move(group, t)
                    nd, np_ = depth()
                    if nd is not None:
                        s = profile(nd, top)
                        # a new crate only where no existing one does as well
                        key = (s, t == fresh, lines)
                        if s < cur and (best is None or key < best[0]):
                            best = (key, group, t, kind, a, b)
                    unmove(group, old)
        if best is None or best[2] != fresh:
            crates.remove(fresh)
        if best is None:
            print(f'no single move shortens it: {cur}')
            break
        (s, _, lines), group, t, kind, a, b = best
        src = ucrate[group[0]]
        move(group, t)
        moved += lines
        names = sorted({lib[i] for u in group for i in members[u]}, key=lambda f: -files[f][1])
        print(f'chain {s[0]} (at depth {top} down: {list(s[1:4])}): cut {short(a)}->{short(b)} by moving {len(names)} files, {lines} lines, '
              f'of {short(src)} to {short(t)}; cost {cost() // 1000}k')
        for f in names[:4]:
            print(f'     {f} ({files[f][1]})')
    d, p = depth()
    print(f'end: chain {d[p[0]]}: {" > ".join(map(short, p))}')
    gg = graph()
    h = height(gg)
    top = d[p[0]]
    crit = collections.defaultdict(list)
    for a in gg:
        for b in gg[a]:
            if d[a] == d[b] + 1 and h[a] + d[a] - 1 == top:
                n = sum(k for u in range(len(members)) if ucrate[u] == a for w, k in uout[u].items() if ucrate[w] == b)
                pin = any(upin[u] for u in range(len(members)) if ucrate[u] == a and any(ucrate[w] == b for w in uout[u]))
                crit[d[a]].append(f'{short(a)}->{short(b)} ({n}{" from a root" if pin else ""})')
    print('every step on a longest path, by depth:')
    for k in sorted(crit, reverse=True):
        print(f'  {k}: ' + ', '.join(sorted(crit[k])))
    print(f'moved {moved} lines; rebuild cost {cost() // 1000}k (from {c0 // 1000}k)')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2:])
