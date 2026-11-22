/** ukl_opencl_shim.h                                     -*- C -*-
    Copyright (c) Jeremy Barnes.  All rights reserved.
    Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE
    This file is part of MLDB.

    UKL's shim for OpenCL: a UKL kernel as OpenCL C 1.2, compiled from source by the device's driver.

    Written in Rerail (decision 0440 §3.5), after the CUDA shim beside it and upstream's OpenCL prelude
    (`opencl-random-forests`):
    - C, not C++: OpenCL C 1.2 has no templates, references or overloading but clang's `overloadable` (every OpenCL
      compiler we target is clang-based). The library is written in the subset both accept (0440 §3.5);
    - no host code: the Rust host finds a kernel by name and binds its arguments in its rows' order;
    - a kernel's threadgroup memory: an `RWLOCAL` row is a `__local` argument the host sizes
      (`clSetKernelArg(k, i, bytes, NULL)`); a `SHARED` row is declared in the kernel (OpenCL C 1.2 declares
      `__local` memory only there);
    - the SIMD group (`UKL_SIMD_WIDTH`, which the host defines): a device without subgroups emulates them through
      threadgroup memory, a word a thread (`__ukl_sg`). The kernel's entry declares it, and every function that
      reaches a SIMD-group operation takes it (`__UKL_PARAMS`) and passes it on (`__UKL_ARGS`). OpenCL 1.2's only
      wait is the workgroup's barrier, so under emulation every SIMD-group operation must be reached by every
      live thread of the threadgroup (the CPU backend checks this rule: `RERAIL_CPU_OPENCL_RULE=1`). At width 1
      the group is the thread: no memory, no barrier. A device with sub-groups (the host defines
      `UKL_OPENCL_SUBGROUPS`, and builds as OpenCL C 3.0) has them natively: each kernel requires its native width,
      `UKL_SIMD_WIDTH` lanes (`intel_reqd_sub_group_size`), and the operations are its `sub_group_*` functions (`ukl/backend.h`).
    - reflection: OpenCL cannot read a program's constants back, so each kernel has a one-thread kernel beside it
      (`Name__ukl_info`) that writes its rows' layout and their text, and each reflected structure one that writes
      its fields' layout and names (`Type__ukl_layout`). The host runs them once per program.
*/

#pragma once
#include "ukl_auto_macros.h"
#include "ukl_macros.h"

#ifndef UKL_SIMD_WIDTH
#define UKL_SIMD_WIDTH 32u
#endif
#ifndef UKL_TG_MAX
#define UKL_TG_MAX 1024u
#endif

// The fixed-width integers by OpenCL's own. The CPU's vector path (`rerail-ukl-vec`, decision 0567) reads C, whose
// grammar has them as types already.
#if !defined(__UKL_VEC)
typedef uint uint32_t;
typedef int int32_t;
typedef ushort uint16_t;
typedef short int16_t;
typedef uchar uint8_t;
typedef char int8_t;
typedef ulong uint64_t;
typedef long int64_t;
#endif
// A word the atomics take: a plain integer, as OpenCL 1.2's atomic functions (`atomic_or`, ...) do. OpenCL C 2.0
// and later have types of these names (C11's `_Atomic`, which 1.2's functions do not take, and which a private
// variable cannot be), so there the names are the integers' by macro; 1.2's functions remain in 3.0. The source is
// preprocessed on the host, before the driver defines `__OPENCL_C_VERSION__`: the host builds as 3.0 exactly where it
// defines `UKL_OPENCL_SUBGROUPS` (`compute_kernel_opencl`).
#if defined(UKL_OPENCL_SUBGROUPS)
#define atomic_uint uint
#define atomic_int int
#else
typedef uint atomic_uint;
typedef int atomic_int;
#endif

// The thread's index in its threadgroup, the threads in order along x, then y, then z; and its lane.
static inline uint __ukl_cl_lid(void)
{
    return (uint)(get_local_id(0) + get_local_size(0) * (get_local_id(1) + get_local_size(1) * get_local_id(2)));
}
#if defined(UKL_OPENCL_SUBGROUPS)
#define __UKL_CL_LANE ((uint)get_sub_group_local_id())
// Every kernel's sub-groups are its SIMD groups: `UKL_SIMD_WIDTH` lanes (the device's native width), required.
#define __UKL_SG_ATTR __attribute__((intel_reqd_sub_group_size(UKL_SIMD_WIDTH)))
#else
#define __UKL_CL_LANE (__ukl_cl_lid() & (UKL_SIMD_WIDTH - 1u))
#define __UKL_SG_ATTR
#endif

// A threadgroup's own storage, declared in a kernel's entry (a `SHARED` row, the SIMD group's scratch): `__local`.
// The CPU's vector path (decision 0567), whose pointers have no address space, marks the declaration instead.
#ifndef __UKL_LOCAL_STORAGE
#define __UKL_LOCAL_STORAGE __local
#endif

#if UKL_SIMD_WIDTH > 1 && !defined(UKL_OPENCL_SUBGROUPS)
#define UKL_OPENCL_EMULATED_SG 1
#define __UKL_PARAMS , __local uint* __ukl_sg
#define __UKL_ARGS , __ukl_sg
#define __UKL_SG_DECL __UKL_LOCAL_STORAGE uint __ukl_sg[UKL_TG_MAX];
#else
#define __UKL_PARAMS
#define __UKL_ARGS
#define __UKL_SG_DECL
#endif

#define ukl_assert(x)
#define ukl_assert_equal(x, y)
#define ukl_assert_not_equal(x, y)
#define ukl_assert_less(x, y)
#define ukl_assert_less_equal(x, y)
#define ukl_assert_greater(x, y)
#define ukl_assert_greater_equal(x, y)

#define ROBUFFER(Type) __global const Type *
#define RWBUFFER(Type) __global Type *
#define WOBUFFER(Type) __global Type *
#define RWLOCAL(Type) __local Type *
#define CONSTBUFFER(Type) __global const Type *

#define CAST_RWBUFFER(Buf, Type) ((__global Type *)(Buf))
#define CAST_ROBUFFER(Buf, Type) ((__global const Type *)(Buf))
#define CAST_RWLOCAL(Buf, Type) ((__local Type *)(Buf))
#define CAST_ROLOCAL(Buf, Type) ((__local const Type *)(Buf))

#define UKL_LOCAL_PTR(Type) __local Type *
#define UKL_LOCAL_PTR_CAST(Type) (__local Type *)
#define UKL_GLOBAL_PTR(Type) __global Type *
#define UKL_GLOBAL_PTR_CAST(Type) (__global Type *)

#define ukl_threadgroup_barrier() barrier(CLK_LOCAL_MEM_FENCE)

#define KERNEL_RETURN() return

// MLDB's kernel functions (the forest's): every one is a plain function. One with a barrier inside (SYNC_*) is an
// ordinary call here; the CPU shim runs it as a coroutine.
#define UKL_FUNCTION(Version, Return, Name) static Return Name
#define SYNC_FUNCTION(Version, Return, Name) static Return Name
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
#define printf(...)

// MLDB's atomics, relaxed (OpenCL 1.2's are), on global or local words alike (the builtins are overloaded).
#define atom_load(x) atomic_add((x), 0)
#define atom_store(x, y) ((void)atomic_xchg((x), (y)))
#define atom_add(x, y) atomic_add((x), (y))
#define atom_sub(x, y) atomic_sub((x), (y))
#define atom_or(x, y) atomic_or((x), (y))
#define atom_inc(x) atomic_inc(x)
#define atom_dec(x) atomic_dec(x)
#define atom_load_local atom_load
#define atom_store_local atom_store
#define atom_add_local atom_add
#define atom_sub_local atom_sub
#define atom_or_local atom_or
#define atom_inc_local atom_inc
#define atom_dec_local atom_dec

#define ukl_vec_el(expr, el) (expr).el
#define ukl_popcount popcount
#define ukl_clz clz

// Metal's `extract_bits`: `bits` bits of `val` from `offset` up.
static inline uint32_t extract_bits(uint32_t val, uint16_t offset, uint32_t bits)
{
    return (val >> offset) & (bits >= 32u ? 0xffffffffu : ((1u << bits) - 1u));
}
static inline int32_t reinterpretFloatAsInt(float val) { return as_int(val); }
static inline float reinterpretIntAsFloat(int32_t val) { return as_float(val); }

// MLDB's SIMD-group operations, for SIMD groups of 32 threads, through `tmp`: one threadgroup word of the caller's
// SIMD group's own (upstream's way through OpenCL). As the library's emulation, each holds threadgroup barriers:
// every live thread of the threadgroup reaches each one (the forest's kernels pass the CPU backend's check of this,
// `RERAIL_CPU_OPENCL_RULE=1`).
#define ukl_simdgroup_barrier() barrier(CLK_LOCAL_MEM_FENCE | CLK_GLOBAL_MEM_FENCE)
static inline uint32_t ukl_simdgroup_ballot(bool val, uint16_t simd_lane, __local atomic_uint * tmp)
{
    barrier(CLK_LOCAL_MEM_FENCE);
    if (simd_lane == 0)
        *tmp = 0u;
    barrier(CLK_LOCAL_MEM_FENCE);
    if (val)
        atomic_or(tmp, 1u << (simd_lane & 31u));
    barrier(CLK_LOCAL_MEM_FENCE);
    return *tmp;
}
static inline uint32_t ukl_simdgroup_sum(uint32_t val, uint16_t simd_lane, __local atomic_uint * tmp)
{
    barrier(CLK_LOCAL_MEM_FENCE);
    if (simd_lane == 0)
        *tmp = 0u;
    barrier(CLK_LOCAL_MEM_FENCE);
    atomic_add(tmp, val);
    barrier(CLK_LOCAL_MEM_FENCE);
    return *tmp;
}
static inline uint32_t ukl_prefix_exclusive_sum_bitmask(uint32_t bits, uint16_t n)
{
    return popcount(bits & ((1u << n) - 1u));
}
static inline uint32_t ukl_simdgroup_prefix_exclusive_sum_bools(bool val, uint16_t simd_lane, __local atomic_uint * tmp)
{
    return ukl_prefix_exclusive_sum_bitmask(ukl_simdgroup_ballot(val, simd_lane, tmp), simd_lane);
}
// The first lane's value: every lane of the group is live where the forest calls it.
static inline uint32_t ukl_simdgroup_broadcast_first(uint32_t val, uint16_t simd_lane, __local atomic_uint * tmp)
{
    barrier(CLK_LOCAL_MEM_FENCE);
    if (simd_lane == 0)
        *tmp = val;
    barrier(CLK_LOCAL_MEM_FENCE);
    return *tmp;
}

// A kernel's parameters: what its body sees (inner), what the launch passes (outer), and how the one becomes the
// other. Positions and sizes come from OpenCL's work-item functions.
#define GEN_INNER_PARAMETER_RWBUFFER(Type) __global Type *
#define GEN_INNER_PARAMETER_ROBUFFER(Type) __global const Type *
#define GEN_INNER_PARAMETER_LITERAL(Type) Type
#define GEN_INNER_PARAMETER_RWLOCAL(Type) __local Type *
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
#define GEN_INNER_PARAMETER_ARGS(Type) const Type
#define GEN_INNER_PARAMETER_CONSTBUFFER(Type) __global const Type *
#define GEN_INNER_PARAMETER_TGID0(Type) const Type
#define GEN_INNER_PARAMETER_TGID1(Type) const Type
#define GEN_INNER_PARAMETER_TGID2(Type) const Type
#define GEN_INNER_PARAMETER_LANE(Type) const Type
#define GEN_INNER_PARAMETER_NLANES(Type) const Type
#define GEN_INNER_PARAMETER_SHARED(Type) __local Type *

#define GEN_INNER_PARAMETER2(Access, Type, Name, Bounds) CALL2_A(GEN_INNER_PARAMETER_, Access, Type) Name

#define PASS_INNER_ARG_RWBUFFER(Type, Name) Name
#define PASS_INNER_ARG_ROBUFFER(Type, Name) Name
#define PASS_INNER_ARG_LITERAL(Type, Name) Name
#define PASS_INNER_ARG_RWLOCAL(Type, Name) Name
#define PASS_INNER_ARG_TUNEABLE(Type, Name) Name
#define PASS_INNER_ARG_GID0(Type, Name) ((Type)get_global_id(0))
#define PASS_INNER_ARG_GID1(Type, Name) ((Type)get_global_id(1))
#define PASS_INNER_ARG_GID2(Type, Name) ((Type)get_global_id(2))
// The grid's size: the launch's, or under an indirect launch the groups its counts give (`__ukl_groups`).
#define PASS_INNER_ARG_GSZ0(Type, Name) ((Type)(__ukl_groups ? __ukl_groups[__ukl_groups_at] * get_local_size(0) : get_global_size(0)))
#define PASS_INNER_ARG_GSZ1(Type, Name) ((Type)(__ukl_groups ? __ukl_groups[__ukl_groups_at + 1u] * get_local_size(1) : get_global_size(1)))
#define PASS_INNER_ARG_GSZ2(Type, Name) ((Type)(__ukl_groups ? __ukl_groups[__ukl_groups_at + 2u] * get_local_size(2) : get_global_size(2)))
#define PASS_INNER_ARG_LID0(Type, Name) ((Type)get_local_id(0))
#define PASS_INNER_ARG_LID1(Type, Name) ((Type)get_local_id(1))
#define PASS_INNER_ARG_LID2(Type, Name) ((Type)get_local_id(2))
#define PASS_INNER_ARG_LSZ0(Type, Name) ((Type)get_local_size(0))
#define PASS_INNER_ARG_LSZ1(Type, Name) ((Type)get_local_size(1))
#define PASS_INNER_ARG_LSZ2(Type, Name) ((Type)get_local_size(2))
#define PASS_INNER_ARG_ARGS(Type, Name) Name
#define PASS_INNER_ARG_CONSTBUFFER(Type, Name) Name
#define PASS_INNER_ARG_TGID0(Type, Name) ((Type)get_group_id(0))
#define PASS_INNER_ARG_TGID1(Type, Name) ((Type)get_group_id(1))
#define PASS_INNER_ARG_TGID2(Type, Name) ((Type)get_group_id(2))
#define PASS_INNER_ARG_LANE(Type, Name) ((Type)__UKL_CL_LANE)
#define PASS_INNER_ARG_NLANES(Type, Name) ((Type)UKL_SIMD_WIDTH)
#define PASS_INNER_ARG_SHARED(Type, Name) Name

#define PASS_INNER_ARG2(Access, Type, Name, Bounds) CALL2_A(PASS_INNER_ARG_, Access, Type, Name)

#define GEN_OUTER_PARAMETER_RWBUFFER(Type, Name) __global Type * Name
#define GEN_OUTER_PARAMETER_ROBUFFER(Type, Name) __global const Type * Name
#define GEN_OUTER_PARAMETER_LITERAL(Type, Name) Type Name
#define GEN_OUTER_PARAMETER_RWLOCAL(Type, Name) __local Type * Name
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
// A structure of arguments goes by value.
#define GEN_OUTER_PARAMETER_ARGS(Type, Name) const Type Name
#define GEN_OUTER_PARAMETER_CONSTBUFFER(Type, Name) __global const Type * Name
#define GEN_OUTER_PARAMETER_TGID0(Type, Name)
#define GEN_OUTER_PARAMETER_TGID1(Type, Name)
#define GEN_OUTER_PARAMETER_TGID2(Type, Name)
#define GEN_OUTER_PARAMETER_LANE(Type, Name)
#define GEN_OUTER_PARAMETER_NLANES(Type, Name)
#define GEN_OUTER_PARAMETER_SHARED(Type, Name)
// An indirect launch's group counts (null for a direct launch) and the word they start at, parameters after the
// rows' that no row names (decision 0440 step B). OpenCL 1.2 has no indirect launch: the host launches the caller's
// bound of groups, and the groups past the counts return first, so a kernel sees the grid the counts give.
#define GEN_OUTER_PARAMETER_GROUPS(Type, Name) __global const Type * Name, uint Name##_at
#define GEN_SHARED_DECL_SHARED(Type, Name, Bounds) __UKL_LOCAL_STORAGE Type Name[Bounds];

#define GEN_OUTER_PARAMETER2(Access, Type, Name, Bounds) CALL2_A(GEN_OUTER_PARAMETER_, Access, Type, Name)

// A type's kind for the host, as the other shims' `ukl_type_code`: f(loat), d(ouble), b(ool), i(nteger, signed),
// u(nsigned), s(tructure). An expression of the type, not the type: `_Generic` (C11, which clang's OpenCL C accepts).
#define __UKL_KIND(e) _Generic((e), float: 'f', bool: 'b', char: 'i', signed char: 'i', short: 'i', int: 'i', \
    long: 'i', uchar: 'u', ushort: 'u', uint: 'u', ulong: 'u', default: 's')

// A string into the host's words, after `at` of them: its length, then its bytes, eight to a word.
static inline uint __ukl_cl_text(__global ulong * out, uint at, __constant char * s)
{
    uint n = 0u;
    while (s[n])
        n++;
    out[at] = n;
    for (uint i = 0u; i < (n + 7u) / 8u; i++) {
        ulong w = 0ul;
        for (uint b = 0u; b < 8u && i * 8u + b < n; b++)
            w |= (ulong)(uchar)s[i * 8u + b] << (8u * b);
        out[at + 1u + i] = w;
    }
    return at + 1u + (n + 7u) / 8u;
}

#define GEN_ROW_LAYOUT2(Access, Type, Name, Bounds) \
    { Type __ukl_e; __ukl_out[__ukl_k++] = sizeof(Type); __ukl_out[__ukl_k++] = __alignof__(Type); \
      __ukl_out[__ukl_k++] = __UKL_KIND(__ukl_e); }

// A scalar's kind from arithmetic on its type: NVIDIA's OpenCL compiler accepts `_Generic` but always takes its
// `default` (probed on a GB10's, 580.95), so an `ARGS` field, which is a scalar (`rows::struct_type`), is not
// measured with it: bool where 2 converts to 1, floating where 1 / 2 is not 0, signed where -1 is below 0.
#define __UKL_SCALAR_KIND(e) ((__typeof__(e))2 == (__typeof__(e))1 ? 'b' \
    : (__typeof__(e))1 / (__typeof__(e))2 != (__typeof__(e))0 ? (sizeof(e) == 8 ? 'd' : 'f') \
    : (__typeof__(e))-1 < (__typeof__(e))0 ? 'i' : 'u')

// A structure passed by value (an `ARGS` row), reflected for the host: a one-thread kernel writes its size and
// alignment, each field's offset, size and kind (measured on an instance), then the fields' names as text.
// One field's three numbers, an expression (the fields are joined by commas).
#define __UKL_FIELD_LAYOUT(Type, Field) \
    (__ukl_out[__ukl_k++] = (ulong)((char *)&__ukl_v.Field - (char *)&__ukl_v), \
     __ukl_out[__ukl_k++] = sizeof(__ukl_v.Field), \
     __ukl_out[__ukl_k++] = __UKL_SCALAR_KIND(__ukl_v.Field))
#if defined(__UKL_VEC)
// The CPU's vector path (decision 0567) has every layout from its own types: a structure's fields and a kernel's rows
// are named in constants it reads.
#define UKL_REFLECT_STRUCT(Type, ...) static const char Type ## __ukl_fields[] = #__VA_ARGS__;
#else
#define UKL_REFLECT_STRUCT(Type, ...) \
    __kernel void Type ## __ukl_layout(__global ulong * __ukl_out) \
    { \
        __constant char * __ukl_names = #__VA_ARGS__; \
        Type __ukl_v; \
        uint __ukl_k = 1u; \
        __ukl_out[__ukl_k++] = sizeof(Type); \
        __ukl_out[__ukl_k++] = __alignof__(Type); \
        __UKL_FOREACH_CTX(__UKL_FIELD_LAYOUT, Type, __VA_ARGS__); \
        __ukl_out[0] = __ukl_k - 1u; \
        __ukl_cl_text(__ukl_out, __ukl_k, __ukl_names); \
    }
#endif

// A kernel: its info kernel (the rows' layout, three numbers a row after their count, then the rows' text), its
// body (inner), and its entry, which declares the threadgroup memory and the SIMD group's scratch.
//
// A program of one kernel of a source (`UKL_ONE_KERNEL`, with `__UKL_ONLY_<name>` defined as `__UKL_PROBE`): every
// other kernel of the source is only its body, a static function nothing calls, which the compiler does not emit. An
// OpenCL driver builds every kernel of a program, and Intel's spent 436 s on the 21 join stages of one source.
#if defined(__UKL_VEC)
#define __UKL_KERNEL_INFO(Name, ...) static const char Name ## __ukl_params[] = #__VA_ARGS__;
#else
#define __UKL_KERNEL_INFO(Name, ...) \
    __kernel void Name ## __ukl_info(__global ulong * __ukl_out) \
    { \
        __constant char * __ukl_text = #__VA_ARGS__; \
        uint __ukl_k = 1u; \
        __UKL_FOREACH_QUAD_NOSEP(GEN_ROW_LAYOUT2, __VA_ARGS__) \
        __ukl_out[0] = __ukl_k - 1u; \
        __ukl_cl_text(__ukl_out, __ukl_k, __ukl_text); \
    }
#endif
#if defined(UKL_ONE_KERNEL)
#define __UKL_SECOND(a, b, ...) b
#define __UKL_IS_PROBE(...) __UKL_SECOND(__VA_ARGS__, 0, 0)
#define __UKL_PROBE ~, 1
#define __UKL_CAT(a, b) __UKL_CAT_I(a, b)
#define __UKL_CAT_I(a, b) a ## b
#define __UKL_SELECTED(Name) __UKL_IS_PROBE(__UKL_CAT(__UKL_ONLY_, Name))
#define DEFINE_KERNEL2(Library, Name, ...) __UKL_CAT(__UKL_KERNEL_, __UKL_SELECTED(Name))(Library, Name, __VA_ARGS__)
#define __UKL_KERNEL_0(Library, Name, ...) \
    static void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) __UKL_PARAMS )
#define __UKL_KERNEL_1(Library, Name, ...) __UKL_DEFINE_KERNEL2(Library, Name, __VA_ARGS__)
#else
#define DEFINE_KERNEL2(Library, Name, ...) __UKL_DEFINE_KERNEL2(Library, Name, __VA_ARGS__)
#endif
#define __UKL_DEFINE_KERNEL2(Library, Name, ...) \
    __UKL_KERNEL_INFO(Name, __VA_ARGS__) \
    static void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) __UKL_PARAMS ); \
    __kernel __UKL_SG_ATTR void Name ( __UKL_FOREACH_QUAD(GEN_OUTER_PARAMETER2, __VA_ARGS__, GROUPS, uint, __ukl_groups, ) ) \
    { \
        if (__ukl_groups && (get_group_id(0) >= __ukl_groups[__ukl_groups_at] || get_group_id(1) >= __ukl_groups[__ukl_groups_at + 1u] \
                             || get_group_id(2) >= __ukl_groups[__ukl_groups_at + 2u])) \
            return; \
        __UKL_SG_DECL \
        __UKL_FOREACH_QUAD_NOSEP(GEN_SHARED_DECL2, __VA_ARGS__) \
        Name ## _inner ( __UKL_FOREACH_QUAD(PASS_INNER_ARG2, __VA_ARGS__) __UKL_ARGS ); \
    } \
    static void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) __UKL_PARAMS )
