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
    for f in sorted(glob.glob("*.rr") + glob.glob("data-layer/**/*.rr", recursive=True) + glob.glob("lib/**/*.rr", recursive=True)
                    + [p for p in glob.glob("packs/**/*.rr", recursive=True) if not p.endswith(".facts.rr")]):
        files = program(os.path.abspath(f)) + [os.path.join(ROOT, "lib/core.rr")]
        texts = [open(g).read() for g in dict.fromkeys(files)]
        for line, h, d, st in souffle.check_strata(texts):
            bad += 1
            print(f"{f}: {h} depends on {d} through ! or an aggregate in a cycle "
                  f"(not stratified; no `ordered by` tie): {st[:100]}")
        decls = collections.defaultdict(list)
        for g in dict.fromkeys(files):
            for m in re.finditer(r"^\.(?:decl|edit)\s+(\w+)\(", open(g).read(), re.M):
                decls[m.group(1)].append(os.path.relpath(g, ROOT))
        for k, v in decls.items():
            if k in souffle.TEXT_TESTS or k in ("match", "match_all"):
                bad += 1
                print(f"{f}: {k} is a built-in constraint and cannot be declared ({', '.join(sorted(v))})")
            if len(v) > 1:
                bad += 1
                print(f"{f}: {k} declared {len(v)} times: {', '.join(sorted(v))}")
    # Libraries of one language must combine: a rule may include any of them, so a relation name is declared in at
    # most one of lib/<lang>/*.rr and lib/core.rr (otherwise Soufflé reports an arity clash or merges two bodies).
    core = set(re.findall(r"^\.(?:decl|edit)\s+(\w+)\(", open("lib/core.rr").read(), re.M))
    for lang in sorted(d for d in glob.glob("lib/*/") if os.path.isdir(d)):
        owner = collections.defaultdict(list)
        for g in sorted(glob.glob(lang + "*.rr")):
            if g.endswith(".facts.rr"):
                continue
            for k in set(re.findall(r"^\.(?:decl|edit)\s+(\w+)\(", open(g).read(), re.M)):
                owner[k].append(g)
        for k, v in sorted(owner.items()):
            if len(v) > 1 or k in core:
                bad += 1
                print(f"{lang}: {k} declared in {', '.join(v + (['lib/core.rr'] if k in core else []))}")
    # Soufflé accepts only \\" \\\\ \\n \\t \\r \\' in a string, so a regex's `\\.` or `\\d` must be written `\\\\.`, `\\\\d`.
    # (The engine's reader keeps an unknown escape as written, so only Soufflé would catch it.)
    for f in sorted(glob.glob("**/*.rr", recursive=True)):
        if f.startswith("site/"):
            continue
        for i, line in enumerate(open(f), 1):
            code = re.sub(r"//.*", "", line) if '"' not in line.split("//")[0] else line
            for lit in re.findall(r'"(?:[^"\\]|\\.)*"', code):
                if any(c not in '"\\ntr\'' for c in re.findall(r"\\(.)", lit[1:-1])):
                    bad += 1
                    print(f"{f}:{i}: unknown escape in a string (write \\\\ for a regex backslash): {lit[:60]}")
    print("ok" if not bad else f"{bad} problems")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
