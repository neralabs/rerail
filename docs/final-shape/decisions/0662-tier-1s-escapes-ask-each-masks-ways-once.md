# 0662: Tier 1's escapes ask each mask's ways once

Status: done (the user, 2026-11-23: "please turn to the 53 seconds outside the prover"). Follows 0654, whose
results left 53 s of a cold Kotlin compile (67 s) after the prover's tree section.

## Intro

**The problem.** Writing tier 1 as Rust (`bitlex_lex_rust::tier1`) took 55 s of Kotlin's compile, cold or warm:
a warm compile, every answer shipped, still took about a minute. The time was all in deciding where each
escape of a region holds: `region_escape` 46 s, `find_loop_fills` 12 s (spark1, `RERAIL_PROVING_TRACE`).

**Why.** Those checks ask a mask's *ways*: `Gen::cubes`, each way a set of bytes at offsets about the byte it
holds at, the mask's definition expanded to a disjunction. `in_region_of` walks a mask's expression, and at
every `&` asks both sides' bytes at nine offsets (`bytes_at`), each one a new `cubes` of the whole subtree.
`inside_shift` and `shift_before` ask it again for each byte of a shift's reach, each region end, each part.
Every call passed a fresh memo, so each made every name's ways again from its definition. A sample on the M1
Max: `cubes` 820 samples of the top frames, malloc and free, SipHash of `String`s, B-tree clones, and
`format!("{:?}")` of expressions compared as text.

## The decision

A mask's ways are a function of the definitions, the classes and the regions, and nothing else; so they are
kept on the `Gen` across calls, and made once.

- **Kept by name** (`cube_cache`) and **by expression** (`shape_cache`, `Expr` now `Eq + Hash`), as an
  `Rc<Cubes>` that readers share: a value read is not cloned.
- **Exact, not approximate.** `cubes` reads a name being read (a cycle) as its placeholder, "anything", so in a
  cycle of two names or more a name's ways depend on which of the cycle was asked first. Such names, and every
  name that reaches one, are *tangled* (`Gen::tangled`, the strongly connected components of the names' reads).
  A value that read a tangled name is not kept, nor one made inside a name's definition (it may read that
  name's placeholder). A carry that reads only itself meets its placeholder at the same place whoever asks, and
  is kept.
- **Forgotten when what it read changes.** `cubes` and `exact_bytes` note each name they look up. A name newly
  defined, made a class or made regional (`forget_name`) forgets every kept value if one of them looked it up
  or a definition reads it, and else none: a new name is no kept value's input. A change of the regions forgets
  all (`forget_cubes`).
- **The loops ask once what does not change in them.** `shift_before` asks whether a mask is none in a
  region's parts once per region and part set, not per byte and end; `inside_shift` makes the opener's and
  closer's bytes once, not per byte; `in_region_of`'s `&` takes each side's ways once for its nine offsets;
  `opener_bytes` keeps an opener's bytes by region and offset.
- **Comparisons by value.** `same` and `simplify`'s `a & a`, `a | a` compare `Expr`s with `==`, not their Debug
  text; region names are matched by prefix, not formatted.

## Results

Kotlin, spark1, `RERAIL_GPU=cpu`:

| | Before | After |
|---|---|---|
| tier 1 as Rust, warm | 55 s | 5.9 s |
| `region_escape` | 46 s | 4.4 s |
| `find_loop_fills` | 12 s | 1.0 s |
| a cold compile | 67 s | 20.4 s |
| a warm compile (shipped answers) | about 60 s | 7.3 s |
| the Kotlin and bitlex suite | 963 s | 172 s |

Tier 1's Rust is the same bytes (its digest compared with the caches off and on, over every step), and the
answers are the same bytes as `kotlin.answers`. The suite passes, 105 of 105.

**What is left.** Of a cold compile's 20 s, 13 s is the prover (0654). Of tier 1's 6 s, the most is still
`in_region_of` over `inside_shift`, hashing and comparing expressions as cache keys, and the B-tree maps of the
ways; an interned expression (an index, not a tree to hash) and ways as flat arrays would take most of that.
