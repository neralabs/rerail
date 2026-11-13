import json, re, sys
FILES = [l.strip() for l in open(sys.argv[1]) if l.strip()]
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
gs = lambda l: [glob_re(g) for g in (l or [])]
out = []
for c in C:
    si, se, pi, pe = gs(c['scope_include']), gs(c['scope_exclude']), gs(c['paths_include']), gs(c['paths_exclude'])
    def ok(f):
        if not any(r.match(f) for r in si) or any(r.match(f) for r in se): return False
        if c['paths_include'] is not None and not any(r.match(f) for r in pi): return False
        return not any(r.match(f) for r in pe)
    d = dict(c); d['files'] = [i for i, f in enumerate(FILES) if ok(f)]
    out.append(d)
json.dump(out, open('out/text_scope.json', 'w'))
print(sum(len(d['files']) for d in out if d['source'] == 'file.content'))
