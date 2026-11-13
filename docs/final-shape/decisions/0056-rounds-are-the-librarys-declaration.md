# 0056: Bounded cross-file rounds are a program's declaration; typing answers are shared facts

Status: proposed (2026-11-07)

## Intro

A program runs in two parts. The local part runs per file. The cross-file part is the rules whose
recursion may cross a file boundary: for example, a call's target is in another module, which
re-exports it from a third.

Decision 0035 bounds that recursion. Each round reads the previous round's rows of the hop
relations, and a round stops at k (0047: k = 4, with a pending round past it). The bound exists for
typing programs. Their answers are lower bounds, and a fact a round could not reach is marked
unknown (03 §13, "Pending is a state distinct from ⊤").

Until now the runner applied the bound to every program with a hop. A data-layer rule whose
recursion crossed files was cut at round 4 and read as complete, which is silently wrong.

Typing work also reaches a rule a second way, through the bridge (`corpus/bridge.rs`). A rule that
reads `typing_<program>__<output>` makes the runner run `typing/programs/<program>.rr` first, and
the output's rows become the rule's facts. Before this record, those rows were printed as `.rr`
text, and the whole run was combined and parsed a second time. Each program prefixed them with its
own namespace, so two rules reading the same answers held two copies.

## Decision

1. **Rounds are declared by the library, in Soufflé syntax.**

   ```text
   // typing/programs/calls.rr
   // A typing program: its cross-file recursion runs in bounded rounds over the previous round's rows (decision 0056).
   .pragma "cross-rounds" "4"
   ```

   - `typing::cross_rounds::declared(prog)` reads the bound. `cross_rounds::k(declared)` returns it, unless a test's
     `with_k` or the `RERAIL_CROSS_ROUNDS` diagnostic overrides it.
   - A program with hops and no pragma runs its cross-file part as one fixpoint. The profile records the decision:
     "none: the program declares no `.pragma "cross-rounds"` …".
   - The pending round (0047) is unchanged. A program that declares `unconverged(0)` still gets it.
   - The `.rdl` copies carry the same line, which both front ends read, so each `.rr` stays its `.rdl`'s
     translation until the copies are removed.
   - The engine knows nothing about typing beyond this one generic switch. Which programs are bounded, and by how
     much, is the library's choice. A future library (a Rust typing program, a taint program) declares it the
     same way.
2. **Bridged answers are one shared fact family, handed over as rows.**
   - `bridge::is_bridged(rel)` recognises `typing_<program>__<output>`. `corpus::combine` does not namespace it,
     as it does not namespace `core.rr`'s relations (0047, core names): the answers are the same whichever rule
     reads them.
   - `bridge::facts` returns rows (`front::Value`), which the runner adds to the combined program. Nothing is
     printed or parsed again, and each program runs once however many rules read it.

## Why

- **Typing-only work is the program's own, and it stays generic.** The bound depends on the program's semantics:
  lower bounds plus a pending state. The runner cannot infer that from the rules, but the library author knows
  it. A pragma is how Soufflé attaches such knobs (`magic-transform` is already read the same way).
- **One fact family** follows decision 0044 (runs start from seeded facts) and 03 §14 (derived facts are cached by
  their inputs). The bridged rows are a derivation of (program, manifest), so they can be cached and recomputed
  only when the inputs change. That step is open item O91.

## Evidence

- `typing::cross_rounds` (3), `typing::oracle_tests` (5, including `rules_read_the_python_program_through_lib_calls`).
- `logic::plan::tests::own_ingestion_derives_what_t_facts_do` still runs the rounds of workload and calls as before
  (both declare the pragma).
- `logic::rr` (15, including `rdl_programs_translate_to_the_same_program`), `logic::front` (11).
- The replay: totals unchanged (3988 295 290 0; 1638 117 113 0; 2350 178 177 0).

## Prior work consulted

- **ast-index-spike:** design index §17 scopes cross-file facts out. It offers no prior form for bounded rounds,
  as 0035 and 0047 record.
- **The ledger's row 20** (partition by file; global machinery only where needed): rounds now run only where a
  program asks for them.
- **0035, 0047:** the mechanism (hops, `@doc`, the digest cutoff, k = 4, the pending round) is unchanged. Only
  who switches it on has moved.

## Departs from

- **0035's "every recursive atom whose node may be in another file reads the previous round".** That now holds
  only for programs that declare `cross-rounds`.
