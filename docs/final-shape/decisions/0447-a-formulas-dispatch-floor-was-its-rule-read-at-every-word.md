# 0447. A formula's dispatch floor was its rule, read at every word

Status: built (audited 2026-11-23). Accepted (2026-11-19). Follows 0457. Answers its "What is left" (the floor of a
dispatch), and keeps the parent reads of ops 7 and 8. Its measurement switch (`RERAIL_GPU_ENCODER_TIMES`) is not on
the hub's device-layer launches since the roll-up (work queue Q335).

## Intro

After 0457, most step formulas took about as long as a formula of one leaf read: 240–330 µs a dispatch in
one-formula mode (`RERAIL_FORMULA_TRACE`, `RERAIL_FORMULA_ONE`). That floor was taken to be the cost of a dispatch
itself. Two questions were left (the user, 2026-11-19: "address these four"):

- what a dispatch costs in itself;
- whether the parent reads left in `label_formula` cost anything: the parents of members (op 7), and the ancestors
  (op 8).

`RERAIL_GPU_KERNEL_TIMES` could not answer the first. It commits each kernel in a command buffer of its own and
waits for it, so its times hold the command buffer's own cost as well.

## Decision

1. **A measurement switch times each encoder inside its command buffer** (`RERAIL_GPU_ENCODER_TIMES=1`). It uses
   stage-boundary timestamp counters, which the GPU writes at each compute encoder's start and end. It needs no
   commit or wait of its own.
   - The profile gets `encoder_us.<kernel>`, `encoder_n.<kernel>`, and `encoder_gap_us` (from one encoder's end to
     the next one's start).
   - The samples are read through `resolveCounterRange:` directly. metal-rs 0.33's `resolve_counter_range` copies
     none of the bytes: it sizes the copy from an empty vector.
   - On the M5 Max these runs used, a tick is 1 ns (`encoder_ps_a_tick`, calibrated against `sampleTimestamps`).
2. **A dispatch's own cost is small.** In an ordinary run (internal_dead_code, s1000), the encoders add up to
   69.4 ms of the command buffers' 70.5 ms, and the gaps between them to 11 µs. `label_settle`'s encoders take
   10 µs each, an upper bound on a dispatch's fixed cost.
3. **The floor was the formula's last step.** In one-formula mode the encoders take as long as their command buffers
   (106.2 against 106.3 ms). A dispatch's time followed its chunk, not its formula: in one chunk a leaf read alone
   took 592 µs, and a 30-word formula 604 µs. The common part was the end of every formula, which writes the result
   into its rule's row. A formula pass's target (mode 2) is within its rule, and for every 32-bit word the kernel
   read the rule over the word's 64-node group (`root_mask`):
   - even where the formula's word was empty;
   - over 64 nodes, of which it kept 32;
   - for a rule over codes, one code read (`tape_code`) a node.

   Now the rule is read only where the formula holds at one of the word's nodes, and only at that word's 32 nodes
   (`root_mask_in`).
4. **The parent reads of ops 7 and 8 are kept.** On format 16 a parent read (`parent16`) reads the slot before the
   node and hops back by sibling distance, at most K = 8 slots. It is bounded, not a search of the tape.
   - internal_dead_code makes 25,710 parent reads in all, about 1.2 per (file, formula) group, against 673,280 child
     reads.
   - Op 7 has 25,422 members, and op 8 has none at s1000.
   - Its formulas are at the floor.
   - Reading through `tape_size` instead, as 0437 does, would scan every node of the file to save about one bounded
     read a group.

## Measured

s1000, pinned to the GPU, the memory line lifted, three alternating pairs against the engine before the change
(load 16–21). The first run after the Metal change was discarded. Rows on Metal: internal_dead_code 988d8fbe,
public_api_triples 3ffb9c9f, as before.

| | `label_formula` (encoders) before | after | The command buffers' GPU time before | after |
|---|---|---|---|---|
| internal_dead_code | 31.6–32.7 ms | 15.4–19.8 ms | 67.9–73.1 ms | 50.8–58.3 ms |
| public_api_triples | 19.8–28.6 ms | 15.2–18.7 ms | 44.9–65.6 ms | 51.5–63.5 ms |

CPU did not move: 5.3–6.0 s before and 5.5–6.3 s after, and 3.6–4.3 s in both. In one-formula mode the median
dispatch went from 204 to 170 µs, and a leaf read alone from 326 to 148 µs.

A wider group did not lower the rest (`RERAIL_FORMULA_WIDTH`, two runs each): 128 lanes took 17.3 and 18.6 ms, 512
took 16.0 and 16.4 ms, 1,024 took 19.7 and 22.4 ms.

## What is left

- **`label_leaves` is now most of the GPU time:** 32.2 ms of internal_dead_code's 50.7 ms, in 12 encoders of about
  2.7 ms each (Q281).

## On rr-align (roll-up 11, Q369)

The fix here (a formula writes into its rule's row only where it holds, at the word's 32 nodes) is in the kernel,
and it is kept. The measuring switch is not: `RERAIL_GPU_ENCODER_TIMES` samples Metal counters on each compute
encoder, and this branch's label passes launch through the device layer (`lane`, 0440 step C), which has no encoders
of its own to sample. `RERAIL_GPU_KERNEL_TIMES` (a command buffer a kernel) still times each kernel. Timestamps per
launch in the lane are an open item (Q369).
