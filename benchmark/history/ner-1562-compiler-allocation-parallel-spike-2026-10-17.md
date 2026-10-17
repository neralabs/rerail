# NER-1562 compiler allocation and parallel spike

## Purpose

This spike examines utility reuse, compiler allocation, clause parallelism, and regex construction.

Raw profiles and generated benchmark inputs remain under `/tmp`.

## Regex investigation

The inspected data-layer policy sources contain 69 regex sites and 64 unique regex strings.

The median regex length is 40 bytes. The 90th percentile is 93 bytes. The longest regex is 216 bytes.

These regexes are small. Their individual complexity does not explain multi-second compilation.

A release microbenchmark compiled the 64 unique regexes once in 9.2 milliseconds. It then compiled the same set 14,742 times.

The repeated run used 6.9 seconds of wall time. A second run used 8.0 seconds of wall time and 2.0 seconds of CPU.

The compiler caused this repetition in four places:

1. AST-rule validation compiled each regex.
2. Each utility-table pass compiled it again.
3. Each grammar dialect compiled it again.
4. Anchor planning parsed the regex again to extract required text.

The `regex` crate builds syntax trees, NFAs, literal automata, and strategy state. A simple regex is cheap once, but not 14,742 times.

## Context-independent artifacts

Regex programs now use one bounded process cache. The cache serves AST rules, parsed sub-rules, values, transforms, file conditions, and query regexes.

Anchor text probes have a separate bounded cache. Thus, required-text analysis does not parse the same regex for each query root.

ast-grep patterns now use a bounded cache keyed by grammar, context, and selector. `AstMatcher::Pattern` holds an `Arc<Pattern>`.

These caches reuse expensive leaves across different utility closures. Exact utility-graph equality is no longer necessary for regex or pattern reuse.

Each cache holds at most 4,096 entries. It clears before it accepts entry 4,097. Existing compiled rules retain their own shared objects.

## Utility-stage concurrency

Clause compilation now uses Rayon's indexed parallel iterator.

The compiler collects each result into its source position. It then reports errors and duplicate IDs in source order.

Each clause owns its required-capability set and LLM flag. The serial merge preserves output order.

The utility-stage cache now uses thread-safe maps and atomic counters. A per-key lock makes each cache miss single-flight.

Different utility stages can compile concurrently. Two clauses that request one stage do not compile it twice.

## Allocation changes

The utility cache previously serialized a complete utility closure into a temporary `String` and `Vec<u8>`.

Cache-key hashing now streams canonical JSON directly into SHA-256. The call salt is a fixed 64-byte digest.

The pattern cache uses a fixed 32-byte key. Cache hits do not copy the pattern source or pattern tree.

Regex cache hits do not allocate a new key. They clone the regex program's shared internal state.

A pre-fix sample of the large synthetic workload attributed 29.2 percent of active leaf samples to allocation and memory copies.

The remaining high-allocation structures are:

- Recursive diagnostic-path `String` values.
- `BTreeMap` and `BTreeSet` capture sets.
- Rebuilt composite `AstMatcher` vectors.
- Query-specific utility tables and index vectors.
- The parsed `serde_json::Value` policy tree.

## Synthetic benchmark

The controlled policy has 439 clauses and 439 shared utilities. Each utility has 20 regex terms from a set of 64 regexes.

The serialized policy is 378,159 bytes.

With one Rayon thread, median compilation was 132.5 milliseconds. With 16 threads, median compilation was 99.8 milliseconds.

Clause parallelism decreased wall time by 24.7 percent. CPU increased because this workload has small tasks and shared-cache synchronization.

A larger workload used 5,000 clauses, 5,000 utilities, and 50 regex terms per utility. Its policy size was 9,575,741 bytes.

Before text-probe reuse, it used 10.38 seconds of wall time and 7.01 CPU-seconds. After reuse, it used 3.57 seconds and 4.29 CPU-seconds.

This is a 65.6 percent wall-time decrease and a 38.8 percent CPU decrease.

The large workload retains 250,000 matcher terms. Its peak RSS was 2.08 GiB after compilation.

This retained representation now dominates memory. Temporary regex and pattern allocation is no longer the principal cause.

## Remaining architecture work

The compiler now shares regex and pattern artifacts across different utility closures. It also compiles equal complete utility stages once.

Composite utility matchers still embed one query-specific `UtilTable`. Two different closures still rebuild their composite vectors and utility references.

A complete component cache needs a relocatable utility IR. The IR must name dependencies by stable utility identity instead of table index.

The query linker can then assign table indexes without rebuilding structural matcher nodes. Recursive components need one SCC artifact and a separate link table.

The next allocation targets are path interning, indexed capture sets, and arena-backed temporary analysis data.
