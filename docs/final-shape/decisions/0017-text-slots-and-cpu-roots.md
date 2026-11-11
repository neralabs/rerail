# 0017: A label walk's text tests are bits computed once per (value, kind); a Metal wave runs its roots on the CPU

Status: accepted
Date: 2026-10-31

## Question
After 0015, Metal still lost to the CPU at S7's size. On 75 of S7's files, labels took 18.2 s and
the GPU's flush 15.2 s. The cause was cliffs, not volume:
- **A whole-run rerun on interpreter overflow.** A walk from a 25 K-node module's root took more
  than 2^16 steps, so every program reran on the interpreter.
- **Two WIDE reruns.** An interpreter task outgrew NARROW's 4,096 states, so the batch reran on 64
  lanes.
- **19.3 M of the 21.3 M remaining interpreter tasks were regex text tests,** which 0015's walk
  refused.
- **Roots ran on Metal's interpreter** across 38 executions, in 4.3 s. The CPU's JIT runs the same
  roots in 1.2 s.

## Decision
- **A walk's step limit scales with its file** (`compile_micro::steps`): at least 2^16, else twice
  nodes × words. One move from the root takes about that, and nested moves may still fail.
- **A text test is a slot of bits.**
  - `Micro::of` accepts `TEXT` and renumbers its value to a slot.
  - A slot's domain is the kind tested just before it, where no jump lands on the test.
  - `Micro::slot` evaluates the program's own value program (`is_match`) on every node of the
    domain, so the results are the interpreter's.
  - On Metal, slots are rows of a `texts` buffer, one label's words each. A row is computed once
    per (value, strings, kind) for the run's levels, in parallel across files.
  - A slot that fails (its work limit) leaves its pass on the interpreter.
  - 538 of the 560 label programs are walks now; 380 were before.
- **A Metal wave runs its roots on the CPU's JIT.** Labels are a dense pass over every node, which
  is the GPU's shape. Roots start only at the prefilter's candidates, few and scattered, and carry
  captures and outputs.

## Prior work consulted
- **Hist-index 2.23, "Generic interpretation is about 12x slower than specialized code" (S250, DH
  L7715-7728):** *Agrees.* A regex per (task, node) inside the interpreter is the generic case. A
  fixed predicate's bits are the specialized one.
- **Design-index 19 (D S162):** unseeded scans cost 27–41 ns a term and are candidates for SIMD or
  GPU scanning. *Partly.* The scan is on the host, in parallel and deduplicated (448 evaluations
  became 56). What the GPU gets is the bit test.
- **Hist-index 2.18, "measure at deployment size" (DH L3630-3635):** *Agrees.* Every cliff here
  appeared only at S7's size. On the 48 fixtures, Metal was already ahead.

## Consequences
- **Measured** (CPU time user+sys and wall, load 45–75):

  | | CPU backend | Metal before | Metal after |
  |---|---|---|---|
  | S7, 250 files, cold, wall | 27.0 s | minutes (3.9 GB) | 12.1 s |
  | S7, 250 files, cold, CPU time | 27.1 s | | 19.6 s |
  | S7, 75 files, labels | | 18.2 s | 2.2 s |
  | 48 fixtures, cold, wall | 1.23 s | 0.81 s | 0.85 s |
  | 48 fixtures, one-file edit, CPU time | 0.29 s | 0.22 s | 0.18 s |

- **Checked by:**
  - `micro_walks_are_the_interpreters_label_outputs`: 3.38 M starts on the fixtures and 13.5 M on
    the 115 KB module, 0 overflowing;
  - S1 and S2 parity: 87 of 87;
  - S4, S5 and S6;
  - S7 on both backends.
- **Tried and dropped:** running the demand's root executions and its partition search in
  parallel. They saved 0.9 s and 1.1 s, but cost 0.7 GB and 0.6 GB of RSS, against a budget
  already exceeded (O30).
- **Left (O28):**
  - the partition search builds batches to test fit, 2.4 s;
  - the CPU backend's labels are single-threaded, 21 s of its 27 s;
  - a first run compiles new root shaders (no longer on Metal's path) and the label pipelines.
