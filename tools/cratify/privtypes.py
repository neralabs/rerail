"""The crate split's visibility sweep (tools/cratify/settle.sh): widens each type a check reports private (`type `rerail_x::code_rule_dsl::a::b::Name<...>` is private`): its
declaration in the module's file, in the source and the rehearsal. Args: check output (short format), the tree's root, tree2.tsv."""
import collections, re, sys
import os
ck, R = sys.argv[1], sys.argv[2].rstrip('/') + '/'
# a second tree kept in step (the prep branch, when R is a rehearsal of it): CRATIFY_SOURCE
SRC = (os.environ.get('CRATIFY_SOURCE') or R).rstrip('/') + '/'
TREE = sys.argv[3]
tree = {}
for l in open(TREE):
    p, f = l.split('\t')[:2]
    tree[p] = f
want = set()
for line in open(ck):
  if 'is private' not in line: continue
  for m in re.finditer(r"(?:rerail_[a-z_]+::)?((?:[a-z_0-9]+::)+)([A-Z][A-Za-z_0-9]*)", line):
    mods, name = m.group(1).rstrip(':'), m.group(2)
    if not m.group(0).startswith('rerail_'):
        # a path as the error names it in scope: the modules ending so that declare the type
        for p, f in tree.items():
            if p.endswith('::' + mods) and re.search(r'(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union|trait|type)\s+' + name + '(?![A-Za-z0-9_])', open(SRC + f).read()):
                want.add((f, name))
        continue
    # the type's module: the longest prefix the tree knows
    segs = mods.split('::') if mods else []
    for k in range(len(segs), 0, -1):
        p = 'crate::' + '::'.join(segs[:k])
        if p in tree:
            want.add((tree[p], name)); break
import glob
for line in open(ck):
    m = re.search(r"(?:crate-private|restricted|private) type `([A-Za-z_][A-Za-z_0-9]*)", line)
    if m and 'public interface' in line:
        name = m.group(1)
        src = line.split(':')[0].replace('native/crates/', '').split('/src/../../../src/')[-1]
        here = 'native/src/' + src
        cands = [here] if re.search(r'(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union|trait|type)\s+' + name + '(?![A-Za-z0-9_])', open(SRC + here).read() if __import__('os').path.exists(SRC + here) else '') else [f for f in set(tree.values()) if re.search(r'(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union|trait|type)\s+' + name + '(?![A-Za-z0-9_])', open(SRC + f).read())]
        for f in cands:
            want.add((f, name))
n = 0
for f, name in sorted(want):
    for root in dict.fromkeys((SRC, R)):
        try:
            s = open(root + f).read()
        except FileNotFoundError:
            continue
        t = re.sub(r'(?m)^(\s*)(?:pub(?:\s*\([^)]*\))?\s+)?((?:unsafe\s+)?(?:struct|enum|union|trait|type|fn|const|static)\s+' + re.escape(name) + r')\b', r'\1pub \2', s)
        if t != s:
            open(root + f, 'w').write(t); n += root == SRC
    print(f, name)
print('widened', n)

# private fields: "field `x` of struct `p::S` is private", "fields `a` and `b` of struct `p::S` are private"
fields = collections.defaultdict(set) if False else {}
for line in open(ck):
    m = re.search(r"fields? ((?:`[A-Za-z_0-9]+`(?:, | and )?)+) of struct `(?:rerail_[a-z_]+::)?((?:[a-z_0-9]+::)*)([A-Z][A-Za-z_0-9]*)", line)
    if not m:
        continue
    names = re.findall(r'`([A-Za-z_0-9]+)`', m.group(1))
    segs = m.group(2).rstrip(':').split('::') if m.group(2) else []
    f = None
    for k in range(len(segs), 0, -1):
        p = 'crate::' + '::'.join(segs[:k])
        if p in tree:
            f = tree[p]; break
    if f:
        fields.setdefault((f, m.group(3)), set()).update(names)
nf = 0
def declares(f, sname):
    return re.search(r'(?m)^\s*(?:pub(?:\s*\([^)]*\))?\s+)?struct\s+' + sname + r'(?![A-Za-z0-9_])', open(SRC + f).read()) is not None
moved = {}
for (f, sname), names in fields.items():
    if not declares(f, sname):
        for g in sorted(set(tree.values())):
            if declares(g, sname):
                moved.setdefault((g, sname), set()).update(names)
    else:
        moved.setdefault((f, sname), set()).update(names)
for (f, sname), names in moved.items():
    for root in dict.fromkeys((SRC, R)):
        s = open(root + f).read()
        d = re.search(r'(?m)^\s*(?:pub(?:\s*\([^)]*\))?\s+)?struct\s+' + sname + r'(?![A-Za-z0-9_])[^{;(]*[{(]', s)
        if not d:
            continue
        a = d.end()
        depth, z = 1, a
        while z < len(s) and depth:
            depth += {'{': 1, '(': 1}.get(s[z], 0) - {'}': 1, ')': 1}.get(s[z], 0); z += 1
        body = s[a:z]
        for n in names:
            if n.isdigit():
                body = re.sub(r'^(\s*)(?:pub(?:\s*\([^)]*\))?\s+)?', r'\1pub ', body, count=1) if n == '0' else body
            else:
                body = re.sub(r'(?m)(^|[{,])(\s*(?:#\[[^\]]*\]\s*)*)(?:pub(?:\s*\([^)]*\))?\s+)?(' + n + r'\s*:)(?!:)', r'\1\2pub \3', body)
        t = s[:a] + body + s[z:]
        if t != s:
            open(root + f, 'w').write(t); nf += root == SRC
print('fields widened', nf)

# names a `use super::*` child cannot find: the parent module's (its file's) private items
byfile = {}
for p, f in tree.items():
    byfile.setdefault(f, []).append(p)
ns = 0
for line in open(ck):
    m = re.match(r"^native/crates/[a-z_]+/src/\.\./\.\./\.\./src/([^:]+):\d+:\d+: error\[E04(?:05|25|33|22|23)\]: cannot find (?:type|function|value|struct, variant or union type|trait) `([A-Za-z_][A-Za-z_0-9]*)` in this scope", line)
    m2 = re.search(r"(?:method|associated function) `([A-Za-z_][A-Za-z_0-9]*)` is private", line)
    if not (m or m2):
        continue
    f = 'native/src/' + (m.group(1) if m else line.split('/src/../../../src/')[-1].split(':')[0])
    name = m.group(2) if m else m2.group(1)
    paths = byfile.get(f, [])
    if not paths:
        continue
    p = min(paths, key=len)
    targets = [tree.get(p.rsplit('::', 1)[0])] if m else [tree.get(x) for x in tree if x.count('::') >= 2] [:0] + [tree.get(p.rsplit('::', 1)[0]), f]
    for t in [x for x in targets if x]:
        for root in dict.fromkeys((SRC, R)):
            s = open(root + t).read()
            u = re.sub(r'(?m)^(\s*)(?:pub(?:\s*\([^)]*\))?\s+)?((?:(?:const|unsafe|async)\s+)*(?:fn|struct|enum|union|trait|type|const|static)\s+' + name + r'(?![A-Za-z0-9_]))', r'\1pub \2', s)
            if u != s:
                open(root + t, 'w').write(u); ns += root == SRC
print('parent items widened', ns)
