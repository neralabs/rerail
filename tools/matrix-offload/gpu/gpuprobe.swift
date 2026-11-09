// GPU microprobes for relational kernels restated as dense work (G1..G6).
// Usage: gpuprobe <shader-dir> <probe> [quick]
// Primary measure: MTLCommandBuffer gpuEndTime - gpuStartTime. Also host CPU (getrusage, whole process,
// includes Metal's own threads) and wall time per command buffer. Every kernel is checked against a CPU
// reference before it is timed; a failed check prints FAIL and the row is not timed.
import Foundation
import Metal
import MetalPerformanceShaders

let dev = MTLCreateSystemDefaultDevice()!
let queue = dev.makeCommandQueue()!
let args = CommandLine.arguments
let shaderDir = args.count > 1 ? args[1] : "shaders"
let probe = args.count > 2 ? args[2] : "all"
let quick = args.contains("quick")

// MARK: - infrastructure

var libCache: [String: MTLLibrary] = [:]
func pipe(_ file: String, _ fn: String, _ macros: [String: String] = [:]) throws -> MTLComputePipelineState {
  let key = file + "|" + macros.sorted { $0.key < $1.key }.map { "\($0.key)=\($0.value)" }.joined(separator: ",")
  if libCache[key] == nil {
    let src = try String(contentsOfFile: shaderDir + "/" + file, encoding: .utf8)
    let o = MTLCompileOptions()
    o.languageVersion = .version4_0
    var m: [String: NSObject] = [:]
    for (k, v) in macros { m[k] = v as NSString }
    o.preprocessorMacros = m
    libCache[key] = try dev.makeLibrary(source: src, options: o)
  }
  guard let f = libCache[key]!.makeFunction(name: fn) else {
    throw NSError(domain: "gpuprobe", code: 1, userInfo: [NSLocalizedDescriptionKey: "no function \(fn)"])
  }
  return try dev.makeComputePipelineState(function: f)
}

struct Sample { var gpu: Double; var cpu: Double; var wall: Double }  // milliseconds
func nowMs() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }
func cpuMs() -> Double {
  var r = rusage()
  getrusage(RUSAGE_SELF, &r)
  return Double(r.ru_utime.tv_sec + r.ru_stime.tv_sec) * 1e3 + Double(r.ru_utime.tv_usec + r.ru_stime.tv_usec) / 1e3
}
func loadAvg() -> String {
  var l = [Double](repeating: 0, count: 3)
  getloadavg(&l, 3)
  return String(format: "%.0f %.0f %.0f", l[0], l[1], l[2])
}
func med(_ x: [Double]) -> Double { let s = x.sorted(); return s.isEmpty ? .nan : s[s.count / 2] }

// Optional GPU pre-warm (env GPUPROBE_WARM_MS): before each timed series, keep the GPU busy with a
// streaming copy for about that long so DVFS has ramped the clock. Off by default (cold = bursty reality).
let warmMs = Double(ProcessInfo.processInfo.environment["GPUPROBE_WARM_MS"] ?? "0") ?? 0
var warmBufs: (MTLBuffer, MTLBuffer, MTLComputePipelineState)? = nil
func warmGPU() {
  if warmMs <= 0 { return }
  if warmBufs == nil {
    warmBufs = (buf(32 << 20), buf(32 << 20), try! pipe("g2_stream.metal", "copy_k"))
  }
  let (a, b, p) = warmBufs!
  let reps = max(1, Int(warmMs / 0.15))  // ~0.15 ms per 64 MB moved at ~450 GB/s
  let cb = queue.makeCommandBuffer()!
  for _ in 0..<reps { enc(cb, p, [a, b], [UInt32((32 << 20) / 16), 4], tgs: sz((32 << 20) / 16 / 4 / 256), tpt: sz(256)) }
  cb.commit(); cb.waitUntilCompleted()
}

@discardableResult
func timeCB(_ reps: Int, warm: Int = 1, _ encode: (MTLCommandBuffer) -> Void) -> [Sample] {
  var out: [Sample] = []
  if reps > 1 { warmGPU() }
  for r in 0..<(reps + warm) {
    let c0 = cpuMs(), w0 = nowMs()
    let cb = queue.makeCommandBuffer()!
    encode(cb)
    cb.commit()
    cb.waitUntilCompleted()
    let w1 = nowMs(), c1 = cpuMs()
    if let e = cb.error { print("  CB error: \(e)") }
    if r >= warm { out.append(Sample(gpu: (cb.gpuEndTime - cb.gpuStartTime) * 1e3, cpu: c1 - c0, wall: w1 - w0)) }
  }
  return out
}

func enc(_ cb: MTLCommandBuffer, _ p: MTLComputePipelineState, _ bufs: [MTLBuffer], _ u32: [UInt32] = [],
         tgs: MTLSize, tpt: MTLSize) {
  let e = cb.makeComputeCommandEncoder()!
  e.setComputePipelineState(p)
  for (i, b) in bufs.enumerated() { e.setBuffer(b, offset: 0, index: i) }
  for (j, v) in u32.enumerated() { var x = v; e.setBytes(&x, length: 4, index: bufs.count + j) }
  e.dispatchThreadgroups(tgs, threadsPerThreadgroup: tpt)
  e.endEncoding()
}
func sz(_ x: Int, _ y: Int = 1, _ z: Int = 1) -> MTLSize { MTLSize(width: x, height: y, depth: z) }
func buf(_ bytes: Int) -> MTLBuffer { dev.makeBuffer(length: max(bytes, 16), options: .storageModeShared)! }
func ptr<T>(_ b: MTLBuffer, _ t: T.Type) -> UnsafeMutablePointer<T> {
  b.contents().bindMemory(to: T.self, capacity: b.length / MemoryLayout<T>.stride)
}

struct Rng {
  var s: UInt64
  mutating func next() -> UInt64 {
    s &+= 0x9E37_79B9_7F4A_7C15
    var z = s
    z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
    z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
    return z ^ (z >> 31)
  }
  mutating func below(_ n: Int) -> Int { Int(next() % UInt64(n)) }
}
func fillRandom(_ b: MTLBuffer, seed: UInt64) {
  let n = b.length / 8
  let p = ptr(b, UInt64.self)
  var r = Rng(s: seed)
  for i in 0..<n { p[i] = r.next() }
}

/// One table row: label, device time min/median, host CPU and wall medians, work rate and bytes/s.
func row(_ label: String, _ s: [Sample], work: Double = 0, unit: String = "", bytes: Double = 0) {
  let g = s.map { $0.gpu }, gmin = g.min() ?? .nan, gmed = med(g)
  var line = String(format: "| %@ | %.3f | %.3f | %.3f | %.3f / %.3f |", label, gmin, gmed, med(s.map { $0.cpu }), s.map { $0.wall }.min() ?? .nan, med(s.map { $0.wall }))
  if work > 0 { line += String(format: " %.4g %@ |", work / (gmin / 1e3), unit) } else { line += " |" }
  if bytes > 0 {
    line += String(format: " %.1f / %.1f |", bytes / (gmin / 1e3) / 1e9, bytes / (gmed / 1e3) / 1e9)
  } else { line += " |" }
  print(line)
  fflush(stdout)
}
func header(_ title: String, _ workCol: String) {
  print("\n### \(title)  (load avg \(loadAvg()))\n")
  print("| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | \(workCol) (at min) | GB/s min / med |")
  print("|---|---|---|---|---|---|---|")
  fflush(stdout)
}
func note(_ s: String) { print(s); fflush(stdout) }

// MARK: - G1 dispatch floor

func g1() throws {
  let p = try pipe("g1_empty.metal", "empty_k")
  let o = buf(16)
  header("G1 dispatch floor (empty kernel, 1 threadgroup x 32 threads unless noted)", "per dispatch")
  let reps = quick ? 30 : 200
  let s1 = timeCB(reps) { cb in enc(cb, p, [o], tgs: sz(1), tpt: sz(32)) }
  row("1 dispatch / CB, waitUntilCompleted", s1)
  let sBig = timeCB(reps / 4) { cb in enc(cb, p, [o], tgs: sz(32768), tpt: sz(256)) }
  row("1 dispatch / CB, 8M threads (32768 TG x 256)", sBig)
  for n in [10, 100, 1000] {
    let s = timeCB(quick ? 5 : 20) { cb in
      let e = cb.makeComputeCommandEncoder()!
      e.setComputePipelineState(p); e.setBuffer(o, offset: 0, index: 0)
      for _ in 0..<n { e.dispatchThreadgroups(sz(1), threadsPerThreadgroup: sz(32)) }
      e.endEncoding()
    }
    row("\(n) dispatches, 1 serial encoder (dev ms total)", s, work: Double(n), unit: "disp/s")
    let sc = timeCB(quick ? 5 : 20) { cb in
      let e = cb.makeComputeCommandEncoder(dispatchType: .concurrent)!
      e.setComputePipelineState(p); e.setBuffer(o, offset: 0, index: 0)
      for _ in 0..<n { e.dispatchThreadgroups(sz(1), threadsPerThreadgroup: sz(32)) }
      e.endEncoding()
    }
    row("\(n) dispatches, 1 concurrent encoder", sc, work: Double(n), unit: "disp/s")
    let se = timeCB(quick ? 5 : 20) { cb in for _ in 0..<n { enc(cb, p, [o], tgs: sz(1), tpt: sz(32)) } }
    row("\(n) dispatches, \(n) encoders", se, work: Double(n), unit: "disp/s")
  }
  // Completion handler, serial: commit one CB, block on a semaphore signalled from its handler.
  let sem = DispatchSemaphore(value: 0)
  var hs: [Sample] = []
  for r in 0..<(reps + 1) {
    let c0 = cpuMs(), w0 = nowMs()
    let cb = queue.makeCommandBuffer()!
    enc(cb, p, [o], tgs: sz(1), tpt: sz(32))
    cb.addCompletedHandler { _ in sem.signal() }
    cb.commit()
    sem.wait()
    let w1 = nowMs(), c1 = cpuMs()
    if r > 0 { hs.append(Sample(gpu: (cb.gpuEndTime - cb.gpuStartTime) * 1e3, cpu: c1 - c0, wall: w1 - w0)) }
  }
  row("1 dispatch / CB, completion handler + semaphore", hs)
  // Pipelined: commit N CBs back to back, handlers count down; per-CB amortised wall and CPU.
  for n in [16, 256] {
    var walls: [Double] = [], cpus: [Double] = [], gpus: [Double] = []
    for _ in 0..<(quick ? 3 : 7) {
      let group = DispatchGroup()
      let c0 = cpuMs(), w0 = nowMs()
      var cbs: [MTLCommandBuffer] = []
      for _ in 0..<n {
        let cb = queue.makeCommandBuffer()!
        enc(cb, p, [o], tgs: sz(1), tpt: sz(32))
        group.enter()
        cb.addCompletedHandler { _ in group.leave() }
        cb.commit()
        cbs.append(cb)
      }
      group.wait()
      let w1 = nowMs(), c1 = cpuMs()
      walls.append((w1 - w0) / Double(n)); cpus.append((c1 - c0) / Double(n))
      gpus.append((cbs.last!.gpuEndTime - cbs.first!.gpuStartTime) * 1e3 / Double(n))
    }
    print(String(format: "| %d CBs in flight, handlers (per CB: first-start..last-end span / n) | %.4f | %.4f | %.4f | %.4f | %.4g CB/s | |",
                 n, gpus.min()!, med(gpus), med(cpus), med(walls), 1e3 / med(walls)))
  }
  note("\nload avg after G1: \(loadAvg())")
}

// MARK: - G2 bandwidth

func g2() throws {
  let pr = try pipe("g2_stream.metal", "reduce_k")
  let pc = try pipe("g2_stream.metal", "copy_k")
  header("G2 DRAM bandwidth (shared-storage buffers unless noted)", "Gelem/s")
  for mb in (quick ? [256] : [4, 16, 32, 64, 128, 256, 512]) {
    autoreleasepool {
      let bytes = mb << 20
      let a = buf(bytes)
      fillRandom(a, seed: 1)
      let n4 = bytes / 16
      let ref = { () -> UInt32 in var s: UInt32 = 0; let q = ptr(a, UInt32.self); for i in 0..<(bytes / 4) { s &+= q[i] }; return s }()
      for iters in (mb >= 256 ? [4, 16, 64] : [16]) {
        let threads = (n4 + iters - 1) / iters
        let tgs = (threads + 255) / 256
        let part = buf(tgs * 4)
        timeCB(1) { cb in enc(cb, pr, [a, part], [UInt32(n4), UInt32(iters)], tgs: sz(tgs), tpt: sz(256)) }
        var s: UInt32 = 0
        let q = ptr(part, UInt32.self)
        for i in 0..<tgs { s &+= q[i] }
        if s != ref { note("| reduce \(mb) MB iters=\(iters) | FAIL got \(s) want \(ref) |"); continue }
        let t = timeCB(quick ? 3 : 10) { cb in for _ in 0..<g4Repeat { enc(cb, pr, [a, part], [UInt32(n4), UInt32(iters)], tgs: sz(tgs), tpt: sz(256)) } }
        row("read+reduce \(mb) MB, \(iters) uint4/thread\(g4Repeat > 1 ? " x\(g4Repeat) per CB" : "")", t, work: Double(g4Repeat * bytes / 4) / 1e9, unit: "Gu32/s", bytes: Double(g4Repeat * bytes))
      }
      if mb < 256 { return }
      // private-storage copy of the same data
      let ap = dev.makeBuffer(length: bytes, options: .storageModePrivate)!
      timeCB(1, warm: 0) { cb in let b = cb.makeBlitCommandEncoder()!; b.copy(from: a, sourceOffset: 0, to: ap, destinationOffset: 0, size: bytes); b.endEncoding() }
      let tgs = (n4 / 16 + 255) / 256
      let part = buf(tgs * 4)
      timeCB(1) { cb in enc(cb, pr, [ap, part], [UInt32(n4), 16], tgs: sz(tgs), tpt: sz(256)) }
      var s: UInt32 = 0
      for i in 0..<tgs { s &+= ptr(part, UInt32.self)[i] }
      if s == ref {
        let t = timeCB(quick ? 3 : 10) { cb in enc(cb, pr, [ap, part], [UInt32(n4), 16], tgs: sz(tgs), tpt: sz(256)) }
        row("read+reduce \(mb) MB PRIVATE, 16 uint4/thread", t, work: Double(bytes / 4) / 1e9, unit: "Gu32/s", bytes: Double(bytes))
      } else { note("| reduce private FAIL |") }
    }
  }
  // copy: 256 MB -> 256 MB (512 MB moved)
  autoreleasepool {
    let bytes = 256 << 20, n4 = bytes / 16
    let a = buf(bytes), b = buf(bytes)
    fillRandom(a, seed: 2)
    for iters in [4, 16] {
      let tgs = (n4 / iters + 255) / 256
      memset(b.contents(), 0, bytes)
      timeCB(1, warm: 0) { cb in enc(cb, pc, [a, b], [UInt32(n4), UInt32(iters)], tgs: sz(tgs), tpt: sz(256)) }
      if memcmp(a.contents(), b.contents(), bytes) != 0 { note("| copy FAIL |"); continue }
      let t = timeCB(quick ? 3 : 10) { cb in enc(cb, pc, [a, b], [UInt32(n4), UInt32(iters)], tgs: sz(tgs), tpt: sz(256)) }
      row("copy kernel 256 MB -> 256 MB, \(iters) uint4/thread (bytes = read+write)", t, bytes: Double(2 * bytes))
    }
    memset(b.contents(), 0, bytes)
    let blit = { (cb: MTLCommandBuffer) in let e = cb.makeBlitCommandEncoder()!; e.copy(from: a, sourceOffset: 0, to: b, destinationOffset: 0, size: bytes); e.endEncoding() }
    timeCB(1, warm: 0, blit)
    if memcmp(a.contents(), b.contents(), bytes) == 0 {
      row("blit copy 256 MB -> 256 MB (read+write)", timeCB(quick ? 3 : 10, blit), bytes: Double(2 * bytes))
    } else { note("| blit FAIL |") }
  }
  note("\nload avg after G2: \(loadAvg())")
}

// MARK: - G3 bit-matrix AND + popcount

func g3() throws {
  header("G3 bit-matrix C[i][j] = popcount(A_i & B_j), n = m; output u32", "Gbit-op/s")
  let variants: [(String, String, [String: String], Int)] = [
    ("naive (1 thread/output)", "bm_naive", [:], 0),
    ("tiled 64x64, 4x4/thread, KC=16 (8 KB tg)", "bm_tiled", ["TM": "4", "KC": "16"], 64),
    ("tiled 128x128, 8x8/thread, KC=8 (8 KB tg)", "bm_tiled", ["TM": "8", "KC": "8"], 128),
  ]
  var pipes: [MTLComputePipelineState] = []
  for v in variants { pipes.append(try pipe("g3_bitmm.metal", v.1, v.2)) }
  let shapes: [(Int, Int)] = quick ? [(1024, 1024), (4096, 4096)] :
    [(1024, 1024), (1024, 16384), (1024, 65536), (2048, 4096), (4096, 4096), (4096, 16384), (8192, 2048), (8192, 8192)]
  for (n, kbits) in shapes {
    autoreleasepool {
      let kw = kbits / 32
      let A = buf(n * kw * 4), B = buf(n * kw * 4), C = buf(n * n * 4)
      fillRandom(A, seed: 3); fillRandom(B, seed: 4)
      let pa = ptr(A, UInt32.self), pb = ptr(B, UInt32.self), pc = ptr(C, UInt32.self)
      var rng = Rng(s: 9)
      let probes = (0..<256).map { _ in (rng.below(n), rng.below(n)) }
      let refs = probes.map { (i, j) -> UInt32 in
        var c: UInt32 = 0
        for w in 0..<kw { c += UInt32((pa[i * kw + w] & pb[j * kw + w]).nonzeroBitCount) }
        return c
      }
      let bitops = Double(n) * Double(n) * Double(kbits)
      let uniq = Double(2 * n * kw * 4 + n * n * 4)
      for (vi, v) in variants.enumerated() {
        if v.0.hasPrefix("naive") && bitops > 1.2e12 { continue }  // too slow for the time cap
        let p = pipes[vi]
        let encode: (MTLCommandBuffer) -> Void = { cb in
          if v.3 == 0 {
            enc(cb, p, [A, B, C], [UInt32(n), UInt32(n), UInt32(kw)], tgs: sz((n + 15) / 16, (n + 15) / 16), tpt: sz(16, 16))
          } else {
            enc(cb, p, [A, B, C], [UInt32(n), UInt32(n), UInt32(kw)], tgs: sz(n / v.3, n / v.3), tpt: sz(16, 16))
          }
        }
        memset(C.contents(), 0xff, n * n * 4)
        timeCB(1, warm: 0, encode)
        var ok = true
        for (q, (i, j)) in probes.enumerated() where pc[i * n + j] != refs[q] { ok = false; break }
        if !ok { note("| \(n)x\(n) k=\(kbits) \(v.0) | FAIL (sampled check) |"); continue }
        let reps = bitops > 1e12 ? 3 : (quick ? 3 : 7)
        row("n=m=\(n), k=\(kbits) bits: \(v.0)", timeCB(reps, encode), work: bitops / 1e9, unit: "Gbit-op/s", bytes: uniq)
      }
    }
  }
  note("(bytes = unique operand + output bytes, A + B + C, once; 256 random (i,j) outputs checked per case)")
  note("\nload avg after G3: \(loadAvg())")
}

// MARK: - G4 matrix units

// G4_REPEAT=k encodes each MPP matmul (and each G2 reduce) k times per command buffer (sustained-clock check).
let g4Repeat = Int(ProcessInfo.processInfo.environment["G4_REPEAT"] ?? "1") ?? 1
func g4() throws {
  header("G4a Metal 4 tensor ops (MPP matmul2d) square M=N=K; C = A*B", "TOPS (2MNK)")
  struct V { let name: String; let ta: String; let tb: String; let tc: String; let tm: Int; let tn: Int; let nsg: Int; var swz = 0 }
  let vs: [V] = [
    V(name: "fp16 x fp16 -> fp32, 64x32 tile, 4 SG", ta: "half", tb: "half", tc: "float", tm: 64, tn: 32, nsg: 4),
    V(name: "fp16 x fp16 -> fp32, 128x64 tile, 4 SG", ta: "half", tb: "half", tc: "float", tm: 128, tn: 64, nsg: 4),
    V(name: "fp16 x fp16 -> fp32, 128x128 tile, 4 SG", ta: "half", tb: "half", tc: "float", tm: 128, tn: 128, nsg: 4),
    V(name: "fp16 x fp16 -> fp16, 64x32 tile, 4 SG", ta: "half", tb: "half", tc: "half", tm: 64, tn: 32, nsg: 4),
    V(name: "int8 x int8 -> int32, 64x32 tile, 4 SG", ta: "int8_t", tb: "int8_t", tc: "int", tm: 64, tn: 32, nsg: 4),
    V(name: "int8 x int8 -> int32, 128x64 tile, 4 SG", ta: "int8_t", tb: "int8_t", tc: "int", tm: 128, tn: 64, nsg: 4),
    V(name: "fp16 x fp16 -> fp32, 128x64 tile, 4 SG, swizzle 8", ta: "half", tb: "half", tc: "float", tm: 128, tn: 64, nsg: 4, swz: 8),
    V(name: "int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 8", ta: "int8_t", tb: "int8_t", tc: "int", tm: 128, tn: 64, nsg: 4, swz: 8),
    V(name: "int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 16", ta: "int8_t", tb: "int8_t", tc: "int", tm: 128, tn: 64, nsg: 4, swz: 16),
    V(name: "int8 x int8 -> int32, 128x128 tile, 4 SG", ta: "int8_t", tb: "int8_t", tc: "int", tm: 128, tn: 128, nsg: 4),
    V(name: "int8 x int8 -> int32, 256x128 tile, 8 SG", ta: "int8_t", tb: "int8_t", tc: "int", tm: 256, tn: 128, nsg: 8),
  ]
  var pipes: [MTLComputePipelineState?] = []
  for v in vs {
    do {
      pipes.append(try pipe("g4_mpp.metal", "mpp_mm", ["TA": v.ta, "TB": v.tb, "TC": v.tc, "TM": "\(v.tm)", "TN": "\(v.tn)", "NSG": "\(v.nsg)", "SWZ": "\(v.swz)"]))
    } catch { note("| \(v.name) | COMPILE FAIL: \(error.localizedDescription.prefix(400)) |"); pipes.append(nil) }
  }
  let sgp = try pipe("g4_simdgroup.metal", "sg_mm")
  for n in (quick ? [1024, 2048] : (ProcessInfo.processInfo.environment["G4_SIZES"] ?? "1024,2048,4096,8192").split(separator: ",").map { Int($0)! }) {
    autoreleasepool {
      var rng = Rng(s: UInt64(n))
      // small integers: exact in fp32 / int32 accumulation
      let ah = buf(n * n * 2), bh = buf(n * n * 2), ai = buf(n * n), bi = buf(n * n), c = buf(n * n * 4)
      let pah = ptr(ah, Float16.self), pbh = ptr(bh, Float16.self), pai = ptr(ai, Int8.self), pbi = ptr(bi, Int8.self)
      for i in 0..<(n * n) {
        let x = Int(rng.next() % 5) - 2, y = Int(rng.next() % 5) - 2
        pah[i] = Float16(x); pbh[i] = Float16(y); pai[i] = Int8(x); pbi[i] = Int8(y)
      }
      let probes = (0..<128).map { _ in (rng.below(n), rng.below(n)) }
      let refs = probes.map { (i, j) -> Int in var s = 0; for k in 0..<n { s += Int(pai[i * n + k]) * Int(pbi[k * n + j]) }; return s }
      let flops = 2.0 * Double(n) * Double(n) * Double(n)
      func check(_ tc: String) -> Bool {
        for (q, (i, j)) in probes.enumerated() {
          let got: Double
          switch tc {
          case "float": got = Double(ptr(c, Float.self)[i * n + j])
          case "half": got = Double(ptr(c, Float16.self)[i * n + j])
          default: got = Double(ptr(c, Int32.self)[i * n + j])
          }
          let tol = tc == "half" ? max(2.0, abs(Double(refs[q])) * 2e-3) : 0.0
          if abs(got - Double(refs[q])) > tol { note("    mismatch at (\(i),\(j)): got \(got) want \(refs[q])"); return false }
        }
        return true
      }
      for (vi, v) in vs.enumerated() {
        guard let p = pipes[vi] else { continue }
        let isInt = v.ta == "int8_t"
        let (a, b) = isInt ? (ai, bi) : (ah, bh)
        let encode: (MTLCommandBuffer) -> Void = { cb in
          for _ in 0..<g4Repeat {
            enc(cb, p, [a, b, c], [UInt32(n), UInt32(n), UInt32(n)], tgs: sz((n + v.tn - 1) / v.tn, (n + v.tm - 1) / v.tm), tpt: sz(32 * v.nsg))
          }
        }
        memset(c.contents(), 0x7f, n * n * 4)
        timeCB(1, warm: 0, encode)
        if !check(v.tc) { note("| n=\(n) \(v.name) | FAIL |"); continue }
        let eb = isInt ? 1 : 2, cb = v.tc == "half" ? 2 : 4
        row("n=\(n) MPP \(v.name)\(g4Repeat > 1 ? " x\(g4Repeat) per CB" : "")", timeCB(quick ? 3 : 5, encode), work: Double(g4Repeat) * flops / 1e12, unit: "TOPS", bytes: Double(2 * n * n * eb + n * n * cb))
      }
      // simdgroup_matrix
      let encSG: (MTLCommandBuffer) -> Void = { cb in
        enc(cb, sgp, [ah, bh, c], [UInt32(n), UInt32(n), UInt32(n)], tgs: sz(n / 64, n / 64), tpt: sz(128))
      }
      memset(c.contents(), 0x7f, n * n * 4)
      timeCB(1, warm: 0, encSG)
      if check("float") {
        row("n=\(n) simdgroup_matrix 8x8 fp16 -> fp32, 64x64/TG", timeCB(quick ? 3 : 5, encSG), work: flops / 1e12, unit: "TOPS", bytes: Double(4 * n * n + 4 * n * n))
      } else { note("| n=\(n) simdgroup_matrix | FAIL |") }
      // MPS (vendor-tuned) fp16 -> fp32 for reference
      let da = MPSMatrixDescriptor(rows: n, columns: n, rowBytes: n * 2, dataType: .float16)
      let dc = MPSMatrixDescriptor(rows: n, columns: n, rowBytes: n * 4, dataType: .float32)
      let mA = MPSMatrix(buffer: ah, descriptor: da), mB = MPSMatrix(buffer: bh, descriptor: da), mC = MPSMatrix(buffer: c, descriptor: dc)
      let mm = MPSMatrixMultiplication(device: dev, transposeLeft: false, transposeRight: false, resultRows: n, resultColumns: n, interiorColumns: n, alpha: 1, beta: 0)
      let encMPS: (MTLCommandBuffer) -> Void = { cb in mm.encode(commandBuffer: cb, leftMatrix: mA, rightMatrix: mB, resultMatrix: mC) }
      memset(c.contents(), 0x7f, n * n * 4)
      timeCB(1, warm: 0, encMPS)
      if check("float") {
        row("n=\(n) MPSMatrixMultiplication fp16 -> fp32", timeCB(quick ? 3 : 5, encMPS), work: flops / 1e12, unit: "TOPS", bytes: Double(4 * n * n + 4 * n * n))
      } else { note("| n=\(n) MPS fp16->fp32 | FAIL |") }
    }
  }
  note("(bytes = A + B + C once; inputs are integers in [-2, 2], 128 sampled outputs checked exactly (fp16 output: 0.2% tolerance))")

  // One-hot shapes: A (R x 256) int8 one-hot rows, B (256 x N) int8, C = A*B (R x N) int32 = row gather.
  header("G4b one-hot shapes: C (R x N) = onehot(code) (R x 256, int8) * B (256 x N, int8) -> int32", "rows/s")
  let R = quick ? 262144 : 1 << 20, K = 256
  try autoreleasepool {
    let A = buf(R * K), code = buf(R * 2)
    let pA = ptr(A, Int8.self), pcode = ptr(code, UInt16.self)
    memset(A.contents(), 0, R * K)
    var rng = Rng(s: 77)
    for r in 0..<R { let g = rng.below(K); pcode[r] = UInt16(g); pA[r * K + g] = 1 }
    for N in [1, 8, 32] {
      let B = buf(K * N), C = buf(R * N * 4)
      let pB = ptr(B, Int8.self)
      for i in 0..<(K * N) { pB[i] = Int8(Int(rng.next() % 255) - 127) }
      func ok() -> Bool {
        let pc = ptr(C, Int32.self)
        for r in stride(from: 0, to: R, by: 97) { for j in 0..<N where pc[r * N + j] != Int32(pB[Int(pcode[r]) * N + j]) { return false } }
        return true
      }
      for (tm, tn) in [(64, 32), (128, 8), (64, 8)] where tn <= max(N, 8) || N == 32 {
        let p: MTLComputePipelineState
        do { p = try pipe("g4_mpp.metal", "mpp_mm", ["TA": "int8_t", "TB": "int8_t", "TC": "int", "TM": "\(tm)", "TN": "\(tn)", "NSG": "4"]) }
        catch { note("| N=\(N) tile \(tm)x\(tn) | COMPILE FAIL: \(error.localizedDescription.prefix(300)) |"); continue }
        let encode: (MTLCommandBuffer) -> Void = { cb in
          enc(cb, p, [A, B, C], [UInt32(R), UInt32(N), UInt32(K)], tgs: sz((N + tn - 1) / tn, (R + tm - 1) / tm), tpt: sz(128))
        }
        memset(C.contents(), 0x7f, R * N * 4)
        timeCB(1, warm: 0, encode)
        if !ok() { note("| R=\(R) N=\(N) MPP tile \(tm)x\(tn) | FAIL |"); continue }
        row("R=\(R), K=256, N=\(N): MPP int8 matmul, tile \(tm)x\(tn)", timeCB(quick ? 3 : 7, encode), work: Double(R), unit: "rows/s",
            bytes: Double(R * K + K * N + R * N * 4))
      }
      let pg = try pipe("g4_simdgroup.metal", "gather_rows")
      let encG: (MTLCommandBuffer) -> Void = { cb in
        enc(cb, pg, [code, B, C], [UInt32(R), UInt32(N)], tgs: sz((N + 7) / 8, (R + 31) / 32), tpt: sz(min(8, N) == 1 ? 1 : 8, N == 1 ? 256 : 32))
      }
      memset(C.contents(), 0x7f, R * N * 4)
      timeCB(1, warm: 0, encG)
      if ok() {
        row("R=\(R), N=\(N): plain gather C[r] = B[code[r]] (u16 codes)", timeCB(quick ? 3 : 7, encG), work: Double(R), unit: "rows/s", bytes: Double(R * 2 + K * N + R * N * 4))
      } else { note("| gather N=\(N) FAIL |") }
    }
  }
  note("\nload avg after G4: \(loadAvg())")
}

// MARK: - G5 intersections

func g5() throws {
  let pt = try pipe("g5_isect.metal", "isect_thread")
  let pbs = try pipe("g5_isect.metal", "isect_simd_bsearch")
  let pm = try pipe("g5_isect.metal", "isect_simd_merge")
  let pbm = try pipe("g5_isect.metal", "isect_bitmap")
  header("G5a sorted u16 array intersection counts (both sides length L, ~25% overlap; 2 x 16M elements total)", "pairs/s")
  let total = quick ? 1 << 22 : 1 << 24
  for L in [16, 64, 256, 1024, 4096] {
    autoreleasepool {
      let P = total / L
      let A = buf(P * L * 2), B = buf(P * L * 2), out = buf(P * 4)
      var rng = Rng(s: UInt64(L))
      let pa = ptr(A, UInt16.self), pb = ptr(B, UInt16.self)
      for q in [pa, pb] {
        for p in 0..<P {
          var v = 0
          for i in 0..<L { v += 1 + rng.below(7); q[p * L + i] = UInt16(v) }  // mean gap 4: values < ~4L
        }
      }
      var ref = [UInt32](repeating: 0, count: P)
      for p in 0..<P {
        var i = 0, j = 0, c: UInt32 = 0
        while i < L && j < L { let x = pa[p * L + i], y = pb[p * L + j]; if x == y { c += 1; i += 1; j += 1 } else if x < y { i += 1 } else { j += 1 } }
        ref[p] = c
      }
      let bytes = Double(P * L * 4 + P * 4)
      let cases: [(String, (MTLCommandBuffer) -> Void)] = [
        ("thread/pair merge", { cb in enc(cb, pt, [A, B, out], [UInt32(L), UInt32(P)], tgs: sz((P + 255) / 256), tpt: sz(256)) }),
        ("SIMD-group/pair, per-element binary search", { cb in enc(cb, pbs, [A, B, out], [UInt32(L), UInt32(P)], tgs: sz((P + 7) / 8), tpt: sz(256)) }),
        ("SIMD-group/pair, lane chunk + merge", { cb in enc(cb, pm, [A, B, out], [UInt32(L), UInt32(P)], tgs: sz((P + 7) / 8), tpt: sz(256)) }),
      ]
      for (name, e) in cases {
        memset(out.contents(), 0xff, P * 4)
        timeCB(1, warm: 0, e)
        if memcmp(out.contents(), ref, P * 4) != 0 { note("| L=\(L) \(name) | FAIL |"); continue }
        row("L=\(L), P=\(P): \(name)", timeCB(quick ? 3 : 7, e), work: Double(P), unit: "pairs/s", bytes: bytes)
      }
    }
  }
  header("G5b bitmap AND + popcount, 8 KB (65536-bit) bitmaps, SIMD-group per pair", "pairs/s")
  for (label, poolN, P, streaming) in [("streaming disjoint pairs", 2 * 16384, 16384, true),
                                        ("random pairs from 4096-bitmap pool (32 MB)", 4096, 65536, false),
                                        ("random pairs from 512-bitmap pool (4 MB)", 512, 65536, false)] {
    autoreleasepool {
      let pool = buf(poolN * 8192), ia = buf(P * 4), ib = buf(P * 4), out = buf(P * 4)
      fillRandom(pool, seed: 11)
      var rng = Rng(s: 12)
      let pia = ptr(ia, UInt32.self), pib = ptr(ib, UInt32.self), pp = ptr(pool, UInt32.self)
      for p in 0..<P {
        if streaming { pia[p] = UInt32(2 * p); pib[p] = UInt32(2 * p + 1) } else { pia[p] = UInt32(rng.below(poolN)); pib[p] = UInt32(rng.below(poolN)) }
      }
      var ref = [UInt32](repeating: 0, count: P)
      for p in 0..<P {
        var c = 0
        let a = Int(pia[p]) * 2048, b = Int(pib[p]) * 2048
        for w in 0..<2048 { c += (pp[a + w] & pp[b + w]).nonzeroBitCount }
        ref[p] = UInt32(c)
      }
      let e: (MTLCommandBuffer) -> Void = { cb in enc(cb, pbm, [pool, ia, ib, out], [UInt32(P)], tgs: sz((P + 7) / 8), tpt: sz(256)) }
      memset(out.contents(), 0xff, P * 4)
      timeCB(1, warm: 0, e)
      if memcmp(out.contents(), ref, P * 4) != 0 { note("| bitmap \(label) | FAIL |"); return }
      row("P=\(P) \(label)", timeCB(quick ? 3 : 7, e), work: Double(P), unit: "pairs/s", bytes: Double(P * 16384 + P * 12))
    }
  }
  note("(G5b bytes = 16 KB of bitmap reads per pair + indices + output, i.e. logical traffic, not unique bytes)")
  note("\nload avg after G5: \(loadAvg())")
}

// MARK: - G6 segmented histogram

func g6() throws {
  let ptg = try pipe("g6_hist.metal", "hist_tg")
  let psg4 = try pipe("g6_hist.metal", "hist_sg", ["SPT": "4"])
  let psg2 = try pipe("g6_hist.metal", "hist_sg", ["SPT": "2"])
  let pdev = try pipe("g6_hist.metal", "hist_dev")
  var pmm: MTLComputePipelineState? = nil
  do { pmm = try pipe("g6_hist_mm.metal", "hist_mm") } catch { note("hist_mm COMPILE FAIL: \(error.localizedDescription.prefix(600))") }
  header("G6 segmented histogram: u16 codes < 1024, segments 2000..4000 elements, dense u32 counts out", "Gelem/s")
  for (total, skew) in (quick ? [(10_000_000, false)] : [(10_000_000, false), (30_000_000, false), (30_000_000, true)]) {
    autoreleasepool {
      var rng = Rng(s: 21)
      var offs: [UInt32] = [0]
      while Int(offs.last!) < total { offs.append(min(UInt32(total), offs.last! + UInt32(2000 + rng.below(2001)))) }
      let nseg = offs.count - 1
      let codes = buf(total * 2), off = buf(offs.count * 4), H = buf(nseg * 4096)
      memcpy(off.contents(), offs, offs.count * 4)
      let pc = ptr(codes, UInt16.self)
      for i in 0..<total {
        if skew {  // heavy duplicates: ~50% of elements on 8 codes
          pc[i] = rng.below(2) == 0 ? UInt16(rng.below(8)) : UInt16(rng.below(1024))
        } else { pc[i] = UInt16(rng.below(1024)) }
      }
      var ref = [UInt32](repeating: 0, count: nseg * 1024)
      for s in 0..<nseg { for e in Int(offs[s])..<Int(offs[s + 1]) { ref[s * 1024 + Int(pc[e])] += 1 } }
      let bytes = Double(total * 2 + offs.count * 4 + nseg * 4096)
      var cases: [(String, (MTLCommandBuffer) -> Void)] = []
      for nt in [256, 1024] {
        cases.append(("TG/segment, tg atomics, \(nt) threads", { cb in enc(cb, ptg, [codes, off, H], tgs: sz(nseg), tpt: sz(nt)) }))
      }
      cases.append(("SG/segment, 4 seg/TG (16 KB tg)", { cb in enc(cb, psg4, [codes, off, H], [UInt32(nseg)], tgs: sz((nseg + 3) / 4), tpt: sz(128)) }))
      cases.append(("SG/segment, 2 seg/TG (8 KB tg)", { cb in enc(cb, psg2, [codes, off, H], [UInt32(nseg)], tgs: sz((nseg + 1) / 2), tpt: sz(64)) }))
      cases.append(("device atomics (+ blit zero of H)", { cb in
        let b = cb.makeBlitCommandEncoder()!; b.fill(buffer: H, range: 0..<(nseg * 4096), value: 0); b.endEncoding()
        enc(cb, pdev, [codes, off, H], tgs: sz(nseg), tpt: sz(256))
      }))
      if let pmm = pmm {
        cases.append(("matrix units: one-hot hi x lo int8 matmul (MPP), SG/segment", { cb in enc(cb, pmm, [codes, off, H], [UInt32(nseg)], tgs: sz((nseg + 3) / 4), tpt: sz(128)) }))
      }
      let tag = "\(total / 1_000_000)M elems, \(nseg) segs\(skew ? ", skewed" : "")"
      for (name, e) in cases {
        memset(H.contents(), 0xff, nseg * 4096)
        timeCB(1, warm: 0, e)
        if memcmp(H.contents(), ref, nseg * 4096) != 0 {
          var bad = 0
          let ph = ptr(H, UInt32.self)
          for i in 0..<(nseg * 1024) where ph[i] != ref[i] { bad += 1 }
          note("| \(tag): \(name) | FAIL (\(bad) of \(nseg * 1024) counts wrong) |"); continue
        }
        row("\(tag): \(name)", timeCB(quick ? 3 : 7, e), work: Double(total) / 1e9, unit: "Gelem/s", bytes: bytes)
      }
    }
  }
  note("(bytes = codes (2 B/elem) + offsets + dense output (4 KB/segment); all counts checked)")
  note("\nload avg after G6: \(loadAvg())")
}

// MARK: - main

note("device: \(dev.name), metal4 family: \(dev.supportsFamily(.metal4)), apple10: \(dev.supportsFamily(.apple10)), maxTG mem: \(dev.maxThreadgroupMemoryLength), recommended working set: \(dev.recommendedMaxWorkingSetSize >> 20) MB")
let probes: [String: () throws -> Void] = ["g1": g1, "g2": g2, "g3": g3, "g4": g4, "g5": g5, "g6": g6]
do {
  if probe == "all" { for k in probes.keys.sorted() { try probes[k]!() } } else if let f = probes[probe] { try f() } else { note("unknown probe \(probe)") }
} catch { note("ERROR: \(error)") }
