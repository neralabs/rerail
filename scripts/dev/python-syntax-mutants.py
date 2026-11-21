"""Seeded syntax-error mutants of a directory's Python files, for `cpython::parity::syntax_error_parity` (Q360, Q389).

    python3 -I -S scripts/dev/python-syntax-mutants.py SRC OUT

Each file under SRC of at most 40,000 characters gives 6 mutants, OUT/<index>/m<k>.py: the text truncated, a
character dropped, a bracket, quote, backslash, `$`, tab, newline or other piece put in, or a line's indentation
changed, at places drawn from `random.Random(293)`. Then, on a box with CPython 3.13:

    RERAIL_PARITY_PYTHON=python3.13 RERAIL_PARITY_ROOTS=fixtures:$PWD/OUT \
      cargo nextest run -p rerail-lang-python --lib --run-ignored all --no-capture -E 'test(/syntax_error_parity/)'
"""
import os, random, sys
src, out = sys.argv[1], sys.argv[2]
rng = random.Random(293)
files = sorted(os.path.join(d, f) for d, _, fs in os.walk(src) for f in fs if f.endswith(".py"))
inserts = ["(", ")", "[", "]", "{", "}", ":", ",", "'", '"', '"""', "\\", "$", "\t", "\n", " ", "=", "print ", " ", "\x0b", "f'{"]
n = 0
for i, path in enumerate(files):
    try: text = open(path, encoding="utf-8").read()
    except Exception: continue
    if not text or len(text) > 40000: continue
    for k in range(6):
        t = text
        m = rng.randrange(4)
        at = rng.randrange(len(t))
        if m == 0: t = t[:at]
        elif m == 1: t = t[:at] + t[at + 1:]
        elif m == 2: t = t[:at] + rng.choice(inserts) + t[at:]
        else:
            lines = t.split("\n"); j = rng.randrange(len(lines))
            lines[j] = lines[j].lstrip() if lines[j].startswith(" ") else "  " + lines[j]
            t = "\n".join(lines)
        d = os.path.join(out, f"{i:04d}")
        os.makedirs(d, exist_ok=True)
        open(os.path.join(d, f"m{k}.py"), "w", encoding="utf-8").write(t)
        n += 1
print(n, "mutants")
