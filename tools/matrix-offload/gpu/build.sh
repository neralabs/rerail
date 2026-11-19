#!/bin/sh
# Build the GPU probe harness. Shaders are compiled at run time (no offline Metal toolchain needed).
# Output: $OUT (default: ./bin/gpuprobe, gitignored).
set -e
here=$(cd "$(dirname "$0")" && pwd)
out=${OUT:-$here/bin/gpuprobe}
mkdir -p "$(dirname "$out")"
xcrun swiftc -O -framework Metal -framework MetalPerformanceShaders "$here/gpuprobe.swift" -o "$out"
echo "built $out"
