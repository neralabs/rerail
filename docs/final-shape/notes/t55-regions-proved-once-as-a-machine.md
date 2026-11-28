# t55: The regions proved once, as a machine, not in every window

Status: design, for review (2026-11-23). Follows decision 0646 (proofs within a budget). Nothing in it
is built.

## Intro

**The problem.** A cold compile of Kotlin's grammar still proves for minutes. Decision 0646 cut the token
pairs from over 70 s to about 7 s. Two parts are left, measured on one Linux host with the budget lifted:

- **The gap check** (`gap_ends`): 314 claims, two per token row. About 140 s, and 86 of the claims are
  still not proved after the first pass.
- **`row_modes`**: about 50 s, most of it on questions that are false, where the prover fails slowly.

Both have one cause: **every question unrolls the regions again.** The prover's window (`Unrolled`)
encodes each carried value in the question's cone, each fill, toggle, escape and round carry, at every
lag up to its reach: the question's own lags plus a horizon of 8. Kotlin's regions nest (braces 4 deep,
block comments 3) and resume (a template's `}` back into its string). So almost every question's cone
holds the whole region machinery, and its window has 25,000 to 55,000 variables. A solve costs 10–20 ms,
and a question needs up to 64 of them:

- **A question that is true but needs a lemma** goes through `prove`'s rounds. The window's far edge
  leaves every carried value free. The prover finds an edge that is no real stream, makes a lemma
  ruling it out, and proves the lemma in turn.
- **A question that is false fails slowly** unless a stream inside the window shows it. A real
  counterexample often needs a region opened before the window (a string opened 20 bytes back). Then
  no bounded solve finds one, and the induction runs out of rounds instead.

Decision 0646's regions lemma shows what a fact about regions proved once is worth: Kotlin's 12
regions proved apart in 13 ms made the pair check about ten times faster. This note takes that to its
end. The regions are one machine with few states, so the prover can know that machine exactly, once,
rather than rediscovering pieces of it in each window.

## What the regions are

A grammar's `regions` statement is a sequential machine. Tier 1's Rust already runs it as one
(`bitlex_lex_rust`'s regions walk: the open kind, `depth`, the `nest` stack, `template`, `tb`). Its
state at a byte:

- **The open region**, or code: Kotlin has 12 kinds, so 13 values.
- **The nest stack**: up to 4 levels. Each level is a plain brace or a template's brace, and a
  template's remembers the kind it resumes (`tail` resumes `str`).
- **A nesting region's depth**: block comments up to 3.

Its input at each byte is a few local values the mask program computes without carries: each kind's
opener and closer, the template closers (`or C2 bytes M`), and the escapes. An escape is the one small
carry left: a run of backslashes (`Escaped`).

The mask program computes the same regions as fixed points over whole words (`rounds`), each region's
fill a carried value. The prover reads the mask program, so it sees the fixed point's carries and not
the machine.

## The design

1. **One machine, from one definition.** A `RegionMachine` is built from the grammar's `Region` list
   and its nests, the same list the tier-1 walk is generated from. Its state is a few small fields;
   its input, the opener and closer values at the byte. Each region value the mask program reads,
   `f_K`, `o_K`, a toggle or a level's flag, is a predicate of the state.
2. **Its reachable states, once a compile.** A walk over the machine's states, on real bytes, finds
   every state some file reaches, and a shortest file to each one. The states are finite, a few
   thousand at most for Kotlin, and the walk takes milliseconds. This set is the strongest invariant
   there is: no lemma about regions can rule out more.
3. **The window encodes the state, not the carries.** For a question, the window holds the machine's
   state at each lag: about 40 variables a lag (the kind one-hot, the depth, each level's entry), with
   the transition's clauses between lags. The state at the far edge is any reachable state. Each
   region value the question reads is its predicate of the state, and the fixed point's carries are
   left out. The window shrinks from tens of thousands of variables to the question's own operations
   plus about 40 a lag.
4. **One solve a question, exact.** Every edge state is reachable, and every byte after it is free. So
   a satisfying assignment is a real stream: the state's shortest file, then the window's bytes. Its
   negation is a proof. No rounds, no lemmas, no induction over the regions:
   - A true question costs one solve.
   - A false one costs one solve too, and its stream is a witness (`Witnesses`) that refutes other
     questions before they are solved.
5. **The questions that need no solve.** A part row's scope (decision 0646) is a region predicate.
   Many questions are a scope and a few bytes:
   - two scopes at one byte;
   - a mode's toggle in a region;
   - a row's last byte in `skip`.

   Those are answered from the reachable set with a byte-class table, by lookup. Only the rest go to
   the solver.

## The hard part: one meaning for the regions

The prover would read the regions from the machine, and tier 1 and tier 2 run the mask program's
fixed point. A proof is sound only where the two agree. Today they are checked against each other in
one place: tier 1's test builds assert that the walk's `code_l` equals the masks' `code` on every file
a test reads (the assertion 0646 made name the word and the file).

Two ways to keep one meaning:

- **The machine defines the regions, and the masks are checked against it.** One compile-time check:
  the fixed point's values equal the machine's predicates on every reachable state and input, a
  finite check over the same set. The masks are then proved to be the machine, once a compile, and
  every proof that reads the machine holds of them.
- **The masks define them, and the machine is derived and checked.** It is the same check, read the
  other way.

Either way it is one check per compile, about as costly as the reachable-set walk, and it replaces
the walk-versus-masks assertion as the place the two meanings meet. I prefer the first. The
`regions` statement is the grammar's meaning, and the fixed point is how it is computed over words.

## What it leaves alone

- **Grammars with walks** (decision 0412's kinds): their walks are this machine's simplest case. They
  would move to it too, and their pairwise lemma would no longer be needed.
- **Carries that are not regions**, such as a token's run or a check's states, stay as they are: few,
  and local to their tokens.
- **The answer cache** (decision 0414) stays as it is. The machine and its reachable set go into a
  question's key, through the regions' operations it already includes. `PROOFS_VERSION` is bumped
  once.

## Steps, each with its tests

1. **`RegionMachine`** from the `Region` list. Its test: on the Kotlin and JSONC corpora, its region at
   every byte equals the tier-1 walk's, and the masks' `f_K`.
2. **The reachable set**, with a shortest file to each state. Its test: each file, run through the walk,
   ends in its state.
3. **The masks-equal-machine check**, at compile time. Its test: a grammar with a region the masks
   compute otherwise is refused.
4. **The window over the machine's state**, behind the prover's settings: the answers must agree with
   today's prover wherever today's proves, and may prove more. Then today's region unrolling is removed
   for grammars the machine covers, not kept beside it.
5. **The lookups for scope questions**, and then a cold Kotlin compile measured against the budget:
   the target is that it fits in `PROVING_STEPS` (about a second), so a grammar edit needs no bless to
   compile.

## Open questions

- **Does the state count stay small for every grammar we have?** Kotlin's nests bound it (4 levels,
  2 entry types, 3 comment depths). A grammar with deep or many resumable templates may not; that
  grammar would keep today's prover until the machine has a compact form.
- **Openers with lookahead past a region's end.** `dq3` reads 3 bytes ahead. The machine's input reads
  the same masks as the walk's, so this should carry over, but step 1's test is where it would show.
