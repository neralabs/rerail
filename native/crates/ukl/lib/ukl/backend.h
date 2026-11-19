// ukl/backend.h: the intrinsics the UKL library is written over, one section per backend (decision 0400 §5).
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

#if defined(UKL_METAL)

// Qualifiers: a function the library inlines; a struct's method; device memory; device memory another threadgroup
// of the same dispatch writes (coherent); threadgroup memory; a thread's own.
#define UKL_INLINE static inline
// A function written as a rare case (a generated read marked cold, decision 0310): a call, not inlined. Inlined at
// every call site, 16z's block decode made the view reads' test kernel take 387 s in Metal's back end (2.9 s as
// calls). A parent walk calling `clif_tape_parent` runs 1.3% slower than with it inlined. A kernel may define
// `UKL_COLD` as `UKL_INLINE` before its includes.
#ifndef UKL_COLD
#define UKL_COLD static __attribute__((noinline))
#endif
#define UKL_METHOD inline
#define UKL_GLOBAL device
#define UKL_COHERENT device coherent(device)
#define UKL_LOCAL threadgroup
#define UKL_PRIVATE thread

UKL_INLINE uint32_t ukl_popcount32(uint32_t x) { return popcount(x); }
UKL_INLINE uint32_t ukl_popcount64(uint64_t x) { return (uint32_t)popcount(x); }
UKL_INLINE uint32_t ukl_clz32(uint32_t x) { return clz(x); }
UKL_INLINE uint32_t ukl_clz64(uint64_t x) { return (uint32_t)clz(x); }
UKL_INLINE uint32_t ukl_ctz32(uint32_t x) { return ctz(x); }
UKL_INLINE uint32_t ukl_ctz64(uint64_t x) { return (uint32_t)ctz(x); }

UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane) { return simd_shuffle(v, (ushort)lane); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d) { return simd_shuffle_up(v, (ushort)d); }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d) { return simd_shuffle_down(v, (ushort)d); }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m) { return simd_shuffle_xor(v, (ushort)m); }
UKL_INLINE uint32_t ukl_sg_ballot(bool b) { return (uint32_t)(uint64_t)(simd_vote::vote_t)simd_ballot(b); }

UKL_INLINE void ukl_sg_barrier() { simdgroup_barrier(mem_flags::mem_device | mem_flags::mem_threadgroup); }
UKL_INLINE void ukl_tg_barrier() { threadgroup_barrier(mem_flags::mem_threadgroup); }
UKL_INLINE void ukl_tg_barrier_device() { threadgroup_barrier(mem_flags::mem_device | mem_flags::mem_threadgroup); }

// Relaxed atomics on a device word, and a device-wide fence.
UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return atomic_load_explicit((device atomic_uint*)p, memory_order_relaxed); }
UKL_INLINE void ukl_atomic_store(UKL_GLOBAL uint32_t* p, uint32_t v) { atomic_store_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_add(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_add_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_sub(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_sub_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
UKL_INLINE uint32_t ukl_atomic_or(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomic_fetch_or_explicit((device atomic_uint*)p, v, memory_order_relaxed); }
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

// Native forms.
#define UKL_NATIVE_SG_REDUCE 1
UKL_INLINE uint32_t ukl_sg_sum_native(uint32_t v) { return simd_sum(v); }
UKL_INLINE uint32_t ukl_sg_min_native(uint32_t v) { return simd_min(v); }
UKL_INLINE uint32_t ukl_sg_max_native(uint32_t v) { return simd_max(v); }
#define UKL_NATIVE_SG_SCAN 1
UKL_INLINE uint32_t ukl_sg_inclusive_sum_native(uint32_t v) { return simd_prefix_inclusive_sum(v); }
UKL_INLINE uint32_t ukl_sg_exclusive_sum_native(uint32_t v) { return simd_prefix_exclusive_sum(v); }

#elif defined(UKL_CUDA)

#define UKL_INLINE static __device__ __forceinline__
// A rare case, called (not inlined): forced inline at every call site, a large cold read (16z's block decode) made
// NVRTC take minutes and gigabytes on a kernel that reads every node's every field.
#ifndef UKL_COLD
#define UKL_COLD static __device__ __noinline__
#endif
#define UKL_METHOD __device__ __forceinline__
#define UKL_GLOBAL
#define UKL_COHERENT volatile
#define UKL_LOCAL
#define UKL_PRIVATE

UKL_INLINE uint32_t ukl_popcount32(uint32_t x) { return __popc(x); }
UKL_INLINE uint32_t ukl_popcount64(uint64_t x) { return __popcll(x); }
UKL_INLINE uint32_t ukl_clz32(uint32_t x) { return __clz(x); }
UKL_INLINE uint32_t ukl_clz64(uint64_t x) { return __clzll(x); }
UKL_INLINE uint32_t ukl_ctz32(uint32_t x) { return x == 0u ? 32u : (uint32_t)(__ffs(x) - 1); }
UKL_INLINE uint32_t ukl_ctz64(uint64_t x) { return x == 0u ? 64u : (uint32_t)(__ffsll(x) - 1); }

UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane) { return __shfl_sync(0xffffffffu, v, lane); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d) { return __shfl_up_sync(0xffffffffu, v, d); }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d) { return __shfl_down_sync(0xffffffffu, v, d); }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m) { return __shfl_xor_sync(0xffffffffu, v, m); }
UKL_INLINE uint32_t ukl_sg_ballot(bool b) { return __ballot_sync(0xffffffffu, b); }

UKL_INLINE void ukl_sg_barrier() { __syncwarp(); }
UKL_INLINE void ukl_tg_barrier() { __syncthreads(); }
UKL_INLINE void ukl_tg_barrier_device() { __syncthreads(); }

UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return *(volatile uint32_t*)p; }
UKL_INLINE void ukl_atomic_store(UKL_GLOBAL uint32_t* p, uint32_t v) { *(volatile uint32_t*)p = v; }
UKL_INLINE uint32_t ukl_atomic_add(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicAdd(p, v); }
UKL_INLINE uint32_t ukl_atomic_sub(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicSub(p, v); }
UKL_INLINE uint32_t ukl_atomic_or(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicOr(p, v); }
UKL_INLINE uint32_t ukl_atomic_exchange(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicExch(p, v); }
UKL_INLINE uint32_t ukl_atomic_min(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicMin(p, v); }
UKL_INLINE uint32_t ukl_atomic_max(UKL_GLOBAL uint32_t* p, uint32_t v) { return atomicMax(p, v); }
UKL_INLINE uint32_t ukl_atomic_cas(UKL_GLOBAL uint32_t* p, uint32_t e, uint32_t n) { return atomicCAS(p, e, n); }
UKL_INLINE void ukl_fence_device() { __threadfence(); }
UKL_INLINE uint32_t ukl_atomic_add_local(UKL_LOCAL uint32_t* p, uint32_t v) { return atomicAdd(p, v); }

// The lanes holding the same value as this one (Volta and later).
#define UKL_NATIVE_SG_MATCH 1
UKL_INLINE uint32_t ukl_sg_match_native(uint32_t v) { return __match_any_sync(0xffffffffu, v); }

#if __CUDA_ARCH__ >= 800
#define UKL_NATIVE_SG_REDUCE 1
UKL_INLINE uint32_t ukl_sg_sum_native(uint32_t v) { return __reduce_add_sync(0xffffffffu, v); }
UKL_INLINE uint32_t ukl_sg_min_native(uint32_t v) { return __reduce_min_sync(0xffffffffu, v); }
UKL_INLINE uint32_t ukl_sg_max_native(uint32_t v) { return __reduce_max_sync(0xffffffffu, v); }
#endif

#else
#  error "the UKL library has no intrinsics for this backend (ukl/backend.h)"
#endif
