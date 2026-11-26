"""What an edit to each workspace crate costs the CLI's rebuild, from `cargo metadata` alone (no build).

usage (from anywhere in the repository):
  fanout.py [--features F] [--root PACKAGE | --workspace] [--top N] [--json OUT] [CRATE...]

A body edit changes a crate's SVH, so cargo runs rustc again on the crate and on every crate that depends on it,
directly or not. So, for each workspace crate in the root's build (the CLI's, `rerail-engine`, by default):

  above     the workspace crates an edit to it recompiles, itself left out;
  chain     the longest dependency chain from it to the root, itself included: the recompiles that run one after
            another however many cores there are;
  lines     the lines of the files its module trees compile (its tests' included): what its own recompile reads.

and the graph's longest chain overall. Normal and build dependencies count (what the CLI builds); dev-dependencies do
not. The resolve is cargo's, with the root's default features and `--features` (`test-hooks` for the build the tests
share, AGENTS.md). Named CRATEs print alone, with the crates that make them reachable above (their dependents).
"""
import argparse, collections, json, os, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
NATIVE = os.path.join(ROOT, 'native')


def metadata(features):
    cmd = ['cargo', 'metadata', '--format-version', '1', '--offline']
    if features:
        cmd += ['--features', features]
    return json.loads(subprocess.run(cmd, cwd=NATIVE, capture_output=True, text=True, check=True).stdout)


def sizes():
    """Each workspace crate's lines: the files its module trees compile (move_sources.py's walk, as rustc resolves
    `mod` and `#[path]`), not its folder, which other crates may compile from too."""
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import move_sources
    out = {}
    for name, (d, w) in move_sources.walks().items():
        out[name] = sum(sum(1 for _ in open(f, encoding='utf-8', errors='replace')) for f in w.files if f.endswith('.rs'))
    return out


def graph(meta, root, dev=False):
    members = set(meta['workspace_members'])
    pkgs = {p['id']: p for p in meta['packages']}
    name = {i: pkgs[i]['name'] for i in members}
    by_name = {v: k for k, v in name.items()}
    deps, tests = collections.defaultdict(set), collections.defaultdict(set)
    for node in meta['resolve']['nodes']:
        if node['id'] not in members:
            continue
        for d in node['deps']:
            if d['pkg'] not in members or d['pkg'] == node['id']:
                continue
            kinds = {k['kind'] for k in d['dep_kinds']}
            if kinds & {None, 'build'}:
                deps[name[node['id']]].add(name[d['pkg']])
            elif dev and 'dev' in kinds:
                tests[name[node['id']]].add(name[d['pkg']])
    # The root's build: what it reaches.
    built, todo = set(), ([root] if root else list(name.values()))
    while todo:
        c = todo.pop()
        if c not in built:
            built.add(c)
            todo += deps[c] | tests[c]
    deps = {c: {d for d in deps[c] if d in built} for c in built}
    users, rebuilt = collections.defaultdict(set), collections.defaultdict(set)
    for c, ds in deps.items():
        for d in ds | tests[c]:
            rebuilt[d].add(c)
        for d in ds:
            users[d].add(c)
    return deps, users, rebuilt, {c: pkgs[by_name[c]]['manifest_path'] for c in built}


def measure(deps, users, rebuilt):
    """`above` follows `rebuilt` (a test target's dev-dependencies too); the chains, the normal graph (acyclic)."""
    above = {}
    for c in deps:
        seen, todo = set(), list(users[c])
        while todo:
            u = todo.pop()
            if u not in seen and u != c:
                seen.add(u)
                todo += users[u]
        # A test target is rebuilt when one of its dev-dependencies is, and nothing depends on it.
        above[c] = seen | {t for d in seen | {c} for t in rebuilt[d] if t != c}
    # The longest chain from a crate up to the root (itself included), and below it (itself included).
    up, down = {}, {}

    def chain_up(c):
        if c not in up:
            up[c] = 1 + max((chain_up(u) for u in users[c]), default=0)
        return up[c]

    def chain_down(c):
        if c not in down:
            down[c] = 1 + max((chain_down(d) for d in deps[c]), default=0)
        return down[c]

    sys.setrecursionlimit(10000)
    for c in deps:
        chain_up(c)
        chain_down(c)
    return above, up, down


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--features', default='')
    ap.add_argument('--root', default='rerail-engine')
    ap.add_argument('--top', type=int, default=30)
    ap.add_argument('--json')
    ap.add_argument('--workspace', action='store_true', help="every member's build, tests' dev-dependencies included")
    ap.add_argument('crates', nargs='*')
    a = ap.parse_args()
    root = None if a.workspace else a.root
    deps, users, rebuilt, manifests = graph(metadata(a.features), root, dev=a.workspace)
    above, up, down = measure(deps, users, rebuilt)
    every = sizes()
    size = {c: every.get(c, 0) for c in manifests}
    longest = max(down.values())
    what = 'the workspace (every member, dev-dependencies included)' if a.workspace else f'{a.root}'
    print(f'{what}: {len(deps)} workspace crates in its build; the longest chain is {longest} crates')
    rows = sorted(deps, key=lambda c: (-len(above[c]) * size[c], c))
    if a.crates:
        rows = a.crates
    print(f'{"crate":34} {"lines":>7} {"above":>5} {"chain":>5} {"below":>5}   cost (lines x above, k)')
    for c in rows[: a.top] if not a.crates else rows:
        print(f'{c:34} {size[c]:>7} {len(above[c]):>5} {up[c]:>5} {down[c]:>5}   {size[c] * len(above[c]) / 1000:>8.0f}')
    for c in a.crates:
        print(f'\n{c}: used directly by {len(users[c])}: {", ".join(sorted(users[c]))}')
    if a.json:
        json.dump({c: {'lines': size[c], 'above': sorted(above[c]), 'chain_up': up[c], 'chain_down': down[c],
                       'deps': sorted(deps[c])} for c in deps}, open(a.json, 'w'), indent=1)


if __name__ == '__main__':
    main()
