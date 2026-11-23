# 0646: Proofs within a budget; a part row's scope; regions proved apart once

Status: decided (2026-11-23, on the user's order: "proofs must be capped at 1 second", and "fix
the ridiculously inefficient proofs"). Follows 0414, 0412.

## Intro

**The problem.** A cold compile of Kotlin's grammar proved for minutes, with the CPU pinned. A farm
trace (one Linux host, 60–300 s windows, the budget lifted) found where:

- **A question asked twice was proved twice.** `Proofs::ask` looked only in the known answers
  (the shipped ones, or a store's), never in those the same compile had just made.
- **The token pairs** (two rows that can begin with one byte, and must never start at one byte):
  3,492 pairs, about 40 ms each, over 70 s in all. Most are the parts of Kotlin's string regions,
  each part a row in each region (`Readers::rows`): 137 of the 156 rows.
- **Nothing proved that two regions never fill one byte.** A walk's kinds are proved apart once
  (decision 0412's lemma), but Kotlin's regions nest and resume, so it has no walk. Each pair of
  rows in two regions had to find it again, in an induction of its own, and hundreds failed: their
  pairs went to tree-sitter's fallback.
- **The mode question per row** (`row_modes`: whether a row's start fixes each toggle's mode)
  had the same two failings: one question per row, with no regions lemma.

## The decision

- **A work budget for the provers** (`bitlex_prove::Budget`): steps of the plain search, each
  window variable built and each conflict of a solve, shared by a compile's questions.
  `PROVING_STEPS` is 2^20, about a second; `RERAIL_PROVING_STEPS` sets another. Past it, a question
  is not proved, with no proof begun. A cold Kotlin compile does not fit in it. The shipped answers
  (`kotlin.answers`) answer every question a compile and the parser's Rust ask, so neither proves
  anything. Blessing them (`RERAIL_BLESS_KOTLIN_ANSWERS=1`) lifts the budget.
- **Answers made in a compile answer it again**: `ask` keeps each answer in `known` as it makes it.
  `Proofs::used` lists the known answers a compile read, so a bless keeps the Rust compile's too.
- **The plain search's memo**: `Proof::eval` keeps each value's answer at each lag, until the
  search changes the bytes it assumes (`zero`). The answers are the same.
- **A part row's scope** (`TokenRule::Inside`'s last field): the mask every start of the row is
  in. That is its region's fill (`f_K`) for a part or the content, and its closer's first byte
  (`prev(f_K) & ~f_K`) for a row that closes the region. Two rows whose scopes are proved apart
  are apart. Many pairs share one scope pair, and its answer is made once. `row_modes` asks of
  the scope first too, since what holds of a scope holds of each row in it.
- **Regions proved apart once** (`regions_apart`): for a grammar with no walk, one value, any two
  regions' fills at one byte, proved never by induction on itself (`never_inductive`). Its answer
  is kept like every question's. Then each pair of regions is a lemma of the prover's proofs after
  (the token pairs', and `row_modes`'). Kotlin's 12 regions: proved in 13 ms.
- **Resumed regions with different templates**: the template is each region's, ORed, at the
  region's own closes. A nest's template level opens past the template closer's last byte
  (`or C2 bytes M`), in the grammar's syntax and in tier 1's walk. A grammar whose template closers
  are one byte reads as before.
- **A failed `code_l` check in a test build** names the word, both values and the file.

## Effect

Measured on one Linux host, the budget lifted, cold:

| | before | after |
|---|---|---|
| token pairs | over 70 s, not finished | about 7 s |
| pairs not proved apart (tree-sitter's) | hundreds | 2 |
| `row_modes` | not finished | about 50 s |

The cold compile still takes minutes. Most of it is now the gap check, `gap_ends` (314 claims,
about 140 s), and `row_modes`' questions that are false. Every question unrolls the regions'
carried values at every lag of its window: 25,000 to 55,000 SAT variables, 10–20 ms a solve. The
next step is to prove the regions once, not in every window (note t55).

Tried and taken back: one solver for all of a compile's questions, with each question's root
assumed. Its variables grew to 1.1 million, and each satisfiable solve assigns all of them: 108 s
against 22 s. Also tried: a gap claim without `refused` first, and the regions lemma for the gap
check's prover. Neither changed the time; a claim's own cone is the regions' too.

The bless: a cold compile and the parser's Rust compile, 117 s on an M-series Mac. `kotlin.answers`
holds fewer questions, so it is smaller: 61 KB before, 46 KB now. Of `kotlin_aot`'s generated Rust,
only tier 1 changed, by the new `code_l` message.

Found on the way and not fixed: on an x86-64 Linux host, a Kotlin compile, cold or warm, fails in
tier 2's Cranelift compile ("Too many return values to fit in registers"). On a Mac it compiles.
Tier 2's program is the one before this decision (its generated Rust is unchanged), so this is not
this decision's.
