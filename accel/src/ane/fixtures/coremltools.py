"""Dev check (never on the product path): writes the fixtures the writer is compared with, using coremltools 9.0.

Usage: python -I coremltools.py OUTDIR

The graph is the lane's bit-product template with two pieces: inputs x0 (64 x 128) and x1 (64 x 64), fp16;
constants W0 (128 x 24) and W1 (64 x 24), random 0/1, palettized to 1 bit (uniform, lossless for 0/1); outputs
y0 = x0 W0 and y1 = x1 W1. It writes coremltools-2piece.mlmodel (the Model protobuf with the metadata cleared,
serialized deterministically: map entries sorted by key) and coremltools-2piece.weight.bin. The Rust test
`writer_matches_coremltools` rebuilds the constant from the weight file and checks that the writer produces both
files byte for byte.
"""
import os, sys
import numpy as np
import coremltools as ct
import coremltools.optimize.coreml as cto
from coremltools.converters.mil import Builder as mb
from coremltools.converters.mil.mil import types
from coremltools.proto import Model_pb2

out = sys.argv[1]
M, K0, K1, N = 64, 128, 64, 24
rng = np.random.default_rng(1)
W0 = (rng.random((K0, N)) < 0.5).astype(np.float16)
W1 = (rng.random((K1, N)) < 0.5).astype(np.float16)


@mb.program(input_specs=[mb.TensorSpec(shape=(M, K0), dtype=types.fp16), mb.TensorSpec(shape=(M, K1), dtype=types.fp16)],
            opset_version=ct.target.macOS15)
def prog(x0, x1):
    return mb.matmul(x=x0, y=W0, name="y0"), mb.matmul(x=x1, y=W1, name="y1")


m = ct.convert(prog, convert_to="mlprogram", compute_units=ct.ComputeUnit.CPU_AND_NE,
               minimum_deployment_target=ct.target.macOS15, compute_precision=ct.precision.FLOAT16)
m = cto.palettize_weights(m, cto.OptimizationConfig(global_config=cto.OpPalettizerConfig(mode="uniform", nbits=1, weight_threshold=0)))
pkg = os.path.join(out, "ref.mlpackage")
m.save(pkg)
spec = Model_pb2.Model()
spec.ParseFromString(open(os.path.join(pkg, "Data/com.apple.CoreML/model.mlmodel"), "rb").read())
spec.description.ClearField("metadata")
open(os.path.join(out, "coremltools-2piece.mlmodel"), "wb").write(spec.SerializeToString(deterministic=True))
open(os.path.join(out, "coremltools-2piece.weight.bin"), "wb").write(open(os.path.join(pkg, "Data/com.apple.CoreML/weights/weight.bin"), "rb").read())
