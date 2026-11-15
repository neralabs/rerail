# 0260: a small run's fixed cost

Status: built in part (2026-11-15). Amends the cost statistics (decision 0100 §14) and the checkout's finding
(`source_tree`, decision 0224).

## Intro

A corpus run over one file still did 6.0 to 7.8 G instructions (0.34 to 0.42 s CPU, debug engine) on `dl1`, whose
one file the policy's scope leaves out: nothing was parsed or evaluated. All of that was fixed cost, paid by every
run, and by every test that starts one. The profile accounted for 0.10 s of the 0.55 s CPU it saw: the rest was on
threads that opened no span, and between spans on the main thread.

The user asked that this fixed cost be found and removed (2026-11-15).

## What a Time Profiler trace showed

A trace of one run (`xctrace record --template 'Time Profiler' --launch`), 770 ms sampled:

| Where | ms | Why |
|---|---|---|
| `rerail-costs read`: `read_samples` | 323 | Of the shared `costs.sqlite`'s 500,356 samples, 488,666 have no units (`strata`). Binaries from before such jobs kept no sample still write them. The read of the newest priced samples went down `samples_by_machine` and read each row to test `units > 0`: half a million rows. So did each lane and form's newest, under `strata`'s group |
| main: `source_tree::image` (`dladdr`) | 84 | `dladdr` finds the symbol nearest the address too, a search of the 415 MB debug engine's symbols. Only the file name was wanted, and `root()` asked it each time |
| main: the matchers compiled | 38 | `compile_matchers`, with the token tests' grammar read from its `grammar.json` (`TokenGrammar::from_json`, 20 ms) |
| main: the whole-run cutoff's check (`reuse::reused`) | 61 | |
| main: phase 0's derive, less the above | ~60 | |

## Decision

- **The priced samples have indexes of their own.** `samples_priced (machine, at)` and `samples_priced_by_form
  (machine, lane, forms, at)`, both `WHERE units > 0`, made once by the writer's `open`. Every query of the read
  tests `units > 0`, so each is a seek down them: the newest priced 20,000 in 1 ms, not 86, and the groups walked
  are the 5 with priced samples. The unpriced rows are still written by older binaries and trimmed as before;
  no read steps through them.
- **The image is found from the loader's list, once.** On macOS, the image whose segments hold the address
  (`_dyld_get_image_header`, its `LC_SEGMENT_64` commands, slid), not `dladdr`. It is the process's to the end, so
  it is found once (`OnceLock`). Elsewhere `dladdr` stays, found once too.

Measured on `dl1` (`layer-violations.rr`, cutoff off, warm store): 6.0 to 7.8 G instructions before, 2.8 to 3.0 G
after; with the statistics off, 2.3 to 2.9 G.

## A combine's programs: one phase 0, on the run's thread

Found while timing the session tests: `combine_read` read its programs on the pool with plain `par_iter`, without
the run's context (`run_state::Carry`). Each read that met a program with includes derived its graph there, a nested
run of `bootstrap.rr`, and in a test it asked the process's store, not the test's ("a store asked for off a test's
thread"). Carrying the context alone made those nested runs share the test's store at once: 3 of 16 runs of the
library tests failed (`content store writer is busy`; a memo made under another atom table than its writer's),
against 15 of 15 without.

Now the programs are parsed on the pool with the context carried (`rr::read_seed`), every graph is derived by one
run of phase 0 on the run's thread (`phase0::graphs`), and each is then resolved on the pool, where it finds its
graph kept (`rr::read_parsed`). Lowering on the pool carries the context too. The library tests: 15 of 15.

## The token automata built with the build

The matchers' 38 ms were mostly one thing: the token tests' census (`token_census::Lang`), which built the
dialect's token automata from its `grammar.json` (`TokenGrammar::from_json`). Parsing the JSON was 2 ms of that.
Determinizing every kind's automaton and finding its implied and kept tokens was the rest, at O0, in every process
whose rules test a token.

The automata are a function of the vendored grammars alone, so they are built with the build:

- The builder is its own file (`tape/token_automata.rs`), which names nothing of the crate. The lang crate's build
  script includes it, builds each grammar's automata (`GRAMMARS`), and writes them to `OUT_DIR` as bytes
  (`encode`). A process decodes its dialect's (`decode`) and resolves the names in its tree-sitter language, as
  before (`TokenGrammar::from_built`).
- What the automata say by name (`Census`) is written beside them. That covers each kind's implied tokens and
  parents, the token texts, and the extras. Compiling a rule asks only these: the token tests' walk, and the gap
  test's compile-time questions. A gap test decodes the automata when it first runs on a tape (`GapTest::grammar`).
- `from_json` stays, built in process, for the test that the two agree on every grammar.

The binary holds 6.1 MB more for the encoded automata (SQL's 1.7 MB). Measured on `dl1` (warm): the matchers' 24 ms
of main-thread samples became 7 ms. What is left decodes the automata for a rule's `pattern`: a pattern is parsed
into a tape, which must leave out what corpus tapes do. In instructions, 2.8 G became 2.6 G.

## Open

- Phase 0 in every process (~27 ms of main-thread samples): its nested run of `bootstrap.rr` for the texts' include
  graphs, whose own whole-run cutoff check is 22 ms of it. The check re-derives what its record could name: the
  included files, by reading every library file's statements (9 ms), and the combined program, only to read a few
  settings off it (11 ms). The cutoff's check reported apart before was this one: `rr.sh` runs the outer run with
  the cutoff off.

  Decision 0103 §8 says the whole-run cutoff is no cache to move: in the target shape, a run with no changed input
  has an empty delta and does nothing. Moving phase 0 there would cost more today. A warm maintained run with no
  change (the outer run here, cutoff off) is 110 ms of main-thread samples, against phase 0's 27 behind its cutoff.
  Phase 0 can drop its cutoff once a warm maintained run's fixed cost is below the cutoff's.
- Building the partial indexes on the shared file the first time took about a second, once.

## Tests

- `the_samples_read_are_found_by_the_priced_indexes` (logic): a store of mostly unpriced samples gives the priced
  ones, each form's included, and each query's plan names the priced indexes.
- `the_image_is_the_one_the_loader_names` (store): the image found is the test binary, in under 5 ms, and an
  address no image holds finds none.
- `the_automata_built_with_the_build_are_the_grammars` (engine): for every grammar, the automata and the census the
  build wrote agree with those built from its JSON in process.
