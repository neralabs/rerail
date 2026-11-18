/** ukl_cuda_shim.h                                     -*- C++ -*-
    Copyright (c) Jeremy Barnes.  All rights reserved.
    This file is part of MLDB.

    UKL's shim for CUDA: a UKL kernel as CUDA C++ for NVRTC.

    Adapted in Rerail (decision 0400 step 2) from upstream's draft, following the Metal shim beside it:
    - no host code: the C++ registration (`registerCudaKernel`) is gone; the Rust host finds a kernel by its
      unmangled name (`extern "C"`) and binds its arguments in its rows' order;
    - no system headers (NVRTC has none): the fixed-width types and `INFINITY` are defined here;
    - a kernel's threadgroup memory (`RWLOCAL` rows) is CUDA's dynamic shared memory: the kernel takes each
      such row as a byte offset into it, which the host chooses (aligned to 16) with the launch's total size;
    - positions: `GID0` is `blockIdx.x * blockDim.x + threadIdx.x` (upstream multiplied by `gridDim`);
    - SIMD-group operations are warp intrinsics over the full warp (`*_sync(0xffffffff, ...)`): every lane of
      the warp must call them (they are convergent), as Metal's simd_* functions require of their SIMD group.
*/

#pragma once
#include "ukl_auto_macros.h"
#include "ukl_macros.h"

#define UKL_SIMD_WIDTH 32


// The thread's lane in its warp: its linear index in the block, modulo the warp.
#define __UKL_CUDA_LANE ((threadIdx.x + blockDim.x * (threadIdx.y + blockDim.y * threadIdx.z)) & 31u)

// Address spaces. CUDA has one generic address space for pointers; `__shared__` and `__constant__` qualify
// variables, never pointers, so only the variable form of `__constant` keeps its qualifier.
#define __constant __constant__
#define __kernel
#define __global
#define __local

#define ukl_assert(x)
#define ukl_assert_equal(x, y)
#define ukl_assert_not_equal(x, y)
#define ukl_assert_less(x, y)
#define ukl_assert_less_equal(x, y)
#define ukl_assert_greater(x, y)
#define ukl_assert_greater_equal(x, y)

#define ROBUFFER(Type) const Type *
#define RWBUFFER(Type) Type *
#define WOBUFFER(Type) Type *
#define RWLOCAL(Type) Type *
#define CONSTBUFFER(Type) const Type *

#define CAST_RWBUFFER(Buf, Type) reinterpret_cast<Type *>(Buf)
#define CAST_ROBUFFER(Buf, Type) reinterpret_cast<const Type *>(Buf)
#define CAST_RWLOCAL(Buf, Type) reinterpret_cast<Type *>(Buf)
#define CAST_ROLOCAL(Buf, Type) reinterpret_cast<const Type *>(Buf)

#define UKL_LOCAL_PTR(Type) Type *
#define UKL_LOCAL_PTR_CAST(Type) (Type *)
#define UKL_GLOBAL_PTR(Type) Type *
#define UKL_GLOBAL_PTR_CAST(Type) (Type *)

#define ukl_simdgroup_barrier() __syncwarp()
#define ukl_threadgroup_barrier() __syncthreads()
#define printf(...)

// Functions: every one is a device function. One with a barrier inside (SYNC_*) is an ordinary call here; the
// CPU shim runs it as a coroutine.
#define UKL_FUNCTION(Version, Return, Name) __device__ Return Name
#define SYNC_FUNCTION(Version, Return, Name) __device__ Return Name
#define SYNC_RETURN(...) return __VA_ARGS__
#define SYNC_CALL(Fn, ...) Fn(__VA_ARGS__)
#define SYNC_CALL_0(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_1(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_2(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_3(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_4(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_5(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_6(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_7(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_8(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_9(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_10(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_11(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_12(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_13(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_14(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_15(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_16(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_17(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_18(...) SYNC_CALL(__VA_ARGS__)
#define SYNC_CALL_19(...) SYNC_CALL(__VA_ARGS__)

typedef unsigned int uint32_t;
typedef int int32_t;
typedef unsigned short uint16_t;
typedef short int16_t;
typedef unsigned char uint8_t;
typedef signed char int8_t;
typedef unsigned long long uint64_t;
typedef long long int64_t;
typedef unsigned int atomic_uint;
typedef int atomic_int;

#ifndef INFINITY
#  define INFINITY __int_as_float(0x7f800000)
#endif
#ifndef NAN
#  define NAN __int_as_float(0x7fffffff)
#endif

// Atomics, relaxed, as Metal's: a plain load or store through `volatile`, and the read-modify-writes.
template<typename T> static __device__ __forceinline__ T ukl_atom_load(T * p) { return *(volatile T *)p; }
template<typename T, typename V> static __device__ __forceinline__ void ukl_atom_store(T * p, V v) { *(volatile T *)p = (T)v; }
#define atom_load(x) ukl_atom_load(x)
#define atom_store(x, y) ukl_atom_store(x, y)
#define atom_add(x, y) atomicAdd(x, y)
#define atom_sub(x, y) atomicSub(x, y)
#define atom_or(x, y) atomicOr(x, y)
#define atom_inc(x) atomicAdd(x, 1)
#define atom_dec(x) atomicSub(x, 1)
#define atom_load_local atom_load
#define atom_store_local atom_store
#define atom_add_local atom_add
#define atom_sub_local atom_sub
#define atom_or_local atom_or
#define atom_inc_local atom_inc
#define atom_dec_local atom_dec

#define get_global_id(n) ((n) == 0 ? blockIdx.x * blockDim.x + threadIdx.x : (n) == 1 ? blockIdx.y * blockDim.y + threadIdx.y : blockIdx.z * blockDim.z + threadIdx.z)
#define get_global_size(n) ((n) == 0 ? gridDim.x * blockDim.x : (n) == 1 ? gridDim.y * blockDim.y : gridDim.z * blockDim.z)
#define get_local_id(n) ((n) == 0 ? threadIdx.x : (n) == 1 ? threadIdx.y : threadIdx.z)
#define get_local_size(n) ((n) == 0 ? blockDim.x : (n) == 1 ? blockDim.y : blockDim.z)

#define KERNEL_RETURN() return

#define ukl_vec_el(expr, el) (expr).el

#define ukl_popcount __popc
#define ukl_clz __clz

// Metal's `extract_bits`: `bits` bits of `val` from `offset` up.
static __device__ __forceinline__ uint32_t extract_bits(uint32_t val, uint16_t offset, uint32_t bits)
{
    return (val >> offset) & (bits >= 32 ? 0xffffffffu : ((1u << bits) - 1u));
}

static __device__ __forceinline__ int32_t reinterpretFloatAsInt(float val)
{
    return __float_as_int(val);
}

static __device__ __forceinline__ float reinterpretIntAsFloat(int32_t val)
{
    return __int_as_float(val);
}

// SIMD-group operations. `simd_lane` and `tmp` are the signature Metal's and the CPU's share; CUDA needs neither.
static __device__ __forceinline__ uint32_t ukl_simdgroup_ballot(bool val, uint16_t simd_lane, atomic_uint * tmp)
{
    return __ballot_sync(0xffffffffu, val);
}

static __device__ __forceinline__ uint32_t ukl_simdgroup_sum(uint32_t val, uint16_t simd_lane, atomic_uint * tmp)
{
#if __CUDA_ARCH__ >= 800
    return __reduce_add_sync(0xffffffffu, val);
#else
    for (int d = 16; d > 0; d >>= 1)
        val += __shfl_xor_sync(0xffffffffu, val, d);
    return val;
#endif
}

static __device__ __forceinline__ uint32_t ukl_prefix_exclusive_sum_bitmask(uint32_t bits, uint16_t n)
{
    return __popc(bits & ((1u << n) - 1u));
}

static __device__ __forceinline__ uint32_t ukl_simdgroup_prefix_exclusive_sum_bools(bool val, uint16_t simd_lane, atomic_uint * tmp)
{
    return ukl_prefix_exclusive_sum_bitmask(__ballot_sync(0xffffffffu, val), simd_lane);
}

static __device__ __forceinline__ uint32_t ukl_simdgroup_broadcast_first(uint32_t val, uint16_t simd_lane, atomic_uint * tmp)
{
    return __shfl_sync(0xffffffffu, val, __ffs(__activemask()) - 1);
}

// A kernel's parameters: what its body sees (inner), what the launch passes (outer), and how the one becomes the
// other. Positions and sizes are no parameters of the launch; they come from CUDA's built-in variables.
#define GEN_INNER_PARAMETER_RWBUFFER(Type) Type *
#define GEN_INNER_PARAMETER_ROBUFFER(Type) const Type *
#define GEN_INNER_PARAMETER_LITERAL(Type) Type
#define GEN_INNER_PARAMETER_RWLOCAL(Type) Type *
#define GEN_INNER_PARAMETER_TUNEABLE(Type) Type
#define GEN_INNER_PARAMETER_GID0(Type) const Type
#define GEN_INNER_PARAMETER_GID1(Type) const Type
#define GEN_INNER_PARAMETER_GID2(Type) const Type
#define GEN_INNER_PARAMETER_GSZ0(Type) const Type
#define GEN_INNER_PARAMETER_GSZ1(Type) const Type
#define GEN_INNER_PARAMETER_GSZ2(Type) const Type
#define GEN_INNER_PARAMETER_LID0(Type) const Type
#define GEN_INNER_PARAMETER_LID1(Type) const Type
#define GEN_INNER_PARAMETER_LID2(Type) const Type
#define GEN_INNER_PARAMETER_LSZ0(Type) const Type
#define GEN_INNER_PARAMETER_LSZ1(Type) const Type
#define GEN_INNER_PARAMETER_LSZ2(Type) const Type

#define GEN_INNER_PARAMETER_ARGS(Type) const Type &
#define GEN_INNER_PARAMETER_CONSTBUFFER(Type) const Type *
#define GEN_INNER_PARAMETER_TGID0(Type) const Type
#define GEN_INNER_PARAMETER_TGID1(Type) const Type
#define GEN_INNER_PARAMETER_TGID2(Type) const Type
#define GEN_INNER_PARAMETER_LANE(Type) const Type
#define GEN_INNER_PARAMETER_NLANES(Type) const Type

#define GEN_INNER_PARAMETER_SHARED(Type) Type *

#define GEN_INNER_PARAMETER2(Access, Type, Name, Bounds) CALL2_A(GEN_INNER_PARAMETER_, Access, Type) Name

#define PASS_INNER_ARG_RWBUFFER(Type, Name) Name
#define PASS_INNER_ARG_ROBUFFER(Type, Name) Name
#define PASS_INNER_ARG_LITERAL(Type, Name) Name
#define PASS_INNER_ARG_RWLOCAL(Type, Name) reinterpret_cast<Type *>(__ukl_shared + Name)
#define PASS_INNER_ARG_TUNEABLE(Type, Name) Name
#define PASS_INNER_ARG_GID0(Type, Name) (blockIdx.x * blockDim.x + threadIdx.x)
#define PASS_INNER_ARG_GID1(Type, Name) (blockIdx.y * blockDim.y + threadIdx.y)
#define PASS_INNER_ARG_GID2(Type, Name) (blockIdx.z * blockDim.z + threadIdx.z)
#define PASS_INNER_ARG_GSZ0(Type, Name) (blockDim.x * gridDim.x)
#define PASS_INNER_ARG_GSZ1(Type, Name) (blockDim.y * gridDim.y)
#define PASS_INNER_ARG_GSZ2(Type, Name) (blockDim.z * gridDim.z)
#define PASS_INNER_ARG_LID0(Type, Name) (threadIdx.x)
#define PASS_INNER_ARG_LID1(Type, Name) (threadIdx.y)
#define PASS_INNER_ARG_LID2(Type, Name) (threadIdx.z)
#define PASS_INNER_ARG_LSZ0(Type, Name) (blockDim.x)
#define PASS_INNER_ARG_LSZ1(Type, Name) (blockDim.y)
#define PASS_INNER_ARG_LSZ2(Type, Name) (blockDim.z)

#define PASS_INNER_ARG_ARGS(Type, Name) Name
#define PASS_INNER_ARG_CONSTBUFFER(Type, Name) Name
#define PASS_INNER_ARG_TGID0(Type, Name) (blockIdx.x)
#define PASS_INNER_ARG_TGID1(Type, Name) (blockIdx.y)
#define PASS_INNER_ARG_TGID2(Type, Name) (blockIdx.z)
#define PASS_INNER_ARG_LANE(Type, Name) __UKL_CUDA_LANE
#define PASS_INNER_ARG_NLANES(Type, Name) 32u

#define PASS_INNER_ARG_SHARED(Type, Name) Name

#define PASS_INNER_ARG2(Access, Type, Name, Bounds) CALL2_A(PASS_INNER_ARG_, Access, Type, Name)

#define GEN_OUTER_PARAMETER_RWBUFFER(Type, Name) Type * Name
#define GEN_OUTER_PARAMETER_ROBUFFER(Type, Name) const Type * Name
#define GEN_OUTER_PARAMETER_LITERAL(Type, Name) Type Name
#define GEN_OUTER_PARAMETER_RWLOCAL(Type, Name) uint32_t Name
#define GEN_OUTER_PARAMETER_TUNEABLE(Type, Name) Type Name
#define GEN_OUTER_PARAMETER_GID0(Type, Name)
#define GEN_OUTER_PARAMETER_GID1(Type, Name)
#define GEN_OUTER_PARAMETER_GID2(Type, Name)
#define GEN_OUTER_PARAMETER_GSZ0(Type, Name)
#define GEN_OUTER_PARAMETER_GSZ1(Type, Name)
#define GEN_OUTER_PARAMETER_GSZ2(Type, Name)
#define GEN_OUTER_PARAMETER_LID0(Type, Name)
#define GEN_OUTER_PARAMETER_LID1(Type, Name)
#define GEN_OUTER_PARAMETER_LID2(Type, Name)
#define GEN_OUTER_PARAMETER_LSZ0(Type, Name)
#define GEN_OUTER_PARAMETER_LSZ1(Type, Name)
#define GEN_OUTER_PARAMETER_LSZ2(Type, Name)

// A structure of arguments goes by value, in the launch's parameter space (CUDA's limit: 4 KB; 32 KB from 12.1).
#define GEN_OUTER_PARAMETER_ARGS(Type, Name) const Type Name
#define GEN_OUTER_PARAMETER_CONSTBUFFER(Type, Name) const Type * __restrict__ Name
#define GEN_OUTER_PARAMETER_TGID0(Type, Name)
#define GEN_OUTER_PARAMETER_TGID1(Type, Name)
#define GEN_OUTER_PARAMETER_TGID2(Type, Name)
#define GEN_OUTER_PARAMETER_LANE(Type, Name)
#define GEN_OUTER_PARAMETER_NLANES(Type, Name)

#define GEN_OUTER_PARAMETER_SHARED(Type, Name)
#define GEN_SHARED_DECL_SHARED(Type, Name, Bounds) __shared__ Type Name[Bounds];

#define GEN_OUTER_PARAMETER2(Access, Type, Name, Bounds) CALL2_A(GEN_OUTER_PARAMETER_, Access, Type, Name)

#define DEFINE_KERNEL2(Library, Name, ...) \
    __device__ __forceinline__ void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) ); \
    extern "C" __global__ void Name ( __UKL_FOREACH_QUAD(GEN_OUTER_PARAMETER2, __VA_ARGS__) ) \
    { \
        extern __shared__ __align__(16) unsigned char __ukl_shared[]; \
        __UKL_FOREACH_QUAD_NOSEP(GEN_SHARED_DECL2, __VA_ARGS__) \
            Name ## _inner ( __UKL_FOREACH_QUAD(PASS_INNER_ARG2, __VA_ARGS__)); \
    } \
    __device__ __forceinline__ void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) )
