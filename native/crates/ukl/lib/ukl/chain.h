// ukl/chain.h: a chain of dispatches, each sized by the one before on the device (decision 0674).
//
// A chain is one kernel dispatched once a phase, in order, in one submission (`rerail_accel::gpu::join::Pass::
// chain_at`): phase `p`'s threadgroups are the three words the phase before wrote, read when phase `p` starts (Metal's
// indirect dispatch; the CPU reads them when the launch runs; where a GPU has none, the shim's re-dispatcher runs the
// count's groups on a fixed number launched). Every group dispatched has its work: no group takes from a queue, and
// none waits for another. The order between phases is the dispatches'.
//
// The chain's control, words in device memory the host zeroes before the chain (but phase 0's counts):
// - word 0: the phase running now, which every group reads as it starts (`ukl_chain_phase`);
// - words `4 + 4 p` to `4 + 4 p + 2`: phase `p`'s counts (x, y, z), the indirect dispatch's (`ukl_chain_counts_at`
//   is their byte offset, for the host).
// A phase's groups are counted as they finish (`ukl_chain_finish`, a word a phase, the caller's); the group that
// finishes last is the phase's finisher, the only group that writes what the next phase reads: its counts and the
// phase word (`ukl_chain_next`). A count of 0 runs no group, so no finisher: the chain ends there, the phases after
// it left at the zeroes the host wrote. A phase that should pass with no work dispatches one group that does none.
//
// The finisher is the last of its phase's groups to finish, so every group of the phase has read the phase word
// before the finisher moves it on.

#pragma once
#include <ukl/backend.h>

// Phase `p`'s counts' byte offset in the chain's control.
#define ukl_chain_counts_at(p) (4u * (4u + 4u * (p)))

// The phase this dispatch is: the chain's control's word 0.
UKL_INLINE uint32_t ukl_chain_phase(UKL_GLOBAL uint32_t* ctl)
{
    return ukl_atomic_poll(&ctl[0]);
}

// One thread of a group, after the group's writes and a device barrier: counts the group finished (`done`, the
// phase's word) and says whether it is the phase's finisher, its last of `groups` groups. Its writes are seen by the
// finisher's reads after (the fence before the count).
UKL_INLINE bool ukl_chain_finish(UKL_GLOBAL uint32_t* done, uint32_t groups)
{
    ukl_fence_device();
    bool last = ukl_atomic_add(done, 1u) + 1u == groups;
    if (last) ukl_fence_device();
    return last;
}

// The finisher of phase `phase`: the next phase runs `groups` groups (0: the chain ends), and is the phase running.
UKL_INLINE void ukl_chain_next(UKL_GLOBAL uint32_t* ctl, uint32_t phase, uint32_t groups)
{
    UKL_GLOBAL uint32_t* c = ctl + 4u + 4u * (phase + 1u);
    ukl_atomic_store(&c[0], groups);
    ukl_atomic_store(&c[1], 1u);
    ukl_atomic_store(&c[2], 1u);
    ukl_atomic_store(&ctl[0], phase + 1u);
    ukl_fence_device();
}
