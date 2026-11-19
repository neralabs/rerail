"""Per-rule candidate scope: files, bytes, nodes, root-kind counts (from out/files.tsv)."""
import json, re, sys, glob, os, csv
RULES = sys.argv[1]
FILES = sys.argv[2]
def g2re(g):
    i, out = 0, ''
    while i < len(g):
        c = g[i]
        if g.startswith('**/', i): out += '(?:.*/)?'; i += 3; continue
        if g.startswith('**', i): out += '.*'; i += 2; continue
        if c == '*': out += '[^/]*'
        elif c == '?': out += '[^/]'
        elif c == '[':
            j = g.index(']', i); out += g[i:j+1]; i = j + 1; continue
        else: out += re.escape(c)
        i += 1
    return re.compile('^' + out + '$')
rows = list(csv.DictReader(open(FILES), delimiter='\t'))
cols = [c for c in rows[0] if c not in ('path',)]
tot = {c: sum(int(r[c]) for r in rows) for c in cols if c != 'path'}
print('corpus', len(rows), {k: tot[k] for k in ('bytes','nodes','call','function_definition','class_definition','attribute','assignment','string')})
out = {}
for f in sorted(glob.glob(os.path.join(RULES, '*.json'))):
    d = json.load(open(f))
    sc = d.get('scope')
    if isinstance(sc, str): sc = json.loads(sc)
    inc = [g2re(x) for x in sc.get('include', ['**'])]
    exc = [g2re(x) for x in sc.get('exclude', [])]
    sel = [r for r in rows if any(p.match(r['path']) for p in inc) and not any(p.match(r['path']) for p in exc)]
    agg = {c: sum(int(r[c]) for r in sel) for c in cols}
    agg['files'] = len(sel)
    agg['max_file_nodes'] = max((int(r['nodes']) for r in sel), default=0)
    out[d['id']] = agg
    print(d['id'].replace('data-layer-',''), 'files', len(sel), 'MB %.1f' % (agg['bytes']/1e6), 'nodes %.2fM' % (agg['nodes']/1e6),
          'call', agg['call'], 'fn', agg['function_definition'], 'cls', agg['class_definition'], 'attr', agg['attribute'], 'asg', agg['assignment'], 'maxfile', agg['max_file_nodes'])
json.dump(out, open(os.path.join(os.path.dirname(FILES), 'scopes.json'), 'w'), indent=1)
