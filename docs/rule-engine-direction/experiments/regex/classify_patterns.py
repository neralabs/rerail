"""Shape classes for node regexes: exact literal set (^(?:a|b)$), prefix set (^(?:a|b)), suffix set ((?:a|b)$),
contains set (a|b), general. These lower to hash/AC lookups instead of DFAs."""
import json, sys, re, collections
import re._parser as P, re._constants as K
def lits(seq):
    """return set of literal strings if seq is a pure literal alternation, else None"""
    out = {''}
    for op, av in seq:
        if op == K.LITERAL: out = {s + chr(av) for s in out}
        elif op == K.SUBPATTERN:
            if av[1] or av[2]: return None
            s2 = lits(av[3])
            if s2 is None: return None
            out = {a + b for a in out for b in s2}
        elif op == K.BRANCH:
            alts = set()
            for b in av[1]:
                s2 = lits(b)
                if s2 is None: return None
                alts |= s2
            out = {a + b for a in out for b in alts}
        elif op == K.IN and len(av) == 1 and av[0][0] == K.LITERAL:
            out = {s + chr(av[0][1]) for s in out}
        else: return None
        if len(out) > 5000: return None
    return out
def shape(p):
    try: t = list(P.parse(p))
    except Exception as e: return 'unparsable', None
    if t and t[0] == (K.AT, K.AT_BEGINNING) or t and t[0] == (K.AT, K.AT_BEGINNING_STRING): start = True; t = t[1:]
    else: start = False
    if t and t[-1][0] == K.AT and t[-1][1] in (K.AT_END, K.AT_END_STRING): end = True; t = t[:-1]
    else: end = False
    if '(?i' in p: return 'general', None
    L = lits(t)
    if L is None: return 'general', None
    return {(True, True): 'exact', (True, False): 'prefix', (False, True): 'suffix', (False, False): 'contains'}[(start, end)], L
H = json.load(open('out/distinct_by_host.json'))
summ = {}
for host, ps in H.items():
    c = collections.Counter(); nl = collections.Counter()
    for p in ps:
        s, L = shape(p); c[s] += 1
        if L: nl[s] += len(L)
    summ[host] = dict(c)
    print(host, dict(c), 'literals:', dict(nl))
gen = [p for p in H['node:identifier'] if shape(p)[0] == 'general']
print(len(gen)); [print('  ', repr(g)[:120]) for g in gen[:40]]
ex = {}
for host in ('node:identifier', 'node:none'):
    for p in H[host]:
        s, L = shape(p)
        ex[p] = {'shape': s, 'literals': sorted(L) if L else None}
json.dump(ex, open('out/pattern_shapes.json', 'w'))
