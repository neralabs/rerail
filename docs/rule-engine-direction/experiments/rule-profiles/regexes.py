"""Write regexes per rule tagged by host kind: identifier/module/other -> out/regex/<rule>.<host>.txt"""
import json, sys, os, glob, collections
RULES, OUT = sys.argv[1], sys.argv[2]
for fn in glob.glob(os.path.join(RULES, '*.json')):
    d = json.load(open(fn)); rid = d['id'].replace('data-layer-', '')
    buckets = collections.defaultdict(set)
    def w(x, host):
        if isinstance(x, dict):
            if isinstance(x.get('all'), list):
                ks = [e['kind'] for e in x['all'] if isinstance(e, dict) and isinstance(e.get('kind'), str)]
                h = ks[0] if ks else host
                for e in x['all']:
                    if isinstance(e, dict) and isinstance(e.get('regex'), str):
                        buckets[h if h in ('identifier', 'module', 'string') else 'other'].add(e['regex'])
                for v in x.values(): w(v, h)
                return
            for v in x.values(): w(v, host)
        elif isinstance(x, list):
            for v in x: w(v, host)
    w(d, 'other')
    for h, s in buckets.items():
        with open(os.path.join(OUT, f'{rid}.{h}.txt'), 'w') as f:
            for r in sorted(s):
                f.write(r.replace('\n', '\\n') + '\n')
tot = collections.Counter()
for p in glob.glob(os.path.join(OUT, '*.txt')):
    tot[p.rsplit('.', 2)[1]] += sum(1 for _ in open(p))
print(tot)
