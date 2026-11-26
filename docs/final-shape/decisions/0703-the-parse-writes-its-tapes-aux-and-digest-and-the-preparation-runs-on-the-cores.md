# 0703: The parse writes its tape's auxiliary index and digest, and the preparation runs on the cores

Date: 2026-11-25 (the user's request, in order: "Make the writer produce the aux sections and the SHA-256 while it
writes. Reuse one writer per parse worker. Number each language's keys once, in a table, instead of by name in every
document. Drop the self-validation and the repeated hashes. Do the directory-level listing, and multithread the
preparation. Split the matcher wave across the workers, and decide chunk sizes without blocking the builder.").
Follows decisions 0293 (a tape read in place), 0400 (listings), 0435 and Q314 (chunk sizes), 0442 (machine values).

## Intro

A PostHog run (29,885 Python files) spent CPU on work done twice. The parse wrote each tape, and then the auxiliary
index was built by reading the tape back. Each document set up a new writer and numbered its keys by name. A tape the
run had just written was walked again on restore, and the facts' digest hashed the tape's bytes. Startup stat-ed and
realpath-ed files one at a time on the main thread. The matcher wave found its roots in one loop over clauses × tapes,
and the builder waited for every growing chunk to be filed before it cut the next.

## Decision

1. **The format 16 writer records each node** (its code, end, start and stop) as it writes it, and hashes the source
   (SHA-256) where its newline index reads it. The auxiliary index is laid from that record (`aux_index::build_from`).
   It is read off the tape only for a producer that writes the tape's words itself (Bitlex's cursor writers).
2. **A parse worker reuses a writer**: one free list per machine (`Writers`), each writer given back when its tape is
   finished.
3. **A language's keys are numbered once**, into a table per language (`KeyNumbers`), not by name in every document.
4. **A tape this process wrote is not walked again.** The image keeps the address and length of the source it was
   written from. Restored over those very bytes, it is trusted and its digest is the writer's. Any other image, read
   from the store or the wire or restored over another text, is checked node by node as before. The facts' digest
   (v3) hashes the source's digest and the format's versions, not the tape.
5. **Files are stat-ed by their directories' listings**, and each level's directories are listed in parallel. Canonical
   forms are reused within a run, and phase 1's paths are canonicalised together. On macOS at most two path lookups
   (opens by path) run at once (`listing::lookup`): they contend on a file-system lock, so 16 at once took the same
   wall time at 25 times the CPU.
6. **A matcher wave's roots are a job for each (clause, tape) pair** on the cores. **Chunk n + window is sized by chunk
   n**, growth included; the first window's chunks are the first chunk's size. Sizes stay a function of the chunks'
   measures in their order (Q314).

## Measured

M3 Max, the profiling build, PostHog, runs in turns, 29,965 findings each:

| Item | Before | After |
|---|---|---|
| 1. Auxiliary index built (CPU) | 2.49–2.55 s | 1.80 s |
| 2. Writer setup (CPU, sampled) | 3.16 s | 0.74 s |
| 3. Keys numbered (CPU, sampled) | 0.86 s | 0.003 s |
| 4. Tape restore for the matchers (CPU) | 1.85 s | 0.96 s |
| 4. Facts' digest (CPU) | 0.69 s | 0.15 s |
| 5. The run's sys time | 38–41 s | 15–16 s |
| 6. The wave's own CPU on its thread | 5.5 s | 2.5 s |

Item 6 buys no wall time on this corpus. The wave's labels were already on the pool, and the chunks' evaluations keep
all 15 threads busy around it. Sizing a window ahead costs 0.5 s of wall time against Q314's sizing, and saves 1.7 s of
user CPU. The builder's remaining waits, about 3 s, are the window's backpressure.

## Left

- Bitlex's cursor writers record no nodes, so their tapes' auxiliary index is still read off the tape.
- On a parse miss, the writer's SHA-256 repeats the startup digest of the same file. Passing the digest in needs the
  `TreeParser` signature changed.
- `tape_identity`, the store's content address, still hashes the tape's bytes.
- Tapes from the store are still checked node by node, by design.
