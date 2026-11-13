# 0107: Matchers run as one wave, with captures; the data-layer programs run on the `.rr` engine

Status: proposed (2026-11-10)

## Intro

Decision 0106 gave the `.rr` corpus engine an executor for `ast` atoms: each matcher
is lowered to the transducer's programs and fills a relation per chunk. Three things
were left open:

- **One launch a chunk.** Each matcher ran as its own pass, outside the accelerator's
  priced and placed launches, and Metal's passes recorded no device time.
- **Captures.** The transducer returns only the matched root. An `ast` atom naming
  nodes (`$A`, `$B`) was refused, and a rule part that read a field's node again stayed
  a join.
- **The data-layer programs.** The 142 data-layer checks, ported to `.rr` by hand,
  use about 248 `ast` atoms. They were written over their own `prelude.rr`,
  not over `core.rr`, and none had run on this engine.

This record closes the first two and runs the first data-layer programs, checked
against the upstream checks (the oracle) and against W, the slice path's reference.

## Decision

### 1. A chunk's matchers run as one wave, one launch

All of a chunk's matchers are lowered together into one wave (`LoweredWave`). The
wave is a `rerail_accel` program (`MatcherWave`, units = nodes × passes), submitted
as one launch through the same executor as every other priced kernel. So the planner
places it on the CPU or on Metal like the rest.

- On Metal the wave's command buffers record GPU start and end, summed into the
  profile's device time (`ast matcher wave`). A batch too big for one launch is
  halved and retried.
- On the CPU the wave runs the label passes, then each matcher's root program, over
  the tapes it is active on.

Measured: the matcher test pack runs in 2 Metal launches (0.138 ms of GPU time); the
Python pack's output is identical, in 4 waves on the CPU and 6 on Metal.

### 2. Captures: the transducer finds roots, W names the nodes

An atom with captures reads `rr_ast_<k>_<m>(n, x1, …, xm)`:

```text
// as written
MCALLF(c, f) :- ast(d, c, {"rule": {"kind": "call", "has": {"pattern": "$F",
                 "field": "function"}}}, F: f).
// as the planner sees it
MCALLF(c, f) :- rr_ast_3_1(c, f).
```

- The transducer finds the roots. At each root, W's own matcher (the policy engine's,
  `waves::findings::captured`) matches again and reads the names' bindings. A
  multi-capture gives every node, so the rows are the product over the names.
- Tapes are confirmed in parallel. A root that exceeds its file's visit budget gives
  no row, and the profile counts it.
- A rule part that reads a field's node again (`field(c, "function", f), kind(f, …)`)
  now joins through a capture. Only a field that holds one node is absorbed
  (`single_field`, from the grammar): a matcher's `field` reads the field's first
  node, a join every node. Absorbing a multi-node field lost 31 findings on the
  Python pack before this gate.
- A `transform` (12 data-layer atoms, all `replace` over a string's decoded value)
  binds text, not a node: the column is a symbol, interned in the store's atoms. The
  transducer lowers the rule without the transform, which matches nothing itself.

Measured on the Python pack: 159 rules' parts run as 21 matchers, output identical.
CPU, with this capture absorption off and then on: 27.06 → 28.25 s and
25.36 → 26.11 s (load 17–33). The join work is the same (203.45 M vs 203.93 M
rows). The matcher spans themselves are small (7.9 ms on the CPU lane), so the extra
second is not yet attributed.

### 3. A matcher the transducer cannot lower is lowered relaxed, and W confirms

Some rules hold terms the transducer's labels cannot carry: `compare`, a fact that
may be unknown under a `not`, `filePath` below the root. These are not refused. The
matcher is lowered as a **superset**, and W's matcher confirms each root
(`Captor::verify`):

- `compare` and nested `filePath` terms are dropped where dropping only widens: as a
  conjunct, or a `not` holding one in a positive place. Then, failing that, every
  `not` in a positive place.
- An `any` with an arm left matching every node goes too.
- The base policy's utils are relaxed the same way, but only when no `not` reaches a
  widened util, directly or through `matches`.
- W confirms with the file's path installed, so path tests hold as in W.

`any` arms that bind one name each no longer count as a shared binding.

### 4. Policies scope documents; W's syntax admission gates matchers

A program's `.policy` JSON scope (include and exclude globs) selects the documents
when no `.scope` is given. A matcher runs only on files W admits: its `syntax`
(valid or invalid) and its text literals. Before this, the data-layer run read every
file and reported 9,024 parse-error findings.

### 5. The data-layer programs are ported mechanically

`scripts/dsl/data_layer_rr_port.py` turns a program (its main file and part files)
into programs over `core.rr`. Each file stays its own program and carries the main
file's `.policy`. The rewrites:

| data-layer form | here |
|---|---|
| `doc(d, p)` | `doc(d, p, _)` |
| `@within(n, a) = 1` / `= 0` | `within(n, a)` / `!within(n, a)` |
| `@depth(a)`, `@line(n)` | `ast_depth(a, k)`, `line(n, l)`, fresh variables |
| `@class_path(c)`, `@class_prefix(n)` | shim relations over `nearest` and a class-kind set |
| `@rx_matches(s, re) = 1` | the text test `@rx(s, re) = 1` (batched, decision 0037) |
| `@substr`, `@instr`, `@trim`, `@like`, … | the catalogue's `sql_` forms, as SQLite has them |
| `@printf("%s:%d", …)` | nested `@concat` |
| `@id(n)` | the node's id as text (an identity, never part of a key) |

The SQLite functions are tested against SQLite's own (`sqlite_functions_as_sqlite_has_them`).

### 6. Results

`scripts/dsl/data_layer_rr_probes.py` runs a ported program over each recorded probe
(`fixtures/host/data-layer-port/batch-06-probes.json`). It compares
(path, line, code) keys with the oracle's.

| check | probes equal | repository (9,010 files) |
|---|---|---|
| serverplan_dispatch_architecture | 10 / 10 (4 findings) | 0 findings, as the oracle; 3.4 s wall, 9.8 s CPU |
| legacy_execution_surfaces | 5 / 5 (2 findings) | 0, as the oracle; 8.5 s wall, 20.9 s CPU |
| serverplan_steps (8 files) | no probes | 0, as the oracle and W; 16.1 s wall, 20.0 s CPU (Metal) |
| serverplan_state_tripwires (3 of 5 files) | see open items | 0, as the oracle; 14.6 s wall, 45.7 s CPU |

The repository is clean for these checks, so the repository runs test only for false
positives. The probes test the findings. The oracle took 6–77 s per check (1 worker).
Load was 30–60 throughout.

### 7. Fixes the port found

- **A deadlock in the text tests.** `Vocabulary::rows` held its cache's mutex while
  the searchers ran on the pool. A worker waiting in that evaluation took another
  chunk's `rows` and waited on the same mutex. About one run in two hung with every
  worker parked. The lock is now held only to read and to add.
- `within` lost its declaration in a program's own part (`own_part` took it as a
  library relation, since `core_names` reads `core.rr` after lowering drops it).
- `finding(clause, code, n)` was decoded as the two-column findings form.
- An undeclared input, and an output with no domains, panicked; both are errors now.

## Open items

Decision 0108 closes the capture cost, `shortlex_min`, the tripwire options and the relaxed
matchers' cost, and records where the remaining programs stop.

- **`shortlex_min`.** Two tripwire part files declare a lattice over symbols ordered
  by text (length, then lexicographic). Lattices hold integers; this one needs the
  atoms' text in its join.
- **Options other than the defaults.** The port fixes options at their defaults (its
  SQL stages were written for entryPoint "all"). The tripwire and quality probes ran
  under `scan_source` and `conditional_architecture_findings`, so they cannot be
  compared yet.
- **The remaining programs.** quality needs `@substr_bytes` and its block inputs;
  public_api_triples and internal_dead_code need the symbol-graph inputs
  (`base_edges`, `module_of`, `reference_edge`); execution_authority_bypass needs
  `@qualname`.
- **The capture cost.** The ~1 s of extra CPU with capture absorption on is not yet
  attributed.
- **Relaxed matchers' cost.** A relaxed matcher's roots are all confirmed by W on the
  host. How many roots that is, per check, is not yet measured.
