# 0323: A region read in parts

Status: decided (2026-11-16). The grammar form, its meaning as masks and the
reference token loop have landed. Two CPU lowerings are being built to compare:
A in tier 1's token loop, B in tier 2's token fetch. Metal keeps the masks
(option C) for now. Part of 0177's token loop.

## Intro

**The problem.** Tier 1 on the CPU is a token loop (0177) only where a
grammar's tokens are told by rules a lexer ends them by: `run`, `pair`, and
`read`, which reads a region such as a string or a comment from its opener
to its closer. `read` makes the whole region one token. rr's strings are one
token, so that was enough for rr.

JSON's are not. tree-sitter-json's tree, and so the engine's JSON tape,
gives each string children: an `escape_sequence` for each `\x` and a
`string_content` for each run of other bytes. The ladder's JSON
(`compiler/bitlex_ladder/14-json.bitlex`) makes those tokens with masks: a
`toggle` for the string state and `start … last …` for each token. So its
tier 1 is the bit-parallel form, 2,393 instructions in the pass.

**The decision.** A region may be read in parts:

```text
regions hidden { str = q .. q }
token '"' = start quote & o_str read str parts { escape_sequence = esc bytes 2 } else string_content
```

The region's tokens are:
- its opener's byte and its closer's bytes, of the token's own kind (`'"'`);
- each part, `N` bytes from a bit of its mask (`esc`, two bytes);
- each run of the bytes between, of the content's kind (`string_content`).

A lexer reads a string with escapes exactly so.

**Its meaning, as masks.** The parser writes the parts' token rows after
the reader's (`Parts::rows` in `tools/bitlex`). With `f` the region's fill
and `body = f & ~opener`:

```text
closer:  c = prev(f) & ~f                  # its first byte; its last bytes-1 after
part P:  ps = E & body;   last prev(ps, N - 1)
content: x = body & ~(each part's bytes);   start x & ~prev(x), last x & ~next(x)
```

These rows have a rule of their own, `TokenRule::Inside`. They are ordinary
tokens to the bit-parallel lowering and to the reference evaluator, and
Metal's form is the masks. To the token loop they are the reader's: no
first byte ends them by a rule (`Lowering::inside`), so `string_content`,
which can start at almost any byte, does not push `{`, the digits and the
rest onto the mask rule.

**The reference.** `records::read_parts`: from the opener's byte on, the
nearer of the closer's next bit and each part's next bit. The bytes before
it are a content token, a part takes its bytes, and the closer ends the
region. `walk` emits each of these tokens, so the interpreter and every
check against it have the parts. The reader's parts are in the program's
text (`recpart`), so the digest covers them.

## A and B

Tier 2 sees the same tokens either way. The lowerings differ in where the
generated code splits a string:

- **A, tier 1.** The token loop's string arm walks the closer's and the
  parts' bits from the quote and writes a record for each token, as a lean
  lexer's string arm does.
- **B, tier 2.** Tier 1 reads the string as one token. Tier 2's token fetch
  splits it by the same bits, as simdjson's stage 2 walks a string.

Until A is built, the CPU builder refuses a reader with parts
(`RecordTables::new`), so no executor can give records that differ from
the reference.

## Checked

`json_parser/json.bitlex` is step 14 with its tokens in the rule forms.
By the interpreter it gives step 14's nodes, or refuses with it, on every
JSONTestSuite file and every corpus file under 1 MB: 129 parsed the same,
216 refused by both, none differ (`rr_parser`'s `json_loop_against_step_14`,
a probe until the JSON crate has its tests). Its end rules are `Run`
(words), `Read` (strings) and `Pair` (structural bytes): a token loop.
