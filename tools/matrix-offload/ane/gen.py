"""Builds the ANE probe models (Core ML ML programs) from their names.

Usage: python -I gen.py OUTDIR NAME [NAME ...]

Each NAME encodes the model; the .mlpackage is written to OUTDIR/NAME.mlpackage (skipped when it exists).
Grammar (all tensors fp16 unless noted; constant matrices W/A are random 0/1, seeded):
  add_{n}                     y = x + 1, x (1, n)
  add4_{C}_{W}                y = x + z, x and z inputs (1, C, 1, W)
  mm_{M}_{K}_{N}_{st}_{form}  Y = X W, X input (M, K), W constant (K, N) 0/1
                              st: fp16 | int8 (linear symmetric) | p4 | p2 | p1 (uniform palette, lossless for 0/1)
                              form: mm (matmul), lin (linear, weight stored N x K), conv (1x1 conv,
                              X given as (1, K, 1, M), Y as (1, N, 1, M))
  dmm_{M}_{K}_{N}             Y = X Y2, both inputs (no constants)
  dconv_{M}_{K}_{N}           1x1 conv with the weight as an input (1,K,1,M) * (N,K,1,1)
  cnt_{M}_{K}_{N}             counting exactness: X (M,K), W (K,N) inputs; outputs y = X W,
                              gt = cast(greater(y, 0.5)), cl = clip(y, 0, 1), rs = reduce_sum(X, axis 1)
  rsum_{M}_{K}                rs = reduce_sum(X, axis 1) only
  eq_{N}_{M}_{how}            counts[i] = sum_j (a[i] == b[j]); a (N, 1), b (1, M); how: sel | cast | sub
                              (sub: 1 - min(|a-b|, 1), no compare op)
  eq2_{N}_{M}_{how}           two-digit key: (ahi, alo) vs (bhi, blo), eq = eq_hi * eq_lo
  chain_{M}_{n}_{k}_{st}_{kind}  k steps over A constant (n, n) 0/1 (sparse, density 2/n), Y input (M, n)
                              kind: clip  Y = clip(Y A, 0, 1);  acc  Y = min(Y + Y A, 1);
                              clipd / accd: same with A dense (density 0.5)
  gat_{V}_{D}_{Q}             rows = gather(T, idx, axis 0), T constant (V, D), idx int32 (Q,)
  gnd_{V}_{D}_{Q}             gather_nd with idx (Q, 1)
  gin_{V}_{D}_{Q}             gather from an input table T (V, D)
  topk_{n}_{k}                topk over x (1, n), values and indices
  sort_{n}                    argsort over x (1, n)
"""
import os
import sys

import numpy as np
import coremltools as ct
import coremltools.optimize.coreml as cto
from coremltools.converters.mil import Builder as mb
from coremltools.converters.mil.mil import types

TARGET = ct.target.macOS15
F16 = types.fp16


def rand01(shape, seed, density=0.5):
    rng = np.random.default_rng(seed)
    return (rng.random(shape) < density).astype(np.float16)


def convert(prog):
    return ct.convert(
        prog,
        convert_to="mlprogram",
        compute_units=ct.ComputeUnit.CPU_AND_NE,
        minimum_deployment_target=TARGET,
        compute_precision=ct.precision.FLOAT16,
    )


def compress(m, st):
    if st == "fp16":
        return m
    if st == "int8":
        cfg = cto.OpLinearQuantizerConfig(mode="linear_symmetric", dtype="int8", weight_threshold=0)
        return cto.linear_quantize_weights(m, cto.OptimizationConfig(global_config=cfg))
    if st.startswith("p"):
        cfg = cto.OpPalettizerConfig(mode="uniform", nbits=int(st[1:]), weight_threshold=0)
        return cto.palettize_weights(m, cto.OptimizationConfig(global_config=cfg))
    raise ValueError(st)


def build(name):
    p = name.split("_")
    kind = p[0]
    if kind == "add":
        n = int(p[1])

        @mb.program(input_specs=[mb.TensorSpec(shape=(1, n), dtype=F16)], opset_version=TARGET)
        def prog(x):
            return mb.add(x=x, y=np.float16(1.0), name="y")

        return convert(prog)

    if kind == "add4":
        C, Wd = int(p[1]), int(p[2])

        @mb.program(input_specs=[mb.TensorSpec(shape=(1, C, 1, Wd), dtype=F16), mb.TensorSpec(shape=(1, C, 1, Wd), dtype=F16)], opset_version=TARGET)
        def prog(x, z):
            return mb.add(x=x, y=z, name="y")

        return convert(prog)

    if kind == "mm":
        M, K, N = map(int, p[1:4])
        st, form = p[4], p[5]
        W = rand01((K, N), 1)
        if form == "mm":
            @mb.program(input_specs=[mb.TensorSpec(shape=(M, K), dtype=F16)], opset_version=TARGET)
            def prog(x):
                return mb.matmul(x=x, y=W, name="y")
        elif form == "lin":
            Wt = np.ascontiguousarray(W.T)

            @mb.program(input_specs=[mb.TensorSpec(shape=(M, K), dtype=F16)], opset_version=TARGET)
            def prog(x):
                return mb.linear(x=x, weight=Wt, name="y")
        elif form == "conv":
            Wc = np.ascontiguousarray(W.T).reshape(N, K, 1, 1)

            @mb.program(input_specs=[mb.TensorSpec(shape=(1, K, 1, M), dtype=F16)], opset_version=TARGET)
            def prog(x):
                return mb.conv(x=x, weight=Wc, name="y")
        else:
            raise ValueError(form)
        del W
        return compress(convert(prog), st)

    if kind == "dmm":
        M, K, N = map(int, p[1:4])

        @mb.program(input_specs=[mb.TensorSpec(shape=(M, K), dtype=F16), mb.TensorSpec(shape=(K, N), dtype=F16)],
                    opset_version=TARGET)
        def prog(x, w):
            return mb.matmul(x=x, y=w, name="y")

        return convert(prog)

    if kind == "dconv":
        M, K, N = map(int, p[1:4])

        @mb.program(input_specs=[mb.TensorSpec(shape=(1, K, 1, M), dtype=F16),
                                 mb.TensorSpec(shape=(N, K, 1, 1), dtype=F16)], opset_version=TARGET)
        def prog(x, w):
            return mb.conv(x=x, weight=w, name="y")

        return convert(prog)

    if kind == "cnt":
        M, K, N = map(int, p[1:4])

        @mb.program(input_specs=[mb.TensorSpec(shape=(M, K), dtype=F16), mb.TensorSpec(shape=(K, N), dtype=F16)],
                    opset_version=TARGET)
        def prog(x, w):
            y = mb.matmul(x=x, y=w, name="y")
            gt = mb.cast(x=mb.greater(x=y, y=np.float16(0.5)), dtype="fp16", name="gt")
            cl = mb.clip(x=y, alpha=np.float16(0.0), beta=np.float16(1.0), name="cl")
            rs = mb.reduce_sum(x=x, axes=[1], keep_dims=True, name="rs")
            return y, gt, cl, rs

        return convert(prog)

    if kind == "rsum":
        M, K = map(int, p[1:3])

        @mb.program(input_specs=[mb.TensorSpec(shape=(M, K), dtype=F16)], opset_version=TARGET)
        def prog(x):
            return mb.reduce_sum(x=x, axes=[1], keep_dims=True, name="rs")

        return convert(prog)

    if kind in ("eq", "eq2"):
        N, M = int(p[1]), int(p[2])
        how = p[3]

        def eqmat(a, b):
            if how == "sel":
                return mb.select(cond=mb.equal(x=a, y=b), a=np.ones((1, M), np.float16), b=np.zeros((1, M), np.float16))
            if how == "cast":
                return mb.cast(x=mb.equal(x=a, y=b), dtype="fp16")
            if how == "sub":
                d = mb.abs(x=mb.sub(x=a, y=b))
                return mb.sub(x=np.float16(1.0), y=mb.minimum(x=d, y=np.float16(1.0)))
            raise ValueError(how)

        if kind == "eq":
            @mb.program(input_specs=[mb.TensorSpec(shape=(N, 1), dtype=F16), mb.TensorSpec(shape=(1, M), dtype=F16)],
                        opset_version=TARGET)
            def prog(a, b):
                return mb.reduce_sum(x=eqmat(a, b), axes=[1], keep_dims=True, name="counts")
        else:
            @mb.program(input_specs=[mb.TensorSpec(shape=(N, 1), dtype=F16), mb.TensorSpec(shape=(N, 1), dtype=F16),
                                     mb.TensorSpec(shape=(1, M), dtype=F16), mb.TensorSpec(shape=(1, M), dtype=F16)],
                        opset_version=TARGET)
            def prog(ahi, alo, bhi, blo):
                e = mb.mul(x=eqmat(ahi, bhi), y=eqmat(alo, blo))
                return mb.reduce_sum(x=e, axes=[1], keep_dims=True, name="counts")

        return convert(prog)

    if kind == "chain":
        M, n, k = int(p[1]), int(p[2]), int(p[3])
        st, how = p[4], p[5]
        dense = how.endswith("d")  # clipd / accd: A dense (density 0.5) to expose any zero-skipping
        how = how.rstrip("d") if dense else how
        A = rand01((n, n), 7, density=0.5 if dense else 2.0 / n)
        np.fill_diagonal(A, 0)

        @mb.program(input_specs=[mb.TensorSpec(shape=(M, n), dtype=F16)], opset_version=TARGET)
        def prog(y):
            Ac = mb.const(val=A, name="A")
            for i in range(k):
                ya = mb.matmul(x=y, y=Ac)
                if how == "clip":
                    y = mb.clip(x=ya, alpha=np.float16(0.0), beta=np.float16(1.0))
                else:
                    y = mb.minimum(x=mb.add(x=y, y=ya), y=np.float16(1.0))
            return mb.identity(x=y, name="out")

        return compress(convert(prog), st)

    if kind in ("gat", "gnd", "gin"):
        V, D, Q = map(int, p[1:4])
        T = np.random.default_rng(3).standard_normal((V, D)).astype(np.float16)
        if kind == "gat":
            @mb.program(input_specs=[mb.TensorSpec(shape=(Q,), dtype=types.int32)], opset_version=TARGET)
            def prog(idx):
                return mb.gather(x=T, indices=idx, axis=0, name="rows")
        elif kind == "gnd":
            @mb.program(input_specs=[mb.TensorSpec(shape=(Q, 1), dtype=types.int32)], opset_version=TARGET)
            def prog(idx):
                return mb.gather_nd(x=T, indices=idx, name="rows")
        else:
            @mb.program(input_specs=[mb.TensorSpec(shape=(V, D), dtype=F16), mb.TensorSpec(shape=(Q,), dtype=types.int32)],
                        opset_version=TARGET)
            def prog(t, idx):
                return mb.gather(x=t, indices=idx, axis=0, name="rows")
        return convert(prog)

    if kind == "topk":
        n, k = int(p[1]), int(p[2])

        @mb.program(input_specs=[mb.TensorSpec(shape=(1, n), dtype=F16)], opset_version=TARGET)
        def prog(x):
            v, i = mb.topk(x=x, k=k, axis=-1, name="tk")
            return mb.identity(x=v, name="vals"), mb.identity(x=i, name="idx")

        return convert(prog)

    if kind == "sort":
        n = int(p[1])

        @mb.program(input_specs=[mb.TensorSpec(shape=(1, n), dtype=F16)], opset_version=TARGET)
        def prog(x):
            return mb.argsort(x=x, axis=-1, name="order")

        return convert(prog)

    raise ValueError(name)


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name in sys.argv[2:]:
        path = os.path.join(out, name + ".mlpackage")
        if os.path.exists(path):
            continue
        m = build(name)
        m.save(path)
        print("built", name, flush=True)


if __name__ == "__main__":
    main()
