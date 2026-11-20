# 0406: Errors in one vector round the loop

Status: decided (the user, 2026-11-18: "deal with this", the per-block mask of the UTF-8
error that 0405 left). Follows 0347, 0454 (its rejected item 2) and 0405.

## Intro

**The problem.** After 0405, the UTF-8 error was made a 64-bit mask each block: four `and`s
with the bit weights, four `addp`s and a `mov`, about 9 instructions. Its word was then ORed
into the totals' running word, the file's flag. simdjson ORs its error bytes into one vector
register (`error`) and tests it once, at the end.

0454 had built the flag (page 39's item 2, kept as a patch) but found it fired on no grammar
here. A flag needs totals that no one reads by their rows (0347's `rowless_totals`), and those
were defined only where the CPU's token loop runs, which `14-json`'s tier 1 does not.

**That condition was stricter than the facts.** Tier 2 reads a total only as its file's flag
(`totals + t * stride`), never a row. Without a token loop, the rows are read only by tier 2's
masks and the marks.

## The decision

**1. Rowless totals without a token loop** (`BlockNative::rowless_totals`). A total no mask,
mark or token-loop read names is rowless wherever the CPU's tier 1 runs. Its row is not stored
(`forward`'s `unmade`).

- The comparisons with the interpreter leave out the rows the CPU does not store:
  `BlockNative::cpu_unstored` (`records_made` and the rowless totals), in the tests that compared
  against `records_made`. A total's flag is still compared.

**2. Each rowless total is ORed into the running word where the body makes it** (`Sinks::ors`).
A stored total had been stored where made, then loaded back at the block's end. An unstored one
would stay live to the end, three words in `14-json`, and the loop's bound spilled. One that the
edge part makes again (`fixed`) keeps the end's fold, which takes the edge part's word.

**3. The flag** (0454's patch, ported).

- A rowless total's tree of `or`s, and `and`s with a value all ones in a full block, is made as
  one word nonzero where any bit is.
- Its leaves on the vectors are ORed there and never made masks. Its word leaves are ORed as
  words.

Three changes made it fire:

- **A word leaf may be a region's value read past the region** (its `zero` list): the region's
  branch gives it as a word.
- **Bit logic of known lanes is known** (`full_words`): JSONC's root is
  `(utf8 | numbers) & (within & ~0)`. `~0` is the stripped prefix, and the lane analysis gave
  up on every lane value.
- **The ASCII branch** (`ascii_split`) made the UTF-8 error as a mask behind its branch. A flag
  leaf now leaves the branch ORed into the error vector instead.

**4. The error vector round the hot loop**, simdjson's `error` register (`At::err`).

- The hot body takes a vector parameter, zero from the loop's head.
- The flag's leaves are ORed into it as bytes, untested (`Lanes::any_bytes`, the rule bytes of
  0405).
- The ASCII branch's fast way passes it on, and its slow way ORs the UTF-8 bytes in.
- On an edge block the edge part passes on the vector the block took, and makes the flag's
  totals exactly, as words.
- Where the loop leaves for the head, the vector is reduced once, its two halves ORed, into the
  totals' running word.
- Not round the token loop, whose calls would spill it. There the flag is reduced each block.

**Found on the way:** `the_parse_path_is_compiled_code` (rr_parser) had failed since 0389. Its
`Batch::new` counted the stripped prefixes with an iterator chain on the parse path. The count
now goes in the loop over the files.

## Requirements, a test each

- R1. `14-json`'s and `15-jsonc`'s hot loops reduce one vector a block to a word, the ASCII
  test's: no error is made a mask or a word there
  (`the_hot_loop_carries_the_bytes_before_its_block`).
- R2. Tier 1 agrees with the interpreter on UTF-8 errors in full blocks, the carried vector's,
  and in the first and last blocks, the edge part's (the same test, now at offsets 20, 128,
  192 and 300 of five-block files). Dropping the reduction where the loop leaves fails it.
  Keeping the full body's vector on edge blocks is not observable here: past the file the bytes
  are zeros, which refuse where the interpreter's end check does.
- R3. The compiler suite (108), the parser_tests guards, the rr_parser suite (48), and the rr
  JSON set: 129 parse the same, 216 are refused by both, 0 differ.

## What changed

| | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| `14-json` after 0405 | 507 | 1 | 85 |
| Rowless totals, unstored | 495 | 1 | |
| ORed where made, the flag | 487 | 1 | |
| The error vector round the loop | **485** | **1** | **76** |
| `15-jsonc` after 0405 | 693 | 9 | |
| `15-jsonc` now | **667** | **11** | |
| simdjson | | | 72 |

JSONC's loop is integer-bound. Its stack accesses moved between 9 and 16 through these steps,
and settle 2 above 0405's.

What is left of the stretch:

- the ASCII way's `mov` of the vector;
- the chain of ORs: three, and one into the vector, where simdjson ORs each chunk's error into
  its register.
