# shadow.py <tsv>: the sketch's estimates against the exact counts, for columns over 64 distinct values (the rest stay exact).
import sys, os
T = int(os.environ.get("T", "64"))
rows = [l.rstrip('\n').split('\t') for l in open(sys.argv[1])]
big = [r for r in rows if int(r[3]) > T]
def ratio(e, x): return float(e) / max(float(x), 1)
fn = [ratio(r[6], r[5]) for r in big]; nd = [ratio(r[4], r[3]) for r in big]
w = lambda xs: max(max(xs), 1 / min(xs)) if xs else 0
print(f"{len(rows)} columns measured, {len(big)} over 64 distinct values; rows up to {max(int(r[2]) for r in rows):,}, ndv up to {max(int(r[3]) for r in rows):,}")
print(f"sq (fanout) worst x{w(fn):.2f}, ndv worst x{w(nd):.2f}; bytes per sketch up to {max(int(r[7]) for r in big) if big else 0}")
exact = sum(int(r[3]) for r in big)
print(f"exact per-value entries for those columns: {exact:,}; sketches: {sum(int(r[7]) for r in big):,} bytes")
for r in sorted(big, key=lambda r: -max(ratio(r[6], r[5]), 1/ratio(r[6], r[5]), ratio(r[4], r[3]), 1/max(ratio(r[4], r[3]),1e-9)))[:6]:
    print(f"  {r[0][:40]:40} {r[1]:6} rows {int(r[2]):>9,} ndv {int(r[3]):>8,} est x{ratio(r[4], r[3]):.2f}  fan {float(r[5])/int(r[2]):>9.1f} est x{ratio(r[6], r[5]):.2f}")
