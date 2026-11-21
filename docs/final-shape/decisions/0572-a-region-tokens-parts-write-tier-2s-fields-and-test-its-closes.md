# 0572: A region token's parts write tier 2's fields and test its closes

Status: decided (the user, 2026-11-21: "Next: the same shift rule inside a region's body, which covers the
raw-string and character escapes and most of tier 2's fields. Then the tails' closer tests, read from the token's
own region. After that, the walk stays only for the test-build asserts, under test-hooks.", the instruction "do
this"). The second part of 0551's step D. Follows 0566 (a shift outside the regions).

## Intro

**The problem.** After 0566, three kinds of reader still read masks the regions' walk made:
- the escapes in a region's body (the raw string's four quotes, the character's `\u`);
- tier 2's fields, the masks tier 2 reads at a record's bytes (a string's parts, its closers);
- the closes of a tail: which closer ends a template's tail (`dq_close & prev(f_tail)`).

Each is made of the regions and shifted. `prev(o_rstr)` in a raw string's body is the byte after its `"""`'s
first; at its closer it depends on where the closer is. So these were masks over the file, made after the walk.

The lean lexer knows each from the token it is reading: a raw string's body starts after its opener, a tail's
closer is its own region's.

## The decision

- **A region token's parts.** A mask's value in a token of region `k` is known at its parts:
  - its first byte (part 0);
  - its body's bytes 1 to 4 after the first, and from 5 on (`BODY_AT + q`, `BODY_LATE`);
  - its closer: for each byte of it, where the closer is 1 to 4 bytes after the token's first, or 5 on
    (`close_part(d, r)`). Part 2 is the closer where the same at each of its bytes, wherever it is.
- **Shifts inside a token** (`inside_shift`):
  - `prev(a, j)` at a part reads the part `j` bytes before: the body's at its offset, the first byte's, or,
    before the token, as before a byte outside (0566) with the opener's bytes known;
  - at a closer byte, the closer's byte before or the body's at its offset;
  - `prev` or `next` of every byte is every byte.
- **Tier 2's fields** (`plan_fields`, `FieldWrite`):
  - a field the same in every part of every region and outside them is that mask, from stage 1;
  - else the field starts as its value outside the regions, and each region token puts its parts over its bytes
    (`put`, `set`, `clear`, in the region's read arm), where they differ from the value outside;
  - stage 1 stores the value outside in the field's own vector as it makes it (`Node::also`), so nothing is
    copied.
- **A close is its value at its region's closer's first byte** (`closer_last`, by `closer_parts`). A close
  none there has no test.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The lean lexer reads a
raw string's parts from the token it is in, by where they are in it. A mask made after the walk asked, at every
byte of the file, which token the byte is in. Now the loop writes each field over the token it has read, from the
grammar's own parts.

## What changed

- `bitlex_lex_rust.rs`: `inside_shift` (part 0 by `shift_before`, the body's offsets, the closer's), `body_parts`,
  `close_part`/`closer_parts`, `FieldWrite`/`FieldPart`, `plan_fields`, `Node::also`, the `put` and `clear`
  helpers, `closer_last` by the closer's parts.
- `tier1.rs` regenerated. No longer read the walk:
  - the raw string's and the character's escapes;
  - tier 2's fields but `r_start` and `rd0_part1_last`;
  - the tails' closes.

Kotlin: 53/53 with the corpus.

## Not yet

- `r_start` and `rd0_part1_last` read `r_id = fill(prev(id_dollar), prev(id_last))`. Its fill in a body needs the
  run that ends at the opener's byte shown none.
- The walk stays for the test-build asserts.
