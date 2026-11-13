# 0075: A text test's automata are compiled once, and kept in the store

Status: built (2026-11-07)

## Intro

A rule's text tests (decision 0037) are plans over strings: exact sets, prefixes, suffixes, substrings, and
automata for the regular expressions that are not literals. The vocabulary engine runs the automata in groups of
16, one multi-pattern DFA per group, each string asked once which patterns match.

Decision 0073 kept a prepared program in the store with its text tests as specs, and named what it did not do:
the matchers were compiled again in every process. This record keeps them too. A group's dense DFA is stored by
what it is a function of, and a later process reads its bytes instead of determinizing.

```
plans ──► groups of 16 automata ──► key (patterns, regex-automata version, limit, endianness)
                                         │
            in the store? ── yes ──► read the DFA's bytes (from_bytes)
                 │ no
                 ▼
            determinize (dense, MatchKind::All, ≤ 4 MB) ──► store it ── too large ──► lazy DFA, stored as "too large"
```

## Decision

### 1. Compiled once per process, before the run

`Vocabulary` compiles its searchers once (`Vocabulary::compile`, kept in a `OnceLock` shared by `again()`), in the
stats phase's span "compile the text tests". Before, they were made inside the first `rows()` call. The run's
profile says what compiling did: "text tests compiled (decision 0075): groups of automata: N read from the store, N
determinized, N on the lazy DFA".

### 2. A group is a dense DFA, or the lazy one

`text::vocabulary::Searchers` builds each group as one dense DFA (`regex_automata::dfa::dense`, `MatchKind::All`,
unanchored starts), within `DFA_LIMIT` (4 MB). A group that does not fit, or that the dense builder refuses (a
Unicode word boundary `\b`, which it does not determinize), runs on the lazy DFA (`meta::Regex`), as every group did
before. A search the dense DFA gives up on is asked of the lazy one. The result is the same set of matches either way. The test
`searchers_read_from_the_cache_match_as_when_built` checks this against `regex::bytes` on random and non-ASCII text.

### 3. The store keeps it as a facts-memo entry

`corpus::prepared::StoredDfas` implements `DfaCache` over the content store, by the mechanism decision 0073 uses:

- **Source:** the group's key, a sha256 of `rerail.dfa/v1`, the `regex-automata` version, the limit, the
  endianness and each pattern's HIR, stored as a source (`memo::source`).
- **Schema:** `DFAS = 103`, under the plan `rerail/dfa`. The path is empty: the entry is content-pure.
- **Bytes:** a tag byte, then the DFA's `to_bytes_native_endian` bytes; tag 0 records a group that does not
  determinize, so the next process does not try again.

Reading copies the bytes into an aligned `u32` buffer and calls `DFA::from_bytes`, which checks them. An entry that
does not read is built again and stored over.

### 4. The run's manifest roots what the program is stored under

No file of the corpus is the source of a stored program: not its text, not its facts document (decision 0074), not
a group's key. A collection pass between two processes took the program text's source, and with it the pointer, so
the next process prepared again. The run's manifest (decision 0053) now names these sources with the files', so
collection keeps them as long as it keeps the run. `a_prepared_program_is_read_from_the_store` failed without this
once the second run lease went (O108).

## Measured

On the 30 Python pack rules whose text tests have automata, each `run-rr` was a fresh process over
`fixtures/python/calls/project`, three passes on one fresh store. CPU is user plus system, at load 64 to 73:

| pass | "compile the text tests", all 30 | process CPU, all 30 |
|---|---|---|
| 1: determinized and stored | 12.5 ms | 60.5 s |
| 2: read from the store | 11.1 ms | 58.0 s |
| 3: read from the store | 11.0 ms | 57.2 s |

The pack's groups are small: half a millisecond to determinize one, about 0.3 ms to read it back. Pass 1 also
prepared and stored each program (decision 0074), so its process CPU is not compiling alone. Groups are shared
between rules: in one pass over the pack, 18 of 30 rules read a group another rule had stored.

A larger group shows what the store saves (`determinizing_a_group_costs_more_than_reading_it`, an ignored
measurement): 16 word-shaped ASCII patterns (`(?i)select\s+\w+\s+from`, `[A-Z][a-z]{3,12}[0-9]{1,4}`, …). Trying to
determinize them took 0.16 s of thread CPU (three runs, 0.159 to 0.163 s, load 100 to 135) and gave a DFA over the
limit, so the group runs lazily. Reading the stored "does not determinize" took 0.2 ms. Without the entry, every
process would pay the 0.16 s again. The same 16 patterns with Unicode classes are refused by the dense builder at
once (2.8 ms).

The cache matters for large groups, and for the regex.md §1.1 case of many patterns. For the pack as it is, the
win is that compiling happens once, outside the run, and is visible in the profile.

## What this does not do yet

- **Literal searchers are built per process.** Aho-Corasick over a plan's literals is fast to build and is not
  stored.
- **The groups are fixed at 16, in plan order.** Grouping by what determinizes well together would make fewer
  groups fall back to the lazy DFA.

## Prior work consulted

- **Lessons ledger 25:** restoring a stored derivation beats redoing it. Here the derivation is cheap for the
  pack's patterns, and the measurement says so.
- **ast-index-spike history index L919-920:** key memoised analysis by what it is a function of. The key names the
  patterns, the regex library's version, the size limit and the byte order, not the rule.
- **ast-index-spike design index S112 (the compiled artifact):** a compiled query is an artifact of its own. A
  group's DFA is one, shared by every rule whose tests make that group.
- **regex.md §1.1 (decision 0037):** one automaton for every pattern blows up (639 patterns, 676 s). Groups of 16
  stay small, and each is determinized once.
- **Decisions 0037, 0052, 0073 and 0074:** text tests, the facts memo, the stored prepared program, and its form.
