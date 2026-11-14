// anebench: compile, load, inspect placement of, and time one Core ML model.
//
// Usage:
//   anebench <model.mlpackage|.mlmodelc> [--iters N] [--warm W] [--cu ane|cpu|gpu|all]
//            [--in DIR] [--out DIR] [--batch B] [--plan] [--plan-only] [--cache DIR]
//            [--tag TEXT] [--gpu-busy SECONDS]
//
// Inputs: for each model input <name>, DIR/<name>.bin is read as raw little-endian values of the
// input's data type (fp16 or fp32 or int32); missing files are filled with random 0/1 values.
// Outputs: with --out, each output is written to DIR/<name>.bin as raw values of its data type.
// Timing: per-call wall time (monotonic clock), min and median over N calls after W warm-up calls;
// host process CPU (getrusage user+sys) over the whole timed loop divided by N.
// Placement: MLComputePlan preferred device per operation (consts skipped), counted per op type.
import CoreML
import Foundation
import Metal
import CoreVideo

func nowNs() -> UInt64 { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
func cpuNs() -> UInt64 {
    var r = rusage()
    getrusage(RUSAGE_SELF, &r)
    let u = UInt64(r.ru_utime.tv_sec) * 1_000_000_000 + UInt64(r.ru_utime.tv_usec) * 1000
    let s = UInt64(r.ru_stime.tv_sec) * 1_000_000_000 + UInt64(r.ru_stime.tv_usec) * 1000
    return u + s
}
func loadAvg() -> String {
    var l = [Double](repeating: 0, count: 3)
    getloadavg(&l, 3)
    return String(format: "%.0f/%.0f/%.0f", l[0], l[1], l[2])
}
func dirSize(_ url: URL) -> Int {
    var total = 0
    if let e = FileManager.default.enumerator(at: url, includingPropertiesForKeys: [.fileSizeKey]) {
        for case let f as URL in e {
            total += (try? f.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0) ?? 0
        }
    }
    return total
}

struct Args {
    var model = ""
    var iters = 200
    var warm = 10
    var cu = "ane"
    var inDir: String? = nil
    var outDir: String? = nil
    var batch = 0
    var plan = false
    var planOnly = false
    var cache: String? = nil
    var tag = ""
    var gpuBusy = 0.0
    var iosurface = false
}

func parseArgs() -> Args {
    var a = Args()
    var it = CommandLine.arguments.dropFirst().makeIterator()
    while let s = it.next() {
        switch s {
        case "--iters": a.iters = Int(it.next()!)!
        case "--warm": a.warm = Int(it.next()!)!
        case "--cu": a.cu = it.next()!
        case "--in": a.inDir = it.next()!
        case "--out": a.outDir = it.next()!
        case "--batch": a.batch = Int(it.next()!)!
        case "--plan": a.plan = true
        case "--plan-only": a.plan = true; a.planOnly = true
        case "--cache": a.cache = it.next()!
        case "--tag": a.tag = it.next()!
        case "--gpu-busy": a.gpuBusy = Double(it.next()!)!
        case "--iosurface": a.iosurface = true
        default: a.model = s
        }
    }
    return a
}

func elemSize(_ t: MLMultiArrayDataType) -> Int {
    switch t {
    case .float16: return 2
    case .float32, .int32: return 4
    case .int8: return 1
    case .double: return 8
    default: return 4
    }
}

func makeInput(_ name: String, _ c: MLMultiArrayConstraint, _ dir: String?) throws -> MLMultiArray {
    let shape = c.shape
    let arr = try MLMultiArray(shape: shape, dataType: c.dataType)
    let n = arr.count
    let es = elemSize(c.dataType)
    if let d = dir, let data = FileManager.default.contents(atPath: "\(d)/\(name).bin") {
        if data.count != n * es { print("input \(name): dtype \(c.dataType.rawValue) file has \(data.count) bytes, want \(n * es)"); exit(2) }
        arr.withUnsafeMutableBytes { p, _ in data.copyBytes(to: p.bindMemory(to: UInt8.self)) }
        return arr
    }
    var rng = SystemRandomNumberGenerator()
    arr.withUnsafeMutableBytes { p, _ in
        switch c.dataType {
        case .float16:
            let q = p.bindMemory(to: Float16.self)
            for i in 0..<n { q[i] = (rng.next() & 1) == 0 ? 0 : 1 }
        case .float32:
            let q = p.bindMemory(to: Float.self)
            for i in 0..<n { q[i] = (rng.next() & 1) == 0 ? 0 : 1 }
        case .int32:
            let q = p.bindMemory(to: Int32.self)
            for i in 0..<n { q[i] = Int32(rng.next() % UInt64(max(1, shape.last!.intValue))) }
        default: break
        }
    }
    return arr
}

// fp16 MLMultiArray backed by an IOSurface (CVPixelBuffer, OneComponent16Half): last dim = width.
func surfaceArray(_ shape: [NSNumber]) throws -> MLMultiArray {
    let w = shape.last!.intValue
    let h = shape.dropLast().reduce(1) { $0 * $1.intValue }
    var pb: CVPixelBuffer?
    let attrs: [String: Any] = [kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
                                kCVPixelBufferMetalCompatibilityKey as String: true]
    let st = CVPixelBufferCreate(nil, w, h, kCVPixelFormatType_OneComponent16Half, attrs as CFDictionary, &pb)
    precondition(st == 0 && pb != nil, "CVPixelBufferCreate failed \(st) for \(w)x\(h)")
    return MLMultiArray(pixelBuffer: pb!, shape: shape)
}

func copyInto(_ dst: MLMultiArray, _ src: MLMultiArray) {
    let n = src.count
    let shape = dst.shape.map { $0.intValue }
    let strides = dst.strides.map { $0.intValue }
    src.withUnsafeBytes { s in
        dst.withUnsafeMutableBytes { d, _ in
            let sp = s.bindMemory(to: UInt16.self)
            let dp = d.bindMemory(to: UInt16.self)
            var idx = [Int](repeating: 0, count: shape.count)
            for lin in 0..<n {
                var off = 0
                for k in 0..<shape.count { off += idx[k] * strides[k] }
                dp[off] = sp[lin]
                var k = shape.count - 1
                while k >= 0 { idx[k] += 1; if idx[k] < shape[k] { break }; idx[k] = 0; k -= 1 }
            }
        }
    }
}

func devName(_ d: MLComputeDevice?) -> String {
    guard let d = d else { return "none" }
    switch d {
    case .cpu: return "CPU"
    case .gpu: return "GPU"
    case .neuralEngine: return "ANE"
    @unknown default: return "?"
    }
}

func walk(_ block: MLModelStructure.Program.Block, _ plan: MLComputePlan, _ acc: inout [String: (Int, Double)]) {
    for op in block.operations {
        if op.operatorName == "const" { continue }
        let u = plan.deviceUsage(for: op)
        let dev = devName(u?.preferred)
        let supp = (u?.supported ?? []).map { devName($0) }.joined(separator: "+")
        let cost = plan.estimatedCost(of: op)?.weight ?? 0
        let key = "\(op.operatorName)@\(dev)[\(supp)]"
        let prev = acc[key] ?? (0, 0)
        acc[key] = (prev.0 + 1, prev.1 + cost)
        for b in op.blocks { walk(b, plan, &acc) }
    }
}

// Keeps the GPU busy with a large memory-bound copy kernel loop for `seconds`.
func startGpuBusy(_ seconds: Double) -> Thread? {
    guard seconds > 0, let dev = MTLCreateSystemDefaultDevice() else { return nil }
    let src = """
    #include <metal_stdlib>
    using namespace metal;
    kernel void k(device const float4* a [[buffer(0)]], device float4* b [[buffer(1)]], uint i [[thread_position_in_grid]]) {
        float4 v = a[i]; for (int r = 0; r < 8; r++) v = fma(v, v, float4(0.5)); b[i] = v; }
    """
    let lib = try! dev.makeLibrary(source: src, options: nil)
    let pso = try! dev.makeComputePipelineState(function: lib.makeFunction(name: "k")!)
    let n = 32 << 20  // 32M float4 = 512 MB? too much; use 8M float4 = 128 MB each
    let m = n / 8  // 4M float4 = 64 MB per buffer
    let a = dev.makeBuffer(length: m * 16, options: .storageModePrivate)!
    let b = dev.makeBuffer(length: m * 16, options: .storageModePrivate)!
    let q = dev.makeCommandQueue()!
    let t = Thread {
        let end = nowNs() + UInt64(seconds * 1e9)
        while nowNs() < end {
            let cb = q.makeCommandBuffer()!
            for _ in 0..<4 {
                let e = cb.makeComputeCommandEncoder()!
                e.setComputePipelineState(pso)
                e.setBuffer(a, offset: 0, index: 0)
                e.setBuffer(b, offset: 0, index: 1)
                e.dispatchThreads(MTLSize(width: m, height: 1, depth: 1),
                                  threadsPerThreadgroup: MTLSize(width: 256, height: 1, depth: 1))
                e.endEncoding()
            }
            cb.commit()
            cb.waitUntilCompleted()
        }
    }
    t.start()
    return t
}

func runTimed(_ model: MLModel, _ provider: MLFeatureProvider, _ opts: MLPredictionOptions, _ a: Args, line l0: String) throws -> String {
        var line = l0
        let busy = startGpuBusy(a.gpuBusy)
        if busy != nil { Thread.sleep(forTimeInterval: 0.5) }

        var out: MLFeatureProvider? = nil
        let tf0 = nowNs()
        out = try model.prediction(from: provider, options: opts)
        line += String(format: " first_ms=%.3f", Double(nowNs() - tf0) / 1e6)
        for _ in 0..<a.warm { out = try model.prediction(from: provider, options: opts) }

        if a.batch > 0 {
            let bp = MLArrayBatchProvider(array: [MLFeatureProvider](repeating: provider, count: a.batch))
            var ts: [Double] = []
            let c0 = cpuNs()
            for _ in 0..<a.iters {
                let s = nowNs()
                _ = try model.predictions(from: bp, options: opts)
                ts.append(Double(nowNs() - s) / 1e3)
            }
            let cpu = Double(cpuNs() - c0) / 1e3 / Double(a.iters)
            ts.sort()
            line += String(format: " batch=%d batch_us_min=%.2f batch_us_med=%.2f per_item_us_min=%.3f per_item_us_med=%.3f host_cpu_us_per_batch=%.2f",
                           a.batch, ts[0], ts[ts.count / 2], ts[0] / Double(a.batch), ts[ts.count / 2] / Double(a.batch), cpu)
        } else {
            var ts: [Double] = []
            ts.reserveCapacity(a.iters)
            let c0 = cpuNs()
            for _ in 0..<a.iters {
                let s = nowNs()
                out = try model.prediction(from: provider, options: opts)
                ts.append(Double(nowNs() - s) / 1e3)
            }
            let cpu = Double(cpuNs() - c0) / 1e3 / Double(a.iters)
            ts.sort()
            line += String(format: " iters=%d us_min=%.2f us_med=%.2f us_p90=%.2f host_cpu_us=%.2f",
                           a.iters, ts[0], ts[ts.count / 2], ts[ts.count * 9 / 10], cpu)
        }
        busy?.cancel()

        if let od = a.outDir, let o = out {
            try? FileManager.default.createDirectory(atPath: od, withIntermediateDirectories: true)
            for n in o.featureNames {
                guard let m = o.featureValue(for: n)?.multiArrayValue else { continue }
                // Copy honoring strides into a dense buffer.
                let es = elemSize(m.dataType)
                var data = Data(count: m.count * es)
                let shape = m.shape.map { $0.intValue }
                let strides = m.strides.map { $0.intValue }
                m.withUnsafeBytes { src in
                    data.withUnsafeMutableBytes { dst in
                        var idx = [Int](repeating: 0, count: shape.count)
                        for lin in 0..<m.count {
                            var off = 0
                            for k in 0..<shape.count { off += idx[k] * strides[k] }
                            memcpy(dst.baseAddress! + lin * es, src.baseAddress! + off * es, es)
                            var k = shape.count - 1
                            while k >= 0 { idx[k] += 1; if idx[k] < shape[k] { break }; idx[k] = 0; k -= 1 }
                        }
                    }
                }
                try data.write(to: URL(fileURLWithPath: "\(od)/\(n).bin"))
                line += " out[\(n)]=\(m.dataType == .float16 ? "f16" : "\(m.dataType.rawValue)")\(shape)"
            }
        }
        return line
}

@main
struct Main {
    static func main() async throws {
        let a = parseArgs()
        let cfg = MLModelConfiguration()
        switch a.cu {
        case "cpu": cfg.computeUnits = .cpuOnly
        case "gpu": cfg.computeUnits = .cpuAndGPU
        case "all": cfg.computeUnits = .all
        default: cfg.computeUnits = .cpuAndNeuralEngine
        }
        var url = URL(fileURLWithPath: a.model)
        var compileMs = 0.0
        if url.pathExtension == "mlpackage" {
            let cacheDir = URL(fileURLWithPath: a.cache ?? (url.deletingLastPathComponent().path))
            let dst = cacheDir.appendingPathComponent(url.deletingPathExtension().lastPathComponent + ".mlmodelc")
            let srcMod = (try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date) ?? Date()
            let dstMod = (try? FileManager.default.attributesOfItem(atPath: dst.path)[.modificationDate] as? Date) ?? nil
            if dstMod == nil || dstMod! < srcMod {
                let t0 = nowNs()
                let tmp = try await MLModel.compileModel(at: url)
                compileMs = Double(nowNs() - t0) / 1e6
                try? FileManager.default.removeItem(at: dst)
                try FileManager.default.moveItem(at: tmp, to: dst)
            }
            url = dst
        }
        let name = url.deletingPathExtension().lastPathComponent
        var line = "model=\(name) cu=\(a.cu) tag=\(a.tag) mlmodelc_bytes=\(dirSize(url))"
        if compileMs > 0 { line += String(format: " compile_ms=%.1f", compileMs) }

        if a.plan {
            do {
                let plan = try await MLComputePlan.load(contentsOf: url, configuration: cfg)
                if case let .program(prog) = plan.modelStructure, let f = prog.functions["main"] {
                    var acc: [String: (Int, Double)] = [:]
                    walk(f.block, plan, &acc)
                    let s = acc.sorted { $0.key < $1.key }
                        .map { String(format: "%@x%d(cost %.3f)", $0.key, $0.value.0, $0.value.1) }
                        .joined(separator: " ")
                    line += " plan={\(s)}"
                } else {
                    line += " plan=not-a-program"
                }
            } catch {
                line += " plan=ERROR(\(error.localizedDescription))"
            }
        }
        if a.planOnly { print(line); return }

        var t0 = nowNs()
        var model = try MLModel(contentsOf: url, configuration: cfg)
        let load1 = Double(nowNs() - t0) / 1e6
        t0 = nowNs()
        model = try MLModel(contentsOf: url, configuration: cfg)
        let load2 = Double(nowNs() - t0) / 1e6
        line += String(format: " load_ms=%.1f reload_ms=%.1f", load1, load2)

        var feats: [String: MLFeatureValue] = [:]
        for (n, d) in model.modelDescription.inputDescriptionsByName {
            let c = d.multiArrayConstraint!
            let plain = try makeInput(n, c, a.inDir)
            if a.iosurface && c.dataType == .float16 {
                let s = try surfaceArray(c.shape)
                copyInto(s, plain)
                feats[n] = MLFeatureValue(multiArray: s)
            } else {
                feats[n] = MLFeatureValue(multiArray: plain)
            }
        }
        let provider = try MLDictionaryFeatureProvider(dictionary: feats)
        let opts = MLPredictionOptions()
        if a.iosurface {
            var backs: [String: Any] = [:]
            for (n, d) in model.modelDescription.outputDescriptionsByName {
                if let c = d.multiArrayConstraint, c.dataType == .float16 { backs[n] = try surfaceArray(c.shape) }
            }
            opts.outputBackings = backs
            line += " iosurface=1"
        }

        line += try runTimed(model, provider, opts, a, line: "")
        line += " load=\(loadAvg())"
        print(line)
    }
}
