# 0431. No guest documents: strings the rules parse are tape values; programs the grammar holds are the file's

Status: decided (2026-11-18), by the user: "there are no guest documents. The CPU and GPU run on exactly the same
data"; "parsed strings are only for annotations and maybe format strings, not all strings; the rules decide them";
"if the rules say 'here is an embedded string, here is its sha and here is its tape', then why can't that be kept in
the facts that the program produces and not changed next time it's run?". On a guest's node ids: (a), a node of a
parsed string is named by its tape ("do a"); and the split below ("okay, I agree with that"). Supersedes 0055's guest
documents (§4, "What was built" 3–4) and 0391 (guest keys) but for a file's programs (§4). On a program's node ids,
§4's second choice, a key of its own from 0391's registry (2026-11-19, the user: "okay, let's try b"). On how a
rule names a program, by its file and place (§4, Q267's first choice; 2026-11-19, the user: "pick a for the decision").
Step 3 done (2026-11-19). Keeps 0055's model of a tape as content, 0058 (hiding,
normalized strings, `@parse`), 0104 (no SQL in the waves), 0353 (blocks never copied), 0426 (format 16, unchanged).
Work queue Q261.

## Intro

A file can hold code in another grammar, in two ways:

- **A string a rule decides is code.** A Python string annotation (`x: "List[int]"`), a format string, an eslint
  directive in a comment, a regex or JSON value in a `.rr` file. The rules decide which strings; what is inside is
  tested and reported at the string or a host node.
- **A program the grammar holds.** An HTML `<script>` or `<style>`, a Vue component's script blocks, a Blade view's
  compiled PHP, Markdown's inline text, a `.rr` pattern. Whole rule sets (the TypeScript, PHP, CSS packs) run over
  them and report findings at their nodes, at the file's lines.

Until now both were *guest documents*: a document of its own, with a key in its own range, its own node ids and a place
in the chunk's batch. In every chunk the rules that ask for guests were cut out and run first (the "embed slice"); the
guests were parsed; the whole batch was built again with them; and the slice ran again, up to four rounds. On PostHog
(cold, M3, 2026-11-18, 1ca497070) that took 32.1 s of CPU on the CPU binding and 34.4 s on Metal, 18.4 s and 13.7 s of
it building the batch again. The device read a batch the host had rebuilt. A guest's tape was never stored (Q30), so a
file read from the memo had no guests, and a cross-file rule could not see them.

What is fixed, whatever rules run: the host's nodes, a text's tape (a function of the text alone), and each node's
position in its tape. A rule only decides whether a fact like `sub_tape(s, g, T)` exists. So nothing about embedded
code may be named by which rules asked for it, or in what order.

```
// x: "List[int]" = []               a.py; the same line in b.py, at line 4
//
// sub_tape(s, "annotation", T)      s = a.py's string node; T = the tape of "(List[int])", keyed by its sha
// parsed(s, "annotation", r)        r = (T, 1): the tape's node, the same r for b.py's string
// guest_of(n, s)                    n is a node of the tape parsed at s: both files' strings, for each n
// kind(n, …), child(…), text(…)     read from T where it lies
// a finding at s, or at s by n      placed through s: a node of T has no line of its own
//
// <script>let a = f(1);</script>   a.html: the script is JavaScript by the grammar, parsed with the file
// node(d, c, "call_expression")     c is a.html's node, numbered after its own: docof(c, a.html), line(c, 1)
```

## Decision

1. **There are no guest documents.** No document key range for guests, no guest key registry (0391) but for a file's
   programs (§4: each one's key), no embed slice, no rounds outside the engine, no batch built again.
2. **A string a rule parses is a tape value.** `@parse(grammar, text)` gives a tape keyed by the sha256 of its text,
   grammar and tape format (0055's key); equal texts anywhere share one. `parsed(s, g, r)` and `sub_tape(s, g, t)` are
   derived by the program's own rules, the parse a functor inside the fixpoint: a rule that reads the tape's nodes is
   in a later stratum than the rule that makes it, and a string inside a parsed string is ordinary recursion. Nothing
   parses a string no rule asked for.
3. **A tape value's node is named by its tape:** (the tape's key, its preorder). The same node for every place, rule
   and program that reaches it. It is no file's: `doc` has no row for its tape, so a finding on it has no path. Its
   `line` and `col` are its tape's own text's, which every view holds for every node, on the CPU and the device alike
   (a node with no row would make the views partial). A file's place comes through the string:
   - `guest_of(n, s)`: n is a node of the code parsed at string s, a row per place;
   - `guest_place(n, s, l, c)`: n, parsed at s, stands at line l, column c of s's file, through s's placement (the
     escapes and parts of a normalized string, decision 0058 §3). A string inside a tape value is placed in that
     value's text, which is no file's;
   - a finding placed at a tape value's node is refused (R9).
   A tape value is never shown as code to rules that do not name a guest (0058 §1's ranges): in the run's language it is
   in the annotations' range, else hidden.
4. **A program the grammar holds is part of its file.** The host language's parse makes it, as the matcher path makes
   its derived programs (`DerivedImage`): what a file embeds depends on its bytes and its grammar, not on rules. Its
   nodes are numbered in the file's id space after the file's own, in the order the parse found them, so `docof`,
   `line` and `col` are the file's and findings land at its lines. The libraries' structural `embed` rules (`html`,
   `vue`, `blade`, `markdown`, `rr` patterns) become the languages' declarations of what they embed.
   **A program is numbered by a key of its own** (decided 2026-11-19, the user: "okay, let's try b", the second
   choice below; step 4 came first, as the user asked on 2026-11-18). Hiding keeps one comparison on a node's key
   (0058, Tim's 0168), and order keys and the batch are unchanged; the `docof` and `node` views map the program's key
   to its file. The choice was made so:
   As built (step 3, `docof`): a program's document entry carries its file's key (`Guest::file`, `Projected::file`,
   `ViewDoc::file`), which `docof` gives for its nodes and spans, on the CPU and the device; its ids keep its own key.
   `node`'s document column stays the program's own key, the document underneath: the guest relations (`doc_role`,
   `doc_root`, `guest`) name a program by it, and a batch's lookups keep to one document by it (0329's `lookup_in`).
   Made the file's, every rule reaching a program through `guest(g, …), node(g, …)` lost it. core.rr's `docof` reads
   the places' view, the file's; where a program lowers `docof` as `node`'s projection (T's, `rr::lower::projections`),
   a program's node has its own key there (Q267).
   **A program is named by its file and place** (decided 2026-11-19, the user: "pick a for the decision", Q267's
   first choice). The guest relations that named a program by its key go (`guest`, `doc_role`, `doc_root`): a rule
   reaches a program as it reaches a parsed string, `parsed(at, grammar, root)` at the host node it stands at, then
   `within`, `child` and the views; `node`'s document column is its file's, as `docof` is, and a lookup by document
   finds a file's programs' rows with its own (a lookup kept to one document, 0329's `lookup_in`, is kept to the file
   and its programs). The key of its own stays underneath, in a node's id alone: hiding's one comparison and the
   batch's order keys are on it. The other choice, `node`'s document left a program's key where a batch holds
   programs, kept a program a document to the rules.
   As built (step 3, a program named by its file and place): `node`'s document column is the program's file on the
   CPU (`Viewed::value_of`), in CLIF (`view_reads::node`, `at::FILE`) and on the device (`view_expand`, `d.file`);
   `core` keeps the document underneath. A lookup of `node` by document reads a file's rows, then its programs'
   (`Batch::programs`, an index by file), and one kept to a document (`lookup_in`) keeps to the file and its programs.
   `guest`, `doc_role`, `doc_root`, `embed` and `embed_value` are gone from core.rr; the engine's own rules (the
   findings' syntax errors, the memo's components, `components.rr`) read `rr_program(d, file, grammar)`, the document
   underneath, which no library a rule reads declares. The libraries' `embed` rules are the languages' declarations:
   Vue's view and Blade's compiled view (as before), HTML's scripts and styles (`raw-text`), and Markdown's inline
   content (`inline-content`, `embed::inline_content`; the registry's `embeds` names the dialects an embedding gives,
   so Markdown embeds its own `markdown-inline`). A rule reaches a program as `parsed(at, grammar, root)` at its host
   node; the JSON rules' `embedded` enters one so (`parsed(_, "typescript", g)`). A grammar named by a constant with
   `at` bound by the rule's other literals asks for the string there (a tape value) where no program stands; a
   wildcard or a grammar named by a variable asks for nothing. Hiding's scope analysis takes a document column one
   way: a document's facts flow to its nodes (a tape value's document is its own), a node gives its document scope
   but not its grammar (a program's document is its file, no one grammar's), so a kind read through a file's
   document is qualified (`"javascript:call_expression"`). A program's facts are read off its embedder's parse
   (`core::build_parsed`), not parsed again.
   As built (step 3, the programs at load): a file's programs are made with its chunk's batch, before any rule runs:
   each embedding its language declares (`GrammarDialect::embeds`) builds them from the file's tape, read in place
   where its facts hold it (`Guests::programs`; `embed::host`), so the batch is built once with them, no slice round
   asks for them and the batch never grows (R5). A program of its own place (`Embedder::per_place`: an HTML file's
   scripts and styles) stands at the innermost node over its bytes, a whole file's (Vue's view, Blade's compiled view)
   at its root, as their libraries' `embed` rules name them; a library's request for one finds it made, under its
   grammar or its embedding's id (`Guests::seed`). The chunk keeps its documents, programs among them, where a rule
   reads a guest relation, and a program that reads one is bounded as one that asks for guests (hiding, 0058 §1, which
   qualifies a guest's kinds in its grammar). `html/embeds.rr` derives nothing now: HTML declares its programs
   (`languages/html.json`, `embed::raw_text`). `program()`, a library's request through an embedding, reads the
   host's tape where it lies too, not parsing the host again. Left: a program's text is parsed twice, once by its
   embedder (`Parse::read`) and once for its tape; the libraries' other `embed` rules (Vue, Blade, Markdown, `rr`)
   and the guest documents underneath (`doc_role`, `doc_root`, `guest`): done below (a program named by its file and
   place).
   A first scheme put its
   nodes in the file's key, as granules in a reserved slot range. It met 0058's hiding, which shows a node by one
   comparison on its document's key (and on document columns), and Tim's 0168 (PR #508), which keeps that one
   comparison and rejected per-asker keys as "a join on every bounded atom". The choices, from a survey of the
   sibling branches (Native-and-GPU-strata, oscar's hub merge in progress, final-shape-rr-align's 0402,
   CUDA-support, Tim's integration branch):
   - the file's key, hiding by key and slot: no registry and no program document; hiding bounds node columns twice
     and no longer hides by document column; order keys take a base in the CLIF id reads; a batch key holds a file
     and its programs;
   - a key of its own from 0391's registry, kept for programs alone: hiding, order keys and the batch unchanged; the
     `docof` and `node` views map it to its file (a `ViewDoc` field); a program is still a document underneath.
   A key of its own cannot be both stable and collision-free without a stored allocation (settled per chunk, it moves
   when the chunk does, and kept facts name it). Either way a program stays a tape of its own (0353 step 4c's reads,
   0399's per-tape presence), made by the host's parse at load, its `docof`, lines and findings the file's.
5. **Format 16 does not change** (0426): a tape is one text, one tree, one grammar. A file's programs are each a tape of
   their own grammar; a tape value is a tape. Around them:
   - a file's record of its programs (each one's tape key, host node, grammar, first id, the map from its text to the
     host's bytes), a derived fact stored beside the file's tape and loaded with it;
   - the reads (CLIF, 0310) resolve a node id to its tape and position: a file's node through that record, a tape
     value's node through its key. Written once, for the CPU and the device.
   Languages without a format 16 table (brace and percent formats, eslint directives, HTML, CSS, Markdown) keep format
   15 for their tapes until they have one (0293 §3).
6. **The batch is built once a chunk, and grows in place** (step 4b, the user, 2026-11-19: "Do 1"). A cold run learns
   which strings to parse by running the embed slice over the host batch, so its tape values come after the batch.
   The chunk keeps what built it (`syntax::Grower`, `Growth`), and a round adds only the documents it changed:
   - a new document is projected once, into the batch's block (its gigabyte of address space past the files'); the
     batch a round makes holds every projection, shared (`Arc`), in key order (0391's guest keys do not ascend with the
     rounds, and the batch's lookups search by key and key range);
   - a tape value the batch holds that gained places adds those places' rows alone (`GuestRows::places_added`);
   - each relation goes on from the last round's (`Relation::grown`): its rows are frozen in their block and their
     writer taken (`Rows::shared`, 0384 step 2), so the round's rows follow in the same block relation, which the
     device binds whole, and the last round's relation still reads its own rows, as `retain::owners_delta` needs;
   - the producers (`ProducerRun`) are kept, and hand over the rows made since they were last asked: the string
     annotations' producer places a later round's nested annotation by what it read off the file.
   Not taken: a batch of the values alone (the views and the device reading several blocks), and rebuilding with each
   document's projection reused (still a second batch).
   The views (`node`, `child`, `field`, `text`, …) are reads of tapes (0353): of
   the files' tapes, their programs' and the tape values the run's facts name. The CPU and the device read the same
   blocks.
7. **Kept, and not made again.** A file's programs are kept with the file. A tape value is stored by its key, written
   behind the run (0306), and loaded with the facts that name it before the first wave (0104). An unchanged file under
   an unchanged program parses nothing; a file read from the memo has its programs and its parsed strings' tapes, so a
   cross-file rule reads them.

8. **A tape value is read on lookup** (the user, 2026-11-19: "yes, please go ahead with that"; "if we need, I am okay
   with having a 'dynamic tape cache' so long as it's GPU-compatible (eg, uses the rolling LRU cache pattern we
   designed elsewhere)"). `@parse` is a deterministic function: its tape is found by its text's sha (step 4a) or
   parsed. `parsed(s, g, r) :- rr_embed_req(s, g), r = @rr_parse(s, g)` is a rule of the program, run in its
   fixpoint, and a string inside a value is the same rule again. Nothing runs before the program, and there is no
   fixpoint per parse: each iteration's new texts are parsed together, in parallel, a cache hit costing nothing.
   - Why the rounds existed: a view is bound before the run as a reader of the batch, and a scan lists only the tapes
     the batch holds, so every tape a rule might scan had to be in the batch first. But a value's nodes are reached
     only through a guest relation (0058's hiding), which binds them: a rule looks them up and never scans for them.
   - So each view answers a value's node on lookup, from the run's value tapes, in rows past those its scan reads.
     No scan lists a value, which is 0058's hiding by layout.
   - The value tapes are the batch's: a table that only grows, each tape admitted at its first lookup and projected
     once into the batch's block. On the device the same table is a rolling cache (0353 step 4b's: generations of a
     hash index and an arena, a miss loaded into the head, a hit in the oldest moved to it), so a run's values need
     not fit at once.
   - The planner keeps a value's nodes on lookups (`made_on_lookup`).
   - The memo files a value's rows with the files that asked for it (R7).
   Not taken: the program itself run in rounds (each round derives again the strata that read the new rows); a hook
   after the stratum that completes the requests (a loop back over base relations in the strata driver); the slice
   kept with its work shared (the slice stays).
   As built (the CPU, step 4c):
   - The request is answered in its component (0065 §2a, `corpus::guests::answered`). `rr_embed_req` produces each
     guest relation the program reads (`parsed`, `sub_tape`, `guest_of`, `guest_place`, `host_node`), and each
     relation a producer makes off a tape (`syntax::taped`: `string_annotation`) that its rules read, through its
     context too. The answer gives `parsed`'s, `sub_tape`'s and the producers' rows as its rows. A string nested in
     a value is a string annotation once its outer one is admitted, so it is asked in a later wave. Every relation's
     rows of the value are a part of the chunk's source (`syntax::values::Layered`), past the batch's rows. A view's
     rows are never given; they are read on the part.
   - A shown guest is a plain view's rows, reached by no guest relation: one in the run's own language, whose copies
     hiding folds back (0058, `FoldOwn`). So the premise above holds for hidden guests only. A request that produces
     an input therefore runs before every stratum outside its cone (`plan::requests_first`). The edges are kept for
     the run's scheduling, as a stratum that asks suspends (0114 §3), and the run goes on with the strata that do not
     depend on it.
   - A request's cover of one variable (`rr/lower.rs`, `covering`) leads with the literal that binds it from a
     relation a producer makes off a tape. So the cover is in the request's component, and reads a value's rows by
     the bound variable. A cover read before the request held none of a value's nodes.
   - A partial hit's cone follows a produced relation to its request (`wanted_cone`). A cone that reads `parsed`
     holds the request and its context.
   - A source made on lookup (containment, an axis) writes a part's rows once per looked-up value, past `within`'s
     row ranges. A part admitted since (a value given a place more) makes them again.
   - A partial hit counts no guest unseen: a value is made only as the cone's rules ask for it, so the cone test
     counts `values_admitted`.

   As built (the device, step 4c): the device reads a value's rows where they lie, as it reads the batch's (0353), and
   nothing is made for it on the host.
   - A value's relation rows are written once, past the chunk's rows and the values' admitted before, in the same
     block relation (`Grower::admitted`, `Relation::grown`). They are staged until then (`Relation::staged`). The
     device binds that relation whole (`Layered::in_block`). On the CPU each value reads its own rows of it
     (`values::Slice`).
   - A view's rows are read in place through one directory of the chunk's documents and each value's, each view's
     rows numbered as the layered source numbers them (`device_chunk::views_over`). The directory is made again only
     when more values are admitted. A source with no values yet is the batch's own, read as before.
   - Containment is solved where the run is bound: on Metal, on the device. The chunk's pairs and each value's are
     solved into one relation (`axis_in_chunk` over several chunks, each one's rows past the ones before). The device
     binds it whole, though the layered source numbers a value's pairs only as they are looked up.
   - A source that grew is bound again: the device's resident copy of a source is reused only while its rows are the
     same (or the source is made on lookup).
   - Left: an input a constant filters (`parsed(s, "annotation", r)`, and the `child` and `node` filters in
     `html/embeds.rr`) is uploaded, the batch's and a value's alike (Q266). R8's test allows exactly `parsed`'s rows
     for it.

## Requirements, each with its test

| | Requirement | Test |
|---|---|---|
| R1 | A program the grammar holds is its file's: `docof` of its nodes is the file, its lines are the file's; an edit inside one program renumbers no other's nodes nor the file's | `a_program_a_file_holds_is_the_files`, `an_edit_in_one_script_renumbers_no_other_program` |
| R2 | `guest_of(n, s)` names the string a tape value's node was parsed at | `guest_of_names_the_string_a_node_was_parsed_from` |
| R3 | One text is one tape and one set of nodes at every place; a tape value's node is no file's | `one_tape_at_two_places_is_one_set_of_nodes` |
| R4 | No string is parsed unless a rule asks for it | `a_string_no_rule_asks_for_is_not_parsed` |
| R5 | The batch is built once a chunk and grows by the rest (§6; `Report::batch_grown` counts the documents added): a file's programs, and the rows read are a rebuilt batch's. A tape value no longer grows it (§8, R10, which supersedes `tape_values_build_no_second_batch`) | `a_files_programs_build_no_second_batch` |
| R6 | A text a run parsed is not parsed again: after an edit elsewhere in its file, in a new file, after a rule's edit (`Report::guest_parses`) | `an_edit_elsewhere_parses_no_text_the_last_run_parsed`, `a_new_file_with_a_text_parsed_before_parses_nothing`, `an_edited_rule_parses_no_text_the_last_run_parsed`; guard `equal_texts_are_parsed_once_a_run` |
| R7 | A file read from the memo has its programs and tape values for a cross-file rule: a value's rows are base rows of each file that asked for it (`owners`, `askers`), kept in its entry and restored with it | `a_file_read_from_the_memo_has_its_tape_values_for_a_cross_file_rule` |
| R8 | CPU and Metal give the same rows, the device reading the same tapes: a value's rows read where they lie, none uploaded (but Q266's filtered inputs) | `cpu_and_metal_read_the_same_tape_values`, with both kinds |
| R9 | A finding placed at a tape value's node is refused: the lowering derives `rr_unplaced(clause)`, and the run fails naming the clauses | `a_finding_at_a_tape_values_node_is_refused` |
| R10 | A string a rule parses is parsed in the program's fixpoint (§8): no slice and no round runs for it, the batch never grows by a value, each value is admitted once at its first lookup (`Report::values_admitted`), a string inside a value too; no scan lists a hidden value's nodes, and a shown one's are read after the request (§8, as built) | `tape_values_are_read_in_the_fixpoint` |

The tests for R1–R4 are in `logic/rr/tests.rs`, R6's in `logic/plan/corpus/tests.rs`, ignored until their step lands (`#[ignore = "0431 …"]`). 0055's tests
keep their meaning, rewritten for `guest_of` and the file's programs where they read `guest(g, …)`.

## Steps

1. The record, its tests, Q261.
2. Tape values: node ids by tape key and preorder, `guest_of`, findings placed through the string (R2, R3, R9); the
   packs' `docof(n, d), guest(d, _, s, _)` over parsed strings rewritten (14 uses in 2 files; the `guest` rows left
   are the grammar's programs, step 3's).
3. Programs as the file's, done (2026-11-19): HTML, Vue, Blade and Markdown declare their programs, made from the
   file's tape at its load, the batch built once a chunk (`with_matchers`, or the grower where a value may come), a
   program named by its file and place, `node`'s document the file's, the guest relations and `embed` gone (§4, as
   built; R1, R5, R8). Left (Q268): `@parse` and `@splice` values still made by the slice's rounds; the rule-file
   language's embeds; the host's parse building the programs it drops.
4. (Before 3's rest.) 4a, done: a tape value is found by its text's sha and grammar among the tapes the root's last
   runs named, read before the first wave (the run's grammar's with the files', the others' by the tags the program's
   requests name, every tag where one is not a constant), else parsed and queued for the store with its text, which
   the manifest names (R6; Q30's "a guest's tape is not stored" is superseded). 4b, done: the batch grows by a round's
   documents (§6, R5). 4c: `@parse` as a functor in the fixpoint, a value read on lookup (§8, R10): the
   CPU and the device done (§8, as built; the device reads a 16z value through the views' one rolling cache, 0353
   step 4b); `parsed` and `sub_tape` of strings derived by the
   program; tape values stored and loaded before the waves (R4, R6, R7, R5's annotations). The embed slice and its
   rounds stay for the grammar's programs until step 3.
5. PostHog cold on CPU and Metal against 1ca497070's 32.1 s and 34.4 s of guests, findings unchanged (29,747,
   fa0efeaed6ff).

## Open

- A value's tape (`@parse(g, x)` of a computed string, `parse_of`): a tape value like any, placed through its asking
  node. To check against the packs' one use.
- `embed_value` (step 2): a library's value placed at a node, as `embed` is, so the second kind: its file's, placed at
  its node. Its one use in the packs is a `.rr` regex.
