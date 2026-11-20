# 0475: The text in P1's iterations: `_Pragma` rows, and `__COUNTER__` in directives

Status: accepted (the user, 2026-11-19: "please implement these two": "A `_Pragma` that changes
macro state (`once`, `push_macro`, `pop_macro`, or one whose operand is a macro)", and
"`__COUNTER__` inside a directive's line"; 0467 left both refused, and named the choice: "either
the text pass joins those iterations for units that have one, or the refusal stays").

## Intro

The preprocessor runs on the device in rounds (0467). In each round, P1's iterations decide the
directives: the definitions' table, every `#if`, the groups, the includes. Then the text, the
tokens between the directives, is expanded. The two cases here run the other way: the text
changes what the directives see.

- `_Pragma("push_macro(\"X\")")`, `_Pragma("pop_macro(\"X\")")` and `_Pragma("once")` in the
  text are pragmas carried out where the expansion reaches them. A pop changes what `X` means
  after it, in the text and in a later `#if`; `once` stops a later `#include` of its file.
- `__COUNTER__` counts every use before it in the serial reading. A use in an `#if` counts the
  text's uses before it, and the text's uses after it count it.

So the text's expansion is an input of the iterations, and the iterations' answer is an input of
the text's expansion. Every dependency points backwards in the reading, as in 0467's iterations,
so iterating both to a common fixpoint gives the serial reading.

## Context

- 0467 §6 refused both (`X_PRAGMA`, `X_COUNTER`) and left the choice open. The user chose to
  build them.
- The spike (`c_pp::stream::parallel`) already places a text `_Pragma`'s pushes, pops and `once`
  as rows by position (`pragma_events`), read by the next round's table. It refuses `_Pragma`
  and `__COUNTER__` in a directive (`checked`). This decision keeps its placing, and builds the
  counter.
- The serial pass is the reference (`Pp::builtin`'s `Pragma` and `Counter`, `pragma_text`).

## Decision

### 1. The text pass runs inside P1's iterations

- An iteration that changes nothing and requests no file is followed, in `pp_setup`, by the text
  pass. That pass was `pp_setup`'s end and `px_expand`'s two launches: the stretches in the
  unit's order, the lookups, the top level, the queue's two phases and the places, and the
  counters' ranks. Its phases are unchanged; they run on the same grid, between the same
  barriers.
- The text pass writes three facts, each a relation in the arena (not a copy):
  - **the text's rows**: a push or a pop, by the key of its name and its slot;
  - **the text's `once` marks**: a file and a slot;
  - **the text's `__COUNTER__` uses**: each one's place in the reading (§3).
- A key over those facts is compared with the last text pass's. An unchanged key ends the
  iterations: the text pass's output is the round's. A changed one is a change, and the
  iterations go on from the facts. The iterations' bound (`P1_ITERS`) covers both.
- The uses' key counts only where a directive's line has uses: otherwise nothing reads them.
- `px_expand` goes: a round is 2 launches, `pp_setup` and `pp_check` (0460 §4's fixed count,
  smaller).
- A text pass that does not end the iterations drops its refusals: they are of facts the next
  pass changes. The lines' refusals are those before the last text pass; the text's are the
  expansion's to report.

### 2. `_Pragma`'s rows

- **Where.** A row's slot is that of its outermost invocation's last top token, the `)` that
  ends it. The serial pass carries the pragma out while it rescans, after reading that `)`. The
  text's lookups are at their own token's slot, and lists' lookups at their outermost
  invocation's `)` slot. `visible` reads the rows strictly before a slot, so a row is seen by
  every token after its invocation, and not by its own invocation's lookups.
- **What.** The pragma's text is the destringized string (C17 §6.10.9). Its first word:
  - `once`: the invocation's file, from the slot on, is not entered again (`P1_ONCE`, as a
    `#pragma once`);
  - `push_macro ( "name" )` and `pop_macro ( "name" )`: a push or pop row, the name's key as
    `pp_pragma_act` keys a directive's (its string's bytes destringized). They join the
    directives' rows in the table and its push and pop matching (0467 §3).
  - Any other pragma makes no row, and no `_Pragma` makes a token.
- **The operand.** A `_Pragma` whose operand holds a name is expanded first, as the serial pass
  does (`expand_list`): the operand is an argument's list (scanned and flattened like a
  parameter's), and the pragma is read from its tokens once they are final. One string literal
  is a pragma; anything else is malformed and makes nothing.
- **Refused, by name (`X_PRAGMA`):**
  - a lookup, inside an invocation, of a name the same invocation pops (it would see the pop
    after it in the serial reading);
  - two rows of one name from one invocation (their order is the rescan's);
  - a row-making `_Pragma` in a directive's line (the spike's refusal, kept).

### 3. `__COUNTER__` in the text and in the lines

- Each use has a place in the reading:
  - in the text: its outermost invocation's `)` slot, then its order inside the invocation (the
    chains' order, 0467 step 7c);
  - in a directive's line: the directive's slot, then its outermost call's first token in the
    line, then its order inside the call.
  A text token and a directive never share a slot, so the places are one order.
- A use's value is the number of uses of its unit at earlier places, the text's and the lines'
  together.
- The lines' uses are placed in each iteration, after their expansion and before `p1_if_eval`.
  They read the text's places from the last text pass. The text pass reads the last iteration's
  lines' places. Two relations, the text's and the lines', each rewritten by its pass.
- An arm after its chain's taken arm (the last iteration's) is not read: its uses count nothing.
- The value is written into the use's digits, as in 0467 step 7c. `X_COUNTER` goes.
- **Refused (`X_COUNTER_SPELLED`): `#` or `##` of the value**, a macro's argument that expanded
  to `__COUNTER__` (the unique-name idiom `CAT(x, __COUNTER__)`, and an include's name made of
  it). The rank among an invocation's uses is known only once the queue has drained, after the
  pastes read the digits. Before this the device pasted the placeholder `0` without a word; now
  the serial pass takes the unit.

## Requirements (each a test, before the code)

| # | Requirement | Test |
|---|---|---|
| R1 | A text `_Pragma`'s `push_macro`, `pop_macro` and `once` give the serial pass's tokens: names after a pop in the text and in an `#if`, an include after `once` skipped, a pop through a macro, in a header, in an inactive group, nested with `#pragma push_macro`; 2 launches. | `pragma_rows_are_the_serial_passes` |
| R2 | A `_Pragma` whose operand is a macro's is read from the operand expanded: rows from it, a diagnostic pragma from it, a malformed one making nothing. | `pragma_rows_are_the_serial_passes` |
| R3 | `__COUNTER__` in `#if`, `#elif` and `#line` counts with the text's uses in the serial order: before and after text uses, in two lines, in an inactive arm and an arm after the taken one, with the text's uses feeding an `#if`, a call over a directive, a header's uses. `#` or `##` of the value is refused by name. | `counters_in_directives_are_the_serial_passes` |
| R4 | The refusals of §2 by name. | `pragma_rows_are_the_serial_passes` |
| R5 | The evaluator's tests, the Metal gates, and the corpus slices unchanged: 3,135 units, none refused, at most 2 rounds of 2 launches. | the existing tests |

## Consequences

- A unit whose text makes no rows and whose lines count nothing runs the text pass once, as
  before, inside `pp_setup` instead of `px_expand`.
- A unit with rows, or with counters in its lines, runs the text pass once more than its rows'
  and counters' dependency depth, at most.

## Built

- Commit "0475: the text in P1's iterations, `_Pragma`'s rows, `__COUNTER__` in directives".
  Every test of R1–R5 passes under Relax. The corpus slices give 3,135 units equal to the serial
  pass, none refused, 2 launches a round.
