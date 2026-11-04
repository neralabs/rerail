# 0033: A label pass's roots are a rule over each document's codes, not a node list

Status: accepted (2026-11-04, list step 28, after decision 0032)

## Context

A label pass runs its program at every node of its domain: all nodes, or the nodes of some
kinds, which the program's leading kind and supertype tests narrow further (`matcher::Entry`).
Before this decision, the plan wrote each pass's domain out as node lists, per file. Each label
run then copied them again:
- the micro walks into `items`, one u32 per (walk, root);
- the pattern kernel into `matches`, five u32 per root;
- the interpreter into one 24-byte `MetalTask` per (program, root).

On S7's 250 files the lists held 70 M roots, about 280 MB, and the walks' `items` another 211 MB.
The interpreted retry's 45 M tasks raised the footprint to 3,017 MB before its batch was refused
(decision 0032).

## Prior work consulted

- **ast-index-spike design index, conclusion 1 ("Nothing may store one row per node"):**
  **agrees.** A node list per pass is a per-node row made on every run.
- **The same index, conclusion 5 ("Fused plans beat materialized intermediates"; a materialized
  per-query anchor cost 289%):** **agrees.** The roots are read where they are used, not written
  out first.
- **Lessons ledger #5 (temp tables plus ANALYZE lose to fused plans):** the same lesson on the
  device. **Agrees.**
- **PIAT spec §20.5.1 (a batch document table, nothing concatenated) and §20.11 (no repacking):**
  extended from the tapes to the roots read from them.
- **Decision 0032:** the residency's reader state gains one index. It is built at admission and
  not stored, as 0026 allows.

## Decision

1. **The residency indexes nodes by code.** Each document gets `code_start` (`code_count + 1`
   u32s) and `by_code` (`nodes` u32s, node order within a code). They are built once at
   admission, in the document's slab, beside the child lists, and named by `DocView`'s
   `code_count`, `code_start` and `by_code` (the words that were reserved).
2. **A pass's roots are a `Roots` rule.**
   - `Roots::Codes { kinds, entry, active }` covers a domain: its kinds, the program's leading
     tests, and the files where the clause is active. Every test is a property of a node's code
     (`DeviceCode`: its kind and its classes), so a document's roots are the nodes of the codes
     the rule admits.
   - `Roots::Listed` covers a clause's prefilter candidates, which are few and scattered.
   - Passes of one domain share one rule.
3. **Segments:**
   - A run turns each rule into segments, one buffer for the whole run: for each pass, a header
     (the segment count and the root count) and then one segment per run of roots,
     `{document, start in by_code or in the listed nodes, count, first index}`.
   - Their size is O(rules × documents × admitted codes): 3.9 MB on S7, where the node lists were
     70 M roots.
4. **The device finds its roots.**
   - A walk thread searches its pass's segments and reads its root from the slab (`root_segment`,
     `root_local`).
   - A fused group's threadgroup goes straight to its file's segments (`root_doc_segment`).
   - The pattern kernel takes one item per pattern pass, found by thread, and fact rows stay one
     item each.
   - `items` and the per-root pattern items are gone.
5. **Not yet:** the interpreter still takes a task per root (`Runner::batch_inputs`, now
   enumerated from the rule). Its tasks should come from segments through a fixed pool of lanes
   (O38), so that its workspace is lanes × states whatever the root count.

## Consequences

Measured with load 24-27; CPU is user + sys.
- **S7's sample, cold on Metal:**
  - 1,259 keys, as before.
  - Audit CPU 7.5 s, down from 12.0 s.
  - The footprint peak is 889 MB. Its rises are now in the GPU run and the roots' partition, not
    in planning (O39).
- **Real files:** 1,259 keys on all four backends. The Metal backends take 6.7 s of CPU and the
  CPU backends 9 s.
- **Tests:**
  - S2: 158 of 158; interpreted, 34 of 39 equal (the same 5 refused).
  - The regression set: 195 of 195.
  - `phase_four`/`jit`: 109 pass, with the same 11 failures as before.
