"""Component estimates (CPU seconds, single thread) per rule from measured unit costs x real corpus counts.
Inputs: out/scopes.json, out/programs.txt. Unit costs are documented in REPORT.md section 2."""
import json, sys
P = sys.argv[1]
scopes = json.load(open(f'{P}/out/scopes.json'))
progs = {}
for line in open(f'{P}/out/programs.txt'):
    f = line.split()
    if len(f) == 12 and f[2].isdigit():
        progs['data-layer-' + f[0]] = int(f[2])
# AST matching, per program-node (ns): low = fused trie (fusion.md K=512), expected = separate anchored simple
# templates x2 for heavier programs, high = 3 ns (10x heavier bodies, no fusion)
AST = (0.3, 1.0, 3.0)
# materialization into SQLite per row (us): insert 0.74-0.83 + ~2 indexes x 1.2-1.8 (measured)
ROW_TABLE = (0.74 + 2 * 1.2, 0.8 + 2 * 1.5, 0.83 + 2 * 1.8)
# binding through a batch vtab / native relation over the tape (ns per row scanned): unmeasured, assumed
ROW_VTAB = (20, 60, 200)
def ast(rule):
    n = scopes[rule]['nodes'] * progs.get(rule, 0)
    return tuple(n * c / 1e9 for c in AST)
rows = {
    # rule: (rows projected to relations under DLRE-style CST re-encoding, rows under fact families with demand)
    'data-layer-serverplan-state-tripwires': (scopes['data-layer-serverplan-state-tripwires']['nodes'] * 2 * (1 + 8.2), 3_700_000 * 2),
    'data-layer-serverplan-quality': (scopes['data-layer-serverplan-quality']['nodes'] * (1 + 8.2), 8_000_000),
    'data-layer-internal-dead-code': (0, 7_900_000 * 0.42),
    'data-layer-public-api-triples': (0, 7_900_000 * 0.42),
}
print('rule\tAST low/exp/high s\trows CST-reencode\trows facts\ttable s (facts) low/exp/high\tvtab s (facts) low/exp/high\ttable s (CST) exp')
for r in ['data-layer-serverplan-state-tripwires', 'data-layer-serverplan-steps', 'data-layer-serverplan-quality',
          'data-layer-internal-dead-code', 'data-layer-public-api-triples', 'data-layer-async-sync-delegation',
          'data-layer-transaction-patterns', 'data-layer-async-sync-boundary']:
    a = ast(r)
    cst, fam = rows.get(r, (0, 0))
    t = tuple(fam * c / 1e6 for c in ROW_TABLE)
    v = tuple(fam * c / 1e9 for c in ROW_VTAB)
    print(f"{r.replace('data-layer-','')}\t{a[0]:.2f}/{a[1]:.2f}/{a[2]:.2f}\t{cst/1e6:.0f}M\t{fam/1e6:.1f}M\t{t[0]:.1f}/{t[1]:.1f}/{t[2]:.1f}\t{v[0]:.2f}/{v[1]:.2f}/{v[2]:.2f}\t{cst*ROW_TABLE[1]/1e6:.0f}")
tot = sum(scopes[r]['nodes'] * progs.get(r, 0) for r in scopes)
print(f'gate AST program-nodes {tot/1e9:.2f}G -> CPU s', [round(tot * c / 1e9, 1) for c in AST])
