"""Edit steps for `dupsa incr`, on a base tree: each step a directory with its changed files' new versions.

usage: python3 make_steps.py <base root> <steps dir> [--include native/src/] [--ext rs] [--seed 1]

The steps, cumulative (a file edited twice holds both edits):
- 1-40: one-line edits, each renaming one identifier on a random line of a random file (a typo fixed, a variable
  renamed): the edit loop's common case;
- 41: a 60-line run pasted from one file into another (a new duplicate);
- 42-45: one-line edits again;
- 46: the paste taken out again;
- 47: a file deleted;
- 48-50: edits to 5 files at once (a branch switch or a commit's worth).
"""
import os
import random
import re
import sys

args = sys.argv[1:]
root, out = args[0], args[1]
include = [args[i + 1] for i, a in enumerate(args) if a == "--include"]
ext = [args[i + 1] for i, a in enumerate(args) if a == "--ext"]
seed = int(next((args[i + 1] for i, a in enumerate(args) if a == "--seed"), 1))
rng = random.Random(seed)

files = []
for dp, dn, fn in os.walk(root):
    dn.sort()
    for f in sorted(fn):
        rel = os.path.relpath(os.path.join(dp, f), root)
        if include and not any(rel.startswith(i) for i in include):
            continue
        if ext and not any(rel.endswith("." + e) for e in ext):
            continue
        files.append(rel)
state = {}


def text(rel):
    if rel not in state:
        state[rel] = open(os.path.join(root, rel), encoding="utf-8", errors="replace").read().split("\n")
    return state[rel]


IDENT = re.compile(r"\b[a-z_][a-z0-9_]{2,}\b")


def rename_line(rel):
    lines = text(rel)
    for _ in range(50):
        k = rng.randrange(len(lines))
        m = list(IDENT.finditer(lines[k]))
        if m:
            x = rng.choice(m)
            lines[k] = lines[k][: x.start()] + x.group() + "_2" + lines[k][x.end():]
            return
    lines.append("// edited")


edited = set()


def write(step, rels, deleted=()):
    edited.update(rels)
    d = os.path.join(out, f"{step:04d}")
    for rel in rels:
        p = os.path.join(d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w", encoding="utf-8").write("\n".join(state[rel]))
    os.makedirs(d, exist_ok=True)
    if deleted:
        open(d + ".deleted", "w").write("\n".join(deleted) + "\n")


big = [f for f in files if len(text(f)) > 200]
step = 0
for _ in range(40):
    step += 1
    f = rng.choice(files)
    rename_line(f)
    write(step, [f])
src, dst = rng.sample(big, 2)
a = rng.randrange(len(text(src)) - 70)
pasted = text(src)[a:a + 60]
at = rng.randrange(len(text(dst)))
state[dst][at:at] = pasted
step += 1
write(step, [dst])
for _ in range(4):
    step += 1
    f = rng.choice([x for x in files if x != dst])
    rename_line(f)
    write(step, [f])
step += 1
del state[dst][at:at + 60]
write(step, [dst])
step += 1
gone = rng.choice([x for x in files if x not in edited])
write(step, [], [gone])
for _ in range(3):
    step += 1
    fs = rng.sample([x for x in files if x != gone], 5)
    for f in fs:
        rename_line(f)
    write(step, fs)
print(f"{step} steps; pasted {src}:{a + 1}-{a + 60} into {dst} at {at + 1}; deleted {gone}", file=sys.stderr)
