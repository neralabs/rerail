# Facts from a run: CI-written artifacts

Five data-layer checks need a fact that no reading of the source can give: how many tests pytest collects, which
models a registry holds after import, what a behaviour probe observes when it runs. The scripts get these facts by
running the repository's code.

Rerail never runs the code it checks. The repository's CI does, in a step of its own: it writes the facts to a
JSON file, and Rerail reads that file as an input. Rerail does three things with it: it checks that the file was
written for the code being checked, it makes the file's rows ordinary facts, and it says Unknown when the file is
missing or stale.

The five checks: test_budget, plan_composition_architecture, serverplan_target_architecture,
explicit_semantic_ids and linter_enforcement. Their runtime parts are in section 7. How Unknown spreads is
[FINDINGS-LAYER.md](FINDINGS-LAYER.md), section 2.

## 1. The CI steps

```yaml
# 1. Produce: the repository's code, in the repository's environment. One small script per artifact.
- run: |
    mkdir -p .rerail/raw
    for p in test_collection plan_composition_probes serverplan_shape_probes model_registry linter_runtime; do
      uv run python -m scripts.rerail_facts.$p > .rerail/raw/$p.json 2> .rerail/raw/$p.err \
        || echo $? > .rerail/raw/$p.status
    done
# 2. Stamp: Rerail records which inputs each file was produced from. Rerail runs no repository code here.
- run: rerail facts stamp .rerail/raw --out .rerail/facts
# 3. Check: the policies read the stamped files. Missing or stale ones fail the run in CI.
- run: rerail scan --pack data-layer --facts .rerail/facts --require-facts --fail-on-violations
```

The scripts run today in CI already, inside the linters: step 1 moves that work, it does not add any. A failed
producer does not stop the others; its exit status is recorded and the stamp marks the artifact failed.

## 2. Declaring an artifact in a policy

```
// What the artifact is built from. A change to any of these files makes it stale.
.artifact "test_collection" inputs "**/*.py", "pyproject.toml", "uv.lock", "conform/backends.sqlite.yaml"

// Reading it: the same readers as repository files, with artifact= instead of filename=.
.decl collected_total(n: number)
.input collected_total(IO=json, artifact="test_collection", path="/total_tests")
```

- `artifact="name"` is resolved in the facts directory (`--facts DIR`, default `.rerail/facts/`), never in the
  repository tree. A committed file would be stale as soon as the code changed.
- `path=` is a JSON pointer into what the producer wrote; the envelope (section 3) is invisible to it. Every
  `IO=json` option works: `records=true`, `*` steps.
- The inputs are declared by the policy, not by the producer, so a producer cannot narrow them. Two policies that
  share an artifact must declare the same inputs: put the `.artifact` line in a library both include. The checker
  rejects two different declarations of one name.
- An artifact's problems are `input_error("artifact:test_collection", kind, detail)` rows, where kind is
  `missing`, `stale`, `failed` or `malformed`. A policy can write rules over them, like any `input_error`. Rules
  that read the artifact itself follow FINDINGS-LAYER.md section 2: kept, withheld, and Unknown places.

## 3. The file

A producer prints any JSON value to stdout. It knows nothing about Rerail. `rerail facts stamp` wraps it:

```json
{
  "format": "rerail.facts/v1",
  "artifact": "test_collection",
  "inputs": "sha256:4f1c…",
  "status": "ok",
  "data": {"total_tests": 12546, "total_skipped": 0, "by_directory": {"api_types": 21, "…": 0}}
}
```

- `inputs` is the digest of the artifact's declared inputs as the tree was when it was stamped (section 4).
- `status` is `ok`, or `failed` with an `error` field: the producer's exit status and the first 4 KB of its stderr.
  A failed artifact has no `data`.
- The stamp takes a directory of raw files by name: `test_collection.json` is the producer's output for artifact
  `test_collection`, `test_collection.err` its stderr, and `test_collection.status` its exit status if it failed.
  The stamped file has the same name in the output directory.

## 4. Freshness: is this artifact for this code?

The digest is SHA-256 over the sorted lines `path NUL blob-id LF`, one per file that matches the inputs' globs.
The files are the ones Rerail's walk sees, and the blob id is git's blob hash of the file's content. For a clean
checkout that is the git index, so the digest is cheap. A modified or untracked file is hashed the same way, so the
digest also works in a dirty tree.

When it reads an artifact, Rerail computes the digest of the policy's declared inputs on the tree it is checking.
If the digest differs from the stamped one, the artifact is `stale`: its rows are not loaded. The check is
per artifact, not per commit:

- an artifact produced on one commit is still valid on another if none of its inputs changed: a documentation-only
  change keeps every artifact;
- a developer can download CI's artifacts and check locally, and they apply exactly when the local inputs match.

The globs should be wide. Pytest collection imports conftest files, plugins and the modules the tests import, so
`test_collection` lists `**/*.py` and the configuration files. A narrow glob that misses an input would let a stale
artifact through; a wide one only costs re-running a producer CI runs anyway.

## 5. Missing, stale and failed artifacts

| State | Rows | Outcome | In CI (`--require-facts`) | Locally |
|---|---|---|---|---|
| ok, fresh | loaded | as computed | as computed | as computed |
| missing | none | Unknown places (FINDINGS-LAYER.md §2) | the run fails | a coverage gap |
| stale | none | Unknown places | the run fails | a coverage gap |
| malformed | none | Unknown places | the run fails | a coverage gap |
| failed | none | Unknown places, unless the policy reports it | the run fails, unless the policy reports it | the same |

A failure the script treats as a finding, the policy writes as one. test_budget's collection failure is a
failure of the check (the script exits 1), so [test-budget.rr](test-budget.rr) reports it:

```
collection_failed(@place(BASE, 1), why) :- input_error("artifact:test_collection", "failed", why).
```

The static parts of every check still run without artifacts. A laptop without `.rerail/facts` gets every static
finding, and Unknown only where an artifact's rows could have landed: one file for the two probe checks, the
baseline file for test_budget.

## 6. Trust

An artifact is as trustworthy as the CI job that produced it. A change under review can alter a producer, just
as it can alter the Python linters that run the same code today. Nothing is lost by moving the run into a
producer.

The artifact cannot do more than a fact can. It holds data, never rules. Paths in its rows are checked against
`doc`, so an artifact cannot place a finding outside the repository. Where a number is protected, the trust root is
still the human-signed baseline: test_budget's ceiling is signed, and the artifact only provides the count it is
compared with.

A repository that wants more can protect its producers like its other CI scripts, with code owners on
`scripts/rerail_facts/`. Rerail does not need to know.

## 7. The five artifacts

Each check splits into a static part, which is ordinary Datalog over the tree, and a runtime part, which maps
artifact rows to findings. The producer is the repository's existing function, called and printed. The data
layer's code already separates the runtime part in every case but one.

### test_collection (test_budget)

- **Only a run knows:** pytest's collected count: parametrize expansions, fixtures, plugins, conftest hooks.
- **Producer:** `collect_test_counts` already returns the JSON. It raises `CollectionError` on a failed collection
  and on fewer than 1000 tests, which makes the artifact failed.
  ```python
  # scripts/rerail_facts/test_collection.py
  import json, sys
  from pathlib import Path
  from scripts.check_test_budget import collect_test_counts
  json.dump(collect_test_counts(Path(__file__).resolve().parents[2]), sys.stdout)
  ```
- **Static part:** a lower bound from the source, and the signed baseline. If the bound already exceeds the
  ceiling, `budget_exceeded` is reported without the artifact, as the script reports it without collecting.
- **Port:** [test-budget.rr](test-budget.rr).

### serverplan_shape_probes (serverplan_target_architecture)

- **Only a run knows:** what the nine behaviour probes observe when they build and run plans.
- **Producer:** `run_shape_probes()` returns `ShapeViolation`s.
  ```python
  from dataclasses import asdict
  from scripts.serverplan_target_shape_probes import run_shape_probes
  json.dump({"violations": [asdict(v) for v in run_shape_probes()]}, sys.stdout)
  ```
- **Policy side:**
  ```
  .artifact "serverplan_shape_probes" inputs "**/*.py"
  .decl probe_violation(code: symbol, probe: symbol, owner: symbol, observed: symbol, required: symbol,
      correction: symbol)
  .input probe_violation(IO=json, artifact="serverplan_shape_probes", path="/violations", records=true)
  .violation SPTB(at: loc, c: symbol, probe: symbol, msg: symbol) code {c} tag {probe} "{probe}: {msg}"
  SPTB(@place("scripts/serverplan_target_shape_probes.py", 1), c, p, m) :- probe_violation(c, p, o, ob, r, co),
      m = cat("owner=", o, "; observed=", ob, "; required=", r, "; correction=", co).
  ```
- **Static part:** everything else, including SPTA014 (a required probe family not registered). That reads the
  `probe_id` fields of the classes listed in `PROBES`.
- **Unknown without it:** one file, `scripts/serverplan_target_shape_probes.py`.

### plan_composition_probes (plan_composition_architecture)

- **Only a run knows:** whether equal plans fingerprint equally (SPC-012), and whether two imports of one fragment
  reuse destination refs (the runtime SPC-005).
- **Producer:** `run_behavior_probes()` returns `CompositionViolation(rule, path, line, message)`.
  ```python
  from scripts.lint_plan_composition_architecture import run_behavior_probes
  json.dump({"violations": [{"code": v.rule, "path": str(v.path), "line": v.line, "message": v.message}
                            for v in run_behavior_probes()]}, sys.stdout)
  ```
- **Policy side:** `code {c}` at `@place(p, l)`, with p checked against `doc`. The static SPC-001…005 are rules
  over `l2070_batching/semantic_plan.py`.
- **Unknown without it:** one file, `l2070_batching/semantic_plan.py`.

### model_registry (explicit_semantic_ids)

- **Only a run knows:** which models are registered after import, and their pydantic fields. Those include fields
  a metaclass adds at class creation and relationships inherited through `dir()`. A static reading also sees
  classes the registry never holds, such as benchmarks and conformance fixtures.
- **Producer:** the registry's own accessors, dumped as observations, not verdicts. The logic that decides a
  finding (effective ids, duplicates, missing ids) stays in the policy, where it is reviewed with the other rules:
  ```python
  # one record per registered model whose class name does not start with "_"
  {"model": "l1000_schema.encrypted_blob.EncryptedBlob", "file": "l1000_schema/encrypted_blob.py",
   "model_id": "…", "declared_id": "…" | null, "participates": true, "participation_violations": ["…"],
   "members": [{"kind": "field" | "relationship", "name": "…", "semantic_id": "…", "declared_id": "…" | null,
                "api_name": "…", "exempt": false}]}
  ```
  The producer calls `get_model_semantic_id`, `get_field_semantic_id`, `get_declared_field_semantic_id` and
  `get_field_api_name` (`l1000_schema/table_registry.py`, `schema_def.py`). These functions are the
  repository's definition of the ids, and computing them needs the classes.
- **Policy side:** `missing_explicit_semantic_id` at `@place(file, 1)` when a model has participation violations,
  a member without a declared id that is not exempt, or two members with one effective id or one API name
  (`count : { … } > 1`).
- **Unknown without it:** the whole policy. Its places come only from the artifact.

### linter_runtime (linter_enforcement)

- **Only a run knows:** for LE003, whether a linter "has an active allowlist". The script decides this by calling
  the module's `_get_allowlists_for_signing()` or `_load_allowlist()` when the module is loaded. Whether it is
  loaded depends on what `scripts/lint_all.py` imported.
- **Producer:** import `scripts.lint_all` as the suite does, then for each config of `DL_SUITE` record the module's
  state:
  ```python
  {"linters": [{"name": "file_length", "loaded": true, "active_allowlist": true}, …]}
  ```
- **Policy side:** LE003 reads `active_allowlist` for a loaded module. For a module that is not loaded it uses the
  script's own fallback, "has any `*SIGNATURE` variable", which is static.
- **Static part:** LE000–LE002 and LE004–LE011, all from the suite's literals. That includes `plugin_name`
  defaulting to the name, which the suite sets after the literal.
- **Unknown without it:** the scripts that have a placeholder signature. Removing the artifact atom from LE003's
  rule leaves exactly those places.

## 8. What Rerail needs, and what the repository needs

**Rerail:**

- the `.artifact` directive, the `artifact=` reader option, and the `missing`, `stale`, `failed` and `malformed`
  input errors;
- `rerail facts stamp DIR --out DIR`: wrap, digest, record failures. It reads the policies to find each artifact's
  inputs;
- `rerail scan --facts DIR`, `--require-facts`;
- `rerail facts plan --facts DIR`, optional: list the artifacts whose inputs changed since the given stamped
  set, so CI can skip producers whose artifact is still fresh;
- the storage side follows the target architecture: artifact rows are facts keyed by (artifact, digest). A
  change to an input invalidates them, as any changed fact is invalidated, and they stay invalid until a new
  artifact arrives. They do not trigger recomputation, because Rerail cannot produce them.

**The repository:** five producer scripts, each a few lines over an existing function. Only linter_runtime is new
logic: a loop over `DL_SUITE` that records the module state the linter computes inline today. Then the three CI
steps.

## 9. Prior work

- **Rerail never runs repository code** (target architecture). Agrees. The only code that runs is in the
  repository's own CI step, and Rerail reads its output.
- **Target architecture point 2 (git tree hashes and content hashes are facts) and point 5 (stale facts must be
  detectable).** The input digest is built from blob hashes Rerail already has, and a stale artifact is detected,
  not trusted.
- **Lessons ledger #14: whole-graph artifacts must not force whole-repository rebuilds.** Agrees. An artifact
  invalidates only its own rows and the rules that read them; with wide input globs it goes stale often, and that
  costs one producer run in CI, not a rebuild of other facts.
- **Findings contract (Unknown, completeness).** A missing artifact is an Unknown at the places its rows could
  reach, never a silent pass.
- **The group analyses (checks/group-1, -4, -6, -8)** proposed a native reader per check (`probe_result`,
  `test_collection`). Departs: one generic reader with an envelope serves all five, and a future check needs only
  a producer and an `.artifact` line.
