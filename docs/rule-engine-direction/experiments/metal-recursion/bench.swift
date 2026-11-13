// Metal microbenchmark: data-dependent descendant search formulations over a regular preorder tape.
// Usage: bench <tape-dir> <kernels.metal>
import Foundation
import Metal

setvbuf(stdout, nil, _IONBF, 0)
let args = CommandLine.arguments
let skipC = ProcessInfo.processInfo.environment["SKIP_C"] != nil
let onlyC = ProcessInfo.processInfo.environment["ONLY_C"] != nil
let dir = URL(fileURLWithPath: args[1])
func load<T>(_ name: String, _: T.Type) -> [T] {
    let data = try! Data(contentsOf: dir.appendingPathComponent(name))
    return data.withUnsafeBytes { Array($0.bindMemory(to: T.self)) }
}
let kind = load("kind.u16", UInt16.self)
let parent = load("parent.u32", UInt32.self)
let size = load("size.u32", UInt32.self)
let meta = try! JSONSerialization.jsonObject(with: Data(contentsOf: dir.appendingPathComponent("meta.json"))) as! [String: Any]
let kinds = meta["kinds"] as! [String]
let maxDepth = meta["max_depth"] as! Int
let n = kind.count
func kid(_ s: String) -> Int? { kinds.firstIndex(of: s) }

// CSR children, for the pointer-chasing formulation.
var childStart = [UInt32](repeating: 0, count: n + 1)
var childList = [UInt32](repeating: 0, count: n)
do {
    var counts = [UInt32](repeating: 0, count: n)
    for i in 0..<n where parent[i] != UInt32.max { counts[Int(parent[i])] += 1 }
    var acc: UInt32 = 0
    for i in 0..<n { childStart[i] = acc; acc += counts[i] }
    childStart[n] = acc
    var fill = childStart
    for i in 0..<n where parent[i] != UInt32.max { let p = Int(parent[i]); childList[Int(fill[p])] = UInt32(i); fill[p] += 1 }
}

let device = MTLCreateSystemDefaultDevice()!
let library = try! device.makeLibrary(source: String(contentsOfFile: args[2]), options: nil)
func pipe(_ name: String) -> MTLComputePipelineState { try! device.makeComputePipelineState(function: library.makeFunction(name: name)!) }
let queue = device.makeCommandQueue()!
func buf<T>(_ a: [T]) -> MTLBuffer { a.withUnsafeBytes { device.makeBuffer(bytes: $0.baseAddress!, length: max($0.count, 4), options: .storageModeShared)! } }
func zeros(_ bytes: Int, fill: UInt8 = 0) -> MTLBuffer { let b = device.makeBuffer(length: max(bytes, 4), options: .storageModeShared)!; memset(b.contents(), Int32(fill), b.length); return b }
func read(_ b: MTLBuffer, _ count: Int) -> [UInt32] { Array(UnsafeBufferPointer(start: b.contents().bindMemory(to: UInt32.self, capacity: count), count: count)) }

struct Params { var rootCount: UInt32; var exists: UInt32; var stackCap: UInt32; var queueCap: UInt32; var spinCap: UInt32; var itemCount: UInt32 }

let kindBuf = buf(kind), parentBuf = buf(parent), sizeBuf = buf(size)
let childStartBuf = buf(childStart), childListBuf = buf(childList)
let pA = pipe("a_stack"), pB = pipe("b_interval"), pB2 = pipe("b2_posting"), pB3o = pipe("b3_owner"), pB3q = pipe("b3_query")
let pFa = pipe("f_prune_anc"), pFs = pipe("f_simd_scan")
let pC = pipe("c_persistent"), pDp = pipe("d_prep"), pDw = pipe("d_wave"), pE = pipe("e_inside")

func run(_ encode: (MTLComputeCommandEncoder) -> Void) -> Double {
    let cb = queue.makeCommandBuffer()!
    let enc = cb.makeComputeCommandEncoder()!
    encode(enc)
    enc.endEncoding()
    cb.commit()
    cb.waitUntilCompleted()
    if let e = cb.error { print("  GPU error: \(e)") }
    return (cb.gpuEndTime - cb.gpuStartTime) * 1000
}
func grid(_ enc: MTLComputeCommandEncoder, _ p: MTLComputePipelineState, _ threads: Int) {
    enc.setComputePipelineState(p)
    enc.dispatchThreads(MTLSize(width: max(threads, 1), height: 1, depth: 1), threadsPerThreadgroup: MTLSize(width: 256, height: 1, depth: 1))
}
func median(_ xs: [Double]) -> Double { xs.sorted()[xs.count / 2] }
func fmt(_ x: Double) -> String { String(format: "%8.3f", x) }

struct Query { let name: String; let roots: [String]; let targets: [String]; let prune: [String]; let exists: Bool; let inside: Bool }
let scopes = ["FunctionDef", "AsyncFunctionDef", "Lambda", "ClassDef"]
let queries = [
    Query(name: "Q1 fn has Yield, stopBy nested scope (EXISTS)", roots: ["FunctionDef", "AsyncFunctionDef"], targets: ["Yield", "YieldFrom"], prune: scopes, exists: true, inside: false),
    Query(name: "Q2 fn has Return, stopBy nested scope (EXISTS)", roots: ["FunctionDef", "AsyncFunctionDef"], targets: ["Return"], prune: scopes, exists: true, inside: false),
    Query(name: "Q3 count Call in fn, stopBy end (COUNT)", roots: ["FunctionDef", "AsyncFunctionDef"], targets: ["Call"], prune: [], exists: false, inside: false),
    Query(name: "Q4 Module has Lambda, stopBy end (COUNT, huge roots)", roots: ["Module"], targets: ["Lambda"], prune: [], exists: false, inside: false),
    Query(name: "Q5 every Call has Lambda, stopBy end (COUNT, 420k nested roots)", roots: ["Call"], targets: ["Lambda"], prune: [], exists: false, inside: false),
    Query(name: "Q6 every Call inside Try, stopBy scope (ancestor)", roots: ["Call"], targets: ["Try"], prune: scopes, exists: true, inside: true),
]

print("nodes \(n), files \(meta["files"]!), kinds \(kinds.count), max depth \(maxDepth), device \(device.name)")
let reps = 7
for q in queries {
    var cls = [UInt8](repeating: 0, count: kinds.count)
    for t in q.targets { if let k = kid(t) { cls[k] |= 1 } }
    for p in q.prune { if let k = kid(p) { cls[k] |= 2 } }
    let rootKinds = Set(q.roots.compactMap(kid))
    let roots = (0..<n).filter { rootKinds.contains(Int(kind[$0])) }.map(UInt32.init)
    let targetKinds = Set(q.targets.compactMap(kid))
    let postings = (0..<n).filter { targetKinds.contains(Int(kind[$0])) }.map(UInt32.init)
    let R = roots.count

    // CPU reference (interval formulation, one thread) plus work statistics.
    var ref = [UInt32](repeating: 0, count: R)
    var touched = 0, expanded = 0
    let t0 = DispatchTime.now().uptimeNanoseconds
    for (s, r) in roots.enumerated() {
        if q.inside {
            var p = parent[Int(r)]
            while p != UInt32.max { touched += 1; let c = cls[Int(kind[Int(p)])]; if c & 1 != 0 { ref[s] = 1; break }; if c & 2 != 0 { break }; p = parent[Int(p)] }
            continue
        }
        let end = Int(r) + Int(size[Int(r)]); var j = Int(r) + 1
        while j < end {
            touched += 1
            let c = cls[Int(kind[j])]
            if c & 1 != 0 { ref[s] += 1; if q.exists { break } }
            if c & 2 == 0 && size[j] > 1 { expanded += 1 }
            j += (c & 2 != 0) ? Int(size[j]) : 1
        }
    }
    let cpuMs = Double(DispatchTime.now().uptimeNanoseconds - t0) / 1e6
    print("\n\(q.name)\n  roots \(R), targets \(postings.count), nodes touched \(touched), CPU 1-thread interval \(fmt(cpuMs)) ms")

    let clsBuf = buf(cls), rootBuf = buf(roots), postBuf = buf(postings)
    var params = Params(rootCount: UInt32(R), exists: q.exists ? 1 : 0, stackCap: 64, queueCap: 0, spinCap: UInt32(ProcessInfo.processInfo.environment["SPIN"].flatMap { UInt32($0) } ?? (1 << 12)), itemCount: UInt32(postings.count))
    let out = zeros(R * 4)
    func check(_ label: String, _ times: [Double], _ result: [UInt32], dispatches: Int, extra: String = "") {
        let bad = zip(result, ref).filter { $0 != $1 }.count
        print("  \(label.padding(toLength: 34, withPad: " ", startingAt: 0)) GPU median \(fmt(median(times))) ms  dispatches \(dispatches)  mismatches \(bad) \(extra)")
    }

    if q.inside {
        var ts: [Double] = []
        for _ in 0..<reps { ts.append(run { e in e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(parentBuf, offset: 0, index: 1); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBuffer(rootBuf, offset: 0, index: 4); e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(out, offset: 0, index: 6); grid(e, pE, R) }) }
        check("(e) parent walk", ts, read(out, R), dispatches: 1)
        continue
    }

    // (a) explicit stack over CSR children
    if !onlyC {
        let ov = zeros(4); var ts: [Double] = []
        for _ in 0..<reps { memset(ov.contents(), 0, 4); ts.append(run { e in e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(childStartBuf, offset: 0, index: 1); e.setBuffer(childListBuf, offset: 0, index: 2); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBuffer(rootBuf, offset: 0, index: 4); e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(out, offset: 0, index: 6); e.setBuffer(ov, offset: 0, index: 7); grid(e, pA, R) }) }
        let res = read(out, R); let overflow = read(ov, 1)[0]
        let badExcl = zip(res, ref).filter { $0.0 != 0xFFFFFFFF && $0.0 != $0.1 }.count
        check("(a) stack DFS, CSR, cap 64", ts, res, dispatches: 1, extra: "(stack overflows \(overflow), mismatches excl. overflow \(badExcl))")
    }
    // (b) interval scan
    if !onlyC {
        var ts: [Double] = []
        for _ in 0..<reps { ts.append(run { e in e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(sizeBuf, offset: 0, index: 1); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBuffer(rootBuf, offset: 0, index: 4); e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(out, offset: 0, index: 6); grid(e, pB, R) }) }
        check("(b) interval scan + size skip", ts, read(out, R), dispatches: 1)
    }
    // (b2)/(b3) postings
    if onlyC {} else if q.prune.isEmpty {
        var ts: [Double] = []
        for _ in 0..<reps { ts.append(run { e in e.setBuffer(sizeBuf, offset: 0, index: 1); e.setBuffer(postBuf, offset: 0, index: 2); e.setBuffer(rootBuf, offset: 0, index: 4); e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(out, offset: 0, index: 6); grid(e, pB2, R) }) }
        check("(b2) postings, 2 binary searches", ts, read(out, R), dispatches: 1)
    } else {
        let owner = zeros(max(postings.count, 1) * 4); var ts: [Double] = []
        for _ in 0..<reps { ts.append(run { e in
            e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(parentBuf, offset: 0, index: 1); e.setBuffer(postBuf, offset: 0, index: 2); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(owner, offset: 0, index: 6); grid(e, pB3o, postings.count)
            e.memoryBarrier(scope: .buffers)
            e.setBuffer(sizeBuf, offset: 0, index: 1); e.setBuffer(owner, offset: 0, index: 3); e.setBuffer(rootBuf, offset: 0, index: 4); e.setBuffer(out, offset: 0, index: 6); grid(e, pB3q, R) }) }
        check("(b3) postings + owner column", ts, read(out, R), dispatches: 2)
    }
    // (f) SIMD-cooperative interval scan using a nearest-pruning-ancestor column
    if !onlyC {
        let anc = zeros(n * 4); var nn = UInt32(n); var ts: [Double] = []; var tAnc: [Double] = []
        for _ in 0..<reps {
            tAnc.append(run { e in e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(parentBuf, offset: 0, index: 1); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBytes(&nn, length: 4, index: 5); e.setBuffer(anc, offset: 0, index: 6); grid(e, pFa, n) })
            ts.append(run { e in e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(sizeBuf, offset: 0, index: 1); e.setBuffer(anc, offset: 0, index: 2); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBuffer(rootBuf, offset: 0, index: 4); e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(out, offset: 0, index: 6); grid(e, pFs, R * 32) })
        }
        check("(f) SIMD-group scan + anc column", ts, read(out, R), dispatches: 1, extra: "(anc column build \(fmt(median(tAnc))) ms, once per prune set per batch)")
    }
    // (c) persistent threads + device work queue
    if !skipC {
        let cap = R + expanded + 1024
        params.queueCap = UInt32(cap)
        let qRoot = zeros(cap * 4, fill: 0xFF), qNode = zeros(cap * 4, fill: 0xFF), ctr = zeros(32)
        let aout = zeros(R * 4)
        var ts: [Double] = []
        var ok = true
        for _ in 0..<reps {
            memset(qRoot.contents(), 0xFF, cap * 4); memset(qNode.contents(), 0xFF, cap * 4); memset(ctr.contents(), 0, 32); memset(aout.contents(), 0, R * 4)
            let qr = qRoot.contents().bindMemory(to: UInt32.self, capacity: cap), qn = qNode.contents().bindMemory(to: UInt32.self, capacity: cap)
            for s in 0..<R { qr[s] = UInt32(s); qn[s] = roots[s] }
            ctr.contents().bindMemory(to: UInt32.self, capacity: 8)[1] = UInt32(R)
            let threads = 40 * 512
            ts.append(run { e in e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(sizeBuf, offset: 0, index: 1); e.setBuffer(qRoot, offset: 0, index: 2); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBuffer(qNode, offset: 0, index: 4); e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(aout, offset: 0, index: 6); e.setBuffer(ctr, offset: 0, index: 7); grid(e, pC, threads) })
            let c = read(ctr, 5); if c[3] != 0 || c[4] != 0 { ok = false }
        }
        let c = read(ctr, 5)
        var res = read(aout, R); if q.exists { res = res.map { min($0, 1) } }
        check("(c) persistent threads + queue", ts, res, dispatches: 1, extra: "(items \(c[1]), overflow \(c[3]), timeout \(c[4]), all-runs-ok \(ok))")
    }
    // (d) frontier waves, GPU-driven indirect dispatch, one command buffer
    if !onlyC {
        let cap = R + expanded + 1024
        params.queueCap = UInt32(cap)
        let f0 = zeros(cap * 8), f1 = zeros(cap * 8), counts = zeros(64 * 4), ind = zeros(64 * 12), aout = zeros(R * 4)
        var ts: [Double] = []
        let waves = maxDepth + 1
        for _ in 0..<reps {
            memset(counts.contents(), 0, 256); memset(aout.contents(), 0, R * 4)
            let fp = f0.contents().bindMemory(to: UInt32.self, capacity: cap * 2)
            for s in 0..<R { fp[2 * s] = UInt32(s); fp[2 * s + 1] = roots[s] }
            counts.contents().bindMemory(to: UInt32.self, capacity: 64)[0] = UInt32(R)
            ts.append(run { e in
                for w in 0..<waves {
                    var wv = UInt32(w)
                    e.setComputePipelineState(pDp); e.setBuffer(counts, offset: 0, index: 0); e.setBuffer(ind, offset: 0, index: 1); e.setBytes(&wv, length: 4, index: 2)
                    e.dispatchThreads(MTLSize(width: 1, height: 1, depth: 1), threadsPerThreadgroup: MTLSize(width: 1, height: 1, depth: 1))
                    e.memoryBarrier(scope: .buffers)
                    e.setComputePipelineState(pDw)
                    e.setBuffer(kindBuf, offset: 0, index: 0); e.setBuffer(sizeBuf, offset: 0, index: 1); e.setBuffer(w % 2 == 0 ? f0 : f1, offset: 0, index: 2); e.setBuffer(clsBuf, offset: 0, index: 3); e.setBuffer(w % 2 == 0 ? f1 : f0, offset: 0, index: 4)
                    e.setBytes(&params, length: MemoryLayout<Params>.size, index: 5); e.setBuffer(aout, offset: 0, index: 6); e.setBuffer(counts, offset: 0, index: 7); e.setBytes(&wv, length: 4, index: 8)
                    e.dispatchThreadgroups(indirectBuffer: ind, indirectBufferOffset: w * 12, threadsPerThreadgroup: MTLSize(width: 256, height: 1, depth: 1))
                    e.memoryBarrier(scope: .buffers)
                }
            })
        }
        var res = read(aout, R); if q.exists { res = res.map { min($0, 1) } }
        let cs = read(counts, 64)
        let peak = cs[0..<waves].max()!, used = cs[0..<waves].filter { $0 > 0 }.count
        check("(d) frontier waves, indirect", ts, res, dispatches: 2 * waves, extra: "(non-empty waves \(used), peak frontier \(peak), overflow \(cs[63]))")
    }
}
