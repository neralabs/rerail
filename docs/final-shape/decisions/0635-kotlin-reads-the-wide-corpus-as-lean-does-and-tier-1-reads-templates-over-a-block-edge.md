# 0635: Kotlin reads the wide corpus as the lean parser does, and the JIT's tier 1 reads templates over a block edge

Status: decided (2026-11-23, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0634.

## Intro

**The problem.** 0634 added a wider corpus: `lean-corpus/kt200`, 200 files from each repository,
1,266 files. Run against it, the grammar:

- refused 14 files the lean parser reads;
- read 5 differently;
- read 2 that the lean parser refuses.

The causes were these:

- `get(…)` and `set(…)` as calls, and an accessor after an annotation (`@JvmName("a") get() = 1`);
- an annotation whose arguments hold `::class` before an expression (`@OptIn(A::class) return x`)
  or before a constructor on the next line;
- a setter's annotated parameter (`set(@A v)`) and a backtick name as a named argument;
- an enum body's `;` and `,` leaves, and the empty `{ ; }`;
- `\$` in a line string: before another `$`, and before the quote;
- `String ?` with a blank before the `?`;
- a receiver with a `.` in its type arguments (`fun A<C.D>.e()`), which the lean parser refuses;
- a block's statements after its last `;`: the lean parser ends them at the `;`, and the grammar
  ran them on over the comments after it;
- one file whose tokens the JIT's tier 1 read differently from the interpreter's (a pkl test).

## The decision

- **Accessors** (`acc`) follow the lean parser's `accessor_ahead`. `get` or `set` is an accessor
  at a line's end, before `;` or `}`, or before an accessor's own shapes:
  - `get() =`, `{` or `:`;
  - `set(name) …` and `set(name: type) …`, with annotations on the parameter.

  Elsewhere `get` and `set` are names: `get(1).x` is a call. After an annotation on its line, `acc`
  holds too.
- **A `before`'s first element may have `with` and `without`**, as the mark's own tokens may
  (`syntax.rs`).
- **Annotations before an expression** (`ann_expr`) and before a constructor (`head_ctor`) read
  their arguments' brackets whole, so a `class` inside them (`A::class`) does not end them. An
  annotation before an accessor is the declaration's, not an expression's.
- **A setter's parameter** takes modifiers. **A named argument's name** may be a backtick name.
- **Nullable types**: a type's `?` touches the token before it (`nq`), as the lean parser's
  `ty_core` asks.
- **`\$` in a line string**: a `$` right after a `\$` starts no template and no name
  (`esc_dollar`), as the lean parser's reader passes the byte after the escape. Two cases are left
  refused:
  - a `\$` before the string's quote: the lean parser ends the string there and writes no content
    before it;
  - a `\$` before a backslash, which then escapes nothing.
- **Enum bodies**: `enum_k` holds at `, ;` and at `; }`. The rules `enum_comma`, `enum_tail`,
  `enum_members`, `enum_end` and `enum_empty` keep the leaves the lean tape keeps.
- **Receivers** with a `.` among their type arguments are refused (`recv`'s `unless`), as the lean
  parser's `receiver_end` refuses them.
- **`semi_close` holds after no `;`.** Tree-sitter's scanner inserts no semicolon after one, so a
  block's statements end at their last `;`, and the comments after it are the block's.
- **Each class member ends with a semicolon** (`member_end`: `semi`, `;` or `semi_close`), as the
  lean parser's `members` asks. So `class A { val x = 1 }` is refused, as the lean parser refuses
  it. Its `;` is implied, as before.
- **The JIT's tier 1**: the stack of open template records is kept across a wait. A part's token
  loop waits where a token reads past the blocks made. The next part then reads again every token
  since the loop's head.
  - The stack had no undo for a `}` that resumes a string: a template's close drops what it pushed
    when it is read again (`later`), but a resume does not.
  - So a string in a template's code whose own `${…}` ended in the next block was read twice. The
    second time, the `}` patched the outer string's record, and the file was refused.

  Now the first flagged token after a head copies the stack (`OPEN_COPY`, `OPEN_COPIED`), and a
  wait puts the copy back. The next part starts from the stack as the head had it.

## Why

The lean parser's functions are the criterion:

- `accessor_ahead`, `declaration_ahead`, `ty_core`, `receiver_end` and `members`;
- the `string_literal` reader;
- `asi`, which gives no semicolon after a `;`.

The JIT's tier 1 must give the interpreter's records. The wait is that tier's own mechanism, so
its state must replay the same. A copy taken only by the cold flagged path keeps the token loop's
common path to one store a head, and only in a program whose strings have templates.

Left refused:

- `$$"…"` strings, whose templates take as many `$`s as their prefix;
- a `\$` before a line string's quote, or before a backslash.

## What changed

- **Bitlex**: `syntax.rs`, `with` and `without` on a `before`'s first element.
- **The compiler**: `native_cpu_block.rs`:
  - `open_slot` holds the stack's copy;
  - `record_tokens`' head clears it;
  - the flagged path copies the stack before its first change;
  - the wait restores it.
- **`kotlin.bitlex`**:
  - the marks `acc`, `named`, `ann_expr`, `head_ctor`, `recv`, `semi_close` and `enum_k`, and the
    new `nq`;
  - the masks `esc_dollar`, `t_dollar` and `s_dollar`, and the escape for `\$` before a quote or a
    backslash;
  - the rules `parameter_with_optional_type`, `enum_class_body`, `class_body`, `member_end` and the
    enum's new choices, and every nullable type's `?`;
  - the `implied` lines for the enums and `member_end`.
- **Regenerated**: `kotlin.answers`, plus `kotlin_aot`'s `tier1.rs` and `tier2.rs`.
- **Tests**: `the_wide_corpus_lines_are_the_lean_parsers`, and
  `templates_over_a_block_edge_are_the_lean_parsers` (three templates at each of 64 offsets).
