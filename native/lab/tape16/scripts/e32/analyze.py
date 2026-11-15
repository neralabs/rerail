#!/usr/bin/env python3
"""E32: what a pack run evaluated, from its profile. analyze.py <run dir> <core.rr> [out.md]

Reads <run dir>/profile/{native.tsv,spans.tsv,placement.tsv,profile.txt}, <run dir>/strata.tsv and the out dir's
per-rule CSV names (p<i>_<CODE>.csv). Every native join of the run is one row of native.tsv: its stratum, its work,
each step's rows in -> out, and the plan's literals in order. Each literal is classed:

  base       a relation core.rr declares .input: the tape's (kind, field, child, text, within, nearest, …)
  derived    a relation the program derives (a rule's own, a library's, a request cone's q<k>_, a split part's)
  neg        a negated atom (base or derived)
  bound      a node-id range test `$n < 5764607523034234880` (the documents' id range, 0091's fact spaces)
  scope      the fact space guard: `$d = @doc($n)` and `!s_rr_scope_not($d)` (0091)
  cmp        another comparison or assignment
  fn         a text function (@contains, @prefix, @rx, @equals, @cat, @rr_node_text, …)
  agg        an aggregate (count/sum/min/max/join … : { … })
"""
import collections, os, re, sys

RUN, CORE = sys.argv[1], sys.argv[2]
OUT = sys.argv[3] if len(sys.argv) > 3 else None
P = os.path.join(RUN, 'profile')

core_inputs = set()
for line in open(CORE):
    if line.startswith('.input'):
        core_inputs.update(x for x in re.split(r'[\s,]+', line[len('.input'):].strip()) if x)
# Engine-made relations that read the tape: nearest per kind set, the long text twin, the requests' named children.
TAPE_HELPERS = re.compile(r'^(rr_nearest_ks\d+|rr_long_text_node|rr_token_named|rr_token_anon|rr_value_lit|node_order)$')

def split_lits(body):
    out, cur, depth = [], [], 0
    for i, c in enumerate(body):
        if c in '({[': depth += 1
        elif c in ')}]': depth -= 1
        if depth == 0 and body.startswith(' , ', i):
            out.append(''.join(cur)); cur = []; continue
        if depth == 0 and cur == [] and c == ' ' : continue
        cur.append(c)
    s = ''.join(cur).strip()
    if s: out.append(s.lstrip(', ').strip())
    return [x.strip(' ,') for x in out if x.strip(' ,')]

ATOM = re.compile(r'^(!?)([A-Za-z_][A-Za-z0-9_]*)\(')
def classify(lit):
    m = ATOM.match(lit)
    if re.match(r'^\$\S+ = [a-z]+ .*: \{', lit) or re.match(r'^\$\S+ = (count|sum|min|max|mean|join)\b', lit):
        return 'agg', 'aggregate'
    if m:
        neg, rel = m.group(1) == '!', m.group(2)
        if rel.endswith('rr_scope_not'):
            return 'scope', 'scope_not'
        base = rel in core_inputs or TAPE_HELPERS.match(rel)
        name = re.sub(r'^rr_nearest_ks\d+$', 'nearest(kind set)', rel) if base else rel
        return ('neg' if neg else ('base' if base else 'derived')), name
    if re.search(r'= @doc\(', lit):
        return 'scope', '@doc'
    fm = re.search(r'@([a-z_0-9]+)\(', lit)
    if re.match(r'^\$\S+ < \d{15,}$', lit):
        return 'bound', 'id range'
    if fm:
        return 'fn', '@' + fm.group(1)
    return 'cmp', re.sub(r'^\$\S+ (\S+) .*', r'\1', lit)

# The rules' codes, by program number (p<i>_<CODE>.csv in the run's out dir).
code_of = {}
for step in ('out',):
    d = os.path.join(RUN, step)
    if os.path.isdir(d):
        for f in os.listdir(d):
            m = re.match(r'^p(\d+)_(.+)\.csv$', f)
            if m: code_of.setdefault(int(m.group(1)), m.group(2))

def owner(stratum):
    m = re.search(r'(?:^|_)p(\d+)_', stratum)
    if m: return 'rule', code_of.get(int(m.group(1)), f'p{m.group(1)}')
    if re.search(r'(?:^|_)q\d+_', stratum): return 'cone', 'request cones (q<k>_)'
    return 'library', 'library and engine'

cls_steps = collections.Counter(); cls_in = collections.Counter(); cls_out = collections.Counter()
rel_steps = collections.Counter(); rel_in = collections.Counter(); rel_out = collections.Counter(); rel_neg = collections.Counter()
fn_steps = collections.Counter(); fn_in = collections.Counter()
joins = 0; work_total = 0; cpu_total = 0.0
by_owner_work = collections.Counter(); by_owner_cpu = collections.Counter(); by_rule_work = collections.Counter(); by_rule_cpu = collections.Counter()
by_part = collections.Counter(); by_part_cpu = collections.Counter()
lits_per_join = []; strata_seen = set(); stratum_cpu = collections.Counter(); stratum_work = collections.Counter()
kinds_per_join = collections.Counter()
with open(os.path.join(P, 'native.tsv')) as f:
    next(f)
    for line in f:
        span, work, steps, plan, cpu = line.rstrip('\n').split('\t')
        work, cpu = int(work), float(cpu)
        stratum = re.sub(r' \(in a job\)$', '', span.split('/stratum ')[-1])
        part = span.split('/')[0]
        joins += 1; work_total += work; cpu_total += cpu
        strata_seen.add(stratum); stratum_cpu[stratum] += cpu; stratum_work[stratum] += work
        by_part[part] += work; by_part_cpu[part] += cpu
        kind, who = owner(stratum)
        by_owner_work[kind] += work; by_owner_cpu[kind] += cpu
        if kind == 'rule': by_rule_work[who] += work; by_rule_cpu[who] += cpu
        head, _, body = plan.partition(': ')
        kinds_per_join[head.split(' ')[0]] += 1
        lits = split_lits(body)
        pairs = [tuple(int(x) for x in p.split(':')) for p in steps.split(',') if ':' in p]
        lits_per_join.append(len(lits))
        for i, lit in enumerate(lits):
            c, name = classify(lit)
            rin, rout = pairs[i] if i < len(pairs) else (0, 0)
            cls_steps[c] += 1; cls_in[c] += rin; cls_out[c] += rout
            if c in ('base', 'neg', 'derived'):
                key = name if c != 'derived' else None
                if c == 'neg':
                    rel_neg[name] += 1
                    key = name if (name in core_inputs or TAPE_HELPERS.match(name)) else None
                if key:
                    rel_steps[key] += 1; rel_in[key] += rin; rel_out[key] += rout
            if c == 'fn':
                fn_steps[name] += 1; fn_in[name] += rin

def pct(a, b): return f'{100 * a / b:.0f}%' if b else '-'
def q(vals, p):
    v = sorted(vals); return v[min(len(v) - 1, int(p * len(v)))] if v else 0

lines = []
w = lines.append
w(f'## {os.path.basename(os.path.normpath(RUN))}: what the run evaluated\n')
w(f'- native joins: {joins} in {len(strata_seen)} strata; {work_total:,} rows of work; {cpu_total:.2f} s CPU in the joins')
w(f'- literals per join: median {q(lits_per_join, .5)}, p90 {q(lits_per_join, .9)}, max {max(lits_per_join) if lits_per_join else 0}')
w(f'- joins by kind: ' + ', '.join(f'{k} {n}' for k, n in kinds_per_join.most_common()))
w('')
w('### Where the work is (by part of the run, by owner)\n')
w('| part | work | CPU s |'); w('|---|---:|---:|')
for part, n in by_part.most_common(): w(f'| {part} | {n:,} ({pct(n, work_total)}) | {by_part_cpu[part]:.3f} |')
w(''); w('| owner | work | CPU s |'); w('|---|---:|---:|')
for o, n in by_owner_work.most_common(): w(f'| {o} | {n:,} ({pct(n, work_total)}) | {by_owner_cpu[o]:.3f} |')
w('')
w('### Steps by literal class (rows in = bindings that reach the step)\n')
w('| class | steps | rows in | rows out | out/in |'); w('|---|---:|---:|---:|---:|')
tin = sum(cls_in.values())
for c, n in cls_steps.most_common():
    w(f'| {c} | {n} ({pct(n, sum(cls_steps.values()))}) | {cls_in[c]:,} ({pct(cls_in[c], tin)}) | {cls_out[c]:,} | {cls_out[c] / cls_in[c]:.2f} |' if cls_in[c] else f'| {c} | {n} | 0 | {cls_out[c]:,} | - |')
w('')
w('### Tape relations read (base relations, positive and negated)\n')
w('| relation | steps | rows in | rows out | negated steps |'); w('|---|---:|---:|---:|---:|')
for r, n in sorted(rel_steps.items(), key=lambda x: -rel_in[x[0]])[:30]:
    w(f'| {r} | {n} | {rel_in[r]:,} | {rel_out[r]:,} | {rel_neg.get(r, 0)} |')
w('')
w('### Text functions\n')
w('| function | steps | rows in |'); w('|---|---:|---:|')
for r, n in sorted(fn_steps.items(), key=lambda x: -fn_in[x[0]])[:15]:
    w(f'| {r} | {n} | {fn_in[r]:,} |')
w('')
w('### Rules by work (their own strata: p<i>_, split parts of them included)\n')
w(f'{len(by_rule_work)} rules have strata of their own. Work: median {q(list(by_rule_work.values()), .5):,}, p90 {q(list(by_rule_work.values()), .9):,}, max {max(by_rule_work.values()) if by_rule_work else 0:,}.\n')
w('| rule | work | CPU s |'); w('|---|---:|---:|')
for r, n in by_rule_work.most_common(15): w(f'| {r} | {n:,} | {by_rule_cpu[r]:.4f} |')
w('')
w('### Costliest strata\n')
w('| stratum | work | CPU s |'); w('|---|---:|---:|')
for s, c in stratum_cpu.most_common(12): w(f'| `{s[:70]}` | {stratum_work[s]:,} | {c:.4f} |')
w('')

# Placement (decision: which engine ran each stratum, the GPU's estimate).
pl = os.path.join(P, 'placement.tsv')
if os.path.exists(pl):
    rows = [l.rstrip('\n').split('\t') for l in open(pl)][1:]
    on_gpu = sum(1 for r in rows if r[2] != '0'); cheaper = sum(1 for r in rows if r[3] != '0')
    forms = collections.Counter(f for r in rows for f in r[9].split(',') if f)
    missing = collections.Counter(m for r in rows if len(r) > 10 for m in r[10].split(',') if m)
    w('### Placement (placement.tsv)\n')
    w(f'- {len(rows)} strata placed; {on_gpu} ran on the GPU; the GPU estimated cheaper for {cheaper}')
    w('- forms: ' + ', '.join(f'{k} {n}' for k, n in forms.most_common(12)))
    if missing: w('- what the GPU path lacks: ' + ', '.join(f'{k} {n}' for k, n in missing.most_common(8)))
    w('')

# Phases (profile.txt), main phase.
pt = os.path.join(P, 'profile.txt')
if os.path.exists(pt):
    txt = open(pt).read()
    m = re.search(r'== Phases.*?\n((?:\s+[\d.]+ s .*\n)+)', txt)
    if m:
        w('### Phases (profile.txt, the run\'s last phase)\n'); w('```'); w(m.group(1).rstrip()); w('```'); w('')
    m = re.search(r'== CPU by thread: (.*)\n', txt)
    if m: w(f'CPU by thread: {m.group(1)}\n')

# Spans: the top own CPU.
sp = os.path.join(P, 'spans.tsv')
if os.path.exists(sp):
    rows = [l.rstrip('\n').split('\t') for l in open(sp)][1:]
    rows.sort(key=lambda r: -float(r[3]))
    w('### Spans by own CPU (spans.tsv)\n'); w('| span | calls | own CPU s |'); w('|---|---:|---:|')
    for r in rows[:15]: w(f'| {r[0][-90:]} | {r[1]} | {float(r[3]):.3f} |')
    w('')

# Strata trace (RERAIL_STRATA_TRACE): where each stratum ran.
st = os.path.join(RUN, 'strata.tsv')
if os.path.exists(st):
    where = collections.Counter()
    for l in open(st):
        f = l.rstrip('\n').split('\t')
        if f and f[0] == 's' and len(f) > 5: where[f[5]] += 1
    w('### Strata trace\n'); w('- strata by where they ran (0 the loop thread, 1 a job, 2 a component at its retire): ' + ', '.join(f'{k}: {n}' for k, n in where.most_common())); w('')

text = '\n'.join(lines)
print(text)
if OUT: open(OUT, 'w').write(text + '\n')
