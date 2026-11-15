#!/usr/bin/env python3
"""E32: static composition of a rule pack. static.py <lib dir> <rules dir> [out.tsv]

Each rule's program is its file plus every library file it includes, transitively (each once). Clauses are split
at a top-level '.', comments and strings removed. A body literal is classed as: a base relation (declared .input in
the library), a derived relation (has clauses in the program), negated (!), an aggregate (count/sum/min/max :),
a comparison, or a builtin/operator (a name with no clauses and no .input: match, contains, cat, nearest, …).
Recursion: relations in a cycle of the program's dependency graph."""
import os, re, sys, collections

LIB, RULES = sys.argv[1], sys.argv[2]
OUT = sys.argv[3] if len(sys.argv) > 3 else None

def strip(src):
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith('//', i):
            j = src.find('\n', i); i = n if j < 0 else j
        elif src.startswith('/*', i):
            j = src.find('*/', i); i = n if j < 0 else j + 2
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == '\\' else 1
            out.append('"S"'); i = j + 1
        else:
            out.append(c); i += 1
    return ''.join(out)

parsed = {}
def parse(path):
    if path in parsed: return parsed[path]
    raw = re.sub(r'(?ms)^\.test\b.*?^\.end\b[^\n]*', '', open(path).read())
    incs = re.findall(r'#include\s+"([^"]+)"', raw)
    src = strip(re.sub(r'#include[^\n]*', '', raw))
    # A directive is a line starting with '.' at column 0, with its indented continuation lines; a clause runs
    # to a top-level '.'.
    stmts, cur, depth, lines, i = [], [], 0, src.split('\n'), 0
    while i < len(lines):
        ln = lines[i]
        if not ''.join(cur).strip() and re.match(r'\.[a-z]', ln):
            d = [ln]; i += 1
            while i < len(lines) and lines[i][:1].isspace() and lines[i].strip(): d.append(lines[i]); i += 1
            stmts.append(' '.join(x.strip() for x in d)); cur = []; continue
        for j, c in enumerate(ln):
            if c in '({[': depth += 1
            elif c in ')}]': depth -= 1
            if c == '.' and depth == 0 and (j + 1 == len(ln) or ln[j + 1].isspace()) and not (j and ln[j - 1].isdigit()):
                t = ''.join(cur).strip()
                if t: stmts.append(t)
                cur = []
            else: cur.append(c)
        cur.append(' '); i += 1
    t = ''.join(cur).strip()
    if t: stmts.append(t)
    parsed[path] = (incs, stmts)
    return parsed[path]

def split_top(s, sep=','):
    parts, cur, depth = [], [], 0
    for c in s:
        if c in '({[': depth += 1
        elif c in ')}]': depth -= 1
        if c == sep and depth == 0:
            parts.append(''.join(cur).strip()); cur = []
        else: cur.append(c)
    if ''.join(cur).strip(): parts.append(''.join(cur).strip())
    return parts

def closure(rule):
    seen, order = set(), []
    def go(p):
        if p in seen: return
        seen.add(p); order.append(p)
        for inc in parse(p)[0]:
            q = os.path.join(LIB, inc)
            if os.path.exists(q): go(q)
    go(rule)
    return order

AGG = re.compile(r'\b(count|sum|min|max|mean)\s*:')
ATOM = re.compile(r'^(!?)\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(')

CORE_INPUTS = set()
for _s in parse(os.path.join(LIB, 'core.rr'))[1]:
    if _s.startswith('.input'):
        CORE_INPUTS.update(x for x in re.split(r'[\s,]+', _s[len('.input'):].strip()) if x)

def analyse(rule):
    files = closure(rule)
    inputs, clauses, directives = set(CORE_INPUTS), [], collections.Counter()
    for f in files:
        for s in parse(f)[1]:
            if s.startswith('.'):
                d = s.split()[0]
                directives[d] += 1
                if d == '.input':
                    for name in re.split(r'[\s,]+', s[len('.input'):].strip()):
                        if name: inputs.add(name)
                continue
            if ':-' in s:
                head, body = s.split(':-', 1)
            else:
                head, body = s, ''
            heads = [m.group(2) for h in split_top(head) for m in [ATOM.match(h.strip())] if m]
            clauses.append((heads, split_top(body) if body else [], f))
    derived = {h for hs, _, _ in clauses for h in hs}
    stat = collections.Counter()
    lits = collections.Counter()
    edges = collections.defaultdict(set)
    for heads, body, f in clauses:
        if not body: stat['facts'] += 1; continue
        stat['clauses'] += 1
        stat['body_literals'] += len(body)
        nb = 0
        for lit in body:
            if AGG.search(lit):
                stat['aggregates'] += 1
                for m in re.finditer(r'(!?)\b([A-Za-z_][A-Za-z0-9_]*)\s*\(', lit):
                    for h in heads: edges[h].add(('agg', m.group(2)))
                continue
            m = ATOM.match(lit)
            if not m:
                stat['comparisons'] += 1; continue
            neg, name = m.group(1) == '!', m.group(2)
            cls = 'base' if name in inputs else 'derived' if name in derived else 'builtin'
            stat[cls] += 1
            if cls == 'base': nb += 1
            if neg: stat['negations'] += 1
            lits[(cls, name)] += 1
            for h in heads: edges[h].add(('neg' if neg else 'pos', name))
        stat['max_body'] = max(stat['max_body'], len(body))
    # recursion: Tarjan over derived relations
    idx, low, onst, st, sccs, k = {}, {}, set(), [], [], [0]
    def sc(v):
        idx[v] = low[v] = k[0]; k[0] += 1; st.append(v); onst.add(v)
        for _, w in edges.get(v, ()):
            if w not in derived: continue
            if w not in idx: sc(w); low[v] = min(low[v], low[w])
            elif w in onst: low[v] = min(low[v], idx[w])
        if low[v] == idx[v]:
            comp = []
            while True:
                w = st.pop(); onst.discard(w); comp.append(w)
                if w == v: break
            sccs.append(comp)
    sys.setrecursionlimit(100000)
    for v in sorted(derived):
        if v not in idx: sc(v)
    rec = [c for c in sccs if len(c) > 1 or any(w == c[0] for _, w in edges.get(c[0], ()))]
    stat['recursive_relations'] = sum(len(c) for c in rec)
    stat['derived_relations'] = len(derived)
    stat['files'] = len(files)
    return stat, lits

rules = sorted(f for f in os.listdir(RULES) if f.endswith('.rr'))
rows, total_lits, per_rule_use = [], collections.Counter(), collections.Counter()
for r in rules:
    st, lits = analyse(os.path.join(RULES, r))
    rows.append((r[:-3], st))
    total_lits.update(lits)
    for key in lits: per_rule_use[key] += 1

cols = ['files', 'derived_relations', 'clauses', 'facts', 'body_literals', 'max_body', 'base', 'derived', 'builtin',
        'negations', 'aggregates', 'comparisons', 'recursive_relations']
if OUT:
    with open(OUT, 'w') as o:
        o.write('rule\t' + '\t'.join(cols) + '\n')
        for r, st in rows: o.write(r + '\t' + '\t'.join(str(st[c]) for c in cols) + '\n')

def q(vals, p):
    v = sorted(vals); return v[min(len(v) - 1, int(p * len(v)))]
print(f'{len(rows)} rules; per rule (each with its includes): median / p90 / max')
for c in cols:
    vals = [st[c] for _, st in rows]
    print(f'  {c:20s} {q(vals, .5):6} {q(vals, .9):6} {max(vals):6}')
print(f'  rules with negation {sum(1 for _, s in rows if s["negations"])}, aggregates {sum(1 for _, s in rows if s["aggregates"])}, recursion {sum(1 for _, s in rows if s["recursive_relations"])}')
print('\nbody literals by class and name, over the distinct library+rule clauses each rule pulls in')
print('(uses = in how many rules\' programs the literal appears)')
for cls in ('base', 'builtin'):
    items = sorted(((n, c) for (k, n), c in per_rule_use.items() if k == cls), key=lambda x: -x[1])
    print(f'  {cls}: ' + ', '.join(f'{n} {c}' for n, c in items[:40]))

# The pack as one program: each library file once, each rule's own clauses.
own = collections.Counter()
for r in rules:
    for s in parse(os.path.join(RULES, r))[1]:
        if not s.startswith('.') and ':-' in s: own['clauses'] += 1; own['literals'] += len(split_top(s.split(':-', 1)[1]))
libs = set()
for r in rules: libs.update(f for f in closure(os.path.join(RULES, r))[1:])
lc = collections.Counter()
for f in libs:
    for s in parse(f)[1]:
        if s.startswith('.'): continue
        if ':-' in s: lc['clauses'] += 1; lc['literals'] += len(split_top(s.split(':-', 1)[1]))
        else: lc['facts'] += 1
print(f'\nthe pack as one program: {len(rules)} rules\' own clauses {own["clauses"]} ({own["literals"]} body literals);'
      f' {len(libs)} library files, {lc["clauses"]} clauses ({lc["literals"]} literals), {lc["facts"]} facts')
own_sizes = sorted(sum(1 for s in parse(os.path.join(RULES, r))[1] if not s.startswith('.') and ':-' in s) for r in rules)
print(f'  own clauses per rule: median {own_sizes[len(own_sizes)//2]}, p90 {own_sizes[int(.9*len(own_sizes))]}, max {own_sizes[-1]}')
