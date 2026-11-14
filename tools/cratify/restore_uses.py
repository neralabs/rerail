"""The crate split's use restorer (tools/cratify/regenerate.sh): restores, in the generated tree, each `use` the generator dropped as upward but whose path is an external crate's."""
import difflib, re, subprocess, sys
R = sys.argv[1].rstrip('/') + '/'
files = subprocess.run(['grep', '-rl', 'cratify: a re-export of', R + 'native/src'], capture_output=True, text=True).stdout.split()
n = 0
for rf in files:
    rel = rf[len(R):]
    # the file before the generator: the tree's commit
    r = subprocess.run(['git', '-C', R, 'show', 'HEAD:' + rel], capture_output=True, text=True)
    if r.returncode:
        continue
    src = r.stdout.split('\n')
    gen = open(rf).read().split('\n')
    out = []
    sm = difflib.SequenceMatcher(None, gen, src, autojunk=False)
    for op, a, b, c, d in sm.get_opcodes():
        if op == 'replace' and all('cratify: a re-export of' in l for l in gen[a:b]):
            orig = '\n'.join(src[c:d])
            if re.match(r'\s*(pub(\([^)]*\))?\s+)?use\s+(?!crate::|super::|self::)', orig) and 'crate::' not in orig:
                out.extend(src[c:d]); n += 1
                continue
        out.extend(gen[a:b])
    open(rf, 'w').write('\n'.join(out))
print('restored', n)
