# What the data-layer checks need, and how the language provides it

## Intro

The data-layer repository enforces its architecture with 142 lint checks, written as Python scripts. They are the
hardest test the `.rr` rule language has: if it can say what these scripts say, with the same findings, it can
replace them. Seven checks were already written by hand. This page asks the question for all 142: what does the
language, its native core or its libraries lack? It then records what was added.

The answer is mostly facts, not syntax. The scripts read things Python gives them for free: CPython's `ast` (where
a decorator belongs, which names are `Name` nodes, the order a visitor walks), its string decoder, the raw lines
of a file, the repository's own files (allowlists, signed baselines, Markdown), and git. Each of those became a
native fact or functor in `lib/core.rr`, or a Datalog relation in a library. Three needs became language:

- `join`, an ordered string aggregate, because nothing else can build the exact text a signature covers;
- `ordered by earlier`, for state where only some writes count;
- `dfs_first`, the first target a depth-first search reaches, for checks whose answer is "the first call that
  leads to X". It is computed set-based; only cycles need a search.

A few directives were added for how findings are reported: findings at a file with no node, a code taken from a
column, gates, and budgets read from files.

Some needs are outside the language on purpose. Checks that run repository code (pytest collection, import-time
registries, behaviour probes) get those facts from a file a CI step writes. Rerail never runs the code it checks.

Two companion pages say how the engine runs what this page designs:

- [FINDINGS-LAYER.md](FINDINGS-LAYER.md): how rows become findings, which files are Unknown, and what `.gate`,
  `.budget`, `.on syntax_error`, `code {c}`, `.mode` and `.severity` do;
- [RUN-FACTS.md](RUN-FACTS.md): the artifacts a CI step writes, how Rerail knows they are for the code being
  checked, and the five checks that need them.

[../ALIGNMENT.md](../ALIGNMENT.md) records how this notation matches the other branches': the `.rdl` dialect of
the final shape, and the JSON policies of the lowering and data-layer branches.

How this was done:

1. Each check was read in the pinned scripts (revision `2b1504ad`) and in its candidate JSON port, if it had one.
   Every need was tagged as already expressible, library, native, language or policy, with `script.py:LINE`
   citations. Eight files, one per group of checks: [checks/group-1.md](checks/group-1.md) …
   [checks/group-8.md](checks/group-8.md).
2. The needs were merged and designed into the core, the libraries and the directives (below).
3. Three checks were written as worked ports that use the new parts, plus one sketch:
   - [file-length.rr](file-length.rr): a JSON allowlist, a signature checked in Datalog, a gate.
   - [noqa-sec-ratchet.rr](noqa-sec-ratchet.rr): a line scan, codes from a column, a ratchet against a signed
     baseline.
   - [test-budget.rr](test-budget.rr): a run artifact (pytest's count), a static bound that works without it, a
     gate. Its signed layout was checked against the real signature too.
   - [versioned-table-names.rr](versioned-table-names.rr): file-wide state in visit order, as plain recursion.
   - [sketches/owner-code.rr](sketches/owner-code.rr): the one shape that needs `ordered by earlier`.
4. Every check was then mapped onto the design: [checks/COVERAGE.md](checks/COVERAGE.md).
5. `site/check_rr.py` checks every `.rr` file for grounded rules, duplicate declarations and, new here,
   stratification. All files pass.

## The three language additions

### 5. `join`: an ordered string aggregate

```
s = join x by k sep "," : { r(k, x) }
```

`s` is every `x` of the rows, ordered by `k` (ties broken by `x`), separated by `","`. It is `""` when there are no
rows. Symbols sort by Unicode code point and numbers by value, as Python's `sorted()` does.

Why it is needed. Soufflé's aggregates are `count`, `sum`, `min`, `max` and `mean`; none builds a string. The
data-layer repository signs its allowlists and baselines. Each signature covers
`json.dumps({...}, sort_keys=True, separators=(",", ":"))` of sorted entries. To check one, a rule must rebuild
those exact bytes from rows, and only an ordered string aggregate can. The same holds where a sorted list is part
of a finding's identity. Most uses the analyses listed were for messages; those drop out, because the findings
contract leaves messages free.

```
// lib/data-layer/signing.rr: the canonical message of an allowlist.
allow_message(l, msg) :- allow_sig(l, linter, _),
    body = join q by e sep "," : { allow_entry(l, e), q = @json_str(e) },
    msg = cat("{\"allowlist\":[", body, "],\"linter\":", @json_str(linter), ",\"version\":1}").
```

Both layouts in `signing.rr` (allowlist and ratchet baseline) were checked against the repository's real
signatures: built as these rules build them, each message verifies. The alternative was a native `signed=` option
on the readers, one per layout. It was rejected: it fixes one repository's scheme in the engine, and a second
repository would need new engine code.

### 6. `ordered by earlier`: recursion through `!` and aggregates

```
.decl src_code(v: node, c: symbol) ordered by earlier
```

Soufflé rejects any relation that depends on itself through `!` or an aggregate. A few checks need exactly that.
A name's state at node n is set by its latest earlier write *that qualifies*, and whether a write qualifies depends
on the state where the write happens. In expression_owner_propagation, a name's code is the code of its latest
assignment whose value has a code. Rebinding to a value without one keeps the old code. The value's code may come
from another name's state at that assignment. "Latest qualifying" is a `max` over the recursive relation.

It is well founded: every step goes to an earlier node. A relation declared `ordered by earlier` may be used under
`!` or in an aggregate in its own recursion when the use's first column `m` is tied to the head's first column `n`
by `earlier(m, n)`, in the same body or aggregate. `site/check_rr.py` enforces the tie: it rejects the sketch if
either the declaration or the `earlier` tie is removed. The engine evaluates such a relation per document in visit
order, so the rows at n depend only on rows already final.

```
// data-layer/sketches/owner-code.rr
src_code(n, c) :- ast_name(n, x), expr_ctx(n, "load"), frame(n, f),
    k = max i : { write(w2, f, x, v2), earlier(v2, n), src_code(v2, _), visit_index(w2, i) },
    visit_index(w, k), write(w, f, x, v), src_code(v, c).
```

**Most state does not need it.** Four analyses asked for ordered recursion. On a closer reading, two of them are
plain positive recursion, which Soufflé already accepts:

- versioned_table_name_manipulation keeps two file-wide maps updated as the visitor walks. Every assignment resets
  the name before it stores, so "the latest write before n" is a `max` over syntax. Only what that write stored
  is recursive, and it is used positively. A def that returns a suffix adds itself without resetting, which is a
  union, also positive. [versioned-table-names.rr](versioned-table-names.rr) is the whole check, and it
  stratifies.
- python/flow's `always_raises` ("every branch of the last `if` raises") was first written with
  `count : { … !always_raises(…) } = 0` and `ordered by within`. "Every branch" is a chain over the `elif`/`else`
  clauses, so it is now positive recursion, and `ordered by within` was dropped from the design.

The rule of thumb, in core.rr: use `ordered by earlier` only when *which* write counts depends on the recursion.
The coverage pass ([checks/COVERAGE.md](checks/COVERAGE.md), "Ordered recursion verdicts") applies it to every
check that claimed the need.

### 7. `dfs_first`: the first target a depth-first search reaches

Three checks search the call graph depth first and report the first path they find: LEG-083 in
legacy_execution_surfaces, `_transitive_sink` in serverplan_state_tripwires, and the transitive helpers in
lazy_architecture. The finding sits at the root's first call on that path. Plain reachability says whether some
path exists, not which one a search finds first, so these three were recorded as approximate. The Data-layer
branch reproduces them with a native producer (ordered_first.rs). This addition gives the same meaning as two
built-ins, in the shape of `nearest`, which already takes relations as arguments.

```
// hit: the first eager call a depth-first search from f reaches. leg_edge(f, n, i)
// lists f's calls in visit order i; first is the i of the call that starts the path.
LEG_083(c) :- leg_root(f, _),
    dfs_first_memo(f, _, i, leg_edge, eager_call, leg_root),
    py_frame(c, f), visit_index(c, i).
```

**Meaning.** `dfs_first(r, hit, first, edge, target)`: search from r, trying each node's edges in increasing
order (`edge(src, dst, ord)`), entering each node once and skipping nodes on the current path. hit is the first
`target` an edge reaches, and first is the ord of r's edge that starts the path. r is never its own hit; a rule
that wants it writes `target(r)` beside the call, which is exactly the producer's `match: "node"` mode (the spike
checks this). `dfs_first_memo(…, roots)` is the scripts' variant: roots are searched in the order of `roots`'
second column with one shared memo, and a node finished while a cycle was open is remembered as reaching
nothing. That memo is why the scripts miss some paths through cycles. Use `dfs_first_memo` only for parity.
The second column may be a number or a symbol; symbols compare as text, which is what Soufflé's `<` on symbols
does (its interpreter decodes both and compares the strings; only `ord()` gives the table index), and what the
scripts' `sorted(symbol)` does.

**Supplied state is a rewrite, not an argument.** The producer also accepts an initial visiting set and an
initial memo (its supplied-fact API). Both are expressible by rewriting the two relations the built-in already
takes, so the language needs no further arguments:

- a node in the visiting set (or with a negative memo entry, which behaves the same): drop its edges, and the
  edges into it, except that an edge into a target still counts as a hit when edges are matched;
- a node with a known answer (t at location l): replace its edges with one edge to t at l. In node mode it stops
  being a target itself, because the memo is read before the node's own target test.

```
edge2(u, d, o) :- edge(u, d, o), !avoid(u), (!avoid(d) ; target(d)), !known(u, _, _).
edge2(v, t, l) :- known(v, t, l).
```

The native spike checks the rewrite against the producer on 20,000 graphs with random visiting sets and memos,
both modes, both variants. The one condition: a known answer names a target that is itself neither avoided nor
known, as a memo entry the search wrote always does.

**Stratification.** A relation passed as an argument is read whole: one more edge can change which target is
first. `site/check_rr.py` now treats every relation argument (of `dfs_first`, `nearest`, `within_stop`, …) as a
negative dependency, so `edge` and `target` must be complete before the search runs.

**How the engine computes it.** The answer is set-based everywhere except inside cycles, because a depth-first
search never has to back out of a node that can reach a target:

1. `live(v)`: v can reach a target. One backward closure.
2. `next(u)`: u's first edge, in order, into a live node (a call to itself is skipped, as the search skips it).
   One `min` per node.
3. `first_of(v)`: follow `next` to a target. `next` is a function, so this is a walk up a forest: one wave per
   level, or about log(depth) waves by pointer doubling.
4. `knot`: live nodes on a cycle (strongly connected components of two or more nodes, a linear native pass).
   Only these can be cut by the current path or poisoned in the memo.
5. A root whose `next` chain never meets the knot is answered by `first_of`, whatever the order of roots. A root
   whose chain meets the knot at u is answered by a search from u, which steps into a node only if its own chain
   meets the knot. Without the shared memo, u's answer is the same for every root and is computed once.

Why step 5 is exact: every edge before `next(u)` leads to a node that cannot reach a target, so the search finds
nothing there; and a node outside the knot cannot be on the current path or poisoned, because either would put it
on a cycle with the path. So the search follows the `next` chain until it enters the knot.

The order of roots matters only to `dfs_first_memo`, and only where chains enter the knot. Which target is first
matters only for rows that become findings, so with the magic-set pragma the search runs for those rows alone.

**Measured.** Two implementations, both checked against a port of ordered_first.rs, both modes, both variants,
with no disagreement: a SQLite + Python spike (25,000 random graphs) and a native Rust one with no SQL and no
materialised tables (40,000 graphs, plus 20,000 with supplied state). Planted bugs (a fresh memo where the
scripts share one; a rewrite that keeps a known node as a target) are caught by both.

SQLite + Python:

| Graph | Fresh-memo oracle | Shared-memo oracle (scripts) | Set-based |
|---|---|---|---|
| 5,000 functions, 25,000 calls, recursion inside modules | 3.4 M steps, 0.60 s | 12 k steps, 0.01 s | 0.09 s SQL; 73 search steps (fresh), 295 (shared); knot 30 nodes |
| 100,000 functions, 500,000 calls, same shape | not run (quadratic) | 203 k steps, 0.25 s | 2.2 s SQL; 15 k / 36 k search steps; knot 739 nodes |
| 5,000 functions, one cycle through almost all of them | 2.5 M steps, 0.45 s | 11 k steps, 0.01 s | 0.09 s SQL; 1.7 M search steps (fresh), 10 k (shared); knot 4,791 nodes |

Native (spikes/dfs-first/native; every set pass is a linear scan of a CSR edge array, Tarjan reads it directly):

| Graph | Set passes | Every root, fresh memo | Every root, shared memo | Scripts' search (oracle) |
|---|---|---|---|---|
| 5,000 functions, 25,000 calls, module recursion | under 1 ms | 449 search steps, under 1 ms | 1,564 steps, under 1 ms | fresh: 7.7 M steps, 0.25 s; shared: 13 k steps, 1 ms |
| 100,000 functions, 500,000 calls | 0.020 s | 15 k steps, 0.004 s | 31 k steps, 0.002 s | shared: 204 k steps, 0.026 s |
| 1,000,000 functions, 5,000,000 calls | 0.20 s (load 0.04, live 0.05, next 0.004, first_of 0.03, knot 0.05, ent 0.03) | 369 k steps, 0.06 s | 441 k steps, 0.03 s | shared: 2.0 M steps, 0.32 s |
| 5,000 functions, one cycle through almost all | under 1 ms | 4.3 M steps, 0.16 s | 11 k steps, 1 ms | fresh: 4.9 M steps, 0.21 s |

Peak resident memory at 1,000,000 functions: 435 MB, including the oracle's memo. CPU time, user+sys; the machine
was heavily loaded (load average 20–150; 50–110 for the real-data, thread and GPU runs), so times are upper bounds.

On the real repository. spikes/dfs-first/realgraph.py rebuilds LEG-083's call graph from the data-layer
repository, reading blobs through `git cat-file` and parsing them with `ast` (nothing from the repository
runs), and runs a copy of the script's own `_transitive_eager_resolve`. The copy is checked against the real
linter's output: the data-layer worktree keeps the fact cache of a lint-all run at 41ab36f, and on all 355
files it covers, the copy gives the same LEG-083 rows (spikes/dfs-first/vs_cache.py).

Four versions of the search are compared, each against the set-based plan on the rewritten graph (legacy
surfaces blocked). The oracle is also compared with the script.

- R: the script as it is (`dfs_first_memo`, any `.resolve()` is eager);
- S: one memo per root (`dfs_first`);
- U: the corrected eager test (lazy terminals only: no arguments, receiver not a pathlib path);
- T: both fixes.

| Graph | Roots | R | S | U | T | Set passes |
|---|---:|---:|---:|---:|---:|---|
| LEG-083's files, 2b1504 (2026-10-18): 63,191 functions, 73,848 calls | 61,403 | 18 | 18 | 0 | 0 | 1 ms; knot 2 |
| LEG-083's files, 41ab36f (2026-08-20): 46,070 functions | 44,811 | 56 | 56 | 40 | 40 | 1 ms; knot 0 |
| every .py file, 2b1504: 154,573 functions, 135,199 calls | 152,753 | 1,830 | 1,864 | 1,234 | 1,234 | 4 ms; knot 65 (3 with lazy terminals) |

The set-based plan equals the script's answer on every root of every version, on one core, 8 threads and
the GPU.

What the versions show (data-layer/upstream/LEG-083.md, with a patch for the script):

- The eager test, not the search, is what is wrong today. At the October commit all 18 LEG-083 findings
  in its files are a pathlib `resolve()` or a `resolve(arg)` lookup, and none is a lazy terminal.
- The shared memo loses 34 functions over the whole repository, all in one group of mutually recursive
  renderers in `scripts/generate_rust_intents.py`, behind a pathlib `resolve()`. With the corrected test
  it loses none (U = T), but which functions it loses depends on how their names sort, so the port uses
  `dfs_first`.
- The script's two entry points scan different files. lint-all reads 14 directories, `--check` the whole
  repository. In August lint-all's 10 LEG-083 findings were all false positives, and the 40 real ones were
  outside its directories.

The target relation decides how exact a `dfs_first` check is. The search reproduces the script exactly;
the corrected `target` is what makes it right. The sketch (sketches/transitive-eager.rr) writes the
receiver test as `path_value`, over python/names.rr's bindings. Real call graphs are mostly dead: under 2%
of functions are live, so steps 2-5 touch few nodes.

Threads and GPU. Steps 1-3 and 5 run as waves: `live` as a frontier per level, `next` as one pass per node,
`first_of` and `ent` by pointer jumping (log₂ n rounds). Step 4, Tarjan, stays on one core. On 8 threads and on
Metal (Apple M5 Max) every array equals the sequential pass on 10,000 random graphs (the GPU on every tenth), on
the bench graphs and on both real graphs.

| 1,000,000 functions, 5,000,000 calls | Wave form, 1 thread | 8 threads (CPU summed over threads) | Metal, steps 1-3 |
|---|---|---|---|
| set passes | 0.13 s (live 0.04, next 0.003, first_of 0.02, knot 0.04, ent 0.02) | 0.14 s | 0.004 s GPU; 0.036 s host (u32 copies, reverse CSR); 2 command buffers |
| every root, fresh memo | 0.06 s | 0.08 s | — |
| every root, shared memo | 0.03 s | 0.03 s | — |

The wave form on one thread is a little cheaper than the sequential passes above (0.13 s against 0.16 s without the load). Threads add little CPU. They pay off where the work is the searches: on the giant cycle, the fresh-memo
searches (8.6 M steps, 0.97 s CPU) are independent and spread over the threads. Under `dfs_first_memo` only roots
that meet no cycle run in parallel; the rest keep root order. On the GPU, the frontier of `live` empties inside
one batch of 32 waves with no host wait between them; the host's share is building the reverse CSR, which the
fact store would already hold. Peak resident memory with both paths: 567 MB.

Three readings. For `dfs_first`, the semantics this design recommends, the plan replaces a search per root,
quadratic, with linear set passes and a search confined to cycles. For the scripts' variant, the native plan is
also cheaper than their own linear search (0.23 s against 0.32 s at a million functions, set passes included),
and it runs in waves with the other facts; the SQLite spike's extra cost was SQLite's, not the plan's. A cycle
through most of the graph is the case no plan avoids: depth-first order on general graphs is P-complete (Reif
1985), and the fresh-memo search there costs what the oracle's does.

**Prior art.**

- Reif, "Depth-first search is inherently sequential", IPL 20(5), 1985: deciding depth-first visit order with
  fixed edge order is P-complete. Why step 5 keeps a search for cycles.
- de la Torre and Kruskal, "Fast parallel algorithms for all-sources lexicographic search and path-algebra
  problems", J. Algorithms 19(1), 1995; and Theory Comput. Syst. 34(4), 2001: lexicographic depth-first search
  on DAGs is in NC, through a correspondence between greedy search and lexicographic path problems. Steps 1–3
  are that correspondence made concrete.
- Naumov, Vrielink and Garland, "Parallel Depth-First Search for Directed Acyclic Graphs", NVIDIA NVR-2017-001:
  depth-first parents of a DAG by path comparison or shortest paths, in a few BFS-like GPU passes. The closest
  system to this plan; it does not handle cycles.
- Wyllie, "The complexity of parallel computations", Cornell TR79-387, 1979 (pointer jumping); JáJá, *An
  Introduction to Parallel Algorithms*, 1992: following `next` to its end in log(depth) waves.
- Greco and Zaniolo, "Greedy algorithms in Datalog", TPLP 1(4), 2001: `choice-least`, the deterministic form of
  step 2. Soufflé's own `choice-domain` is documented as non-deterministic, so it cannot express `next`.
- Fleischer and Shallit, "The state complexity of lexicographically smallest words…", DLT 2020: the smallest
  word of an automaton, found by first pruning to states that can reach acceptance (step 1's `live`).
- Chen and Warren, "Tabled evaluation with delaying for general logic programs", JACM 43(1), 1996: SLG tabling,
  the standard repair for memoised top-down search inside cycles, which is what the scripts' memo lacks.
- Zhao, Subotić and Scholz, "Debugging large-scale Datalog", TOPLAS 42(2), 2020: Soufflé builds a witness after
  the fixpoint, on demand. The same split as here: sets first, the chosen path last.

## Native facts and functors (lib/core.rr)

Native means the binary computes it: IO, parsing a format, or a fact the tree-sitter tree does not show. Each entry
says why Datalog cannot derive it.

| Fact or functor | What it gives | Why native |
|---|---|---|
| `source(d, sp)`, `src_line(d, l, sp)`, `line_count(d, k)`, `end_line(n, l)`, `col(n, c)` | the raw text, physical lines (Python's `str.splitlines()` terminators), extents | Comments and blank lines are not in the tree; splitlines has eight terminators |
| `.scope text "**/*.md"` | Markdown, Rust or SQL files read as text, with no tree | No grammar for them |
| `op_at(n, i, s)` | operator tokens in order; `not in`, `is not` as one token | `op` is unordered; chained comparisons need the order |
| `syntax_error(d, l, msg)` | the file does not parse, by CPython's grammar (a native parser: Ruff's) | Tree-sitter recovers from errors, so the tree cannot show it |
| `earlier(m, n)`, `visit_index(n, i)`, `ast_depth(n, k)` | CPython's `ast` visit order and depth | Decorators and return annotations come after a def's body, defaults after all parameters; rebuilding that from fields needs ancestor pairs |
| `str_part(lit, i, v, at, spec)` | str constants as CPython builds them: escapes decoded, adjacent literals merged, f-string pieces | Literal decoding, native (checked once against CPython offline) |
| `@place(path, line)` | a location with no node: a file, a line of it, a missing file | A finding needs a location |
| `@json_str`, `@sha256`, `@nfkc`, `@count_matches` | JSON escaping as `json.dumps`, digests, identifier normalisation, match counts | Byte-level text functions |
| `@ed25519_ok(key, message, sig)` | signature check | Cryptography |
| `@tree_digest(n)`, `@unparse(n)` | `ast.dump`-style comparison; canonical text for messages | Hashing and printing a subtree, native |
| `git_tracked`, `git_rev`, `git_has` | the git index, the revision under check and its base | Git |
| `.input … IO=json \| yaml \| lines (dir=) \| file`, `path=`, `at="base"`, `input_error` | repository files as rows; a file at the base revision; a row per missing or malformed file | Format parsing and IO |

A fact that only a run of repository code can produce is read with `IO=json` from a file a CI step writes. It is a
declared input of the policy, so a missing file is an `input_error`, never a silent "no findings".

## Library relations

A library relation is a named derived fact, and follows the admission rules of the derived-facts design (Pulling
it all together, decision 0005): its name is a concept, not a rule's name; at least two checks use it; and it has
at most as many rows per document as the document has nodes. A helper that one check needs stays in that check.

**lib/python/ast.rr** (new) is Python as CPython's `ast` module sees it. The scripts walk `ast`, not tree-sitter,
and the two differ in ways that change findings:

- `ast_name(n, x)`, `expr_ctx(n, c)`: which identifiers are `ast.Name`, and their Load/Store/Del context;
- `py_frame(n, f)`, `def_frame(n, f)`: the def that holds n, with decorators, defaults and annotations inside
  it, as `ast` nests them;
- `sig_param(f, p, nm, i, kind, ann, dflt)`, `param_named`: parameters by kind (positional-only, positional,
  `*args`, keyword-only, `**kw`);
- `type_param(o, p, nm, i, kind, bound, dflt)`, `tparam_named`: PEP 695 type parameters (TypeVar, TypeVarTuple,
  ParamSpec) with bound and PEP 696 default (broad_type_surfaces' BTS008/BTS009);
- `dotted_expr`, `leaf_name`, `call_name`, `call_dotted`, `chain_root`, `base_name`, `from_module_raw`:
  names as the scripts spell them, unresolved;
- `assign_target`, `assign_value`, `assign_ann`, `body_stmt`, `body_len`, `docstring`, `py_if`, `py_orelse`,
  `bool_operand`, `cmp_pair`: statement shapes as `ast` has them (`a = b = v` is one Assign; a chained comparison
  has ordered pairs);
- `fstring`, `string_form`, `str_const`, `ast_constant`: strings and constants;
- `class_path`, `def_chain`, `py_qualname`, `path_module`: qualified names, `<locals>` included;
- `ann_node`, `ann_name`, `ann_head`: names inside annotations, string annotations parsed;
- `latest_assign`: last write before a read, a `max` over visit order;
- `same_file_call`, `name_reach`: calls followed by spelling, as several scripts do.

**lib/data-layer/signing.rr** (new) holds the repository's key, its unsigned placeholders, and its three signed
layouts: allowlist, named multi-allowlist and ratchet baseline. Each layout gives a `*_status` of `valid`, `invalid`
or `unsigned`. The library reports; each check decides what a status means, because the scripts differ. An empty
allowlist is not checked, and a missing baseline passes.

**Changes to existing libraries.**

- imports: `exported(m, x)`.
- flow: `always_raises`.
- names: `type_checking_block` tested by qualified name. Two real cycles that Soufflé would reject were found and
  fixed (see "Bugs found" below).

**What stays in the check.** Decision 0005 admits a library relation only when it is concept-named and used by at
least two rules. Check-specific classifiers stay in the check file. The coverage table marks them LOCAL. They need
nothing new.

## Directives: how findings are reported

The findings contract makes a finding a key: (policy, code, path, line), with path or line absent for
repository- or file-level findings. Order, multiplicity and messages are free. Several needs from the analyses
dropped out under it: messages that list joined names, two findings at one key, and first-witness choice.

| Directive | Meaning | Needed by |
|---|---|---|
| `V(@place("a.json", 1), …) :- …` | a finding at a file and line with no node there; line 0 for the whole file | missing files, allowlist and baseline outcomes, Markdown lines |
| `.violation V(at: span, c: symbol) code {c}` | the code is a column | open code sets: SEC codes, pattern names |
| `.violation V(…, t: symbol) tag {t}` | an extra key field, so findings at one place stay apart | one finding per failing probe |
| `.severity V warning` | per-code severity, for presentation; it never decides blocking | warning-only checks |
| `.mode log CODE` | these codes are reported and never block | findings that only a ratchet enforces |
| `.gate V` | when V has keys they are the report, and the policy's other files are Unknown | bad signatures, missing baselines: the scripts exit before scanning |
| `.on syntax_error report CODE`, `.on input_error report CODE` | report those as findings too | scripts that report their own extraction failures |
| `.budget V by code from base [floor] [drop N%]` | findings may exist up to ceilings that are rows of a relation, usually a verified baseline; counts are of distinct keys | ratchets over findings, read from `.ratchet-baselines/*.json` and similar |
| `.artifact "name" inputs …`, `.input … artifact=` | facts from a file a CI step wrote, and the files it was built from | the five checks that run repository code |
| `.group G = "A", "B"` | a named set of codes for budgets | ratchets over category groups |
| `.scope report dirs …`, `.scope text …`, `.scope tests false` | where findings land; text-only documents; test directories | checks that read outside where they report |

A missing or badly signed baseline is an ordinary violation, usually a `.gate`. It is not a special outcome of
`.budget`: the check states the script's own behaviour. In noqa-sec-ratchet, a missing baseline passes and an
unsigned one fails.

A ratchet on a metric (annotations, tests, tracked documents) is a rule that compares the metric with its
ceiling, not a `.budget`. A budget counts finding keys, and a metric counts what the script counts: two SEC codes
on one line are two annotations but one key per code. FINDINGS-LAYER.md, section 6.

## Bugs found on the way

The new stratification check in `site/check_rr.py` found two cycles through negation in the existing Python
libraries. Soufflé would have rejected both:

1. `resolve_export` → `!exports` → … → `resolve_export`. It now negates `exported(m, x)`, which does not depend
   on resolution.
2. `import_time` → `!type_checking_block` → `sym_of` → `resolve_export` → `exports` → `import_time`.
   `type_checking_block` now tests the qualified name (`qn`). The cost is one case: a `TYPE_CHECKING` that is
   re-exported through another module is not recognised.

One analysis misread its script. versioned_table_name_manipulation's f-string exemption returns early when the
number of placeholders is *not* one. So `f"{a}_log"` is reported and `f"{a}{b}_log"` is not, the reverse of what
group-2.md says. The port follows the code.

## Coverage

135 checks were analysed; the other 7 were already written by hand. Each was mapped onto the design in
[checks/COVERAGE.md](checks/COVERAGE.md). Where a check has both a suite plugin and a CLI, the mapping follows the
plugin.

| Status | Checks |
|---|---|
| expressible with the design | 122 |
| expressible, with facts from a CI-written artifact | 5 (linter_enforcement, explicit_semantic_ids, plan_composition_architecture, serverplan_target_architecture, test_budget) |
| approximate | 1 (execution_authority_bypass: a registry read statically) |
| gap found by the mapping | 7, all now designed in (below) |

What the checks use most:

| Element | Checks |
|---|---|
| python/ast.rr relations | 76; `sig_param` alone is 28, `ast_name` 19 |
| check-local helpers | 60 |
| findings at `@place` | 46 |
| source text and lines | 38 |
| signing.rr | 30 |
| `.gate` | 27 |
| `.scope report / text` | 23 |
| file readers | 21 |
| dropped by the findings contract | 20 |
| `.budget … from` (metric ratchets among them are now rules; FINDINGS-LAYER.md §6) | 16 |
| `join` | 16 |
| `syntax_error` | 14 |
| `str_part` | 12 |
| visit order | 10 |
| `ordered by earlier` | 2 (plan_helpers_allowlist, expression_owner_propagation) |

The three approximations:
- execution_authority_bypass reads its registry statically, so the port matches only while the registry stays a
  literal.
- serverplan_state_tripwires and legacy_execution_surfaces use a memoised search. It answers "not reached" on some
  paths through cycles. A Datalog fixpoint does reach them, so the ports would report more.

The mapping's gaps, and how each was closed in core.rr:

| Check | Need | Resolution |
|---|---|---|
| cached_pure_lazy_allowlist | stored hashes are SHA-256 of `ast.dump` text (verify_cached_pure_lazy_allowlist.py:160-170, :470) | Re-sign the allowlist once with `@tree_digest(n)` (no CPython, so no exact `ast.dump` text) |
| permission_test_classification | JSON Lines files; arrays nested in arrays (`groups[].operations[]`, :552-597) | `IO=jsonl`; a `*` step in `path=`, each `*` a leading column |
| matching_architecture | scope from `--application-root` (:1162-1172) | `.scope dirs option("application_root")` |
| architecture_rule_docs | a cited commit exists; files at that commit (:219-247) | `git_commit(sha)`; `git_has` for any commit reachable from head |
| standalone_linter_entrypoints | a suite fails when a member policy fails or times out (plugin.py:127-152) | `.requires "policy", …`: one finding per member that blocks or ends Unknown(budget or timeout) |
| broad_type_surfaces, broad_type_surfaces_ratchet | PEP 696 type-parameter defaults (lint_broad_type_surfaces.py:701) | not a language need: Rerail's vendored tree-sitter-python already parses them (`type_default`, its patch 1); python/ast.rr's `type_param` reads bound and default |

## Prior work consulted

Recorded as for a decision: the source, what it says, and whether this design agrees.

- **Findings contract** (`rule-engine-direction/findings-contract.md`: keys :19-20, multiplicity :45).
  Agrees. Findings are keys, so message-only joins, duplicate findings and witness choice are not needs. File- and
  repository-level findings have no line, which is what `@place(path, 0)` and a missing path give.
- **Decision 0005, named derived facts** (admission rule 3). Agrees. New library relations are concept-named and
  shared by at least two checks (python/ast.rr, signing.rr). Single-check helpers stay in the check.
- **Stateful-reset semantics are a limit of relational lowering** (ast-index-spike history index, topic 2.21:
  DH L3791-3795; design index §7: LCS L191-195). Agrees for SQL, departs for this language. A guard cleared at
  each reassignment cannot be one flat anti-join. As Datalog it is "the latest reset before n", a `max` over
  syntax, followed by positive recursion; versioned-table-names.rr is that case. The remaining case, "latest
  qualifying write", is what `ordered by earlier` adds. It is evaluated in visit order, not lowered to SQL.
- **The policy layer (allowlist, ratchet, signatures) sits outside the query** (design index §18, LA). Departs, in
  part. Verification moves into Datalog (signing.rr), because each repository signs its own layout and the
  engine should not hard-code one. Enforcement stays outside the query: `.gate` and `.budget` are directives
  applied to the finished relations, not rules.
- **Lessons ledger #18: structured facts over text matching.** Agrees. `str_part`, `op_at`, `ast_name` and
  `expr_ctx` are structured facts where the analyses would otherwise match text. Text scans (`src_line`) are kept
  for checks that are about text: comments, line counts, Markdown.
- **Lessons ledger #21: expose extents and ordinals as columns, and let the caller join.** Agrees. `earlier`,
  `visit_index`, `ast_depth`, `end_line` and `col` are columns. `ordered by earlier` is evaluated over the same
  order, and no closure is materialised.
- **Rerail never runs repository code** (target architecture). Agrees: facts that only a run can produce are
  artifact inputs, not functors.

## Status

Nothing here has been compiled or run against an engine. `site/check_rr.py` checks groundedness, duplicate
declarations and stratification statically; it passes on every file. The signing layouts were checked against the
real signatures with PyNaCl. The rules that build those messages were not run. The findings layer and the run
artifacts are designed (FINDINGS-LAYER.md, RUN-FACTS.md), not built; the producers are sketches over the data
layer's existing functions. `dfs_first` exists as a spike (spikes/dfs-first), differential-tested against the
Data-layer branch's ordered_first.rs semantics; no engine has it.
