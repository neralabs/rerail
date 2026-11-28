# The `rerail-engine` rules: the new engine's coding requirements

Rules over the Rust of decision 0686's crates (`native/crates/{blocks,delta_core,delta_exec,delta_device,delta_twins}`),
each a requirement AGENTS.md, a decision or the user has set for that code, written as the syntax the old engine's
rules read. The old engine runs them (`rerail-engine test-rr`, `run-policies`); `scripts/check/check_engine_rules.py`
runs both:

```sh
python3 scripts/check/check_engine_rules.py            # each rule's tests, then every rule over the checkout
python3 scripts/check/check_engine_rules.py --list     # and list each Log rule's findings
RERAIL_ENGINE=path/to/rerail-engine python3 scripts/check/check_engine_rules.py engine-host-never-sorts
```

A Steer rule holds on the tree, and a finding of one fails the check. A Log rule names code that still breaks it;
its findings are the work its requirement asks for. The Python checks in `scripts/check/` read the lines a commit
adds; these read the whole tree, so what came in unchecked (a merge, a moved file, a crate a check's list leaves
out) is seen too. The vocabulary they share is `docs/rule-syntax/lib/rust/rerail-engine.rerail`: test code (by file
name, and the items `#[test]`, `#[cfg(test)]` and `#[cfg(any(test, …))]` apply to), a call's name and arguments, and a
comment's mark on a line or the line above.

| Rule | The requirement | Mode | Findings |
|---|---|---|---|
| `engine-block-copies-marked` | Decision 0353: a call that copies values into another structure says which allowed kind it is (`// copy: load \| growth \| derived \| not block data`, `// copy (0353 step N)`) | Log | 41 |
| `engine-copy-mark-kinds` | 0353: a copy's mark names one of those five kinds, nothing else | Steer | 0 |
| `engine-no-zeroing-on-hand-out` | The user, 2026-11-26: no allocator, room, chunk or region zeroes what it hands out; no "a region reads zero" contract | Steer | 0 |
| `engine-host-never-sorts` | The user, 2026-11-24: "the host should not be sorting ANYTHING" (the executor's crates) | Steer | 0 |
| `engine-no-global-state` | Decision 0442, the twins' B1: no mutable static, cell, lock, atomic, `Once` or thread-local in the delta crates | Steer | 0 |
| `engine-block-layout-fields` | `Pod` holds no address; 32 bits where they fit; references across blocks are (descriptor, offset): no `usize`, `isize`, pointer or reference in a `Pod` struct | Steer | 0 |
| `engine-no-git-processes` | Decision 0497: gix, never a git process | Steer | 0 |
| `engine-no-machine-paths` | AGENTS.md: no machine's details in the tree: no home directory in a literal, tests included | Steer | 0 |
| `engine-no-language-labels` | No language knowledge in the generic layers (the user, 2026-11-26; REVIEW.md) | Steer | 0 |

The counts are this tree's. Three rules logged at first (5 zeroed hand-outs, 7 host sorts, 2 global states) and steer
since the code was fixed (decision 0686, "Three logged rules hold").

One mark excuses a zeroing: `// zeroing: fresh memory (…)` on a fresh allocation's call (`alloc_zeroed`, `calloc`),
where a reservation must read zero as an OS mapping does (`mem.rs`'s fallback for a platform with no mapping). It
never excuses a `write_bytes` or `fill`.

## What the Log rule names today

- **Unmarked copies (41).** Mostly in `blocks` (`kept.rs`, `chunk.rs`, `col.rs`, `relation.rs`, `sort.rs`, `block.rs`) and
  in `delta_twins` and `delta_core`, which `check_no_block_copies.py`'s directory list leaves out: a twin's output written
  into its block, a key built in a stack array, a block grown. Each takes its kind's mark, or is a read in place.
  `blocks/src/region.rs`'s `m.upload(…)` is a device upload, a kind 0353 names as a copy.

## What the rules do not see

- Code a macro writes: a `structure!` struct's fields, or a `thread_local!`'s statics, are token trees.
- A copy written as a loop of stores, or a `collect()` of a block's rows: the rule names the calls 0353's check names
  (the copying calls, an upload, a row or a row's read made a `Vec`).
- Which code runs on the device path: `engine-host-never-sorts` holds the executor's crates whole.
- The UKL kernels' C (`crates/ukl/lib/`): the rules read Rust.
