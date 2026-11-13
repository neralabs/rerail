"""The crate split's definition-site sweep (tools/cratify/settle.sh): widens, at the definition rustc's notes name, the items an E0624/E0603/E0423/E0616 error reports private.
    python3 defsite.py HUMAN_CHECK_OUTPUT ROOT"""
import os, re, sys
out, reh = sys.argv[1], sys.argv[2]
src = os.environ.get('CRATIFY_SOURCE') or reh  # a second tree kept in step
text = open(out).read()
blocks = re.split(r'\n(?=(?:error|warning)(?:\[E\d+\])?: )', text)
sites = set()
for b in blocks:
    m = re.match(r'error\[(E0624|E0603|E0423|E0616|E0451)\]', b)
    if not m:
        continue
    locs = re.findall(r'(?:-->|:::) (native/[^:\s]+):(\d+):(\d+)', b)
    if len(locs) < 2:
        continue
    f, l, c = locs[1]
    f = os.path.normpath(f)
    sites.add((f, int(l), int(c)))
KW = r'(?:(?:const|unsafe|async)\s+)*(?:fn|struct|enum|union|trait|type|const|static|mod|use)\b'
n = 0
for f, l, c in sorted(sites):
    for root in dict.fromkeys((src, reh)):
        p = os.path.join(root, f)
        if not os.path.exists(p):
            continue
        lines = open(p).read().split('\n')
        line = lines[l - 1]
        seg = line[c - 1:]
        r = re.match(r'pub\s*\([^)]*\)', seg)
        if r:
            line = line[:c - 1] + 'pub' + seg[r.end():]
        elif not seg.startswith('pub '):
            line2 = re.sub(r'^(\s*)(?:pub(?:\s*\([^)]*\))?\s+)?(' + KW + ')', r'\1pub \2', line, count=1)
            if line2 == line and re.match(r'[A-Za-z_]\w*\s*:', seg):  # a field
                line2 = line[:c - 1] + 'pub ' + seg
            line = line2
        if line != lines[l - 1]:
            lines[l - 1] = line
            open(p, 'w').write('\n'.join(lines))
            n += root == src
print(f'definitions widened {n} of {len(sites)}')
