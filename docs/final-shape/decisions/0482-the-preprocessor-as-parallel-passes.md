# 0482: The preprocessor as parallel passes: a spike of the shape

(Was 0421 on Go-C-and-C-support; renumbered at its roll-up into rr-align, roll-up 12, past 0474.)

Status: analysis (audited 2026-11-23). Draft (2026-11-18). Follows 0408, whose gate missed. A spike: no product path
calls it. The placement of the passes (transducer side programs, or `.rerail` rules for the directives) waits on the
user, from these numbers.

## Intro

0408 ran the preprocessing words on Metal as one serial machine per translation unit (TU). It
was correct, but more than 100× slower than one CPU core: a GPU thread runs about 9 µs a token,
and a dispatch lasts as long as its longest TU.

The user's point: the shape was wrong, not the device. A preprocessor is mostly independent
work. Lines, comments and tokens are data-parallel already (stage 1 and mode (a), on Metal).
Only the directives have an order, and they are few. Text identifiers either name a macro
where they stand or not. Each expansion is local to its invocation.

This spike reads a TU as passes over arrays. Each pass is a map, a scan, a sort, a search, a
gather or a scatter, so each can move to the device as it is. The spike checks that the passes
read every corpus TU as the serial pass does (which itself equals `clang -E`, 0327), and
measures the shape: rounds, widths, and the serial pieces left.

The code is `rerail_c_pp::stream::parallel`. Its corpus test is `the_corpus_as_passes`
(ignored), under clang's profile from `oracle_profile` (moved out of `examples/pp_oracle.rs`,
which now includes it).

## The passes

Positions are positions in the TU: every file instance's directives and tokens, includes in
place, in one order. Each serial rule becomes a question about a position.

**P0, each file once** (`files`). The file's text tokens, its directives with their lines (read
by the serial pass's lexer, kind by kind, as `Pp::directive` reads them), and its conditionals'
nesting: chains, arms and groups. A conditional's nesting is the file's alone, since the serial
pass skips a group by counting directives' names. Each `#define` is parsed here, once a file. So
is the include guard: where a file is one `#ifndef X` … `#endif` with nothing outside it, an
include where `X` is defined enters nothing (clang's multiple-include reading).

**P1, the directives' fixpoint** (`rounds`). Over directives only, never text tokens. A round
reads the state the last one left (the include tree, each instance's active groups, the
pragmas its text made) and:
1. lays the TU out: each instance's size bottom-up, then each first position top-down;
2. sorts the active definitions by (name, position). A name at position `p` means the latest
   row of it before `p`: one search. `#pragma push_macro` and `pop_macro` are rows too, a pop
   the value at its push, matched per name by depth;
3. evaluates each test at its position, each alone. A chain is read where its parent group is
   active, its arms up to the first true;
4. resolves each active include at its position (a literal name once a file), and drops those
   an earlier `#pragma once` or `#import` of their file skips;
5. expands the few text invocations that can reach `_Pragma` (libc++'s
   `_LIBCPP_PUSH_MACROS`), for the pragmas they make.

The rounds stop when one leaves the state it read. Every dependency points backwards in the
TU, so the fixpoint is unique, and it is the serial reading.

**P2, candidates** (`text`). The active text is stretches: an instance's tokens between two
directives, in an active group. A candidate is an identifier that names a macro at its position.
A function-like one runs to its `(`'s matching `)` (parentheses matched by depth). The
outermost invocations are those past every earlier one's reach (a prefix maximum).

**P3, invocations, each alone.** Each outermost invocation is expanded in isolation by the
serial pass's own expansion, under the definitions of its position. One that asks past its end
is merged with what follows and expanded again: a function-like name at its end before a `(`,
or a call left open.

**P4, assembly.** Each piece's count of tokens, a prefix sum, and a scatter. `__COUNTER__` is a
prefix sum too.

**Refused to the serial pass, with the reason:** `#embed`, `#line`, a call across a directive
(a directive inside a call's arguments), and `__COUNTER__` or `_Pragma` in a directive.

The serial loops left are the rounds, one invocation's expansion, and the host's reading of
files.

## Measured

**Correct.** Both corpora (`~/orca/rerail-shared/lean-corpus`, under clang's profile with
the SDK's headers), every unit read as passes and compared token by token with the serial
pass. Run under Relax, in chunks (`the_corpus_as_passes`, `RERAIL_PP_EACH`).

| Corpus | Units | The same | Differ | Refused |
|---|---:|---:|---:|---|
| C | 1,574 | 1,572 | 0 | 1 `#line`, 1 call across a directive (ghc's `RtsFlags.c`) |
| C++ | 1,561 | 1,560 | 0 | 1 `#line` (a flex scanner) |

**The shape**, over the units read the same (per unit: median, p90, max):

| | C | C++ |
|---|---|---|
| P1 rounds | 10, 14, 20 | 13, 24, 28 |
| Include depth | 7, 12, 18 | 9, 18, 21 |
| Rounds less depth | 0 to 7 (0 or 4 for most) | 0 to 8 |
| File instances | 49, 166, 793 | 115, 1,329, 1,546 |
| Directives / active | 9,803 / 4,518 | 11,663 / 5,716 |
| Tests read, all rounds | 6,903, 18,389, 87,064 | 10,924, 182,246, 274,148 |
| Tests read a round | 630, 1,277, 4,353 | 910, 7,776, 11,423 |
| Active text tokens | 4,432, 16,011, 314,177 | 9,707, 490,064, 614,843 |
| Outermost invocations | 180, 968, 15,975 | 375, 16,926, 20,440 |
| Longest invocation, tokens in | 22, 32, 234 | 22, 104, 1,082 |
| Longest invocation, tokens out | 52, 84, 6,516 | 60, 1,393, 8,000 |

Over each corpus:

| | C | C++ |
|---|---:|---:|
| Active text tokens | 15.3 M | 265 M |
| Invocations | 700,113 (4.6% of the tokens) | 9.77 M (3.7%) |
| Distinct by their tokens | 163,572 (23%) | 624,631 (6.4%) |
| Tokens the invocations make | 6.28 M (9.0 each) | 113 M (11.6 each) |
| Merges | 0 | 0 |
| Output tokens | 19.6 M | 363 M |

## What the numbers say

- **The text is wide and shallow.** About 96% of the active tokens are copied as they are:
  a prefix sum and a scatter. The rest is invocations, and no invocation in either corpus
  asked for a token past its end (no merges), so each is independent work. The longest
  serial piece left in the text is one invocation: at most 1,082 tokens in and 8,000 out
  (gtest's macro-stack test), and 84 out at C's p90.
- **Invocations repeat.** Only 23% (C) and 6.4% (C++) are distinct by their tokens. A cache of
  expansions, keyed by the tokens and the definitions they reach, removes most of P3.
- **The directives' depth is the include tree's.** The rounds are the tree's depth plus 0 to 8
  (definition chains through conditionals). The most, 28, is a handful of launches a round,
  with every unit of a batch in the same launches.
- **The directives' width is the work.** A round reads every live test again. But a test's
  value depends only on the definitions it looks up (its line, file and include level are its
  instance's). Each test now keeps them, and is evaluated again only where one changed. On a
  sample (200 C units, then 100 libc++-heavy C++ units from 300 on), every unit still the
  same:

  | | Tests read | Evaluated | Share |
  |---|---:|---:|---:|
  | C, 200 units | 1,067,490 | 246,286 | 23% |
  | C++, 100 units | 5,886,618 | 2,041,444 | 35% |

  Most of the rest is re-entered headers' tests: the same file under the same definitions,
  in another instance. A memo keyed by (file, directive, looked-up definitions) instead of by
  instance would share them, across instances and across units.
- **Instances.** libc++ makes 1,000–1,500 instances per unit even with guarded re-entries
  skipped: headers included many times under different macros, or guarded otherwise than one
  `#ifndef`.

None of this is a time on the device yet. Against 0408 the change is in the critical path:
0408's dispatch lasted as long as the longest unit's whole serial run (0.8 s alone). Here
the longest chain is the rounds, each as long as its slowest single test or include, then
one invocation of at most a few thousand tokens.
