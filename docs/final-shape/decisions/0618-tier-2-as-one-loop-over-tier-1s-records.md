# 0618: Tier 2 as one loop over tier 1's records

Status: done by the implementer. The item is the user's (2026-11-22, goal: "Make the one-loop tier 2 read .rr's records
and codes, adding those features one at a time. Each step is checked by the existing tests that compare the generated
parser's tapes with the JIT's, on the samples and on the 1,784-file corpus"). How it is done is the implementer's
choice. It follows 0551 and 0581-0583, which wrote JSON's tier 2 as one loop.

## What was there

The one loop (`bitlex_rust/one_loop.rs`, `tier2_loop`) wrote a grammar whose tokens tier 2 reads from the bytes at tier
1's index: JSON's. It refused any grammar with marks, extras, flags or readings: "marks, extras, flags and readings
are not written yet". `.rerail`'s grammar has all four. Its tier 2 (`rerail-rr-aot`'s `tier2.rs`) was the recursive
form (`bitlex_rust::tier2`): a method per rule, and a level's call a Rust call.

## The decision

1. **The one loop has a second form, over tier 1's records** (`one_loop/records.rs`). It is for a grammar whose
   tokens tier 1 decides: each token's first and last byte and its code (kind, word, marks). Both forms are one loop
   (`'run: loop { match st { … } }`) whose places are the levels' parts. They differ in what a place reads:
   - the byte form reads the bytes at the index;
   - the records form reads the records, as the recursive form does.
   `tier2_loop` takes the records form where the byte form refused.
2. **A level's call is a push and a jump.** The call pushes the caller's state that the level changes, then the place
   it returns to, onto `P::stack`, and goes to the level's first place. The state is:
   - the questions it asks (`asks`, where some mark has them);
   - its word (`word`, where the grammar has a `remember` choice);
   - the marks it refuses (`refusing`, where the grammar has `refuse … in …`).
   The level's end closes its node, pops these in reverse and goes to the return place. No call recurses. The depth is
   counted as before (`deeper`, `DEPTH`).
3. **The code after a call is a place.** Code that holds a call is written continuation by continuation: each
   decision's arms go on to the place after it, a loop's head is a place, and the same code is one place.
   - A flag set around a call (`Flag`) is pushed before its body and popped after.
   - A rule that is no level but holds a call is places too, written once. Its call pushes only the return place.
     That is the recursive form's method, with its follow as its exits.
4. **What holds no call is the rules' code** (`G::code`), as the recursive form writes it:
   - the decisions at a record's code;
   - the marks and their questions;
   - the comments;
   - the rewrites (`wrap`, `relabel`, `drop`, `merge`, the readings);
   - the regions read in parts.
   A rule that is no level and holds no call stays a method.
5. **The runtime is the recursive form's** (`G::assemble`, shared by both), with the stack added. So the records form
   writes the same tape operations in the same order. Only the control flow differs.

`.rerail`'s `Compiled::rust` is now the loop form, and `rerail-rr-aot`'s `tier2.rs` is generated from it.

## Checked

On a Linux arm64 farm host:

- `rr_parser`'s `rust_tests`: `rust_is_current`, `tapes_are_the_jits` (the samples), `corpus_tapes_are_the_jits` (the
  1,784 files) and both tier 1 record tests: 5 of 5.
- JSON's `json_rust_is_current` and Kotlin's `rust_is_current` (unchanged files), the compiler's and bitlex's library
  tests: all pass.

## Each feature checked by the tests, one at a time

The features went in together, and passed together. The goal asked for them one at a time, each checked by the tape
comparisons. So afterwards each was taken out alone: the generator changed in one line, `tier2.rs` generated again,
then the samples' and the corpus's comparisons run (`tapes_are_the_jits`, `corpus_tapes_are_the_jits`).

| Taken out | The first difference |
|---|---|
| The caller's word restored at a level's end | sample `X = min(max(Y) : { c(Y) }) : { b(Y) }`: the outer reading takes `max`'s kind |
| The caller's refused marks restored | corpus file 1679: refused |
| A level's own refused marks set at its call | sample `.policy {"a": "\q"}`: parsed, the JIT refuses it |
| A flag restored after its body | corpus file 4: refused |
| The depth counted back at a level's end | sample 2 and corpus file 13: refused |

`asks` is not in the table: no `.rerail` mark asks a question, so its tier 2 has none.

Then the features were added one at a time. The run starts from the records form with all five out. Each step adds
one back, generates `tier2.rs` again, and runs both comparisons. The comparisons now count every case and file that
differs, not only the first. The corpus now holds 1,831 files, since the repository has grown.

| Step | Samples that differ (of 53) | Corpus files that differ (of 1,831) |
|---|---|---|
| The five out | 9 | 693 |
| The depth counted back | 4 | 48 |
| A flag restored | 4 | 0 |
| A level's own refused marks set | 1 | 1 (file 1679) |
| The caller's refused marks restored | 1 | 0 |
| The caller's word restored | 0 | 0 |

Each step clears its own differences. Setting a level's refused marks without giving the caller's back refuses file
1679, and the next step clears it.

On the first run, the word's restore was not caught. No sample read a word back after a nested level had set its
own. The samples now hold `min(…) : {…}` with an aggregate in its arguments.

Those samples then found a difference in the unchanged parser, and the recursive form shares its runtime. The machine's
`drop` leaves the node's record in the table as `DEAD`, and a reading that reaches it refuses, since it names no such
kind. That is how `.rerail` refuses a second argument in `sum(Y, Z) : {…}`. The runtime's `drop_made` took the node
out of the tape, so the reading found nothing and the parser took the file.

Now, where a grammar uses `drop`, the runtime holds each dropped node as dead, by the node it was in (`P::dead`). A
reading that reaches that node refuses (`read_dead`). A wrap's insert and a reading's rewrite move the indices with the
nodes. The tape still leaves the node out. A sample holds `sum(Y, Z) : {…}`, and the baseline passes again.

## Shape

The loop form's `tier2.rs` is 7,103 lines (7,142 before tier 1's masks went, 0619 step 3), against 6,350 for the recursive form.

The first version wrote a no-level rule that holds a call at every use, and was 29,324 lines. Point 3's single copy
replaced that.

The largest places are the grammar's own merge loops over a punctuation choice. The recursive form has the same loops,
as `punctuation` and `setting_punctuation`.
