# 0225: Preparation costs the change: its analyses move onto the image, as rules the engine maintains

Status: draft (2026-11-14), for review. Builds on decision 0173 (a program is a flat image), and is its stage 3
made specific. Stops decision 0210's line of work at its §12.

## Intro

Before a batch of policies runs, the engine *prepares* its program: it prunes, simplifies, lowers, pushes joins
into the file-local part, factorizes, locates and splits it. Each of those passes rewrites the program, and most of
them first ask questions of the whole program:
- which relations are constants;
- which columns hold locations;
- which relations are string filters, and which are file-local;
- the strata;
- which relations are equal (`congruence`);
- the domains of columns;
- the spans a pattern reads.

Decision 0210 made these questions cheaper. dl1 (44 data-layer policies, cold) went from 23.64 to 20.80 G
instructions in twelve steps. Each step found the same shape:
- a pass that rebuilds its answer over the whole program, every time it is asked;
- relations keyed by their names;
- an answer found by a hand-written fixpoint with its own numbering.

This record says what is wrong with that shape and what replaces it. In short: preparation should cost what
changed, not what is there, and the way to get that is decision 0173's: one program form keyed by ids, with every
whole-program question a rule over it that the engine maintains by delta.

## What was found

On dl1, over one cold run (0210, 2026-11-14):

| Question | Times asked in a run | Done over |
|---|---|---|
| the split (`file_local::split_in`) | 44 (21 computed, 23 from the memo) | the cone of what changed, else everything |
| `constant_relations` | 52 | the whole program |
| `simplify`'s `congruence` | 24 | 27,664 rules in all; 19,306 distinct bodies |
| the rule index (`RuleIndex::of`) | 89 | the whole program |
| the strata (`front::components_of`) | about 20 callers | the whole program |

Four things follow.

1. **Preparation is redone, whole.** A round of pushing joins changes a few hundred rules. The next split, the next
   `congruence` and the next index read all of the program again. The split memo (0197) and the bodies it now keeps
   (0210 §10) find what did not change by hashing all of it: reading unchanged data, made cheaper. Across processes,
   a prepared program is kept whole, by its text's digest (0073, 0155). An edit of one rule prepares the batch
   again.
2. **Each question is its own evaluator.** The split's four fixpoints (constants, locations, filters, file-local),
   span patterns, column domains, text-kind demand, the keeps factors, `congruence` and the strata are each a Rust
   fixpoint, with their own rounds and their own invalidation. They are Datalog over the program. The engine has
   that schema (0074), phases, and delta maintenance (counting, DRed). Preparation uses none of them: the "phases,
   not new evaluators" rule, broken by the compiler itself.
3. **Each pass numbers the program again.** Names are the keys, so each fast path makes its own ids. 0210 added
   several: `shape::Rels` and `Form`, `ColMap`, `front::Graph`, the span-pattern ids, `lower::domains`' and
   `text_kind_demand`'s numberings, the keeps factors' bits. Each is a copy of what 0173 §1 gives once.
4. **The benchmark measures the path that should not exist.** dl1 runs with `RERAIL_TAPE_CACHE=off`: no store, so
   every run prepares everything. In use that happens once per change. What a user waits on is the edit: change a
   rule or a file, run again (t34). There, preparation costs what is redone, not how fast it is redone, and 0210's
   numbers do not measure it.

## Decision (proposed)

### 1. Preparation is steered by the edit

The measure of preparation is the edit profile of t34: a one-file edit and a one-rule edit, warm, instructions and
CPU. A cold dl1 run stays as a regression check, not a target. No more constant-factor work on the tree's passes:
0210 ends at §12.

### 2. The interim numberings are stand-ins

0210's numberings (§3 above) are kept only until their pass reads the image. Each one goes when its pass moves, and
no new one is added. A pass that needs ids before then waits for stage 2.

### 3. The whole-program questions are rules over the image

0173 §9 and §10 name prune, magic sets, the strata and merging repeats as rules over the image's relations, kept by
the engine's maintenance. This record adds every question in "What was found":

- **The split.** Constants, locations, filters and file-local are greatest fixpoints: a relation stays until some
  rule of it fails. Each is the complement of a least fixpoint ("leaves"), which is plain recursive Datalog with
  stratified negation. The split goes first: it is asked most, has the most rounds, and its form is the clearest.
  A cross-file relation's cause (`causes`) stays a Rust explanation, made for a rule the rules say fails.
- **`congruence`.** Partition refinement, as 0173 §10's group-by over rules' byte hashes, labels by class.
- **Span patterns, column domains, text-kind demand, the keeps factors.** Each is a fixpoint over reads.

Each is checked against the tree pass it replaces on every pack program, as 0173 stage 3 says. Then a pass's edit
(a few rules appended or dropped, 0173 §9's `from`) is a delta of the analysis, and a round costs its rules.

### 4. A prepared program is kept as its image and its analyses

A prepared program is kept today as its text and a pointer (0073, 0155). With the analyses maintained, what is kept
is the image and the analyses' relations, per module (0089's modules: a library once, each policy's own part on its
own). An edit of one policy prepares that policy's part and maintains the rest by delta. A cold run is an append of
everything.

### 5. What `congruence` still merges is measured first

0089 holds libraries once, so the copies `congruence` was made to merge back should be gone for libraries. On dl1 it
still runs 24 times. Before §3 ports it: measure what it merges (library copies left, policies' own repeated rules,
or nothing), and drop the calls that merge nothing.

### 6. Who does what

Stage 2 of 0173 (the planner on the image: t36 item 7) belongs to the Typing-support-final-shape work, and is under
way. This record's steps begin when it is merged, so its passes and the planner share one image and one set of ids.
The order then: §5's measurement and §1's edit profile, then the split (§3), then the other questions, then §4.

## What this is not

- Not a new evaluator: the analyses run in the engine, as rules.
- Not a cache: what is kept is the program and relations derived from it, maintained as any relation is.
- Not a change of findings: each step is checked against the pass it replaces.

## Open questions

- Whether the analyses run in phase 0's bootstrap run, or in a phase of their own over the image's relations.
- Whether the explanation of a cross-file relation (`causes`) can be read off the rules' failing reads, so no Rust
  explanation is left.
- What §4's per-module key is for a batch of policies that share a request cone (0089 stage 2).
