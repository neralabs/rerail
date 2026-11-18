#!/usr/bin/env python3
"""The loops of a function's disassembly (`objdump -d --no-show-raw-insn`): each backward branch and
the instructions from its target to it, with the calls in it. `--loop N` prints loop N's
instructions; `--mnemonics` their mnemonics, one a line, for a diff.
Usage: loops.py FILE.s [--loop N] [--mnemonics]"""
import re, sys
lines = open(sys.argv[1]).read().splitlines()
insts = []
for l in lines:
    m = re.match(r"\s*([0-9a-f]+):\s+(\S+)\s*(.*)", l)
    if m:
        insts.append((int(m.group(1), 16), m.group(2), m.group(3)))
pos = {a: i for i, (a, _, _) in enumerate(insts)}
loops = []
for i, (a, op, args) in enumerate(insts):
    if op.startswith("b") and op not in ("bl", "blr", "br", "bic", "bif", "bit", "bsl", "bfi", "bfxil") or op in ("cbz", "cbnz", "tbz", "tbnz"):
        m = re.search(r"0x([0-9a-f]+)", args)
        if m:
            t = int(m.group(1), 16)
            if t <= a and t in pos:
                loops.append((pos[t], i))
want = None
if "--loop" in sys.argv:
    want = int(sys.argv[sys.argv.index("--loop") + 1])
for n, (s, e) in enumerate(loops):
    body = insts[s:e + 1]
    calls = [x[2].split("<")[-1].rstrip(">") for x in body if x[1] == "bl"]
    if want is None:
        print(f"loop {n}: {e - s + 1} instructions at +{hex(insts[s][0] - insts[0][0])}..+{hex(insts[e][0] - insts[0][0])}, calls {calls}")
    elif n == want:
        for a, op, args in body:
            print(op if "--mnemonics" in sys.argv else f"{hex(a - insts[0][0])}: {op} {args}")
