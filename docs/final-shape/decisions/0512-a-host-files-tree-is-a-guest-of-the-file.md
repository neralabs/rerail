# 0512. A file read for its programs is shown as a file; its tree is a guest of it

Status: decided (2026-11-20), by the user ("go ahead"), on Q385. The user's two requirements: what a rule sees is a
generic mechanism, not one made for Python ("why do we have ANNOTATION_KEYS? Shouldn't this be a more generic
mechanism, not Python-only?"), and "rules need to know line numbers and text in the host so that they can report
properly". Builds 0470 step 2; keeps 0058 §1 (what a rule sees is a bound on keys) and 0431 §4 (a program's nodes are
its file's).

## Intro

Under `.embedded on` a run reads files of other languages for the programs they hold: a JSON run reads a `.rerail`
file for its JSON objects, an HTML run's Python reads nothing new, but a Python run under it reads an HTML file for
its `<script type="text/python">`. The run must see:

- the **programs**, as code of its own language, at their file;
- the **file** as a file: its path (`doc`), its lines and their text (`src_line`, `source`, `line_count`), so a
  finding at a program's node is reported at the file's line, and a rule can read that line (a suppression comment, a
  line's text in a message);

and not:

- the **file's own nodes** (a `.rerail` file's `source_file` and `clause`, an HTML file's elements): code of another
  language, which no rule of this one names (0058).

Until now a host file took one key, from the hidden range (0470 step 2). Its nodes were under it, and so were its
`doc` row, its lines and its programs' document column (Q324). One key cannot be shown and hidden at once: hiding the
key hid the programs and the lines with the nodes, and exempting it showed every other hidden document too (Q385).

## Decision

1. **A document has one key; a file read for its programs is two documents over one source.**
   - **The file**: a text document under the file's own key (`memo::doc_keys`, below `GUEST_KEYS`, as every file's).
     Its `doc` row, its spans (`source`, `src_line`, `src_line_lf`, `line_count`) and their text are the file's, as
     a `.scope text` file's are. Its facts are the parsed file's source and line table, shared (`core::text_of`: two
     `Arc`s, no copy).
   - **Its tree**: the file parsed in its own grammar, a guest of the file document (`Made::File`), standing at the
     file's whole text (`Placement::Copied { delta: 0, .. }`), its `file` the file's key. It is a guest in another
     grammar, so its key is in the hidden range (`guests::hidden_file_key`, by path, claimed as a guest's is).
2. **What follows, with no rule of its own**:
   - its nodes are hidden as every guest of another grammar is: their ids are under a hidden key, and hiding bounds
     every unscoped node column;
   - their document column (`node`, `docof`) is the file's key (`Guest::file_key`), and their lines are the file's,
     through the placement, as a program's are;
   - its programs are guests of the tree (their host nodes are its nodes); their `file` is its file's key, through
     their host (`Guests::host_of`), so a program's nodes' document is the shown file and their lines its lines;
   - its rows are its file's in the memo and in maintenance (`guests::hosts_of`: a guest's key to its file's), as a
     program's are.
3. **`Made::File` is no program.** It has no `rr_program`, `sub_tape` or `parsed` row, and the store keeps no
   program record for it: it is the file itself, not code the file holds. It is a host of programs
   (`Guests::programs`).
4. **Hiding runs for `.embedded on`.** A program that says it bounds its unscoped base reads as a program that makes
   guests does. The bound is unchanged (`hiding::BOUND`).
5. **What a rule sees is by role and grammar, never by language.** A document's key range says what it is to the
   run: a file; a program in the run's language (shown); a tape value in the run's language (shown to the relations it
   reaches, `combining::FoldOwn`); anything else (hidden). The third range holds Python's string annotations today, but
   any language's tape value in its own run lands there (`guests::Guests::guest`, `label == self.run`). Its constant
   and comments say so: `RUN_VALUE_KEYS`, not "annotations".

## Rejected

- **Two prefixes in one document** (its nodes under a hidden key, its spans under the file's): every reader that
  finds a document by an id's top bits (`Batch::doc`, `Batch::between`, the batch's key order, the CLIF and device
  reads of decision 0310, the memo's `document_of`) would find it under two keys.
- **A second bound** (shown, or this host's key): a disjunction on every bounded atom, and it still shows the host's
  own nodes, which carry that key.
- **Hiding the file's lines too**: the user's requirement; a finding must be reported at, and read, its file's line.

## Tests

- `rr::tests::a_host_files_own_nodes_are_not_shown`: under `.embedded on` a JSON run sees a `.rerail` file's JSON
  program at the file's path and line, the file's lines and their text, and none of its own nodes.
- The 0470 and 0495 tests (`rr-pattern-on`, `a_t_programs_docof_of_a_programs_node_is_its_file`, the `.embedded`
  tests) keep passing: a program's file is still the host file, now under its shown key.
