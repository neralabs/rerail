// dfs_first steps 1-3 on the GPU (Metal), for the spike in ../src/main.rs.
//
//   1. live:     backward frontier waves from the targets. Each wave is an indirect dispatch sized
//                on the device from the previous wave's count, so a batch of waves runs with no host
//                round trip; the host checks for an empty frontier once per batch (bounded: at most
//                n waves in all).
//   2. next:     one thread per node: the first out-edge into a live node.
//   3. first_of: pointer jumping, double-buffered, ceil(log2 n) + 2 rounds in one command buffer.
//
// Objective-C++ for the raw string literal below. Built by build.rs with clang; the shaders are compiled at run time from the source below.
#import <Metal/Metal.h>
#include <stdint.h>
#include <string.h>

static NSString *const SRC = @R"(
#include <metal_stdlib>
using namespace metal;
constant uint NONE = 0xffffffffu;

// Indirect dispatch arguments for a wave of `count` threads, 256 per group.
kernel void wave_args(device const atomic_uint* count [[buffer(0)]],
                      device uint* args [[buffer(1)]]) {
    uint c = atomic_load_explicit(count, memory_order_relaxed);
    args[0] = (c + 255) / 256; args[1] = 1; args[2] = 1;
}
kernel void live_wave(device const uint* front [[buffer(0)]],
                      device const atomic_uint* count_in [[buffer(1)]],
                      device uint* next [[buffer(2)]],
                      device atomic_uint* count_out [[buffer(3)]],
                      device const uint* roff [[buffer(4)]],
                      device const uint* rsrc [[buffer(5)]],
                      device atomic_uint* live [[buffer(6)]],
                      uint i [[thread_position_in_grid]]) {
    if (i >= atomic_load_explicit(count_in, memory_order_relaxed)) return;
    uint v = front[i];
    for (uint k = roff[v]; k < roff[v + 1]; k++) {
        uint u = rsrc[k];
        if (atomic_exchange_explicit(&live[u], 1u, memory_order_relaxed) == 0u) {
            uint at = atomic_fetch_add_explicit(count_out, 1u, memory_order_relaxed);
            next[at] = u;
        }
    }
}
kernel void zero(device atomic_uint* c [[buffer(0)]]) { atomic_store_explicit(c, 0u, memory_order_relaxed); }

kernel void next_edge(device const uint* off [[buffer(0)]],
                      device const uint* dst [[buffer(1)]],
                      device const uint* live [[buffer(2)]],
                      device const uint* sink [[buffer(3)]],
                      device uint* nxt [[buffer(4)]],
                      constant uint& n [[buffer(5)]],
                      uint u [[thread_position_in_grid]]) {
    if (u >= n) return;
    uint r = NONE;
    for (uint e = off[u]; e < off[u + 1]; e++) {
        uint d = dst[e];
        if (live[d] != 0u && (d != u || sink[d] != 0u)) { r = e; break; }
    }
    nxt[u] = r;
}
// first_of by pointer jumping. jump[v]: the node v's chain has reached; res[v]: its target once
// known (NONE while unknown; DEAD for no target).
kernel void first_init(device const uint* nxt [[buffer(0)]],
                       device const uint* dst [[buffer(1)]],
                       device const uint* sink [[buffer(2)]],
                       device uint* jump [[buffer(3)]],
                       device uint* res [[buffer(4)]],
                       constant uint& n [[buffer(5)]],
                       uint v [[thread_position_in_grid]]) {
    if (v >= n) return;
    uint e = nxt[v];
    if (e == NONE) { jump[v] = v; res[v] = NONE - 1u; return; }   // DEAD
    uint d = dst[e];
    if (sink[d] != 0u) { jump[v] = v; res[v] = d; return; }
    jump[v] = d; res[v] = NONE;
}
kernel void first_round(device const uint* jump_in [[buffer(0)]],
                        device const uint* res_in [[buffer(1)]],
                        device uint* jump_out [[buffer(2)]],
                        device uint* res_out [[buffer(3)]],
                        constant uint& n [[buffer(4)]],
                        uint v [[thread_position_in_grid]]) {
    if (v >= n) return;
    uint r = res_in[v], j = jump_in[v];
    if (r == NONE) {
        uint rj = res_in[j];
        if (rj != NONE) r = rj; else j = jump_in[j];
    }
    jump_out[v] = j; res_out[v] = r;
}
)";

static id<MTLDevice> dev;
static id<MTLCommandQueue> queue;
static id<MTLComputePipelineState> p_args, p_live, p_zero, p_next, p_finit, p_fround;

static id<MTLComputePipelineState> pipe(id<MTLLibrary> lib, NSString *name) {
    NSError *e = nil;
    id<MTLComputePipelineState> p = [dev newComputePipelineStateWithFunction:[lib newFunctionWithName:name] error:&e];
    return p;
}

extern "C" int dfs_gpu_init(char *name, int cap) {
    @autoreleasepool {
        dev = MTLCreateSystemDefaultDevice();
        if (!dev) return 1;
        NSError *e = nil;
        id<MTLLibrary> lib = [dev newLibraryWithSource:SRC options:nil error:&e];
        if (!lib) {
            strncpy(name, [[e description] UTF8String], cap - 1);
            return 2;
        }
        queue = [dev newCommandQueue];
        p_args = pipe(lib, @"wave_args"); p_live = pipe(lib, @"live_wave"); p_zero = pipe(lib, @"zero");
        p_next = pipe(lib, @"next_edge"); p_finit = pipe(lib, @"first_init"); p_fround = pipe(lib, @"first_round");
        strncpy(name, [[dev name] UTF8String], cap - 1);
        return 0;
    }
}

static id<MTLBuffer> zeros(size_t bytes) {
    id<MTLBuffer> b = [dev newBufferWithLength:bytes ? bytes : 4 options:MTLResourceStorageModeShared];
    memset([b contents], 0, bytes ? bytes : 4);
    return b;
}
// An empty array may come with a dangling pointer (Rust's empty Vec): never read it.
static id<MTLBuffer> buf(const void *p, size_t bytes) {
    if (!bytes) return zeros(4);
    return [dev newBufferWithBytes:p length:bytes options:MTLResourceStorageModeShared];
}
static void grid(id<MTLComputeCommandEncoder> enc, id<MTLComputePipelineState> p, uint32_t n) {
    [enc setComputePipelineState:p];
    [enc dispatchThreads:MTLSizeMake(n ? n : 1, 1, 1) threadsPerThreadgroup:MTLSizeMake(256, 1, 1)];
}

// Inputs: CSR (off[n+1], dst[m]), reverse CSR (roff[n+1], rsrc[m]), sink flags[n].
// Outputs: live[n] (0/1), nxt[n] (edge index or NONE), first[n] (target or NONE).
// times[0]: GPU seconds (sum over command buffers), times[1]: waves, times[2]: command buffers.
extern "C" int dfs_gpu_prepare(uint32_t n, uint32_t m, const uint32_t *off, const uint32_t *dst, const uint32_t *roff,
                    const uint32_t *rsrc, const uint32_t *sink, uint32_t *live_out, uint32_t *nxt_out,
                    uint32_t *first_out, double *times) {
    @autoreleasepool {
        double gpu = 0; int waves = 0, cbs = 0;
        id<MTLBuffer> b_off = buf(off, 4 * (n + 1)), b_dst = buf(dst, 4 * (size_t)m);
        id<MTLBuffer> b_roff = buf(roff, 4 * (n + 1)), b_rsrc = buf(rsrc, 4 * (size_t)m);
        id<MTLBuffer> b_sink = buf(sink, 4 * n);
        id<MTLBuffer> b_live = zeros(4 * n);
        id<MTLBuffer> fr[2] = {zeros(4 * n), zeros(4 * n)};
        id<MTLBuffer> cnt[2] = {zeros(4), zeros(4)};
        id<MTLBuffer> args = zeros(12);
        // Seed the frontier with the targets.
        uint32_t *live = (uint32_t *)[b_live contents], *f0 = (uint32_t *)[fr[0] contents], k = 0;
        for (uint32_t v = 0; v < n; v++)
            if (sink[v]) { live[v] = 1; f0[k++] = v; }
        *(uint32_t *)[cnt[0] contents] = k;
        // 1. live: batches of 32 waves per command buffer.
        int cur = 0;
        while (*(uint32_t *)[cnt[cur] contents] != 0 && waves < (int)n + 1) {
            id<MTLCommandBuffer> cb = [queue commandBuffer];
            id<MTLComputeCommandEncoder> enc = [cb computeCommandEncoder];
            for (int w = 0; w < 32; w++) {
                int nx = 1 - cur;
                [enc setComputePipelineState:p_zero];
                [enc setBuffer:cnt[nx] offset:0 atIndex:0];
                [enc dispatchThreads:MTLSizeMake(1, 1, 1) threadsPerThreadgroup:MTLSizeMake(1, 1, 1)];
                [enc memoryBarrierWithScope:MTLBarrierScopeBuffers];
                [enc setComputePipelineState:p_args];
                [enc setBuffer:cnt[cur] offset:0 atIndex:0];
                [enc setBuffer:args offset:0 atIndex:1];
                [enc dispatchThreads:MTLSizeMake(1, 1, 1) threadsPerThreadgroup:MTLSizeMake(1, 1, 1)];
                [enc memoryBarrierWithScope:MTLBarrierScopeBuffers];
                [enc setComputePipelineState:p_live];
                [enc setBuffer:fr[cur] offset:0 atIndex:0];
                [enc setBuffer:cnt[cur] offset:0 atIndex:1];
                [enc setBuffer:fr[nx] offset:0 atIndex:2];
                [enc setBuffer:cnt[nx] offset:0 atIndex:3];
                [enc setBuffer:b_roff offset:0 atIndex:4];
                [enc setBuffer:b_rsrc offset:0 atIndex:5];
                [enc setBuffer:b_live offset:0 atIndex:6];
                [enc dispatchThreadgroupsWithIndirectBuffer:args indirectBufferOffset:0
                                      threadsPerThreadgroup:MTLSizeMake(256, 1, 1)];
                [enc memoryBarrierWithScope:MTLBarrierScopeBuffers];
                cur = nx;
                waves++;
            }
            [enc endEncoding];
            [cb commit];
            [cb waitUntilCompleted];
            gpu += [cb GPUEndTime] - [cb GPUStartTime];
            cbs++;
        }
        // 2-3. next and first_of, one command buffer.
        id<MTLBuffer> b_nxt = zeros(4 * n);
        id<MTLBuffer> jb[2] = {zeros(4 * n), zeros(4 * n)}, rb[2] = {zeros(4 * n), zeros(4 * n)};
        id<MTLCommandBuffer> cb = [queue commandBuffer];
        id<MTLComputeCommandEncoder> enc = [cb computeCommandEncoder];
        [enc setBuffer:b_off offset:0 atIndex:0];
        [enc setBuffer:b_dst offset:0 atIndex:1];
        [enc setBuffer:b_live offset:0 atIndex:2];
        [enc setBuffer:b_sink offset:0 atIndex:3];
        [enc setBuffer:b_nxt offset:0 atIndex:4];
        [enc setBytes:&n length:4 atIndex:5];
        grid(enc, p_next, n);
        [enc memoryBarrierWithScope:MTLBarrierScopeBuffers];
        [enc setBuffer:b_nxt offset:0 atIndex:0];
        [enc setBuffer:b_dst offset:0 atIndex:1];
        [enc setBuffer:b_sink offset:0 atIndex:2];
        [enc setBuffer:jb[0] offset:0 atIndex:3];
        [enc setBuffer:rb[0] offset:0 atIndex:4];
        [enc setBytes:&n length:4 atIndex:5];
        grid(enc, p_finit, n);
        int rounds = 2;
        for (uint32_t x = n; x > 1; x >>= 1) rounds++;
        int c = 0;
        for (int r = 0; r < rounds; r++) {
            [enc memoryBarrierWithScope:MTLBarrierScopeBuffers];
            [enc setBuffer:jb[c] offset:0 atIndex:0];
            [enc setBuffer:rb[c] offset:0 atIndex:1];
            [enc setBuffer:jb[1 - c] offset:0 atIndex:2];
            [enc setBuffer:rb[1 - c] offset:0 atIndex:3];
            [enc setBytes:&n length:4 atIndex:4];
            grid(enc, p_fround, n);
            c = 1 - c;
        }
        [enc endEncoding];
        [cb commit];
        [cb waitUntilCompleted];
        gpu += [cb GPUEndTime] - [cb GPUStartTime];
        cbs++;
        memcpy(live_out, [b_live contents], 4 * (size_t)n);
        memcpy(nxt_out, [b_nxt contents], 4 * (size_t)n);
        const uint32_t *res = (const uint32_t *)[rb[c] contents];
        for (uint32_t v = 0; v < n; v++) first_out[v] = res[v] >= 0xfffffffeu ? 0xffffffffu : res[v];
        times[0] = gpu; times[1] = waves; times[2] = cbs;
        return 0;
    }
}
