# 0597. A guest context is a pass of its own rules, reading its host's finished facts explicitly

Status: proposed (2026-11-21), from the user: "should we instead do another fixedpoint of annotation/template
analysis AFTER the main program has run, with the facts from the main program as an input?"; "I was already thinking
about having each language run in a different pass ... scan the code inside f-strings and handle DSLs like vue's
html-like code"; "yes, an explicit pass"; "if we want to check annotations and f-strings in Python, we will need to
run an extraction pass first as a fixedpoint or after and then check them". Would replace 0058's annotation range and
0165's narrow copies and fold for guests; keeps 0431 (guests are tape values, no guest documents, no batch built
again), 0353 (blocks never copied), 0442 (no global state).

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

3. **Extraction is rules of the host, in the host's fixpoint.** Which strings are annotations depends on the host's
   analysis (`cast("Foo", x)` needs `cast` resolved to `typing.cast`), so a pass's roots are derived by its host:
   `.root annotation(s) :- string_annotation(s), …`. A root is one of two kinds:
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
2. **The engine.** A pass's roots as its documents: text roots as tape values (0431's admission, unchanged), region
   roots bounded at each read; the views bounded to a pass's roots; a pass's `.from` inputs read in place in the
   earlier pass's blocks; the strata of every pass in one order, on the CPU and the device (one launch per pass's
   stratum, bounded). Tests: a context's rules see its roots' nodes alone; a host stratum after the pass reads its
   exports.
3. **The Python annotation context.** `lib/python/annotation.rerail` from the annotation rules of `names.rerail`,
   `typing` and `ann_*`; the host's readers of annotation facts read `annotation.*`. Tests, kept and passing:
   `quoted_annotations_are_read_as_their_code`, #923's `a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone`,
   C408's `a: "dict({})"`, a nested annotation, R9's placement.
4. **Removal.** Hiding's annotation range and copies, `FoldOwn`, the narrow copies, `combining::one_document`, and
   their tests, ported to the passes' (removal is removal: no switch to the old path).
5. **f-strings and Vue.** `python/format-spec` and SQL in a template; `vue/expression` reading the script's bindings.
6. **Measure.** corpus0 × B-now against the targets (one pack 0.786 T, normal packs 2.428 T, 463 findings,
   d067ec2ebf6f).

## Open

- The declarations' names (`.context`, `.from`, `.export`, `.root`) and the qualified read (`annotation.ann_type`),
  against Soufflé's own components (`.comp`, `.init`, `inst.rel`), which instantiate rules textually but say nothing
  of a document set.
- Planning: decided interleaved (2026-11-21, the user: "try interleaved planning then"): a pass's strata are planned
  in one plan with its host's, ordered by rule 4; a program of its own, with its own plan and image, stays the
  alternative if one plan's size or its replanning costs show.
