# 0634: Kotlin reads what 0633 left refused, and the node kinds the lean parser makes

Status: decided (2026-11-23, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0633.

## Intro

**The problem.** After 0633, the grammar refused some lines after a split `as` type. It also had no
rule for some constructs the lean parser reads:

- a split's line that starts with a string or a character (`"s" + a`): a token mark could not name
  the string's kind, `'"'`;
- a word whose operand may take the operator the trial stops at: `if (c) d == e`, `return a == b`;
- two generic calls on a line (`f<T>(a) + g<U>(b)`);
- a navigation after type arguments (`a<B>.c`, `a<B>::c`, `a<B>?.c`): refused everywhere, not only
  after a split;
- intersection and not-null types (`T & Any`, `(T) & Any`): refused everywhere;
- anonymous functions (`fun(x: Int) = x`, `fun Int.(x: Int) = …`, `suspend fun() {}`);
- a `when` entry's guard (`is A if c ->`).

A comparison of the node kinds the lean parser makes (`kinds.rs`) with the grammar's found the last
three. They are kinds the lean parser writes and the grammar never did (`not_nullable_type`,
`parenthesized_user_type`, `anonymous_function`, `guard_condition`).

## The decision

- **`kind "N"` in a token mark** names a token kind whose name is quoted (`kind '"'`, `kind "'"`). A
  quoted name alone is a word.
- **Split lines** (generated from one list, as in 0633):
  - The operand's first token may be a string or a character.
  - An `if` whose body is a statement is read in full, to the line's end. So is the statement after a
    block's `else`, and a jump's value (`return`, `throw`) with no assignment in it. The lean parser's
    `if_expression` and `jump_expression` read these with `expression()`, whatever the operator.
  - Up to two generic calls or navigations after type arguments.
  - Keep patterns:
    - an `if`, a `when` or a `try` that ends at a block, then an operator the trial stops at;
    - a generic navigation, then such an operator.
- **Generic navigation** is the lean parser's `navigation` with type arguments in its suffix:
  - `gen_nav`, the `generic` mark before a `.`, `?` or `::`;
  - `generic_nav`, a `navigation_suffix` that holds the `type_arguments`.

  `generic` holds before `(` or `{` only.
- **Not-null types**, as the lean parser's `ty_core`:
  - after a user type, `&` and a user type or a parenthesized user type (`not_null`);
  - a parenthesized type before `&` is a `parenthesized_user_type` (`paren_amp`).

  `type_core` and `as_core` both read them, so `as` and `is` do. `&` is a token kind of its own. The
  lean tape keeps neither the `&` nor the parenthesized user type's brackets (`implied`).
- **Anonymous functions**: `anonymous_function`, which is:
  - `['suspend'] 'fun'`;
  - a receiver's types and `.`s (implied);
  - the parameters, a type and a body.

  It is read where `anon_fn` holds:
  - at `fun (`, where the lean parser's `declaration_ahead` finds no declaration;
  - at a `fun` after a token that only an expression follows (`=`, `(`, `,`, `return`, …).

  `anon_sfn` holds at `suspend fun (`.
- **`when` guards**: `guard_condition`, `'if' expression`, after each condition.
  - In an entry with a guard (`w_guard`: an `if` before its `->`, brackets whole), the commas are
    leaves of the lean tape. tree-sitter's automaton keeps them there, and the lean writer follows it.
  - Elsewhere they are implied, as before.
- **The new marks are hidden** (`w_guard`, `anon_fn`, `anon_sfn`). A token mark holds at every token
  its pattern fits, and a mark that is not hidden refuses where no rule takes it.
- **The probe** prints the program's words where the grammar refuses
  (`RERAIL_KOTLIN_PROBE_EXPLAIN`).

## Why

The lean parser's functions are the criterion:

- `split_operand_reads`, `if_expression` and `jump_expression`: what a split's trial reads;
- `navigation`, `ty_core`, `anonymous_function` and `when_entry`: what the new rules read;
- `declaration_ahead`: where a `fun` is a declaration.

Each new split pattern is a subset of what the trial reads. Each keep pattern is a line where the
trial stops. Anything else is refused, never read differently.

Left refused:

- `suspend fun (` at a statement's start: the declaration's modifiers take `suspend`, where the lean
  parser reads an anonymous function;
- `suspend fun A.()` inside an expression;
- an anonymous function with a receiver after a token that `anon_fn` does not list.

## What changed

- **Bitlex**: `syntax.rs`, `kind "N"` among a token mark's choices, with a test.
- **`kotlin.bitlex`**:
  - the split marks, `as_open` and `as_kwn` (from the generator's lists);
  - `generic`, plus the new marks `gen_nav`, `paren_amp`, `anon_fn`, `anon_sfn` and `w_guard`;
  - the new rules `generic_nav`, `not_null`, `parenthesized_user_type`, `anonymous_function`,
    `guarded_conditions` and `guard_condition`;
  - `type_core`, `as_core`, `primary_tight` and `when_entry`;
  - `&` among the punctuation kinds;
  - the new `implied` lines and the hidden marks.
- **Regenerated**: `kotlin.answers`, plus `kotlin_aot`'s `tier1.rs` and `tier2.rs` (the new `&` kind
  shifts the token codes).
- **Tests**:
  - new: `split_lines_past_0633_are_the_lean_parsers`, `generic_navigations_are_the_lean_parsers`,
    `intersection_types_are_the_lean_parsers`, `anonymous_functions_are_the_lean_parsers` and
    `when_guards_are_the_lean_parsers`;
  - the refused lists: the string-first line and `f<A & B>()` leave them, and `suspend fun() = 1` at a
    statement's start joins them.
- **A wider corpus**: `lean-corpus/kt200` in the shared data, 200 files a repository (1,266 files).
