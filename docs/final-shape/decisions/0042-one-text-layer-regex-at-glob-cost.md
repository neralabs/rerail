# 0042: One text layer: a regex costs what its best rewrite costs

Status: proposed (2026-11-05)

## Intro

A rule can test text in two ways:

- **the typed way** (decision 0037): `name($n, $s), @glob($s, "_*cache*")`, name sets,
  `@prefix`, path globs;
- **a regex**, as the source rules do: `regex: "^_\w*cache\w*$"` on an identifier.

0037 makes the typed way fast: each test runs once per distinct string, not once per node. But we
can't count on rules being written well (user, 2026-11-05: "I want both the regex paths and the
non-regex paths to be equally efficient, we can't always count on the rules being great"). Rules
will carry regexes, and badly written ones:

- `(?s).*`, which tests nothing (587 uses in serverplan_quality's port);
- a regex on the text of every node, whatever its kind;
- a regex over a whole module's text to ask whether the file mentions a name.

This record makes both ways **one computation**. Every text test, whatever its spelling, is reduced
to a canonical matcher. A regex and its glob rewrite get the same key, so they are the same job,
run once and shared. The remaining real regexes run on the same two paths the typed tests use.

## Decision

### 1. Every text test is normalized to a canonical matcher

Regexes, globs, `@prefix`/`@suffix`/`@contains`, name sets and literal `regex:` slots are all parsed
to one syntax tree (`regex_syntax`'s HIR; a glob is translated first). Then:

1. **Simplify.**
   - Drop captures and redundant flags.
   - `(?s).*` and any pattern that matches the empty string unanchored become `true`.
   - `\b\B` and other empty languages become `never`.
   - `^(?s).*R.*$` becomes "contains R".
2. **Find a literal shape.** The literal extractor (`regex_syntax::hir::literal`, already used for
   0041's prefix pass) decides whether the language is a finite set of strings. With the anchors
   that gives:

   | Anchors | Finite set of strings becomes |
   |---|---|
   | both | an exact set: a hash of symbol ids |
   | start only | a prefix set |
   | end only | a suffix set |
   | neither | a contains set |

   Otherwise it stays an automaton.
3. **Key it.** The key is the normal form's digest. Equal keys are one job across every rule, every
   clause and both front ends (matcher slots and Datalog tests).

So `^(?:on_commit|on_rollback)$`, `$s in {"on_commit", "on_rollback"}` and an `any` of two
`regex:` slots all become one exact set. `_\w*cache\w*$` on an identifier and `@glob($s,
"_*cache*")` become one automaton.

### 2. Where the text has a vocabulary, the test runs once per distinct string

The vocabulary holds the distinct texts of token-like nodes: identifiers, dotted names, string
literals (raw, quotes and prefix included), numbers, comments, and file paths. A matcher on any of
these runs over the distinct strings, never over nodes:

| Canonical matcher | Runs as | Measured (0037, nera-data-layer) |
|---|---|---|
| exact set | hash lookup of symbol ids | 396 of 444 identifier patterns |
| prefix, suffix, contains sets | one multi-string search per group | — |
| automaton | dense or lazy DFA per group of at most 64 patterns, over the distinct strings | hash + DFA: build 0.44 s, run 0.11 s |

- **A regex on a string literal runs on the raw text in the vocabulary.** Nothing has to be
  rewritten into decoded `str` values for it to be fast.
- **The result is a set of symbol ids.** A node's test is one bit lookup (spec-transducer
  `TEXT_ID`). A Datalog test is an integer join.
- **Warm runs evaluate only new symbols.** Results are kept per (matcher key, symbol).
- **Groups are bounded.** 639 patterns in one DFA took 676 s to build, and a `RegexSet` was 92.6 s
  against 0.52 s one pattern at a time (regex.md §1.1). Both rule out one combined automaton.

### 3. Where it doesn't, the file is scanned once and matches are placed with the tape

Some text has no vocabulary: a whole module, a function body, a regex on nodes of any kind. Each
file is scanned once per group of matchers, linearly, and the match positions are then mapped to
nodes using the tape's preorder and spans. No node's text is read on its own, so nesting costs
nothing extra (0041's O(bytes × depth) is gone).

How a node's verdict is read from one file-wide scan:

- **Unanchored (contains a match).** Node `n` with span `[s, e)` matches iff some match lies inside
  the span.
  - A forward DFA in overlapping mode marks every position where a match ends.
  - From each end, a reverse DFA run backwards in earliest mode finds the latest start of a match
    ending there.
  - `n` matches iff the largest of those starts, over the ends in `(s, e]`, is at least `s`. That
    is a range-max query, answered per node from a sparse table over the ends.
  - This is exact for overlapping matches. A leftmost, non-overlapping scan is not: for `ab|b` on
    `xab`, it misses the `b` node.
- **Anchored at the start only.** A forward DFA from `s`, earliest mode: the shortest match must end
  by `e`. It stops at its first accept or its dead state.
- **Anchored at the end only.** The mirror case: a reverse DFA from `e`.
- **Anchored at both ends.** A forward DFA from `s` must accept exactly at `e`. Literal prefixes
  pick the candidate nodes first (0041's pass), so most nodes are never started.
- **Look-arounds at the node's edges** (`\b`, multi-line `^`/`$`) can differ between the file's
  context and the node's own text. A match touching a node's edge is re-checked on that node's text.
  This is exact, and rare.
- **Results are kept per (content digest, matcher key):** a warm run does no work for unchanged
  files.

### 4. What "equally efficient" means

- **A regex that is a set or a glob in disguise is the same job as the set or the glob.** It gets
  the same key, the same run and the same cached result, so its cost is identical. That is 247 of the 252 distinct regexes in
  serverplan_quality (census below).
- **A real regex on vocabulary text** is a DFA over the distinct strings, as a glob that isn't a
  literal set is.
- **A regex on text with no vocabulary** has no typed equivalent except structure. It is one linear
  pass per file per group, cached. A structural rewrite (`==|!=` as the `op` relation) reads
  postings and stays cheaper by a constant factor, never by an asymptotic one.
- **Badly written tests cost nothing extra.**
  - `(?s).*` is `true`, `\b\B` is `never`.
  - A regex on every node's text is one file scan, not a scan per node per ancestor.
  - A module-text regex is one scan per file, cached by content.

## Evidence: serverplan_quality's regexes

252 distinct, 2,139 uses (`gate/data-layer/final-shape/data-layer-serverplan-quality.json`, a
census by form):

| What it tests | Distinct | Uses | Canonical matcher |
|---|---:|---:|---|
| exact set of names | 145 | 586 | exact set |
| file path | 38 | 481 | over the path vocabulary (9,010 paths on nera-data-layer) |
| prefix, suffix or contains set | 26 | 193 | literal set |
| other name shapes (`[A-Za-z_]\w*Intent`, `\b\B`) | 14 | 231 | suffix set, `never`, small DFAs |
| string literal values (`^[rRuU]*'''branch_id'''…`) | 10 | 18 | DFA over the raw string vocabulary |
| dotted names (`(?:^\|\.)PipelineLifecycleEffect(?:\[.*\])?$`) | 8 | 25 | DFA over the name vocabulary |
| case-insensitive | 5 | 12 | over lowercased symbols |
| code written as text (`==\|!=`, `^raise\s+TypeError`) | 5 | 6 | file scan (§3) |
| `(?s).*` | 1 | 587 | `true` |

Only the 5 code-as-text regexes (6 uses) need §3's file scan.

Today's cost of the same tests: text slots 1.6 s CPU at 1,000 files (t18 § 3), each slot evaluated
per (value, kind) over nodes after a file-wide literal pass.

## Steps

1. **The normalizer and its key** (`text::canonical`), built from `logic/convert.rs`'s
   `shape`/`alternatives`/`is_anything` and micro.rs's `prefixes`.
   - Test: every serverplan_quality regex gets the census's class.
   - Test: each regex and its glob or set rewrite get one key.
2. **The vocabulary matchers and result sets** (0037 step 4): symbol-id sets per key, grouped DFAs
   and literal searches, kept per symbol.
3. **The file-scan path** (§3): overlapping forward scan, reverse earliest scans, range-max per
   node. Differentially tested against `Regex::is_match` on each node's own text over the 200-file
   corpus, including the `ab|b` case and edge look-arounds.
4. **Both front ends read the results.** The matcher's `TEXT` slots become `TEXT_ID` bits, and
   Datalog tests become joins. Datalog gains `@rx($s, "…")`, normalized like the rest. The per-row
   `str_test` SQL function goes.
5. **Measure, cold and warm, CPU:** serverplan_quality as ported (regexes) against the same rules
   with the regexes rewritten. The goal is the same CPU within noise, and text time near zero
   against today's 1.6 s.

## Prior work consulted

- **0037 (proposed).** *Extended.* Its vocabulary matchers and result relations are §2. This record
  adds regexes as an equal input, the canonical key, and §3 for text with no vocabulary.
- **0041 (chunked text).** *Narrowed.* Its file-wide literal pass is the candidate step of §3. Its
  per-node DFA confirmation is replaced by the overlapping scan and range-max. The device part (O66)
  becomes the vocabulary and file-scan matchers, chunked.
- **PIAT `research/regex.md` §1.1, §2.1–2.3.** *Agrees.* Hash for exact sets, small DFA groups, no
  combined `RegexSet`.
- **Lessons ledger, rows 17–18.** *Agrees.* No per-row callback in the SQL phase. Row 18's "no LIKE
  or INSTR string games" is about rule authoring, and is kept: the canonical form is internal, and
  rule authors still write structure where they can.
