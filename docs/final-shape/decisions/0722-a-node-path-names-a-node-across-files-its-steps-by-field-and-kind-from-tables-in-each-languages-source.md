# 0722. A node path names a node across files: its granule, then steps by field and by kind, from tables in each language's source

Status: decided (2026-11-27), by the user, over five revisions of the design page "Portable Node Handles"
(https://claude.ai/artifact/8JaBpWc381zxjpNA8F9yzx): "Use node_path or something similar", "of course they may
cross outside", "encoding and decoding it needs the tape; think through how we get the right tape when decoding",
then, of the step forms measured, "okay, do E then / but I want the tables to be in the source code like the other
per-language tables so we can't get out of sync with them." Being built (§ Built). It amends 0677 (§ 6).

## Intro

0677 keeps node ids out of the cross-file part. Python's points-to analysis names nodes across files anyway (a call
site, an argument, the value a name holds), so each node gets a term made with `cat`, `pt_tm(n, t)`:
`m.C.run@14+5^0`, its definition's symbol, its offset and length there, and its depth among nodes of the same span.
On the Django pack over 300 files those terms are 33.5 MB of string bytes and about 16 MB of atom overhead, about 50
of the store's 98 MB; they are made for every node a rule might ask about (about 1.6 s of CPU, 0677's measurement);
and an offset moves for every later node of the definition on any edit, a rename included. Every other language's
cross-file rules need the same as soon as they name nodes.

## Decisions

1. **`node_path` names a node, in every language: its granule and its steps from there.** The granule is the one
   the numbering already cuts (0083, 0094: a definition, or a statement of a container, keyed by its parent's key, its
   kind, its name and an ordinal). The steps go down from the granule's start to the node, comments and other extras
   skipped. A path is one 64-bit word: its base, 32 bits, the atom of the granule's key (the document's path, then the
   key: `pkg/mod.py/class_definition:C/function_definition:run#0`); then a stop bit and the steps. A path whose steps
   pass 31 bits restarts at an anchor (decision 5).
2. **A step says which child, by its slot and, in a list, by its kind.** A slot is a parent kind and a field, or its
   children in no field. A step is:
   - the slot, coded at a fixed width over the parent kind's slots in its table;
   - in a **kind-grouped slot** (a list of statements, or a field that holds several children: Python's `elif`s and
     `else`), the child's kind, gamma coded by its rank in the slot's kinds, then its ordinal among its siblings of
     that kind, gamma coded;
   - in any other slot, its ordinal in the slot, gamma coded, only where this node's slot holds more than one child.
     The tape shows that to both the encoder and the decoder.
   - where a parent's only path child costs no bits by those rules (one child, in its kind's one slot, not
     kind-grouped), a run of such only children is one number: how many nodes of the run the path takes, less one,
     gamma coded. Without it a node and its only child had the same bits (found by the round-trip test: `import os`'s
     `dotted_name` and its `identifier`). The design page's census charged those steps nothing, so its path lengths are
     a little short.
   A slot or kind a table lacks has an escape: a code past the table's, then its grammar id. So every node has a
   path whatever the table says; a table out of date makes paths longer, never wrong.
3. **The tables are in each language's source**, as its comment and paren kinds are: an `Adapter` const of its
   family (`NODE_PATH`), generated into the language crate by `rerail-engine node-path-table` and checked in. Per
   parent kind, its slots, most frequent first, each with whether it is kind-grouped and, if so, its kinds, most
   frequent first. Its lists are complete: every slot and kind the sample has, then those the grammar's node types
   allow that it never had. Beside them in the same const, written by hand: the kinds whose children are a list of
   statements (`PathTables::lists`) and the language's labels (`PathTables::labels`, decision 4). A test checks every
   name a table holds against the grammar's kinds and fields. The engine's build identity covers every source file, and
   every kept result is keyed by it, so a table edited makes every kept path again; none can be read with another
   table than the one it was made with. The tables are a few hundred entries a grammar (Python's 132 kind-grouped
   slots, 640 kinds).
4. **A path is spelled** `file "::" unit ("." unit)* ["/" step ("." step)*]`, `#k` the 0-based ordinal among siblings
   with the same spelling and left out where there is one (`pkg/mod.py::C.run/body.if#0.then.return.call.args`). A
   step is spelled by its field's label, or its kind's where it has no field or its slot is kind-grouped. Labels come
   from one rule for every grammar (a kind drops `_statement`, `_expression`, `_definition`, `_declaration`, `_item`,
   `_clause`, `_function`; `consequence` is `then`, `alternative` `else`, `condition` `cond`, `arguments` `args`,
   `parameters` `params`, `return_type` `returns`, `type_parameters` `tparams`), and the language's own labels. A
   name holding `. / # :` or a space is backquoted. Labels change how a path reads, never the word.
5. **Anchors are shared and canonical.** A node whose steps would pass 31 bits is put under its deepest ancestor
   within 16 bits of its base, and that ancestor is the anchor, a base of its own (an atom of its parent base and its
   steps); every overflowing node below shares it. A node has one base, the nearest anchor or granule start. Where
   anchors go depends only on the granule's subtree.
6. **0677 is amended: a `node_path` may cross into the cross-file part.** It names a place in a document by its
   granule's key and steps, which do not move when the file's node ids do. The check lets a `node_path` column and
   `@node_path(n)`'s value through, and still refuses every other value made from a node (`@to_string(n)`, `n + 0`,
   a `cat` with one). `@path_node(p)`'s value is a node.
7. **Which tape decodes a path.** A path carries no tape key. Its base's root is its document's path, and it means a
   node of that document as the state it was derived in holds it.
   - `@path_node(p)` runs in the per-file part and decodes only the current document's paths; another document's
     gives no row. A rule that needs another file's node asks that file's part by path.
   - A program a file holds (0431 §4) is a granule under its host node: the file's tape, then its programs' record.
   - A parsed string's node (0431 §2–3) has the tape value's key as its root, and is placed through the string that
     holds it (`guest_place`).
   - The writer (`{p}`, `{p.file}`, `{p.line}`) and tools read the tape key the run's document table records for the
     path, from the run's tapes; a tape collected since is an error naming the path and the key.
   - A granule is found by its key from its home slot, the candidates' keys recomputed from the tape.
8. **The column type is `node_path`**: `.type node_path <: number` in core.rerail, an `int` to the executors, and a
   column type of its own to the store (`p`), whose high 32 bits are an atom and are mapped as a symbol column's are
   wherever a forked process's rows are written or read (0170).
9. **Rules:** `p = @node_path(n)`, `n = @path_node(p)`, `u = @path_unit(p)` (its granule's own path), and in a
   message `{p}`, `{p.file}`, `{p.line}`.

## Measured (the design page's census: seven samples, simulated)

Path bits and anchors over each sample; for stability, 1,000 to 1,500 statements inserted per language (a copy of a
sibling, its subtree, at a random place in a block), the granule's anchors placed again, the other nodes whose path
changed counted. Tables counted on half of a sample's files, measured on the other half.

| Language | Anchors per 1,000 nodes: child index / E as measured / as built | Paths in 31 bits, as built | Nodes renamed per statement insert: child index / as built |
| --- | --- | --- | --- |
| Python | 3.13 / 3.30 / 2.99 | 92.9% | 25.9 / 16.1 |
| Rust | 5.98 / 2.64 / 2.32 | 87.5% | 46.0 / 25.3 |
| TypeScript | 7.46 / 5.39 / 4.41 | 70.5% | 65.1 / 30.6 |
| PHP | 3.09 / 1.94 / 1.62 | 95.3% | 25.6 / 17.6 |
| C | 5.88 / 3.36 / 3.30 | 84.4% | 45.6 / 20.6 |
| SQL | 6.83 / 7.60 / 7.17 | 53.0% | — |
| Rerail rules | 0.11 / 0.10 / 0.10 | 99.96% | — |

The renames with child indexes are a 20,000-insert run without anchors placed again; the others place them again.
"As built" differs from the E the user chose in two codes, both decided by what the engine can read: the slot is
coded over the table's slots (E took a parent's slots from node types), and an ordinal outside a kind-grouped slot is
written only where the slot holds more than one child (E took which fields can from node types, which the engine
does not have at run time). Both cost no stability, and both made paths shorter.

## Built

1. **The tables** (this commit). `PathTables` (`rerail-lang`, `code_rule_dsl/path_tables.rs`), with the labels'
   rule; `Adapter::NODE_PATH` in all 20 adapters, each `parents` an `include!` of its `<stem>_paths.rs` beside it;
   `LanguageBase::path_tables()`; `Codes` (facts, `vtab/syntax/paths.rs`), a grammar's tables as kind and field
   codes, held by its `Dialect`. `rerail-engine node-path-table <label> <root>... [--node-types <json>]... --out
   <file.rs>` counts the slots of every file of the label's family on the tape (path children: named, extras left
   out) and writes them, completed from the node types of the grammar versions the lock file pins, leaving out every
   name the family's grammars lack. Generated for 19 families, from Python's 300-file corpus, the C, C++, Go,
   Kotlin, Scala and Swift files of the shared lean corpus, and this repository's TypeScript, Rust, PHP, Blade, SQL,
   CSS, HTML, Markdown, JSON, Vue and rule files; the format family's table is empty. Java and C# have 11 and 2
   files: their tables lean on the node types. The test `every_name_in_a_familys_path_tables_is_one_of_its_grammars`
   checks every parent, field, kind, list and label against the family's grammars.
2. **The codec** (facts, `vtab/syntax/paths.rs`, `DocPaths`), over a whole projection of a document (a narrowed one
   cuts its granules over fewer nodes). Keys are the numbering's own cut again (`granules::Table::keys`: the table
   keeps how far it folded); a base's text is the document's path, a tab and the key; an anchor's, its base's text,
   `\t@`, its steps in hex and their count. Each parent's steps are made once, in one pass, the first time a path
   passes it (`Steps`), and a step is read back by its bits, which are prefix free among siblings; a granule's anchors
   are placed once, measured by the same coding. The interning is the caller's (`Atoms`). Tests:
   `a_node_path_decodes_to_its_node_in_every_language` (every node the paths reach in Python, TypeScript, Rust, PHP, C,
   Go, Java, SQL and JSON decodes to itself, a word a node; a 300-deep expression and a 70,000-element list are placed
   under anchors; 0.27 s), `an_edit_in_one_function_leaves_another_functions_paths` (a statement inserted in `f`, a
   function added before `g`: `g`'s paths are the same words).
   - Fixed with item 3: anchor placement kept an earlier sibling's branch in its chain, so a step's length came out
     as no coding and every path of a file was put under anchors (a round trip held: encode and decode read the same
     anchors). A file whose paths fit their words now has none (`a_small_files_paths_need_no_anchor`); 300 nested
     parentheses are one only-child run and need none either; 300 nested sums and a 70,000-element list do.
3. **The type, the functions, the store and the outputs** (this commit).
   - `@node_path(n)`, `@path_node(p)`, `@path_unit(p)`: node reads of the batch (`NODE_READS`, the functions'
     catalogue, core.rerail's functors). A document's paths are cut over its whole core-shape projection (the
     batch's own, else made whole for it), once per document in a batch. `@path_node` finds the document by its
     base's root.
   - Lowering's domain `Path`, from a declared `node_path` and the functions. A number that is also a path is a path
     (the type is `<: number`), and a derived number column a path is written to becomes one, so its atom is mapped;
     a path is never a node, a name, a kind or a field.
   - The store's `p` column: its high 32 bits are mapped as a symbol's are in a forked process's rows (`Mapped`),
     counted in `atoms_max`, content-keyed in a row's digest, and kept out of the stages' fast path; the memo's
     symbol columns carry the program's path columns too (`Syms::paths`), and its image keeps them as `col::PATH`:
     the base's string by index, the steps as they are.
   - An output shows a path as its text, `path_tables::path_text`: the base's text, a tab, `/`, its low 32 bits in
     hex. The native route and SQL's (`sym_text(c >> 32) || char(9) || '/' || printf('%x', c & 4294967295)`) give the
     same string, so outputs and memo keys are the same in every process. The spelling of §4 is item 4.
   - The 0677 check (§6): a node path is a value of its node; `@path_node`'s value is a node again, and a cross-file
     rule that decodes one, in its body or its head, is named.
   - Differs from §7: a guest document is rooted at its own document key (its path, a tab, `#` and the key), not
     under its host's node; its host's path and its key name it, the same in every run.
   - Tests: `node_paths_name_nodes_and_decode_back_in_rules` (two calls' paths, under `function_definition:f#0` and
     `class_definition:C#0/function_definition:m#0`, decode back to the calls; `@path_unit` is the granule's own path),
     `a_node_path_columns_atom_is_mapped_and_its_steps_kept`, `a_node_path_crosses_and_the_node_it_decodes_is_named`;
     the coverage ledger probes the three functions (native; on the device, no computed term, as `@text_len`).
4. **The spelling, and a message's `{p}`, `{p.file}`, `{p.line}`** (this commit).
   - `DocPaths::spell`: `file::units/steps` as §4 has it. A unit is its granule's name (its key's), else its kind's
     label, `#k` where its parent holds another of the same kind and name; a step is its field's label, or its kind's
     where it has no field or its slot is kind-grouped, `#k` where siblings are spelled alike; a name holding
     `. / # :`, a space or a backquote is backquoted (`pkg/mod.py::C.m/body.if.then.return`, `pkg/mod.py::f#1`).
   - A message's placeholder of a `node_path` column is a hole the rule fills with the word (`findings::PATH_HOLE`,
     `hole{j}_path`, `_path_file`, `_path_line`); the writer reads it (`corpus::path_hole`): the path's file read from
     the run's snapshot and made into documents as the run makes them (its tree, and its programs; a file of another
     grammar split, decision 0512), once a run, then decoded and spelled; `{p.line}` is its node's line in its file.
     So a finding names another file's node: its batch could not decode the path, and a rule that read it there would
     lose the finding. A path the tape no longer has is shown as it is, with why.
   - `@path_spelling(p)` and `@path_file(p)`: the same in a rule, over the batch's documents.
   - Differs from §7 in two ways. A value document (a tape value, `@parse`'s, `@splice`'s) is rooted at its identity,
     its grammar and text (`paths::tape_root`), not its key: a key is a run's registry's, and another process could
     not decode it; the writer parses it again (`guests::tape_value_again`). It is no file's: `{p.file}` and
     `{p.line}` are empty. And the writer makes a file's documents again from the run's snapshot, rather than reading
     the tapes the run kept: the run makes a memo hit's documents so too (decision 0458), and the same source and
     guest keys give the same tapes.
   - Tests: `a_message_spells_a_node_path_of_another_file` (`calls b.py::g in b.py at line 5, returning at
     b.py::g/body.return (line 6)`), `..._of_a_program_a_file_holds` (an HTML script's call, at its file's lines),
     `..._of_a_parsed_string` (two strings' annotation, one tape value, one path),
     `a_path_is_spelled_by_its_units_and_labelled_steps`; the ledger probes `@path_spelling` and `@path_file`.
