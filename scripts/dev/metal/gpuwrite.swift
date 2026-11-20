// t49 §2: how pages the GPU writes count in the footprint (an allocated buffer, a no-copy wrap). Build outside the tree: swiftc -O scripts/dev/metal/gpuwrite.swift -o "$TMPDIR/metal-gpuwrite"
import Foundation
import Metal
func vm() -> (Int, Int, Int) {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
    _ = withUnsafeMutablePointer(to: &info) { $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count) } }
    return (Int(info.phys_footprint), Int(info.resident_size), Int(info.internal))
}
func sysUsed() -> Int {
    var s = vm_statistics64_data_t(); var c = mach_msg_type_number_t(MemoryLayout<vm_statistics64_data_t>.size / MemoryLayout<integer_t>.size)
    _ = withUnsafeMutablePointer(to: &s) { $0.withMemoryRebound(to: integer_t.self, capacity: Int(c)) { host_statistics64(mach_host_self(), HOST_VM_INFO64, $0, &c) } }
    return Int(s.active_count + s.wire_count + s.inactive_count + s.speculative_count + s.compressor_page_count) * Int(vm_page_size)
}
let d = MTLCreateSystemDefaultDevice()!
let q = d.makeCommandQueue()!
func fill(_ b: MTLBuffer, _ n: Int, _ v: UInt8) { let c = q.makeCommandBuffer()!; let e = c.makeBlitCommandEncoder()!; e.fill(buffer: b, range: 0..<n, value: v); e.endEncoding(); c.commit(); c.waitUntilCompleted() }
let w = d.makeBuffer(length: 1 << 16, options: .storageModeShared)!; fill(w, 1 << 16, 1)
func show(_ what: String, _ a: (Int, Int, Int), _ s: Int) { let b = vm(); print(String(format: "%-46@ footprint %+7.1f  resident %+7.1f  internal %+7.1f  system %+8.1f MB", what as NSString, Double(b.0 - a.0)/1048576, Double(b.1 - a.1)/1048576, Double(b.2 - a.2)/1048576, Double(sysUsed() - s)/1048576)) }
let n = 1 << 30
let p = mmap(nil, n, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0)!
let b = d.makeBuffer(bytesNoCopy: p, length: n, options: .storageModeShared, deallocator: nil)!
var (a, s) = (vm(), sysUsed())
fill(b, 512 << 20, 7)
show("GPU fills 512 MB of an untouched 1 GB wrap", a, s)
let bytes = p.assumingMemoryBound(to: UInt8.self)
print("host reads byte 0 = \(bytes[0]), byte 500 MB = \(bytes[500 << 20]), byte 600 MB = \(bytes[600 << 20])")
var sum = 0; for i in stride(from: 0, to: 512 << 20, by: 16384) { sum &+= Int(bytes[i]) }
show("  after the host reads every page of it", a, s)
let m = d.makeBuffer(length: 512 << 20, options: .storageModeShared)!
(a, s) = (vm(), sysUsed())
fill(m, 512 << 20, 7)
show("GPU fills a 512 MB newBuffer", a, s)
print(sum, m.length)
