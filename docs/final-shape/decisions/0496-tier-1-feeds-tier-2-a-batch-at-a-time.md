# 0496: Tier 1 feeds tier 2 a batch at a time

Numbered 0493 where it was written.

Status: decided (the user, 2026-11-19: "tier 1 interleaved with tier 2 in batches, instead of two
whole-file passes"). Item 4 of the lean shape; follows 0489, 0490 and 0492.

## Intro

**The problem.** Tier 1 made every record of a file, and then tier 2 read them: two passes over
the whole file. The lean parser has one pass. Its lexer runs stage 1 over the file at once
(`Blocks::of`), but its token loop runs only as the parser needs tokens. The parser's `fill(k)`
asks `lex_batch`, which makes at least 16 tokens more (`lex_run`).

Since 0490, tier 1 has one forward pass and no back fill. A record is written once, at its token,
and never revised. So the token loop can stop after any token and resume there.

## The decision

1. **The token loop resumes.** `Tokens::run(src, m, batch)` starts where the last run stopped
   (`at`), and stops after `batch` records more, or at the file's end (`done`). `lex` runs it to
   the end.
2. **Tier 1 has a feed** (`native_cpu_block::Feed`, set by `CompiledBlock::set_feed`). Its three
   entries are:
   - `tier1_feed`: a file's stage 1, its masks and extracts written, no records yet;
   - `tier1_more`: the next batch of `FEED_BATCH` (16) records, written into the batch's
     positions; it returns the count made, with `FEED_DONE` (the file's counts are written then)
     and `FEED_REFUSED`;
   - `tier1_feed_free`.

   `Batch::call` makes each file's feed in place of its records.
3. **Tier 2 asks.** The generated tier 2 reads records only through `has(i)`. Where record `i` is
   not made yet, `has` asks `more` until it is, or until the file's records are all made. This is
   the lean parser's `fill(k)`. `TOKEN`, every token mark's run, and every gap pattern go through
   it. A refused feed refuses at the end of the records. `Ctx::more` and `Ctx::feed` carry the
   feed; they are zero where tier 1 made every record.
4. **The harness.** A tier 2 that reads only the records made, the JIT's, has them all made first
   (`Batch::fed_all`, `CompiledStream::asks_feed`). This includes the wider tier 2 that reads a
   file again. The tape's room for a fed file is that of a token a byte (`fed_bound`).
5. **Where it applies.** The AOT crate sets the feed where both tiers are the generated Rust. A
   file whose prefix is stripped (0389) is still lexed whole, because its positions move after the
   passes.

## Results

- kotlin_aot: the interleaved tapes are the JIT's on every file (508 files, 465 taken). Every
  other test is unchanged.

## Still to do

- **Copies.** The token loop writes its records into its own vectors, and `put_records` copies
  each batch into the harness's positions (`copy`, as the whole-file entry did before). Next: the
  loop writes into the positions.
- **The JIT.** The JIT's tier 1 and tier 2 do not feed yet: its tier 1 makes the records in its
  passes. The same entry shape (a token loop that resumes, and a `TOKEN` that asks) is the JIT's
  next step.
