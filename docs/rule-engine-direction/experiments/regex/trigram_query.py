"""Cox-style regex -> trigram boolean query (codesearch 'regexp/match' analysis, simplified).
Writes out/tri_queries.json aligned with out/text_clauses.json."""
import json, re, sys
import re._parser as P
import re._constants as K
MAXSET = 32
TRUE = {'all': True}
def tris(s):
    b = s.encode('utf-8')
    return {b[i:i+3].decode('latin-1') for i in range(len(b) - 2)}
def AND(*qs):
    args = []
    for q in qs:
        if q is TRUE or q == TRUE: continue
        if q.get('op') == 'or' and not q['args']: return q
        if q.get('op') == 'and': args += q['args']
        else: args.append(q)
    # merge tri leaves
    t = set(); rest = []
    for a in args:
        if 'tri' in a: t |= set(a['tri'])
        else: rest.append(a)
    if t: rest.insert(0, {'tri': sorted(t)})
    if not rest: return TRUE
    return rest[0] if len(rest) == 1 else {'op': 'and', 'args': rest}
def OR(*qs):
    args = []
    for q in qs:
        if q == TRUE: return TRUE
        if q.get('op') == 'or': args += q['args']
        else: args.append(q)
    return args[0] if len(args) == 1 else {'op': 'or', 'args': args}
def strings_q(S):
    if S is None: return TRUE
    qs = []
    for s in S:
        t = tris(s)
        if not t or any(ord(c) > 127 for x in t for c in x): return TRUE
        qs.append({'tri': sorted(t)})
    return OR(*qs) if qs else {'op': 'or', 'args': []}
class Info:
    def __init__(s, exact=None, prefix=None, suffix=None, q=TRUE, empty=False):
        s.exact, s.prefix, s.suffix, s.q, s.empty = exact, prefix, suffix, q, empty
    def unexact(s):
        if s.exact is not None:
            ex = s.exact
            return Info(None, set(ex), set(ex), AND(s.q, strings_q(ex)), '' in ex)
        return s
def any_info(empty): return Info(None, {''}, {''}, TRUE, empty)
def lit(c): return Info({c})
def trim(S, from_end):
    if S is None or len(S) <= MAXSET: return S
    # shorten to 3 chars (keeps trigram info for single trigram); then give up
    S2 = {(x[-3:] if from_end else x[:3]) for x in S}
    return S2 if len(S2) <= MAXSET else {''}
def concat(x, y):
    if x.exact is not None and y.exact is not None and len(x.exact) * len(y.exact) <= MAXSET:
        return Info({a + b for a in x.exact for b in y.exact}, q=AND(x.q, y.q))
    q = AND(x.q, y.q)
    xe, ye = x.exact is not None, y.exact is not None
    xp = x.exact if xe else x.prefix; xs = x.exact if xe else x.suffix
    yp = y.exact if ye else y.prefix; ys = y.exact if ye else y.suffix
    if len(xs) * len(yp) <= MAXSET:
        q = AND(q, strings_q({a + b for a in xs for b in yp}))
    else:
        q = AND(q, strings_q(xs) if xe else TRUE, strings_q(yp) if ye else TRUE)
    if xe and len(xp) * len(yp) <= MAXSET: pre = {a + b for a in xp for b in yp}
    else:
        pre = set(xp)
        if xe: q = AND(q, strings_q(xp))
        if (not xe) and x.empty: pre |= set(yp)
    if ye and len(xs) * len(ys) <= MAXSET: suf = {a + b for a in xs for b in ys}
    else:
        suf = set(ys)
        if ye: q = AND(q, strings_q(ys))
        if (not ye) and y.empty: suf |= set(xs)
    return Info(None, trim(pre, False), trim(suf, True), q, x.empty and y.empty)
def alt(x, y):
    if x.exact is not None and y.exact is not None and len(x.exact | y.exact) <= MAXSET:
        return Info(x.exact | y.exact, q=OR(x.q, y.q) if (x.q != TRUE or y.q != TRUE) else TRUE)
    x, y = x.unexact(), y.unexact()
    return Info(None, trim(x.prefix | y.prefix, False), trim(x.suffix | y.suffix, True), OR(x.q, y.q), x.empty or y.empty)
CLASS_MAX = 10
def class_chars(items):
    out = set()
    for op, av in items:
        if op == K.LITERAL: out.add(chr(av))
        elif op == K.RANGE:
            if av[1] - av[0] > CLASS_MAX: return None
            out |= {chr(c) for c in range(av[0], av[1] + 1)}
        elif op == K.NEGATE: return None
        else: return None
        if len(out) > CLASS_MAX: return None
    return out
def analyze(seq, icase=False):
    info = Info({''})
    for op, av in seq:
        if op == K.LITERAL:
            c = chr(av)
            n = Info({c, c.swapcase()}) if icase and c.isalpha() else lit(c)
        elif op == K.NOT_LITERAL or op == K.ANY: n = any_info(False)
        elif op == K.IN:
            cs = class_chars(av)
            n = Info(cs) if cs else any_info(False)
        elif op == K.AT: n = Info({''})
        elif op == K.SUBPATTERN:
            _, add, dele, sub = av
            n = analyze(sub, icase or bool(add & K.SRE_FLAG_IGNORECASE))
        elif op == K.BRANCH:
            subs = [analyze(b, icase) for b in av[1]]
            n = subs[0]
            for s in subs[1:]: n = alt(n, s)
        elif op in (K.MAX_REPEAT, K.MIN_REPEAT, getattr(K, 'POSSESSIVE_REPEAT', None)):
            lo, hi, sub = av
            x = analyze(sub, icase)
            if lo == 0:
                if hi == 1 and x.exact is not None and len(x.exact) < MAXSET: n = Info(x.exact | {''}, q=TRUE)
                else: n = any_info(True)
            elif lo == 1 and hi == 1: n = x
            else:
                xu = x.unexact(); n = Info(None, xu.prefix, xu.suffix, xu.q, xu.empty)
        elif op in (K.ASSERT, K.ASSERT_NOT, K.GROUPREF): n = Info({''})
        else: n = any_info(True)
        info = concat(info, n)
    return info
def regex_query(p):
    p = re.sub(r'\(\?<([A-Za-z_]\w*)>', r'(?P<\1>', p)
    tree = P.parse(p)
    icase = bool(tree.state.flags & K.SRE_FLAG_IGNORECASE)
    i = analyze(list(tree), icase).unexact()
    q = AND(i.q, strings_q(i.prefix) if i.prefix else TRUE, strings_q(i.suffix) if i.suffix else TRUE)
    return q
if __name__ == '__main__':
    C = json.load(open('out/text_clauses.json'))
    out = []
    for c in C:
        q = regex_query(c['pattern']) if c['source'] == 'file.content' else TRUE
        # content gates are ANDed (they are authored prefilters, already required)
        for g in c['content'].get('include', []): q = AND(q, regex_query(g))
        out.append({'rule': c['rule'], 'clause': c['clause'], 'pattern': c['pattern'], 'query': q})
        print(c['clause'][:24], c['pattern'][:60], '=>', json.dumps(q)[:160])
    json.dump(out, open('out/tri_queries.json', 'w'), indent=0)
