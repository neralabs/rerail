# 0446. A fact's findings are written once, in a block, and the device reads them there

Status: accepted (2026-11-19). Closes Q289; removes 0353's copy 16.

## Intro

A fact is a semantic predicate the host evaluates, such as `binds`, `nthChild` or `filePath` (decision 0030). Its
findings on a file are rows: a node, then each slot's target. A fact with no slot finds a node set, kept as bits.
The host finds them before any label pass (`fact_rows`), and the label passes then test each row's targets against
their slots' labels.

On Metal the rows were copied three times before the device saw them:

1. `Fact::found` collected them into vectors;
2. `matcher_metal` turned them into `Pass::Fact` rows of batch-numbered `(node, label)` pairs;
3. `Runner::level` copied those into the level's constraints, and uploaded them.

0353 listed the upload as copy 16. The user, on Q289: "address these four".

## Decision

1. **A batch's facts are found into one block** (`FactRows::block`, an anonymous `kind::FACTS` block). Its
   reservation is address space: room for a row at every node of every tape, for each fact. Only what is written
   becomes resident.
2. **`Fact::found` writes the findings where it finds them** and returns where they are (`FactFound`):
   - rows: `n` rows of `1 + slots` u32s, the node and then the targets, file-local. Their region is the file's
     nodes; what is not used is given back (`shrink_last`) where no other tape allocated after it;
   - a node set: a word per 64 nodes, written into the block by the producer itself (`write_bits`). Facts on a tape
     that ask the same question (`Fact::question`) share one region (`SiblingPositions::placed`).
3. **The CPU's label passes read the block** (`FactFound::rows`, `FactFound::nodes`).
4. **The device reads the same bytes.** `Pass::Fact` holds the block and, per active file, where the findings are.
   The block is bound to the pattern kernels whole, as a no-copy wrap of its written extent (`Block::gpu_buffer`, at
   buffer 7). A fact item is one file's findings:
   - its root is the findings' first u32;
   - its first constraint is the file and whether the findings are bits;
   - then comes a label per slot.

   A thread takes a row, or a word of the bits as its root mask. Targets are made batch-numbered on the device
   (`node_base`). The level's constraints are now only a task description: the patterns' (variable, label) pairs, and
   each fact item's file and labels.

## Measured

- The upload is gone: on the 15 fixtures it was 32 MB (8.1 M words) before 0460, and once a level after it.
- Rows: s1000 internal_dead_code 988d8fbe and public_api_triples 3ffb9c9f, on Metal and native, as before.
  On s1000 the facts' passes run in the host's fallback (`labels.fallback.fact`, 25), so these runs test the CPU's
  read of the block, not the device's.
- The device's read is tested by `facts_on_metal_are_read_in_their_block`, which covers:
  - rows with a slot (`binds` with a `value`);
  - rows with none;
  - bits (`nthChild`).

  It is also tested by the pack's parity gate, `lowered_clauses_find_the_same_roots_on_metal`, whose clauses include
  `binds` facts. Both match the CPU.
