"""For every whole-file text clause: scope files, run the clause with today's line semantics
(content gates, line split, line include/exclude, first match per line; line-state and indentation
filters are NOT applied, they are replaced by token classification), and classify every hit by
the Python token at the hit start: code / comment / docstring / string / fstring-literal.
Output out/hits.json and per-clause summary."""
import json, re, sys, io, tokenize, os, collections, time
CORPUS = sys.argv[1]; FILES = [l.strip() for l in open(sys.argv[2]) if l.strip()]
C = json.load(open('out/text_clauses.json'))
def glob_re(g):
    i, out = 0, ''
    while i < len(g):
        if g.startswith('**/', i): out += '(?:.*/)?'; i += 3
        elif g.startswith('**', i): out += '.*'; i += 2
        elif g[i] == '*': out += '[^/]*'; i += 1
        elif g[i] == '?': out += '[^/]'; i += 1
        else: out += re.escape(g[i]); i += 1
    return re.compile(out + r'\Z')
def gs(lst): return [glob_re(g) for g in (lst or [])]
def pyre(p):
    return re.compile(re.sub(r'\(\?<([A-Za-z_]\w*)>', r'(?P<\1>', p))
SPLIT = re.compile(r'\r\n|[\n\r\x0b\x0c\x1c\x1d\x1e\x85  ]')
def lines_of(text, lf):
    pat = re.compile(r'\n') if lf else SPLIT
    s = 0
    for m in pat.finditer(text):
        yield s, text[s:m.start()]; s = m.end()
    if s < len(text): yield s, text[s:]
def token_map(text):
    """list of (start,end,cls) for comment/string tokens, char offsets"""
    rows = [0]
    for m in re.finditer('\n', text): rows.append(m.end())
    spans = []
    try:
        toks = list(tokenize.generate_tokens(io.StringIO(text).readline))
    except Exception:
        return None
    prev_sig = None
    for t in toks:
        if t.type in (tokenize.COMMENT, tokenize.STRING) or t.type in (getattr(tokenize, 'FSTRING_START', -1), getattr(tokenize, 'FSTRING_MIDDLE', -1), getattr(tokenize, 'FSTRING_END', -1)):
            a = rows[t.start[0]-1] + t.start[1]; b = rows[t.end[0]-1] + t.end[1]
            if t.type == tokenize.COMMENT: cls = 'comment'
            elif t.type == tokenize.STRING:
                cls = 'docstring' if (prev_sig in (None, tokenize.INDENT, tokenize.DEDENT, tokenize.NEWLINE) and t.string.lstrip('rRbBuU')[:3] in ('"""', "'''")) else 'string'
            else: cls = 'fstring'
            spans.append((a, b, cls))
        if t.type not in (tokenize.NL, tokenize.COMMENT):
            prev_sig = t.type
    return spans
def classify(spans, off):
    if spans is None: return 'untokenizable'
    import bisect
    i = bisect.bisect_right(spans, (off, 1 << 60, '')) - 1
    if i >= 0 and spans[i][0] <= off < spans[i][1]: return spans[i][2]
    return 'code'
texts = {}
def text(f):
    if f not in texts:
        texts[f] = open(os.path.join(CORPUS, f), encoding='utf-8', errors='surrogateescape').read()
    return texts[f]
res = []
t0 = time.time()
for c in C:
    si, se, pi, pe = gs(c['scope_include']), gs(c['scope_exclude']), gs(c['paths_include']), gs(c['paths_exclude'])
    def inscope(f):
        if not any(r.match(f) for r in si) or any(r.match(f) for r in se): return False
        if c['paths_include'] is not None and not any(r.match(f) for r in pi): return False
        return not any(r.match(f) for r in pe)
    files = [f for f in FILES if (os.environ.get('ALLSCOPE') == '1' and f.endswith('.py')) or inscope(f)]
    rec = dict(rule=c['rule'], clause=c['clause'], source=c['source'], scope_files=len(files), scope_bytes=0, files_hit=0, hits=0, cls=collections.Counter(), hit_files=[])
    try: rx = pyre(c['pattern'])
    except re.error as e:
        rec['error'] = str(e); res.append(rec); continue
    if c['source'] == 'file.path':
        hf = [f for f in files if rx.search(f)]
        rec['files_hit'] = len(hf); rec['hits'] = len(hf); rec['hit_files'] = hf; res.append(rec); continue
    L = c['lines'] or None
    RAW = os.environ.get('RAW') == '1'
    ci = [pyre(p) for p in c['content'].get('include', [])]; ce = [pyre(p) for p in c['content'].get('exclude', [])]
    li = [pyre(p) for p in (L or {}).get('include', [])]; le = [pyre(p) for p in (L or {}).get('exclude', [])]
    mi = [pyre(p) for p in c['match_include']]; me = [pyre(p) for p in c['match_exclude']]
    lf = bool(L and L.get('terminators') == 'lf')
    for f in files:
        t = text(f); rec['scope_bytes'] += len(t)
        if not RAW and (not all(r.search(t) for r in ci) or any(r.search(t) for r in ce)): continue
        hits = []
        ctxs = lines_of(t, lf) if L else [(0, t)]
        for off, ln in ctxs:
            if not RAW and L and (not all(r.search(ln) for r in li) or any(r.search(ln) for r in le)): continue
            for m in rx.finditer(ln):
                if not all(r.search(m.group()) for r in mi) or any(r.search(m.group()) for r in me): continue
                hits.append(off + m.start())
                if L and L.get('first'): break
        if hits:
            spans = token_map(t)
            rec['files_hit'] += 1; rec['hits'] += len(hits); rec['hit_files'].append(f)
            for h in hits: rec['cls'][classify(spans, h)] += 1
    res.append(rec)
    print(f"{c['rule'][11:]:34s} {c['clause'][:28]:28s} scope={len(files):5d} hitfiles={rec['files_hit']:4d} hits={rec['hits']:5d} {dict(rec['cls'])}", flush=True)
print('elapsed', time.time() - t0)
json.dump(res, open('out/hits_all.json' if os.environ.get('ALLSCOPE') == '1' else 'out/hits_raw.json' if os.environ.get('RAW') == '1' else 'out/hits.json', 'w'), indent=0)
