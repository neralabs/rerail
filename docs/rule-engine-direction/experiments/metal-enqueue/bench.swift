// Device-side enqueue for unbounded programs on Metal: an unbounded fixpoint run as frontier waves,
// driven (a) by the CPU per wave, (b) by a pre-encoded command buffer with GPU-written indirect args,
// (c) by GPU-encoded indirect command buffer (ICB) commands, (c2) ICB commands encoding later commands of
// the same ICB, (d) chained command buffers, (e) a single-threadgroup persistent loop, (f) hybrid b+e.
// Usage: bench <tape-dir> <kernels.metal>
import Foundation
import Metal

setvbuf(stdout, nil, _IONBF, 0)
let args = CommandLine.arguments
let dir = URL(fileURLWithPath: args[1])
let env = ProcessInfo.processInfo.environment
func load<T>(_ name: String, _: T.Type) -> [T] {
    let data = try! Data(contentsOf: dir.appendingPathComponent(name))
    return data.withUnsafeBytes { Array($0.bindMemory(to: T.self)) }
}

struct Workload { let name: String; let kind: [UInt16]; let parent: [UInt32]; let size: [UInt32]; let pass: [UInt8]; let seeds: [UInt32] }

// CPython tape: seeds = Lambda nodes, pass = every kind except Module (component = the rest of the file).
func cpython() -> Workload {
    let kind = load("kind.u16", UInt16.self), parent = load("parent.u32", UInt32.self), size = load("size.u32", UInt32.self)
    let meta = try! JSONSerialization.jsonObject(with: Data(contentsOf: dir.appendingPathComponent("meta.json"))) as! [String: Any]
    let kinds = meta["kinds"] as! [String]
    var pass = [UInt8](repeating: 1, count: 256)
    pass[kinds.firstIndex(of: "Module")!] = 0
    let lam = UInt16(kinds.firstIndex(of: "Lambda")!)
    let seeds = (0..<kind.count).filter { kind[$0] == lam }.map(UInt32.init)
    return Workload(name: "CPython stdlib: Lambda-seeded component fixpoint (5.4M nodes)", kind: kind, parent: parent, size: size, pass: pass, seeds: seeds)
}

// A synthetic degenerate document: a chain of depth D (each node one child plus one leaf), seed at the bottom.
func chain(_ D: Int) -> Workload {
    let n = 2 * D + 1
    var kind = [UInt16](repeating: 1, count: n), parent = [UInt32](repeating: 0, count: n), size = [UInt32](repeating: 1, count: n)
    // preorder: chain node k at position 2k, its leaf at 2k+1; the last chain node at 2D.
    parent[0] = UInt32.max
    for k in 0..<D {
        let c = 2 * k
        parent[c + 1] = UInt32(c)       // leaf
        parent[c + 2] = UInt32(c)       // next chain node
        kind[c + 1] = 2
    }
    for k in stride(from: D, through: 0, by: -1) { let c = 2 * k; size[c] = UInt32(n - c) }
    var pass = [UInt8](repeating: 0, count: 256); pass[1] = 1
    return Workload(name: "synthetic chain, depth \(D) (\(n) nodes), seeded at the bottom", kind: kind, parent: parent, size: size, pass: pass, seeds: [UInt32(2 * D)])
}

// CPU reference: BFS by waves; returns (marked count, waves with work).
func reference(_ w: Workload) -> (Int, Int) {
    var mark = [UInt8](repeating: 0, count: w.kind.count)
    var front = w.seeds; for s in front { mark[Int(s)] = 1 }
    var total = 0, waves = 0
    while !front.isEmpty {
        var next: [UInt32] = []
        func visit(_ j: Int) { if w.pass[Int(w.kind[j])] != 0 && mark[j] == 0 { mark[j] = 1; next.append(UInt32(j)) } }
        for i in front.map({ Int($0) }) {
            if w.parent[i] != UInt32.max { visit(Int(w.parent[i])) }
            var c = i + 1; let end = i + Int(w.size[i])
            while c < end { visit(c); c += Int(w.size[c]) }
        }
        if !next.isEmpty { waves += 1; total += next.count }
        front = next
    }
    return (total, waves)
}

let device = MTLCreateSystemDefaultDevice()!
let library = try! device.makeLibrary(source: String(contentsOfFile: args[2]), options: nil)
let queue = device.makeCommandQueue(maxCommandBufferCount: 4096)!
func pipe(_ name: String, icb: Bool = false) -> MTLComputePipelineState {
    let d = MTLComputePipelineDescriptor()
    d.computeFunction = library.makeFunction(name: name)!
    d.supportIndirectCommandBuffers = icb
    return try! device.makeComputePipelineState(descriptor: d, options: [], reflection: nil)
}
func buf<T>(_ a: [T]) -> MTLBuffer { a.withUnsafeBytes { device.makeBuffer(bytes: $0.baseAddress!, length: max($0.count, 4), options: .storageModeShared)! } }
func zeros(_ bytes: Int) -> MTLBuffer { let b = device.makeBuffer(length: max(bytes, 16), options: .storageModeShared)!; memset(b.contents(), 0, b.length); return b }
func u32(_ b: MTLBuffer, _ i: Int) -> UInt32 { b.contents().bindMemory(to: UInt32.self, capacity: i + 1)[i] }
func fmt(_ x: Double) -> String { String(format: "%9.3f", x) }
func now() -> UInt64 { DispatchTime.now().uptimeNanoseconds }

let pExpand = pipe("expand", icb: true), pPrep = pipe("prep"), pLoop = pipe("tg_loop")
let pPrepIcb = pipe("prep_icb", icb: true), pPrepSelf = pipe("prep_icb_self", icb: true)

final class State {
    let w: Workload
    let kind, parent, size, pass, mark, f0, f1, counters, iargs, sel0, sel1, selPair: MTLBuffer
    let cap: UInt32
    init(_ w: Workload) {
        self.w = w
        kind = buf(w.kind); parent = buf(w.parent); size = buf(w.size); pass = buf(w.pass)
        let n = w.kind.count
        cap = UInt32(n)
        mark = zeros(n * 4); f0 = zeros(n * 4); f1 = zeros(n * 4); counters = zeros(64); iargs = zeros(64)
        sel0 = buf([UInt32(0), cap]); sel1 = buf([UInt32(1), cap]); selPair = buf([UInt32(0), cap, UInt32(1), cap])
    }
    func reset() {
        memset(mark.contents(), 0, mark.length); memset(counters.contents(), 0, counters.length)
        let m = mark.contents().bindMemory(to: UInt32.self, capacity: w.kind.count)
        let f = f0.contents().bindMemory(to: UInt32.self, capacity: w.seeds.count)
        for (k, s) in w.seeds.enumerated() { m[Int(s)] = 1; f[k] = s }
        let c = counters.contents().bindMemory(to: UInt32.self, capacity: 16)
        c[0] = UInt32(w.seeds.count)
        let a = iargs.contents().bindMemory(to: UInt32.self, capacity: 6)
        a[0] = (UInt32(w.seeds.count) + 255) / 256; a[1] = 1; a[2] = 1; a[3] = 0; a[4] = 1; a[5] = 1
    }
    func bindExpand(_ e: MTLComputeCommandEncoder, src: Int) {
        e.setComputePipelineState(pExpand)
        e.setBuffers([kind, parent, size, pass, mark, src == 0 ? f0 : f1, src == 0 ? f1 : f0, counters, src == 0 ? sel0 : sel1], offsets: Array(repeating: 0, count: 9), range: 0..<9)
    }
    func encodeWave(_ e: MTLComputeCommandEncoder, wave: Int, handoff: UInt32, prep: Bool = true) {
        let src = wave & 1
        bindExpand(e, src: src)
        e.dispatchThreadgroups(indirectBuffer: iargs, indirectBufferOffset: 0, threadsPerThreadgroup: MTLSize(width: 256, height: 1, depth: 1))
        if !prep { return }
        e.setComputePipelineState(pPrep)
        var s = SIMD3<UInt32>(UInt32(1 - src), handoff, 0)
        e.setBuffer(counters, offset: 0, index: 0); e.setBuffer(iargs, offset: 0, index: 1); e.setBytes(&s, length: 16, index: 2)
        e.dispatchThreads(MTLSize(width: 1, height: 1, depth: 1), threadsPerThreadgroup: MTLSize(width: 1, height: 1, depth: 1))
    }
    func encodeLoop(_ e: MTLComputeCommandEncoder, indirect: Bool, limit: UInt32) {
        e.setComputePipelineState(pLoop)
        var l = limit
        e.setBuffers([kind, parent, size, pass, mark, f0, f1, counters, sel0], offsets: Array(repeating: 0, count: 9), range: 0..<9)
        e.setBytes(&l, length: 4, index: 9)
        let tg = MTLSize(width: 1024, height: 1, depth: 1)
        if indirect { e.dispatchThreadgroups(indirectBuffer: iargs, indirectBufferOffset: 12, threadsPerThreadgroup: tg) }
        else { e.dispatchThreadgroups(MTLSize(width: 1, height: 1, depth: 1), threadsPerThreadgroup: tg) }
    }
    var result: (Int, Int, Int, Int) { (Int(u32(counters, 3)), Int(u32(counters, 2)), Int(u32(counters, 5)), Int(u32(counters, 6))) }
}

func report(_ label: String, _ st: State, _ ref: (Int, Int), gpuMs: Double, wallMs: Double, extra: String) {
    let (marked, waves, loopWaves, ovf) = st.result
    let ok = marked == ref.0 && waves == ref.1 && ovf == 0
    let per = waves > 0 ? wallMs * 1000 / Double(waves) : 0
    print("  \(label.padding(toLength: 46, withPad: " ", startingAt: 0)) wall \(fmt(wallMs)) ms  gpu \(fmt(gpuMs)) ms  waves \(waves) (tg-loop \(loopWaves))  \(String(format: "%7.2f", per)) us/wave  \(ok ? "ok" : "MISMATCH marked \(marked)/\(ref.0)") \(extra)")
}

func runAll(_ w: Workload) {
    let (refMarked, refWaves) = reference(w)
    print("\n\(w.name)\n  reference: marked \(refMarked), waves \(refWaves), seeds \(w.seeds.count)")
    let st = State(w)
    let ref = (refMarked, refWaves)
    let maxWaves = refWaves + 2

    // (a) CPU-driven: one command buffer per wave, CPU reads the count and stops.
    do {
        st.reset()
        let t0 = now(); var gpu = 0.0; var wave = 0
        while true {
            let cb = queue.makeCommandBuffer()!; let e = cb.makeComputeCommandEncoder()!
            st.encodeWave(e, wave: wave, handoff: 0); e.endEncoding(); cb.commit(); cb.waitUntilCompleted()
            gpu += (cb.gpuEndTime - cb.gpuStartTime) * 1000
            wave += 1
            if u32(st.counters, 4) == 2 || wave > maxWaves { break }
        }
        report("(a) CPU-driven, 1 command buffer per wave", st, ref, gpuMs: gpu, wallMs: Double(now() - t0) / 1e6, extra: "")
    }

    // (b) one command buffer, K waves pre-encoded, GPU-written indirect args, empty waves after convergence.
    for slack in [0, 1000] {
        st.reset()
        let K = maxWaves + slack
        let t0 = now()
        let cb = queue.makeCommandBuffer()!; let e = cb.makeComputeCommandEncoder(dispatchType: .serial)!
        for k in 0..<K { st.encodeWave(e, wave: k, handoff: 0) }
        e.endEncoding()
        let tEnc = now()
        cb.commit(); cb.waitUntilCompleted()
        if let err = cb.error { print("  (b) error \(err)") }
        report("(b) pre-encoded \(K) waves in one command buffer", st, ref, gpuMs: (cb.gpuEndTime - cb.gpuStartTime) * 1000, wallMs: Double(now() - t0) / 1e6, extra: "encode \(String(format: "%.1f", Double(tEnc - t0) / 1e6)) ms; \(slack) empty trailing waves")
    }

    // (d) chained command buffers of C waves each; CPU commits ahead, checks the done flag between chunks.
    for C in [64, 1024] {
        st.reset()
        let t0 = now(); var wave = 0; var gpu = 0.0; var inflight: [MTLCommandBuffer] = []
        var submitted = 0
        outer: while true {
            let cb = queue.makeCommandBuffer()!; let e = cb.makeComputeCommandEncoder(dispatchType: .serial)!
            for _ in 0..<C { st.encodeWave(e, wave: wave, handoff: 0); wave += 1 }
            e.endEncoding(); cb.commit(); submitted += 1; inflight.append(cb)
            if inflight.count >= 2 {
                let first = inflight.removeFirst(); first.waitUntilCompleted(); gpu += (first.gpuEndTime - first.gpuStartTime) * 1000
                if u32(st.counters, 4) == 2 { break outer }
            }
            if wave > maxWaves + 2 * C { break }
        }
        for cb in inflight { cb.waitUntilCompleted(); gpu += (cb.gpuEndTime - cb.gpuStartTime) * 1000 }
        report("(d) chained buffers, \(C) waves each", st, ref, gpuMs: gpu, wallMs: Double(now() - t0) / 1e6, extra: "\(submitted) buffers")
    }

    // (e) single-threadgroup persistent loop (barrier-synchronised; forward progress guaranteed in a threadgroup).
    do {
        st.reset()
        let t0 = now()
        let cb = queue.makeCommandBuffer()!; let e = cb.makeComputeCommandEncoder()!
        st.encodeLoop(e, indirect: false, limit: UInt32.max); e.endEncoding(); cb.commit(); cb.waitUntilCompleted()
        if let err = cb.error { print("  (e) error \(err)") }
        report("(e) single-threadgroup loop, one dispatch", st, ref, gpuMs: (cb.gpuEndTime - cb.gpuStartTime) * 1000, wallMs: Double(now() - t0) / 1e6, extra: "")
    }

    // (f) hybrid: device-wide waves while the frontier is wide, hand off to the threadgroup loop when narrow.
    for handoff in [UInt32(1024), UInt32(8192)] {
        st.reset()
        let slots = min(maxWaves, 400)
        let t0 = now()
        let cb = queue.makeCommandBuffer()!; let e = cb.makeComputeCommandEncoder(dispatchType: .serial)!
        for k in 0..<slots { st.encodeWave(e, wave: k, handoff: handoff) }
        st.encodeLoop(e, indirect: true, limit: UInt32.max)
        e.endEncoding(); cb.commit(); cb.waitUntilCompleted()
        if let err = cb.error { print("  (f) error \(err)") }
        let done = u32(st.counters, 4) == 2
        report("(f) hybrid, hand off at <= \(handoff), \(slots) slots", st, ref, gpuMs: (cb.gpuEndTime - cb.gpuStartTime) * 1000, wallMs: Double(now() - t0) / 1e6, extra: done ? "" : "NOT DONE (needs more device slots)")
    }

    // (c) GPU-encoded ICB: per wave, a scheduler kernel writes the next expand command (PSO, buffers, grid).
    runICB(st, ref, maxWaves: maxWaves)
}

func makeArgs(_ st: State, icb: MTLIndirectCommandBuffer, fn: String) -> (MTLBuffer, MTLBuffer) {
    let f = library.makeFunction(name: fn)!
    let ae = f.makeArgumentEncoder(bufferIndex: 0)
    let ab = device.makeBuffer(length: ae.encodedLength, options: .storageModeShared)!
    ae.setArgumentBuffer(ab, offset: 0)
    ae.setIndirectCommandBuffer(icb, index: 0)
    ae.setComputePipelineState(pExpand, index: 1)
    // ExpandBinds: nine device pointers, written as GPU addresses.
    let binds = device.makeBuffer(length: 9 * 8, options: .storageModeShared)!
    let p = binds.contents().bindMemory(to: UInt64.self, capacity: 9)
    let list = [st.kind, st.parent, st.size, st.pass, st.mark, st.f0, st.f1, st.counters, st.selPair]
    for (k, b) in list.enumerated() { p[k] = b.gpuAddress }
    return (ab, binds)
}

func runICB(_ st: State, _ ref: (Int, Int), maxWaves: Int) {
    let K = maxWaves
    let d = MTLIndirectCommandBufferDescriptor()
    d.commandTypes = [.concurrentDispatch]
    d.inheritBuffers = false
    d.inheritPipelineState = false
    d.maxKernelBufferBindCount = 10
    guard let icb = device.makeIndirectCommandBuffer(descriptor: d, maxCommandCount: max(K, 1), options: []) else { print("  (c) ICB creation failed"); return }
    let (ab, binds) = makeArgs(st, icb: icb, fn: "prep_icb")
    let resources: [MTLResource] = [icb, st.kind, st.parent, st.size, st.pass, st.mark, st.f0, st.f1, st.counters, st.selPair, binds]

    // (c) per wave: [prep_icb encodes slot k] then [executeCommandsInBuffer slot k]; the first wave is a plain dispatch.
    st.reset()
    let t0 = now()
    let cb = queue.makeCommandBuffer()!; let e = cb.makeComputeCommandEncoder(dispatchType: .serial)!
    e.useResources(resources, usage: [.read, .write])
    st.encodeWave(e, wave: 0, handoff: 0, prep: false)  // wave 0 (expand from seeds + prep for args; prep's args unused afterwards)
    // after wave 0, counters[0] was consumed and the new frontier is in slot 1
    for k in 1..<K {
        e.setComputePipelineState(pPrepIcb)
        var w = SIMD2<UInt32>(UInt32(k & 1), UInt32(k))   // slot k&1 holds the frontier produced by wave k-1
        e.setBuffer(ab, offset: 0, index: 0); e.setBuffer(binds, offset: 0, index: 1); e.setBytes(&w, length: 8, index: 2)
        e.dispatchThreads(MTLSize(width: 1, height: 1, depth: 1), threadsPerThreadgroup: MTLSize(width: 1, height: 1, depth: 1))
        e.memoryBarrier(resources: [icb])
        e.executeCommandsInBuffer(icb, range: k..<(k + 1))
    }
    e.endEncoding(); cb.commit(); cb.waitUntilCompleted()
    if let err = cb.error { print("  (c) error \(err)") }
    report("(c) GPU-encoded ICB command per wave", st, ref, gpuMs: (cb.gpuEndTime - cb.gpuStartTime) * 1000, wallMs: Double(now() - t0) / 1e6, extra: "")
    runSelf(st, ref, maxWaves: maxWaves)
    // NOTE: (c) encodes the prep for wave k with src slot k&1; wave 0 used encodeWave which already ran prep (zeroing),
    // so prep_icb re-reads the produced count from slot (k&1) and zeroes the other.
}

let which = env["WORK"] ?? "all"
if which == "all" || which == "chain" {
    for D in (env["DEPTHS"] ?? "10000,100000").split(separator: ",").map({ Int($0)! }) { runAll(chain(D)) }
}
if which == "all" || which == "cpython" { runAll(cpython()) }

// (c2) self-encoding: one ICB holds [prep_self k, expand k] pairs. Each prep command (CPU-encoded into the ICB)
// GPU-encodes the next slot of the SAME ICB; the whole chain runs in a single executeCommandsInBuffer call.
func runSelf(_ st: State, _ ref: (Int, Int), maxWaves: Int) {
    let K = maxWaves
    let d = MTLIndirectCommandBufferDescriptor()
    d.commandTypes = [.concurrentDispatch]
    d.inheritBuffers = false
    d.inheritPipelineState = false
    d.maxKernelBufferBindCount = 10
    let count = 2 * max(K - 1, 1)
    guard let icb = device.makeIndirectCommandBuffer(descriptor: d, maxCommandCount: count, options: []) else { print("  (c2) ICB creation failed"); return }
    let (ab, binds) = makeArgs(st, icb: icb, fn: "prep_icb_self")
    var wtab: [UInt32] = [], ks: [UInt32] = []
    for k in 1..<max(K, 2) { wtab += [UInt32(k & 1), UInt32(2 * (k - 1) + 1)]; ks.append(UInt32(k - 1)) }
    let wbuf = buf(wtab), kbuf = buf(ks)
    let tEnc0 = now()
    for k in 1..<max(K, 2) {
        let c = icb.indirectComputeCommandAt(2 * (k - 1))
        c.setComputePipelineState(pPrepSelf)
        c.setKernelBuffer(ab, offset: 0, at: 0)
        c.setKernelBuffer(binds, offset: 0, at: 1)
        c.setKernelBuffer(wbuf, offset: 0, at: 2)
        c.setKernelBuffer(kbuf, offset: 4 * (k - 1), at: 3)
        c.concurrentDispatchThreadgroups(MTLSize(width: 1, height: 1, depth: 1), threadsPerThreadgroup: MTLSize(width: 1, height: 1, depth: 1))
        c.setBarrier()
        icb.indirectComputeCommandAt(2 * (k - 1) + 1).reset()
    }
    let encMs = Double(now() - tEnc0) / 1e6
    let resources: [MTLResource] = [icb, st.kind, st.parent, st.size, st.pass, st.mark, st.f0, st.f1, st.counters, st.selPair, binds, ab, wbuf, kbuf]
    st.reset()
    let t0 = now()
    let cb = queue.makeCommandBuffer()!; let e = cb.makeComputeCommandEncoder(dispatchType: .serial)!
    e.useResources(resources, usage: [.read, .write])
    st.encodeWave(e, wave: 0, handoff: 0, prep: false)
    e.memoryBarrier(scope: .buffers)
    let split = env["C2SPLIT"].flatMap { Int($0) } ?? 0
    if split > 0 {
        var r = 0
        while r < count { let hi = min(r + split, count); e.executeCommandsInBuffer(icb, range: r..<hi); e.memoryBarrier(resources: [icb]); r = hi }
    } else {
        e.executeCommandsInBuffer(icb, range: 0..<count)
    }
    e.endEncoding(); cb.commit(); cb.waitUntilCompleted()
    if let err = cb.error { print("  (c2) error \(err)") }
    report("(c2) self-encoding ICB, execute split \(split == 0 ? "none" : String(split))", st, ref, gpuMs: (cb.gpuEndTime - cb.gpuStartTime) * 1000, wallMs: Double(now() - t0) / 1e6, extra: "ICB pre-encode \(String(format: "%.1f", encMs)) ms (CPU, reusable)")
}
