# Simulates deletable approximate statistics over the phase store's exports: AMS (sq), levelled counting bitmap (ndv).
import sqlite3, collections, hashlib, math, statistics, sys
db = sqlite3.connect('file:/tmp/c84-ph.sqlite.full.phases?mode=ro', uri=True)
rels = collections.defaultdict(list)
for rel, g, row in db.execute("SELECT rel,granule,row FROM export_rows"):
    rels[rel].append(row.split('\t'))
def h(v, seed):
    return int.from_bytes(hashlib.blake2b(f"{seed}\0{v}".encode(), digest_size=8).digest(), 'little')
class AMS:  # count-sketch rows; F2 = median over rows of sum of squared counters. Linear: insert +1, delete -1.
    def __init__(s, w, d): s.w, s.d, s.t = w, d, [[0]*w for _ in range(d)]
    def add(s, v, k):
        for i in range(s.d):
            x = h(v, i); s.t[i][x % s.w] += k * (1 if (x >> 40) & 1 else -1)
    def f2(s): return statistics.median(sum(c*c for c in r) for r in s.t)
class L0:  # levelled counting bitmap: level = trailing zeros of hash, m counters per level. Linear, deletable.
    def __init__(s, m, levels=32): s.m, s.c = m, [[0]*m for _ in range(levels)]
    def add(s, v, k):
        x = h(v, 99); lvl = min((x & -x).bit_length() - 1, 31) if x else 31
        s.c[lvl][(x >> 32) % s.m] += k
    def ndv(s):
        # Values at level >= L are a 2^-L sample. Lowest L whose levels >= L hold few enough values for linear counting.
        for L in range(32):
            occ = [0]*s.m
            for l in range(L, 32):
                for j in range(s.m): occ[j] += s.c[l][j] != 0
            z = sum(1 for o in occ if o == 0)
            if z >= s.m * 0.3:  # linear counting is accurate here
                return (2**L) * (-s.m * math.log(max(z, 1) / s.m))
        return float('inf')
W, D, M = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
errs_f, errs_n, errs_u = [], [], []
for rel, rs in sorted(rels.items()):
    for c in range(len(rs[0])):
        cnt = collections.Counter(r[c] for r in rs); n = len(rs)
        ndv, sq = len(cnt), sum(x*x for x in cnt.values())
        a, l = AMS(W, D), L0(M)
        # Insert all, then delete a third and re-insert it: the sketch must equal the one of the final multiset.
        for r in rs: a.add(r[c], 1); l.add(r[c], 1)
        for r in rs[::3]: a.add(r[c], -1); l.add(r[c], -1)
        for r in rs[::3]: a.add(r[c], 1); l.add(r[c], 1)
        fe, ne = a.f2() / sq, l.ndv() / ndv
        ue = (n * n / sq) / ndv  # ndv from rows and sq alone (uniform assumption, a lower bound)
        errs_f.append(fe); errs_n.append(ne); errs_u.append(ue)
        print(f"{rel}.c{c}: rows={n} ndv={ndv} fan={sq/n:.1f} | fan est x{fe:.2f}  ndv est x{ne:.2f}  rows²/sq x{ue:.2f}")
worst = lambda e: max(max(e), 1 / min(e))
print(f"w={W} d={D} m={M}: fan worst x{worst(errs_f):.2f}, ndv worst x{worst(errs_n):.2f}, rows²/sq worst x{worst(errs_u):.2f}")
print(f"bytes per column: AMS {W*D*4}, ndv {M*32*2} (u16 counters)")
