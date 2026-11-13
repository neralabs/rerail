# ndv_tune.py: the levelled counting bitmap's ndv estimate, by level choice and M, over many distinct counts and seeds.
import math, random
def mix(x):
    x = (x + 0x9e3779b97f4a7c15) & (2**64-1)
    x = ((x ^ (x >> 30)) * 0xbf58476d1ce4e5b9) & (2**64-1)
    x = ((x ^ (x >> 27)) * 0x94d049bb133111eb) & (2**64-1)
    return x ^ (x >> 31)
def occ(vals, M, salt):
    lv = [set() for _ in range(32)]
    for v in vals:
        h = mix(v ^ salt); t = min((h & -h).bit_length() - 1, 31) if h else 31
        lv[t].add((h >> 32) % M)
    return lv
def est(lv, M, rule):
    occupied = set(); cands = []
    for L in range(31, -1, -1):
        occupied |= lv[L]; empty = M - len(occupied)
        if empty == 0: break
        cands.append((L, empty))
    if rule == 'lowest30':
        ok = [c for c in cands if c[1] * 10 >= M * 3]; L, e = ok[-1]
        return 2**L * -M * math.log(e / M)
    if rule == 'half':  # the level whose union is nearest half full
        L, e = min(cands, key=lambda c: abs(c[1] / M - 0.5))
        return 2**L * -M * math.log(e / M)
    if rule == 'avg2':  # geometric mean of the two levels nearest half full
        cs = sorted(cands, key=lambda c: abs(c[1] / M - 0.5))[:2]
        return math.exp(sum(math.log(2**L * -M * math.log(e / M)) for L, e in cs) / len(cs))
for M in (64, 128):
    for rule in ('lowest30', 'half', 'avg2'):
        worst = 1; errs = []
        for n in [70, 100, 154, 245, 500, 1000, 2831, 6040, 22162, 59793, 113255]:
            for salt in range(8):
                base = random.Random(n * 31 + salt).randrange(1 << 40)
                vals = range(base, base + n)
                r = est(occ(vals, M, 0x5bd1e995 + salt * 7777), M, rule) / n
                worst = max(worst, r, 1 / r); errs.append(abs(math.log(r)))
        print(f"M={M} {rule:9} worst x{worst:.2f}  mean |log err| {sum(errs)/len(errs):.3f}")
