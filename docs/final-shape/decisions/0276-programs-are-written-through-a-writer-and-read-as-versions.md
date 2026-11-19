# 0276: Programs are written through a writer and read as versions, from the tape on; `lower::Tree` and `front::Program` go

Status: draft (2026-11-15), a design. Drafted as 0274; renumbered (0274 and 0275 are taken in other worktrees). Builds on 0173 (the program image), 0241 (a maintained form is a
derivation), 0244 (a split part is a view with its fields overlaid) and t36 item 7 (the run reads versions). Aimed
at 0266's `Program` block (§1, path step 3), whose writer, views and base + delta it takes its API from, so that
step 3 changes what is under the API and not its callers.

## Intro

`lower::Tree` is the lowered program's tree beside its version. It exists so that code written against
`front::Program` keeps working while the run moves to versions: a tree is given, or decoded from a version on the
first read, and each read is counted (`version_trees_read`). After t36 item 7 a maintained run reads no tree.
Everything else still does:

- **Preparation:** every pass from `lower` to the split rewrites the lowered program as a tree.
- **Other run paths:** a run that does not maintain, the cross memo, the cross rounds' bounded program, the SQL
  emitter, the session's diagnostics.
- **Parallel statements:** the edits already made on versions also state themselves on the tree (`qualify`'s
  fallback, `Cut::program`'s, the input deltas', the cross memo's), for the programs that have no version.

So every program the run holds has two forms that must agree, a debug check that they do (`same_as_tree`), and a
decode wherever a reader was not ported. And before `lower` there is a third form. A rule file's tape becomes
statements (`rr/from_tape.rs`), then a `front::Program` tree (`rr::lower`), and the front passes rewrite that tree
until `plan::lower` makes the first version.

This decision gives a program one form from its tape on: a version, made only through a writer and read only through
views. `lower::Tree` goes, and so does `front::Program`, as 0246 §1 and §4 already decided for loading: the loader
walks the tape once and writes into the image builder, and the preparation passes are derivations of the linked
base. With them go `Tree::{given, of, beside, maintained}`, `TREE_READS`, `same_as_tree`, `made_from`'s `had`, every
tree fallback, the statements, and the `front::Program` → version encoding.

## What was found

Surveyed on 2532b2a90. A site is a line that reads or edits a lowered program's tree.

| Where | Sites | What they do |
|---|---|---|
| Preparation, `corpus.rs` 2750–2810 | 14 | `bound`, `push_local_joins`, `factorize_exports`, `demand_exports`, `locate`, `merge_identical` / `merge_and_drop_repeats`, the inputs no rule reads, `RERAIL_RULE_ORDER`, `file_local::programs` |
| The prepared program's readers, `corpus.rs` | 43 | `lowered.program.decls[rel].len()` and its types (24), `constant_relations`, `cross_causes`, `embed_slice`, `Stats::of_schema`, `Own::constants`, `lower::domains`, `constants_tested`, `cone_digests` |
| The split's parts, `corpus.rs` | 15 | `split.local`, `split.cross`: dumps, the cross memo's `split`, the parts made per run (`part_of`), the image made where a part has no version |
| The run's parts, `corpus.rs` | 59 | the constants' part (3159–3191), the slice, the memoised count, the cross part's types, dumps, `cross_rounds::bound`, the cut, the input deltas, `qualify`, the maintained part's diagnostics (6990–7125), `recursive_spike::run`, the bounded rounds (7396–7513, 8195–8310) |
| `maintain.rs` | 11 | the rewrite's fallback part, the gates test, `input_views`, the `O125_RULES` dump |
| `retain.rs` | 5 | `Cut::program`'s tree path, `input_views` decoding a version (315) |
| `cross_memo.rs` | 6 | `drop_guards`, `keep_copies`: tree edits stated beside the version's |
| `emit.rs`, `plan.rs` | 8 | the SQL emitter (`emit_inputs(&lowered.program)`, 140 lines), `decode_kept`, `outputs` |
| `plan.rs` aids | 9 | `version_trace` notes, `analysis_of` (the planner's analysis reads a tree) |
| `prepared.rs` | 2 | `prepared_of` makes `Tree::given`; `remember` stores the lowered tree's text |
| `file_local.rs` | 5 | `Programs::{local, cross}` are `Tree::beside` |
| `lower.rs` | 8 | the type, `Lowered::placed`, `digest`, `tree_maintained` |
| Tests | 16 + | `lower::lower` then `Lowered { program: Tree::given(..) }` |

Two more facts shape the design:

- **Every part is made per run from the prepared one,** and each carries `domains: lowered.domains.clone()`, a map
  of every relation's column domains (`part_of`, `Cut::program`, `retain`, `maintain`). The domains are the
  program's, so they belong to it, not to each copy.
- **The hub has 0246 stages 3 and 7, which this branch does not:** programs lowered in their libraries' scope as
  module images, and a stored program kept as its image (`program_text::store`). Both change `lower`'s input and
  `remember`'s output. This port starts after that merge (step 0).

## Decision

### 1. One form from the tape on

The boundary is the tape (0061, 0162). The lean parser makes it, and the loader reads it in place. Nothing after
that holds a program as a tree:

- **Lowering** walks a module's tape once and writes its clauses' core rules through `ProgramW::lowering()` (0246
  §1, per clause). The whole-program half (the findings' rules, `declare_core`, kind sets, `front::finish`, aggregate
  demand) runs at link time over the linked base (0246 §2).
- **What a pass reads** is a view, `Placed` and its `Fields`, plus the analyses as relations (0246 §3).
- **What a pass makes** it writes through `ProgramW`, which freezes to a `Placed`.
- **A program is printed** from its version (`Version::show_rule`, §6), never by decoding it to a tree.

`plan::lower` stops being a boundary. It becomes one derivation among the preparation passes: it interns the atoms,
fixes each column's domain and writes the ops.

### 2. The data structures

```rust
/// A program the run evaluates: its version and what its run reads beside it. Replaces `lower::Lowered`.
#[derive(Clone)]
pub struct Part {
    pub program: Placed,             // the version, its order, its maintained parts (`Maint`), its fields' cache
    pub lowering: Arc<Lowering>,     // shared by every part made from one prepared program
}

/// What lowering fixed for a whole program, shared by its parts. Today `Lowering<'l>` borrows these from a
/// `Lowered`; it becomes the owned form, and `Lowered`'s `domains`, `ops` and `dialect` move into it.
pub struct Lowering {
    pub domains: Domains,            // §8: moves into the declarations
    pub ops: [i64; 3],
    pub dialect: GrammarDialect,
}

/// The prepared program (`Prepared`): versions only. Its key is the linked base's digest (its modules' image keys)
/// with the run's settings, not a hash of texts and of `{:?}` prints of the facts, inputs and outputs.
pub struct Prepared {
    pub key: [u8; 32],
    pub lowered: Part,                        // the whole prepared program
    pub split: Split,
    /* vocabulary, sets, matchers, reports: unchanged */
    pub cone: OnceLock<..>, pub maintained: OnceLock<..>, pub parts: OnceLock<..>,
}

/// The split (`file_local::Programs`): its parts as versions, never trees, and what it found, by name.
pub struct Split {
    pub local: Placed,                        // a view of `lowered`'s version, its fields overlaid (0244)
    pub cross: Placed,
    pub names: SplitNames,                    // exports, base, locations, same_doc, file_local, constants
}

/// The cone of requests (`Cone`): versions only. The `trees` field goes.
pub struct Cone { pub load: Placed, pub reads: BTreeSet<String>, pub rest: Placed }
```

- **A version is required.** `Programs::versions: Option<Parts>`, `Lowered::version: Option<_>` and the image's
  "rules alone" form (`Placed::made`, for "a comparison it does not know") go. Where the image cannot hold a
  program today, it is extended to hold it. A program it still cannot hold fails preparation with the construct
  named. Step 1 lists those constructs; the survey found none on dl1 or the session tests.
- **`Part` is cheap to clone:** an `Arc<Version>`, the `Maint`'s `Arc`, the order and the `Lowering`'s `Arc`. Making
  a part per run no longer copies the domains.

### 3. The writer: `ProgramW`

`ProgramW` is the writer class: the only way to make a version. It is a thin owner of today's `Builder`, with
`Builder`'s scattered setters grouped, the bulk calls added, and the result always a `Placed`. Its shape is 0266
§8's: open a writer against a base, write with bulk and single calls, then freeze to something shareable.

```rust
// Opening: what the result is made of.
let mut w = ProgramW::edit(&p);       // an edit of `p`: removals make a view, additions an overlay (Builder::on)
let mut w = ProgramW::derive(&p);     // a pass that rewrites `p`'s rules: a new base under `p`'s ids (0241)
let mut w = ProgramW::lowering();     // `lower`'s own: a new base

// Rules, in bulk. A pass states what it keeps, copies or rewrites; it never walks a rule's nodes itself.
w.keep(|r| !clean.contains(name(r)));        // removals only: a view, no rule copied
w.copy(&p, p.rules().filter(..));            // contiguous runs copied as node ranges, not rule by rule
w.copy_all(&p);                              // every rule, as one range per run of the base's ids
w.rewrite_all(&p, &mut rw);                  // each rule through one `Rewrite` (normalize, ungate)
w.rewrite(&p, r, &mut rw).finish()?;         // one rule through a `Rewrite`; more literals may follow
w.rule().head(..).atom(..).finish()?;        // a rule written term by term (`RuleBuilder`, as today)

// Fields: one sub-writer, by kind. Each call says what it replaces or adds.
let f = w.fields();
f.declare(rel, &cols);  f.declare_like(new, old);  f.undeclare(rel);
f.inputs().push(rel);  f.inputs().retain(|r| ..);  f.outputs().set(&names);
f.qualifiers().add(&words);  f.answers().set(&map);  f.gates().add(&gates);  f.nullary().add(&rels);
f.keep_relation_fields(natives, lattices, gates);   // 0244's part fields

// Freezing.
let q: Placed = w.freeze()?;                  // whole: its rules in the writer's order
let q: Placed = w.freeze_with(maint)?;        // a maintained form, with its parts (0215)
```

What it adds to `Builder`:

- **Bulk copies.** `copy` and `copy_all` copy runs of consecutive rule ids as one node range when the writer shares
  the source's ids (`Builder::same`). Today `b.copy(v, r)` copies rule by rule, and a derivation copies every rule
  it keeps that way. This is 0266 §3's range copy (`copy_stratum`, `carry`) for programs.
- **`rewrite_all`,** so a pass that applies one rewrite everywhere (normalize, `RoundProgram`'s ungating) is one call.
- **`fields()`.** Today's setters are `Builder` methods spread over `build.rs` and `about.rs` (`set_inputs`,
  `set_answers`, `add_tree_gates`, `keep_relation_fields`, `add_qualifiers`, `set_qualifiers`, `declare`,
  `undeclare`, `set_facts`, `add_nullary`). They become one sub-writer, grouped by field. That is the surface 0266's
  `Program` block puts in its root.
- **`freeze` returns a `Placed`,** with its `Fields` cache empty. `made_from` goes, because a writer cannot fail to
  make a version: an error is an error, never a quiet fall back to a tree.

A pass is a function `fn(&Placed, ..) -> Result<Placed>`, from a view to a frozen version. The profile span and
the version trace (§6) are taken by one wrapper, `pass("locate", &p, |p, w| ..)`, not by each pass.

Programs are written by the host only. `ProgramW` is not `Send` and has no claims. 0266's GPU writing is for
`Facts` and `Outputs`.

### 4. The views

Readers use `Placed` and `Fields`, which already exist. The bulk readers the sites need are added to them, each
computed once per version in `FieldsData`:

| Today | View |
|---|---|
| `prog.decls[rel].len()`, `prog.decls[rel]` types (24 sites) | `f.width(rel)`, `f.types(rel)` |
| `prog.rules.iter().map(|r| r.head.rel)` | `f.heads()` (a set, cached) |
| every relation a rule reads | `f.read()` (a set, cached) |
| `front::components_of(prog)`, `rules_by_stratum` | `p.components()`, `p.rules_by_stratum()` |
| `prog.inputs.contains(r)`, outputs | `f.is_input`, `f.is_output` (as today) |
| rule loops | `p.rules()`, `v.heads(rel)`, `v.readers(rel)`, handles |
| `answers::produced(prog)` | `f.produced`, `f.request_of` (as today) |

`Placed` is 0266's merged view. A base is a version with no delta, so readers have one type and never ask which
form they hold.

### 5. Every call site

**Loading and the front passes** (`rr.rs`, `rr/lower.rs`, `front.rs`, `simplify.rs`, `magic.rs`, `reach_form.rs`,
`strings.rs`, `corpus.rs` 2560–2745). This is about 10,000 lines of tree code, most of the port:

| Site | Port |
|---|---|
| `rr/from_tape.rs` (statements off the tape) | goes: the loader reads the tape in place (0162's `View`), as `program_text/from_tape.rs` already does |
| `rr::lower::lower`, per-clause half (3,790 lines with the other half) | writes core rules through `ProgramW::lowering()` in the tape walk, form by form (0246 §1). Until a form moves, its tree output is the check, compared by digest |
| `rr::lower`, whole-program half; `front::finish`; `aggregate_demand::optimize` | passes over the linked base (`derive`), at link (0246 §2) |
| `tape_type::check`, `check_names`, `front::declared` | readers of the module image, once per module (0246) |
| `world::wanted`, `cross_memo::declared`, `cross_memo::add` | readers on `Fields`; `add` is an `edit` |
| `prepared_key` | the linked base's digest and the settings (§2) |
| `cpython_functors`, `base_inputs` | `edit`: rewrites of the rules that call the functors; `f.inputs().push` |
| `front::prune` (3–4 times) | `edit` + `keep` over one reachability relation (0246 §3) |
| `simplify::simplify` (2,797 lines) | a `derive`; congruence is one maintained relation (0246 §3), and the rewrite reads it |
| `reach_form::find`, `undo_in_cycles` | `derive`; the closures' rules rewritten to the bitmap relations |
| `magic::demand_negated_tests`, `demand_bound_columns` | `derive`; magic rules written term by term, adorned copies by `rewrite` |
| `matchers::absorb`, `matchers::lower` | `derive`; the absorbed parts' literals removed through a `Rewrite`, the matcher relations declared |
| `strings::node_subject_tests`, `long_text_twins`, `strings::as_relations` | `derive`; twins by `copy` + `rewrite` |
| `plan::lower` (`lower::lower`) | a `derive` that interns atoms and writes the domains and ops; returns `Part` |

**Preparation after lowering** (`corpus.rs` 2745–2810, `file_local.rs`, `lower.rs`):

| Site | Port |
|---|---|
| `bound` | a reader: on `Fields` |
| `push_local_joins` / `push_with` (231 lines) | a pass: `derive`; each pushed rule rewritten (the component replaced by its helper's atom), helpers written term by term; the fallback (`original`) is the input view, unchanged |
| `factorize_exports` (204 lines) | a pass: `derive`, the export's rules replaced by its parts' |
| `demand_exports` | a pass: `derive`; each cut export's rules rewritten |
| `locate` (127 lines) | a pass: `edit`; a located relation's rules `keep`-removed and written again as `r_at`, `read_at` added, `r_at` declared |
| `merge_identical`, `merge_and_drop_repeats` | a pass: `derive`; a merged relation's readers rewritten by `Rewrite::atom_as`, its own rules dropped |
| the inputs no rule reads | `f.inputs().retain(|r| read.contains(r) || f.is_output(r))` on an `edit` |
| `RERAIL_RULE_ORDER` (aid) | `derive` with `copy` in the order asked |
| `file_local::programs`, `assemble`, `split_located`, `split_memo`, `constant_relations`, `cross_causes`, `reads_no_document` | readers on `Placed` / `Fields`; `assemble` makes `Split` by `Version::part` (0244), as it does today beside the trees |
| `embed_slice` | a reader making a part: `edit` with `keep` |
| `Stats::of_schema`, `Stats::empty`, `Own::constants`, `constants_tested`, `cone_digests` | readers on `Fields` |
| `lower::domains` at 3416 | the prepared `Lowering`'s |

**The prepared program** (`prepared.rs`):

| Site | Port |
|---|---|
| `remember` | stores `lowered`'s image (0246 stage 7's form, under its bytes' digest), not the tree's text |
| `prepared_of` | reads the image back to a `Placed` (`Version::open`), and the split by `assemble` on it |

**The split's parts** (15 sites): `split.local` and `split.cross` are `Placed`. `Placed::of_program(&split.cross)`
(2877, 2881, 3076) goes, since the version is always there. `dump_split` prints them (§6). The cross memo's
`split(&mut split.cross, ..)` takes and returns a `Placed`.

**The run's parts:**

| Site | Port |
|---|---|
| `part_of`, `part` (3021) | `Part { program, lowering: prepared.lowered.lowering.clone() }` |
| constants' part (3159–3191) | its version path; the tree fallback (`(*local.program).clone()`, `rules.retain`, `inputs.push`) goes |
| `slice` (3211) | `Part` of `embed_slice`'s view; `slice_reads` from `f.read()` |
| `kept_part_on` | done (t36) |
| memoised count (6434–6445), the cross part's types (6450) | `p.len()`, `f.read()`, `f.types` |
| `RERAIL_*_DUMP` (6497 and others) | aids (§6) |
| `cut.program` (6740, 7163; `retain.rs` 346–379) | its version path only |
| input deltas (6782–6792) | its version path only; `f.declare_like(name, rel)`, `f.nullary().add` |
| `qualify` (9618–9630), `maintained_cross` (9670) | their version paths only; `requalified` goes |
| maintained part's diagnostics (6990–7125) | on `Fields`: `f.width`, `f.decls()`, `p.components()`, `p.rules_by_stratum()`, `f.heads()` |
| `cross_memo::drop_guards`, `keep_copies` | their version edits only (`edit`: `keep` and a rewrite of each guarded rule); the tree edits go |
| `recursive_spike::run(&cross.program)` | retired with the spike (§9) |
| `cross_rounds::bound`, `cone_split`, `pre_split`, `run_rounds`, `round.program` (7396–7513, 8195–8310), and the `hops` check each run makes (6338, 6499) | retired with the bounded rounds (§9) |
| `maintain::rewrite`'s fallback part, gates test, `input_views` | its version path; `f.gates().is_empty()`; `input_views` on handles (also ends `retain.rs` 315's decode) |
| `emit::emit_inputs` (140 lines), `emit::outputs`, `plan::decode_kept` | the SQL emitter reads `Placed` through `Body`, as `plan::run`'s kernel does; `decoded_outputs` reads `f.is_input`, `f.width` |
| `Model::analysis_of` (`plan.rs` 1726), `plan::program_digest` | on `Placed`: the planner's analysis reads the strata, heads and reads from the views, and the digest is the version's |
| `sql_executor/compiler.rs`, `coverage.rs`, `emit.rs` 279 | take the loader's `Part`, not a `front::Program` |

**Tests:** a test that writes a program as `.rr` text loads it through the loader (`Part::load(src, dialect)`, a
helper with its own interner and store). Assertions on rules compare `show_rule` lines, which print what `front`'s
`Display` prints today, so expected strings stay. Tests that build a `front::Program` by hand write it through
`ProgramW` instead. The `rr/tests.rs` lowering tests (3,492 lines) compare the loader's printed rules.

### 6. Printing, not decoding

A version prints itself: `Version::show_rule`, `show_atom` and `show_lit` (`program_image/show.rs`) write the text
`front`'s `Display` writes. `Placed::show()` prints a whole program by them, declarations and fields included.
Nothing turns a version back into a tree, so `Placed::program(&lists)` goes with `front::Program`.

The measurement aids print this way: `version_trace`, `RERAIL_SPLIT_DUMP`, `RERAIL_DEMAND_DUMP`,
`RERAIL_DUMP_MATCHERS`, `O125_RULES`, `RERAIL_MEMO_PROGRAM_DUMP`. The run's counter `version_trees_read` and
`run_state`'s entry for it go with `lower::Tree`. A guard test refuses `front::Program` and `lower::Tree` in the
engine's sources, so neither can come back.

What stays in `front`: the term and name types the image's handles and the rr parser share (`Name`, `Value`,
`AggOp`, `Lists`), and `front/cycles.rs` and `front/name.rs` where they read versions.

### 7. Base and delta, in 0266's terms

| Today (`program_image`) | 0266 `Program` block | In this port |
|---|---|---|
| base | full block | `ProgramW::lowering`, `derive` |
| view (removals, few ranges) | a delta with removals | `edit` + `keep` |
| overlay (additions, fields) over any version | a delta over a full block, never a delta | `edit`; the writer folds an overlay of an overlay into one cumulative delta over the full block, copying the old delta's additions whole (0266 §6's carry) |
| fold at half the parent | roll-up by the cost rule | unchanged in this port; the cost rule is §8 |
| a module image, linked (0246 §2) | a module nested in the `Program` block | linking appends module images; a pass over the linked base is a `derive` |

Callers see none of this. They open, write, freeze and read `Placed`. So 0266 step 3 can put a version into a block
without changing a caller.

### 8. Afterwards, not in this port

- **Domains in the declarations.** A declaration's columns gain their domain, and `Lowering` keeps only `ops` and
  `dialect`. This is 0266's `Program` root, where declarations are nested whole.
- **Roll-up by the cost rule** in place of the fold at half, measured against today's fold on dl1's session runs.
- **The whole-program questions as maintained rules** (0246 §3, 0225 §5). This port reads them from the views as
  passes compute them today. Moving congruence, reachability and the strata to rules changes what a pass reads, not
  how it writes.

## Path

Each step leaves the engine working, runs its targeted tests, and is measured on dl1 (findings digest and retired
instructions) against the step before. Until a pass has moved, its tree form is its check: the moved pass's printed
rules must equal the old pass's, compared by digest, as 0246 §1 does for lowering's forms.

0. **Merge the hub's 0246 stages 2, 3 and 7:** module images, programs lowered in their libraries' scope, and the
   stored image.
1. **`ProgramW` and `Part`.** The writer over `Builder`, with `copy`/`copy_all` as range copies, `rewrite_all` and
   `fields()`. `Part` and the shared `Lowering`. Port the existing version passes to them: supports, gate, split,
   the round program, the cut, `qualify`, the input deltas, the constants, the cross memo, the maintenance rewrite.
   Delete each pass's tree fallback and `made_from`. List what the image cannot hold, and extend it. Check: the
   session, cone, cross memo and maintenance tests.
2. **The run holds versions only.** `Split`'s parts are `Placed`. `Cone` drops its trees. Port every run reader in
   §5: the diagnostics, the emitter, `decode_kept`, `input_views`, `analysis_of`. Printing by `Placed::show`. Retire the
   bounded rounds and the spike (§9). Check: corpus tests, the SQL extension's tests, the guard that a non-maintained run reads no
   tree too.
3. **Preparation after lowering on the writer.** `plan::lower` becomes a derivation from the front's version (encoded
   once from its tree, until step 4). The readers come first (split, constants, causes, statistics), then the
   rewriting passes, one commit each: `locate`, the merges, `demand_exports`, `factorize_exports`,
   `push_local_joins`. `remember` and `prepared_of` store and read the image. Check: the `file_local` tests
   (`rerail-device-metal`), the prepared tests, the phase tests. Then delete `lower::Tree`, `Lowered`,
   `TREE_READS`, `same_as_tree`.
4. **The front passes on the writer,** last first, so the encoding point moves earlier one pass at a time:
   `strings::as_relations` and the twins, the matchers, demand (`magic`), `reach_form`, `simplify`, `prune`,
   `cpython_functors` and `base_inputs`. `prepared_key` becomes the linked base's digest.
5. **Lowering from the tape** (0246 §1): the per-clause half of `rr::lower` writes through `ProgramW` in the tape
   walk, form by form; the whole-program half runs over the linked base. `rr/from_tape.rs`'s statements go. Check:
   `rr/tests.rs`, the tape tests, dl1.
6. **Delete `front::Program`,** its statements and its encoding. Add §6's guard.

Progress (2026-11-18):

- **Step 3's fields.** Every pass reads and edits a program's fields through `program_image::fields`
  (`ProgramFields`, `FieldsMut`) on the version, a builder or an edit; `Builder::set_header` is gone, and so is
  every header a pass decoded. Preparing by parts selects a part's fields over the version (`parts::Own`).
- **`finish`'s checks** (work queue Q83) read the version: `front::finish_checks_on`, with `cycles::check` over
  the rules' handles and its strata from `front::graph`. The tree's check ran beside it on every test program
  bringing a program to `finish`, agreeing, and is deleted.
- **Step 5, first part.** Every text lowers on versions: one module written in the walk, linked through the
  lowering's tail (`rr::link_library_versions`); the tree's `lower`, `link`, `link_in`, `link_seeded` and `whole`
  are gone. The run reads its programs as versions (`rr::parse_version`), and a program handed to it is one.
- **Step 5, second part.** The tail's tree twins are gone, their tests checking the version passes. A module is its
  version, rules and header: `seed` and the link read a module's header there, `relinked` renames it there, and an
  image is that version with its `module_*` rows (no header decoded). The lowerer's header is a builder written
  through the fields API, and the walk writes its rules onto it. Left of step 5: each clause still builds a
  `front::Rule` that the builder encodes (`Builder::add`), and the walk reads `rr/from_tape.rs`'s statements.
- **Step 6's order** (the user, 2026-11-18: step 6 first, then every intermediate form goes). Bottom-up, each
  stage committed when its tests pass under Relax:
  1. *The run's trees.* Policy combination (`corpus::combine*`, `scoping::guard`, `hiding`, `merge_and_drop_repeats`,
     `drop_repeated_rules`) on versions; the dialect runs (`run_dialects*`); phases', the bridge's and `tools`'
     handed trees (`handed_version` goes); `waves` (`policies`, `schedule`, `matches`, its `magic_sets`); typing's
     split (`file_local`, `split_memo`, `export_demand`, `shape`); the SQL executor's and the vtab's readers; the
     memo (`corpus/memo.rs`).
  2. *The tree twins.* `simplify.rs`, `magic.rs`, `reach_form.rs`, `strings`' tree passes, `matchers`', `front`'s
     passes (`prune`, `strata`, `declared`, `subsume_requests`, `finish`) and `lower::sources_on_request`: each one's
     tests ported to its version pass, then the tree form deleted.
  3. *The converters.* `json_rules/*`, `front/yaml.rs`, `convert.rs`, `program_text.rs` write through the builder.
  4. *Tests* that build or read a `front::Program` load text (`Part::load`) or write through the builder, and
     compare `show_rule` lines.
  5. *The forms.* `front::Program`, `program_facts`' encoding of it (`Builder::of_program`, `set_program_header`,
     `about_of`), the decoder (`program_image::Tree`, `Version::program`), `lower::Tree`; §6's guard.
  6. *The intermediate forms* (step 7, the user's): `front::Rule`, `Literal`, `Atom`, `Term`; `rr/from_tape.rs`'s
     statements. The clause lowering writes through the builder from the tape, form by form (§5).
- **Step 6, stage 1 so far** (2026-11-18): the combine is on versions only (its tree path, the tree `link::Part`,
  `scoping::guard` and the tree tail passes gone); phases and the bridge read and hand versions; the whole-run
  cutoff's `Summary` reads the combined version's fields; the SQL emitter reads `Placed` (`emit_inputs`), and
  `lower::lowered` lowers a version as the run reads it, for the tests and tools that run a program as written.
  The tree simplify's 22 tests run on its version passes. Still on trees: `rr::read_file` (the tools' path), the
  waves (their demand programs and the `json_rules` they join), the SQL extension's front (`sql/program.rs`), the
  planner's tree entries (`body::alone`, `Stats::empty`, `Placed::of_program`), the lowering's tree form
  (`lower_tree`, `Given`), and the tree twins of stage 2.
- **Step 6, stage 2 so far** (2026-11-18): typing's split, hiding (`hide`, `fold_own`), the merges
  (`merge_and_drop_repeats`, `drop_repeated_rules`) and the whole of `simplify.rs` are the version's only. Each tree
  function's explanations moved onto its version pass, and its tests run on the version passes (the corpus-wide
  check of dropped unreached clauses too, its Q3 property read off `congruence`). The memo's dump prints
  `Version::show`. Since then: `magic.rs` is gone (T's `magic_sets` is `KeyDemand`, the gated adornment
  `ProgramDemand { gate }`, their 15 golden combinations checked on the version), and so are `reach_form.rs`'s,
  `strings`' and `matchers`' tree passes. The waves are versions: a pack's policies are one version
  (`front::yaml::program`, the round trip read back by `yaml::documents`), the wave's rules and demand program are
  written through the builder, rules of one shape merge by `shape` over slots, and the wave's source joins them as
  versions (`matches::joined`, keyed by `Placed::whole`). Still to go in stage 2: `front`'s passes and
  `lower::sources_on_request`.
- **Stage 1, the planner** (2026-11-18): the native fixpoints (`native_fixpoint`'s lattice parts, the set
  fixpoint's edge tables) read rules through their handles and write their parts through the builder
  (`made_parts`); `Run::tree`, `identities_of` and `Leaves`' tree entries are gone; the version trace, the inline
  and run dumps print versions. The findings layer reads its tables from its version.
- **Stage 3, `program_text`** (2026-11-18): a stored program is its image only. The `.rr` text form of 0155 and its
  tape reader of 0162 (`print`, `read`, `program_text/from_tape.rs`) read and wrote the tree, and are gone with it,
  as is `RERAIL_PROGRAM_IMAGE` (`off` kept texts, `require` refused them). A program a store kept as text alone (a
  store written before images) is not read: `stored` leaves it out, the run compiles it again, and its source stays
  named (`sources`) so collection keeps it with the run that named it. Plan choices and the combined program are
  written by `encode_version` and opened from their images.
- **Step 6 done** (2026-11-18): `front::Program` is gone, with its encoding (`Builder::of_program`,
  `set_program_header`, `about_of`, `program_facts::write`/`program`), its decoder (`Version::program`,
  `Tree::program`), `print(Program)`, the tree decoders of `rr` (`parse`, `parse_own`, `parse_file`, `lower`,
  `lower_in`, `Library::program`, `with_cone`), `front`'s text parser and tree passes, the planner's tree keys
  (`Versions::with`, `key`, `body`; `Placed::of_program`, `program`, `made`), the converter's tree overlay and the
  tree's `sources_on_request` and `undo_on_request`. Each one's explanations moved onto its version twin (a field's
  onto its `program_facts::Field` variant). Every test reads versions; `examples/versions.rs`, which measured the
  tree against versions, went (0173 names its last commit). §6's guard is `front::tests::no_tree_program_comes_back`.
  Left: the intermediate forms (stage 6): the clause lowering still builds `front::Rule`s for `Builder::add`, reads
  back its helpers through `program_image::Tree::rule`, and walks `rr/from_tape.rs`'s statements.
  The hub's merge (65e368a0b) brought the product-demand pass (`BoundProducts`), which rewrites trees: it reads its
  version through `product_demand::Decoded` (each rule as `Tree::rule` gives it, and the fields its proofs read)
  and writes through the builder. It is the last tree pass on the preparation path, and goes with the forms. The
  rank form's tree entry (`rank_form::find`) went, and its tests moved onto the version's pass.

- **Stage 6, the intermediate forms: the plan (user, 2026-11-18: "work through all of these").** The goal is
  0246's: a module's tape is walked once, and the walk writes its image. The tape is read where it is, and nothing
  builds the program in another form or decodes the image back into one. Four steps, each in commits verified
  under Relax:
  - **A. The walk writes the image.** A1: a file is carried as its tape (its text and nodes), never as
    statements. Each directive writes its field through the fields API as the walk reaches it, and every reader of
    statements (includes, the checks, testing, the type check) reads the tape's statement nodes. `from_tape`'s
    statement types go. A2: the clause lowering reads the clause's tape nodes. A disjunction's alternatives are
    positions in the tape, and a rule is written by walking the chosen positions. A3: each rule is written term by
    term with the builder's rule writer, so the lowering builds no `front::Rule`.
    - *Done (A1, two commits).* The lean image is read in place (`from_tape::Tape`). A file is an `RrFile`: its
      text, its tape and the tape's code keys, checked once when read, and shared by the process's library cache
      (`read_shared`). A statement is a `Stmt`, a node of its file's tape. `from_tape` is `rerail-rr`'s, below
      the lowering. Includes, the program cache, unreached-clause dropping, the closed-library and reserved-name
      checks, `check`, the tape type check, the token-read refusal, testing's blocks (their statements are nodes
      of the same tape) and core's declarations all read the nodes. The lowering's walk reads each directive off its
      node and writes it as it reaches it. Two pieces remain for A2. A clause is still read as the parser's form,
      by `Tape::clause`, for the lowering and the clause checks. The parser's `syntax::Item` is still built for the
      tape tests' parser comparison only. The coverage census still parses its files with the text parser.
    - *Done (A2, the lowering).* The clause lowering reads its clause off the tape (`lower/reads.rs`). A literal
      and a term are positions (`LitAt`, `At`), viewed as they are read (`L`, `T`); a disjunction's alternatives
      are positions, and each combination is lowered by walking them again. What the lowering moved by rewriting a
      clause's copy is a substitution: a hoisted aggregate, tape operation or digest, a wildcard named for a demand
      or a count, is a fresh variable in a per-literal substitution set (`Sub`) over the node it stands for. What it
      made is named: a value's binding (`LitAt::Eq`), a cover's atom (`LitAt::Made`). Fresh names come in the same
      order as before, so the lowered programs are unchanged (the print-back tests). The reads are
      `rr::clause::Reads`, a trait: the lowering supplies its substitutions, a file (`RrFile`) reads a clause as
      written. The clause checks (`check`'s grounding, strata and undeclared reads, the tape type check, the
      token-read refusal) read through it, and no reader takes `Tape::clause`'s parser form.
    - *Done (A2, from_tape's forms).* `from_tape` makes no parser form: `Tape::clause` and its term readers go, and
      so does the `Violation` holder. The lowering reads a `.violation`'s code, tag, fixes and suppressions off its
      node and writes each as it reaches it, and a `.lattice`'s `Lub` off its value's node. The tape tests' parity
      check compares each statement's kind, name, line and heads with the parser's (and each clause passes
      `check_clause`) instead of converting the tape to the parser's form.
    - *Done (A2).* The coverage census reads the shipped files' tapes (`RrFile`, `clause::Reads`), not the text
      parser's statements: 1733 files, none unread, 165 operations. Outside `rerail-rr-syntax`, the parser's
      `Item`/`Head`/`Lit`/`Term` are named only by the parity test, which compares with the parser.
    - *A3's shape (decided 2026-11-18).* A rule is written while no other rule is open (the encoder writes one
      rule's nodes at a time), and lowering a literal may write rules of its own (a context, a cover, an outlined
      alternative, a negated conjunction's helper, a demand). So a clause is lowered in two walks over the same
      positions, by one code path: each literal's lowering writes into a sink. The first walk's sink decides: no rule
      is open, so it writes the helper rules and demands, and it records the fresh names in the order they are made.
      The second walk's sink writes: for each combination of the disjunctions' alternatives it opens one rule
      (`Builder::rule`) and writes the head, then runs each literal's lowering again. That run makes the same calls in
      the same order, so its names are the recorded ones replayed; it writes only the chosen alternative's literals,
      term by term, and its helper and demand writes are no-ops. An outlined literal's helper is written in the first
      walk the same way: the literal's lowering runs once per alternative into a rule of the helper. Nothing between
      the walks holds a body, only names in order. What reads written rules (the demand dedup, `helper_once`'s
      template key, `.shape`'s check) reads them in the version (`RuleRef`), by the hash the encoder computes
      (`end_rule`); a duplicate is abandoned before it ends (`RuleBuilder`'s drop).
    - *Done (A3.1, A3.2).* The builder writes an open rule literal by literal (`Builder::open_rule`, `put_*`,
      `close_rule`, `close_rule_once`), each write its own borrow, byte for byte what `RuleBuilder` writes. The
      lowering writes every rule through it (no `Builder::add`): a demand once by the encoder's hash, and `.shape`'s
      check reads the finished version's rules. The holders of written trees go (`Out::Before`, `Written::kept`).
      Left: the lowering still builds each rule as a `front::Rule` and puts it (`put_rule`); each literal's lowering
      moves to its own `put_*` writes, then `put_rule` goes.
    - *A3.3's mechanics (decided 2026-11-18).* The sink is the lowerer's mode. Deciding: no rule is open; each
      value a run makes that a later run must see again is appended to a log in the order made: a fresh name, the
      name `helper_once` returns, a literal's count of alternatives. Writing: one rule is open; the same run reads
      those values back from the log at a cursor (set to the run's start, so a clause's combinations, and an
      outlined literal's alternatives, each replay the same segment); a literal writes only the alternative it is
      told to (`pick`), and its helper, demand and declaration writes do nothing. A conjunction's combination is a
      pick per literal, by the counts the log holds. Two rewrites of a whole body become writes: `unset` (a variable
      bound empty, written as the empty value in the head and body) by a scanning run per combination before the
      rule opens, which collects the chosen alternatives' `unset(v)` variables, then applied as each term is put;
      and the merge of consecutive negated tests of one subject (`merge_negated_tests`) by holding one pending
      negated test (its operation, its subject's position and its patterns) until the next literal is written.
  - **B. Helpers are written once, as image rules**, and readers name them by relation id. `helpers_of`'s decode
    (`Tree::rule`) goes.
  - **C. Passes rewrite image rules**: the rank plan, the product bound (`Decoded`), simplify, magic,
    `lowering_tail`, text and the compose stages. Smallest first, each one's tests ported before its tree path goes.
  - **D. The forms go**: `front::Rule`/`Literal`/`Atom`/`Term`, `Builder::add(&Rule)`, `encoder::tree_rule` and
    `program_image::Tree`, and §6's guard refuses them.

## 9. Retired, not ported (the user's questions, answered 2026-11-15)

**The bounded cross-file rounds** (`typing::cross_rounds`, `.pragma "cross-rounds"`).

- **Why they were built.** They came from the Typing branch's summary model, in which each file is typed alone from
  its imports' summaries. 0052 and 0056 (117ecd9cb, 2026-11-07) made them the program's declaration: k rounds,
  each reading the last round's rows wherever a chain crosses a file.
- **Why nothing uses them.** 0065 step 1 (30ee40e7b, the same day) replaced them with one semi-naive fixpoint. It
  was more accurate (6,299 against 6,198 agreeing with Jedi on 84 files) and cheaper: each round derived 112k to
  191k hop rows again to add a few thousand. Every typing program dropped the pragma. 0065 kept the mechanism "for
  a program that declares rounds. None of ours will". It left one case open: typing over files not loaded together,
  such as a package's summaries alone.
- **Why they are not needed.** That case is now a library's rows replayed as a derivation (0059), and phased facts
  over every document read (0083). No program, test or pack declares rounds. But each run still pays for them: the
  run computes the hops (`cross_rounds::hops`, 6338 and 6499) to check that no program does.
- **What goes:**
  - the bounded program (`bound`, `cone_split`, `pre_split`);
  - `run_rounds`, `rounds_loop`, `Rounds` and `Prepared::rounds`;
  - the per-run `hops` check.

  The pragma becomes an error that cites 0065.

  Done 2026-11-16 (0281 step 4): the module and its tests, the run's three branches, `run_rounds`, `rounds_loop`,
  the report's `rounds` and the per-run `hops`. `.pragma "cross-rounds"` is refused by both readers. The cross
  memo's `same_document` is now its own, over a version's rule, so nothing moved to `file_local`.
- **What stays, moved to `file_local`:** `same_document`, which the cross memo uses, and the `@doc($a) = @doc($b)`
  join (`join_spec`, `select`), which is the split's, not the rounds'.

**The recursive maintenance spike** (`plan/corpus/recursive_spike.rs`, 2,020 lines, `RERAIL_SPIKE_RECURSIVE`).

- **Why it was built.** 0090 (d8cbb6d8f, 2026-11-09) needed to know how to maintain recursive components exactly.
  The spike compared DRed, levels and backward/forward against recomputing each component, in an evaluator of its
  own.
- **What it answered.** Levels was cheapest at every step: 0.89 s against DRed's 2.04 s at step 08, with 8 bytes a
  row of kept state. The spike also listed the pitfalls any implementation must handle.
- **Why it is not needed.** The answer is built. Levels is in the engine (`setfix`'s `Levels`), used where the
  planner says `leveled`. Its results and pitfalls are recorded in 0090. Nothing calls the spike except its
  environment variable.
- **What goes:** the module, its call in the cross part's run, and its row in `facts_only`'s list. Done
  2026-11-16, with the rounds.

## Agreement with prior work

This agrees with:

- **0061, 0162:** a rule file has a tape, read in place;
- **0173 and 0241:** one image form, and passes as derivations;
- **0244:** split parts as views;
- **0246:** modules as images read in one tape walk, no statements and no `front::Program`, preparation as
  derivations of the linked base, the stored program as its image;
- **0266:** a writer, views and base + delta, with `Program` as a block.

It departs from nothing. Its first draft kept `front::Program` before `plan::lower`, which 0246 §1 had already
decided against. It removes t36 item 7's transitional form, a tree held or decoded beside each version, and the tree
forms before it.
