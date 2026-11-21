# 0414: A grammar's proofs are answers by content

Status: decided (the user, 2026-11-18: "please figure out how we can also implement a proof
cache so that we don't do the heavy work over and over"). Follows 0264 (images by content), 0092
and 0103 (a cache is a key and a result, nothing more).
Numbered 0402 where it was written.

## Intro

**The problem.** The Bitlex compiler proves facts about a grammar's masks before it uses them: a
row's start is never inside a string, a mark's bytes are never a token's. Each such question
asks whether a value is zero on every stream. A small SAT solver over a window of bytes answers
it (`bitlex_prove::Induction`), and a cheaper search answers some first (`bitlex_prove::never`).

For Kotlin these questions are most of the compile. Before this work it took 125 s of the
grammar compiler's 128 s, and a JIT can not wait that long. Speeding up the prover helps (below),
but a compile still asks the same questions as the last compile of the same grammar. An edited
grammar asks nearly the same ones.

**What is wanted.** A question asked before is answered without its proof, across compiles and
across processes, with the same program as a cold compile.

## The decision

A question is a pure function of its content. Its answer is kept under a key made of that content,
as an image is (0264). The compiler takes known answers as an input and reports the answers it
made as an output. It keeps nothing itself.

- **The key.** The question's cone: the operations its value reads, renumbered in the order a walk
  from the value meets them. Each class operation carries its table's bytes. Each carry carries
  its fixed point's next. Then the delay, the prover's settings (horizon, conflicts, solves, levels,
  rounds), the kind of question (`never`, `first_bytes`, the plain search) and a version, bumped
  with any change to what the prover answers. The operations' numbers and the grammar's other
  values are not in it, so a question means the same in another grammar or another place in
  the same one.
- **No history.** An answer depends on nothing but its key. The prover's lemmas from an earlier
  question are not used by a later one. They were not needed: every proof Kotlin's compile made
  took two solves, the bounded one and one induction step. Without them the program is the same
  and the cold compile is faster, since the windows carry fewer lemma clauses. The streams the
  bounded solves find (`Witnesses`) only refute questions, which a proof would answer the same
  way: a real stream with a bit is no proof.
- **In, out.** `bitlex_masks::compile_proved(…, known)` takes the known answers, and
  `Compiled::proofs` returns the ones it made, as keys and answers (`bitlex_prove::encode` and
  `decode` as bytes). The caller keeps `made` where it keeps compiled images: the engine build's
  content store, with a shipped bundle as its seed (Q234). A test passes them from one compile to
  the next.
- **Kept by digest** (2026-11-20, Q234). A question's text stays its key's content; what is kept is
  the first 16 bytes of the text's SHA-256 (`bitlex_prove::Key`). The texts were 4.7 KB each, and
  Kotlin's answers were 14 MB; as digests they are 52 KB. Two questions share a digest with odds of
  about 2^-128 a pair, as two contents share a content store's key.
- **The same program.** A compile with every answer known makes the same program as one with
  none (`compile_time`'s fingerprint, and a test).

**Given lemmas (with 0412).** `gap_ends` proves lemmas of two regions and gives them to its
prover for the claims after (`Induction::add_lemma`, `never_inductive`). Those are kept apart
from a question's own (`given`): `ask` clears only the question's. Its single questions are
not keyed, since their answers depend on the lemmas given before. The whole of `gap_ends` is one
question instead: whether all its claims never hold, keyed by their union (`all`) and the
prover's settings. The lemmas are made only of the regions that union reads, which makes the
procedure a function of its key: a lemma over values outside a claim's window is never applied.
The claims are asked first. A claim a real stream refutes (`Witnesses`) ends the question: no
lemma proves it. Pair lemmas are made only for a claim neither proved nor refuted.

Kotlin, at the merge with 0412: the pairs of all 64 regions, proved at every compile, had taken
the cold compile past 10 minutes. With the above, cold 16.2 s, warm 0.86 s.

## Why not

- **A map inside the compiler** (a static, or a field kept across compiles): a cache store of
  its own, with its own life and memory (0092, 0103). The answers are facts made from inputs,
  and the caller keeps them as it keeps the rest.
- **The lemmas kept across questions, in the key:** then a key holds the prover's state as well
  as the question. Two compiles reach the same state only by asking the same questions in the
  same order, so an edited grammar would miss on nearly every question.
- **The whole program cached by the grammar's bytes:** that is the image (0264), and it stays.
  It does not help an edited grammar. This decision does.

## Measured

Kotlin (step 35), the dev build (the compiler at opt-level 3), one machine under load:

| | Bitlex compiler | whole compile | answers made |
|---|---|---|---|
| before this work | 125.7 s | 128.5 s | |
| the prover's speedups below, lemmas kept | 29–33 s | 31–35 s | 2,355 |
| lemmas per question | 16.5 s | 18.7 s | 2,355 |
| every answer known | 0.57 s | 2.7 s | 0 |

An edited grammar, given the unedited one's answers (`compile_time` with `RERAIL_KOTLIN_EDIT`),
each time the same program as the edit's own cold compile:

| edit | new questions | warm | its cold |
|---|---|---|---|
| a soft keyword removed (`reified`) | 0 of 2,355 | 0.52 s | 22.4 s |
| a byte added to a class (`lam_punct`, `!`) | 521 of 2,355 | 7.3 s | 35.0 s |

The rest of a warm compile is Cranelift (tier 1 2.5 s and tier 2 1.3 s cold, about 2 s warm).

The prover's speedups, each with the same program:

- the solver's clauses in one arena, and its memory kept between solves;
- a question's window built once for all its solves;
- fixed-hash maps, which also make the compile the same each run: tier 1's program had differed
  between runs, by `HashMap`'s order;
- the streams the bounded solves find, kept as witnesses that refute later questions;
- `row_modes` asking a row's toggle only where its `x` is fixed.

Retired instructions: 2.50 T to 0.80 T, with lemmas kept.

## Tried, not kept

Each measured on Kotlin's cold and warm compile together (`compile_time`), in retired instructions:

- **CaDiCaL** (the `cadical` crate, 1.9.5), incremental. Each question's window was a live solver.
  Its definitions and the question's lemmas stayed in it for good. Each solve's own clauses (the
  bounded start, the bytes left out, the hypothesis) went under an assumption, retired after.
  Result: 566 G, against 473 G for our solver, the same with the `plain` configuration. The
  queries are small, so a solver instance costs more than it saves. That is why rIC3 has its own
  solver (GipSAT) for IC3's queries. The program was the same.
- **A memo of the plain search's values** (`Proof::eval` per operation and lag, under the
  choices made). No measurable change: 6.0 s to 5.4 s under varying load.
- **A real stream before the plain search** (a bounded solve, as `Induction`'s first step). No
  change. Of the 588 plain questions on tier 2's rows that cost 6–7 s, 216 are proved, and the
  rest fail by the search's budget, not on a real stream within the window.

What is left of a cold compile: `row_modes`' questions (974, 11–12 s under load), the rows' pair
questions (588, 6–7 s), and the rest about 4 s.

