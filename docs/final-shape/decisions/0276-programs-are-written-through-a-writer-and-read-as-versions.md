# 0276: Programs are written through a writer and read as versions; `lower::Tree` goes

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
decode wherever a reader was not ported. This decision gives the program one form after lowering, a version, made
only through a writer and read only through views. `lower::Tree`, `Tree::{given, of, beside, maintained}`,
`TREE_READS`, `same_as_tree`, `made_from`'s `had` and every tree fallback are deleted.

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

### 1. One form after lowering

The boundary is `lower`. Before it, the front end works on `front::Program`, as 0246 leaves it: parsing,
`.include` resolution, demand, matchers, the text tests. `lower` takes a `front::Program` and makes a version. After
it, nothing holds a lowered program's tree:

- what a pass reads is a view, `Placed` and its `Fields`;
- what a pass makes, it writes through `ProgramW`, which freezes to a `Placed`;
- a measurement aid that prints a program decodes it explicitly (§6).

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

/// The prepared program (`Prepared`): the front program for the store's key and text, and the rest as versions.
pub struct Prepared {
    pub prog: front::Program,                 // before lowering: what `prepared_key` and 0246's scope read
    pub lowered: Part,                        // the whole lowered program
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

**Lowering and preparation** (`corpus.rs` 2700–2810, `file_local.rs`, `simplify.rs`, `lower.rs`):

| Site | Port |
|---|---|
| `lower::lower` | writes `ProgramW::lowering()`: each lowered rule by `RuleBuilder`, declarations and fields by `fields()`; returns `Part`. Its tree goes. `domains` is computed on the front tree it was given, which is still the input |
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
(2877, 2881, 3076) goes, since the version is always there. `dump_split` decodes them as an aid. The cross memo's
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
| `recursive_spike::run(&cross.program)` | an aid behind `recursive_spike::on()`: decodes (§6), or is retired (question 2) |
| `cross_rounds::bound`, `cone_split`, `pre_split`, `run_rounds`, `round.program` (7396–7513, 8195–8310) | question 1 |
| `maintain::rewrite`'s fallback part, gates test, `input_views` | its version path; `f.gates().is_empty()`; `input_views` on handles (also ends `retain.rs` 315's decode) |
| `emit::emit_inputs` (140 lines), `emit::outputs`, `plan::decode_kept` | the SQL emitter reads `Placed` through `Body`, as `plan::run`'s kernel does; `decoded_outputs` reads `f.is_input`, `f.width` |
| `Model::analysis_of` (`plan.rs` 1726) | out of scope: the planner's analysis is its own port. It keeps decoding, as one named reader (§6), until then |
| `sql_executor/compiler.rs`, `coverage.rs` | `lower` returns a `Part` |

**Tests:** `lower::Lowered { program: Tree::given(p), .. }` becomes `Part::of(&p, dialect)`, a test helper that lowers
with its own interner. The 16 test sites of `lower::lower` take the `Part`, and assertions that read rules decode it.

### 6. Decoding: named, and only for aids

`Placed::program(&lists)` stays: it is how a version is printed. It is called only from:

- the measurement aids, each behind its environment variable: `version_trace`, `RERAIL_SPLIT_DUMP`,
  `RERAIL_DEMAND_DUMP`, `RERAIL_DUMP_MATCHERS`, `O125_RULES`, `RERAIL_MEMO_PROGRAM_DUMP`;
- `Model::analysis_of`, until its port;
- tests.

A guard test greps the engine's sources for `.program(&` on a `Placed` or `Version` and allows only the callers
listed. The list may only shrink. The run's counter `version_trees_read` and `run_state`'s entry for it go with
`lower::Tree`.

### 7. Base and delta, in 0266's terms

| Today (`program_image`) | 0266 `Program` block | In this port |
|---|---|---|
| base | full block | `ProgramW::lowering`, `derive` |
| view (removals, few ranges) | a delta with removals | `edit` + `keep` |
| overlay (additions, fields) over any version | a delta over a full block, never a delta | `edit`; the writer folds an overlay of an overlay into one cumulative delta over the full block, copying the old delta's additions whole (0266 §6's carry) |
| fold at half the parent | roll-up by the cost rule | unchanged in this port; the cost rule is §8 |

Callers see none of this. They open, write, freeze and read `Placed`. So 0266 step 3 can put a version into a block
without changing a caller.

### 8. Afterwards, not in this port

- **Domains in the declarations.** A declaration's columns gain their domain, and `Lowering` keeps only `ops` and
  `dialect`. This is 0266's `Program` root, where declarations are nested whole.
- **Roll-up by the cost rule** in place of the fold at half, measured against today's fold on dl1's session runs.
- **The planner's analysis on versions** (`analysis_of`, `plan::program_digest`).

## Path

Each step leaves the engine working, runs its targeted tests, and is measured on dl1 (findings digest and retired
instructions) against the step before.

0. **Merge the hub's 0246 stages 2, 3 and 7.** `lower` in a library's scope and the stored image are what steps 1 and
   3 build on.
1. **`ProgramW` and `Part`.** The writer over `Builder`, with `copy`/`copy_all` as range copies, `rewrite_all` and
   `fields()`. `Part` and the shared `Lowering`. Port the existing version passes to them: supports, gate, split,
   the round program, the cut, `qualify`, the input deltas, the constants, the cross memo, the maintenance rewrite.
   Delete each pass's tree fallback and `made_from`. List what the image cannot hold, and extend it. Check: the
   session, cone, cross memo and maintenance tests.
2. **The run holds versions only.** `Split`'s parts are `Placed`. `Cone` drops its trees. Port every run reader in
   §5: the diagnostics, the emitter, `decode_kept`, `input_views`. Answer questions 1 and 2. Check: corpus tests, the
   SQL extension's tests, the guard asserting that a non-maintained run reads no tree too.
3. **Preparation on the writer.** `lower` writes a version. The readers come first (split, constants, causes,
   statistics), then the rewriting passes, one commit each: `locate`, the merges, `demand_exports`,
   `factorize_exports`, `push_local_joins`. `remember` and `prepared_of` store and read the image. Check: the
   `file_local` tests (`rerail-device-metal`), the prepared tests, the phase tests.
4. **Delete `lower::Tree`.** `Lowered`, `TREE_READS`, `same_as_tree` and `version_trees_read` go. Add §6's guard.

## Questions for the user

1. **The cross rounds' bounded program** (`cross_rounds`, `.pragma "cross-rounds"`): no `.rr` program declares it
   today, so the path is dormant, and its passes (`bound`, `cone_split`, `pre_split`) are tree rewrites. Port them
   to `ProgramW` in step 2, or retire the path?
2. **`recursive_spike::run`,** behind `recursive_spike::on()`, reads the cross part's tree. Keep it as an aid that
   decodes, or retire the spike?

## Agreement with prior work

This agrees with:

- **0173 and 0241:** one image form, and passes as derivations;
- **0244:** split parts as views;
- **0246:** modules as images, and the stored program as its image;
- **0266:** a writer, views and base + delta, with `Program` as a block.

It departs from nothing. It removes t36 item 7's transitional form: a tree held or decoded beside each version.
