"""Inputs and checks for the exactness probes (A4 counting, A5 equality counts).

  python -I exact.py mkcnt IODIR M K N     write x.bin, w.bin (0/1 fp16) with known row counts
  python -I exact.py chkcnt IODIR M K N    compare OUTDIR y/gt/cl/rs against exact int64 results
  python -I exact.py mkeq IODIR N M [two]  write key vectors a, b (and hi/lo halves for two-digit keys)
  python -I exact.py chkeq IODIR N M [two] compare counts against exact

Row i of X has c_i ones at random positions; c_i sweeps the interesting fp16 boundaries
(2048, 4096, ..., 32768, 65504) up to K. W column 0 is all ones (y[i,0] = c_i), the rest random 0/1.
Outputs are read from IODIR/out/<name>.bin (written by anebench --out).
"""
import os
import sys

import numpy as np

BOUNDARIES = [1, 100, 1000, 2047, 2048, 2049, 2050, 2051, 3000, 4095, 4096, 4097, 4098, 4099, 8191, 8193,
              10001, 16383, 16385, 20001, 32767, 32768, 32769, 40001, 65503, 65504, 65505, 65535, 65536]


def counts(M, K):
    c = [b for b in BOUNDARIES if b <= K]
    rest = M - len(c)
    if rest > 0:
        c += list(np.linspace(1, K, rest).astype(int))
    return np.array(c[:M], dtype=np.int64)


def mkcnt(d, M, K, N):
    rng = np.random.default_rng(11)
    c = counts(M, K)
    X = np.zeros((M, K), dtype=np.float16)
    for i, ci in enumerate(c):
        X[i, rng.choice(K, size=ci, replace=False)] = 1
    W = (rng.random((K, N)) < 0.5).astype(np.float16)
    W[:, 0] = 1
    os.makedirs(d, exist_ok=True)
    X.tofile(f"{d}/x.bin")
    W.tofile(f"{d}/w.bin")
    np.save(f"{d}/exact.npy", X.astype(np.int64) @ W.astype(np.int64))
    np.save(f"{d}/c.npy", c)


def chkcnt(d, M, K, N):
    ex = np.load(f"{d}/exact.npy")
    c = np.load(f"{d}/c.npy")
    o = f"{d}/out"
    y = np.fromfile(f"{o}/y.bin", dtype=np.float16).reshape(M, N).astype(np.float64)
    gt = np.fromfile(f"{o}/gt.bin", dtype=np.float16).reshape(M, N)
    cl = np.fromfile(f"{o}/cl.bin", dtype=np.float16).reshape(M, N)
    rs = np.fromfile(f"{o}/rs.bin", dtype=np.float16).reshape(M).astype(np.float64)
    fp16 = ex.astype(np.float16).astype(np.float64)  # exact value rounded once to fp16
    err = y - ex
    bad = np.abs(err) > 0
    worse = np.abs(y - fp16) > 0  # wrong beyond the single final fp16 rounding
    print(f"K={K}: y wrong {bad.sum()}/{bad.size}; wrong beyond final fp16 rounding {worse.sum()}")
    smallest_bad = ex[bad].min() if bad.any() else None
    largest_ok = ex[~bad].max()
    print(f"  smallest exact value that came out wrong: {smallest_bad}; largest value exact: {largest_ok}")
    print(f"  inf outputs: {np.isinf(y).sum()}; max output {np.nanmax(y[np.isfinite(y)])}")
    for i in range(min(M, 29)):
        print(f"  row c={c[i]:6d}: y[i,0]={y[i,0]:9.1f} rs={rs[i]:9.1f} err={y[i,0]-c[i]:+.0f} rs_err={rs[i]-c[i]:+.0f}")
    gt_ok = (gt == (ex > 0)).all()
    cl_ok = (cl == np.minimum(ex, 1)).all()
    print(f"  gt (greater 0.5) exact: {gt_ok}; clip(0,1) exact: {cl_ok}; rs wrong: {(rs != c).sum()}/{M}")


def mkeq(d, N, M, two):
    rng = np.random.default_rng(5)
    os.makedirs(d, exist_ok=True)
    if two:
        a = rng.integers(0, 1 << 15, N)
        b = rng.integers(0, 1 << 15, M)
        b[: M // 4] = a[rng.integers(0, N, M // 4)]  # guarantee matches
        for nm, v in (("ahi", a >> 11), ("alo", a & 2047)):
            v.astype(np.float16).reshape(N, 1).tofile(f"{d}/{nm}.bin")
        for nm, v in (("bhi", b >> 11), ("blo", b & 2047)):
            v.astype(np.float16).reshape(1, M).tofile(f"{d}/{nm}.bin")
    else:
        a = rng.integers(0, 2048, N)
        b = rng.integers(0, 2048, M)
        a.astype(np.float16).reshape(N, 1).tofile(f"{d}/a.bin")
        b.astype(np.float16).reshape(1, M).tofile(f"{d}/b.bin")
    np.save(f"{d}/exact.npy", (a[:, None] == b[None, :]).sum(1))


def chkeq(d, N, M, two):
    ex = np.load(f"{d}/exact.npy")
    y = np.fromfile(f"{d}/out/counts.bin", dtype=np.float16).astype(np.float64)
    print(f"eq N={N} M={M} two={two}: wrong {(y != ex).sum()}/{N}; total exact {ex.sum()} got {y.sum():.0f}; "
          f"max count {ex.max()}")


def main():
    cmd, d = sys.argv[1], sys.argv[2]
    a = [int(v) for v in sys.argv[3:5 if cmd.endswith("eq") else 6]]
    two = len(sys.argv) > 5 and sys.argv[5] == "two"
    {"mkcnt": lambda: mkcnt(d, *a), "chkcnt": lambda: chkcnt(d, *a),
     "mkeq": lambda: mkeq(d, *a, two), "chkeq": lambda: chkeq(d, *a, two)}[cmd]()


if __name__ == "__main__":
    main()
