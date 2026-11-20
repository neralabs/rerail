// t49 §2: what the driver counts for no-copy wraps, and what a wrap costs. Build outside the tree: swiftc -O scripts/dev/metal/nocopy.swift -o "$TMPDIR/metal-nocopy"
// How the driver counts buffers: an allocated one, a no-copy wrap of untouched reserved memory, of part of it, and the
// cost of wrapping a prefix (as a kernel's dispatch would, each time its rows grow).
import Foundation
import Metal
let d = MTLCreateSystemDefaultDevice()!
func mb(_ x: Int) -> String { String(format: "%.1f MB", Double(x) / 1048576) }
let base = d.currentAllocatedSize
let a = d.makeBuffer(length: 64 << 20, options: .storageModeShared)!
print("newBuffer 64 MB:            +\(mb(d.currentAllocatedSize - base))")
let reserve = 1 << 30
let p = mmap(nil, reserve, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0)!
let b0 = d.currentAllocatedSize
let whole = d.makeBuffer(bytesNoCopy: p, length: 64 << 20, options: .storageModeShared, deallocator: nil)!
print("no-copy 64 MB untouched:    +\(mb(d.currentAllocatedSize - b0))")
memset(p, 1, 4 << 20)
print("  after 4 MB written:       +\(mb(d.currentAllocatedSize - b0))")
let b1 = d.currentAllocatedSize
let prefix = d.makeBuffer(bytesNoCopy: p, length: 4 << 20, options: .storageModeShared, deallocator: nil)!
print("no-copy 4 MB prefix:        +\(mb(d.currentAllocatedSize - b1))")
let big = d.makeBuffer(bytesNoCopy: p, length: 1 << 30, options: .storageModeShared, deallocator: nil)!
print("no-copy 1 GB reservation:   +\(mb(d.currentAllocatedSize - b1 - prefix.length))")
var t = Date()
var keep: [MTLBuffer] = []
for i in 1...1000 { keep.append(d.makeBuffer(bytesNoCopy: p, length: (i % 64 + 1) << 16 , options: .storageModeShared, deallocator: nil)!) }
print(String(format: "wrap a prefix: %.1f µs each", Date().timeIntervalSince(t) * 1e3))
keep.removeAll()
t = Date()
for _ in 1...100 { _ = d.makeBuffer(bytesNoCopy: p, length: 256 << 20, options: .storageModeShared, deallocator: nil)! }
print(String(format: "wrap 256 MB: %.1f µs each", Date().timeIntervalSince(t) * 1e4))
_ = (a, whole, big)
