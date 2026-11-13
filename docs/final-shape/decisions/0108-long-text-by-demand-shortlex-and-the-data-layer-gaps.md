# 0108: Long node text by demand, shortlex symbols, and the data-layer gaps closed

Status: proposed (2026-11-10)

## Intro

Decision 0107 ran the first data-layer programs on the `.rr` engine and left five
open items. These were the cost of capture absorption, the `shortlex_min` lattice,
options other than the defaults, the remaining programs, and the cost of relaxed
matchers. This record closes the first three and the last. It moves the remaining
programs forward and records where each one stops.

Running all five tripwire files over the data-layer repository (9,010 files) found
two engine defects, which this record also fixes:

- `text` was missing for long nodes.
- One file over the matcher work limit failed the whole wave.

## Decision

### 1. `text` at any length for the kinds a rule reads

`core.rr` says `text(n, s)` holds every node's text. The projection made it only up
to 256 bytes, or for token-like nodes. Interning every subtree's text costs
`O(bytes × depth)` per file (decision 0042).

The tripwires' dataclass rule reads the text of a decorated class, then tests it
with a regex:

```text
// a class is a dataclass when its decorated definition's text says so
dataclasses(c, n) :- classes(c, n), child(w, c, _), kind(w, "decorated_definition"),
    text(w, wt), @rx(wt, "(?m)^\\s*@(?:[\\w.]*\\.)?dataclass\\b") = 1.
```

Every decorated class over 256 bytes had no `text` row, so it was not a dataclass.
That made 117 false findings over the repository (SPT-017, SPT-018). The oracle
has none.

Now the projection reads the program for the **node kinds whose text a rule
reads**. That is a variable `v` of a positive `text(v, …)` whose kind the same body
fixes, either:

- by `kind(v, "K")`, or
- as the root of a matcher atom (`rr_ast_*(v, …)`), with the kind its rule names at
  the top.

Those kinds' nodes get `text` at any length. Every other node keeps the bound. The
demand is read from the prepared program and its matchers, so a cached program
gives the same demand.

A node whose kind the body does not fix still has the bound (open item).

### 2. A matcher over its work limit is confirmed by W, not failed

The CPU interpreter caps a root program's steps per task. One file over the cap
returned `WorkLimit` and failed the chunk's whole wave.

Now that (clause, file) returns every candidate root (the clause's domain, past its
entry) and is listed in `WaveRun::limited`. W's matcher then confirms those roots on
that file, under its own visit budget, as relaxed matchers are confirmed. Over the
repository this happened for 5 to 4 matcher-files per chunk.

### 3. `shortlex_min` over the run's atoms

`shortlex_min<symbol>` keeps the shorter symbol, then the lesser by its UTF-8 bytes.
Lattices hold integers, and this one reads the atoms' text in its join. So each run
declares it over its store's atoms once the store is chosen. Shortlex is a
well-order, so the lattice is finite. `TOP`, and an atom the run does not hold,
absorb.

### 4. Probes run under their recorded options

The tripwire SQL reads its options only in the matchers' `$option` cases. A probe's
recorded options (its `oracle_config`) now become the programs' `.policy` option
defaults, so the cases resolve as the oracle's did.

The probe runner also reads every part file's findings. Before this it read only
the main file's.

`serverplan_quality` differs: its final SQL is a case per entry point. Comparing its
probes needs a port per case (open item).

### 5. Costs measured

**Capture absorption.** Retired instructions are steady on a loaded machine, and CPU
time is not. With absorption on, a cold run of the Python pack retires 2.4–3.5 G
more instructions, out of about 553 G (0.5%).

- That is about 0.25 s of CPU, not the 1 s that CPU time suggested. Most of that
  second was load.
- By phase:
  - `local`: +1.3–1.5 G (more matchers in each guest round, and one plan that fans
    out: `impmod` seeded from a matcher relation, 0.8 M join rows).
  - `wait`: +0.6–1.3 G (the parse workers run the larger wave and the captures).
  - `write`: +0.2–0.4 G.
  - `combine`, `stats` and `cross`: unchanged.

The run summary now gives each phase's process instructions.

**Relaxed matchers.** These are the roots W confirms on the host, over the
repository:

| check | relaxed matchers | roots confirmed | kept | W's CPU |
|---|---|---|---|---|
| serverplan_dispatch_architecture | 8 | 0 | 0 | 0.00 s |
| legacy_execution_surfaces | 0 | 0 | 0 | 0.02 s |
| serverplan_steps | 4 | 94,365 | 94,365 | 0.12 s |
| serverplan_state_tripwires | 26 | 1,293 | 920 | 0.44 s |

W's CPU is the whole confirmation span, which includes captures. The counters are
`ast.relaxed_roots` and `ast.relaxed_kept`.

### 6. The converter

- `@qualname(f)` is the relation `dl_qualname(f, q)`, which folds over `nearest`.
  A frame's qualname is its parent's, then `.<locals>.` where the frame is a
  function under a function frame (else `.`), then its own name. This is the
  oracle's `contexts` stage verbatim. It gives `outer.C.<locals>.m` where Python
  gives `outer.<locals>.C.m`.
- `@printf("%0Nd", x)` is zero-padded: the last N characters of N zeros and `x`.
- `in_list("NAME", v)`, one relation over every list, held kinds in one list and
  field names in another. Each list is now its own relation, `in_list_NAME(v)`.

## Results

| check | probes equal | repository (9,010 files) |
|---|---|---|
| serverplan_state_tripwires (all 5 files) | 2 / 2 (1 finding) | 0, as the oracle; 137.5 s wall, 161.9 s CPU (load 94–105) |
| execution_authority_bypass | no probes | 0, as the oracle; 38.2 s wall, 71.8 s CPU (load 26–72) |
| serverplan_dispatch_architecture | 10 / 10 | unchanged |
| legacy_execution_surfaces | 5 / 5 | unchanged |

The Python pack's output is identical after each change.

## Open items

Decision 0109 closes the first two and moves the rest forward.

- **Long text where the kind is not fixed.** A rule reading `text(v, t)` for a node
  whose kind its body does not fix still sees the 256-byte bound. Decision 0042's
  file scan (§3) for node text would remove it.
- **`@rx` on a node** (`@rx(w, re)` with `w` a node) gives no rows, silently. It
  should test the node's text or be refused.
- **The symbol-graph inputs.** public_api_triples and internal_dead_code stop at
  `path_module`, `module_of` and `reference_edge`:
  - `path_module` is the path alone.
  - `module_of` is W's `file.module`, not yet checked against `core.rr`'s module.
  - `reference_edge` is W's cross-file references linker.
- **serverplan_quality** stops at its block inputs (`file_content`). It still needs
  `@substr_bytes`, and a port per entry-point case for its probes.
- **`impmod`'s plan** after absorption seeds from the matcher relation and joins
  `pkg` before `docof` (0.8 M rows; 0.02 s).
