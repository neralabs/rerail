"""Checks on the hand-written .rr files: every rule grounded, and no relation declared twice in one program
(a rule file plus everything it includes). Run: python3 site/check_rr.py"""
import collections, glob, os, re, sys
sys.path.insert(0, os.path.dirname(__file__))
import souffle

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def includes(f):
    return re.findall(r'^#include "([^"]+)"', open(f).read(), re.M)


def program(f, seen=None):
    seen = [] if seen is None else seen
    if f not in seen:
        seen.append(f)
        for i in includes(f):
            program(os.path.join(ROOT, "lib", i), seen)
    return seen


def main():
    os.chdir(ROOT)
    bad = 0
    edits = set()
    for f in glob.glob("lib/**/*.rr", recursive=True):
        edits |= souffle.fix_relations(open(f).read())
    for f in sorted(glob.glob("**/*.rr", recursive=True)):
        if f.startswith("site/"):
            continue
        for line, vs, st in souffle.check_text(open(f).read(), edits):
            bad += 1
            print(f"{f}:{line}: unbound {', '.join(vs)}")
    for f in sorted(glob.glob("*.rr") + glob.glob("data-layer/*.rr")):
        files = program(os.path.abspath(f)) + [os.path.join(ROOT, "lib/core.rr")]
        decls = collections.defaultdict(list)
        for g in dict.fromkeys(files):
            for m in re.finditer(r"^\.(?:decl|edit)\s+(\w+)\(", open(g).read(), re.M):
                decls[m.group(1)].append(os.path.relpath(g, ROOT))
        for k, v in decls.items():
            if len(v) > 1:
                bad += 1
                print(f"{f}: {k} declared {len(v)} times: {', '.join(sorted(v))}")
    print("ok" if not bad else f"{bad} problems")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
