"""ANE models, inputs and exact checks for real workloads from the data-layer corpus (W1, W5).

Usage: python -I real.py DATA_DIR RUNS CMD VARIANT
  CMD build  : write RUNS/models/<VARIANT>.mlpackage and RUNS/io/<VARIANT>/<input>.bin
  CMD check  : compare RUNS/io/<VARIANT>/out/*.bin against an exact numpy (int64) reference

Variants (the constant side is 1-bit palettized unless the name says fp16):
  w1_docs_in[_fp16]   Y = D R^T: documents D (13088 x 767) fp16 input, rules R^T (767 x 444) constant
  w1_docs_i8 / _u8    as w1_docs_in with an int8 / uint8 input cast to fp16 on chip
  w1_rules_in         Y^T = R D^T: rules (444 x 767) fp16 input, documents D^T (767 x 13088) constant
  w1_packed{B}        documents as packed bits: B bits per fp16 value (13088 x ceil(767/B)), expanded on
                      chip with floor arithmetic into B blocks; R^T rows permuted to match the blocks
  w4_slice{R}         kind histogram of the first R*2048 named nodes: equal against 115 kind constants,
                      cast, reduce_sum over 2048-node rows (partials <= 2048), output (115, R)
  w5_plain            partials C_k = B[:, K_k] B[:, K_k]^T: B (2048 x 13088) fp16 input, K split in 7 chunks
                      (<= 1870 documents, so every partial count is <= 1870 and exact in fp16);
                      constants B[:, K_k]^T 1-bit; output (7, 2048, 2048) summed on the host in integers
  w5_packed{B}        B packed B bits per fp16 value (2048 x ceil(13088/B)); bit block k is the chunk of
                      documents {B*p + k}, so the chunks come for free (B blocks of <= 13088/B documents)
"""
import os
import sys
import time

import numpy as np
import coremltools as ct
import coremltools.optimize.coreml as cto
from coremltools.converters.mil import Builder as mb
from coremltools.converters.mil.mil import types

TARGET = ct.target.macOS15
F16 = types.fp16


def bits(path, rows, words, cols):
    w = np.fromfile(path, dtype="<u8").reshape(rows, words)
    b = np.unpackbits(w.view(np.uint8).reshape(rows, words * 8), axis=1, bitorder="little")
    return b[:, :cols].astype(np.uint8)


def load(data):
    D = bits(f"{data}/w1/docs_bits.u64", 13088, 12, 767)
    R = bits(f"{data}/w1/rules_bits.u64", 444, 12, 767)
    return D, R


def load5(data):
    return bits(f"{data}/w5/names_bits.u64", 2048, 205, 13088)


def convert(prog, st="p1"):
    m = ct.convert(prog, convert_to="mlprogram", compute_units=ct.ComputeUnit.CPU_AND_NE,
                   minimum_deployment_target=TARGET, compute_precision=ct.precision.FLOAT16)
    if st == "p1":
        cfg = cto.OpPalettizerConfig(mode="uniform", nbits=1, weight_threshold=0)
        m = cto.palettize_weights(m, cto.OptimizationConfig(global_config=cfg))
    return m


def pack(X, B):
    """X (rows, n) 0/1 -> (rows, P) fp16 values, value p holds columns B*p .. B*p+B-1 (bit k = column B*p+k)."""
    rows, n = X.shape
    P = -(-n // B)
    Xp = np.zeros((rows, P * B), np.int64)
    Xp[:, :n] = X
    v = (Xp.reshape(rows, P, B) << np.arange(B)).sum(2)
    assert v.max() < 2048 or B <= 11
    return v.astype(np.float16), P


def unpack_blocks(x, B):
    """On-chip expansion: x (rows, P) fp16 integers -> list of B (rows, P) 0/1 blocks, block k = bit k."""
    out = []
    q = x
    for k in range(B):
        q1 = mb.floor(x=mb.mul(x=q, y=np.float16(0.5)))
        out.append(mb.sub(x=q, y=mb.mul(x=q1, y=np.float16(2.0))))
        q = q1
    return out


def build(data, runs, v):
    io = f"{runs}/io/{v}"
    os.makedirs(io, exist_ok=True)
    path = f"{runs}/models/{v}.mlpackage"
    st = "fp16" if v.endswith("_fp16") else "p1"
    if v.startswith("w1"):
        D, R = load(data)
        if v.startswith("w1_docs_in"):
            W = R.T.astype(np.float16)  # 767 x 444

            @mb.program(input_specs=[mb.TensorSpec(shape=(13088, 767), dtype=F16)], opset_version=TARGET)
            def prog(x):
                return mb.matmul(x=x, y=W, name="y")

            D.astype(np.float16).tofile(f"{io}/x.bin")
        elif v in ("w1_docs_i8", "w1_docs_u8"):
            W = R.T.astype(np.float16)
            dt = types.int8 if v == "w1_docs_i8" else types.uint8

            @mb.program(input_specs=[mb.TensorSpec(shape=(13088, 767), dtype=dt)], opset_version=TARGET)
            def prog(x):
                return mb.matmul(x=mb.cast(x=x, dtype="fp16"), y=W, name="y")

            D.astype(np.float32).tofile(f"{io}/x.bin")  # Core ML presents int8/uint8 inputs as fp32 arrays
        elif v == "w1_rules_in":
            W = D.T.astype(np.float16)  # 767 x 13088

            @mb.program(input_specs=[mb.TensorSpec(shape=(444, 767), dtype=F16)], opset_version=TARGET)
            def prog(x):
                return mb.matmul(x=x, y=W, name="y")

            R.astype(np.float16).tofile(f"{io}/x.bin")
        elif v.startswith("w1_packed"):
            B = int(v[len("w1_packed"):])
            Xp, P = pack(D, B)
            Rp = np.zeros((444, P * B), np.uint8)
            Rp[:, :767] = R
            # block k holds columns B*p + k for p in 0..P-1; W rows in the same order
            Wk = [Rp[:, k::B].T.astype(np.float16) for k in range(B)]  # each P x 444
            Wcat = np.concatenate(Wk, axis=0)  # (B*P) x 444

            @mb.program(input_specs=[mb.TensorSpec(shape=(13088, P), dtype=F16)], opset_version=TARGET)
            def prog(x):
                blocks = unpack_blocks(x, B)
                xx = mb.concat(values=blocks, axis=1)
                return mb.matmul(x=xx, y=Wcat, name="y")

            Xp.tofile(f"{io}/x.bin")
        else:
            raise ValueError(v)
    elif v.startswith("w5"):
        Bm = load5(data)  # 2048 x 13088
        if v == "w5_plain":
            edges = np.linspace(0, 13088, 8).astype(int)
            chunks = [(edges[i], edges[i + 1]) for i in range(7)]
            Ws = [Bm[:, a:b].T.astype(np.float16) for a, b in chunks]

            @mb.program(input_specs=[mb.TensorSpec(shape=(2048, 13088), dtype=F16)], opset_version=TARGET)
            def prog(x):
                parts = []
                for (a, b), W in zip(chunks, Ws):
                    xs = mb.slice_by_index(x=x, begin=[0, int(a)], end=[2048, int(b)])
                    parts.append(mb.expand_dims(x=mb.matmul(x=xs, y=W), axes=[0]))
                return mb.concat(values=parts, axis=0, name="y")

            Bm.astype(np.float16).tofile(f"{io}/x.bin")
        elif v.startswith("w5_packed"):
            B = int(v[len("w5_packed"):])
            Xp, P = pack(Bm, B)
            Bp = np.zeros((2048, P * B), np.uint8)
            Bp[:, :13088] = Bm
            Ws = [Bp[:, k::B].T.astype(np.float16) for k in range(B)]  # P x 2048 each

            @mb.program(input_specs=[mb.TensorSpec(shape=(2048, P), dtype=F16)], opset_version=TARGET)
            def prog(x):
                blocks = unpack_blocks(x, B)
                parts = [mb.expand_dims(x=mb.matmul(x=b, y=W), axes=[0]) for b, W in zip(blocks, Ws)]
                return mb.concat(values=parts, axis=0, name="y")

            Xp.tofile(f"{io}/x.bin")
        else:
            raise ValueError(v)
    elif v.startswith("w4_slice"):
        rows = int(v[len("w4_slice"):])  # rows x 2048 node codes, the first rows*2048 named nodes
        codes = w4codes(data)[: rows * 2048]
        K = np.arange(115, dtype=np.float16).reshape(115, 1, 1)

        @mb.program(input_specs=[mb.TensorSpec(shape=(1, rows, 2048), dtype=F16)], opset_version=TARGET)
        def prog(x):
            e = mb.cast(x=mb.equal(x=x, y=K), dtype="fp16")  # 115 x rows x 2048
            return mb.reduce_sum(x=e, axes=[2], keep_dims=False, name="y")  # partials <= 2048

        codes.astype(np.float16).reshape(1, rows, 2048).tofile(f"{io}/x.bin")
        st = "fp16"
    else:
        raise ValueError(v)
    if not os.path.exists(path):
        convert(prog, st).save(path)
    print("built", v, "input bytes", os.path.getsize(f"{io}/x.bin"))


def w4codes(data):
    b = open(f"{data}/kinds.bin", "rb").read()
    n = int(np.frombuffer(b, "<u4", 1)[0])
    off = np.frombuffer(b, "<u8", n + 1, 4)
    return np.frombuffer(b, "<u2", int(off[-1]), 4 + 8 * (n + 1))


def check(data, runs, v):
    o = f"{runs}/io/{v}/out/y.bin"
    y = np.fromfile(o, dtype=np.float16)
    if v.startswith("w4"):
        rows = int(v[len("w4_slice"):])
        codes = w4codes(data)[: rows * 2048].reshape(rows, 2048).astype(np.int64)
        ex = np.stack([(codes == k).sum(1) for k in range(115)])
        got = y.reshape(115, rows).astype(np.int64)
        print(f"{v}: nodes {rows * 2048}, cells {ex.size}, wrong {(got != ex).sum()}, total {got.sum()} of {ex.sum()}")
        return
    if v.startswith("w1"):
        D, R = load(data)
        ex = D.astype(np.int64) @ R.T.astype(np.int64)  # 13088 x 444
        got = y.reshape(444, 13088).T if v == "w1_rules_in" else y.reshape(13088, 444)
        got = got.astype(np.int64)
        wrong = (got != ex).sum()
        print(f"{v}: cells {ex.size}, wrong {wrong}, nonzero exact {(ex != 0).sum()} got {(got != 0).sum()}, "
              f"max {ex.max()}")
    else:
        Bm = load5(data).astype(np.int64)
        ex = Bm @ Bm.T
        parts = y.reshape(-1, 2048, 2048)
        t = time.process_time()
        got = parts.astype(np.int64).sum(0)
        t = time.process_time() - t
        iu = np.triu_indices(2048, 1)
        print(f"{v}: partials {parts.shape[0]}, max partial {parts.max()}, wrong cells {(got != ex).sum()} of "
              f"{ex.size}, upper-triangle sum {got[iu].sum()} (expected 29397646, exact ref {ex[iu].sum()}); "
              f"host int64 sum of partials {t * 1e3:.1f} ms CPU (numpy)")


if __name__ == "__main__":
    data, runs, cmd, v = sys.argv[1:5]
    {"build": build, "check": check}[cmd](data, runs, v)
