# Phase 2 S5 report: gate harness

Status: **in progress.** Branch `Moti-Velz/facts-engine-s5-gate`, worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s5-gate`. Raw runs: `~/rerail-bench/facts/phase-2/s5-gate/runs/<timestamp>-<label>/`.

## Gate (interface, first commit)

One command: `just facts-gate [--mutants]` (`scripts/check/facts-gate.sh` → `scripts/check/facts-gate.py`). It builds `facts-a`, `facts-b` and, once it exists, `rerail-facts` in release with each package's `mutants` feature where declared, in `target/facts-gate` (seeded by an APFS clone of `target/release`), then runs three lanes concurrently:

| Check | Lane | Command (in `<run>/<check>/command.txt`) | Passes when |
|---|---|---|---|
| `a-replay` | a | `facts-a replay --recording batch-06-recording.json --recording batch-06-probes.json` | every recorded call replayed, 0 FP, every miss or failure is accepted residue |
| `a-repo` | a | `facts-a repo --threads 4` over `~/rerail-bench/nera-data-layer-2b1504ad`, then `repo-compare` against `suite-b6.json` | 0 FP, 0 FN, the oracle's 9,010 files, every failure accepted residue |
| `a-subset` | a | `facts-a repo --threads 4 --root <subset>` | runs (the compiled per-file path over the subset) |
| `a3-check` | a | the same with `FACTS_A3_CHECK=1` | no substitution, kernel or relation memo differs from the unsubstituted path |
| `a-reference` | a | the same with `FACTS_A3_OFF=1` (the per-file reference) | findings and failures equal `a-subset`'s as multisets |
| `b-exactness` | b | `facts-b replay --events seed-5eedb/events.jsonl` | 0 differences between the dataflow and the from-scratch reference, every event replayed |
| `classifier-leak` | c | `cargo test --release -p rerail-facts leak` | at least one leak test runs and all pass (G2.1's seeded leak is rejected) |

Accepted residue: `serverplan_quality` / `quality-contracts-1` failures whose message names the `annotation` parse grammar, until S4's `string_type_definition` lands (`RESIDUE` in the driver; nothing else is accepted).

Subset (deterministic): every corpus file whose `sha256(relative path)[0] % 8 == 0`, plus every file holding a suite-oracle finding; about 1,150 of 9,010 files, hard-linked under `~/rerail-bench/facts/phase-2/s5-gate/subset-8/` (list in `subset-8.list`, rebuilt when it changes). The repository parity check runs on the full corpus.

With `--mutants`, each lane then runs its seeded mutants (`FACTS_MUTANT=<n>`) against its checks. A mutant is killed when one of them detects it: `a3-check` by the `FACTS_A3_CHECK` panic, `a-reference` by findings that differ from the clean reference, `b-exactness` by a difference (`--fail-fast`), `classifier-leak` by a failing leak test. An exit without detection is `error`, never a kill.

| n | Mutant | Site | Breaks | Checks |
|--:|---|---|---|---|
| 1 | `view-dropped-filter` | `facts-a` `subst.rs::edges` | the edges view keeps rows whose `edge_parent` is NULL | a3-check, a-reference |
| 2 | `view-swapped-ties` | `facts-a` `subst.rs::nodes` | `row_number()` ties broken in reverse union order | a3-check, a-reference |
| 3 | `view-subtree-end` | `facts-a` `ast.rs::has_end_memo` | the descendant range ends at `subtree_end - 1` | a3-check, a-reference |
| 4 | `delta-join-dd` | `facts-b` `flow.rs::tag` | the join drops the ΔR⋈ΔS term | b-exactness |
| 5 | `delta-count-retraction` | `facts-b` `flow.rs::presence` | presence counts ignore retractions | b-exactness |
| 6 | `delta-recursion-early` | `facts-b` R3 `iterate` | the recursion stops one level early | b-exactness |
| 7 | `classifier-leak` | `rerail-facts` `classify::file_source` | a per-file plan reading an exported relation is accepted | classifier-leak |

Output, one line per event on stdout, each prefixed `facts-gate: `:

```
facts-gate: run <run dir> commit <sha>[+dirty] load <1-min load>
facts-gate: build <ok|FAIL> <s>s <package>[mutants] ...
facts-gate: check <id> <pass|FAIL> <s>s <summary>
facts-gate: mutant <n> <name> <killed|SURVIVED> <s>s <check>=<killed|survived|error> ...
facts-gate: <passed|FAILED> <s>s: <p>/<c> checks pass, <k>/<m> mutants killed[; failing: <ids>]
```

`<run>/gate.json` (format `rerail.facts-gate/v1`) holds the same rows with details. Each check and mutant run has its own directory with `command.txt`, `stdout.txt`, `stderr.txt` (`/usr/bin/time -l`), `uptime-before.txt` and `uptime-after.txt`. Exit 0 passed, 1 failed, 2 usage. Options: `--only <ids>`, `--no-build`, `--run-dir <dir>`.

## Resume notes

- Next: seed the mutants (facts-a sites in S2's files, announced to S2; facts-b hooks and `--fail-fast` are S3's, agreed by message; mutant 7 in S1's `classify::file_source` once it lands), the `--engine facts` bridge, the plan-driven reference evaluator over S1's IR.
