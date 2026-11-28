# 0690: A corpus is spread over the chunks that evaluate at once

Date: 2026-11-25 (the user's choice: "Item 4: commit it"). Follows decision 0435 (chunks grow from what the first
measured) and Q314 (the same cut on every run).

## Intro

A corpus run evaluates its files in chunks, up to `at_most` at once (8 on the M3). The first chunk is small
(`FIRST_NODES`, 65,536 nodes); each next one grows `GROWTH` times from what the last measured, up to the caller's most
or what the memory room holds. On a small corpus that cut leaves the run serial: 300 files of 456 K nodes, with a most
of 1 M, were 2 chunks, the first's 65,536 nodes and then all the rest. The second evaluated alone while 7 workers
waited.

## Decision

1. **The corpus's nodes are estimated from the chunks filed.** The schedule is given the corpus's files. Each chunk
   filed adds its nodes and its files (`Measured::files`; a host and its guests are one). The corpus's nodes are the
   filed chunks' nodes a file times the corpus's files: a function of the chunks filed in their order, so the same on
   every run (Q314).
2. **No chunk is past the spread.** The spread is the estimated nodes over `SPREAD` (2) times `at_most`. A chunk's size
   is the least of the caller's most, what the room holds and the spread, or the grown size when that is less
   (`By::Spread` when the spread set it). It is never below the first chunk's size. Two chunks for each worker make the
   run's tail half a chunk's work, not a whole one.
3. A corpus whose spread is past the most (PostHog: about 49 M nodes) is cut as before.

## Measured

On the M3, after the hub merge (41654b2e52, fe19d42bab), two runs each, in turns.

300 files × 317 rules, all 661 relation outputs identical:

| | before | this decision |
|---|---|---|
| Chunks | 2 | 7 |
| Wall | 31.8 s, 32.9 s | 23.8 s, 23.0 s |
| Waiting for a chunk | 17.1 s, 18.5 s | 9.1 s, 8.3 s |
| Instructions | 1,023 G, 1,022 G | 1,014 G, 1,015 G |
| Peak | 2.42 GB, 2.47 GB | 3.23 GB, 3.18 GB |

The peak rose 0.76 GB: 7 chunks' blocks are alive at once where 2 were.

PostHog with `len-_py`: the same chunks and findings; instructions 2,421 G and 2,430 G, against 2,415–2,423 G before,
within the run-to-run spread.

## Requirements

- R1: a corpus is cut into at least `SPREAD` times as many chunks as evaluate at once, sized by the spread, as many at
  once as the room holds (`a_corpus_is_spread_over_the_chunks_at_once`).
- R2: a corpus whose spread is past the most is cut at the most (same test).
- R3: the spread is never below the first chunk's size (`the_spread_is_never_below_the_first_size`).
