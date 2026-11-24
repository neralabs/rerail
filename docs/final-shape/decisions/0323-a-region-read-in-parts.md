# 0323: A region read in parts

Status: built (audited 2026-11-23). Decided (2026-11-16). The grammar form, its meaning as masks and the reference
token loop have landed. Both CPU lowerings have landed: A in tier 1's token loop, and B in tier 2's token fetch.
Either is a compile option of the same grammar (`Records::split_parts_in`). B took 18% fewer cycles over JSON's
corpus. Metal keeps the masks (option C) for now. Part of 0177's token loop.

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

### A, as built

`rule_last`'s reader arm ends the token at the opener's byte when its reader
has parts. The tail of the loop writes the opener's record, then
`read_in_parts` walks the rest of the region:
- from the byte after the opener, a word at a time, it ORs the closer's word
  and each part's word, masked from where it stands;
- at the lowest bit, the bytes before it are a content token;
- a part's bit there makes a part token, and the walk goes on after its
  bytes;
- the closer's bit makes the closer token, which wins where both hold, as
  `read_parts` has it.

Each record's code is `records::pass`'s: its first byte's kind (the tables'
first 256 bytes) and its word (`word_hit`, the lookup the tail uses, now one
function). JSON needs the word: an `escape_sequence` takes its kind from its
keyword (`required`). The loop goes on from the byte after the closer, with
the cursor past the region's records. No closer in the file refuses the
file, as `read_parts`' `Err` does.

The walk's tokens are one turn each (`Step::Token`), and its word steps are
`Step::Word`. No loop steps a byte.

In the string rule's own tail (a rare class, `Side::Rare`), the walk only
tests the opener's bit. Where a tail is shared by several rules
(`Side::Both`), it also tests the token's first byte against the rule's.

`RecordTables::new` still refuses what A does not give a part:
- more than one reader with parts, or one with more than one region;
- marks, retags or gap marks;
- a token test of the reader's rule;
- words past 16 bytes.

JSON has none of these.

### B, as built

The choice is on the reader (`Reader::split`, `Split::Tier1` or
`Split::Tier2`), in the program's text (`recsplit K tier2`), so the digest
covers it. Under B, tier 1 reads the region as one token, as a reader
without parts does: its record runs from the opener to the closer.

Tier 2's `TOKEN` splits a record that is a whole region: one at its
opener's bit and longer than that byte. Its token is the opener's byte, and
its parts are pending in two state fields, `PartAt` (where the next part
starts) and `PartEnd` (the closer's last byte). Each `TOKEN` then takes the
next part before the next record (`split_next`):
- at the closer's byte, the closer, and the region is done;
- else the nearest part bit before the closer, a word at a time;
- a part there takes its bytes;
- else the bytes up to it, or up to the closer, are content.

Codes are `plain_code`'s, the same function as A's walk. Tier 2 reads the
masks as tier 1 left them. Where tier 1 makes an output from the bytes
(`native_cpu_bytes`), the CPU stores none of it, so tier 2 makes it from the
bytes too (`output_word`), as tier 2's checks do.

A record already split is a token as it is: tier 1's under A, and Metal's
records from the masks. So B's tier 2 is right over either.

A file gives more tokens than records under B. So the tape's room is sized
by `CompiledStream::tokens`: the records plus the file's bytes, since each
part is at least one byte.

Under B, tier 2 refuses:
- marks, retags or gap marks;
- words past 16 bytes;
- two such readers;
- a `TOKEN` whose taking is flagged;
- tier 2 ahead of time (`rust_stream`), which is not built yet.

The interpreter's `TOKEN` (`stream::run`) splits the same way, with
`records::read_parts` and `records::pass`.

## A against B

JSON's corpus files under 1 MB that tier 2's tape holds: 12 files, 1,345,779
bytes. `rr_parser`'s `json_a_against_b`, the least of 15 rounds. The machine
was loaded (load average 32 to 39), so the cycles vary by about 10% between
runs. The instruction counts are stable.

| | Tier 1 | Tier 2 | Both |
|---|---|---|---|
| A, instructions a byte | 62.3 | 23.0 | 85.3 |
| B, instructions a byte | 43.1 | 34.3 | 77.5 |
| A, cycles a byte | 11.3 to 12.5 | 3.6 to 4.2 | 14.9 to 16.7 |
| B, cycles a byte | 7.5 to 8.3 | 4.7 to 5.5 | 12.2 to 13.7 |

B writes and reads 353,244 fewer records over JSONTestSuite and the corpus
together, and splits each string where tier 2 consumes it. A's walk writes
a record a part that tier 2 then reads back. B is also the lowering that
keeps Metal's tier 1 as it is.

Both tiers are far from simdjson's: 43 to 62 instructions a byte in tier 1
alone. That is the next measure, by pass.

## Checked

`json_parser/json.bitlex` is step 14 with its tokens in the rule forms.
By the interpreter it gives step 14's nodes, or refuses with it, on every
JSONTestSuite file and every corpus file under 1 MB: 129 parsed the same,
216 refused by both, none differ (`rr_parser`'s `json_loop_against_step_14`,
a probe until the JSON crate has its tests). Its end rules are `Run`
(words), `Read` (strings) and `Pair` (structural bytes): a token loop.

On the CPU (A), `rr_parser`'s `jsons_cpu_records_are_the_interpreters`
compares the token loop's starts, lasts, codes and totals with the
interpreter's. Its inputs are strings with escapes at each offset around a
block's edge, strings over blocks, strings open at the file's end, and
JSON's files: 759 files, 1,165,206 records, all the same.

Under B, `jsons_tier2_split_is_tier1s` checks three things:
- the interpreter's nodes equal A's on every input;
- Cranelift's tier 2 under B equals A's interpreter over the CPU's tier 1
  under B, and over A's records;
- the whole tapes by rules (rr's cut, `take_token` outlined) are the same
  bytes under A and under B.

That is 386 files, 288 that both tier 1s accept. Four of them are past the
tape's 16-bit widths: A's tier 2 refuses those too.
