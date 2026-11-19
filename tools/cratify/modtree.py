import os, re, sys, json, collections
ROOT = sys.argv[1]
files = {}
for d, _, fs in os.walk(ROOT):
    for f in fs:
        if f.endswith('.rs'): files[os.path.normpath(os.path.join(d, f))] = None
cm = re.compile(r'//[^\n]*|/\*.*?\*/', re.S)
moddecl = re.compile(r'((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;')
src = {p: cm.sub('', open(p, encoding='utf-8', errors='ignore').read()) for p in files}
rsrc = {p: re.sub(r'pub\(in [^)]*\)', 'pub', re.sub(r'"(?:[^"\\\n]|\\.)*"', '""', cm.sub('', open(p, encoding='utf-8', errors='ignore').read()))) for p in files}
raw = {p: open(p, encoding='utf-8', errors='ignore').read() for p in files}
# resolve module tree from lib.rs
mods = {}; test = {}; cfgs = {}
def child_path(p, name, attrs, parent_is_modrs):
    m = re.search(r'#\[path\s*=\s*"([^"]+)"\]', attrs)
    d = os.path.dirname(p)
    if not parent_is_modrs:
        d = os.path.join(d, os.path.basename(p)[:-3])
    if m:
        base = os.path.dirname(p) if parent_is_modrs else os.path.join(os.path.dirname(p), os.path.basename(p)[:-3])
        # rust: path attr in non-mod.rs file relative to dir of file + stem?  Actually relative to the file's directory for top-level decl
        c1 = os.path.normpath(os.path.join(os.path.dirname(p), m.group(1)))
        c2 = os.path.normpath(os.path.join(base, m.group(1)))
        for c in (c1, c2):
            if c in files: return c
        return None
    for c in (os.path.join(d, name + '.rs'), os.path.join(d, name, 'mod.rs')):
        c = os.path.normpath(c)
        if c in files: return c
    return None
def walk(p, mp, is_test, cfg):
    if p in mods: return
    mods[p] = mp; test[p] = is_test; cfgs[p] = cfg
    isroot = os.path.basename(p) in ('mod.rs', 'lib.rs', 'main.rs')
    for attrs, name in moddecl.findall(src[p]):
        c = child_path(p, name, attrs, isroot)
        if c: 
            t = is_test or 'cfg(test)' in attrs.replace(' ', '') or bool(re.search(r'cfg\(any\(test', attrs.replace(' ','')))
            walk(c, mp + [name], t, cfg or ('target_os' in attrs and 'macos' in attrs))
walk(os.path.join(ROOT, 'lib.rs'), ['crate'], False, False)
unreached = [p for p in files if p not in mods]
print('reached', len(mods), 'unreached', len(unreached), unreached[:10])
ref_abs = re.compile(r'\bcrate((?:::[A-Za-z_][A-Za-z0-9_]*)+)')
ref_sup = re.compile(r'\b((?:super::)+)((?:[A-Za-z_][A-Za-z0-9_]*::)*[A-Za-z_][A-Za-z0-9_]*|\{[^}]*\})')

IDENT = re.compile(r'[A-Za-z_][A-Za-z0-9_]*')
def expand(chunk, i, prefix, out):
    # parse a path/use-tree at chunk[i:], prefix is list of segments
    segs = list(prefix)
    while True:
        while i < len(chunk) and chunk[i] in ' \n\t': i += 1
        if chunk.startswith('{', i):
            depth = 0; j = i; start = i + 1; items = []
            while j < len(chunk):
                ch = chunk[j]
                if ch == '{': depth += 1
                elif ch == '}':
                    depth -= 1
                    if depth == 0: items.append(chunk[start:j]); break
                elif ch == ',' and depth == 1: items.append(chunk[start:j]); start = j + 1
                j += 1
            for it in items:
                it = it.strip()
                if not it: continue
                if it.startswith('self'): out.append(segs); continue
                expand(it, 0, segs, out)
            return
        m = IDENT.match(chunk, i)
        if not m: out.append(segs); return
        segs = segs + [m.group(0)]; i = m.end()
        if chunk.startswith('::', i): i += 2; continue
        out.append(segs); return
def refs(chunk, m):
    tg = []
    for mm in re.finditer(r'\b(crate|(?:super::)+)', chunk):
        tok = mm.group(1)
        i = mm.end()
        if tok == 'crate':
            if not chunk.startswith('::', i): continue
            base = ['crate']; i += 2
        else:
            n = tok.count('super::'); base = m[:max(1, len(m)-n)]
        o = []
        expand(chunk, i, base, o)
        tg.extend(o)
    return tg
out = collections.Counter()
for p, m in mods.items():
    s = rsrc[p]
    # split at inline test module
    k = re.search(r'#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*mod\s+\w+\s*\{', s)
    parts = [(s, test[p])] if not k else [(s[:k.start()], test[p]), (s[k.start():], True)]
    for chunk, t in parts:
        tg = refs(chunk, m)
        for x in tg: out[(p, '::'.join(x), t)] += 1
json.dump({'edges': [[a, b, t, c] for (a, b, t), c in out.items()], 'loc': {p: raw[p].count('\n') for p in mods}, 'mods': {p: '::'.join(m) for p, m in mods.items()}, 'test': test, 'mac': cfgs}, open('edges.json', 'w'))
print('test files', sum(test.values()), 'test loc', sum(raw[p].count('\n') for p in mods if test[p]))
