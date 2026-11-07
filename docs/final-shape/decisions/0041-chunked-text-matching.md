# 0041: Text matching is chunk-at-a-time and per file, never per node per byte

Status: accepted (user, 2026-11-04); host part done, device part open (O66)
Date: 2026-11-04

## Question
serverplan_quality on 200 data-layer files spent 4.3 s of a 7.1 s cold audit confirming walk text
tests, and a module's text failed a regex slot's NFA work limit, which aborted the audit
(`WorkLimit`). Why are text tests a byte-at-a-time NFA, run on every node's own text?

- A lowered text test is a `ByteNfa` value program (t6 §7, Tempo's regex lowering): one control
  flow for the CPU and Metal, stepping one byte per NFA state with a per-path work budget.
- 0017 made a walk's text test a slot of bits computed on the host before dispatch, one node at a
  time over the slot's domain (the kind tested before it, else every node). The host searched with
  a `regex_automata` DFA only under a length bound and fell back to the NFA above it.
- A test of no kind reads every node's text, and nested spans read each byte once per ancestor:
  O(bytes × depth) per regex per file.

## Decision (user, 2026-11-04)
"GPUs can match workgroup-sized chunks of a string just as fast as single characters; we should not
be slow in string matching; chunk-at-a-time NFAs/DFAs should be the default."

1. **Chunk-at-a-time automata are the default** on every backend. A byte-at-a-time NFA per node is
   not an acceptable default.
   - **Host:** `regex_automata` (lazy DFA with SIMD literal prefilters) and `memchr`, with no work
     limit: they are linear in the text, and W runs the same regex crate unbounded.
   - **Device (O66, open):** each workgroup takes a chunk of the file. A lane per start state
     computes the chunk's state map, the maps are composed by a prefix scan, and the matches are
     marked. That is a DFA's transition function per chunk, or an NFA's as state bitsets. The work
     is bounded by bytes, not by paths.
2. **A text test reads the file, not each node.**
   - A test with a finite set of prefix literals (at most 64, none empty) searches the file once
     for them. Only nodes whose span holds an occurrence start are confirmed. Every match starts
     with one of them, so this is exact.
   - Terminal text stays on the vocabulary (§16.1, 0026): one test per distinct term, then postings.
3. **A host computation never aborts an audit on a work budget.** Budgets are device mechanics.

## Done (host)
- `Micro::scan` (`transducer/packed/compile/micro.rs`):
  - the DFA is used at every length;
  - prefix literals are found by one `memmem` pass per file (`prefixes`);
  - only nodes whose span holds an occurrence are confirmed.
  - On 200 files: 2,392 of 2,491 slot scans take the literal pass.
- `prefilter::terms`: a slot with no kind is not confirmed on every uncovered node. The walk's scan
  answers it.
- Measured, serverplan_quality, 200 data-layer files, cold, CPU (load ~30):

  | | before | after |
  |---|---|---|
  | `demand.texts` | 4.3 s | 0.3 s |
  | audit aborted by `WorkLimit` | yes, with `nthChild` lowered | no |

## Prior work consulted
- **0017:** *Kept.* Text tests are slots of bits computed once per (value, kind). This decision is
  about how a slot is computed.
- **§16 (03-architecture):** *Agrees and extends.* Vocabulary first, and whole-file text only where
  needed. §16.4's "a DFA on the node's bytes" becomes "the file's bytes, chunked".
- **PIAT `research/regex.md` §2.2–2.3:** a RegexSet per name is slower than one regex at a time;
  exact literal sets go by hash. *Agrees.* The prefix literals here are the same extraction
  (`regex_syntax` `Extractor`).
- **t6 §7 (Tempo, `ByteNfa`):** *Superseded as the default* for text tests. It stays the reference
  semantics for the differential tests.
