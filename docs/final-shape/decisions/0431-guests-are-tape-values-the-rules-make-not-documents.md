# 0431. Guests are tape values the rules make and keep, not documents

Status: decided (2026-11-18), by the user: "there are no guest documents. The CPU and GPU run on exactly the same
data"; "parsed strings are only for annotations and maybe format strings, not all strings; the rules decide them";
"if the rules say 'here is an embedded string, here is its sha and here is its tape', then why can't that be kept in
the facts that the program produces and not changed next time it's run?". Supersedes 0055's guest documents (§4,
"What was built" 3–4) and 0391 (guest keys). Keeps 0055's model of a tape as content, 0058 (hiding, normalized
strings, `@parse`), 0104 (no SQL in the waves), 0353 (blocks never copied). Work queue Q261.

## Intro

Some strings in a file are code: a Python string annotation (`x: "List[int]"`), maybe a format string. A rule decides
which ones, and reads the code inside as a tree.

Until now the engine made each such string a *guest document*: a document of its own, with a key in its own range,
its own node ids, and a place in the chunk's batch. In every chunk, the rules that ask for guests were cut out and run
first (the "embed slice"); the guests they asked for were parsed; the whole batch was built again with them; and the
slice ran again, up to four rounds, for strings inside strings. Then the program ran over the last batch. On PostHog
(cold, M3, 2026-11-18, 1ca497070) that took 32.1 s of CPU on the CPU binding and 34.4 s on Metal, 18.4 s and 13.7 s
of it building the batch again. The device read a batch the host had rebuilt. A guest's tape was never stored (Q30), so
a file read from the memo had no guests at all, and a cross-file rule could not see them.

The rules already say everything needed: this string node, parsed as this grammar, is this tape. That is a fact like
any other the program derives. So it is one:

```
// x: "List[int]" = []               a.py, its nodes 0..11
//
// sub_tape(s, "annotation", T)      s = a.py's string node 7; T = sha256("(List[int])", annotation, format)
// parsed(s, "annotation", r)        r = a.py's node 12: the guest's root, numbered after the file's own nodes
// guest_of(n, s)                    n = 12..15: the guest's nodes, each parsed from s
// node(n, …), child(…), text(…)     read from tape T where it lies, as a file's are read from its tape
// line(n, 1), col(n, 4) for List    through s's place in a.py
```

## Decision

1. **There are no guest documents.** A parsed string is a tape value: `@parse(grammar, text)`, keyed by the sha256 of
   its text, grammar and tape format (0055's tape key). Equal texts anywhere share one tape.
2. **The rules decide what is parsed.** `parsed(s, g, r)` and `sub_tape(s, g, t)` are derived by the program's own
   rules, the parse a functor inside the fixpoint: a rule that reads a guest's nodes is in a later stratum than the rule
   that makes it, and a string inside a guest is ordinary recursion. No slice is cut out and run first, and there are
   no rounds outside the engine. Nothing parses a string no rule asked for.
3. **A guest's nodes are its host file's.** They are numbered in the file's id space, after the file's own nodes:
   `file << 32 | local`, a guest's range starting at the file's node count plus the guests before it. A file's guests
   are numbered in stratum order, and within a stratum by (string node, grammar), so the numbering depends only on the
   file and the program: never on the chunk, the run or another file. `docof(n, d)` of a guest node is its file. A
   shared tape has one range per place, so `line`, `col` and `host_node` of each node are its place's. No guest key,
   no key range, no collision: 0391's registry goes.
4. **Where a guest node came from** is `guest_of(n, s)`: n was parsed from string node s. It replaces
   `docof(n, d), guest(d, _, s, _)`, the packs' 21 uses in 6 files. `guest(g, host, at, grammar)`, `doc_role` and
   `doc_root` of a guest go: a guest has no document to name.
5. **The views read a guest's tape in place.** `node`, `child`, `field`, `text`, `line` and the rest are reads of a
   tape (0353); a guest node's read finds its file's guest range (a derived fact, a relation in the run's block) and
   reads the guest's tape there. The batch is built once per chunk and never again. The CPU and the device read the
   same blocks: the files' tapes, the guests' tapes and the range relation.
6. **Kept, and not made again.** A file's guests are facts of the file: its ranges and the tapes they name are kept
   with its other rows (0402). The tapes are stored by their key, written behind the run (0306), and loaded with the
   kept facts before the first wave (0104), as a file's are. An unchanged file under an unchanged program parses
   nothing; a file read from the memo has its guests, so a cross-file rule reads them, and a chunk of partial hits no
   longer leaves them out (`guests_unseen`).

## Requirements, each with its test

| | Requirement | Test |
|---|---|---|
| R1 | A guest node is its host file's: `docof` gives the file | `a_guest_node_is_its_host_files` |
| R2 | `guest_of(n, s)` names the string a guest node was parsed from | `guest_of_names_the_string_a_node_was_parsed_from` |
| R3 | Positions are the place's: two files with the same annotation share a tape and keep their own lines | `one_tape_at_two_places_keeps_each_places_lines` |
| R4 | No string is parsed unless a rule asks for it | `a_string_no_rule_asks_for_is_not_parsed` |
| R5 | The batch is built once a chunk, guests or not | written with step 3, on the report's count of builds |
| R6 | A second run over unchanged files and program parses no guest | written with step 4, on the report's count of parses |
| R7 | A file read from the memo has its guests for a cross-file rule | written with step 4 |
| R8 | CPU and Metal give the same rows, the device reading the same guest blocks | the Metal path's parity tests, with a guest |

The tests for R1–R4 are in `logic/rr/tests.rs`, ignored until their step lands (`#[ignore = "0431 …"]`). 0055's
tests keep their meaning: names at their host lines, guests that do not parse, `.guests off`, embeds of embeds.

## Steps

1. The record, its tests, Q261.
2. Ids: a file's guest ranges as a relation of its block; guests numbered in the file's space (R1, R3); `guest_of`
   (R2) and the packs' 21 uses rewritten.
3. The views read a guest's tape through the range relation; `with_matchers` runs once a chunk; the batch rebuild and
   the guest documents go (R5, R8).
4. `@parse` as a functor in the fixpoint; `parsed` and `sub_tape` derived by the program; the embed slice, `DEPTH` and
   `GUEST_ROUNDS_KEY` go (R4). Kept: ranges and tapes stored and loaded before the waves (R6, R7). 0391's registry goes.
5. PostHog cold on CPU and Metal against 1ca497070's 32.1 s and 34.4 s of guests, findings unchanged (29,747,
   fa0efeaed6ff).

## Open

- A value's tape (`@parse(g, x)` of a computed string, `parse_of`): numbered in each asking file's space, standing at
  the asking node. To check against the packs' one use.
- The matcher path's derived programs (Vue, Blade, `file_documents`): already made at parse and stored; whether they
  become the same tape values is a later step, not this one.
