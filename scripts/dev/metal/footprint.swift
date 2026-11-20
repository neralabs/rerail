// t49 §2: the driver's count against real memory. Build outside the tree: swiftc -O scripts/dev/metal/footprint.swift -o "$TMPDIR/metal-footprint"
// Real memory (the process's physical footprint) against the driver's count, for a no-copy wrap of reserved memory:
// untouched, after the host writes a prefix, and after a GPU command reads and writes a prefix of it.
import Foundation
import Metal
func footprint() -> Int {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
    _ = withUnsafeMutablePointer(to: &info) { $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count) } }
    return Int(info.phys_footprint)
}
let d = MTLCreateSystemDefaultDevice()!
let q = d.makeCommandQueue()!
func mb(_ x: Int) -> String { String(format: "%7.1f MB", Double(x) / 1048576) }
let (f0, a0) = (footprint(), d.currentAllocatedSize)
func line(_ what: String) { print("\(what.padding(toLength: 40, withPad: " ", startingAt: 0)) footprint +\(mb(footprint() - f0))  driver +\(mb(d.currentAllocatedSize - a0))") }
let p = mmap(nil, 1 << 30, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0)!
let b = d.makeBuffer(bytesNoCopy: p, length: 1 << 30, options: .storageModeShared, deallocator: nil)!
line("wrap 1 GB reserved, untouched")
memset(p, 1, 16 << 20)
line("host writes 16 MB")
let c = q.makeCommandBuffer()!
let e = c.makeBlitCommandEncoder()!
e.fill(buffer: b, range: 0..<(64 << 20), value: 2)
e.endEncoding(); c.commit(); c.waitUntilCompleted()
line("GPU fills 64 MB")
let c2 = q.makeCommandBuffer()!
let e2 = c2.makeBlitCommandEncoder()!
let dst = d.makeBuffer(length: 1 << 20, options: .storageModeShared)!
e2.copy(from: b, sourceOffset: 900 << 20, to: dst, destinationOffset: 0, size: 1 << 20)
e2.endEncoding(); c2.commit(); c2.waitUntilCompleted()
line("GPU reads 1 MB at 900 MB (untouched)")
madvise(p, 64 << 20, MADV_FREE_REUSABLE)
line("host frees its first 64 MB (madvise)")
let n = d.makeBuffer(length: 256 << 20, options: .storageModeShared)!
line("newBuffer 256 MB, untouched")
memset(n.contents(), 1, 16 << 20)
line("  host writes 16 MB of it")
_ = (b, n)
