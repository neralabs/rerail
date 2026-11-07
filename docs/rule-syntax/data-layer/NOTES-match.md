# Group `match`: notes

Five checks, written by hand from the Python linters at `2b1504adc`.

| Check | Python lines | `.rr` lines | rrlab dl (exact / events) | Engine reader |
|---|---|---|---|---|
| hnsw_architecture | 263 | 132 | 60 / 61 | lowers |
| generated_index_sources | 275 | 90 | 24 / 24 (a) | lowers |
| conditional_architecture | 426 (+750 facts module) | 301 | 82 / 82 (a, b), or 42 / 82 counting dropped findings | lowers |
| embedding_vector_orchestration | 596 | 129 | 70 / 70 (a) | lowers |
| matching_architecture | 1,268 | 446 | 286 / 286 (a); the 287th event was not run (c) | lowers |

There is also one library: `lib/data-layer/match-frames.rr` (41 lines). conditional_architecture and
matching_architecture use it.

(a) The stock `rrlab dl` stops with an error on these four checks, before it runs any rule. Their batch-10 records
write `expected` in shapes that `dl.py` does not read (see "Harness change request" below). I scored them with a
wrapper that only changes how those records are read:
`/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-More-ergonomic-rule-syntax/1ecc536f-2d93-443b-ac6f-d3945bb921ac/scratchpad/dl/match/dlx.py`.
It loads `dl.py` and changes only `_load` and `events`.

(b) In 47 events, conditional_architecture is called through `conditional_architecture_findings`. That function
returns every finding of the facts module, and the findings have no code. The plugin keeps only the findings whose
message holds one of three markers, and reports them as CONDA-005:
- callback fields, state writes, backend controls, child compiles and lowering levels are computed;
- they are never reported, so the rule does not write them (group-7.md says the same).

The wrapper maps a message with a marker to CONDA-005, and any other message to `CONDA-005-dropped`. Scored on the
reported codes only (`DROP=1`), every event is exact. Counting the dropped findings, 40 events miss. Each of those
misses is a finding that the linter computes but does not report, so it comes only from emulating the entry
function: a harness limit.

(c) The event `test_current_matching_architecture_is_clean_tdd_red` holds the real repository: 2,871 files and no
findings. rrlab writes tape relations out as fact files, 3.5 GB for this event. Souffle had used 3.7 GB of RAM after
4 minutes, so I stopped it. This is beyond the 1 GB and 2-minute budgets, and it is a harness limit, not a rule
limit. `SKIP_BIG=1` in the wrapper leaves it out.

## Misses

- **hnsw_architecture, `test_hnsw_text_and_parser_forms[json = (\n]`.** The file does not parse, so the linter
  reports nothing. core.rr says the engine holds back a document with a syntax error, so it gives no findings.
  rrlab does not hold such documents back: it runs the rule on tree-sitter's recovered tree, and finds HNSW007 at
  line 1. This is a harness gap: rrlab should drop the facts of documents that have a `syntax_error` row.

## Deliberate readings (no miss, recorded for review)

- **Trigger gates.** Most of these linters run their collector only when the file's text holds a trigger
  substring. The rules test the whole text with `source(d, sp), text(sp, t), contains(…)`.
  - hnsw_architecture's recorded events all call `find_hnsw_architecture_findings`, which skips the gate. The rule
    follows that call, which is the only one recorded. The plugin path, which applies the gate, is noted in the
    rule's header.
- **matching_architecture's gate and the entry functions.** The rule follows `lint`, the CLI, which is the check
  CI runs. `lint` scans a file only when it is a matching path, holds one of the seven markers, or is in a layer and
  defines `match(`; it scans application roots whole.
  - `scan_sources` scans every file it is given.
  - The only files where the two differ are top-level `applications/…` files. Under `lint` such a file can only
    come from an application root, which is scanned whole. So the rule scans application files without the gate,
    and that makes the two agree on every recorded event.
  - MA033 and MA034 are `lint`'s own checks. `scan_sources` events are not compared on them.
- **matching_architecture `--application-root`.** COVERAGE.md lists this as a gap. Here it is
  `.scope dirs option("applicationRoots")`, plus `ma_app` for the files under those roots. The recording passes
  the files themselves as `applicationPaths`, which `ma_app` also reads. `--phase` is not modelled; the suite runs
  without it.
- **embedding_vector_orchestration EVO107 and EVO108.** The collector computes them and `scan_source` returns
  them, but the plugin drops them, because they are not in FAILING_CODES. The rule sets them to `.mode log`. rrlab
  keeps log codes on direct calls, so the `scan_source` events still compare them, and all match.
- **Names that the scripts build by hand are joined, not built recursively.** Three relations need a name built
  from parts:
  - conditional's `_dotted_name` and the framework's `call_name`;
  - matching's `_annotation_name` for `A | B`;
  - matching's `_static_string` for `"a" + "b"`.

  A recursive `@dot`/`cat` form does not lower: the engine refuses it with `UnboundedValueCycle`, which it also
  reports for python/ast.rr's own `dotted_expr`. So each name is a `join` of the chain's pieces, ordered by offset,
  over a recursion that collects nodes only. These forms lower.

  matching's `_annotation_name` uses core `name` for attribute chains. On an attribute over a subscript it differs
  from the script: `x[0].y` gives "y", where the script gives "x.y". No event has that case.
- **conditional_architecture's class model.** The rule follows the facts module's dictionaries:
  - one row per class name per file, the last one counting;
  - a class's methods are the defs whose innermost class it is, nested helpers included, the last def of each name
    counting;
  - base classes resolve only their first segment, through the imports seen before the class.

  In `class A: def m(): ... def m(): ...`, a call in the first `m` still counts for `ca_belongs`. The script would
  have replaced that method's entry. No event has that case.

## Library and core changes

None needed. The new vocabulary is in `lib/data-layer/match-frames.rr`:
- `mf_class(n, k)`: the innermost class around n, through defs, as the scripts' class stack has it;
- `mf_def(n, f)`: the innermost def around n, through classes, decorators included;
- `mf_walk(n, o)`: n is in `ast.walk(o)`, decorators included.

python/ast's `def_frame` covers defs only, and `nearest(…, "class_definition")` treats decorators as outside the
class or def they decorate. The checks in other groups whose scripts keep `_class_stack` / `_function_stack` may
want these relations.

## Harness change request (`tools/rrlab/dl.py`, `events`)

Batch-10 records write `expected` in three shapes:
- `"path:line: message"` strings (generated_index_sources);
- dicts keyed `file` / `file_path` / `rule` (conditional, embedding);
- dicts with no code (`conditional_architecture_findings`).

`events` reads only `{"path", "line", "code"}`. Here is a replacement for its `want` expression in the
complete-api loop. `files` is the list of paths given in the record.

```python
def _want(r, files, check):
    out = set()
    for f in lit(r["expected"]):
        if isinstance(f, str):
            m = re.match(r"(.*?):(\d+): (?:([A-Z][A-Z0-9_-]*\d+): )?", f)
            p, line, code = m.group(1), int(m.group(2)), m.group(3) or DEFAULT_CODE.get(check, check)
        else:
            p = f.get("path") or f.get("file") or f.get("file_path")
            line, code = int(f.get("line") or 1), f.get("code") or f.get("rule") or UNCODED.get(check, check)
        p = next((g for g in files if p == g or p.endswith("/" + g)), p)   # temp-dir prefixes
        out.add((p, line, code))
    return out
DEFAULT_CODE = {"generated_index_sources": "missing_source"}
UNCODED = {"conditional_architecture": "CONDA-005"}   # and drop messages without the plugin's three markers
```

Two more fixes would let rrlab score the remaining events:
- skip events with more than a few hundred files, or record them as not run (note (c));
- hold back documents that have a `syntax_error` row, as core.rr specifies.

## Nothing left unexpressed

Every code of the five linters is written. The only parts left out are on purpose:
- the facts module findings that the plugin drops;
- the hnsw plugin-path gate;
- matching's `--phase` filter.
