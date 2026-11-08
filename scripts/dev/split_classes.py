import sys, os, re, collections
d, R = sys.argv[1], sys.argv[2]
ORDER = ["file-local","config","same-unit","recursive-local","lookup","exists","grouped","total","recursive"]
rank = ORDER.index
checks = {}
for f in sorted(os.listdir(d)):
    if not f.endswith('.tsv'): continue
    n = f[:-4]
    log = open(os.path.join(d, n + '.log')).read()
    if 'rerail-engine:' in log:
        checks[n] = None; continue
    src = open(os.path.join(R, n + '.rr')).read()
    own = set(re.findall(r'^\.(?:decl|violation|metric|output)\s+(\w+)', src, re.M))
    rows = [l.rstrip('\n').split('\t', 4) for l in open(os.path.join(d, f)) if l.strip()]
    rels = {}
    for rel, o, i, out, rule in rows:
        base = re.sub(r'^fl\d+_', '', rel)
        who = 'own' if base in own else ('lowering' if rel.startswith('rr_') else 'library')
        rels.setdefault(rel, (o, who, out, []))[3].append(rule)
    checks[n] = rels
# per check: worst own-class among its own relations, and among library/lowering
tab = collections.Counter(); rows_out = []
causes = collections.defaultdict(lambda: collections.defaultdict(list))
for n, rels in checks.items():
    if rels is None:
        rows_out.append((n, 'FAILED', '', '')); continue
    def worst(who):
        cs = [o for (o, w, _, _) in rels.values() if w in who]
        return max(cs, key=rank, default='file-local')
    wo, wl = worst({'own'}), worst({'library','lowering'})
    rows_out.append((n, wo, wl, max([wo, wl], key=rank)))
    for rel, (o, w, out, rules) in rels.items():
        if rank(o) >= rank('lookup'):
            causes[o][(w, rel)].append(n)
print("check | own rules | library/lowering | overall")
for r in sorted(rows_out, key=lambda r: (r[3] == '' , rank(r[3]) if r[3] in ORDER else 99, r[0])):
    print(" | ".join(r))
print()
c1 = collections.Counter(r[1] for r in rows_out); c3 = collections.Counter(r[3] for r in rows_out)
print("own:", dict(c1)); print("overall:", dict(c3))
print()
for cls in ["recursive","total","grouped","exists","lookup"]:
    print(f"### {cls}")
    for (w, rel), ns in sorted(causes[cls].items(), key=lambda x: (x[0][0] != 'own', -len(x[1]))):
        print(f"  [{w}] {rel}  ({len(ns)} checks: {', '.join(ns[:4])}{'…' if len(ns) > 4 else ''})")
