# 0684. A vector's guard is called only while the device holds its block

Status: done (2026-11-24).

## Intro

After the hub merge (18b4bf20a8), a cold PostHog run on the CPU took about 50% more CPU than before it. It also
retired 3.70 T instructions against 2.33 T. The instruction count does not depend on the machine's load, so it was
the measure used.

A sample of both engines over the same 40 s of a run showed a new hot frame at the top of the stack:
`Vector::as_ref` (1,637 samples), with the ledger's `Guard::cpu_read` beside it (897). Neither was in the pre-merge
engine.

The cause is commit 4de2121865, which folded `BlockVec` into `Vector` (decision 0628 §8):

- Every `Vector` checks each CPU read and write against its block's ledger (decision 0487 R4).
- The check sits in `Deref`, so every read of a value through `v[i]` makes it.
- On a CPU run no use is queued, and the ledger answers from a relaxed load. But reaching that load took a call
  through `Arc<dyn Guard>`, which cannot be inlined, at every value.

0487 itself asks for the check "at each write's start, not each value".

## Measured

M3, cold PostHog, CPU, profiling build. Findings were the same in each run: 29,930, hash `7e89457edab0`.

| Engine | Instructions |
|---|---|
| Before the merge | 2.33 T |
| After the merge | 3.70 T |
| The guard's check removed (a probe, not kept) | 3.12 T |
| This decision | 3.30 T |

## Decision

R4 stays as it is. Only reaching the guard gets cheaper.

- `Guard::queued` gives the guard's count of the device's queued uses of its block. For the ledger, that is
  `chunk_uses`, which its own check reads first.
- `Region::guarded` keeps a pointer to that count, alongside the guard it holds. A region with no guard holds a null
  pointer, and reads none queued.
- `Vector::check` reads the count inline (`Region::device_queued`). It calls the guard only while a use is queued, and
  then the guard checks exactly as before.

## Left

The 0.18 T between this and no check is the count's load at each value read through `Deref`. Checking once per
slice taken, rather than at each index, would remove it. That changes where R4 is checked, so it is left as it is
here.

The rest of the merge's 0.97 T is elsewhere. Candidates, from the same profiles:

- the ast matcher wave: 17.8 s of CPU, against 5.1 s;
- rayon's idle workers spinning between jobs: 49 s of CPU outside every span, against 16 s;
- the granule rule's identity maps and granules, whose cost per document doubled.

## Tests

`rerail-vector`, `rerail-blocks` and `rerail-facts-base`, on the Linux CPU backend: 169 of 169, R4's ledger tests
included (`a_cpu_write_under_a_queued_gpu_use_fails`).

## Merged with the hub's (2026-11-25)

The hub made the same change on its own, in 359dc17781 ("A region asks its guard only while the device has uses of
its block queued"). Its names are kept, so that there is one definition: `Guard::queued`, `Region::device_queued`,
and a null pointer where there is no guard, where this record had a static count of zero. The behaviour is the same,
and the hub's change adds the test this record lacked: `a_guarded_region_asks_its_guard_only_while_uses_are_queued`
(rerail-blocks).
