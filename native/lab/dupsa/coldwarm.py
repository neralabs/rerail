"""Cold and warm cost of each design, both methods: cold is the first run from nothing (a process: read, build,
blocks; for the saved index, also writing it); warm is one later edit: its median per-edit count in a running process
for the in-memory incremental designs; a fresh process for the others (mapping the saved index, or rebuilding the
edited tree). Instructions retired (the
process's own count, `proc_pid_rusage`): the machine's load does not change them, as it does CPU time.

usage: python3 native/lab/dupsa/coldwarm.py <out.tsv>   (from the worktree root; steps in tmp/dup-incr)
"""
import re
import statistics
import subprocess
import sys

D = "native/lab/dupsa/target/release/dupsa"
CAP = ["zsh", "tmp/capped.sh", "3000", "600", "nice", "-n", "19"]
I = "tmp/dup-incr"
METHODS = {
    "lines": ("tmp/dup-corpus/jeff", ["--include", "native/src/", "--ext", "rs", "--min", "30"]),
    "nodes": ("tmp/dup-corpus/all.cap", ["--include", "native/src/", "--ext", "rs", "--cells", "local", "--min", "150", "--min-lines", "30"]),
}


def proc(args):
    """Instructions of a whole process, the lesser of two rounds, and its stdout."""
    best, out = None, ""
    for _ in range(2):
        r = subprocess.run(CAP + [D] + args, capture_output=True, text=True)
        m = re.search(r"process: (\d+) instructions", r.stderr)
        if not m:
            sys.exit(f"no count: {args}\n{r.stderr[-2000:]}")
        n = int(m.group(1))
        if best is None or n < best:
            best, out = n, r.stdout
    return best, out


def steps(out, col):
    rows = [l.split("\t") for l in out.strip().split("\n")[1:]]
    return [float(r[col]) for r in rows]


SINGLE = ["0001", "0002", "0041", "0046", "0047", "0048"]
rows = []
for m, (base, opts) in METHODS.items():
    empty = ["incr", m, base, f"{I}/empty"] + opts
    cold_sais, _ = proc(empty + ["--build", "sais"])
    cold_dbl, _ = proc(empty + ["--build", "doubling"])
    # A design that rebuilds every run has no warm state: a run after an edit is a fresh process over the edited tree.
    warm_sais = statistics.median([proc(["incr", m, base, f"{I}/single-{s}"] + opts + ["--build", "sais"])[0] for s in SINGLE])
    warm_dbl = statistics.median([proc(["incr", m, base, f"{I}/single-{s}"] + opts + ["--build", "doubling"])[0] for s in SINGLE])
    _, out = proc(["incr", m, base, f"{I}/native"] + opts + ["--whole-doc"])
    warm_whole = statistics.median([u + d for u, d in zip(steps(out, 20), steps(out, 21))])
    _, out = proc(["incr", m, base, f"{I}/native"] + opts)
    warm_local = statistics.median([u + d for u, d in zip(steps(out, 20), steps(out, 21))])
    cold_saved, _ = proc(empty + ["--save", f"{I}/{m}.dupix"])
    warm_saved = statistics.median([proc(["incr", m, base, f"{I}/single-{s}"] + opts + ["--load", f"{I}/{m}.dupix"])[0] for s in SINGLE])
    rows += [
        (m, "Rebuild every run, SA-IS", cold_sais, warm_sais),
        (m, "Rebuild every run, doubling", cold_dbl, warm_dbl),
        (m, "Whole file re-inserted", cold_dbl, warm_whole),
        (m, "Tombstones and edit-local delta, in memory", cold_dbl, warm_local),
        (m, "Tombstones and edit-local delta, saved index mapped", cold_saved, warm_saved),
    ]
    for r in rows[-5:]:
        print("\t".join(map(str, r)), flush=True)
with open(sys.argv[1], "w") as f:
    f.write("method\tdesign\tcold_instr\twarm_instr\n")
    for r in rows:
        f.write("\t".join(str(int(x)) if isinstance(x, (int, float)) else x for x in r) + "\n")
