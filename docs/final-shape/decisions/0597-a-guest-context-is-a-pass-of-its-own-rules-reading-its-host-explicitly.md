# 0597. A guest context is a pass of its own rules, reading its host's finished facts explicitly

Status: built (2026-11-22; the audit's "partly built" corrected 2026-11-23: steps 1-7 are built, step 6 measured,
below). Proposed (2026-11-21), from the user:
"should we instead do another fixedpoint of annotation/template analysis AFTER the main program has run, with the
facts from the main program as an input?"; "I was already thinking about having each language run in a different pass
... scan the code inside f-strings and handle DSLs like vue's html-like code"; "yes, an explicit pass"; "if we want to
check annotations and f-strings in Python, we will need to run an extraction pass first as a fixedpoint or after and
then check them". Would replace 0058's annotation range and 0165's narrow copies and fold for guests; keeps 0431
(guests are tape values, no guest documents, no batch built again), 0353 (blocks never copied), 0442 (no global
state).

## Intro

Code inside a file's code is analysed in a different context from its host: a Python string annotation
(`x: "List[Foo]"`) is a type expression whose names resolve in the host's scope but whose own rules differ (it is never
run); an f-string's template can be SQL with holes; a Vue directive's value is a TypeScript expression whose names are
the component script's bindings.

Today a guest is folded into the host's program. Hiding (0058) writes a copy of each library rule that can reach a
guest (`rr_in_annotation_0_unparen`), the fold (0165) widens the originals to take the copies' rows, and the policies
that do not read guests (#923) get narrow copies of the widened originals back. On corpus0 × B-now (one Python pack):

| | |
|---|---|
| string annotations | 56, about 112 nodes, against 427,653 host nodes |
| hiding's annotation copies | 616 relations, 1,311 rules: 16% of the program |
| narrow copies and their filters | about 8 s of CPU, the widened library evaluated a second time |
| cycles mixing host and annotation relations | 4, all a structural comparison of two trees (`q*_same_pair`) |
| host relations the annotation rules read | 172 (scopes, bindings, imports, kinds) |

The work is the machinery that keeps 0.03% of the nodes apart from the rest. The dependency check (the program as
hiding leaves it, `tmp/l4/hidden.py`) found no cycle that crosses host and guest but the tree comparisons, so the
guest's analysis can be ordered after the host facts it reads, and the host facts that read the guest's after it.

What it would recover, measured (corpus0 × B-now, off, one thread, an M1 Max, 2026-11-21): the same build with no
narrow copy, every reader reading the widened originals (a temporary switch, the semantics before #923, the cost a
host program with a handful of annotation rows has): one pack 0.983 T to 0.801 T instructions (the target before the
hub is 0.786 T), the normal packs 2.179 T to 1.828 T; 463 findings, the same set. The narrow copies are 0.18 T of the
one pack's 0.197 T gap.

## Decision

1. **A context is a program of its own, run as a pass.** A context is a grammar and a role: `python` (a file),
   `python/annotation`, `python/format-spec`, `sql`, `vue/expression`. Its program is ordinary rules (Soufflé `.rr`,
   the library's `#include`s), run by the same engine over the context's roots as its documents: the views (`kind`,
   `child`, `text`, `within`, …) give the nodes under its roots alone. No rule sees another context's nodes but through
   what that context exports.

2. **What a pass reads of another is declared, and read finished.** A context program names the relations of another
   pass it reads (`.from python binding, scope_node, import_name`), and reads them qualified (`python.binding(n, x)`,
   Soufflé's qualified name). They are inputs, read in place in that pass's blocks (0353: no copy, no upload). A
   context exports what others may read (`.export ann_type, ann_uses`); a host rule reads it as `annotation.ann_type`.
   As built (step 1, 2026-11-21): a relation name cannot hold a `.`, which ends a clause, so a read is by a name its
   `.from` declares, not qualified: `.from python binding, scope_node for annotation` (the context reads the host's
   `binding` as `binding`), `.from annotation ann_type as quoted_type` (the host, the program's language, the
   default reader). `.export` is phase 0's already (a file's exports, `phases::Declared`), so a context's exports are
   on its own line: `.context annotation of python roots ann_root exports ann_type, ann_uses`.

3. **Extraction is rules of the host, in the host's fixpoint.** Which strings are annotations depends on the host's
   analysis (`cast("Foo", x)` needs `cast` resolved to `typing.cast`), so a pass's roots are derived by its host:
   a relation of the host's whose first column is the root's node (`roots ann_root` on the `.context` line, as built:
   `ann_root(r) :- string_annotation(s), parsed(s, "annotation", r)`). A root is one of two kinds:
   - **a text**, parsed in the context's grammar as a tape value (0431 §2): a string annotation, a Vue directive's
     value, an f-string's template as SQL (`fstring_template`, its holes markers). Equal texts share one tape;
   - **a region of the host's tape**, its subtree, parsed already: a JSX `{…}`, a format spec's node. Nothing is
     parsed; the pass's views are bounded to the region at each read, as hiding bounds a document today.

   A guest inside a guest (`"List['Foo']"`) is a root its own pass derives: a pass's roots are its host's and its own,
   admitted in its fixpoint (0431 §2's recursion), not a round outside the engine.

4. **The passes run in their dependency order.** The engine builds one graph of every pass's strata from the reads of
   rule 2: the host's strata its extraction needs, then the context pass, then the host's strata that read the
   context's exports (an F401's `used(i) :- annotation.ann_uses(i)`), and so on for each context. A cycle across two
   passes is refused at compile time, naming the relations. A comparison over two contexts' trees (`same_pair`: a
   host expression against an annotation's) is a rule of the later pass that reads both trees, each through its own
   pass's export (`annotation.unparen`).

5. **A relation has one meaning, in every program that holds it.** A host relation that counts an annotation's uses
   says so in its rules, and every policy reading it reads the annotation pass; one that does not never does. So a
   policy finds in a pack what it finds alone by construction (#923), and nothing is widened, narrowed or folded: the
   annotation range (0058 §1's `RUN_VALUE_KEYS` to `HIDDEN_KEYS` for annotations), hiding's annotation copies, the fold
   (`FoldOwn`), the narrow copies and their filters (0165) go.

6. **Findings in a context stand at its host.** A finding at a node of a text root stands at each string that holds
   it, through `guest_place` (0431 §3, R9); at a region root, at the node itself. A policy is a host policy whose rules
   read a context's exports, or a context policy (`.context python/annotation`), placed so.

7. **A pass runs only on what changed in its inputs.** Its inputs are its roots (its host's extraction rows) and the
   relations it names in `.from`, nothing else: that is what the explicit reads buy. A pass with no roots makes nothing
   (on corpus0 and most files, no file holds an annotation). Maintained (0331), a pass's strata run on the deltas of
   those inputs alone: an edit that changes no extraction row and no row of a `.from` relation the pass reads runs
   none of its strata. An annotation unchanged in text is not enough: renaming the class `"Foo"` names changes its
   resolution, through `python.binding`'s delta.
8. **A pass's tape-local facts are kept by its tape.** A context relation that reads only its own tape (an
   annotation's `unparen`, its kinds, a SQL statement's shape) is the same for every string holding that text: it is
   kept by the tape's key and made once per text, in a run and across runs (0431's "kept in the facts the program
   produces"). One that reads a host's export (name resolution) is the host's place's, maintained by that export's
   deltas (0331).

## The Python contexts

- **`python` (the file).** The library as it is, less its annotation rules. It derives the annotation roots
  (`string_annotation`, the `typing` contexts) and reads `annotation.*` where a host fact counts an annotation: used
  imports (F401), undefined names (F821), a parameter's type from its annotation (`ann_type`), typing-only imports
  (TC001–TC004).
- **`python/annotation`.** The roots: each string annotation, parsed as the `annotation` grammar. Reads of `python`:
  its scopes and bindings, imports, the class and module a string stands in, `typing` names. Exports: the names it
  uses and what each resolves to, its type, its nested annotations. Its rules: the library's expression rules
  (`ast.rerail`'s `unparen`, `call_*`, `attr_of`), included as they are, and its own name resolution, which looks
  names up in the host's scope at the string (`look`, `walk` and `gbefore` as one context's rules, not copies).
- **f-strings.** An interpolation's expression is the host's own code (Ruff checks it so) and stays in `python`. The
  contexts are what is not Python: a format spec (`python/format-spec`, a region root: the spec's mini-language), and
  an embedded language in the template (`sql` on `fstring_template`, a text root whose holes are markers; a hole's
  node links to its interpolation by an export of the extraction, `hole(m, interpolation)`).

## Vue

A component's script block and its template's expressions are already parsed as guests (`lib/vue/code.rerail`:
`vue-expression`, `vue-statements`, `vue-params`, `vue-for-alias`). As passes: `vue` (the template's markup) derives
the roots; `typescript` over the script block exports its bindings and props; `vue/expression` over the directives'
values and `{{ }}` reads `vue.*` (the element, `v-for`'s aliases in scope) and `typescript.*` (the script's bindings).
The template's findings stand at the file's lines.

## To build

1. **The front end.** `.context`, `.from`, `.export`, `.root`, qualified reads; a pack's programs into one pass graph;
   the cross-pass cycle check. Tests: a context program's reads and exports; a cycle across passes refused, naming it.
   Done (2026-11-21, `logic::contexts`, applied by `rr::parse_version`): the declarations read and checked (a pass
   named is declared, a context's roots exist with a node first, a read of a context is of its exports, an export is
   derived, a reader names a relation once); each context's instance `rr_pass_<context>_<r>` of every relation its
   exports reach but its `.from` reads and the views, the library's rules as written (`unparen` in the annotation's
   and the host's own); each reader's `.from` names reading the source's relation; a relation only a context reaches
   is not the host's; a cycle through two passes refused, naming them and its relations. The views are shared until
   step 2 bounds them. Tests: `logic::contexts::tests` (six) and `a_context_holds_the_librarys_rules_it_reaches_as_its_own`.
   Left for step 2: the run's paths (a pack's `combine`, a library held once), which lower without `parse_version`.
2. **The engine.** A pass's roots as its documents: text roots as tape values (0431's admission, unchanged), region
   roots bounded at each read; the views bounded to a pass's roots; a pass's `.from` inputs read in place in the
   earlier pass's blocks; the strata of every pass in one order, on the CPU and the device (one launch per pass's
   stratum, bounded). Tests: a context's rules see its roots' nodes alone; a host stratum after the pass reads its
   exports.

   Built on the integration's preparation (merged first, ca7e1c18a6: 0563's reach by kind, 0575's diffed narrow
   copies, the corpus's preparation in its own crate), so the passes replace what the branches have, not a tree they
   left. The ladder, each rung a test written red before its code, in the order the mechanism needs:

   | Rung | Test (run-level unless said) | Locks |
   |---|---|---|
   | 2.1 | the passes made of a program, printed and read again, are the same program (`same_program`); the lowered form holds `.pass` settings, not `.context` or `.from` (unit) | made once, on any path that reads a printed program |
   | 2.2 | a program whose host reads a context's export runs, through `guests_run` (`parse_file_version`, a printed program, `corpus::run`) and a pack (`run_all`) | every reading path makes the passes |
   | 2.3 | the context's `kind`, `child`, `text` give its roots' nodes alone: `x: "List[int]"`'s export is `List`, `int`, never `x` | a context's views bounded to its documents, read in place |
   | 2.4 | the host's views give the host's nodes alone in a program with a context: its identifiers are `x`, never `List`; the prepared program holds no `rr_in_annotation_` copy and no `rr_narrow_` relation for the rules the context holds | no widening, no narrow copy, for what a context holds |
   | 2.5 | a context reads its host's finished rows: `class Foo: ...` then `x: "Foo"` resolves `Foo`, `x: "Bar"` does not | `.from` read in place, ordered before the reader |
   | 2.6 | host, context, host: classes no code and no annotation names (`.from python defined for annotation`, `.from annotation ann_used`) | the three-step order in one plan |
   | 2.7 | an annotation inside an annotation (`q: "Opt['Baz']"`): the context's own roots relation adds its roots | a pass's roots are its host's and its own |
   | 2.8 | a finding at a context's node stands at its string's line and column in the file | placement through `guest_place` (0431 §3) |
   | 2.9 | two policies, one with a context, find together what each finds alone | #923 by construction |
   | 2.10 | a run over files with no root makes no stratum of the context | 0563 A: no rule can fill it |
   | 2.11 | a maintained edit that changes no root and no `.from` relation runs none of the context's strata; renaming the class an annotation names runs them | 0563 B: reach by its inputs alone |
   | 2.12 | 2.3 and 2.5 on Metal (a farm Mac), the same rows | one mechanism on the CPU and the device |

   As built:
   - 2.1 (adc1b74e7d): what is made holds `.pass NAME of HOST roots REL exports R, … [reads R, …]` in place of
     `.context` and `.from`; `contexts::made` reads it. A later `.from` of a made pass (a pack's policy reading the
     library's context) reads its instance; the same read twice is one; a read for a made pass is refused.
   - 2.2: the passes are a program pass of the lowering's tail (`contexts::Passes`, before `Declared`, since a `.from`
     name is declared by its pass), so every reading path makes them: a policy file, a printed program, a pack's
     `combine`. A made pass is completed there with what its instance reaches that it does not hold yet (a part
     lowered in a library's scope holds less than the combined pack); its roots and reads stay its host's.
   - 2.3: each context's nodes, `rr_pass_<context>_rr_nodes(n)`, are its roots and what is `within` them, a derived
     fact; each node variable a positive view atom of its rules reads is joined with them. The views stay read in
     place (the tape); nothing is made of their rows. Tests: `a_host_reading_a_contexts_export_runs_on_every_reading_path`,
     `a_contexts_views_are_its_roots_nodes_alone` (`x: "List[int]"` and `def f(y: float)`: `List`, `int`, never `float` or `x`, alone and in a
     pack).
   - 2.4 (4798383568): hidden at the run values' bound and folded as a pack folds it, a context's instances get no
     `rr_in_annotation_` copy and no `rr_narrow_` copy, and the host's relation both passes read stays the host's; the
     control, the same reach written into the annotation's trees from the host's rules, is copied and widened.
     Nothing in hiding was changed for it: a context's views are scoped by its nodes, which its roots' guest relation
     scopes, and nothing of the host reaches them.
   - 2.5, 2.6: one engine fixpoint orders the passes by their reads: the context reads the host's finished classes,
     and the host's rows after the context's read them.
   - 2.7: a nested annotation's tree is a root already: the host's roots rule reads `parsed`, so a string inside an
     annotation's tree is scoped, and its parse a root. No roots relation of the context's own was needed.
   - 2.8: a finding at a context's node stands at its string's line (`guest_place`, unchanged).
   - 2.9: moved after step 3. A policy whose own clauses read `parsed` (its own roots rule) asks guests of its own and
     runs alone (`tools::policy_groups`, `asks.own`), so the pack the rung tests is the shape step 3 makes: the roots and
     the context in `python/annotation.rerail`, which every policy that includes `ruff.rerail` includes.
   - 2.10: with no root the context's nodes are empty, and the file-local part holds each of its strata empty (no
     component opened). The cross-file part read the local part's export of it as an input and joined it: an input bound
     already is now taken as empty when it holds no rows, in a program that answers no request (no source grows while
     it runs). Test: a profiled child run's `heads.tsv` names no head of the context.
   - 2.11: in a session, an edit that changes no root and nothing the context reads changes no input of its cross
     strata; renaming the class an annotation names changes `defined_at`, the host relation the context reads.
   - 2.12: 2.3's and 2.5's rows on Metal (every stratum the device takes placed there) are the CPU's.
3. **The Python annotation context.** `lib/python/annotation.rerail` from the annotation rules of `names.rerail`,
   `typing` and `ann_*`; the host's readers of annotation facts read `annotation.*`. Tests, kept and passing:
   `quoted_annotations_are_read_as_their_code`, #923's `a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone`,
   C408's `a: "dict({})"`, a nested annotation, R9's placement.

   As built (2026-11-22). The ladder:

   | Rung | Test | Shape it locks |
   |---|---|---|
   | 3.1 | `a_context_declared_twice_exports_both_declarations` | each library file declares the context again, with its own exports |
   | 3.2 | `a_contexts_read_of_its_hosts_node_binds_only_its_own` | `host_node` and `guest_of` are not guarded: their host column is no node of the context |
   | 3.3 | `a_hosts_rule_reading_a_contexts_export_is_no_rule_of_the_context` | a host rule that unions an export, by an alias, is no rule of the instance |
   | 3.4 | `a_program_of_no_language_is_its_librarys_host` | a test's or a phase's program takes its contexts' host as its pass |
   | 3.5 | `a_contexts_rule_reading_an_export_by_its_own_name_is_the_contexts` | a read with no alias names the context's own relation in both passes |
   | 3.6 | `a_nested_quoted_annotation_is_its_code_and_no_any` | ANN401 over a nested string: parsed, a forward reference, not `Any` |

   - **The library.**
     - `python/annotation.rerail` holds the extraction (rule 3), which is the host's:
       - `ann_root`, `ann_ctx`, `str_node`, `ann_str`;
       - `ann_parse(s, r)`, every string's tree, nested ones too;
       - `ann_host(h, s)`, a tree's host node and its outermost string;
       - `ann_tree`, the roots.
     - It also holds `ann_in`, the context's nodes by their outermost string.
     - `ann_parse` is the only rule that asks for a parse. A reader reads it, never `parsed`: one request of a
       cross-file relation would make every request of the program cross-file, unanswered by its files (the oracle
       test's runs admitted no value until this held).
     - The context reads the host's `ann_parse` (`.from python ann_parse for annotation`). Its own instance would hold
       nothing, as no annotation root is in a tree.
     - Each reading file declares the context again with its exports:
       - `names` exports `look` and `miss`;
       - `ast` exports `ann_dotted`;
       - `annotations` (ANN) exports `annt_root_*`;
       - `points-to` exports `pt_aleaf`, `pt_ahead`, `pt_aarg`, `pt_dtype`, `pt_aself`, `pt_look`, `pt_qn` and
         `pt_builtin_ref`.
     - The host counts an export's rows in rules of its own (rule 5).
   - **Lookups.** A guest's name starts from its tree's host node:
     - `ann_start` and `pt_hstart` are keyed by `ann_host`'s h, which `host_node` gives every node of a tree.
     - Keying them by the string matched nothing: `host_node` gives the string's content.
     - The walk then steps through the host's `scope_up` and `pt_up`, as one meaning (rule 4).
   - **Engine.**
     - Repeated declarations merge (3.1).
     - An export a library declares whose rules the program's outputs never reach (no rule derives or reads it) is
       dropped, and a context left with no export is no pass. An export declared nowhere is still refused.
   - **Dropped.** `ruff-classes`' `ann_input` from `parsed` went: RUF013 never found a quoted annotation, with or
     without parses.
   - **Not changed.** `typing`'s `type_ctx` reads quoted strings that are not annotation roots. Those are not the
     context's.
   - **2.9.** Covered by #923's test in the shape 2.9 needed: both policies include `ruff.rerail`, so both ask the same
     guests and run as one pack.
   - **Results.**
     - The probe gives 20 findings, the baseline's.
     - corpus0 × B-now gives 463 findings, identical to the earlier run's.
     - The targeted set passes: contexts, corpus, session, rr, tape and oracle tests (88 and 13).
     - The names.rerail include of annotation comes before imports: names and imports are one unit, which must be
       listed after what it includes.
4. **Removal.** Hiding's annotation range and copies, `FoldOwn`, the narrow copies, `combining::one_document`, and
   their tests, ported to the passes' (removal is removal: no switch to the old path).

   As built (2026-11-22), in two commits:
   - **Part 1: a guest of a context is read by the context's rules alone.** `Hide` drops, in a rule of no context,
     the facts of a guest whose grammar a made pass's roots carry (`hide::as_read`). Such a rule carries the guest's
     nodes as values (a root, an export's row) and is neither cloned nor copied for them. The leak was the library's
     shared rules: `pt_amem(t, r) :- pt_amem(t, s), host_node(s, _), pt_qann(s, r)` made dozens of
     `rr_in_annotation_` copies of points-to in the host.
     - The made passes (`PREFIX`, `of_a_context`, `made`) are `passes_made`, in rerail-planner, below hiding and the
       contexts, which re-export them.
     - A pack completes a library's context over the combined program: `Tail::Many` runs `Passes`, then
       `CrossingCycles`. A member's rule of the library's context (#923's `anns`, `.context annotation … exports
       ann_list`) reads the library's rules as the context's. A member's own context, made whole in its part, is left
       as it is.
     - Instances are recognised under a pack's prefix (`p0_rr_pass_quoted_…`). The link renames a member's `.pass`
       roots and reads as it renames its relations.
     - A view is guarded though a combined program derives it (`kind` from `node`). A guard is on base reads only.
     - `CrossingCycles`, made in rung 2.2, was never run; it runs after `Passes` on both paths.
     - A program's metadata leaves out the passes' settings: they are structure, not metadata.
   - **Part 2: the removal.**
     - The run values' key range is gone (`RUN_VALUE_KEYS`, `Range::RunValue`, `Seen::Annotations`,
       `guests::widened`). Code guests in the run's language take `GUEST_KEYS..HIDDEN_KEYS`. Every value, an
       annotation among them, is hidden. Hiding's `BOUND` is `HIDDEN_KEYS`.
     - `combining`'s fold is gone, about 1,800 lines: `FoldOwn`, `fold_own_guests`, `Unnarrow`, `Narrowing`, `Diffed`,
       `one_document`, `filters_of`, `ANN_NODE` and `ANN_DOC`. So are the preparation's fold step and the switches
       `RERAIL_SHOWN_COPIES` and `RERAIL_NARROW_DIFF`.
     - A copy hiding makes for a code guest in the run's language now stays a copy. Its rows are its guest's, so it
       holds the same rows the fold gave.
     - **Tests.** Five fold tests went with the fold. `an_axis_over_a_guests_kind_set_names_a_set_of_the_program`
       keeps #862's check, an axis that names no set, on hiding alone. Rung 2.4's test checks hiding alone. Q407's
       ignored test, `a_partial_hit_makes_only_the_guests_its_cone_can_see`, passes now: `ids` over `idents` reads
       no annotation.
   - **Results.**
     - The probe gives 20 findings, the baseline's.
     - corpus0 × B-now gives 463 findings, digest 75193ca656bd, identical to the baseline.
     - The targeted set passes (102), and so do the contexts tests (13) and the hide and guest-key tests (7 and 2).
     - The quick tier of rerail-logic and rerail-tests-logic (1,230 tests) found three failures since step 3, which
       the targeted set had not run. Each is fixed:
       - A program lowered without its rules (`lower_version(false)`) keeps the library's declarations. A written
         or made export that is declared is not refused; the refusal is for a name declared nowhere.
       - Two flat members of a pack (`print_version`'s text) each carry their whole made `annotation` pass, under
         their own prefixes (`p0_ann_tree`, `p1_ann_tree`). These are one pass with two instances. Made passes of one
         context and host merge, and each member's roots are checked (`Context::other_roots`).
       - A refused twice-declared context names both declarations.
       The fourth, `programs_hidden_alone_keep_their_attribute_copies_declared`, was missing its pack test data. It
       passes with the lint data placed: restored from 754e41916b~1 and matched to its `test-data.lock` sha.
     - The quick tier again on 12d3738235, with that data, gave 1,229 of 1,230. The one failure,
       `signed_gate_statistics_reach_the_background_cost_store`, waited 10 s of wall time for the cost store's
       service thread, which runs behind the run every second, and the full suite's load starved it. It now writes
       what is left as a run's end does (`costs::flush`) and checks once: the writer holds its store for a whole
       write, so nothing taken before is uncommitted. It passes in 0.45 s.
     - The quick tier after that fix: all 1,230 tests of rerail-logic and rerail-tests-logic pass.
5. **f-strings and Vue.** `python/format-spec` and SQL in a template; `vue/expression` reading the script's bindings.

   The plan (2026-11-22). Each rung is a test written red before its code:

   | Rung | Test | Locks |
   |---|---|---|
   | 5.1 | `f"{x:>{w}}"`: the format-spec context's identifiers are none, and its nodes are the spec; the host's identifiers include `w` | a region root is bounded at each read; its holes (`holes REL` on `.context`) are the host's code |
   | 5.2 | `f"{x:>10.2f}"` exports align `>`, width `10`, precision `2`, type `f`; `f"{d:%Y}"` is no standard spec | `python/format-spec.rerail`: the spec's mini-language, its rules the context's |
   | 5.3 | `parsed(s, "sql", r)` of `f"SELECT a FROM users WHERE id = {uid}"` gives a tree whose `users` stands at its own column; the hole's node stands at `{uid}`, and its host node is the interpolation; `parsed(s, "annotation", _)` of an f-string is none | an f-string's template is a text root in a language's grammar, its holes a marker placed at each field |
   | 5.4 | `python/sql-template.rerail`: the context `sql` over the templates exports the tables a query names and each hole's role (a value or a name); its rules are the SQL library's | SQL in a template, read by a host rule |
   | 5.5 | a Vue template's code is the context `expression`, its identifiers the template's alone; the script is the context `script` | the two passes of a component's code |
   | 5.6 | `<li v-for="item in items">` beside `const item = 1` in the script: no-template-shadow finds the shadow through the script's export | `vue/expression` reads the script's bindings |
   | 5.7 | the Vue pack's policies find on their examples what they found before | the move kept each policy's meaning |
   | 5.8 | a context policy's finding at a template piece's node stands at the file's line and column | rule 6: findings at the host's bytes |
   | 5.9 | the pack's policies that read the scripts find their examples, alone and beside another policy | the script policies are context policies |
   | 5.10 | a policy's own rule of a library's context reads the library's relations of that context by their own names | a member lowered apart is completed against the library's instances |

   As built (2026-11-22), rungs 5.1-5.5:
   - 5.1-5.4 (474213092b, a39939f137): a region root's holes are its host's (`holes`); `python/format-spec`; an
     f-string's template a text root; `python/sql-template`'s context `sql`. A context guards no token column
     (`first_token`, `token_after`, …: `contexts::view_nodes`' `TOKENS`), so a hole's value stands at its token.
   - A context's instances are `rr_pass_<context>_<r>` (`passes_made::PREFIX`): `rr_cx_` was the import components'
     prefix too (`rr_cx_reach`), and a program reading both read the one as the other.
   - **A Vue component's code is pieces built on demand.** The registry's `"demand": true` embedding
     (`languages/vue.json`: `vue-script`, `vue-expression`, `vue-statements`, `vue-params`, `vue-for-alias`,
     `vue-for-source`, each TypeScript's) is no file's program at its load: `embeds()` and `program_embeds()` leave
     it out, `demand_embeds()` lists it, and a demand embedding has no places. A rule's `parsed(v, "vue-expression",
     g)` asks for it at its place (a constant grammar, `rr_embed_req`); the run builds the host's pieces of that
     kind once (`guests::demanded`, `embed::vue_code`) and gives the one at the place, its `Guest.grammar` the name
     asked. A piece's code is copied byte for byte between written wrappers (`(E)`, `(P)=>0`, `const [A]=0`: `let [x]=0` reads as an assignment to `let[x]`), so its
     nodes stand at the file's bytes; its `docof` is its file's (0431 §4). Test:
     `a_components_code_is_pieces_made_where_a_rule_asks`; the registry's `a_demand_embedding_is_listed_apart`.
   - **Grammars that share their kinds join to their label.** `vcode`'s `g` is filled from five roles, each its
     own grammar name; `hiding::Fact::join` joins two grammars whose kinds' label is one (`kinds_label`, a demand
     embedding's one dialect) to that label, where it joined them to none and every kind went unqualified. Copies
     made for a grammar's kinds (`rr_in_<label>_…`: the nearest set's, an axis's, a rows copy) are named by the
     label, sanitized.
   - **A membership test of a guest's kind is the guest's.** `(k = "identifier" ; k = "…")` reaches hide as
     `@equals($k, @set(…))`; each of its strings is qualified as one comparison's (`hide`'s `S::Member`), where
     they stayed the run language's (Vue's: none), and `tsx/names`' `bound_name` bound nothing in a piece. Test:
     `a_guest_kind_compared_with_constants_is_the_guests`.
   - **A scan of a source made on lookup holds its values' rows.** A tape value admitted in the fixpoint is a part
     of each chunk source (`values::Layered`); an axis's (`within`, made on lookup) gave its parts' rows to a lookup
     only, at `MADE_AT`, and its scan held the batch's alone. `vcode(_, g, _), has(g, r, S)` planned the axis's scan
     first and found no piece's node. The scan now makes every part's rows once, numbered past the base's (whose
     scan made them all), a segment per round of parts admitted, each written once where it is made (0353); a lookup
     then reads the same rows. Test: `values::tests::a_scan_of_a_source_made_on_lookup_holds_its_values_rows`.
   - 5.5: `vue/script` (the context `script` over the script blocks: `vscript_decl`, `vscript_option`,
     `vscript_prop`, by document) and `vue/expression` (the context `expression` over the template's pieces:
     `vexpr_ref(g, x)`, `vexpr_script_ref(g, x)`, by piece, read through `script`'s `vscript_decl`). The host reads
     them at each piece's host node (`vtemplate_ref(a, x) :- vcode(a, g, _), vexpr_q_ref(g, x)`): rule 1, no rule
     reads another context's nodes, so a host rule's `line(r, l)` of a piece's identifier is nothing. Test:
     `a_components_template_and_script_are_contexts_of_their_own`.
   - 5.6, found: `vue/code`'s template variables (`tvar`, `tref`, `tbind`, `tfree`, read by seven policies) are host
     rules over the pieces' nodes; with a context of the component's code in the program they read nothing (rule
     1). They move into the context `expression`, exported by piece and read at the directive's node. Asking the
     scripts by a grammar name of their own (`vue-script`) kept the two contexts' grammars apart but never ended
     hide's rounds (two names, one label); the scripts stay `typescript`.

   As built (2026-11-22), rungs 5.6-5.7:
   - 5.6: no-template-shadow reads `vue/expression`'s `vtemplate_var(e, x, a, k, l, c)` (a template variable by the
     attribute declaring it and its line and column) and `vue/script`'s names (`vscript_option_name`,
     `vscript_prop_name`, and `vscript_name` under `<script setup>`); `vue/script` reads the host's `vscript`
     (`.from vue vscript for script`), so `vue/component`'s `vcomp` and `vname` find the script's component.
   - **A guest copy runs above the answers.** A request is answered where a stratum reads what it produces
     (`parsed`); the answer admits the documents (the pieces), and the strata above read their rows. A copy hiding
     makes for a guest's grammar (`rr_in_typescript_0_decl_site`, only base atoms) depended on no derived relation
     and could be placed below, seeing no piece: `vue/code`'s `tvar` was empty unless another rule happened to place
     it above. `front::graph` now orders each copy (`GUEST_COPY`) after each produced relation, never negatively,
     and not where the answer reaches the copy (a cycle). Test: `a_guest_copy_reads_the_pieces_a_request_made_in_its_run`.
   - A v-for alias is `const [A]=0`: `let [x]=0` is an assignment to `let[x]`, no declaration.
   - `earlier` was CPython's visit order (`visit_index`, Python documents only): no-unused-vars orders a piece's
     declarations by their bytes (`used_after`). (Since 5.9 `visit_index` is the tape's order elsewhere, below.)
   - 5.7: `the_vue_packs_template_policies_find_their_examples`, alone and beside a plain program: no-unused-vars,
     no-use-v-if-with-v-for, valid-v-for, no-unused-refs, no-template-shadow.
   - **Found (built in rungs 5.7-5.10, below): a pack's Vue policies beside a context.** Beside no-template-shadow, which brings the contexts
     `script` and `expression`, a policy reading the script's or the template code's nodes in host rules finds
     nothing (rule 1: those nodes are the contexts'), and no-unused-refs fails (`no such table: main.vname`). 15 of
     the pack's 23 policies read them so (11 through `vue/component` or `vue/mutation`, 4 through `vue/code`). Rule 5
     needs each to read the contexts: their rules over the script and the template code run in the contexts, and
     their findings stand at the host's bytes (rule 6's context policy, not built).

   As built (2026-11-22), rungs 5.7-5.10 (the user's choice: context policies):
   - **The 15 policies are context policies.** Each policy's rules over the script run in `script`
     (`.context script of vue roots vscript_root exports …`), its rules over the template code in `expression`, and
     the host reports what they export (`.from script dupe_key_in`). A relation both read is two relations, one per
     context (no-mutating-props' `script_mutated_in` and `template_mutated_in`, no-unused-refs' `used` read as
     `script_reads` and `template_reads`): a relation exported from two contexts ran all its rules in both, and the
     script's read `vcode_el` from the host. A template rule reads the script's props through `.from script tcomp,
     tprop, props_var for expression`. `vue/code`'s `tvar_at` is gone: a finding stands at its piece node's own
     bytes (`guest_place` rows for a program's nodes, `guests::GuestRows::document`; rung 5.8).
   - **A member is completed against the library's instances.** A pack member's part is lowered without the
     library's rules, and its rules read a library relation by its own name; the combined program holds it only as
     a context's instance. `contexts::held_instance` names that instance (the reading pass's own, else the one
     context that exports it), `Renaming::rename` reads it, and `Plan::rebind` gives the combine's `Passes` work when
     a rule reads one (before: "Derived vname is not a relation", "no such table main.tvar"). A pass's plan stops at
     what the engine makes on request (`on_request_rule`, `made_on_request`) and at what a made pass reads from the
     host (`from_host`), so neither is made again inside a context.
   - **What a pass does not hold gives nothing there.** A library relation a policy extends from two contexts
     (`vue/mutation`'s `vmut_ask`) reaches, from one pass, a relation only the other holds (`vprops_call`, held only
     as `rr_pass_script_vprops_call`): `contexts::unseen`. A rule that reads one gives nothing and is not written; a
     negation of one holds and is dropped (before: "no such table: main.vprops_call" beside no-mutating-props).
   - **A node a relation carries is not guarded.** `view_nodes` guards only the node variables a view alone binds:
     a node a relation of the program binds is already the pass's, or a host node it carries on purpose
     (`tfree(r, x, a)`'s attribute `a`). Guarded, a template rule's `docof(a, d)` found no file to join the
     script's props on.
   - **A request reads what its readers read.** `rr_value_req` and a reader's ask (`rr_value_req_by_<reader>_<col>`)
     made for a context's rule ask of the context's nodes: hide's `as_read` keeps their guest facts, where it named
     `kind(s, "string")` in the host's grammar and no string of a piece was asked a value. Hide joins a rule's
     definite columns' grammars first: `value`'s node column may hold an `eslintDirective` comment's node (a library
     asks for those), and joined with `vexpr`'s TypeScript node it left `$e` of no grammar.
   - **A batch's long-text nodes are kept when empty.** `values::Values::add` skipped an empty source, and the
     binding a node read goes through (`LongTexts::of`) then looked a piece's string up in the chunk's batch, not its
     own: `value` of `props: ['foo']`'s string was none.
   - **`visit_index` is the tape's order outside Python**, and a guest's nodes have theirs (`tree::Tree::visits`):
     `earlier` held of no TypeScript node, so no-dupe-keys found no repeated key and no-watch-after-await no watch
     after an await. Test: `visit_index_outside_python_is_the_tape_order` (rerail-tests-lang).
   - Tests: `the_vue_packs_template_policies_find_their_examples` (5.7),
     `a_finding_at_a_pieces_node_stands_at_its_own_bytes` (5.8), `the_vue_packs_script_policies_find_their_examples`
     (5.9: no-async-in-computed-properties, no-dupe-keys, no-export-in-script-setup, no-mutating-props,
     no-ref-as-operand, no-setup-props-reactivity-loss, no-side-effects-in-computed-properties,
     no-watch-after-await, require-valid-default-prop, return-in-computed-property), and
     `a_policys_rule_of_a_librarys_context_reads_the_librarys_relations` (5.10). Each case runs alone and beside
     another policy, and its findings are the same. `a_components_code_is_pieces_made_where_a_rule_asks` reads the
     pieces' nodes in the contexts now: with `vue/component`'s contexts in the program a host rule reads none (step 4).
   - **Step 5's gate** (2026-11-22, an M1 Max):
     - corpus0 × B-now first gave 426 findings: B009, B010 and PYI034 lost in files with a quoted annotation. A
       value's long-text part was registered under its places' hosts' keys too, so, kept when empty, it took every
       host node's value read to a string annotation's batch. A value's long-text nodes now answer for its own key
       alone (`values::Values::add`; test `a_values_long_text_nodes_answer_for_its_own_nodes_alone`, rerail-facts).
     - corpus0 × B-now then gives 463 findings, digest 75193ca656bd, identical to the baseline.
     - The probe gives 20 findings, the same with and without string annotations, the baseline's.
     - The quick tier of rerail-logic, rerail-tests-logic, rerail-tests-lang and rerail-facts (1,603 tests): two
       failures fixed, two data tests not run.
       - Rung 3.2 asserted `$n` guarded in `ann_names(x) :- name_node(n), host_node(n, h), …`. Its node is bound by
         the context's `rr_pass_annotation_name_node`, whose own rule is guarded: the test now checks that, and that
         `$h` is never guarded.
       - The executors' coverage ledger: the bound text match's row counts 72 library files, not 70. The two are
         `vue/expression` and `vue/script`, new in step 5, which include `vue/template`'s match.
       - `vue::view::tests::corpus` and `definition_ids_are_pinned` need pack test data that is not in git
         (eslint-plugin-vue's fixtures, pyflakes' F601.py); the host has none.
6. **Measure.** corpus0 × B-now against the targets (one pack 0.786 T, normal packs 2.428 T, 463 findings,
   d067ec2ebf6f).

   Measured (2026-11-22, 02f9fa20a6, off, one thread, an M1 Max at load averages of 10 to 28; instructions retired,
   the profiling build):

   | Run | Instructions | Target | Findings |
   |---|---|---|---|
   | one pack (policies grouped by language alone, a temporary switch in `tools.rs`) | 0.726 T | 0.786 T | 463, d067ec2ebf6f |
   | normal packs | 1.563 T | 2.428 T | 463, digest 75193ca656bd (the tsv's) |

   Both are under their targets with the same findings. Against this decision's measurement before the passes
   (one pack 0.983 T, normal packs 2.179 T), the narrow copies' cost is gone and more.
7. **Rule 8 across chunks and runs; rule 7 on the memo path.** A pass's tape-local rows of a value are kept under
   the value, and a later chunk or run that places its text is given them and runs none of the pass's strata over it.
   Chosen (2026-11-22, the user: "Value entries"): a memo entry per tape value, keyed by its text and the program,
   holding the pass's frontier rows with a document column; not a block format of its own.

   Done (2026-11-22):
   - **The rewrite** (`preparation::kept::Kept`: in `front_of` for a whole program, before `BaseInputs`; for a pack,
     in `parts::front` on its whole program before it is split, since a pass's relations and their readers may lie
     in different parts). For each kept pass (`kept::groups`):
     - each tape-local relation `R` becomes `R_rr_kd(rr_kd, …)`, led by its value's document;
     - the pass's nodes become `rr_pass_<p>_rr_fresh(rr_kd, n)`, the nodes of its fresh roots;
     - `rr_fresh_roots_<p>(r) :- roots(r), !rr_kept_roots_<p>(r)`: the roots no entry gave;
     - each frontier relation (one a rule outside the pass reads) gets an input of kept rows, `R_rr_kept`, and the
       host's read of it, `rr_read_<p>_<R>`: its made rows and its kept ones;
     - each rule outside reads that read in place of `R`.

     A pass is kept only where its roots are named once and have no holes, no tape-local relation is an output, and
     each tape-local rule binds its document. Otherwise it runs as before, and the profile names why
     (`the passes kept by their tapes`).
   - **What stays as written.** Two names say so:
     - `rr:by-name` (`plan::fields::BY_NAME`): the relations the run reads by name at a chunk's end (the fresh roots,
       the frontier's tape rows, and the host's read, whose document column is the split's location). Merge, inline
       and `columns no reader reads` leave them alone.
     - `front::document_column`: the `_rr_kd` and `_rr_kept` relations' column 0, so the split keeps them file-local.
   - **The engine** (`plan::corpus`):
     - A chunk binds each kept input empty, grown with its values (`Values::add`).
     - A value made in the chunk (`Got::New`) is looked up as `tape:<grammar>:<key>` by its text's digest
       (`value_entry`). That is no statement inside the waves: the run admits each path's newest entry before its
       first wave, and reads what it queued. Its rows are given as the request's answer and bound as the value's part.
     - At the chunk's end, each value whose root ran fresh is written as its entry (`kept_rows`): its roots and its
       frontier rows. The run holds those relations to the chunk's end with its exports.
   - **Rungs.**
     - 7.1 `a_memo_edit_that_changes_no_root_runs_no_stratum_of_the_context` (rule 7 on the memo path): a comment added
       to a file runs no stratum of the pass, and its rows stand; a new annotation text runs it, with the new rows.
     - 7.2 `a_text_a_later_chunk_places_runs_no_stratum_of_the_context` (rule 8 across chunks): two files holding one
       annotation text, each a chunk of its own, do the pass's work of one file, and both files' rows stand. With the
       entry lookup off, the second chunk doubles the work, so the rung fails then.
     - 2.10 now names the pass's relation `rr_pass_quoted_ann_ident_rr_kd`.
   - **Checks.** The pass, context, tape and memo tests of rerail-tests-corpus and rerail-rr-root (48) pass.
     corpus0 x B-now, normal packs, gives 463 findings, digest 75193ca656bd. Its profile keeps the annotation pass:
     23 tape-local relations, 5 in its frontier (`def_outside`, `dotted_expr`, `in_first_iter`, `nameref`,
     `tp_ref`).

## Rule 8, measured (2026-11-22)

The spike after the hub merge (04a01ff961), on 40 hogql files that hold quoted annotations (2 chunks) x the Python
packs, off, one thread, profiled into one store: cold, warm, and an edit of one file (a comment line).

- **Which relations are tape-local.** `contexts::tape_local` sorts each made pass's instances by what their rules read.
  A relation is tape-local when its rules read only views, constants, the pass's node set and the pass's other
  tape-local relations. It is a host's when it reads `host_node`/`guest_of`, a host export, or a relation outside the
  pass. Of the annotation pass's 26 relations, 21 are tape-local and 5 read a host: `cls_early`, `gbefore`, `miss`,
  `walk` and `wstart`, all name resolution. A profiled run reports the split (`the file-local part's made passes, by
  what their relations read`). Rung: `a_passs_relations_are_its_tapes_or_its_hosts`.
- **Within a chunk, rule 8 already holds.** A value is its text's (`tape_value`: keyed by grammar and text). A second
  string holding the text adds a place (`Got::Placed`), and `Grower::admit` admits that place's rows alone. Cold: 214
  strings asked, 93 values made, 89 distinct texts. The other 121 strings only added a place. Rung:
  `one_annotation_text_in_two_files_is_one_value` (one value, one parse, both files' rows).
- **Across chunks, a text is made once in each chunk that asks for it.** 4 of the 93 values were a second chunk's.
  The key is the same across chunks and runs (`GuestKeys`, from the store, decision 0391), but each chunk projects
  the value and runs the pass's strata over it again.
- **Cost.** The pass's strata's plan steps took 2.3 ms of the run's 3,461 ms of strata CPU (0.07%). Admitting the
  values ("tape values admitted", projection included) took under 0.1 s of 6.1 s. Making a value once per run
  instead of once per chunk saves a part of those two figures, and so does keeping it across runs.
- **Across runs.** The warm run is a full memo hit (16 G instructions against 1,041 G cold). It makes no value and
  runs no pass stratum. The edit run (116 G) runs the passes' 26 strata for the edited file's chunk alone. It made no
  value: 0 of its 26 inputs changed. Findings are 231 in all three runs.

What rule 8's keeping would add: each tape-local relation's rows of a value, written once per text into a block
keyed by the value's key (as a file's kept rows are, 0456's one format), and read in place by every chunk and run
that places the text. On this corpus the gain is bounded by the two cost figures above. A corpus where one text has
many places in many chunks would raise the cross-chunk share. hogql has 3,063 quoted annotations over 777 distinct
texts, and 6 chunks.

## Open

Both items closed (2026-11-22):

- **The declarations' names.** `.context`, `.from`, `.export`, `.root` and the qualified read stay (the user: "Keep
  our names"). Soufflé's `.comp`/`.init`/`inst.rel` instantiate rules textually, and say nothing of a document set:
  a context's roots and its host are what these declarations name.
- **Planning.** One plan, the passes' strata ordered by rule 4 among the host's, stays. On the 40-file hogql run, the
  passes took 0.005 s of 0.035 s of planning, 26 of 214 strata and 75 of 561 rules. Neither the plan's size nor its
  replanning cost argues for a program of its own.
