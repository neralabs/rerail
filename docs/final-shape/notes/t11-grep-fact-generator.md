# t11. A grep-like fact generator, and what plan-composition-architecture needs (2026-11-07)

## 1. Intro

The data-layer policy `plan-composition-architecture` checks one file, `l2070_batching/semantic_plan.py`. Five
of its checks read the file's syntax tree, and they already run. Two more do not read the source at all. They
come from the Python linter running the plan builder and watching what it does:

- **SPC-012**: build the same small plan twice and compare the two fingerprints. They must be equal.
- **SPC-005 (probe)**: import one plan fragment into a builder twice, and check that the two imports were given
  different destination refs.

The policy reads what the probes saw from a file a CI step writes, the `plan_composition_probes` artifact. The
engine does not yet read run artifacts, so the policy stops before it runs.

The question was whether a grep-like fact generator, rows made by matching text in files, could stand in for that
artifact. The short answer is no, not for these two checks. A fingerprint is a hash of objects the builder makes
while it runs, so no reading of the text gives it. What text and trees can give is a *static approximation*: rules
that flag the code shapes that would make the probes fail. Those need only the syntax tree, which the engine
already has.

The generator is still worth having for other policies. They want facts from files that are not documents of the
run: lock files, CI YAML, `.md` tables, files outside `.scope`. This note specifies it (§5–§8).

## 2. What the probes observe

The linter's `run_behavior_probes()` (in `scripts/lint_plan_composition_architecture.py`) does this:

```python
# SPC-012: two builds of one plan must fingerprint equally.
def build_fingerprint() -> str:
    builder = SemanticPlanBuilder(namespace=PlanNamespaceId("composition-linter"),
                                  generation=PlanGenerationId("probe"))
    ref_id = builder.allocate_ref(kind="step")
    builder.add_step(StaticValueAfterDepsStep(ref_id, 1, []))
    builder.add_return("value", ref_id)
    return builder.freeze().fingerprint
if build_fingerprint() != build_fingerprint(): ...        # SPC-012

# SPC-005: one fragment imported twice must get fresh destination refs.
first = destination.add_fragment(fragment)
second = destination.add_fragment(fragment)
if set(first.ref_map.values()) & set(second.ref_map.values()): ...   # SPC-005
```

`freeze()` computes the fingerprint in `_semantic_fingerprint`:

```python
payload = (str(namespace), str(generation), legacy_ref_ids,
           tuple((str(seed.ref_id), seed.value or ("runtime-input", …), seed.sql_hints) for seed in seeds),
           tuple((str(step.ref_id), step.planning_fingerprint()) for step in steps),
           tuple((str(b.name), str(b.ref_id)) for b in returns))
encoded = json.dumps(payload, default=str, separators=(",", ":"), sort_keys=True)
return hashlib.sha256(encoded.encode("utf-8")).hexdigest()
```

`add_fragment` gives each non-reused source ref a destination ref from `allocate_ref`, which counts up
`self._next_ref_ordinal` and skips any ref already in use.

So both probes test **behaviour**:

- SPC-012 tests that the whole path is deterministic: `freeze`, `_semantic_fingerprint`, each step's
  `planning_fingerprint()` (here `StaticValueAfterDepsStep`'s, from
  `impl/common/l2070_server_plan/steps/collect_results.py` or a base class), and `str()` of every value
  `default=str` reaches.
- SPC-005 tests that the allocator never hands out a ref twice.

Neither result is a fact about the text. The engine runs no repository code (no CPython), so it cannot compute
them. A grep can only find text.

## 3. A static approximation, with the relations we have

Each probe fails only through a few code shapes, and each shape is visible in the tree of `semantic_plan.py`
(and of the step classes, through the cross-file part). The rules below are sketches over `python/ast.rr`,
`python/names.rr` and `python/calls.rr` relations. They are not part of the policy, which is the data-layer
authors' to change.

```
// SPC-012 (static): what makes a fingerprint differ between two equal builds.
// spc_fp_def(f): a function on the fingerprint path: _semantic_fingerprint, freeze, planning_fingerprint.
.decl spc_fp_def(f: node)
spc_fp_def(f) :- kind(f, "function_definition"), def_name(f, x),
    (x = "_semantic_fingerprint" ; x = "freeze" ; x = "planning_fingerprint").
// A call whose value differs from run to run: id(), time, random, uuid, hash() of a str (PYTHONHASHSEED).
.decl spc_unstable(c: node, x: symbol)
spc_unstable(c, x) :- spc_fp_def(f), within(c, f), callee_name(c, x),
    (x = "id" ; x = "hash" ; prefix("time.", x) ; prefix("random.", x) ; prefix("uuid.", x)).
// json.dumps on the path without sort_keys=True: dict order is insertion order, which a set may feed.
.decl spc_unsorted_dump(c: node)
spc_unsorted_dump(c) :- spc_fp_def(f), within(c, f), callee_name(c, "json.dumps"),
    !call_kw_true(c, "sort_keys").
// Iterating a set into the payload (`tuple(x for x in some_set)`) is ordered by hash.
// (A sketch: needs the inferred type `set` of the iterated value; typing gives it as a fact.)
```

```
// SPC-005 (static): the allocator must count up and skip refs in use, and add_fragment must allocate.
.decl spc_counts(f: node)
spc_counts(f) :- def_name(f, "allocate_ref"), within(a, f), kind(a, "augmented_assignment"),
    field(a, "left", l), text(l, "self._next_ref_ordinal").
.violation SPC_005_static(at: node) code "SPC-005" "ref allocator does not count up"
SPC_005_static(f) :- spc_method(_, f, _), def_name(f, "allocate_ref"), !spc_counts(f).
```

What this gives up:

- A probe sees every path the run takes. A rule sees only the shapes it names. A new source of nondeterminism,
  such as a `__str__` that prints an address through `default=str`, passes the rule and fails the probe.
- Exactness needs the steps' `planning_fingerprint` overrides, which are spread over many files. They are in the
  corpus (cross-file part), but `StaticValueAfterDepsStep`'s may be inherited, so the rule needs the class
  hierarchy: `calls.rr`'s method resolution, or the typing facts.

Whether to replace the probes with these rules, keep the artifact, or keep both is the policy authors' call
(§9). None of it needs the grep generator.

## 4. Does `source` and `match_all` over `.scope text` already give grep?

For files that are documents of the run, mostly yes:

```
// Every `TODO(name)` in Markdown files: path, line, name.
.scope text "**/*.md"
.decl todo(p: symbol, l: number, who: symbol)
todo(p, l, who) :- doc(d, p, _), source(d, sp), match_all("TODO\\((?<who>[a-z]+)\\)", sp, _, m),
    line(m, l), text(who_sp, who).   // captures over a span are spans (rx_span): line and text read them
```

What is missing:

1. **Files outside the run's documents.** `.scope text` makes files *documents* of the policy: they get rows in
   `doc`, `path_part` and `source`, take a chunk slot, and are listed in a scope. A policy over Python that wants
   one fact from `uv.lock` or `.github/workflows/*.yml` must widen its scope, so every rule over `doc` sees those
   files too. The readers in `corpus/inputs.rs` (`IO=lines` with `filename=` or `dir=`) read files outside the
   scope, but only one named file or one directory, and only whole lines. There is no glob, no regex and no
   captures.
2. **Per-relation selection.** One policy may grep `**/*.py` for a marker and `conform/*.yaml` for a key.
   `.scope text` is one set of files for the whole policy.
3. **A plain row shape.** `match_all` over a span gives span captures, which need `line`/`text` to read. A fact
   generator should give `(path, line, groups…)` directly, typed by the declaration.
4. **Caching by content.** Input readers run once per policy run, before partitioning, and their rows are
   `program.facts`: kept in the program, re-read on every run, and not memoised per file. On a 9,010-file repo,
   grepping `**/*.py` that way reads 9,010 files into one fact table on every run.

## 5. The generator: surface syntax

A new reader for `.input`, beside `file`, `lines`, `json`, `yaml` and `jsonl`:

```
// One row per match of `re` in each file the glob selects. Columns: the file's path, the 1-based line where
// the match starts, then the named groups in the order the declaration lists them (by name, not position).
.decl todo(path: symbol, line: number, who: symbol)
.input todo(IO=grep, glob="**/*.md", re="TODO\\((?<who>[a-z]+)\\)")

// Several globs, and an `ordinal` column: the match's 0-based number within its file.
.decl dep(path: symbol, line: number, ordinal: number, name: symbol, version: symbol)
.input dep(IO=grep, glob="uv.lock", glob="**/requirements*.txt",
           re="(?m)^name = \"(?<name>[^\"]+)\"\\nversion = \"(?<version>[^\"]+)\"")
```

Rules:

- `glob=` takes path globs (core.rr's dialect, repository-relative). Walks skip `.git`, `.venv`, `venv`,
  `__pycache__` and `tmp`, as `.scope` walks do. A file need not be a document of the run.
- `re=` is in the engine's regex dialect: Rust `regex`, no look-around or back-references (refused as a bad
  regex at load). `(?m)` and `(?s)` work as usual. Without a capture, a row is `(path, line)`.
- Columns are matched by name: `path`, `line`, `ordinal`, `start`, `end` (byte offsets) are the generator's.
  Every other column names a capture group, and a missing group is a load error. Column types follow the
  declaration (`number` groups are parsed as integers, and a bad one is a malformed-input error, as in the
  JSON readers).
- Matches don't overlap, as with `match_all`. A group that doesn't take part gives no row; use `(?:…)?` with a
  default in the rule.
- Files that are not UTF-8 are skipped, with an `input_error(path, "unreadable", …)` row, the readers' error
  relation.
- `filename=` and `dir=` stay as they are; `glob=` is the grep reader's only selector.

## 6. Semantics and where it plugs in

The grep relation is a *repository relation*, not a policy input. The engine has both kinds today:

- `corpus/inputs.rs` readers fill `program.facts` once, before partitioning. That suits one small file (a JSON
  baseline).
- `facts::repository::relations` (used by `corpus::run_with_options`) gives relations over the run's files as
  vtab `Source`s, bound beside every chunk's and measured once (`syntax::statistics`).

The grep reader belongs with the second:

1. **Load.** `rr::lower` records each `IO=grep` input as a `GrepInput { rel, globs, re, columns }` on the
   program (new `Program::greps`) and keeps the relation an input, not a fact. `inputs.rs` rejects `IO=grep` with
   `filename=`/`dir=`.
2. **Selection.** At run start, the corpus walks the root once for all grep inputs together: one walk, each
   path tested against every input's globs with the glob matcher the scope already uses.
3. **Matching, chunked.** Each selected file is read once. All grep inputs that select it are compiled into one
   multi-pattern matcher: a `regex::bytes::RegexSet` for which patterns match at all, then each matching pattern's
   own `Regex` for captures, as `logic::native::match_all` does. A file is scanned once per run for every input
   together, never once per rule or per row. This follows the chunked string-matching rule: one pass over the
   bytes, DFA steps, no per-row text scans.
4. **Rows.** Each input's rows form a `columns_source` (typed integer columns, symbols interned in one
   `intern_batch` call per file). The relation is bound as a repository relation with its statistics, so the
   planner prices joins on it like any other.
5. **Cross-file only.** Grep rows have no document of the chunk: their `path` is a symbol, not a `doc` key. The
   rules that read them run in the cross-file part, as `option_item` and `git_tracked` rules do. A rule that
   needs the document joins `doc(d, path, _)`.

## 7. Caching

Per file, the rows depend only on the file's bytes and the input's (`re`, columns). The tape cache already keys
stored work by content:

- key: `(content digest of the file, digest of re + column list)`;
- value: the file's rows, as one zstd frame (the per-file memo's encoding, decision 0052).

A warm run walks, stats each file (size and mtime, then the content digest if they changed), and reads the kept
rows. Only changed files are scanned. A rule change that leaves the regex alone keeps every row. The run record
lists the grep keys it read, so collection keeps them (decision 0053).

## 8. Cost model

- **Walk:** one per run for all grep inputs, shared with the scope walk when the globs fall inside it.
- **Cold scan:** bytes of the selected files × one DFA step per byte for the `RegexSet`. Then, in files the set
  hits, one capture pass per matching pattern. On the 9,010-file repo (about 120 MB of Python), a `**/*.py` grep
  costs about what `source` + `match_all` cost now for the same pattern: tens of milliseconds of DFA work, plus
  reading the files, which the tape cache usually has.
- **Warm:** a stat per file and the kept rows, with no reading or matching.
- **Memory:** the rows only (8 bytes a column). Each file's bytes are dropped after its scan, and nothing is
  held per file.
- **Regex vs plain text:** a plain literal (`re="TODO("`) goes through the same set. The engine's literal
  prefilter makes it as cheap as a substring search.

## 9. Implementation plan

1. **Parse and lower** (`rr/lower.rs`, `inputs.rs::parameters`): accept `IO=grep`, repeated `glob=` and `re=`.
   Validate the regex in the engine's dialect, and check that each column names a generator column or a group.
   Tests: good and bad declarations, unknown group, look-around refused.
2. **Matcher** (`logic/native/match_all.rs` neighbour, `grep.rs`): `scan(bytes, inputs) -> rows per input`,
   one `RegexSet` pass plus capture passes, with line numbers from a newline index built once per file. Tests:
   overlapping patterns, `(?m)` anchors, CRLF line counts, a group absent from a match, non-UTF-8 skipped.
3. **Repository relation** (`facts::repository`): walk, select, scan and bind as a `Source` with statistics.
   Test: a two-file fixture where a grep relation joins `doc` and a finding lands at `@place(path, line)`.
4. **Memo** (`content_store`): rows per (content, pattern) digest; collection by run record. Test: a second run
   reads no file; editing one file rescans only that file.
5. **Profile:** a `grep` span (files, bytes, rows, kept/made), and a decision line per input.
6. **Docs:** core.rr's input readers section, with the samples of §5.

Steps 1–3 make the feature. Step 4 makes it cheap on re-runs.

## 10. Open questions

1. **plan-composition-architecture itself:** replace the two probes with the static rules of §3 (approximate:
   a new nondeterminism source can slip past), keep the CI artifact (exact, but it needs the stamped-artifact
   reader, `.artifact`), or both: the static rules everywhere, and the probes where CI stamps them. This is the
   data-layer authors' choice.
2. Should grep see git-ignored files? `.scope` walks today skip only a fixed list. A generator over `**/*` would
   read build outputs.
3. Size limits: skip files over some size (lock files can be megabytes), or scan everything?
4. Should a grep relation be allowed in the file-local part when its glob is inside the run's scope, so its rows
   are memoised per document with the rest? This would mean giving `path` a document key.
5. Multi-line matches give the line where the match starts. Should an `end_line` column be offered too?
6. Binary and non-UTF-8 files: skip with an `input_error` row (proposed), or match bytes and give lossy text?
