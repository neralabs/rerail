// A Metal kernel's compile, timed: its library's (from source) and its pipeline's (the GPU backend's).
//
// The engine writes each library's source it compiles, and the pipelines it made from it, into a directory with
// `RERAIL_METAL_DUMP=<dir>` (`rerail_metal::runtime::pipeline_named_constant`). This compiles one of them apart:
//
//   swiftc -O scripts/dev/metal/compile-time.swift -o "$TMPDIR/metal-compile-time"   # once, outside the tree
//   "$TMPDIR/metal-compile-time" DIR/<digest>.metal label_micro 1016 [FROM TO]...
//
// The third argument is function constant 0 (the label kernels' `tape_format`), or -1 for none. The system's
// shader cache keeps a pipeline by its code, so a second compile of the same code (a comment added does not
// count) takes milliseconds. For a cold compile, give pairs FROM TO that change the code, such as a literal
// (`ROOT_GROUP=64u ROOT_GROUP=128u`): each FROM must be in the source. Decision 0353 step 4b measured with it
// that a read's 16z body, inlined at every call site, made a label kernel's compile 10 to 18 times as long.
import Foundation
import Metal

let a = CommandLine.arguments
guard a.count >= 4, a.count % 2 == 0 else {
    print("usage: compile-time FILE.metal KERNEL CONSTANT [FROM TO]...")
    exit(2)
}
var src = try! String(contentsOfFile: a[1], encoding: .utf8)
var k = 4
while k + 1 < a.count {
    guard src.contains(a[k]) else {
        print("\(a[k]) is not in the source")
        exit(2)
    }
    src = src.replacingOccurrences(of: a[k], with: a[k + 1])
    k += 2
}
let device = MTLCreateSystemDefaultDevice()!
var t = Date()
let library = try! device.makeLibrary(source: src, options: MTLCompileOptions())
let librarySeconds = Date().timeIntervalSince(t)
let constant = Int64(a[3])!
t = Date()
let function: MTLFunction
if constant >= 0 {
    let values = MTLFunctionConstantValues()
    var value = UInt32(constant)
    values.setConstantValue(&value, type: .uint, index: 0)
    function = try! library.makeFunction(name: a[2], constantValues: values)
} else {
    function = library.makeFunction(name: a[2])!
}
let pipeline = try! device.makeComputePipelineState(function: function)
let pipelineSeconds = Date().timeIntervalSince(t)
print(String(format: "%@ %@ %lld: library %.2f s, pipeline %.2f s, %d threads a group", (a[1] as NSString).lastPathComponent, a[2], constant, librarySeconds, pipelineSeconds, pipeline.maxTotalThreadsPerThreadgroup))
