# 0485. The parse workers' time is the reference graph's summaries, not the parse

(Was 0466 on final-rerail-shape; renumbered at its roll-up into rr-align, roll-up 18, past 0482.)

Status: accepted (2026-11-19). Follows 0484 (its next steps 1–3, Q343).

## Intro

0484 found the parse workers taking 13.4 s of CPU on all (3,089 files, 9.6 M nodes), 4.3 ms a file. The user: "please
do all 3. Our parsing should be lightning fast".

The parse is fast already. On all, on an M1 Max:

| Span (own CPU) | Before | After |
|---|---|---|
| lean Python parse and tape | 1.1 s (0.36 ms a file) | 1.1 s |
| W's reference graph | 11.8–12.4 s | 6.5–6.8 s |
| ast captures (the CPU's label passes) | 4.4–4.7 s | 2.8–2.9 s |
| the parse workers' phase | 13.6–14.1 s | 8.1–8.5 s |
| the run's instructions | 635 G | 502–505 G |
| the run's wall | 23.9–25.7 s | 19.0–19.8 s (an unloaded host) |

So most of the parse phase is **W's reference graph**: `calls::summarize`, W's call-graph summary of each file, read
off the run's tape through ast-grep's node API. The rows were the same as native at every step (ed502024).

## Decision

1. **`NodeRef::find` from a file's root is a binary search.**
   - A file's tape lists its nodes in preorder, which is by start byte. So a nonempty range is found by
     `find_in_tape`, as `find_near` already found it below a covering ancestor.
   - The descent it replaces read every child's range down the path, and every sibling before it.
   - The summary looks a node up from the root for each parameter, member and branch. Those reads were a quarter of
     its time. The ast captures look nodes up the same way and gained too.
2. **A tape node's `ancestors` climb by parents.**
   - A parent read in format 16 is at most `K - 1` = 7 sibling steps.
   - The root path it replaces (`Tape::path`, one descent from the root) read every sibling before the path at each
     level. A climb mostly stops a few levels up, at the nearest function or class. In a module of hundreds of
     statements, the summary's climbs from its parameters were a sixth of its time.
3. **The name interner is sharded and interns a file's names at once.**
   - `calls::names` interns every dotted name per process, behind one `RwLock`. Ten workers summarizing at once
     waited on it for a third of the summaries' samples, because most of a file's names are new to the table and
     take the write lock.
   - Now the table is 256 shards by the name's hash. `Name::new_all` interns a file's names with each shard's lock
     taken once a prefix depth: a read for the names the shard has, a write for the rest. The entries are made before
     the write lock, and each batch starts at a shard of its own.
   - Waits are still 15–18% of the summaries' samples on an unloaded M1 Max: parked readers behind a shard's writer.
     A parked thread takes no CPU, so this costs parallelism, not CPU. The last two changes (entries made outside the
     lock, the rotated start) measured neutral. The fix is structural, below.

## Measured

Samples on all, an M1 Max (`sample`, 12–15 s of a run). The share of `calls::summarize`'s samples:

| | Before | After |
|---|---|---|
| lock waits (`swtch_pri`, `cvwait`) in `Name::new` / `new_all` | 32% | 15–18% |
| `Tape::end`, `start`, `path`, `next_sibling` | 28% | 14%, spread over the walk's `range` and text reads |

## After (0472)

Two more reads taken out of W's walk, on all, native, an M1 Max at load 4–8, alternating runs:

- `walk` read every node's parent and its field's range for `head_of_attribute` and `call_target`; only an
  identifier or an attribute is tested with them. Read for those alone: the reference graph 6.41–6.57 → 6.10–6.23 s.
- `TapeNode::range` read the start, then the end, and format 16's end searched for the start again. The end is now
  the start plus the length (`end_with_start`): 6.11–6.13 → 5.87–5.90 s, instructions 497 → 492 G.

The parse workers' phase is 7.42 s (8.0–8.2 s before both). The rows were ed502024; the tape and summary tests (160)
pass.

## What is next

- **The interner per run, or after the parallel part.** The table is process state: per run, or each summary's names
  kept as text until the link (`relations_of`, serial), takes the waits away.
- **The summary as a tape pass.** What is left of `summarize` is W's walk, a `range` or text read a node through
  `AnyTape`'s dispatch: 2.1 ms a file, six times the parse. A pass over the tape's words, as the transducer's, is the
  shape that makes it as fast as the parse.
- **A test failing at HEAD:** `value::eslint_rules::no_dupe_keys_through_key_nodes` finds 3 of 16 keys. It fails
  without this record's changes too (checked on the farm host's copy at 1ccf569fe).
