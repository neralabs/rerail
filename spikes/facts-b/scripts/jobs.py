#!/usr/bin/env python3
"""Per-file export jobs (path, content hash, source file) for `facts-a xexport`.

replay: every (path, hash) of worktrees 01-40, the replay's clone base (base.tsv) and every
content the edit script introduces (sources: the worktree checkouts, verified by hash in
xexport, and the edit script's generated contents).
oracle: the repository suite's pinned tree (the recording's files), sources written by hash.
"""
import hashlib, json, os, sys
from pathlib import Path

try:
    from blake3 import blake3  # type: ignore
except ImportError:  # the recording carries sources; hash with the facts-b binary instead
    blake3 = None

HOME = Path.home()
SPIKE = HOME / "rerail-bench/facts/spike-b"
CORPUS = HOME / "tempo-qe/workspaces47"


def replay(out):
    seen = {}
    for i in range(1, 41):
        for line in (SPIKE / f"membership/{i:02d}.tsv").read_text().splitlines():
            p, h = line.split("\t")
            seen.setdefault((p, h), str(CORPUS / f"{i:02d}" / p))
    for line in (SPIKE / "membership/base.tsv").read_text().splitlines():
        p, h = line.split("\t")
        seen.setdefault((p, h), None)
    for line in (SPIKE / "edits/seed-5eedb/events.jsonl").read_text().splitlines():
        ev = json.loads(line)
        for p, old, new in ev["changes"]:
            if new:
                seen.setdefault((p, new), str(SPIKE / "contents" / f"{new}.py"))
    missing = 0
    with open(out, "w") as f:
        for (p, h), src in sorted(seen.items()):
            if src is None or not os.path.exists(src):
                alt = SPIKE / "contents" / f"{h}.py"
                src = str(alt) if alt.exists() else None
            if src is None:
                missing += 1
                continue
            f.write(f"{p}\t{h}\t{src}\n")
    print(f"{len(seen) - missing} jobs, {missing} without a source", file=sys.stderr)


def oracle(recording, srcdir, out, hasher):
    rec = json.load(open(recording))
    os.makedirs(srcdir, exist_ok=True)
    rows = []
    for f in rec["files"]:
        if not f["path"].endswith(".py"):
            continue
        data = f["source"].encode()
        tmp = Path(srcdir) / ("tmp-" + hashlib.sha1(f["path"].encode()).hexdigest() + ".py")
        tmp.write_bytes(data)
        rows.append((f["path"], tmp))
    # content hashes with the facts-b binary's blake3 (`facts-b b3sum FILES…`)
    import subprocess
    names = [str(t) for _, t in rows]
    hashes = {}
    for i in range(0, len(names), 500):
        r = subprocess.run([hasher, "b3sum", *names[i:i + 500]], capture_output=True, text=True, check=True)
        for line in r.stdout.splitlines():
            h, n = line.split("  ", 1)
            hashes[n] = h
    with open(out, "w") as fo:
        for p, t in rows:
            h = hashes[str(t)]
            dst = Path(srcdir) / f"{h}.py"
            if not dst.exists():
                os.replace(t, dst)
            else:
                t.unlink()
            fo.write(f"{p}\t{h}\t{dst}\n")
    print(f"{len(rows)} oracle jobs", file=sys.stderr)


if __name__ == "__main__":
    if sys.argv[1] == "replay":
        replay(sys.argv[2])
    else:
        oracle(sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5])
