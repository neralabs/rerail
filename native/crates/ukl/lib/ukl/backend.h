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

// A SIMD group's lanes as bits, lane `l` at bit `l` (a ballot's, a match's): 32 bits, or 64 where the group is wider
// (AMD's 64 lanes; the CPU at `RERAIL_CPU_SIMD_WIDTH=64`). `ukl_mask_*` below read it whatever its width.
#if UKL_SIMD_WIDTH > 32
typedef uint64_t ukl_mask_t;
#else
typedef uint32_t ukl_mask_t;
#endif

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
// A function the compiler inlines or calls as it chooses (`UKL_FUNC`): a generated program's (`clif_ukl`), large and
// many, and a small read that should inline where it is called (`clif_t_ld`).
#define UKL_FUNC static
#ifndef UKL_COLD
#define UKL_COLD static __attribute__((noinline))
#endif
// A function compiled once and called, however many sites call it: a grid-wide operation (`grid.h`: its barriers
// cost far more than a call), or a large body at many sites (a kernel's tape reads: 12 to 46 KB each inlined, at
// ~600 sites they made the preprocessor's setup kernel 14 MB of code and its Metal back end's compile minutes).
#ifndef UKL_ONCE
#define UKL_ONCE static __attribute__((noinline))
#endif
// A small read many places of a kernel reach (a tape read, a node's, a relation's column): `UKL_SHARED`. A large
// body many reach (an interpreter's step, its evaluators, a join stage's row writer): `UKL_LARGE`. Both inlined
// here; calls on CUDA and OpenCL (decision 0576 §22). A rare case's body, called on every
// backend but OpenCL: `UKL_RARE`.
#define UKL_SHARED UKL_INLINE
#define UKL_LARGE UKL_INLINE
#define UKL_RARE UKL_COLD
#define UKL_GLOBAL device
#define UKL_COHERENT device coherent(device)
#define UKL_LOCAL threadgroup
#define UKL_PRIVATE thread
// A table at file scope (`UKL_TABLE uint32_t t[2]={1u,2u};`: read-only on every backend, the macro its qualifiers) and a
// pointer into one (`UKL_TABLE_SPACE const T*`).
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
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS) { return (ukl_mask_t)(uint64_t)(simd_vote::vote_t)simd_ballot(b); }

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
// An f16's bits as an f32 (ukl/dot.h): Metal's half. Metal has no integer or mixed-precision dot product, so
// ukl/dot.h's are the reference's arithmetic over this conversion.
#define UKL_NATIVE_F16 1
UKL_INLINE float ukl_f16_to_f32_native(uint32_t h) { return float(as_type<half>((ushort)h)); }

#elif defined(UKL_CUDA)

#define UKL_INLINE static __device__ __forceinline__
// A rare case, called (not inlined): forced inline at every call site, a large cold read (16z's block decode) made
// NVRTC take minutes and gigabytes on a kernel that reads every node's every field.
// A function the compiler inlines or calls as it chooses (`UKL_FUNC`): a generated program's (`clif_ukl`), large and
// many, and a small read that should inline where it is called (`clif_t_ld`).
#define UKL_FUNC static __device__
#ifndef UKL_COLD
#define UKL_COLD static __device__ __noinline__
#endif
// A function compiled once and called, however many sites call it: a grid-wide operation (`grid.h`: its barriers
// cost far more than a call), or a large body at many sites (a kernel's tape reads: 12 to 46 KB each inlined, at
// ~600 sites they made the preprocessor's setup kernel 14 MB of code and its Metal back end's compile minutes).
#ifndef UKL_ONCE
#define UKL_ONCE static __device__ __noinline__
#endif
// A small read many places of a kernel reach (a tape read, a node's, a relation's column): `UKL_SHARED`. A large body
// many reach (an interpreter's step, its evaluators, a join stage's row writer): `UKL_LARGE`. Both calls here
// (decision 0576 §22): NVRTC compiles each inlined copy, and the transducer's tests' interpreter and JIT kernels
// (`all_backends_execute_every_verified_instruction_and_mode`, `wildcard_and_missing_flags_preserve_native_node_meaning`)
// took it 40 s and 26 s with both called, 187 s and 131 s with the reads inlined, over 240 s with both, forced or at
// its choice. A rare case's body, called: `UKL_RARE`.
#define UKL_SHARED static __device__ __noinline__
#define UKL_LARGE UKL_SHARED
#define UKL_RARE UKL_COLD
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

#if defined(UKL_HIP)
// HIP (AMD's wavefronts, 32 or 64 lanes): the shuffles take no mask, as every lane of a wavefront runs them together;
// a lane past the group's edge keeps its own value (`__shfl_up`, `__shfl_down`), as CUDA's. The ballot is 64 bits,
// a 32-lane wavefront's in its low half.
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return __shfl(v, (int)lane, UKL_SIMD_WIDTH); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS) { return __shfl_up(v, d, UKL_SIMD_WIDTH); }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS) { return __shfl_down(v, d, UKL_SIMD_WIDTH); }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return __shfl_xor(v, (int)m, UKL_SIMD_WIDTH); }
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS) { return (ukl_mask_t)__ballot(b); }
UKL_INLINE void ukl_sg_barrier() { __UKL_SYNCWARP(); }
#else
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return __shfl_sync(0xffffffffu, v, lane); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS) { return __shfl_up_sync(0xffffffffu, v, d); }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS) { return __shfl_down_sync(0xffffffffu, v, d); }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return __shfl_xor_sync(0xffffffffu, v, m); }
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS) { return __ballot_sync(0xffffffffu, b); }
UKL_INLINE void ukl_sg_barrier() { __syncwarp(); }
#endif
#if defined(UKL_HIP)
// A threadgroup barrier orders threadgroup memory alone, as Metal's (`mem_threadgroup`) and OpenCL's
// (`CLK_LOCAL_MEM_FENCE`): the fences are the workgroup's on LDS ("local"). HIP's `__syncthreads()` fences every
// address space, so at each barrier it waited for the wave's global stores (`s_waitcnt_vscnt 0`) and invalidated the
// vector L0 cache (`buffer_gl0_inv`): in the radix sort's scatter, after each round's writes. That was about 1 ms of
// a 19 ms pass of 4M rows on a Radeon 680M (0440, "HIP's copies and fills, and its other overheads").
UKL_INLINE void ukl_tg_barrier()
{
    __builtin_amdgcn_fence(__ATOMIC_RELEASE, "workgroup", "local");
    __builtin_amdgcn_s_barrier();
    __builtin_amdgcn_fence(__ATOMIC_ACQUIRE, "workgroup", "local");
}
#else
UKL_INLINE void ukl_tg_barrier() { __syncthreads(); }
#endif
UKL_INLINE void ukl_tg_barrier_device() { __syncthreads(); }

UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return *(volatile uint32_t*)p; }
// A poll of a word another group writes (Metal's comment): a volatile load reaches L2 here.
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
// The same atomics on a coherent word (`UKL_COHERENT`, volatile here), as Metal's: C++ takes no volatile pointer for
// a plain one, and an atomic is served where the word's writes meet already.
#define __UKL_COHERENT_ATOMIC(op)                                                                                     \
    UKL_INLINE uint32_t ukl_atomic_##op(UKL_COHERENT uint32_t* p, uint32_t v) { return ukl_atomic_##op((UKL_GLOBAL uint32_t*)p, v); }
__UKL_COHERENT_ATOMIC(add)
__UKL_COHERENT_ATOMIC(sub)
__UKL_COHERENT_ATOMIC(or)
__UKL_COHERENT_ATOMIC(and)
__UKL_COHERENT_ATOMIC(exchange)
__UKL_COHERENT_ATOMIC(min)
__UKL_COHERENT_ATOMIC(max)
#undef __UKL_COHERENT_ATOMIC
UKL_INLINE uint32_t ukl_atomic_load(UKL_COHERENT uint32_t* p) { return ukl_atomic_load((UKL_GLOBAL uint32_t*)p); }
UKL_INLINE uint32_t ukl_atomic_poll(UKL_COHERENT uint32_t* p) { return ukl_atomic_poll((UKL_GLOBAL uint32_t*)p); }
UKL_INLINE void ukl_atomic_store(UKL_COHERENT uint32_t* p, uint32_t v) { ukl_atomic_store((UKL_GLOBAL uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_cas(UKL_COHERENT uint32_t* p, uint32_t e, uint32_t n) { return ukl_atomic_cas((UKL_GLOBAL uint32_t*)p, e, n); }
UKL_INLINE uint32_t ukl_atomic_add_local(UKL_LOCAL uint32_t* p, uint32_t v) { return atomicAdd(p, v); }
UKL_INLINE uint32_t ukl_atomic_load_local(UKL_LOCAL uint32_t* p) { return *(volatile uint32_t*)p; }
UKL_INLINE void ukl_atomic_store_local(UKL_LOCAL uint32_t* p, uint32_t v) { *(volatile uint32_t*)p = v; }

#if defined(UKL_HIP)
// ukl/dot.h on HIP: an f16's conversion (the compiler's `_Float16`), and on RDNA2 (`UKL_HIP_AMD_DOT`, the device's
// define, `CudaDevice::defines`) the byte and f16 dot products, as OpenCL's AMD section (`v_dot2_f32_f16` rounds
// the exact sum once). No bf16 form before RDNA3's.
#define UKL_NATIVE_F16 1
UKL_INLINE float ukl_f16_to_f32_native(uint32_t h) { return (float)__builtin_bit_cast(_Float16, (uint16_t)h); }
#if defined(UKL_HIP_AMD_DOT)
typedef _Float16 __ukl_half2 __attribute__((ext_vector_type(2)));
#define UKL_NATIVE_DOT4 1
UKL_INLINE int32_t ukl_dot4_i8_native(uint32_t a, uint32_t b, int32_t acc) { return __builtin_amdgcn_sdot4((int)a, (int)b, acc, false); }
UKL_INLINE uint32_t ukl_dot4_u8_native(uint32_t a, uint32_t b, uint32_t acc) { return __builtin_amdgcn_udot4(a, b, acc, false); }
#define UKL_NATIVE_DOT2_F16 1
UKL_INLINE float ukl_dot2_f16_native(uint32_t a, uint32_t b, float acc)
{
    return __builtin_amdgcn_fdot2(__builtin_bit_cast(__ukl_half2, a), __builtin_bit_cast(__ukl_half2, b), acc, false);
}
#endif
#else
// The lanes holding the same value as this one (Volta and later).
#define UKL_NATIVE_SG_MATCH 1
UKL_INLINE ukl_mask_t ukl_sg_match_native(uint32_t v __UKL_PARAMS) { return __match_any_sync(0xffffffffu, v); }

// ukl/dot.h: an f16's conversion (PTX's, with no header), and the byte dot products (`dp4a`, sm_61 and later). CUDA
// has no f32-accumulating dot product of f16s or bf16s: those are the reference's arithmetic.
#define UKL_NATIVE_F16 1
UKL_INLINE float ukl_f16_to_f32_native(uint32_t h)
{
    float f;
    asm("cvt.f32.f16 %0, %1;" : "=f"(f) : "h"((unsigned short)h));
    return f;
}
#if __CUDA_ARCH__ >= 610
#define UKL_NATIVE_DOT4 1
UKL_INLINE int32_t ukl_dot4_i8_native(uint32_t a, uint32_t b, int32_t acc) { return __dp4a((int)a, (int)b, acc); }
UKL_INLINE uint32_t ukl_dot4_u8_native(uint32_t a, uint32_t b, uint32_t acc) { return __dp4a(a, b, acc); }
#endif

#endif

#if !defined(UKL_HIP) && __CUDA_ARCH__ >= 800
#define UKL_NATIVE_SG_REDUCE 1
UKL_INLINE uint32_t ukl_sg_sum_native(uint32_t v __UKL_PARAMS) { return __reduce_add_sync(0xffffffffu, v); }
UKL_INLINE uint32_t ukl_sg_min_native(uint32_t v __UKL_PARAMS) { return __reduce_min_sync(0xffffffffu, v); }
UKL_INLINE uint32_t ukl_sg_max_native(uint32_t v __UKL_PARAMS) { return __reduce_max_sync(0xffffffffu, v); }
#endif

#elif defined(UKL_CPU)

// The CPU (ukl_cpu_shim.h): a threadgroup's threads are coroutines on one host thread, so every barrier and SIMD-group
// operation is a suspension, and memory is the host's.
#define UKL_INLINE static inline
// A function the compiler inlines or calls as it chooses (`UKL_FUNC`): a generated program's (`clif_ukl`), large and
// many, and a small read that should inline where it is called (`clif_t_ld`).
#define UKL_FUNC static
#ifndef UKL_COLD
#define UKL_COLD static __attribute__((noinline))
#endif
// A function compiled once and called, however many sites call it: a grid-wide operation (`grid.h`: its barriers
// cost far more than a call), or a large body at many sites (a kernel's tape reads: 12 to 46 KB each inlined, at
// ~600 sites they made the preprocessor's setup kernel 14 MB of code and its Metal back end's compile minutes).
#ifndef UKL_ONCE
#define UKL_ONCE static __attribute__((noinline))
#endif
// A small read many places of a kernel reach (a tape read, a node's, a relation's column): `UKL_SHARED`. A large
// body many reach (an interpreter's step, its evaluators, a join stage's row writer): `UKL_LARGE`. Both inlined
// here; calls on CUDA and OpenCL (decision 0576 §22). A rare case's body, called on every
// backend but OpenCL: `UKL_RARE`.
#define UKL_SHARED UKL_INLINE
#define UKL_LARGE UKL_INLINE
#define UKL_RARE UKL_COLD
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
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS) { return (ukl_mask_t)__ukl_cpu_ballot(b); }
// Native forms: a reduction or a scan is one wait of the SIMD group, not a shuffle's per step.
#define UKL_NATIVE_SG_REDUCE 1
UKL_INLINE uint32_t ukl_sg_sum_native(uint32_t v __UKL_PARAMS) { return __ukl_cpu_reduce(v, 0u); }
UKL_INLINE uint32_t ukl_sg_min_native(uint32_t v __UKL_PARAMS) { return __ukl_cpu_reduce(v, 1u); }
UKL_INLINE uint32_t ukl_sg_max_native(uint32_t v __UKL_PARAMS) { return __ukl_cpu_reduce(v, 2u); }
#define UKL_NATIVE_SG_SCAN 1
UKL_INLINE uint32_t ukl_sg_inclusive_sum_native(uint32_t v __UKL_PARAMS) { return __ukl_cpu_prefix(v, true); }
UKL_INLINE uint32_t ukl_sg_exclusive_sum_native(uint32_t v __UKL_PARAMS) { return __ukl_cpu_prefix(v, false); }
#define UKL_NATIVE_SG_SEG 1
UKL_INLINE uint32_t ukl_sg_seg_inclusive_native(bool add, uint32_t f, uint32_t v, uint32_t* fin __UKL_PARAMS) { return __ukl_cpu_seg(add, f, v, fin); }
// The scans with the SIMD group's whole aggregate, read from the same operation's results.
#define UKL_NATIVE_SG_TOTALS 1
UKL_INLINE uint32_t ukl_sg_inclusive_sum_total_native(uint32_t v, uint32_t* total __UKL_PARAMS) { return __ukl_cpu_prefix_total(v, total); }
UKL_INLINE uint32_t ukl_sg_seg_inclusive_total_native(bool add, uint32_t f, uint32_t v, uint32_t* fin, uint32_t* tv, uint32_t* tf __UKL_PARAMS)
{
    return __ukl_cpu_seg_total(add, f, v, fin, tv, tf);
}
UKL_INLINE uint32_t ukl_sg_seg_inclusive_ex_native(bool add, uint32_t f, uint32_t v, uint32_t* fin, uint32_t* ev, uint32_t* ef, uint32_t* tv,
                                                  uint32_t* tf __UKL_PARAMS)
{
    return __ukl_cpu_seg_ex(add, f, v, fin, ev, ef, tv, tf);
}
// A scan's tile is 16 values a thread: each of the group's operations switches all its threads.
#define UKL_SCAN_VT 16u

UKL_INLINE void ukl_sg_barrier() { __ukl_cpu_wait(__UKL_AT_SG); }
UKL_INLINE void ukl_tg_barrier() { __ukl_cpu_wait(__UKL_AT_TG); }
UKL_INLINE void ukl_tg_barrier_device() { __ukl_cpu_wait(__UKL_AT_TG); }
// A poll of another group's progress (a grid barrier's): the thread suspends, and once its whole group polls, the
// group yields to the others that run with it (ukl_cpu_shim.h). So a poll there is a round of every other group, and
// a grid barrier's bound is a thousand rounds, not a GPU's million spins.
#define ukl_poll_yield() __ukl_cpu_wait(__UKL_YIELD)
// The groups that run together, known exactly (the launch's `resident`): a grid's join closes when they all have
// joined, with no window (decision 0540 §3).
#define UKL_GRID_RESIDENT() __ukl_cpu_resident()
#ifndef UKL_GRID_SPINS
#define UKL_GRID_SPINS 1000u
#endif

UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return __atomic_load_n(p, __ATOMIC_RELAXED); }
// A poll of a word another group writes (Metal's comment): the host's loads are coherent.
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
// A function the compiler inlines or calls as it chooses (`UKL_FUNC`): a generated program's (`clif_ukl`), large and
// many, and a small read that should inline where it is called (`clif_t_ld`).
#define UKL_FUNC static
#ifndef UKL_COLD
#define UKL_COLD static __attribute__((noinline))
#endif
// A function compiled once and called, however many sites call it: a grid-wide operation (`grid.h`: its barriers
// cost far more than a call), or a large body at many sites (a kernel's tape reads: 12 to 46 KB each inlined, at
// ~600 sites they made the preprocessor's setup kernel 14 MB of code and its Metal back end's compile minutes).
#ifndef UKL_ONCE
#define UKL_ONCE static __attribute__((noinline))
#endif
// Intel's compiler (IGC) compiles each inlined copy of a body apart, and its time follows the code reached through
// inlining: the interpreter inlined took 10.7 s on an Arc 140V, its shared bodies called 5.8 s (decision 0576 §13).
// So here a shared read and a large body (`UKL_SHARED`, `UKL_LARGE`) are calls. A rare case's body is inlined:
// called from a kernel with a required sub-group size (every kernel here), IGC misread the 16 format's side read at
// 16 lanes (decision 0576 §19).
#define UKL_SHARED static __attribute__((noinline))
#define UKL_LARGE UKL_SHARED
#define UKL_RARE UKL_INLINE
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

// ukl/dot.h: an f16's conversion, OpenCL C's own (`vload_half`, which needs no `cl_khr_fp16`).
#define UKL_NATIVE_F16 1
UKL_INLINE float ukl_f16_to_f32_native(uint32_t h)
{
    ushort v = (ushort)h;
    return vload_half(0, (const __private half*)&v);
}
#if defined(UKL_OPENCL_AMD_DOT)
// AMD's dot products (RDNA2, `gfx103x`: the host defines `UKL_OPENCL_AMD_DOT` for those alone, whose instructions
// these are): `v_dot4_i32_i8`, `v_dot4_u32_u8` and `v_dot2_f32_f16`, none clamping. The last rounds the exact sum once
// (ukl/dot.h).
#pragma OPENCL EXTENSION cl_khr_fp16 : enable
#define UKL_NATIVE_DOT4 1
UKL_INLINE int32_t ukl_dot4_i8_native(uint32_t a, uint32_t b, int32_t acc) { return __builtin_amdgcn_sdot4((int)a, (int)b, acc, false); }
UKL_INLINE uint32_t ukl_dot4_u8_native(uint32_t a, uint32_t b, uint32_t acc) { return __builtin_amdgcn_udot4(a, b, acc, false); }
#define UKL_NATIVE_DOT2_F16 1
UKL_INLINE float ukl_dot2_f16_native(uint32_t a, uint32_t b, float acc) { return __builtin_amdgcn_fdot2(as_half2(a), as_half2(b), acc, false); }
#elif defined(UKL_OPENCL_INT_DOT)
// `cl_khr_integer_dot_product` (OpenCL C 3.0): the bytes' dot product, added to `acc` wrapping.
#define UKL_NATIVE_DOT4 1
UKL_INLINE int32_t ukl_dot4_i8_native(uint32_t a, uint32_t b, int32_t acc) { return (int32_t)((uint32_t)acc + (uint32_t)dot(as_char4(a), as_char4(b))); }
UKL_INLINE uint32_t ukl_dot4_u8_native(uint32_t a, uint32_t b, uint32_t acc) { return acc + dot(as_uchar4(a), as_uchar4(b)); }
#endif

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
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS)
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
#elif defined(UKL_OPENCL_SUBGROUPS) && defined(UKL_OPENCL_AMD)
// AMD's waves, `UKL_SIMD_WIDTH` (32) lanes (the host checks the width at the device's opening): AMD's OpenCL 2.0 has
// `cl_khr_subgroups` (reductions, scans, broadcast, barrier) but no shuffle or ballot extension, so those are the
// wave's own: `ds_bpermute` (each lane reads the lane at its byte address, a lane's 4 bytes) and `ballot_w32`. A lane
// outside the group, or the group's own (`shuffle_up` below lane `d`, `shuffle_down` past the last lane), reads its own
// value, as CUDA's and the emulation's do.
UKL_INLINE uint32_t __ukl_amd_read(uint32_t v, uint32_t from) { return (uint32_t)__builtin_amdgcn_ds_bpermute((int)((from & (UKL_SIMD_WIDTH - 1u)) << 2), (int)v); }
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return __ukl_amd_read(v, lane); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t l = __UKL_CL_LANE;
    return __ukl_amd_read(v, l >= d ? l - d : l);
}
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t l = __UKL_CL_LANE;
    return __ukl_amd_read(v, l + d < UKL_SIMD_WIDTH ? l + d : l);
}
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return __ukl_amd_read(v, __UKL_CL_LANE ^ m); }
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS) { return (ukl_mask_t)__builtin_amdgcn_ballot_w32(b); }
UKL_INLINE void ukl_sg_barrier() { sub_group_barrier(CLK_LOCAL_MEM_FENCE | CLK_GLOBAL_MEM_FENCE); }
// No native forms: on a Radeon 680M (gfx1035, driver 32.0.21030.2001) `cl_khr_subgroups`' scans and reductions in
// the engine's kernels worked on each 16-lane row of the 32-lane wave apart, while a small kernel's were right: the
// block interpreter's `sub_group_scan_inclusive_add` scanned the second row without the first row's total, and the
// device scan's `sub_group_reduce_add` gave a 1,024-value tile half its sum (a kind-set axis lost half its rows). So
// both are the shuffles' (`ukl/simd.h`), `ds_bpermute` above.
#elif defined(UKL_OPENCL_SUBGROUPS)
// The device's sub-groups, `UKL_SIMD_WIDTH` lanes (the shim requires it of each kernel): OpenCL 3.0's functions. A lane outside
// the group, or the group's own (`shuffle_up` below lane `d`, `shuffle_down` past the last lane), reads its own
// value, as CUDA's and the emulation's do; the device's functions leave those undefined.
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return sub_group_shuffle(v, lane & (UKL_SIMD_WIDTH - 1u)); }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t r = sub_group_shuffle_up(v, d);
    return __UKL_CL_LANE >= d ? r : v;
}
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS)
{
    uint32_t r = sub_group_shuffle_down(v, d);
    return __UKL_CL_LANE + d < UKL_SIMD_WIDTH ? r : v;
}
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return sub_group_shuffle_xor(v, m & (UKL_SIMD_WIDTH - 1u)); }
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS) { return sub_group_ballot(b ? 1 : 0).x; }
UKL_INLINE void ukl_sg_barrier() { sub_group_barrier(CLK_LOCAL_MEM_FENCE | CLK_GLOBAL_MEM_FENCE); }
#if defined(__UKL_VEC)
// The CPU's vector code (rerail-ukl-vec, decision 0567): the group's reductions in one operation each, whose value
// is the group's (the butterfly of shuffles is every lane's too, but that it agrees is not the shuffles' to show).
#define UKL_NATIVE_SG_REDUCE 1
UKL_INLINE uint32_t ukl_sg_sum_native(uint32_t v __UKL_PARAMS) { return sub_group_reduce_add(v); }
UKL_INLINE uint32_t ukl_sg_min_native(uint32_t v __UKL_PARAMS) { return sub_group_reduce_min(v); }
UKL_INLINE uint32_t ukl_sg_max_native(uint32_t v __UKL_PARAMS) { return sub_group_reduce_max(v); }
// A poll of another group's progress: the group yields to the groups that run with it, as the CPU's coroutines
// do (a persistent grid's barrier is met there by its resident groups taking turns).
#define ukl_poll_yield() __ukl_vec_yield()
#ifndef UKL_GRID_SPINS
#define UKL_GRID_SPINS 1000u
#endif
#endif
#elif UKL_SIMD_WIDTH == 1
// A group of one lane: every lane reads its own value.
UKL_INLINE uint32_t ukl_sg_shuffle(uint32_t v, uint32_t lane __UKL_PARAMS) { return v; }
UKL_INLINE uint32_t ukl_sg_shuffle_up(uint32_t v, uint32_t d __UKL_PARAMS) { return v; }
UKL_INLINE uint32_t ukl_sg_shuffle_down(uint32_t v, uint32_t d __UKL_PARAMS) { return v; }
UKL_INLINE uint32_t ukl_sg_shuffle_xor(uint32_t v, uint32_t m __UKL_PARAMS) { return v; }
UKL_INLINE ukl_mask_t ukl_sg_ballot(bool b __UKL_PARAMS) { return b ? 1u : 0u; }
UKL_INLINE void ukl_sg_barrier() { mem_fence(CLK_LOCAL_MEM_FENCE | CLK_GLOBAL_MEM_FENCE); }
#else
#  error "ukl/backend.h: OpenCL's emulated SIMD groups are 32 lanes or 1"
#endif

// Relaxed atomics on a device word (OpenCL 1.2's are relaxed), and a fence. OpenCL 1.2 has no coherent memory but
// its atomics: a plain load, `volatile` or not, may be answered by a cache another threadgroup's write has not
// reached (Apple's). So an atomic load is an atomic read-modify-write that changes nothing, an atomic store an
// exchange, and a coherent load (`ukl_ldc_*`, ukl/memory.h) reads its words' halves so.
UKL_INLINE uint32_t ukl_atomic_load(UKL_GLOBAL uint32_t* p) { return atomic_or((volatile __global uint32_t*)p, 0u); }
// A poll of a word another group writes (Metal's comment): every atomic load here is already one.
UKL_INLINE uint32_t ukl_atomic_poll(UKL_GLOBAL uint32_t* p) { return ukl_atomic_load(p); }
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
// A fence between this thread's device accesses as other threads see them. OpenCL 1.2's `mem_fence` orders a
// work-item's own accesses alone; OpenCL C 3.0 (the device's sub-groups, `UKL_OPENCL_SUBGROUPS`) has a fence of device
// scope (2.0's too: AMD's waves). Without it, on an Arc 140V a pool's taker read a pushed block's link from before its
// push, and a block was handed out twice (`ukl/pool.h`).
// The CPU's vector code (rerail-ukl-vec, decision 0567) reads OpenCL 1.2's fence, its one fence.
#if defined(UKL_OPENCL_SUBGROUPS) && !defined(__UKL_VEC)
UKL_INLINE void ukl_fence_device() { atomic_work_item_fence(CLK_GLOBAL_MEM_FENCE, memory_order_seq_cst, memory_scope_device); }
#else
UKL_INLINE void ukl_fence_device() { mem_fence(CLK_GLOBAL_MEM_FENCE); }
#endif
UKL_INLINE uint32_t ukl_atomic_add_local(UKL_LOCAL uint32_t* p, uint32_t v) { return atomic_add((volatile __local uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_load_local(UKL_LOCAL uint32_t* p) { return *(volatile __local uint32_t*)p; }
UKL_INLINE void ukl_atomic_store_local(UKL_LOCAL uint32_t* p, uint32_t v) { atomic_xchg((volatile __local uint32_t*)p, v); }
#define UKL_CONSTANT_U32(name) __constant uint32_t name = (UKL_CONSTANT_0);

#else
#  error "the UKL library has no intrinsics for this backend (ukl/backend.h)"
#endif

// The same atomics on a coherent word (`UKL_COHERENT`): an atomic is served where the word's writes meet already.
// Overloads in C++ backends whose coherent memory is a qualifier of its own: Metal's `coherent(device)`, and CUDA's
// (and HIP's) `volatile`, which C++ does not drop on its own (a join program's worker, decision 0603, did not compile
// on NVRTC). OpenCL C takes a `volatile` pointer where a plain one is asked; the CPU's blocks are never coherent.
#if defined(UKL_METAL) || defined(UKL_CUDA)
#define __UKL_COHERENT_ATOMIC(op)                                                                                     \
    UKL_INLINE uint32_t ukl_atomic_##op(UKL_COHERENT uint32_t* p, uint32_t v) { return ukl_atomic_##op((UKL_GLOBAL uint32_t*)p, v); }
__UKL_COHERENT_ATOMIC(add)
__UKL_COHERENT_ATOMIC(sub)
__UKL_COHERENT_ATOMIC(or)
__UKL_COHERENT_ATOMIC(and)
__UKL_COHERENT_ATOMIC(exchange)
__UKL_COHERENT_ATOMIC(min)
__UKL_COHERENT_ATOMIC(max)
#undef __UKL_COHERENT_ATOMIC
UKL_INLINE uint32_t ukl_atomic_load(UKL_COHERENT uint32_t* p) { return ukl_atomic_load((UKL_GLOBAL uint32_t*)p); }
UKL_INLINE uint32_t ukl_atomic_poll(UKL_COHERENT uint32_t* p) { return ukl_atomic_poll((UKL_GLOBAL uint32_t*)p); }
UKL_INLINE void ukl_atomic_store(UKL_COHERENT uint32_t* p, uint32_t v) { ukl_atomic_store((UKL_GLOBAL uint32_t*)p, v); }
UKL_INLINE uint32_t ukl_atomic_cas(UKL_COHERENT uint32_t* p, uint32_t e, uint32_t n) { return ukl_atomic_cas((UKL_GLOBAL uint32_t*)p, e, n); }
#endif

// Written once for every backend whose language is C++ (Metal, CUDA, the CPU); a C backend (OpenCL) defines them in
// its section. A function defined for several types (by a macro over the type) is `UKL_OVERLOAD`, so its callers
// need not name the type; a check on types and layouts is `UKL_STATIC_ASSERT`.
#ifndef UKL_OVERLOAD
#define UKL_OVERLOAD
#endif
// A lane mask's set lanes; its lowest set lane (the width when none); lane `l`'s bit; the lanes below `l`; how many
// of `m`'s are below `l` (a compaction's slot).
#if UKL_SIMD_WIDTH > 32
UKL_INLINE uint32_t ukl_mask_count(ukl_mask_t m) { return ukl_popcount64(m); }
UKL_INLINE uint32_t ukl_mask_lowest(ukl_mask_t m) { return ukl_ctz64(m); }
#else
UKL_INLINE uint32_t ukl_mask_count(ukl_mask_t m) { return ukl_popcount32(m); }
UKL_INLINE uint32_t ukl_mask_lowest(ukl_mask_t m) { return ukl_ctz32(m); }
#endif
UKL_INLINE ukl_mask_t ukl_mask_lane(uint32_t l) { return (ukl_mask_t)1u << l; }
UKL_INLINE ukl_mask_t ukl_mask_below(uint32_t l) { return l >= 8u * (uint32_t)sizeof(ukl_mask_t) ? ~(ukl_mask_t)0u : ukl_mask_lane(l) - 1u; }
UKL_INLINE uint32_t ukl_mask_rank(ukl_mask_t m, uint32_t l) { return ukl_mask_count(m & ukl_mask_below(l)); }

#ifndef UKL_STATIC_ASSERT
#define UKL_STATIC_ASSERT(c, m) static_assert(c, m)
#endif
// A field's offset, as a constant for `UKL_STATIC_ASSERT`. NVRTC (its front end is EDG's) has no `offsetof` nor
// `__builtin_offsetof`, and no standard headers; EDG's own `__INTADDR__` makes the address a constant.
#ifndef UKL_OFFSETOF
#if defined(UKL_CUDA) && !defined(UKL_HIP)
#define UKL_OFFSETOF(T, f) ((unsigned long)__INTADDR__(&(((T*)0)->f)))
#else
#define UKL_OFFSETOF(T, f) __builtin_offsetof(T, f)
#endif
#endif

// A word record (decision 0603 step 5b): a struct whose words a kernel writes and other groups read one word at a
// time, coherent: offsets into an arena, counts, sizes, never a pointer. `UKL_WORD_RECORD(T, f, ...)` names every
// field of `T`, and the compiler refuses the record when one is a pointer (a pointer has no remainder) or not 32 bits,
// or when a field is left out (their sizes are not the record's). A word is read by its index (`UKL_WORD_INDEX`),
// which names a record's field only. A coherent load of a struct holding pointers crashed the M1's Metal compiler
// service at each try, and the offline check did not show it: a pointer in a record is an error here instead.
#define __UKL_WORD_SIZE(T, f) + (0u * sizeof(((UKL_PRIVATE T*)0)->f % 1u) + (sizeof(((UKL_PRIVATE T*)0)->f) == 4u ? 4u : 1u))
#define __UKL_EACH_1(m, T, a) m(T, a)
#define __UKL_EACH_2(m, T, a, ...) m(T, a) __UKL_EACH_1(m, T, __VA_ARGS__)
#define __UKL_EACH_3(m, T, a, ...) m(T, a) __UKL_EACH_2(m, T, __VA_ARGS__)
#define __UKL_EACH_4(m, T, a, ...) m(T, a) __UKL_EACH_3(m, T, __VA_ARGS__)
#define __UKL_EACH_5(m, T, a, ...) m(T, a) __UKL_EACH_4(m, T, __VA_ARGS__)
#define __UKL_EACH_6(m, T, a, ...) m(T, a) __UKL_EACH_5(m, T, __VA_ARGS__)
#define __UKL_EACH_7(m, T, a, ...) m(T, a) __UKL_EACH_6(m, T, __VA_ARGS__)
#define __UKL_EACH_8(m, T, a, ...) m(T, a) __UKL_EACH_7(m, T, __VA_ARGS__)
#define __UKL_EACH_9(m, T, a, ...) m(T, a) __UKL_EACH_8(m, T, __VA_ARGS__)
#define __UKL_EACH_10(m, T, a, ...) m(T, a) __UKL_EACH_9(m, T, __VA_ARGS__)
#define __UKL_EACH_11(m, T, a, ...) m(T, a) __UKL_EACH_10(m, T, __VA_ARGS__)
#define __UKL_EACH_12(m, T, a, ...) m(T, a) __UKL_EACH_11(m, T, __VA_ARGS__)
#define __UKL_EACH_N(_1, _2, _3, _4, _5, _6, _7, _8, _9, _10, _11, _12, N, ...) N
#define __UKL_EACH_CAT(a, b) a##b
#define __UKL_EACH_AT(n) __UKL_EACH_CAT(__UKL_EACH_, n)
#define __UKL_EACH(m, T, ...) __UKL_EACH_AT(__UKL_EACH_N(__VA_ARGS__, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0))(m, T, __VA_ARGS__)
#define UKL_WORD_RECORD(T, ...)                                                                                        \
    enum { T##_is_a_word_record = 1 };                                                                                 \
    UKL_STATIC_ASSERT(sizeof(T) == 0u __UKL_EACH(__UKL_WORD_SIZE, T, __VA_ARGS__), #T ": a word record's fields are its 32-bit words, all named")
#define UKL_WORD_INDEX(T, f) ((uint32_t)T##_is_a_word_record * (uint32_t)(UKL_OFFSETOF(T, f) / 4u))

// A program's specialization constant (decision 0440 step B): `UKL_CONSTANT_U32(name)` declares `name`, a uint32_t
// fixed when the program is made, so the compiler folds it (the label passes' tape format). Metal's function constant
// 0; elsewhere the define `UKL_CONSTANT_0` each value's program is compiled with (`rerail_gpu::kernels::program_with`).
#ifndef UKL_CONSTANT_U32
#define UKL_CONSTANT_U32(name) static const uint32_t name = (UKL_CONSTANT_0);
#endif

// A poll of another group's progress: nothing on a GPU, where the groups run at once.
#ifndef ukl_poll_yield
#define ukl_poll_yield() ((void)0)
#endif

// The device's clock in nanoseconds, for a measurement's trace: CUDA's global timer. 0 where the backend gives a kernel
// no clock (Metal, OpenCL C).
#ifndef ukl_clock_ns
#if defined(UKL_CUDA)
UKL_INLINE uint64_t ukl_clock_ns(void) { uint64_t t; asm volatile("mov.u64 %0, %%globaltimer;" : "=l"(t)); return t; }
#else
#define ukl_clock_ns() ((uint64_t)0)
#endif
#endif
