/** ukl_vec_shim.h                                     -*- C -*-
    Copyright (c) Jeremy Barnes.  All rights reserved.
    Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE
    This file is part of MLDB.

    UKL's shim for the CPU's vector code (`rerail-ukl-vec`, decision 0567): a UKL kernel as plain C whose every
    operation of the machine is one intrinsic, `__ukl_*`, which the vectorizer knows by name. Written in Rerail, for
    the vectorizer alone: no other compiler reads it.
    - C as C's grammar reads it: the fixed-width integers are its types, a pointer has no address space, and what a
      backend's keyword would say is an attribute (a kernel `ukl_kernel`, a threadgroup's storage `ukl_local`);
    - every position, barrier, SIMD-group operation and atomic is a call of an intrinsic (`__ukl_local_id(d)`,
      `__ukl_barrier()`, `__ukl_sg_sum(v)`, `__ukl_atomic_load(p)`, ...): one name for one operation, with no flags
      or spellings of another backend to read through (`ukl/backend.h`'s `UKL_VEC` section);
    - the SIMD group is the hardware's: `UKL_SIMD_WIDTH` lanes, the vectorizer's;
    - reflection: the vectorizer has every layout from the program's own types, so a kernel's rows and a structure's
      fields are only named, in constants (`Name__ukl_params`, `Type__ukl_fields`).
*/

#pragma once
#include "ukl_auto_macros.h"
#include "ukl_macros.h"

#ifndef UKL_SIMD_WIDTH
#define UKL_SIMD_WIDTH 32u
#endif

// A word the atomics take: a plain integer.
typedef uint32_t atomic_uint;
typedef int32_t atomic_int;

// A threadgroup's own storage, declared in a kernel's entry (a `SHARED` row).
#define __UKL_LOCAL_STORAGE __attribute__((ukl_local))
#define __UKL_KERNEL __attribute__((ukl_kernel))

#define __UKL_PARAMS
#define __UKL_ARGS

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

#define CAST_RWBUFFER(Buf, Type) ((Type *)(Buf))
#define CAST_ROBUFFER(Buf, Type) ((const Type *)(Buf))
#define CAST_RWLOCAL(Buf, Type) ((Type *)(Buf))
#define CAST_ROLOCAL(Buf, Type) ((const Type *)(Buf))

#define UKL_LOCAL_PTR(Type) Type *
#define UKL_LOCAL_PTR_CAST(Type) (Type *)
#define UKL_GLOBAL_PTR(Type) Type *
#define UKL_GLOBAL_PTR_CAST(Type) (Type *)

#define ukl_threadgroup_barrier() __ukl_barrier()

#define KERNEL_RETURN() return

// MLDB's kernel functions: every one a plain function (one with a barrier inside, SYNC_*, too).
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

// MLDB's atomics, relaxed, on global or local words alike.
#define atom_load(x) __ukl_atomic_load(x)
#define atom_store(x, y) __ukl_atomic_store((x), (y))
#define atom_add(x, y) __ukl_atomic_add((x), (y))
#define atom_sub(x, y) __ukl_atomic_sub((x), (y))
#define atom_or(x, y) __ukl_atomic_or((x), (y))
#define atom_inc(x) __ukl_atomic_inc(x)
#define atom_dec(x) __ukl_atomic_dec(x)
#define atom_load_local atom_load
#define atom_store_local atom_store
#define atom_add_local atom_add
#define atom_sub_local atom_sub
#define atom_or_local atom_or
#define atom_inc_local atom_inc
#define atom_dec_local atom_dec

#define ukl_vec_el(expr, el) (expr).el
#define ukl_popcount __ukl_popcount
#define ukl_clz __ukl_clz

// Metal's `extract_bits`: `bits` bits of `val` from `offset` up.
static inline uint32_t extract_bits(uint32_t val, uint16_t offset, uint32_t bits)
{
    return (val >> offset) & (bits >= 32u ? 0xffffffffu : ((1u << bits) - 1u));
}
static inline int32_t reinterpretFloatAsInt(float val) { return __ukl_as_int32_t(val); }
static inline float reinterpretIntAsFloat(int32_t val) { return __ukl_as_float(val); }

// MLDB's SIMD-group operations (`tmp`, upstream's threadgroup word, is not needed: the group is the hardware's).
#define ukl_simdgroup_barrier() __ukl_sg_barrier()
static inline uint32_t ukl_simdgroup_ballot(bool val, uint16_t simd_lane, atomic_uint * tmp) { return __ukl_sg_ballot(val); }
static inline uint32_t ukl_simdgroup_sum(uint32_t val, uint16_t simd_lane, atomic_uint * tmp) { return __ukl_sg_sum(val); }
static inline uint32_t ukl_prefix_exclusive_sum_bitmask(uint32_t bits, uint16_t n) { return __ukl_popcount(bits & ((1u << n) - 1u)); }
static inline uint32_t ukl_simdgroup_prefix_exclusive_sum_bools(bool val, uint16_t simd_lane, atomic_uint * tmp)
{
    return ukl_prefix_exclusive_sum_bitmask(__ukl_sg_ballot(val), simd_lane);
}
static inline uint32_t ukl_simdgroup_broadcast_first(uint32_t val, uint16_t simd_lane, atomic_uint * tmp) { return __ukl_sg_shuffle(val, 0u); }

// The thread's index in its threadgroup, the threads in order along x, then y, then z.
static inline uint32_t __ukl_vec_lid(void)
{
    return __ukl_local_id(0) + __ukl_local_size(0) * (__ukl_local_id(1) + __ukl_local_size(1) * __ukl_local_id(2));
}

// A kernel's parameters: what its body sees (inner), what the launch passes (outer), and how the one becomes the
// other, by the intrinsics of positions and sizes.
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
#define GEN_INNER_PARAMETER_ARGS(Type) const Type
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
#define PASS_INNER_ARG_RWLOCAL(Type, Name) Name
#define PASS_INNER_ARG_TUNEABLE(Type, Name) Name
#define PASS_INNER_ARG_GID0(Type, Name) ((Type)__ukl_global_id(0))
#define PASS_INNER_ARG_GID1(Type, Name) ((Type)__ukl_global_id(1))
#define PASS_INNER_ARG_GID2(Type, Name) ((Type)__ukl_global_id(2))
// The grid's size: the launch's, or under an indirect launch the groups its counts give (`__ukl_groups`).
#define PASS_INNER_ARG_GSZ0(Type, Name) ((Type)(__ukl_groups ? __ukl_groups[__ukl_groups_at] * __ukl_local_size(0) : __ukl_global_size(0)))
#define PASS_INNER_ARG_GSZ1(Type, Name) ((Type)(__ukl_groups ? __ukl_groups[__ukl_groups_at + 1u] * __ukl_local_size(1) : __ukl_global_size(1)))
#define PASS_INNER_ARG_GSZ2(Type, Name) ((Type)(__ukl_groups ? __ukl_groups[__ukl_groups_at + 2u] * __ukl_local_size(2) : __ukl_global_size(2)))
#define PASS_INNER_ARG_LID0(Type, Name) ((Type)__ukl_local_id(0))
#define PASS_INNER_ARG_LID1(Type, Name) ((Type)__ukl_local_id(1))
#define PASS_INNER_ARG_LID2(Type, Name) ((Type)__ukl_local_id(2))
#define PASS_INNER_ARG_LSZ0(Type, Name) ((Type)__ukl_local_size(0))
#define PASS_INNER_ARG_LSZ1(Type, Name) ((Type)__ukl_local_size(1))
#define PASS_INNER_ARG_LSZ2(Type, Name) ((Type)__ukl_local_size(2))
#define PASS_INNER_ARG_ARGS(Type, Name) Name
#define PASS_INNER_ARG_CONSTBUFFER(Type, Name) Name
#define PASS_INNER_ARG_TGID0(Type, Name) ((Type)__ukl_group_id(0))
#define PASS_INNER_ARG_TGID1(Type, Name) ((Type)__ukl_group_id(1))
#define PASS_INNER_ARG_TGID2(Type, Name) ((Type)__ukl_group_id(2))
#define PASS_INNER_ARG_LANE(Type, Name) ((Type)__ukl_lane())
#define PASS_INNER_ARG_NLANES(Type, Name) ((Type)UKL_SIMD_WIDTH)
#define PASS_INNER_ARG_SHARED(Type, Name) Name

#define PASS_INNER_ARG2(Access, Type, Name, Bounds) CALL2_A(PASS_INNER_ARG_, Access, Type, Name)

#define GEN_OUTER_PARAMETER_RWBUFFER(Type, Name) Type * Name
#define GEN_OUTER_PARAMETER_ROBUFFER(Type, Name) const Type * Name
#define GEN_OUTER_PARAMETER_LITERAL(Type, Name) Type Name
#define GEN_OUTER_PARAMETER_RWLOCAL(Type, Name) Type * Name
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
#define GEN_OUTER_PARAMETER_CONSTBUFFER(Type, Name) const Type * Name
#define GEN_OUTER_PARAMETER_TGID0(Type, Name)
#define GEN_OUTER_PARAMETER_TGID1(Type, Name)
#define GEN_OUTER_PARAMETER_TGID2(Type, Name)
#define GEN_OUTER_PARAMETER_LANE(Type, Name)
#define GEN_OUTER_PARAMETER_NLANES(Type, Name)
#define GEN_OUTER_PARAMETER_SHARED(Type, Name)
// An indirect launch's group counts (null for a direct launch) and the word they start at, parameters after the
// rows' that no row names (decision 0440 step B): the groups past the counts return first.
#define GEN_OUTER_PARAMETER_GROUPS(Type, Name) const Type * Name, uint32_t Name##_at
#define GEN_SHARED_DECL_SHARED(Type, Name, Bounds) __UKL_LOCAL_STORAGE Type Name[Bounds];

#define GEN_OUTER_PARAMETER2(Access, Type, Name, Bounds) CALL2_A(GEN_OUTER_PARAMETER_, Access, Type, Name)

// A structure's fields and a kernel's rows, named in constants the vectorizer reads.
#define UKL_REFLECT_STRUCT(Type, ...) static const char Type ## __ukl_fields[] = #__VA_ARGS__;
#define __UKL_KERNEL_INFO(Name, ...) static const char Name ## __ukl_params[] = #__VA_ARGS__;

// A kernel: its rows named, its body (inner), and its entry, which declares the threadgroup memory.
#define DEFINE_KERNEL2(Library, Name, ...) \
    __UKL_KERNEL_INFO(Name, __VA_ARGS__) \
    static void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) ); \
    __UKL_KERNEL void Name ( __UKL_FOREACH_QUAD(GEN_OUTER_PARAMETER2, __VA_ARGS__, GROUPS, uint32_t, __ukl_groups, ) ) \
    { \
        if (__ukl_groups && (__ukl_group_id(0) >= __ukl_groups[__ukl_groups_at] || __ukl_group_id(1) >= __ukl_groups[__ukl_groups_at + 1u] \
                             || __ukl_group_id(2) >= __ukl_groups[__ukl_groups_at + 2u])) \
            return; \
        __UKL_FOREACH_QUAD_NOSEP(GEN_SHARED_DECL2, __VA_ARGS__) \
        Name ## _inner ( __UKL_FOREACH_QUAD(PASS_INNER_ARG2, __VA_ARGS__) ); \
    } \
    static void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) )
