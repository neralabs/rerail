//   swiftc -O scripts/dev/metal/exit256.swift -o "$TMPDIR/exit256"   # once, outside the tree
//   "$TMPDIR/exit256" 66; "$TMPDIR/exit256" 67   # SAME both, but DIFF at 67 on a G13 (decision 0672)
//
// exit256 N [SRC_OUT]: a kernel of N inlined calls of a loop with an early return, run on one thread, its result
// set against the CPU's. On Apple G13 (M1) the compiler numbers each loop's exits in a 16-bit register and tests
// them with an 8-bit immediate; past 255 exits a thread takes no exit and leaves the function.
import Metal
import Foundation
let n = Int(CommandLine.arguments[1])!
let len = 8
var src = """
#include <metal_stdlib>
using namespace metal;
// Two exits per loop that lead to different code: the early return skips the second search and its store.
static inline uint find(device const uchar* p, uint n, uint key, device uint* out) {
    for (uint i = 0; i < n; i++) { if (p[i] == key) return i; out[1u + (i & 7u)] += 1u; }
    out[9] = key;
    for (uint i = 0; i < n; i++) if (p[i] == (key ^ 8u)) return 100u + i;
    return 0xffffffffu;
}
kernel void k(device const uchar* p [[buffer(0)]], device uint* out [[buffer(1)]], constant uint& n [[buffer(2)]]) {
    uint acc = 0u;

"""
for j in 0..<n { src += "    acc = acc * 31u + find(p, n, (acc + \(j)u) & 15u, out);\n" }
src += "    out[0] = acc;\n}\n"
if CommandLine.arguments.count > 2 { try! src.write(toFile: CommandLine.arguments[2], atomically: true, encoding: .utf8) }
// The bytes: 8 of 0..15, so half the keys are found by the first loop, the others (key ^ 8) by the second.
let bytes: [UInt8] = (0..<len).map { UInt8(($0 * 7 + 3) & 15) }
var want: UInt32 = 0
for j in 0..<n {
    let key = UInt8((want &+ UInt32(j)) & 15)
    let r = bytes.firstIndex(of: key).map { UInt32($0) } ?? bytes.firstIndex(of: key ^ 8).map { 100 + UInt32($0) } ?? 0xffffffff
    want = want &* 31 &+ r
}
let dev = MTLCreateSystemDefaultDevice()!
let t0 = Date()
let lib = try! dev.makeLibrary(source: src, options: MTLCompileOptions())
let pso = try! dev.makeComputePipelineState(function: lib.makeFunction(name: "k")!)
let compile = Date().timeIntervalSince(t0)
let p = dev.makeBuffer(bytes: bytes, length: len, options: .storageModeShared)!
let out = dev.makeBuffer(length: 64, options: .storageModeShared)!
var count = UInt32(len)
let q = dev.makeCommandQueue()!, cb = q.makeCommandBuffer()!, e = cb.makeComputeCommandEncoder()!
e.setComputePipelineState(pso)
e.setBuffer(p, offset: 0, index: 0); e.setBuffer(out, offset: 0, index: 1); e.setBytes(&count, length: 4, index: 2)
e.dispatchThreadgroups(MTLSize(width: 1, height: 1, depth: 1), threadsPerThreadgroup: MTLSize(width: 1, height: 1, depth: 1))
e.endEncoding(); cb.commit(); cb.waitUntilCompleted()
let got = out.contents().load(as: UInt32.self)
print(String(format: "%@ N=%d: got %08x want %08x %@ (compile %.1f s)", dev.name, n, got, want, got == want ? "SAME" : "DIFF", compile))
