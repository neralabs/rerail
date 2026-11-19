# Parser generators: moving token-level checks out of tier 1

Status: 2026-11-09, plan. Step 1 done (§4). The payoff measured (§5): smaller than planned.

## Intro

Tier 1 runs once per 64-byte block, whatever the bytes are. Every Bitlex `mask` becomes a
64-bit word computed for every block. The `.rr` grammar has 832 such word operations, about
13 per byte. simdjson's first stage, which this design follows, has about one.

Most of those operations are not byte-level work at all. They are checks about one token,
written as masks over every byte:

```
# "This identifier is `fix`": true only at an identifier's first byte.
mask fix_w = id_first & c_f & next(c_i) & next(c_x, 2) & ~next(ident, 3)
# A mark tier 2 reads only at a token's first byte.
mark fix_on = fix_w & next(id_ahead, 3)
```

Tier 2 already visits every token, and already looks up an identifier's keyword. Done there,
`fix_w` costs a compare per identifier instead of eight word operations per block, and it
holds no register across tier 1's body. The profile's register pressure report shows those
values are a large share of what tier 1 spills.

simdjson draws the same line: stage 1 makes only the masks that need state carried across
bytes (strings, escapes, structure), and `true`, `false` and `null` are checked in stage 2,
once per atom.

This page plans how the compiler finds such checks, proves that moving them changes no
output, decides when moving pays, and moves them. Nothing is written by hand: it is all an
analysis of the block program the Bitlex file compiles to.

## 1. What can move

A value can move to tier 2 when three facts hold.

1. **It is local.** Its bit at a byte depends on a bounded window around that byte: byte
   tests at fixed offsets, and the words of a few carried values (a `fill`, a `toggle`, a
   fixed point) at fixed offsets. Those carried values stay in tier 1, as masks tier 2 reads.
2. **It is anchored.** Its bits can only be set at the first bytes of one token kind, at a
   fixed offset. This is proved from the program: `a & b` is anchored where either side is,
   `a | b` where both are, `prev(a, k)` where `a` is, shifted by `k`.
3. **Its readers read it only at those tokens.** A mark is read at token starts only. An
   escape or an error is read as "any bit in the file", which is the same as "any token
   where it is set". A mask that a `fill` or a `back_fill` reads is read at every byte: such
   a value stays.

Each fact is computed for every value of the program. Each is checked against the
interpreter: over the corpus, the anchored value's bits must fall on its anchor's bits. The
local value, evaluated at each bit from its leaves alone, must equal the interpreter's word.
That per-bit evaluation is also the meaning of the check that tier 2 will run.

## 2. When moving pays

Moving saves the tier 1 operations that nothing else uses (the value's exclusive cone). It
costs a check at each token of the anchor's kind. So the decision needs:

- the cone's operations, exclusive of what stays;
- the anchor kind's tokens per block, measured on the corpus;
- what the check costs in tier 2. A check that is "the token's text is this literal" costs
  nothing new: tier 2 already has the token's keyword. Recognizing that needs one more fact,
  about the token's extent. If the identifier's last byte is `ident & ~next(ident)`, then
  `~next(ident, 3)` at its first byte means "the token is 3 bytes long".

The cost model is tested on decisions with known answers. Its inputs are measured, not
guessed.

## 3. The steps

Each step's tests are written first, and fail until the step is done.

| Step | What | Tests |
|---|---|---|
| 1 | The facts: local cone and leaves, anchors, readers | exact facts on small grammars; every claim checked against the interpreter on the `.rr` corpus |
| 2 | Token extent: a token's run mask, offsets inside the token | exact facts; checked against the interpreter |
| 3 | Literal checks: a cone that is "the token's text is `w`" | `fix_w`, `suppress_w`, `agg_word`, …; non-literals refused |
| 4 | The cost model and the decision | decisions on small grammars; `.rr`'s decisions, with measured densities |
| 5 | Tier 2 runs a moved check: a mark or an escape from a keyword test | outputs equal before and after, every executor |
| 6 | Window checks that are not literals, and checks of the next token (`back_fill(…, solid)`) | as step 5 |

Later, and apart from this: values that are constants in a full block (`within`) are never
stored between passes; look-ahead reads the next block instead of delaying outputs.

## 4. Step 1: the facts

`token_local::facts` computes the three facts for every operation of a block program, given
the anchors (each token kind's first bytes). `token_local::check` tests every claim against
the interpreter's words. Every operation's word comes from `BlockProgram::all_values`, not
cut at the file's end.

- An anchored value's set bits must all fall on its anchor's, shifted.
- A local value, made from its leaves' words alone, must equal the interpreter's word.
- A leaf read at an offset its window does not hold is an error.

The tests (`token_local_tests`):

- exact windows, anchors and readers on small grammars. The windows include the `within`
  the compiler adds to clean a `~` at the file's end: it is a leaf of the position alone;
- the checker refuses wrong claims: a leaf dropped, a window moved, a false anchor, a wrong
  shift;
- the per-bit rule (`eval`, the meaning of tier 2's future check) equals the interpreter at
  every bit;
- on `.rr`: the keyword masks (`fix_w`, `suppress_w`, `options_w`, `over_w`, `input_w`,
  `output_w`, `agg_word`) are local and anchored on an identifier's first byte. Every claim
  holds on 55 corpus files (517 KB): 80 anchors and 680 local values, none wrong.

## 5. What can move on `.rr`: measured

The census (`rr_census`, run by hand) sorts tier 1's 832 operations by their facts:

| Operations | Count |
|---|---|
| carried (`fill`, `toggle`, counts, fixed points) | 83 |
| byte tests and positions | 69 |
| local and anchored on a token's first byte | 79 |
| local, anchored on no token | 601 |

A value's inner terms are often unanchored: in `agg_word`, only the final `& id_first` is
anchored, and its letter tests are not. So the movable work is the cones of anchored values,
not the anchored operations alone:

- **Movable now:** `fix_w`, `suppress_w`, `options_w`, `over_w` and their marks, about 90
  operations (11%). Their only carried reader is the mark's placement, which reads zero
  when the mark is on a token's first byte.
- **Movable with checks of the next token (step 6):** `agg_word`, `input_w` and `output_w`.
  Each feeds a `fill` that says "the token after this one" (`agg_span`, `io_run`). That
  brings the total to about 25%.
- **Not token-level:** the rest, about 75%. The regions, the depth count, the `\u`
  escapes, the two-byte punctuation, JSON's refusals, and the tree section's checks are byte
  work. They stay in tier 1.

The earlier estimate, tier 1 down to 50–100 operations, was wrong. The literal check of
step 3 is also weaker than planned. Tier 2 pairs a token's start with the next bit of a mask
of every kind's last bytes. Proving that `fix_w` equals "the identifier's keyword is `fix`"
also needs facts about the region masks (`code`, `hashword`), not only about the token's own
masks.
