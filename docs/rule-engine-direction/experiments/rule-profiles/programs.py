"""Count AST query programs, rule-tree size, relational stages per rule; multiply by scope nodes."""
import json, glob, os, sys
RULES = sys.argv[1]; scopes = json.load(open(sys.argv[2]))
def walk(x, f):
    if isinstance(x, dict):
        f(x)
        for v in x.values(): walk(v, f)
    elif isinstance(x, list):
        for v in x: walk(v, f)
rows = []
for fn in sorted(glob.glob(os.path.join(RULES, '*.json'))):
    d = json.load(open(fn))
    st = {'ast': 0, 'rel': 0, 'stages': 0, 'recursive': 0, 'ops': 0, 'regex': 0}
    def f(o):
        if 'ast' in o and isinstance(o['ast'], dict): st['ast'] += 1
        if 'relational' in o: st['rel'] += 1
        if 'stages' in o and isinstance(o['stages'], list): st['stages'] += len(o['stages'])
        if o.get('recursive') is True: st['recursive'] += 1
        if any(k in o for k in ('kind','has','inside','follows','precedes','pattern','regex','not','all','any','matches','nthChild','field')): st['ops'] += 1
        if 'regex' in o: st['regex'] += 1
    walk({k: v for k, v in d.items() if k != 'define'}, f)
    du = {'ops': 0}
    walk(d.get('define', {}), lambda o: du.__setitem__('ops', du['ops'] + (1 if any(k in o for k in ('kind','has','inside','pattern','regex','not','all','any','matches')) else 0)))
    sc = scopes[d['id']]
    rows.append((d['id'].replace('data-layer-', ''), os.path.getsize(fn), st['ast'], st['rel'], st['stages'], st['recursive'], st['ops'], du['ops'], st['regex'], sc['files'], sc['nodes']))
rows.sort(key=lambda r: -r[1])
print('rule bytes ast_programs relational stages recursive ops util_ops regex files nodes ast*nodes(M)')
for r in rows:
    print(*r, '%.0f' % (r[2] * r[10] / 1e6))
print('TOTAL ast programs', sum(r[2] for r in rows), 'program-nodes %.0fM' % (sum(r[2] * r[10] for r in rows) / 1e6), 'ops', sum(r[6] for r in rows))
