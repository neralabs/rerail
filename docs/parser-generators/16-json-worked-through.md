# Parser generators: JSON worked through every level, before building

**Superseded by doc 18 (decision 0077):** the tree section is now a grammar, and tier 2 is its
state machine. The start and end rules here (`after`, `before`, `also`, `parent`, code facts)
and the dataflow built on them are gone; Metal's tier 2 is redone from the state machine
(doc 17 §6).

Status: worked example, for review (2026-11-07). Nothing here is built. It follows
`json.bitlex` from the Bitlex file to generated Cranelift and Metal. Doc 14 gave the design
and doc 15 the representation. Working the example through changed six things in them (§9).

**Corrected after review (doc 17 §0):** the scan phase has one barrier, not six; entries go
to device memory as arrays of words, written in coalesced segments; threadgroup memory is a
budget of about 9 KB, for occupancy; and there is no chunk limit. Where this page says
otherwise, doc 17 holds.

## Intro

The compiler has five levels. This page shows JSON's code at each one:

| § | level | what it is | exists? |
|---|---|---|---|
| 2 | Bitlex | the language, as written | yes, with one change |
| 3 | block program | tier-1 ops and tier-2 rows, printed by `rir` | yes, with two new ops |
| 4 | dataflow | every value as an op, with its class, scan and depth | no |
| 5 | plan | passes, scans, barriers, entries | no |
| 6 | CPU | the Cranelift function the plan lowers to | tier 1 only |
| 7 | Metal | the MSL kernel the plan lowers to | tier 1 only, in another shape |

§8 traces one small file through the Metal plan to its tape, row by row.

The code at §6 and §7 is what the generators would emit for JSON, written out by hand to fix
its shape. Names stand in for the generator's numbered values. Constants are JSON's.

## 1. The example file

```
{"a":[1,"x"]}
0123456789012          byte offsets 0 to 12
```

The masks of §3 for it, as sets of byte offsets:

| mask | bytes |
|---|---|
| `q`, unescaped quotes | 1 3 8 10 |
| `inside` | 1 2 8 9 |
| `opening`, `closing` | 1 8 / 3 10 |
| `content` | 2 9 |
| `code` (outside strings) | 0 4 5 6 7 11 12 |
| punctuation starts | 0 4 5 7 11 12 |
| word starts (number) | 6 |
| every token start, `T` | all 13 bytes |

Its tape has 19 nodes (§8).

## 2. Level 1: Bitlex

One change to `json.bitlex`. The pair is found from the token after its string, and the rule
from the left stays as a check:

```
# A key is a string whose next token is ':' (doc 14 §1): known from the bytes alone.
node pair start string when before ':'
  # The same pairs, found from the left. A target that uses one rule checks the other, and a
  # file where they differ goes to tree-sitter.
  also when after ('{' | ',') and parent object
  end with value
```

**Why both rules stay.** Metal uses `before`: it needs no stack, so the node count needs no
third pass over the bytes. The CPU uses `after … parent`: it has the stack at hand, and it
cannot see a backward carry in one forward loop (§6).

`before X` and `after X`, with `X` a set of one-byte kinds, read the next or previous token
outside strings. The compiler lowers them to byte ops (§3).

## 3. Level 2: the block program

Today's program, from `rir`'s dump of the compiled `json.bitlex` (the comments name the
Bitlex masks):

```
block 0 %0  = eq 0x5c                  # backslash
block 0 %1  = escaped %0               # escaped                    carried: borrow
block 0 %2  = eq 0x22
block 0 %3  = andnot %2, %1            # q
block 0 %4  = toggle %3                # inside                     carried: parity
block 0 %5  = and %3, %4               # opening
block 0 %6  = andnot %3, %4            # closing
block 0 %7  = andnot %4, %5            # body
block 0 %9  = andnot %8, %1            # esc
block 0 %10 = and %1, %7               # esc_last
block 0 %12 = andnot %11, %10          # content
block 0 %15 = andnot %14, %6           # code
block 0 %19 = andnot %17, %18          # word
block 0 %20 = prev %12, 1              # content, a byte before      carried: halo
block 0 %21 = andnot %12, %20          # starts(content)
block 0 %26 = and %15, %18             # punctuation starts (code & op)
block 0 %27 = prev %19, 1              # word, a byte before         carried: halo
block 0 %28 = andnot %19, %27          # starts(word)
block 0 %30 = andnot %27, %29          # lasts(word), a byte late (delay 1)
...                                    # 41 ops in all
```

**Built differently (doc 17 §1):** the block program keeps `before` and `after` as row
conditions, and the analysis lowers them to the values below. The one-byte kinds are read from
the token's byte, not tier-1 outputs. The ops below are the dataflow's, not the block
program's.

**Two new ops,** the lowering of `before` and `after`. Each carries one bit, the edge's
answer, so no `prev` is needed at the block's edge:

```
# bit p: the last bit of `set` before p is in `of`. Carry: the same, at the block's end.
%a = after  %set, %of
# bit p: the first bit of `set` after p is in `of`. Carry: the same, from the block's right.
%b = before %set, %of
```

`after` is one `fill`, as today's (`carry::fill`). `before` is the same on bit-reversed words
(`rbit` on arm64, `reverse_bits` in MSL).

**The tier-2 masks** that the tree section adds for JSON:

```
block 0 %41 = or %26, %28              # tokens outside strings: punctuation, words,
block 0 %42 = or %41, %5               #   and opening quotes ("U")
block 0 %43 = eq 0x3a
block 0 %44 = and %26, %43             # ':' tokens
block 0 %45 = before %42, %44          # the next U after the byte is ':'
block 0 %46 = and %5, %45              # keys: an opening quote whose string is followed by ':'
block 0 %47 = after %42, %44           # the U before the byte is ':'
block 0 %48 = and %42, %47             # values: a U right after ':'
block 0 %49 = eq 0x7b
block 0 %50 = and %26, %49             # '{'
block 0 %51 = eq 0x5b
block 0 %52 = and %26, %51             # '['
block 0 %53 = eq 0x7d
block 0 %54 = and %26, %53             # '}'
block 0 %55 = eq 0x5d
block 0 %56 = and %26, %55             # ']'
block 0 %57 = eq 0x2c
block 0 %58 = and %26, %57             # ','
```

**The rows** then read masks where they read kinds before. Keyword kinds stay for the tokens
whose kind needs their text (`true`, `false`, `null`, the escapes):

```
block 0 node 0 kind 15 start first end %37                    # document
block 0 node 1 kind 18 start %46 end with 2                    # pair
block 0         also after k1 k2 parent k17                    #   the other rule: a check
block 0 node 2 kind 17 start %50 end %54 nested 13             # object
block 0 node 3 kind 19 start %52 end %56 nested 13             # array
block 0 node 4 kind 20 start %5 end %6                         # string
block 0 node 5 kind 7 start %3 end %3 leaf                     # '"'
block 0 node 6 kind 8 start %38 end %39 leaf                   # string_content
block 0 node 7 kind 0 start %9 end %40 leaf                    # escape_sequence (keywords)
block 0 node 8 kind 0 start %26 end %26 leaf                   # punctuation (keywords)
block 0 node 9 kind 10 start %28 end %30 leaf check 10 json_number
block 0 code 4 field 1 when start r1                           # a pair's string: key
block 0 code 2 field 2 when %48                                # after ':': value
...
```

The rows still state rules. They are not run, and nothing interprets them. §4 turns them into
values, and §6 and §7 into code.

## 4. Level 3: the dataflow, and the analysis worked through

### 4.1 Every value

The analysis reads the block program and gives each value a class, a monoid if it is carried,
and a depth. JSON's values, in the order the analysis finds them:

| value | ops | class | monoid | depth |
|---|---|---|---|---:|
| byte classes, `eq` | `%0 %2 %16 %18 %43 %49 …` | local | | 0 |
| `escaped` | `%1` | forward | `fn`: borrow | 1 |
| `inside` | `%4` | forward | `xor`: parity | 1 |
| `q opening closing body esc content code word` | | local, given S1 | | 1 |
| `prev content`, `prev word` | `%20 %27` | halo, given S1 | | 1 |
| token starts and lasts, `T` | `%21 %28 %30 %38 …` | local, given S1 | | 1 |
| U, `':'`, brackets, `','` | `%42 %44 %50 … %58` | local, given S1 | | 1 |
| the next U is `':'` | `%45` | backward | `last 1` | 2 |
| the last U was `':'` | `%47` | forward | `last 1` | 2 |
| keys, values | `%46 %48` | local, given S2 | | 2 |
| the first token | | forward | `or` | 2 |
| the stack, with the pair bit | `nested`, `end with` | forward | `dyck` | 2 |
| nodes starting at each byte | from the rows (§4.3) | local, given S2 | | 2 |
| a node's `ord` | | forward | `add` | 3 |
| the last token start (`tail`) | | forward | `last 12` | 2 |
| an open's `ord`, at its close | | forward, per level | `last 32` × 27 | 4 |
| errors | | forward | `or` | any |

### 4.2 The analysis, step by step

1. **Find the carried ops,** and their monoids: `escaped` (a `fn` on 1 bit), `toggle`
   (`xor`), `before` and `after` (`last 1`), and the tier-2 rows' scans: `dyck` from
   `nested` and `end with`, `add` for `ord`, `or` for the first token.
2. **Merge small state functions that feed each other.** `escaped` feeds `toggle` through
   `q`. Both carries are 1 bit, so they merge into one scan, S1. Its state is 2 bits,
   `(e, s)`: escaped at the block's first byte, and inside a string.
3. **Find the halos.** `prev content` and `prev word` read the last byte of the block before.
   Those bits depend on S1 there, so they are S1's outputs too. S1 is a function from
   `(e, s)` in to `(e, s, content, word)` out: 4 inputs × 4 bits, 16 bits.
4. **Give each later scan its edge inputs.** The block summaries of `before`, `after`,
   `dyck`, `or` and `add` depend on earlier scans only through S1's state at the block's left
   edge, and the two halo bits:
   - `(e, s)`, 2 bits, change every mask after them. They become the **four candidates**.
   - The halo bits change only bit 0, where a token may or may not start. They enter the
     counts by adding 1, and `before` by one more copy of its 2 bits. They are stored as
     fixes, not as candidates.
5. **Order the scans by depth:**
   - S1, depth 1: `fn`;
   - S2, depth 2: `before` (backward), and `after`, `dyck`, `or`, `last 12` (forward);
   - S3, depth 3: `add` for `ord`.

   The node count at a block depends on keys, and so on `before`'s carry from the right. That
   is one bit, added: the block's last opening quote with no U after it is a key or not.
6. **Drop dead values.** Nothing in tier 2 reads the token count. It is not in the entry.
7. **Match closes to opens:** `match barrier`, so pass B gets its mid barrier (§7.5).

**With the rule from the left instead,** step 4 finds that the pair starting at a quote depends
on the stack's top. That is 38 bits, not an edge bit, so no candidates cover it. The node count
then waits for a pass over the bytes after S2, and the plan has three byte passes. The test of
doc 15 §7 pins both plans.

### 4.3 Nodes per byte, as bit planes

Several nodes start at one byte, outermost first: `document`, `object` and `'{'` at byte 0
of the example. The analysis reads the rows for the most nodes at one byte: 3 for JSON.
Then the count at each byte is two bit planes, `c0` and `c1`:

```
extra = D + K + (O | A) + S          # first token, key, '{' or '[', opening quote
count = 1 + extra                    # 1: the token itself; at most 3
c0, c1 = count's two bits            # a carry-save add, generated from the rows
```

A node's `ord` at byte `p` is then two popcounts, with no walk:

```
ord(p) = before + popcount(c0 & below(p)) + 2 * popcount(c1 & below(p))
```

## 5. Level 4: the plan

### 5.1 The entry, bit by bit

One entry per SIMD group, 2 KB of the file:

| field | monoid | bits | cand |
|---|---|---:|---|
| `s1`: `(e, s)` → `(e, s, content, word)` | `fn` | 16 | |
| `stack`: closes 4 + 13 kinds, opens 4 + 13 kinds, pair 2, error 2 | `dyck` | 38 | ×4 |
| `key`: `before` U ':' | `last 1` backward | 2 | ×4 |
| `value`: `after` U ':' | `last 1` forward | 2 | ×4 |
| `first`: a token seen | `or` | 1 | ×4 |
| `start`: the last token start, from the entry's start | `last 11` | 12 | ×4 |
| `nodes`: node starts | `add` | 14 | ×4 |
| fixes: an open key; bit 0 with halo `content`, `word` | | 5 | ×4 |
| `error` | `or` | 1 | ×4 |
| total | | 16 + 4 × 75 = 316 | |

Stored as 52 bytes: `s1` in 4, and per candidate a `ulong` (stack, key, value, first,
fixes, error) and a `uint` (start, nodes). 512 entries, 1 MB of file, take 26.6 KB of the
32 KB threadgroup memory.

After the scans each entry is overwritten in place with its exclusive prefix: the exact state
at its first byte, 28 bytes.

### 5.2 The plan, printed

```
plan json gpu
  threads 1024   simd 32   entry 2048   chunk 512 entries        # 1 MB per chunk
  entry 52 bytes: s1 fn 16 | 4 x (stack dyck 38, key last1< 2, value last1> 2, first or 1,
                                  start last 12, nodes add 14, fix 5, error or 1)
  tg: entries 26624, opens 3456, scan 256                          # 30336 of 32768 bytes

  chunk                                    # per 1 MB: the steps below, then the carry
    pass bytes A                           # every lane: tier 1 x4, summaries x4
      simd: s1 compose; halo shuffle; summaries combine x4
      writes entry
    barrier
    scan S1  fn         forward            reads entry.s1
    scan S2  dyck last1> or last12 forward, last1< backward      pick s1
    scan S3  add        forward                                  pick s1, fix key
      # each scan: SIMD shuffles, then the 32 SIMD totals in tg memory, 2 barriers
    barrier
    pass bytes B                           # exact carry-ins, then the tape
      B1 lanes: tier 1 once; tier-2 masks; c0 c1; simd prefixes
      B2 outputs: node j = l, l+32, ...: code, off, base           writes tape
      B3 lanes: opens and closes in order; in-lane closes: size, tail
      barrier                              # match barrier: unmatched opens per level
      B3 closes whose open is in an earlier lane: size, tail      writes tape
    barrier
    pass tape C                            # off relative to base; narrow; renumber codes
    carry s1 stack value first start nodes opens27 error           # in tg memory
    commit before an open key at the chunk's end
```

`rir` prints this from the program. The verifier checks it:

- **Every cross-thread read follows a barrier.** `scan S1` reads entries that other SIMD
  groups wrote in pass A: the barrier before it. Pass B reads the prefixes the scans wrote:
  the barrier before it. B3's second half reads the opens of other SIMD groups: its barrier.
  Pass C reads the tape that every thread wrote: its barrier. Removing any one fails.
- **Every loop with a SIMD operation or a barrier is uniform** (§7.3).
- **The threadgroup memory fits.** 30,336 bytes of 32,768.
- **The bytes are read in two passes,** A and B.

## 6. Level 5: the CPU

**Wrong, superseded by doc 18 and doc 17 §5.** Tier 2 on the CPU is the grammar's state machine,
lowered to the IR: one loop over the token stream, a branch on each token's kind in its state,
straight-line work. The sketch below steps the old rows.

The CPU plan is one pass. Every scan is a running carry, the stack is in registers, and keys
come from the rule from the left. Cranelift emits one function. In Rust, its shape is:

```rust
// Generated from json.bitlex. One call per file; no allocation; returns false to fall back.
fn json(bytes: &[u8], tape: &mut Tape) -> bool {
    // S1 and the halos: registers.
    let (mut e, mut s, mut h_content, mut h_word) = (0, 0, 0, 0);
    // The stack: 13 levels of kind bits and open ords, fixed slots in the frame.
    let (mut depth, mut kinds) = (0u32, 0u16);
    let mut open_ord = [0u32; 27];
    let mut pair_open = false;
    let (mut ord, mut last_u) = (0u32, NONE);       // `ord`; the last U token's kind
    for block in blocks(bytes) {
        // Tier 1: today's 178-instruction body, unchanged (decision 0071).
        let m = tier1(block, &mut e, &mut s, &mut h_content, &mut h_word);
        // Tier 2: one case per token kind, in byte order, over the token starts.
        let mut t = m.tokens;
        while t != 0 {
            let p = t.trailing_zeros();
            t &= t - 1;
            match token_case(&m, p) {               // a few mask bits at p pick the case
                Case::OpenBrace => {                // '{': object, then the token
                    if ord == 0 { tape.node(DOCUMENT, p) ; ord += 1; }
                    open_ord[depth] = ord;
                    tape.node(object_code(last_u), p); tape.node(LBRACE, p); ord += 2;
                    kinds &= !(1 << depth); depth += 1; pair_open = false;
                    if depth > 13 { return false; }
                }
                Case::OpenQuote => {                // a pair, if from the left it is one
                    let key = matches!(last_u, LBRACE | COMMA) && top_is_object(kinds, depth);
                    if key { pair_open = true; open_pair(&mut open_ord, depth, ord); }
                    tape.node(string_code(key, last_u), p); tape.node(QUOTE, p);
                    ord += 2 + key as u32;
                }
                Case::Colon => {                    // the check of the other rule
                    if !string_before_was_key() { return false; }
                    /* ... */
                }
                // ... one arm per kind: '}' ']' ',' closing quote, string_content,
                // escape_sequence (keywords), number (keywords, json_number)
            }
        }
    }
    depth == 0
}
```

Every arm is straight-line code with its constants inlined: codes, kinds, the 8-byte keyword
compares. `token_case` is a few bit tests of tier-1 masks at `p`. Nothing reads the rows at run
time.

## 7. Level 6: Metal

One kernel per program. One threadgroup of 1024 threads takes a run of files, up to a chunk,
and every step ends at a threadgroup barrier. No threadgroup waits on another. This replaces
the look-back of today's tier-1 kernel (`metal_block_program.rs`, `LOOK_BACK`,
`HALO_EXCHANGE`), which waits on other groups.

### 7.1 The kernel's frame

```metal
// Generated from json.bitlex (plan "json gpu"). 1024 threads: 32 SIMD groups of 32 lanes.
#include <metal_stdlib>
using namespace metal;

constant uint SG = 32, GROUPS = 32, BLOCK = 64, ENTRY = 2048, ENTRIES = 512;

struct Cand   { ulong w; uint counts; };       // §5.1; 12 bytes
struct Entry  { uint s1; Cand c[4]; };         // 52 bytes
struct Prefix { ulong stack; uint s1, last_start, nodes, flags, pad; };   // after the scans

kernel void json_tape(device const uchar* bytes [[buffer(0)]],
                      device const FileRow* files [[buffer(1)]],
                      device ushort* code_col [[buffer(2)]],       // the tape's columns
                      device uint* size_col [[buffer(3)]],
                      device uint* off_col [[buffer(4)]],
                      device uint* tail_col [[buffer(5)]],
                      device uint* base_col [[buffer(6)]],         // every 16th node
                      device atomic_uint* codes [[buffer(7)]],     // first use, per code
                      uint t [[thread_index_in_threadgroup]],
                      uint sg [[simdgroup_index_in_threadgroup]],
                      uint l [[thread_index_in_simdgroup]],
                      uint run [[threadgroup_position_in_grid]]) {
  threadgroup Entry entries[ENTRIES];
  threadgroup uint  opens[GROUPS][27];        // match barrier: one per SIMD group and level
  threadgroup ulong scratch[GROUPS];          // the scans' SIMD totals
  Carry carry = carry_for(files, run);        // a chunk's state, from the chunk before
  for (uint chunk = 0; chunk < chunks(files, run); chunk++) {
    pass_a(...);     threadgroup_barrier(mem_flags::mem_threadgroup);
    scans(...);      threadgroup_barrier(mem_flags::mem_threadgroup);
    pass_b(...);     threadgroup_barrier(mem_flags::mem_device);
    pass_c(...);     threadgroup_barrier(mem_flags::mem_device);
    carry = next_carry(...);
  }
}
```

### 7.2 The monoids, as generated helpers

Each monoid has a combine (`a` then `b`) and an identity. The generator emits them for the
plan's fields:

```metal
// S1: bits [4x + 0..4) are the outputs for input x = e | s << 1: e, s, content, word.
// Bit 16 marks the identity: it has no last byte, so no halos of its own.
constant uint S1_ID = 1u << 16;
static inline uint s1_out(uint f, uint x) { return (f >> (4u * x)) & 15u; }
static inline uint s1_then(uint a, uint b) {           // unrolled: 4 inputs
  if (a & S1_ID) return b;
  if (b & S1_ID) return a;
  uint r = 0u;
  for (uint x = 0u; x < 4u; x++) r |= s1_out(b, s1_out(a, x) & 3u) << (4u * x);
  return r;                                            // the halos are b's last byte's
}
// A state (e, s, content, word) through a prefix; the identity keeps the halos.
static inline uint s1_apply(uint f, uint st) { return (f & S1_ID) ? st : s1_out(f, st & 3u); }

// last 1, either direction: bit 1 "present", bit 0 the value. The later present one wins.
static inline uint last1_then(uint a, uint b) { return (b & 2u) ? b : a; }

// dyck: doc 14 §3, 38 bits. Opens and closes innermost first; kind 0 '{', 1 '['.
static inline ulong dyck_then(ulong a, ulong b) {
  const uint ao = (a >> 17) & 15u, bc = b & 15u, bo = (b >> 17) & 15u, ac = a & 15u;
  const uint m = min(ao, bc);
  const ulong ak = (a >> 21) & 0x1fff, bk = (b >> 4) & 0x1fff;
  const ulong mm = (1ul << m) - 1ul;
  ulong err = ((a | b) >> 36) & 3ul;
  err |= (((ak ^ bk) & mm) != 0ul) ? 1ul : 0ul;             // a close of the wrong kind
  const uint closes = ac + bc - m, opens = ao - m + bo;
  err |= (closes > 13u || opens > 13u) ? 2ul : 0ul;          // deeper than `max 13`
  const ulong ck = ((a >> 4) & 0x1fff) | ((bk >> m) << ac);
  const ulong ok = ((b >> 21) & ((1ul << bo) - 1ul)) | ((ak >> m) << bo);
  const ulong pair = pair_then(a, b, bc > 0u || bo > 0u);   // doc 14 §3's pair bits
  return ulong(min(closes, 15u)) | ((ck & 0x1fff) << 4) | (ulong(min(opens, 15u)) << 17)
       | ((ok & 0x1fff) << 21) | (pair << 34) | (err << 36);
}

static inline uint add_then(uint a, uint b) { return a + b; }
```

Each has a SIMD form over the 32 lanes, five shuffle steps (Hillis–Steele), as today's
`texclusive`. A 64-bit value goes through `simd_shuffle_up` as two `uint`s:

```metal
static inline ulong shuffle_up64(ulong x, ushort d) {
  return as_type<ulong>(simd_shuffle_up(as_type<uint2>(x), d));
}
static inline ulong simd_exclusive_dyck(ulong x, uint l, thread ulong& total) {
  for (ushort d = 1; d < 32; d <<= 1) {
    const ulong y = shuffle_up64(x, d);
    x = (l >= d) ? dyck_then(y, x) : x;
  }
  total = as_type<ulong>(simd_broadcast(as_type<uint2>(x), ushort(31)));
  const ulong e = shuffle_up64(x, 1);
  return l == 0u ? DYCK_ID : e;
}
```

### 7.3 Pass A: tier 1 four times, summaries four times

Each thread takes a 64-byte block. The loop strides over the chunk 64 KB at a time.

**Every loop with a SIMD operation or a barrier in it is uniform.** All threads run the same
iterations, and a thread past the end runs with `live` false and identities for its values.
A SIMD shuffle from an inactive lane, or a barrier some threads skip, is undefined on Metal.

```metal
static void pass_a(...) {
  for (uint b0 = 0u; b0 < blocks_in_chunk; b0 += 1024u) {
    const uint b = b0 + t;
    const bool live = b < blocks_in_chunk;
    const Block x = load_block(bytes, f, b, live);     // zeros when not live
    // Tier 1 for each S1 state at the block's start: the state-free part once, then the
    // part after `escaped` and `toggle`. The four differ only from those two ops on.
    const ulong bs = eq(x, '\\'), quote = eq(x, '"'), op = class_op(x), ws = class_ws(x);
    T1 m[4];
    for (uint x0 = 0u; x0 < 4u; x0++) m[x0] = tier1_after(bs, quote, op, ws, x0 & 1u, x0 >> 1);
    // S1 of the block: 16 bits, from the four.
    uint f = 0u;
    for (uint x0 = 0u; x0 < 4u; x0++) f |= m[x0].out << (4u * x0);
    // The SIMD group's prefix of S1: each lane's state as a function of the group's state.
    uint total;
    const uint pre = simd_exclusive_s1(f, l, total);
    // For each of the group's four states g: this lane's state, its halo bits from the lane
    // before, and its summaries.
    Cand c[4];
    for (uint g = 0u; g < 4u; g++) {
      const uint st = s1_apply(pre, g);                // (e, s, content, word) at the lane
      const T1 mm = m[st & 3u];
      // The halos: exact inside the group. At lane 0 they are the entry's fixes.
      const ulong starts = tokens_with_halo(mm, st >> 2);
      c[g] = live ? summaries(mm, starts) : CAND_ID;   // §7.4
      c[g] = simd_reduce_cands(c[g], l);               // the group's: dyck, last, or, add
    }
    if (l == 0u) entries[b / 32u] = Entry{ total, { c[0], c[1], c[2], c[3] } };
  }
}
```

### 7.4 A lane's summaries: bit operations, and one walk over the brackets

```metal
static inline Cand summaries(T1 m, ulong T) {
  // The tier-2 masks of §3, from tier 1.
  const ulong U = m.punct | m.w_start | m.opening;
  const ulong colon = m.punct & m.eq_colon;
  // before / after: the summaries only, `(present, value)` from the block's ends.
  const uint key = U ? (2u | uint((colon >> ctz(U)) & 1ul)) : 0u;            // first U
  const uint value = U ? (2u | uint((colon >> (63u - clz(U))) & 1ul)) : 0u;  // last U
  // Nodes starting here: c0, c1 from the rows (§4.3). Keys need the key bit from the right,
  // so the open key at the block's end is a fix, added after S2.
  const ulong K = m.opening & before_in_block(U, colon);   // keys known inside the block
  const ulong OA = m.punct & (m.eq_lbrace | m.eq_lbracket);
  // The document, at the file's first token, is added in S3 from `first`.
  const uint nodes = popcount(T) + popcount(K) + popcount(OA) + popcount(m.opening);
  // The stack: a walk over the brackets, the keys, the commas. Usually 0 to 4 steps.
  ulong st = DYCK_ID;
  ulong ev = OA | (m.punct & (m.eq_rbrace | m.eq_rbracket | m.eq_comma)) | K;
  while (ev != 0ul) {
    const uint p = ctz(ev);
    ev &= ev - 1ul;
    st = dyck_then(st, dyck_event(m, p));               // a one-event effect, generated
  }
  return pack(st, key, value, T != 0ul, last_start(T), nodes, fixes(m, U, colon));
}
```

The bracket walk is the one loop in pass A. It is bounded by 64, and it is over the brackets
only. simdjson's stage 2 walks every structural byte.

### 7.5 The scans

512 entries, one per thread in threads 0 to 511. Each scan is a SIMD scan, the 32 SIMD
totals in threadgroup memory, one SIMD group's scan of those, then the add back:

```metal
template <typename M>
static typename M::T tg_exclusive(typename M::T x, uint t, uint sg, uint l,
                                  threadgroup typename M::T* tot) {
  typename M::T s;
  const typename M::T e = M::simd_exclusive(x, l, s);
  if (l == 31u) tot[sg] = s;
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if (sg == 0u) tot[l] = M::simd_exclusive(tot[l], l, s);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  return M::then(tot[sg], e);
}

**The scans of one depth are one scan,** over the product of their monoids: a tuple combined
field by field is a monoid too. So S2's four forward fields are one `tg_exclusive`, and its
backward field runs in the same barrier pair, over the entries in reverse order.

```metal
struct S2f { ulong stack; uint value, first, start; };       // dyck, last 1, or, last 12
struct S2  { static S2f then(S2f a, S2f b); /* field by field */ };

static void scans(...) {
  const bool live = t < n_entries;                      // threads 512..1023: identities
  const Entry en = live ? entries[t] : ENTRY_ID;
  // S1: the state at each entry's first byte, from the chunk's carry.
  const uint s1 = s1_apply(tg_exclusive<S1>(en.s1, t, sg, l, scratch), carry.s1);
  // Pick the candidate, and apply the fixes for the halos at bit 0.
  const Cand c = fix_halo(en.c[s1 & 3u], s1 >> 2);
  // S2: four fields forward, one backward, in one barrier pair.
  const S2f s2 = tg_exclusive<S2>(S2f{ stack_of(c), value_of(c), first_of(c),
                                       start_abs(c, t) }, t, sg, l, scratch2);
  const uint key_r = tg_exclusive_backward<Last1>(key_of(c), t, sg, l, scratch1);
  // S3: node counts, with the open key fixed from the right, and the document.
  const uint here = nodes_of(c) + (open_key(c) & key_r & 1u)
                  + (first_of(c) & ~s2.first & ~carry.first & 1u);
  const uint nodes = tg_exclusive<Add>(here, t, sg, l, scratch1);
  threadgroup_barrier(mem_flags::mem_threadgroup);       // every entry read: overwrite
  if (live) prefixes[t] = Prefix{ dyck_then(carry.stack, s2.stack), s1,
                                  last_start(carry, s2), carry.nodes + nodes,
                                  flags(s2.value, key_r, s2.first | carry.first) };
}
```

So the scan phase has 6 barriers, two per depth, over at most 512 values each, and one more
before the entries are overwritten.

### 7.6 Pass B: exact carry-ins, then the tape

```metal
static void pass_b(...) {
  for (uint b0 = 0u; b0 < blocks_in_chunk; b0 += 1024u) {             // uniform, as pass A
    const uint b = b0 + t;
    const bool live = b < blocks_in_chunk;
    // B1, per lane: the entry's prefix, then the SIMD prefix, gives this lane's exact state.
    const Prefix pe = prefixes[min(b, blocks_in_chunk - 1u) / 32u];
    const T1 m0 = tier1(load_block(bytes, f, b), /* every state, once: */ lane_state(pe, l));
    const T2 m = tier2(m0, lane_key_from_right(pe, l), lane_value(pe, l), lane_first(pe, l));
    const ulong c0 = m.c0, c1 = m.c1;                   // nodes per byte, §4.3
    const uint n = live ? popcount(c0) + 2u * popcount(c1) : 0u;
    uint group_nodes;
    const uint my_first = pe.nodes + simd_prefix_exclusive_sum(n);
    group_nodes = simd_sum(n);

    // B2, per output: the SIMD group's nodes spread over its 32 threads, so consecutive
    // threads write consecutive tape rows.
    for (uint j0 = 0u; j0 < group_nodes; j0 += 32u) {               // uniform in the group
      const uint j = j0 + l;
      const bool out = j < group_nodes;
      const uint src = lane_of(j, my_first - pe.nodes, n);          // 5 shuffle steps
      const ulong sc0 = shuffle64(c0, src), sc1 = shuffle64(c1, src);
      const uint p = byte_of(j - first_of(src), sc0, sc1);          // 6 popcount steps
      const uint r = j - first_of(src) - rank_before(p, sc0, sc1);  // which node at p
      const uint rows = rows_at(shuffle_masks(m, src), p);          // D K O A S leaf bits
      const uint row = nth_set(rows, r);                            // outermost first
      const uint ord = pe.nodes + j;
      const uint code = code_of(row, shuffle_facts(m, src), p);     // §3's code rows, inlined
      const uint at = block_start(b - l + src) + p;
      if (!out) continue;                                           // after the shuffles
      code_col[ord] = ushort(code);
      off_col[ord] = at;                                            // absolute, for pass C
      if ((ord & 15u) == 0u) base_col[ord >> 4] = at;
      atomic_fetch_min_explicit(&codes[code], ord, memory_order_relaxed);
      if (is_leaf(row)) leaf_end(ord, row, m, p);   // a 1-byte leaf, or one ending here
    }

    // B3, per lane: opens and closes in byte order, matched by level. A close matches the
    // latest open at its level before it, so each lane needs, per level, its last open.
    uint last_open[27];                              // ords; 0 when none (ords grow)
    uint outside = 0u;                               // levels this lane closes from outside
    close_walk(m, my_first, lane_stack(pe, l), last_open, outside, size_col, tail_col);
    // The match barrier. Each SIMD group's latest open per level goes to threadgroup memory.
    // MSL has no prefix max, so inside the group it is 5 shuffle steps, and only for the
    // levels some lane closes from outside.
    for (uint v = 0u; v < 27u; v++) {
      const uint best = simd_max(last_open[v]);
      if (l == 0u) opens[sg][v] = best;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint need = simd_or(outside);
    uint before[27];                                 // filled for the needed levels only
    while (need != 0u) {
      const uint v = ctz(need);
      need &= need - 1u;
      uint x = simd_exclusive_max(last_open[v], l);  // earlier lanes of this group
      for (uint g = 0u; g < sg; g++) x = max(x, opens[g][v]);   // earlier SIMD groups
      before[v] = max(x, carried_open[v]);           // earlier iterations and chunks
    }
    close_outside(m, before, outside, size_col, tail_col);
    carried_open = latest_per_level(opens);          // for the next iteration
    threadgroup_barrier(mem_flags::mem_threadgroup); // `opens` is rewritten next iteration
  }
}
```

Every close writes the node it ends:

```
size = (nodes started at or before end) - ord        # the lane's running count
tail = end + 1 - (the last token start at or before end)
```

A leaf that ends in a later lane than it starts (a long string) is written by the lane where it
ends. No node starts inside a leaf, so its `ord` is the number of nodes before that lane,
minus 1.

### 7.7 Pass C: the tape

```metal
static void pass_c(...) {
  for (uint o = t; o < n_nodes; o += 1024u) {
    const uint off = off_col[o] - base_col[o >> 4];
    max_off = max(max_off, off);
    off_col[o] = off;
  }
  // A threadgroup max, a barrier, then the 2-byte narrowing of `off` and `tail` where the
  // maxima allow, and codes renumbered by first use if it differs from the guess.
}
```

## 8. The example, through the Metal plan

`{"a":[1,"x"]}` is 13 bytes: one lane, one entry. The entry's S1 is the identity on `e`, with
even parity: 4 quotes. The file's carry-in is `(0, 0)`, so S1 picks candidate 0.

B1's tier-2 masks, as byte sets:

| mask | bytes | why |
|---|---|---|
| U | 0 1 4 5 6 7 8 11 12 | punctuation, the number, opening quotes |
| `before` U ':' | 1 2 3 | the next U after them is byte 4 |
| K: keys | 1 | an opening quote in `before` |
| `after` U ':' & U: values | 5 | `[` follows `:` |
| D, O \| A, S | 0 / 0 5 / 1 8 | |
| `c0` | 0 1 2 3 4 6 7 9 10 11 12 | an odd count: 1 or 3 nodes |
| `c1` | 0 1 5 8 | 2 or 3 nodes |

So `ord(5) = 5 + 2 × 2 = 9` and `ord(8) = 7 + 2 × 3 = 13`. B2 then writes 19 rows, one
per thread, threads 0 to 18:

| ord | node | at | size | tail | code facts |
|---:|---|---:|---:|---:|---|
| 0 | document | 0 | 19 | 1 | |
| 1 | object | 0 | 18 | 1 | `_value` |
| 2 | `{` | 0 | 1 | 1 | |
| 3 | pair | 1 | 15 | 1 | |
| 4 | string | 1 | 4 | 1 | `key` (start of pair) |
| 5 | `"` | 1 | 1 | 1 | |
| 6 | string_content | 2 | 1 | 1 | |
| 7 | `"` | 3 | 1 | 1 | |
| 8 | `:` | 4 | 1 | 1 | |
| 9 | array | 5 | 9 | 1 | `value`, `_value` |
| 10 | `[` | 5 | 1 | 1 | |
| 11 | number | 6 | 1 | 1 | `_value` |
| 12 | `,` | 7 | 1 | 1 | |
| 13 | string | 8 | 4 | 1 | `_value` |
| 14 | `"` | 8 | 1 | 1 | |
| 15 | string_content | 9 | 1 | 1 | |
| 16 | `"` | 10 | 1 | 1 | |
| 17 | `]` | 11 | 1 | 1 | |
| 18 | `}` | 12 | 1 | 1 | |

The rows with `size` above 1 are B3's: its walk sees `{` (open, level 0), key `"` (pair,
level 0), `"` (string), `"` (close: size 8 − 4 = 4), `[` (level 1), `"` and `"` (13: size
17 − 13 = 4), `]` (size 18 − 9 = 9; the pair's value ends, so the pair: 18 − 3 = 15), and `}`
(size 19 − 1 = 18). The document ends at the last byte: 19 − 0 = 19.

## 9. What working it through changed

1. **The halos are S1's outputs.** `prev content` and `prev word` at a block's first byte
   depend on the state before it, so S1's function is 16 bits, not 4 (doc 14 §2). They change
   only bit 0, so they are fixes, and four candidates still hold.
2. **`before` and `after` are byte ops** with a 1-bit carry. Keys and the `value` field are
   then bit operations. The token-domain ops of doc 15 §1 (`last kind`, `next kind`) are not
   needed for JSON, and the kinds field of doc 14 §2 goes.
3. **The pair keeps both rules.** Metal uses the one from the right, and the CPU the one from
   the left. Each checks the other.
4. **Node counts per byte are bit planes,** `c0` and `c1`. `ord` is two popcounts, and pass B
   spreads its writes over the outputs (B2), as your step 5 has it. The walk is left for the
   sparse opens and closes (B3).
5. **Scans of one depth are one scan,** over the product of their monoids. So a depth costs
   one barrier pair, however many fields it has.
6. **Loops with SIMD operations or barriers are uniform,** with `live` flags. The plan's
   verifier checks it, as it checks the barriers.

The entry is 316 bits, 52 bytes as stored. The token count is dead, and S1 is wider.

## 10. Open, to measure when built

- **B2 spread over the outputs, or a walk per lane.** A spread node costs about 20 shuffles
  and 12 popcount steps, and its writes are coalesced. A walk costs less per node, but its
  lanes diverge and its writes scatter. Both can be emitted, so both can be timed.
- **Pass A runs the state-dependent part of tier 1 four times,** about 25 ops of 41.
- **B3's 27 levels are 27 registers** per lane.
