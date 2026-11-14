"""Compare two runs of the planner measurement test (`o148_plans_and_their_work`, its `O148_OUT` files).

usage: compare.py before.tsv after.tsv head[,head...]

Prints each program's total SQLite VM steps, before and after, and the steps of every head relation's statements
that differ or are named. VM steps are deterministic: they count the work of the plans, not the machine's load.
"""
import sys, re, collections
def load(p):
    per = collections.defaultdict(int); tot = collections.defaultdict(int)
    for l in open(p):
        f = l.rstrip('\n').split('\t')
        if f[0] != 'STMT': continue
        prog, vm, text = f[1], int(f[2]), f[4]
        tot[prog] += vm
        m = re.match(r'INSERT (?:OR IGNORE )?INTO (?:new_|delta_)?(\w+) ', text)
        if m: per[(prog, m.group(1))] += vm
    return per, tot
a, ta = load(sys.argv[1]); b, tb = load(sys.argv[2])
for p in ta: print(f"{p:16} total VM steps  old {ta[p]:>10,}  new {tb[p]:>10,}  ({tb[p]/ta[p]:.3f}x)")
heads = sys.argv[3].split(',')
for (p, h) in sorted(set(a) | set(b)):
    if h in heads or a.get((p,h),0) != b.get((p,h),0):
        print(f"  {p:14} {h:22} old {a.get((p,h),0):>9,}  new {b.get((p,h),0):>9,}")
