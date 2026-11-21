// ukl/backend.h: the intrinsics the UKL library is written over, one section per backend (decision 0440 §5).
//
// Everything else in the library is written once, against these. A backend is added here and nowhere else: its
// qualifiers, its bit counts, its SIMD-group shuffles and ballot, its barriers and its atomics. Where a backend has a
// faster form of a library operation (Metal's `simd_sum`, CUDA's `__reduce_add_sync`), it is defined here as
// `ukl_..._native` with a `UKL_NATIVE_...` flag; the library picks it over its own reference form (`..._ref`), and the
// library's tests check the two equal on every backend.
//
// The SIMD-group operations are convergent: every lane of the group calls them (CUDA's `*_sync` with the full mask,
// Metal's `simd_*`). A 1-D threadgroup's SIMD groups are its threads in order, `UKL_SIMD_WIDTH` at a time.

#pragma once

#include <ukl>

// The SIMD group's state, for a backend that keeps it in memory (OpenCL without subgroups: a threadgroup's scratch,
// which its shim defines, decision 0440 §3.5): every function that reaches a SIMD-group operation takes
// `__UKL_PARAMS` after its parameters, and passes `__UKL_ARGS` after its arguments. Empty where the hardware holds the
// group (Metal, CUDA, the CPU).
#ifndef __UKL_PARAMS
#define __UKL_PARAMS
#define __UKL_ARGS
#endif

#if defined(UKL_METAL)

// Qualifiers: a function the library inlines; a struct's method; device memory; device memory another threadgroup
// of the same dispatch writes (coherent); threadgroup memory; a thread's own.
#define UKL_INLINE static inline
// A function written as a rare case (a generated read marked cold, decision 0310): a call, not inlined. Inlined at
// every call site, 16z's block decode made the view reads' test kernel take 387 s in Metal's back end (2.9 s as
// calls). A parent walk calling `clif_tape_parent` runs 1.3% slower than with it inlined. A kernel may define
// `UKL_COLD` as `UKL_INLINE` before its includes.
// A function the compiler inlines or calls as it chooses: a generated program's (`clif_ukl`), large and many.
#define UKL_CALLED static
#ifndef UKL_COLD
#define UKL_COLD static __attribute__((noinline))
#endif
#define UKL_GLOBAL device
#define UKL_COHERENT device coherent(device)
#define UKL_LOCAL threadgroup
#define UKL_PRIVATE thread
// A table at file scope (`UKL_TABLE const uint32_t t[2]={1u,2u};`) and a pointer into one (`UKL_TABLE_SPACE const T*`).
#define UKL_TABLE constant
#define UKL_TABLE_SPACE constant

UKL_INLINE uint32_t ukl_popcount32(uint32_t x) { return popcount(x); }
UKL_INLINE uint32_t ukl_popcount64(uint64_t x) { return (uint32_t)popcount(x); }
UKL_INLINE uint32_t ukl_clz32(uint32_t x) { return clz(x); }
UKL_INLINE uint32_t ukl_clz64(uint64_t x) { return (uint32_t)clz(x); }
// The high 64 bits of the signed 128-bit product (an overflow test of `*`).
UKL_INLINE int64_t ukl_mulhi64(int64_t a, int64_t b) { return mulhi(a, b); }
UKL_INLINE uint64_t ukl_umulhi64(uint64_t a, uint64_t b) { return mulhi(a, b); }
UKL_INLINE uint32_t ukl_ctz32(uint32_t x) { return ctz(x); }
UKL_INLINE uint32_t ukl_ctz64(uint64_t x) { return (uint32_t)ctz(x); }

UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return simd_shuffle(v, (ushort)lane); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS) { return simd_shuffle_up(v, (ushort)d); }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS) { return simd_shuffle_down(v, (ushort)d); }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return simd_shuffle_xor(v, (ushort)m); }
UKL_INLINE uint32_t ukl_sg_ballot(bool b __UKL_PARAMS) { return (uint32_t)(uint64_t)(simd_vote::vote_t)simd_ballot(b); }

UKL_INLINE void ukl_sg_barrier() { simdgroup_barrier(mem_flags::mem_device | mem_flags::mem_threadgroup); }
UKL_INLINE void ukl_tg_barrier() { threadgroup_barrier(mem_flags::mem_threadgroup); }
UKL_INLINE void ukl_tg_barrier_device() { threadgroup_barrier(mem_flags::mem_device | mem_flags::mem_threadgroup); }

// Relaxed atomics on a device word, and a device-wide fence.
UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return atomic_load_explicit((device atomic_uint*)p, memory_order_relaxed); }
// A poll: the read of a word another threadgroup writes and this thread waits for, again and again (decision 0540).
// On Apple's GPUs a relaxed load in a loop may keep reading the polling core's copy and never see the store (the
// M1 Max, Q123; the Mac mini too: a grid barrier's waiters polled its generation a million times each and saw it
// unchanged, while their compare-and-swaps after found it stepped). A read-modify-write that changes nothing reads
// the coherent copy.
UKL_INLINE uint32_t ukl_atomic_poll(UKL_GLOBAL uint32_t* p) { return atomic_fetch_or_explicit((device atomic_uint*)p, 0u, memory_order_relaxed); }
UKL_INLINE void ukl_atomic_store(UKL_GLOBAL uint32_t* p, uint32_t v) { atomic_store_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_add(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_add_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_sub(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_sub_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_or(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_or_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_and(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_and_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_exchange(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_exchange_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_min(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_min_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_max(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_max_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
// A strong compare-and-swap: the value seen, `e` where it swapped. Metal's is weak: a spurious failure is tried
// again, a bounded number of times, then reported as a value other than `e`.
UKL_INLINE uint32_t ukl_atomic_cas(UKL_GLOBAL uint32_t* p, uint32_t e, uint32_t n)
{
    for (uint32_t k = 0u; k < 64u; k++) {
        uint32_t x = e;
        if (atomic_compare_exchange_weak_explicit((device atomic_uint*)p, &x, n, memory_order_relaxed, memory_order_relaxed))
            return e;
        if (x != e)
            return x;
    }
    return ~e;
}
UKL_INLINE void ukl_fence_device() { atomic_thread_fence(mem_flags::mem_device, memory_order_seq_cst, thread_scope_device); }
// A relaxed add on a threadgroup word.
UKL_INLINE uint32_t ukl_atomic_add_local(UKL_LOCAL uint32_t* p, uint32_t v) { return atomic_fetch_add_explicit((threadgroup atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_load_local(UKL_LOCAL uint32_t* p) { return atomic_load_explicit((threadgroup atomic_uint*)p, memory_order_relaxed); }
UKL_INLINE void ukl_atomic_store_local(UKL_LOCAL uint32_t* p, uint32_t v) { atomic_store_explicit((threadgroup atomic_uint*)p, v, memory_order_relaxed); }
// The specialization constant (`UKL_CONSTANT_U32`, below): Metal's function constant 0.
#define UKL_CONSTANT_U32(name) constant uint32_t name [[function_constant(0)]];

// Native forms.
#define UKL_NATIVE_SG_REDUCE 1
UKL_INLINE uint32_t ukl_sg_sum_native(uint32_t v __UKL_PARAMS) { return simd_sum(v); }
UKL_INLINE uint32_t ukl_sg_min_native(uint32_t v __UKL_PARAMS) { return simd_min(v); }
UKL_INLINE uint32_t ukl_sg_max_native(uint32_t v __UKL_PARAMS) { return simd_max(v); }
#define UKL_NATIVE_SG_SCAN 1
UKL_INLINE uint32_t ukl_sg_inclusive_sum_native(uint32_t v __UKL_PARAMS) { return simd_prefix_inclusive_sum(v); }
UKL_INLINE uint32_t ukl_sg_exclusive_sum_native(uint32_t v __UKL_PARAMS) { return simd_prefix_exclusive_sum(v); }

#elif defined(UKL_CUDA)

#define UKL_INLINE static __device__ __forceinline__
// A rare case, called (not inlined): forced inline at every call site, a large cold read (16z's block decode) made
// NVRTC take minutes and gigabytes on a kernel that reads every node's every field.
#define UKL_CALLED static __device__
#ifndef UKL_COLD
#define UKL_COLD static __device__ __noinline__
#endif
#define UKL_GLOBAL
#define UKL_COHERENT volatile
#define UKL_LOCAL
#define UKL_PRIVATE
#define UKL_TABLE __constant__
#define UKL_TABLE_SPACE

UKL_INLINE uint32_t ukl_popcount32(uint32_t x) { return __popc(x); }
UKL_INLINE uint32_t ukl_popcount64(uint64_t x) { return __popcll(x); }
UKL_INLINE uint32_t ukl_clz32(uint32_t x) { return __clz(x); }
UKL_INLINE uint32_t ukl_clz64(uint64_t x) { return __clzll(x); }
UKL_INLINE int64_t ukl_mulhi64(int64_t a, int64_t b) { return __mul64hi(a, b); }
UKL_INLINE uint64_t ukl_umulhi64(uint64_t a, uint64_t b) { return __umul64hi(a, b); }
UKL_INLINE uint32_t ukl_ctz32(uint32_t x) { return x == 0u ? 32u : (uint32_t)(__ffs(x) - 1); }
UKL_INLINE uint32_t ukl_ctz64(uint64_t x) { return x == 0u ? 64u : (uint32_t)(__ffsll(x) - 1); }

UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return __shfl_sync(0xffffffffu, v, lane); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS) { return __shfl_up_sync(0xffffffffu, v, d); }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS) { return __shfl_down_sync(0xffffffffu, v, d); }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return __shfl_xor_sync(0xffffffffu, v, m); }
UKL_INLINE uint32_t ukl_sg_ballot(bool b __UKL_PARAMS) { return __ballot_sync(0xffffffffu, b); }

UKL_INLINE void ukl_sg_barrier() { __syncwarp(); }
UKL_INLINE void ukl_tg_barrier() { __syncthreads(); }
UKL_INLINE void ukl_tg_barrier_device() { __syncthreads(); }

UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return *(volatile uint32_t*)p; }
// A poll (Metal's, above): a volatile load reads the coherent copy on CUDA.
UKL_INLINE uint32_t ukl_atomic_poll(UKL_GLOBAL uint32_t* p) { return *(volatile uint32_t*)p; }
UKL_INLINE void ukl_atomic_store(UKL_GLOBAL uint32_t* p, uint32_t v) { *(volatile uint32_t*)p = v; }
UKL_INLINE uint32_t ukl_atomic_add(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicAdd(p, v); }
UKL_INLINE uint32_t ukl_atomic_sub(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicSub(p, v); }
UKL_INLINE uint32_t ukl_atomic_or(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicOr(p, v); }
UKL_INLINE uint32_t ukl_atomic_and(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicAnd(p, v); }
UKL_INLINE uint32_t ukl_atomic_exchange(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicExch(p, v); }
UKL_INLINE uint32_t ukl_atomic_min(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicMin(p, v); }
UKL_INLINE uint32_t ukl_atomic_max(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicMax(p, v); }
UKL_INLINE uint32_t ukl_atomic_cas(UKL_GLOBAL uint32_t* p, uint32_t e, uint32_t n) { return atomicCAS(p, e, n); }
UKL_INLINE void ukl_fence_device() { __threadfence(); }
UKL_INLINE uint32_t ukl_atomic_add_local(UKL_LOCAL uint32_t* p, uint32_t v) { return atomicAdd(p, v); }
UKL_INLINE uint32_t ukl_atomic_load_local(UKL_LOCAL uint32_t* p) { return *(volatile uint32_t*)p; }
UKL_INLINE void ukl_atomic_store_local(UKL_LOCAL uint32_t* p, uint32_t v) { *(volatile uint32_t*)p = v; }

// The lanes holding the same value as this one (Volta and later).
#define UKL_NATIVE_SG_MATCH 1
UKL_INLINE uint32_t ukl_sg_match_native(uint32_t v __UKL_PARAMS) { return __match_any_sync(0xffffffffu, v); }

#if __CUDA_ARCH__ >= 800
#define UKL_NATIVE_SG_REDUCE 1
UKL_INLINE uint32_t ukl_sg_sum_native(uint32_t v __UKL_PARAMS) { return __reduce_add_sync(0xffffffffu, v); }
UKL_INLINE uint32_t ukl_sg_min_native(uint32_t v __UKL_PARAMS) { return __reduce_min_sync(0xffffffffu, v); }
UKL_INLINE uint32_t ukl_sg_max_native(uint32_t v __UKL_PARAMS) { return __reduce_max_sync(0xffffffffu, v); }
#endif

#elif defined(UKL_CPU)

// The CPU (ukl_cpu_shim.h): a threadgroup's threads are coroutines on one host thread, so every barrier and SIMD-group
// operation is a suspension, and memory is the host's.
#define UKL_INLINE static inline
#define UKL_CALLED static
#ifndef UKL_COLD
#define UKL_COLD static __attribute__((noinline))
#endif
#define UKL_GLOBAL
#define UKL_COHERENT volatile
#define UKL_LOCAL
#define UKL_PRIVATE
#define UKL_TABLE static const
#define UKL_TABLE_SPACE

UKL_INLINE uint32_t ukl_popcount32(uint32_t x) { return (uint32_t)__builtin_popcount(x); }
UKL_INLINE uint32_t ukl_popcount64(uint64_t x) { return (uint32_t)__builtin_popcountll(x); }
UKL_INLINE uint32_t ukl_clz32(uint32_t x) { return x == 0u ? 32u : (uint32_t)__builtin_clz(x); }
UKL_INLINE uint32_t ukl_clz64(uint64_t x) { return x == 0u ? 64u : (uint32_t)__builtin_clzll(x); }
UKL_INLINE int64_t ukl_mulhi64(int64_t a, int64_t b) { return (int64_t)(((__int128)a * (__int128)b) >> 64); }
UKL_INLINE uint64_t ukl_umulhi64(uint64_t a, uint64_t b) { return (uint64_t)(((unsigned __int128)a * (unsigned __int128)b) >> 64); }
UKL_INLINE uint32_t ukl_ctz32(uint32_t x) { return x == 0u ? 32u : (uint32_t)__builtin_ctz(x); }
UKL_INLINE uint32_t ukl_ctz64(uint64_t x) { return x == 0u ? 64u : (uint32_t)__builtin_ctzll(x); }

// A lane past the group's end, or the group's own (`shuffle_up` below lane `d`, `shuffle_down` past the last lane),
// reads its own value, as CUDA's do.
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return (uint32_t)__ukl_cpu_exchange(v, lane); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t l = __ukl_self->lane;
    return (uint32_t)__ukl_cpu_exchange(v, l >= d ? l - d : l);
}
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t l = __ukl_self->lane;
    return (uint32_t)__ukl_cpu_exchange(v, l + d < UKL_SIMD_WIDTH ? l + d : l);
}
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return (uint32_t)__ukl_cpu_exchange(v, __ukl_self->lane ^ m); }
UKL_INLINE uint32_t ukl_sg_ballot(bool b __UKL_PARAMS) { return __ukl_cpu_ballot(b); }

UKL_INLINE void ukl_sg_barrier() { __ukl_cpu_wait(__UKL_AT_SG); }
UKL_INLINE void ukl_tg_barrier() { __ukl_cpu_wait(__UKL_AT_TG); }
UKL_INLINE void ukl_tg_barrier_device() { __ukl_cpu_wait(__UKL_AT_TG); }

UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return __atomic_load_n(p, __ATOMIC_RELAXED); }
// A poll (Metal's, above): an atomic load is coherent on the CPU.
UKL_INLINE uint32_t ukl_atomic_poll(UKL_GLOBAL uint32_t* p) { return __atomic_load_n(p, __ATOMIC_RELAXED); }
UKL_INLINE void ukl_atomic_store(UKL_GLOBAL uint32_t* p, uint32_t v) { __atomic_store_n(p, v, __ATOMIC_RELAXED); }
UKL_INLINE uint32_t ukl_atomic_add(UKL_GLOBAL uint32_t* p, uint32_t v) { return __atomic_fetch_add(p, v, __ATOMIC_RELAXED); }
UKL_INLINE uint32_t ukl_atomic_sub(UKL_GLOBAL uint32_t* p, uint32_t v) { return __atomic_fetch_sub(p, v, __ATOMIC_RELAXED); }
UKL_INLINE uint32_t ukl_atomic_or(UKL_GLOBAL uint32_t* p, uint32_t v) { return __atomic_fetch_or(p, v, __ATOMIC_RELAXED); }
UKL_INLINE uint32_t ukl_atomic_and(UKL_GLOBAL uint32_t* p, uint32_t v) { return __atomic_fetch_and(p, v, __ATOMIC_RELAXED); }
UKL_INLINE uint32_t ukl_atomic_exchange(UKL_GLOBAL uint32_t* p, uint32_t v) { return __atomic_exchange_n(p, v, __ATOMIC_RELAXED); }
UKL_INLINE uint32_t ukl_atomic_min(UKL_GLOBAL uint32_t* p, uint32_t v)
{
    uint32_t x = __atomic_load_n(p, __ATOMIC_RELAXED);
    while (v < x && !__atomic_compare_exchange_n(p, &x, v, true, __ATOMIC_RELAXED, __ATOMIC_RELAXED)) {
    }
    return x;
}
UKL_INLINE uint32_t ukl_atomic_max(UKL_GLOBAL uint32_t* p, uint32_t v)
{
    uint32_t x = __atomic_load_n(p, __ATOMIC_RELAXED);
    while (v > x && !__atomic_compare_exchange_n(p, &x, v, true, __ATOMIC_RELAXED, __ATOMIC_RELAXED)) {
    }
    return x;
}
UKL_INLINE uint32_t ukl_atomic_cas(UKL_GLOBAL uint32_t* p, uint32_t e, uint32_t n)
{
    __atomic_compare_exchange_n(p, &e, n, false, __ATOMIC_RELAXED, __ATOMIC_RELAXED);
    return e;
}
UKL_INLINE void ukl_fence_device() { __atomic_thread_fence(__ATOMIC_SEQ_CST); }
UKL_INLINE uint32_t ukl_atomic_add_local(UKL_LOCAL uint32_t* p, uint32_t v) { return __atomic_fetch_add(p, v, __ATOMIC_RELAXED); }
UKL_INLINE uint32_t ukl_atomic_load_local(UKL_LOCAL uint32_t* p) { return __atomic_load_n(p, __ATOMIC_RELAXED); }
UKL_INLINE void ukl_atomic_store_local(UKL_LOCAL uint32_t* p, uint32_t v) { __atomic_store_n(p, v, __ATOMIC_RELAXED); }

#elif defined(UKL_OPENCL)

// OpenCL C 1.2 (ukl_opencl_shim.h): C with address spaces, clang's `overloadable`, and C11's `_Static_assert`.
#define UKL_INLINE static inline
#define UKL_CALLED static
#ifndef UKL_COLD
#define UKL_COLD static __attribute__((noinline))
#endif
#define UKL_GLOBAL __global
#define UKL_COHERENT volatile __global
#define UKL_LOCAL __local
#define UKL_PRIVATE
#define UKL_TABLE __constant
#define UKL_TABLE_SPACE __constant
#define UKL_OVERLOAD __attribute__((overloadable))
#define UKL_STATIC_ASSERT(c, m) _Static_assert(c, m)

UKL_INLINE uint32_t ukl_popcount32(uint32_t x) { return popcount(x); }
UKL_INLINE uint32_t ukl_popcount64(uint64_t x) { return (uint32_t)popcount(x); }
UKL_INLINE uint32_t ukl_clz32(uint32_t x) { return clz(x); }
UKL_INLINE uint32_t ukl_clz64(uint64_t x) { return (uint32_t)clz(x); }
UKL_INLINE int64_t ukl_mulhi64(int64_t a, int64_t b) { return mul_hi(a, b); }
UKL_INLINE uint64_t ukl_umulhi64(uint64_t a, uint64_t b) { return mul_hi(a, b); }
// OpenCL C 1.2 has no `ctz`: the lowest set bit's position from the count of zeros above it.
UKL_INLINE uint32_t ukl_ctz32(uint32_t x) { return x == 0u ? 32u : 31u - clz(x & (0u - x)); }
UKL_INLINE uint32_t ukl_ctz64(uint64_t x) { return x == 0ul ? 64u : 63u - (uint32_t)clz(x & (0ul - x)); }

UKL_INLINE void ukl_tg_barrier() { barrier(CLK_LOCAL_MEM_FENCE); }
UKL_INLINE void ukl_tg_barrier_device() { barrier(CLK_LOCAL_MEM_FENCE | CLK_GLOBAL_MEM_FENCE); }

#if defined(UKL_OPENCL_EMULATED_SG)
// The SIMD group through threadgroup memory: each thread writes its value to its word of `__ukl_sg`, and reads its
// source lane's. A barrier before the write keeps the last operation's reads ahead of it; one after makes the
// writes seen. OpenCL 1.2's barrier is the workgroup's: every live thread of the threadgroup reaches each operation.
// A lane outside the group, or the group's own (`shuffle_up` below lane `d`, `shuffle_down` past the last lane),
// reads its own value, as CUDA's do.
UKL_INLINE uint32_t __ukl_cl_exchange(uint32_t v, uint32_t from __UKL_PARAMS)
{
    uint32_t l = __ukl_cl_lid();
    barrier(CLK_LOCAL_MEM_FENCE);
    __ukl_sg[l] = v;
    barrier(CLK_LOCAL_MEM_FENCE);
    return __ukl_sg[(l & ~(UKL_SIMD_WIDTH - 1u)) + (from & (UKL_SIMD_WIDTH - 1u))];
}
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return __ukl_cl_exchange(v, lane __UKL_ARGS); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t l = __UKL_CL_LANE;
    return __ukl_cl_exchange(v, l >= d ? l - d : l __UKL_ARGS);
}
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t l = __UKL_CL_LANE;
    return __ukl_cl_exchange(v, l + d < UKL_SIMD_WIDTH ? l + d : l __UKL_ARGS);
}
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return __ukl_cl_exchange(v, __UKL_CL_LANE ^ m __UKL_ARGS); }
// The ballot: every lane reads the group's words.
UKL_INLINE uint32_t ukl_sg_ballot(bool b __UKL_PARAMS)
{
    uint32_t l = __ukl_cl_lid(), base = l & ~(UKL_SIMD_WIDTH - 1u);
    barrier(CLK_LOCAL_MEM_FENCE);
    __ukl_sg[l] = b ? 1u : 0u;
    barrier(CLK_LOCAL_MEM_FENCE);
    uint32_t m = 0u;
    for (uint32_t j = 0u; j < UKL_SIMD_WIDTH; j++)
        m |= __ukl_sg[base + j] << j;
    return m;
}
UKL_INLINE void ukl_sg_barrier() { barrier(CLK_LOCAL_MEM_FENCE | CLK_GLOBAL_MEM_FENCE); }
#elif UKL_SIMD_WIDTH == 1
// A group of one lane: every lane reads its own value.
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return v; }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS) { return v; }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS) { return v; }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return v; }
UKL_INLINE uint32_t ukl_sg_ballot(bool b __UKL_PARAMS) { return b ? 1u : 0u; }
UKL_INLINE void ukl_sg_barrier() { mem_fence(CLK_LOCAL_MEM_FENCE | CLK_GLOBAL_MEM_FENCE); }
#else
#  error "ukl/backend.h: OpenCL's native subgroups (cl_khr_subgroup_shuffle, _ballot) are not written yet (decision 0440 §3.5)"
#endif

// Relaxed atomics on a device word (OpenCL 1.2's are relaxed), and a fence. OpenCL 1.2 has no coherent memory but
// its atomics: a plain load, `volatile` or not, may be answered by a cache another threadgroup's write has not
// reached (Apple's). So an atomic load is an atomic read-modify-write that changes nothing, an atomic store an
// exchange, and a coherent load (`ukl_ldc_*`, ukl/memory.h) reads its words' halves so.
UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return atomic_or((volatile __global uint32_t*)p, 0u); }
// A poll (Metal's, above): OpenCL's atomic load is a read-modify-write already.
UKL_INLINE uint32_t ukl_atomic_poll(UKL_GLOBAL uint32_t* p) { return atomic_or((volatile __global uint32_t*)p, 0u); }
UKL_INLINE void ukl_atomic_store(UKL_GLOBAL uint32_t* p, uint32_t v) { (void)atomic_xchg((volatile __global uint32_t*)p, v); }
UKL_INLINE uint64_t __ukl_cl_coherent_load64(volatile __global const uint64_t* p)
{
    volatile __global uint32_t* w = (volatile __global uint32_t*)p;
    return (uint64_t)atomic_or(w, 0u) | ((uint64_t)atomic_or(w + 1, 0u) << 32);
}
#define UKL_COHERENT_LOAD64(p) __ukl_cl_coherent_load64(p)
// The bits `m` of the 64-bit word at `p` set to `x`'s, by halves: each half's bits cleared, then set, by atomics that
// leave its other bits as another thread writes them.
UKL_INLINE void __ukl_cl_coherent_store_bits64(volatile __global uint64_t* p, uint64_t m, uint64_t x)
{
    volatile __global uint32_t* w = (volatile __global uint32_t*)p;
    uint32_t ml = (uint32_t)m, mh = (uint32_t)(m >> 32), xl = (uint32_t)x & ml, xh = (uint32_t)(x >> 32) & mh;
    if (ml != 0u) {
        atomic_and(w, ~ml);
        atomic_or(w, xl);
    }
    if (mh != 0u) {
        atomic_and(w + 1, ~mh);
        atomic_or(w + 1, xh);
    }
}
#define UKL_COHERENT_STORE_BITS64(p, m, x) __ukl_cl_coherent_store_bits64(p, m, x)
UKL_INLINE uint32_t ukl_atomic_add(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_add((volatile __global uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_sub(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_sub((volatile __global uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_or(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_or((volatile __global uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_and(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_and((volatile __global uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_exchange(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_xchg((volatile __global uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_min(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_min((volatile __global uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_max(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_max((volatile __global uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_cas(UKL_GLOBAL uint32_t* p, uint32_t e, uint32_t n) { return atomic_cmpxchg((volatile __global uint32_t*)p, e, n); }
UKL_INLINE void ukl_fence_device() { mem_fence(CLK_GLOBAL_MEM_FENCE); }
UKL_INLINE uint32_t ukl_atomic_add_local(UKL_LOCAL uint32_t* p, uint32_t v) { return atomic_add((volatile __local uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_load_local(UKL_LOCAL uint32_t* p) { return *(volatile __local uint32_t*)p; }
UKL_INLINE void ukl_atomic_store_local(UKL_LOCAL uint32_t* p, uint32_t v) { atomic_xchg((volatile __local uint32_t*)p, v); }
#define UKL_CONSTANT_U32(name) __constant uint32_t name = (UKL_CONSTANT_0);

#else
#  error "the UKL library has no intrinsics for this backend (ukl/backend.h)"
#endif

// Written once for every backend whose language is C++ (Metal, CUDA, the CPU); a C backend (OpenCL) defines them in
// its section. A function defined for several types (by a macro over the type) is `UKL_OVERLOAD`, so its callers
// need not name the type; a check on types and layouts is `UKL_STATIC_ASSERT`.
#ifndef UKL_OVERLOAD
#define UKL_OVERLOAD
#endif
#ifndef UKL_STATIC_ASSERT
#define UKL_STATIC_ASSERT(c, m) static_assert(c, m)
#endif

// A program's specialization constant (decision 0440 step B): `UKL_CONSTANT_U32(name)` declares `name`, a uint32_t
// fixed when the program is made, so the compiler folds it (the label passes' tape format). Metal's function constant
// 0; elsewhere the define `UKL_CONSTANT_0` each value's program is compiled with (`rerail_gpu::kernels::program_with`).
#ifndef UKL_CONSTANT_U32
#define UKL_CONSTANT_U32(name) static const uint32_t name = (UKL_CONSTANT_0);
#endif
