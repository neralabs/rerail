// ukl/chain.h: a chain of dispatches, each sized by the one before on the device (decision 0674).
//
// A chain is one kernel dispatched once a phase, in order, in one submission (`rerail_accel::gpu::join::Pass::
// chain_at`): dispatch `d`'s threadgroups are the three words at the host's offset for it, read when the dispatch
// starts (Metal's indirect dispatch; the CPU reads them when the launch runs; where a GPU has none, the shim's
// re-dispatcher runs the count's groups on a fixed number launched). Every group dispatched has its work: no group
// takes from a queue, and none waits for another. The order between dispatches is the device's.
//
// Who sizes the next dispatch is the kernel's: one group of a dispatch, its finisher (the last of its groups to
// finish, `ukl_chain_finish`), writes the next dispatch's counts (`ukl_chain_set`) and whatever the next reads to know
// what it is. A count of 0 runs no group, so no finisher: the chain ends there, the dispatches after it left at the
// zeroes the host wrote. The finisher is the last of its dispatch's groups, so every group has read what it reads at
// its start before the finisher writes the next's.

#pragma once
#include <ukl/backend.h>

// One thread of a group, after the group's writes and a device barrier: counts the group finished (`done`) and says
// whether it is the last of `groups` groups. Its writes are seen by the finisher's reads after (the fence before the
// count, the finisher's fence after it).
UKL_INLINE bool ukl_chain_finish(UKL_GLOBAL uint32_t* done, uint32_t groups)
{
    ukl_fence_device();
    bool last = ukl_atomic_add(done, 1u) + 1u == groups;
    if (last) ukl_fence_device();
    return last;
}

// The finisher's: the dispatch whose counts are `counts` runs `groups` groups along x (0: none, the chain ends).
UKL_INLINE void ukl_chain_set(UKL_GLOBAL uint32_t* counts, uint32_t groups)
{
    ukl_atomic_store(&counts[0], groups);
    ukl_atomic_store(&counts[1], 1u);
    ukl_atomic_store(&counts[2], 1u);
}
