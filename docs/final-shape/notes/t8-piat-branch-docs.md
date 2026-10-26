# T8 notes: Pulling-it-all-together branch docs

Source root: `/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/docs/` (excluding `rule-engine-direction/`).
Citations are `file:line` relative to that root. Read-only research; nothing built or run.

Number tags: **M** = measured (the doc reports a run), **D** = design target/limit/declared constant, **A** = arithmetic/derived by the doc or by me from the doc's numbers (I mark mine "A, mine").

Common workload in most measurements: five pinned data-layer files, 1,026,452 source bytes, data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a` (e.g. native-relations-spikes.md:64-65). Many docs carry a banner: "historical experiments; final decisions are in sql-shape-spikes.md#final-decisions; the current branch removes experimental flags and alternate input adapters" (e.g. relational-execution-design.md:3-5).

---

## relational-execution-design.md (129 lines) -- "Shared SQL execution for data-layer rules"

Mechanism
- SQLite kept as the relational engine. Compatible detectors compile into one authored relational clause; source inputs loaded once; shared temp tables + indexes created once; bounded diagnostic SELECTs combined with `UNION ALL` (:9-12).
- A removed Rust spike used stage-name admission, content fingerprints, a retained-table cache; replaced with no runtime artifact cache, no content hashing, no table renaming, no stage-name special-casing (:14-16).
- Executor implements each stage as `CREATE TEMP TABLE ... AS SELECT ...`, creates declared indexes, collects SQLite statistics (ANALYZE-style) (:18-19). SQLite owns intermediates, indexes, recursive queries, joins, result queries. Connection closes after the clause completes (:21-22).
- Grouping: original generator produced 9 contract clauses due to stage-count and statement-size limits; new generator produces 3 compatible groups, each sharing source extraction, input insertion, temp tables, indexes (:26-28). Primary + fallback detectors share a group (production SELECTs apply `primary_file`/`fallback_file`); shared AST input reads the union of file domains (:30-32). Callback/inventory detectors keep separate source-trigger gates; project taxonomy and cross-file conditional rules keep separate scopes (:33-34).
- Limits (D): diagnostic SELECT packets <= ~12 KB before option expansion, each packet a SQL stage (:36-37); 16 KB authored SQL statement limit (:38); stage limit raised 32 -> 64 in native+TS schemas; largest quality clause uses 59 stages after typed-input partitioning (:39-40). Input relation count, witness limits, VM work limits, cancellation enforced by evaluator (:41).
- Invalidation: generator uses declared detector dependencies and source groups; does NOT discover equivalent runtime tables or maintain an invalidation protocol. "The existing final-result cache still handles unchanged files and edits" (:43-45). Regrouping changes clause IDs and finding IDs; parity compares path, line, code, multiplicity (:46). Failure boundary is clause-level: failed shared prerequisite fails that clause (:48).
- Materialized CTE vs temp tables: SQLite `AS MATERIALIZED` shares a producer within one statement; a direct test confirms two consumers execute it once and EXPLAIN QUERY PLAN reports materialization (:52-54). Temp tables chosen because stages declare explicit indexes; recursive descendant relation + indexes now has multiple consumers (:56-57). "No claim that temporary tables outperform materialized CTEs for every query" (:58).

Measurements (M; 5 files, 1,026,452 bytes, same native binary) (:93-106)

| Measurement | Independent SQL | Shared SQL |
|---|---:|---:|
| Cold wall, tracing off | 53.178 s | 18.308 s |
| Cold process CPU, tracing off | 46.707 s | 14.370 s |
| Edited-file wall, tracing off | 14.784 s | 4.086 s |
| Unchanged-file wall | 9.636 ms | 10.143 ms |
| Peak RSS | 1.346 GB | 1.334 GB |
| Cold wall, plans+sampling | 59.311 s | 18.805 s |
| File-local descendant builds | 30 | 5 |
| SQL stage builds | 890 | 443 |
| SQL index builds | 1,362 | 602 |
| AST input extractions | 1,155 | 355 |

- A (doc): 2.90x faster cold, 69.2% less cold CPU, edited-file 3.62x faster; unchanged-file latency did not improve; second pair 3.15x with plans and 3 samples (:108-110). Two matched pairs on shared host (:111).
- Root cause: descendant plans keep the same recursive SQL + indexed edge lookup; improvement is from executing fewer copies (:113-114). Baseline samples dominated by SQLite record comparison and B-tree navigation; shared-SQL samples expose input extraction and trace writing (:115-116).
- Validation: 200 quality cases (135 positive, 65 clean) pass; 40 resident-workspace transitions pass (:119-121). Profiling deadline <= 115 s (:124).
- Next per doc: query plans, unnecessary input columns, recursive closure size, input insertion cost; no parallel relational execution or complete batching claimed (:126-127).

Note: edit cost here is 4.086 s wall (M) for an edited file in the 5-file workload: i.e., per-edit cost is dominated by re-running the whole clause, not incremental.

---

## native-relations-implementation.md (175 lines) -- "Native forest traversal and deferred SQL columns" (at commit `f52817f`, :7)

Mechanism
- Reachability stage can declare `graph: "forest"`: native adapter builds an iterative forest index and enumerates requested subtrees; rejects cycles and multiple distinct parents before publishing; duplicate links, isolated roots, dangling endpoints, file-qualified identities keep semantics (:17-20). Default `graph: "directed"` keeps cycle-safe recursive SQL (:21).
- Both adapters share SQL identity normalization and indexed result relation; forest adapter reads SQLite rows directly (no JSON), writes results in bounded batches through existing insertion (:23-25). No rule-specific logic (:26). "Direct tape ordinals remain future work because authored graphs currently accept arbitrary SQL identities" (:27).
- Deferred columns: stage declares `deferred: {"text": "source_text(__path,__start,__end)"}` (:31-39). Physical table holds metadata + indexes; a SQL view supplies deferred columns on read (:42-43). Deferred expressions read only that stage's stored columns; cannot reference another deferred column or be indexed; >= 1 physical column (:44-46).
- `source_text(path,start,end)` reads the active immutable source snapshot in rule scope; holds shared source refs, returns borrowed slice to SQLite (SQLite copies; no intermediate Rust String) (:48-50). Missing path / null / invalid range / split UTF-8 -> SQL NULL (:51). Workspace sources restore through the versioned document store after eviction; unavailable versions fail explicitly (:52-53).
- Quality generator removes node text before matcher projection and typed input partitioning; defers exact source text in node-derived producers; keeps eager names, transformed text, conditional policy's thin text; window inputs select metadata before ranking; identifier text keeps materialization + index (:55-59). IR-name propagation stays in SQL "because its native experiment was slower" (:61).
- Lifetime: compiler records relation dependencies after option resolution; executor releases inputs and stage tables after last recorded consumer; deferred views and reachability views release backing tables together (:63-65). After input insertion, compact anchors keep spans + suppression marks while full Rust captures are released (:66).
- Invalidation: "Repository-wide source reads also register source-version dependencies. Same-length edits and edits without evidence therefore invalidate their results." (:67-68)
- Work limits: native graph work charged in units of 64 bounded steps including conservative copy-and-sort charge; derived result insertion spends SQL VM work (not AST-input-row allowance); cancellation active during native work + insertion; exhaustion = failure with no partial findings (:72-76). Initial implementation over-charged (each native step as a full unit; graph output as new AST input); 5-file workload exposed it; constrained closure test now compares native vs SQL under same allowance (:78-81).

Measurements (M; three paired runs, medians, 5 files) (:85-95)

| Median | Baseline | Candidate | Reduction |
|---|---:|---:|---:|
| Cold process CPU | 12.393 s | 11.687 s | 5.7% |
| Edit process CPU | 3.128 s | 3.045 s | 2.7% |
| Peak RSS | 1,074.6 MB | 983.6 MB | 8.5% |
| Cold wall | 24.993 s | 26.424 s | -5.7% |

- Host load varied; one pair ~identical cold CPU; supports modest CPU improvement + consistent memory reduction, not wall (:97-99). Build: native opt-level 1, thin LTO, 256 codegen units, system-malloc shim, profile feature (:103-105).
- Remaining costs (M, sampled cold run, accumulated leaf-span wall, may overlap): SQL materialization 9.13 s, index construction 4.56 s, input extraction 4.40 s, graph normalization 3.57 s, forest traversal + result insertion 0.73 s (:115-118). Samples: matcher anchor selection, child traversal, sorting, budget checks; later SQLite VM, B-tree movement, balancing, memcpy (:120-121).
- Next targets: repeated ownership materialization, SQL graph identity normalization, stage index construction, matcher anchor traversal; graph adapter still normalizes arbitrary SQL identities before native traversal; removing that needs an explicit identity contract (:124-127).
- Validation: 200 oracle cases; 219 production comparisons; 57 edit/restoration/removal transitions match (:131-134). Regressions include 30,000-edge chain, constrained work limits, UTF-8, SQL authorization, source eviction, source-only edits (:139). Tape adapter checks owner lookup work vs returned postings (:140).

---

## native-relations-spikes.md (232 lines) -- "Native SQL relation experiments"

Principle: keep SQL for rule decisions, joins, aggregation, diagnostics; native algorithms supply facts through functions and virtual tables (:9-10). Strongest candidate: native tree indexing behind existing reachability contract; broad vtabs, automatic column demand, generic inference need more work (:13-14). Measurements include preparation costs (:15).

Interfaces (:19-30)
- `native_nodes` vtab: reads real parser tape, projects text only when requested.
- `node_text`, `owner`, `is_ancestor` point functions over tape + owner metadata.
- `reachability` vtab + `is_reachable` fn sharing a strict forest index.
- `native_bindings` table + `binding_start` fn: builds existing `ScopeTable` once.
- Resolved calls: SQL joins over calls and bindings.
- `flow_has` fn + inferred relation: bounded worklist with dense bitsets.
- `--native-input captures` (shared capture projections, lazy field reads); `--native-input rows` (owned complete SQL row vectors, allocation control).
- `narrow` tape backend: metadata in SQLite, text via function.
- Replay compiler: SQLite bytecode dependencies, narrow tables, logical views, last-consumer release.

Storage/identity facts
- Native interfaces compile only with `code-rule-profile`; `NativeSqlSpike` class; parser adapter supports Python only (:32-34). Each instance owns an immutable source snapshot and connection; "Node ordinals belong to that snapshot. They are not persistent identities across edits or files." (:35-37)
- Equality/`IN` use native postings; SQLite rechecks predicates (type, collation); unsupported affinity conversions scan (:39-41). Point functions and relation queries share result; lazy inference starts at execution not planning (:42-43).
- Binding/resolved-call experiment is file-local; no cross-file resolution (:45-47).
- Owner adapter originally scanned every owner row per lookup; now reverse postings for function, class, outer-function owners built once; work proportional to requests + returned memberships; regression issues 200 owner requests; postings <= 3 memberships per tape node; built eagerly in constructor (:49-54).
- Capture adapter shares `Arc<Value>` projections and interns paths; avoids extra SQL cell vector; still retains all matcher projection values; "not a canonical typed column store" (:56-60).

Measurements (M)
- Complete captured-input SQL replay (input loading, planning, stage execution, result retrieval; parsing/extraction excluded; single runs) (:71-84):

| Backend | Pipeline CPU | Peak RSS |
|---|---:|---:|
| SQL control | 7.30 s | 640 MB |
| Owned-row virtual inputs | 7.19 s | 727 MB |
| Column demand and lifetime | 7.81 s | 589 MB |
| Native forest traversal | 6.70 s | 669 MB |
| Native IR-name propagation | 7.55 s | 666 MB |
| Combined | 7.44 s | 733 MB |

- Forest: graph-stage CPU 2.67 -> 1.99 s; traversal + pair indexing 1.23 -> 0.37 s; native loading/indexing +0.17 s; compact-key preparation still ~1.44 s; next improvement is reading tape identities directly (:89-93).
- Generic worklist `ir_names`: ~152 ms vs 9.5 ms SQL; breakdown ~64 ms SQL prep + graph retrieval, 20 ms serialization, 55 ms load+solve; files with no seeds still pay prep; keep in SQL (:95-99).
- Automatic demand: removes only 6 of 1,993 logical columns in main shared pipeline; conditional pipeline 1,363 -> 1,254 columns; wide intermediates and window ops keep fields live; reduces RSS but adds planning/view costs; "A stronger solution needs explicit narrow producer contracts and demand before extraction." (:101-105)
- Actual engine input adapter (:111-114): SQLite tables 36.92 s cold CPU / 1,116 MB; shared capture access 36.99 s / 1,104 MB. Capture adapter cuts input-stage CPU ~1.58 -> 0.27 s but SQL materialization grows ~1.10 s: moves work (:116-119). opt-level=0 build; do not compare with opt-level=1 reports (:121-123). Wall 45.94 vs 38.37 s but ~identical CPU (:124).
- Tape access suite (real tape construction + SQL access; identical digests) (:133-137): direct tape vtab 1.38 s / 395 MB; full SQL materialization 1.27 s / 448 MB; narrow SQL materialization 1.24 s / 429 MB. "Virtual access saves memory here, but repeated callbacks can outweigh initial materialization ... does not support replacing every table with a virtual table." (:139-141)
- Profiler: engine samples in matcher selection and tape navigation (`FileAnchorIndex::relation_selection`, binary searches, memmove); third sample SQLite record comparison + VM (:149-151). Replay samples: SQLite execution, B-tree, record compare, sort, alloc, copy, consistent with wide materialized relations and indexes; native solver not dominant (:155-157).

Recommended architecture (D) (:161-176): 1) keep shared SQL producers + batched request/result contract, carry scope guards into shared producers; 2) snapshot-scoped native fact sources with explicit identity, completion, budget, lifetime contracts; reuse binding/call analysis; 3) scalar fns for point membership/single value, vtabs for enumeration, batched lookup, reverse lookup, joins; 4) native forest indexing with direct tape keys, reject cycles/multi-parents in strict adapter; 5) materialize narrow frequently reused relations in SQLite, broad text lazy; 6) explicit field demand before matcher projection and extraction, cache demand by policy/options/schema/source version, release after last consumer; 7) keep IR-name inference in SQL.
- Data-layer PR 2539 transferable ideas: typed fact catalogs, field demand, shared producers, narrow rows, explicit invalidation (:178-180).
- Worklist is finite-set propagation over supplied edges/generators/kills, not a type system or CFA (:182-185). Solver state cap up to 8 million words per bitset matrix, three working matrices plus output (D) (:187). Forest enumeration allocates and sorts each result vector; only point ancestry uses constant-time interval checks (:188-189).
- Validation: combined replay 134 positive/66 clean (one nonrelational positive difference) (:194-195); 57 transitions (:196).

---

## shared-producer-contracts.md (220 lines) -- "Named shared producers"

Status (:3-8): engine contract catalog, live candidate exports, physical span and child demand waves implemented. Decoder reuse uses registered binding IDs; candidate captures export actual consumers and pinned source versions. Retained SQL tables remain physical owners; independent span and child demand share fills per dependency wave. Scoped node-property fills and repeated selector construction remain open. Physical `(fid, pre)` owners stream final projection into CTAS without a second private ephemeral copy. Shared upstream CTEs keep boundaries. "Generic selector-family packing remains rejected because measured CPU regresses." (:8). Principle: explicit contracts define sharing; automatic discovery only supplies review candidates (:12).

Definition locations (:16-21)
- Generic engine facts/native algorithms: engine backend, typed contract beside SQL template or Rust impl under `native/src/code_rule_dsl/`.
- Data-layer-specific derived work: pack, declarative producer docs under `packs/data-layer/candidates/producers/`.
- Rule-private: the rule/clause, local name.
- Registered contracts + bindings: evaluation planner registry under `native/src/code_rule_dsl/producers/` (references, doesn't copy). "The registry is a catalog, not another executor." (:23). Spans/child facts beside the decoder `native/src/code_rule_dsl/matcher/anchors/sql/decode.rs`; native producers beside `relational/producer.rs` (:24-25). No serverplan-specific algorithm in engine (:26).
- Pack producers use SQL or declared engine operations, no arbitrary native callbacks; pack loader validates/resolves before compilation; candidate producer docs unshipped until pack format declares publication and dependencies (:28-31).
- Namespaces: engine reserves e.g. `syntax`; pack owns e.g. `data_layer.serverplan`; private stage identity = rule + clause; only explicit contract refs enter shared namespace (:33-36).

Contract and binding (:40-75)
- Contract = one versioned name, implementation definition, input slots, typed options, output schema, completion rule; plus multiplicity, required ordering, input key for keyed completion; origins identify impl + registration site (:40-42). Example `syntax.source_spans.v1` accepts a syntax tape, returns spans for requested node keys (:44).
- Binding (within pinned evaluation snapshot) = `contract name + input bindings + typed options + semantic domain (when required)` (:47-51). Registry compares complete binding definitions and assigns compact IDs; hashes are diagnostic labels/lookup accelerators, "Hash equality alone never establishes sharing" (:53-54). No general cross-evaluation version graph; "Existing workspace fact invalidation remains authoritative across edits." (:55-56)
- Excluded from keyed fact identity: rule names, clause names, result packets, demand selectors, transport batches. A selector contributes requested keys. Answer-changing options/filters are semantic inputs; contract decides, planner does not guess from SQL text (:58-61).
- Child requests with different `field` options = different bindings under same contract; can share one SQL submission; physical transport dedups `(fid, pre, field)` keys before calling the child decoder (:63-66). `decoded_children` owner stores a row with NULL `child` for a completed absence; missing-coverage check tests for absent owner row, not NULL answer (:67-68). Child and span dependencies introduce separate barriers when later selectors need earlier answers (:69). Same name with different schema/impl = compile error reporting both origins and differing fields (:70-71).
- Exact structural dedup for compiler-generated expressions retained; no arbitrary SQL equivalence inference; undeclared authored SQL stays private until explicit extraction or proven exact match (:73-75).

Completion models (:79-95)
- Keys: union ready demand, compute missing keys, record completed keys independently of output rows.
- Domain: wait for declared input domain to close, publish complete derived relation once.
- File or root partition can be one key if contract declares it independently computable; aggregates, windows, negation use complete domains unless contract proves a smaller unit; transport batches do not define domains (:84-86).
- Controller tracks pending, admitted attempts, completed coverage; one writer owns publication (no distributed reservation); in-flight native work needs explicit admitted coverage; rows and completion publish atomically; empty results also complete their keys (:88-91). Demand sources stay open until consumers can't request more keys (:93).

Discovery tools (:99-131): `scripts/profiling/shared_work.py discover|check`. `discover` reports repeated materialized CTE structures from candidate replay captures, resolving local deps, typed params, CTAS projections, retained-table write epochs; per-capture only (:106-109). Separates constant-folding candidates from producer-extraction candidates; counts are SQL placements, not runtime build counts (:111-113). `check` reports conflicting names, duplicate local bindings, missing deps, cycles, bindings resolving to the same work, valid separate bindings and why, identical declarations under different names (:116-123). Unknown semantic fields fail; different demand sets/consumer labels don't prevent keyed sharing; different input generations, domains, typed options do (:125-127). Full identity comparisons even under simulated digest collisions (:128). Exit 0/1/2 (:130).

Manifest `rerail.shared-work-plan/v1` (:138-159): evaluation snapshot, contracts, bindings. Contract fields: `name`, `inputs`, `options`, `columns`, `key`, `completion` (`keys`|`domain`), `multiplicity`, `ordering` (set/bag, ordered columns, direction, NULL placement), `implementation` (exact template/backend def incl. revision), `origin` (excluded from identity) (:142-150). Bindings reference a contract and supply inputs, options, consumers, optional diagnostic demand; inputs reference another local binding or an external source with explicit name and generation; domain bindings supply exact semantic domain descriptor; "The compiler must export membership and generations, rather than only an alias for a request table." (:152-155). Runtime registry accepts only trusted engine or validated pack definitions (:159).

Compiler integration (:163-178): candidate compiler exports live structural bindings via SQL capture; each binding keeps rule ID, clause ID, policy digest, result index, selector name; no authored file/line available from policy API (:163-165). First catalog milestone = structural property contracts unified across consumers; next = reviewed derived producers + frontier controller completion; cross-clause storage sharing separate (:175-178).

Implemented catalog boundary (:182-220)
- Candidate admission per plan before composition; compiler compares complete generated definitions (schemas, typed bound values, dependency identities); canonical topological numbering gives reproducible SQL names/placement; aliases are local, not sharing identities across frontiers or source generations (:182-186). Each composed program has one pinned tape/domain source slot; incompatible file-domain definitions can't share a program (:187).
- Catalog in `native/src/code_rule_dsl/producers/`; decoder defines `syntax.node_properties.v1`, `syntax.source_spans.v1`, `syntax.child_fields.v1` (:189-190). Identity = complete ordered binding values; no digest equality or SQL equivalence heuristic (:191). Child-field options typed integers; no pack declarations yet (:192).
- Input slot is language-qualified; capture resolves it against the workspace database and its complete ordered `(fid, language, digest)` generations; snapshot identifies the workspace DB; generation descriptor has actual membership + digests; "Templates never retain a resolved generation across edits." (:196-199)
- First physical slice: live span demand for one tape binding grouped by dependency wave; reads existing selector tables; publishes missing rows into `decoded_spans` under the owning transaction; each valid span key has one output row which also records completed coverage; empty-output producers still need separate completion records (:201-204).
- `candidateFrontierAudit()` compiles fixed permutations to detect order-dependent admission/placement (:210-214).
- Early connection check: physical public owner tables for scoped readers; passes publication barriers, pinned WAL snapshots, rollback, private scratch, forbidden private-table reads under production authorizer (:216-217). Protected-view experiment FAILS: SQLite loses view context for a count shortcut, and CTEs can impersonate accessor labels; don't extend the generated-view exception to shared owners (:218-220).

Relation names surfaced: `decoded_spans`, `decoded_children` (with `(fid, pre, field)` key, NULL child = completed absence). Fact-key shape is `(fid, pre)` (file id + preorder ordinal) (:7, :66).

---

## shared-producer-workplan.md (326 lines)

Context (:3-24)
- WP1 and WP2 have first slices; complete shared execution open. SQL recovery workplan defines order after the aggregate regression; WP1 starts at recovery step R4 (:3-6).
- Aggregate checkpoint recorded a **19.93-fold first-50 cold CPU regression** and failed full-repository 60-rule execution (M) (:9). Source-corpus parity passes but doesn't replace the repository gate (:10).
- Ranked opportunity plan supersedes small slices: R5-A removes 4,048 submissions vs 3,474 target; R5-B removes 1,435 vs 1,296; delivered C leaves 975 complete-guard submissions; final 100-submission gate mandatory (M/D) (:18-20). C delivers 1,275 actual removals vs fixed 1,261 target (:22).

Foundation (:28-36): discovery CLI + checker with 25 focused tests; engine retains decoded properties, spans, children in SQL; "Adding names to already shared rows alone does not establish a performance improvement." (:32). Scheduler handles explicit table dependencies + local lifetimes; clause executor batches native demands but clause databases still import compatible facts separately (:34-35).

Work items (:40-49): WP1 contracts/registry/export; WP2 first shared span producer; WP3 remaining structural contracts + demand prep; WP4 compiled frontiers, domain completion, complete lifetimes; WP5 workspace SQL owners across clause connections; WP6 pack producer declarations; WP7 reviewed derived-producer extraction; WP8 parity/profiling.

WP1 (:57-91): trusted engine catalog, full typed binding comparisons, clash diagnostics, live exports, early scoped-reader storage check. Exports resolve input slots from actual source membership and digests (:58). Registry serves structural integer options and direct tape inputs only (:61). "The evaluation pins input generations. Existing workspace invalidation remains authoritative across edits. This step introduces no general cache across evaluations." (:70-71). Storage check: one workspace writer + two scoped readers; physical public owner table; WAL snapshots, atomic publication, rollback, private scratch preserved; authorizer rejects private-table access and writes while admitting public owner + aggregates; protected-view alternative fails on count optimization and ambiguous CTE accessor labels (:83-91).

WP2 (:95-142) measurements (M)
- Production two-rule test preserves 130 findings across two result packets with one span-fill execution; original state guard reduces 22 fills per normal frontier to 2 (:96-97).
- Selected guard submits 9,481 statements vs 9,113 at catalog checkpoint (regression) (:102). Root cause: named provenance exposed changed clause order under **allocation-address sorting** and different upstream lowered frontiers (:103). Fix: independent bounded lowering per plan before canonical composition; all 4 permutations of the 139-plan state rule match (:105-106). Small guard then 8,669 submissions; matched CPU 2.318 -> 1.617 s (:107). Rejected: authored-order and work-estimate heuristics (:109).
- Per-rule file eligibility and conditional gates restrict demand union before producer execution (:116). Checkpoint finds a language-wide plan/file frontier despite narrower final matcher eligibility; "A complete compiler inventory must not become an eager Cartesian execution frontier." Batched request rows preserve eligible plan/file pairs without a host query per pair (:117-119).
- Executor unions ready span demand, computes missing coverage through batched SQL; completion records distinguish absent output from unfinished work; rows + completion atomic; admitted coverage prevents overlapping attempts (:121-123). Renderer replaces shared deps with owner references and rejects an embedded materialized copy of the owner; demanded-key fence stays before expensive source predicates; cheap single-use projections remain pipelines (:126-129).
- Acceptance includes disjoint-scope regression (eligible rule must not trigger another rule's universal preparation on unrelated files) and disabled conditional consumers contribute no demand (:139-140).

WP3 (:146-191) measurements (M)
- Child-field slice: field-qualified requests share one SQL fill per dependency wave; small guard 7,733 submissions vs 8,669 before; first captured frontier child fills 246 -> 11 while retaining 393 temporary selector tables; nine matched pairs median CPU 1.740 -> 1.708 s, overlapping ranges; SQLite heap peak unchanged; "no substantial CPU or process memory gain" (:147-153). Universal node-property fill already executes once (:156).
- Removing private ephemeral copy inside `(fid, pre)` owner CTAS: five pairs median CPU 1.651 -> 1.639 s, overlapping; heap peak and 7,733 guard unchanged (:158-162).
- Generic selector-family packing REJECTED: prototype reaches 4,109 submissions but median CPU increases beyond control variation; code removed; "Further batching must use typed operator requests before rendering complete SQL trees." (:164-167)
- Literal empty relations simplify without a dedicated owner/table, preserving set/bag (:179-180). Existing kind, value, vocabulary indexes stay direct access paths; does not restore rejected expanded-posting or generic row caches (:181-182).
- Aggregate checkpoint observes pager pressure and statement-journal writes with 1 MiB main and 512 KiB temp cache targets (M/D) (:188). Cache capacity is physical resource admission, not semantic identity (:190).

WP4 design (D) (:195-216): replace SQL-token dependency inference with explicit producer/consumer/ordering/storage edges compiled into indexed operation descriptors and adjacency arrays; runtime Discover/Build/Evaluate/Emit over affected frontiers; no scheduling task per node/key/rule instance (:195-198). Keyed producers advance on coverage; domain producers publish after exact input domain closes; recursion stays inside declared operations (:200-202). Demand-source leases; skipped branches close leases; retirement waits for readers and admitted continuations (:204-207). Memory admission includes retained owners, private algorithm allocation, readers, output buffering; spill or defer without rebuilding a live owner; "Arbitrary file splits cannot change a global domain." (:209-211). Native operations receive broad typed frontiers for count, scan, scatter passes; profile frontier width, barriers, op counts to expose GPU launch shape (:215-216).

WP5 (:220-237): reusable rows and owner indexes in workspace SQL backing; one writer; scoped clause readers consume registered bindings without importing rows; publication barriers close readers and BLOB handles before writes or schema retirement; readers acquire published snapshot under pinned logical generations; each owner maintains access paths once; "Two clauses cannot create identical fact copies or rebuild the same owner index." (:229-230)

WP6-8: pack declarations; extraction needs two real consumers; "Cheap indexed reads and constant expressions do not become retained caches merely because they repeat." (:262); final gate; "Global analyses cannot use independent file shards merely to satisfy that deadline." (:285). Historical ~1,400-second observation cannot establish a CPU ratio without CPU evidence (:290).

Measurement gates (M) (:299-307): addon `86846fe4...`; State one file median cold CPU 2.597 s, RSS 452.5 MiB, SQLite exec alloc peak 68.5 MiB; Ten rules two files 13.106 s, 553.3 MiB, 98.6 MiB. Three rotated observations per side; tracing/capture off during CPU timing (:311-313). "Moving cost into another phase does not satisfy the performance gate." (:317)

---

## shared-syntax-requirement.md (47 lines)

- Requirement: reduce expensive preparation and fact construction before faster per-stage algorithms; the previous change (native forest) accelerated traversal but kept extraction, textual identities, repeated ownership construction, repeated indexes; median cold CPU reduction only 5.7% (M) (:5-7).
- One immutable document version supplies shared syntax metadata to all relational consumers; "Node identities must use parser ordinals qualified by file and source version." Consumers request fields before extraction; source text read only when necessary; native relations expose indexed lookup + batched enumeration to SQLite; rule decisions, joins, aggregation, inference, diagnostics stay in SQL (:9-13).
- Execution order (:17-21): build shared syntax metadata from parser tape; expose node, edge, owner relations without matcher captures or SQL reconstruction; integer node identities throughout authored structural SQL; enumerate descendants from same metadata without identity normalization; compare diagnostics, incremental results, prep counts, CPU, memory.
- Contracts (:25-37): "Facts belong to one exact source version and grammar. Edits, removals, grammar changes, and document eviction cannot reuse facts from another version." Shared facts = structural metadata, not retained SQL stages or detector results. Authored input controls each syntax relation's eligible files; sharing cannot widen source gates or rule scope. Incomplete syntax, unavailable versions, exhausted limits, cancellation = explicit failures; "Only complete metadata enters the shared cache." Indexes use postings or ranges with work proportional to requests + returned rows; predicates keep SQLite type/null/collation; deferred text keeps byte ranges + Unicode boundaries.
- Acceptance (:41-47): stop per-group construction of syntax captures, node, edge, owner tables + indexes; document cache shares prep across groups with explicit bounds; descendant execution avoids textual-key normalization; parity incl. same-length edits, restoration, removal, stored document eviction; every command < 2 minutes.

---

## shared-syntax-implementation.md (169 lines) -- "Shared syntax facts and direct tree execution" (commit `c4bd76bf`, :7)

Preparation / storage (:16-39)
- A document supplies shared immutable node metadata, field links, kind postings, parent postings, owner indexes (:16). Document cache keyed by exact source version, file context, grammar; an evaluation pool shares facts across clauses and repository consumers; both caches have residency limits; failed/incomplete prep never enters either cache (:17-20).
- Producer declares file eligibility input, fields, columns; input gate preserves source group + rule scope; column demand precedes extraction (unused field text never enters SQL rows); requested source text lazy; grammar comes from eligibility input (:22-26).
- "Node identities are integer parser ordinals within a file version. Fields return those same identities. SQL consumers retain file qualification in every join. No textual identities or runtime table fingerprints mediate sharing." (:28-31)
- Node, edge, owner virtual tables share metadata; equality and `IN` use postings or direct identity lookup; start ranges by binary search; upper end bound narrows start range; SQLite rechecks all predicates (end bound, affinity, nulls, collations) (:33-37). Owner indexes use authored kind and nearest or outermost selection (:38). No data-layer decisions in native (:39).

Execution (:43-66)
- Quality policy keeps 3 source groups, each uses shared syntax source instead of matcher captures + reconstructed node/edge/ownership tables; frequently reused derived relations keep SQLite materialization + indexes; name propagation and annotation resolution stay in SQL (:43-46).
- Direct descendant adapter: roots over syntax relation, enumerates visible nodes inside parser subtree, writes bounded batches, keeps SQLite indexes for consumers; avoids graph-key normalization, temp edge conversion, forest reconstruction (:48-51).
- **Affinity lesson**: "Unary plus removes declared affinity from temporary identity columns without changing their values. Without this change, integer inputs force numeric comparisons against the untyped key catalog. SQLite then uses only the file prefix of the identity index and scans keys repeatedly." (:54-57). Public syntax metadata and descendant identities keep neutral affinity; values stay integers; preserves full identity-index access downstream (:59-61).
- Shared execution allowance: SQL execution, regexes, copied source bytes, native work (:64-66).

Contracts checked (:82-87): root selection keeps field demand until all descendant requests declared; JSON object keys convert integer node identities to text; function-body scans apply start and end bounds before enumeration; evidence line numbers zero-based; suppression removes findings without removing syntax facts.

Measurements (M; three matched rounds, 5 files, tracing/sampling off) (:95-100)

| Median | Previous | Shared prep, normalized graph | Shared prep, direct descendants |
|---|---:|---:|---:|
| Cold process CPU | 12.619 s | 3.642 s | 3.192 s |
| Edit process CPU | 3.268 s | 1.672 s | 1.284 s |
| Peak RSS | 936 MiB | 485 MiB | 537 MiB |
| Policy compilation CPU | 431 ms | 77 ms | 83 ms |

- A (doc): shared prep + integer identities -71.1% cold CPU before direct adapter; direct adapter another -12.4%; total -74.7% cold, -60.7% edit, -42.7% RSS (:102-104). Direct descendants +10.8% RSS vs normalized control: "path-qualified result rows and two SQL indexes cost more memory than compact graph pairs" (:105-106). Direct adapter stays default for CPU (:107).
- Diagnostic trace (M): exactly 5 syntax preparations and 15 owner-index preparations; no node/edge/ownership table reconstruction; syntax prep 27 ms worker CPU; owner prep 8 ms; input extraction 1,865 ms -> 59 ms (:110-113).
- External samples: SQLite execution stacks 94.9% of active native samples (incl. callbacks to native sources); record comparison, memmove, vtab columns, VM self costs prominent (:116-118). Next costs: SQL materialization 1.536 s worker CPU, descendant indexes 397 ms (:119-120). Targets: `self_stores`, `scoped_nodes`, `arguments`, `py_names` for query-plan analysis; no evidence for native detector replacements (:121-122).
- Build: native opt-level 1, 256 CGUs, thin LTO, deps opt-level 3, system-malloc shim (:126-127). Metadata cache sizes use conservative reservations rather than allocation measurements (:129).
- Validation: 286 native + 52 Python tests; 200 upstream; 219 production (130 positive, 89 clean); 57 transitions (:133-136).

---

## gpu-producer-execution-contract.md (130 lines) -- "Batched producer execution contract"

Boundary: SQL owns decisions, joins, inference, diagnostics; structural producers operate on immutable packed data through an explicit batch boundary; applies to CPU backend and every future GPU backend (:3-5).

Required shape (D) (:9-14)
1. Collect document, field, owner, structural demands across ready files and clauses.
2. Prepare each exact document version once; freeze numeric columns and posting ranges.
3. Advance SQL consumers until their next structural dependency.
4. Gather all ready requests into one producer wave.
5. Count results, scan counts, scatter numeric output columns across the complete wave.
6. Publish only completed producer relations; resume dependent SQL consumers.

Rules (D) (:16-32)
- Vtab callbacks only read prepared data and CPU indexes; cannot dispatch producers, submit device commands, or wait for device completion (:16-17).
- Source text = immutable side array with checked byte ranges; paths and grammar names in host metadata, not repeated device records (:18-19).
- Node identities keep document version + file qualification; output order uses logical request identities, not physical execution order; every requested relation gets explicit completion or failure; failed producers expose no partial rows and cannot prove absence; one failed relation cannot spend another's allowance; suspension keeps work counters and cancellation state (:21-26).
- "Resident input and result capacity determine execution partitions. SQL insertion batches, publication pages, file boundaries, and exact signatures cannot determine dispatch boundaries." Publication consumes completed output without replaying semantic execution; all partitions share one evaluation stage allowance; capacity/stage exhaustion rejected explicitly (:28-32).

Enforcement (:36-46): with fixed dependencies and sufficient capacity, 100x more independent roots or files must keep the same producer stage count; thread/row counts scale with volume; genuinely dependent producer requires a subsequent wave; repeated SQL scans or insertion batch size changes cannot create more stages (:36-39). CPU stage traces are CPU passes, not GPU launches; a GPU backend must record kernel launches, command submissions, intermediate readbacks separately and preserve GPU-owned counts between dependent kernels; general recursive inference can require additional semantic waves (:42-46).

Provenance (:50-53): Landing NER-1562 at `ce7d2da32` is the execution-shape reference (broad classes, homogeneous signature blocks, compact frontiers, shared-command counts); its dynamic production integration incomplete; Rerail does not import Landing's rule engine or persisted tape ABI.

Current CPU implementation (:57-89)
- Workspace driver collects dirty relational work across policies, clauses, files; stateless collects all partitions in the policy (:57-58). SQL sessions retain connections, stage positions, work counters, diagnostic anchors across suspension (:59). File-local result caches keep exact document-version keys (:60). Workspace eligibility witnesses gate native syntax inputs (:61). Global clauses can keep selective projected evidence when a file gate can't express row selection (:62). Preparation extracts full declared inputs before consumers run (:63).
- **Storage**: "The syntax backing uses fixed-width node columns, field ranges, owner columns, and CSR postings. Names and source text remain host dictionaries and immutable source storage." Producer packs only columns required for subtree execution; shares document columns and visible-node selectors across ready relations; requests, counts, offsets, result columns are numeric identities (:65-69).
- CPU backend: one count pass, one segmented scan, one scatter pass per ready wave; completed output owns result columns until SQL consumes them; SQL insertion + index construction remain publication work; empty successful relation still gets a completion token; only success token advances dependent SQL (:71-75).
- Limits (D): producer arena 64 MiB packed input, 256 MiB result columns; batch permits 100 producer passes across dependent waves; driver admits suspended consumers by packed input capacity and a 256 MiB SQLite storage target (:77-79). Admission measures SQLite cache, schema, statement storage without scanning rows; measured at first producer suspension; one candidate can cross target while previous partition drains; single consumer can exceed; projected evidence, prepared syntax facts, publication indexes, document caches outside target; "not a process memory bound" (:81-86). Oversized relation fails explicitly without rejecting peers; partitions release packed input without resetting the 100-pass allowance (:87-88).

Required check (:93-101): `npm run test:producer-shape` (fails if profiling absent); covers 100x roots, files, policies, independent insertion sizes, dependent waves; dependent wave must reuse resident document input; descendant rows checked against SQL forest oracle; failed/interrupted outputs never enter file-result cache.

Recorded results (M) (:106-119)
- Production 5-file workload: 78,911 subtree requests across six relations in one three-pass wave; five syntax documents prepared once (:106-107).

| Median | Before (`c4bd76bf`) | Batched producers |
|---|---:|---:|
| Cold process CPU | 2.898 s | 2.553 s |
| Edit process CPU | 1.192 s | 1.036 s |
| Peak RSS | 572 MiB | 652 MiB |

- -11.9% cold CPU, -13.0% edit CPU; RSS +13.9% because driver retains more ready SQL sessions; does not isolate batching from packed-storage change (:118-120).
- Contradiction/inconsistency to note: the "before" here (`c4bd76bf`) is 2.898 s cold / 1.192 s edit, whereas shared-syntax-implementation.md reports the same commit's candidate at 3.192 s cold / 1.284 s edit (shared-syntax-implementation.md:97-98). Different runs/host load; not reconciled in the docs.
- Validation: 406 native + 52 Python tests; 200 corpus, 219 production, 57 transitions (:123-124). Source contract keeps file-local SQL semantics through separate sessions; producer execution coalesces ready requests (:126).
- "Numeric kind slots belong to each document's host dictionary. A future accelerator adapter must bind those slots to its verified grammar catalog during preparation. It cannot reinterpret them as another executor's persisted kind codes or tape ABI." (:128-130)

---

## reachability-spikes.md (254 lines) -- "Reachability adapters" (replay experiment, 2026-10-20 artifacts)

Contract (:16-29)
- Request relation: distinct `(path, root)` pairs; result: distinct `(path, root, node)` incl. root itself; diamonds produce one pair; cycles terminate; traversal passes through dangling endpoints; identical node IDs in different files separate (:16-20).
- Root producer belongs to rule authoring (quality linter uses conservative SQL producer; conditional linter has explicit root CTE); "not a general optimizer that infers roots from arbitrary SQL" (:22-24).
- Each immutable SQL scope assigns integer keys to `(path, node)`; adapters use these + same indexed edge relation; public `descendants` view maps back (:26-29).

Backends (:33-41): `baseline` (authored recursive SQL), `sql` (recursive UNION over root batches), `scalar` (Rust `is_reachable(graph, root, node)` with bounded memo), `vtab` (one `IN` scan per root batch), `vtab-direct` (consumers constrain directly), `interval` (SQL DFS intervals then batched pair materialization), `interval-direct`.

Data structures / constants (D)
- Batches of 256 integer roots default, configurable 1..4096; largest batch 32 KiB of integers (:43-45). SQLite retains shared pair relation with forward + reverse indexes; "Batching limits traversal requests. It does not bound the total size of a retained closure." (:46-47)
- Recursive SQL dedups `(root, node)` via `UNION`; recursive rows omit paths since integer keys identify file+node; roots enter via temp batch table, not expanded parameter list (:49-51).
- Rust adapters: one generic directed-graph impl, loaded once per SQL scope, forward + reverse traversal; memo <= 250,000 node memberships across roots; unknown node doesn't create empty cache entry; per-graph work budget + monotonic deadline; interrupted traversal publishes no partial memo (:53-59).
- Vtab: root equality, target equality, complete `IN` sets; rejects scans without root or target constraint; prefers consumer equality binding over broad request set; bound target + root set -> reverse traversal + root membership; both `IN` sets -> smaller seed set at execution; composite key `(graph, root, node)` via `WITHOUT ROWID` (:61-66).
- Interval: DFS intervals in SQL; checks distinct parent counts, then coverage; diamonds/cycles -> recursive fallback; verified forest uses `UNION ALL`; path strings + sorted enter/leave events, not source-byte containment (:69-73).

Harness boundaries (:77-105): replay of captured SQL inputs, excludes native extraction, compilation, scheduling, wire encoding; Python's SQLite (3.53.4) vs engine pinned 3.53.2 (:94-99). Rust bridge loads graph via JSON; production needs bounded graph-input batches + engine cancellation token/work counter (:101-103).

Results (M; median CPU of two fresh-DB replays; 5-file capture) (:117-125)

| Adapter | Statistics | CPU s | Peak RSS MB |
|---|---|---:|---:|
| Original SQL | Sampled | 9.25 | 957 |
| Batched recursive SQL | Sampled | 7.22 | 639 |
| Batched SQL, ordered pair inserts | Sampled | 7.18 | 637 |
| Batched Rust vtab | Sampled | 6.75 | 669 |
| Batched recursive SQL | Full | 8.01 | 639 |
| Direct Rust vtab | Full | 7.53 | 649 |
| Materialized SQL intervals (1 run) | Sampled | 8.18 | 657 |

- Batched SQL removes ~22% replay CPU and 318 MB RSS vs original; vtab modest further reduction, not a stable 6% (:127-128). Sorting inserts no material change (:129). Full statistics repair direct vtab but add work to SQL control (:130). Shared key, edge, request preparation ~0.9 CPU s on largest file in both adapters (:131).
- Scalar probe (M): 20,000 candidate pairs, 128 function roots, largest file; SQL closure + first probe ~21 ms after common prep; Rust retrieval + serialization + graph construction + first probe ~76 ms; warm batches ~11-12 ms SQL vs 8 ms Rust scalar (:134-138). Bulk scalar enumeration would need 2,737,798,504 candidate pairs on the largest file (:140). Scalar rejects enumeration above 5 million candidate pairs by default (D) (:248).
- Direct interval view exhausts 200-million-step SQL budget; full statistics move the failure from `method_nodes` to `annotation_atoms`: plan checks identifier candidates against interval membership instead of enumerating each small annotation subtree first (:142-145). Does not measure extractor-supplied intervals (:146).
- Samples: 2,490 active stacks, 2,469 in SQLite (VM, record compare, B-tree, varint decode) (:148-150).
- Parity: 7 adapters pass 1,400 complete-row comparisons; largest file SQL + Rust match 421,698 requested pairs against original 631,463-pair closure (:156-159).

Engine integration decisions (D) (:164-176): keep batched recursive SQL default; keep root request relation separate from traversal and from public descendants relation (allows vtab without detector SQL change); adapter loads requests in bounded batches, retains completed pairs, shares across consumers in the same immutable scope; explicit integer types on internal tables; both traversal directions in result indexes; "Treat statistics as part of the physical plan"; selective statistics policy preferred over unconditional full ANALYZE; Rust extension gain doesn't justify default.

Regressions found (SQL-shape lessons) (:180-200)
- `CREATE TABLE AS` for a computed row number gave the column no integer affinity while pair table declared `INTEGER` keys; mapping join scanned targets per method; full `ANALYZE` didn't fix; explicit `INTEGER` declaration restored indexed access (:180-184).
- First direct vtab plan treated root batch like single root and walked every root when consumer already bound target; corrected to distinguish batch scans and use reverse traversal (:187-190).
- Sampled stats gave direct view a poor join order in `loop_names` (enumerated roots before selective loop consumer); full ANALYZE scanned loop candidates first; failed run ~711,000 Rust work units before 200M VM steps exhausted (:192-196).
- Repository scopes exposed repeated textual IDs across files; key relation now includes source path before assigning global integer IDs (:198-200).

---

## data-layer-engine-gaps.md (159 lines) -- feature ledger

Not about storage/perf mostly. Facts:
- Every new Linear issue requires Jeremy's approval (:3-5). Oracle revision `2b1504ad...` (:9).
- Retained feature issues (implemented): NER-1969 line queries/filters/first matches/paths/same-file gates/line terminators; NER-1970 whole-file valid/invalid syntax gates; NER-1971 current/previous line predicates, bounded context (:18-22). Withdrawn NER-1968, 1972-1977 (:34-42).
- String parity defects: adjacent literal concatenation and decoded-newline second diagnostic line; outside passing 317-call corpus (:46-61).
- Batch two: per-file content gates, bounded line state, indentation filters, LF-only scanning, bounded context windows (:78-83).
- Batch three: all ten shapes use existing capabilities (AST relations, captures, node identity, configurable name lists, decoded values, per-binding reporting); candidate defects: attribute-store mismatch, excessive recursive descendant scans (:95-99).
- Batch four: 514 calls -> 573 comparisons with reporter (:105-106). New capabilities: authored evidence closure (global names, staged inheritance/aliases, class-local cyclic helper graphs); formatted-string parsing with interpolation marker; decoded identifier-word parsing (:112-114). Native tests cover incremental invalidation, multiplicity, cache isolation (:116). Excessive work-budget polling exposed and fixed (:117).
- Aggregate reporter: escape-hatch reporter reports all findings only when total > 16; engine supports one `define.report_when.count.gt` gate per document counting combined reported findings; "Edits across the ceiling replace complete results, including findings from unchanged and evicted files." (:128-135) -- an invalidation case: a document-level count gate makes results non-local.
- Batch five: declared-reference graphs (sibling links, constructor field conventions, imports, annotations, inheritance, callbacks); relational clauses join projected source facts, group exported families, recursive reachability with native reference edges (:147-148). Native source corpus 312 comparisons; repository gate 23,835 findings (M) (:152). "The complete candidate set remains slower than the Python oracle in the measured sample. Repeated fact extraction and global query rebuilding remain performance work." (:155-156)

---

## data-layer-rule-port.md (292 lines) -- "Data layer rule migration assessment" (pre-port survey)

- Revisions: data-layer main `2b1504ad...` (oracle, retrieved Oct 19 2026); PR 2539 `281f500d...` (historical declarative impl, open, unsuitable as spec); Rerail `0ab3cedc` (:19-28).
- Inventory (:32-41): current main 142 declared checks (141 enabled; 115 hard-fail, 16 ratchet, 4 info, 7 metric) vs PR 2539 104; 99 common, 43 additions, 5 removals.
- Main retains a fact-first framework: versioned extractors, shared traversal for AST collectors, source triggers, caches, project-derived facts, symbol graph, storage providers; default fact provider is filesystem JSON; optional raw SQLite provider stores facts but suite still runs Python reporters (:54-57). Main lacks PR 2539's `lint_specs/` YAML catalog and `fact_query/` compiler; extraction errors become `FACT-EXTRACTION-FAILED` findings (failure visibility must be preserved) (:59-62).
- Capability table (:73-83): general relation programs absent ("No general DSL for named intermediate relations, arbitrary joins, recursive relations, derived columns, and conditional aggregation"); public API / inherited signatures need export identities, re-export closure, inherited ownership, normalized signatures; signed exceptions/ratchets need invalidation keys; ordered effects: "Reachability alone does not establish value flow or transaction ownership."
- General relations (:87-99): evidence projections expose captures and file identity fields; row predicates `all`, `any`, `not`, `eq`, `ne`; `define` shares AST utilities, not a catalog of materialized relations. PR 2539 language had joins, left joins, unions, recursion, derived expressions, classifiers, aggregates. Broadest extension: "a typed relation interface over shared summaries ... definitions, imports, exports, signatures, and policy entries without exposing parser storage"; "New relations need declared dependencies, bounded work, incomplete-result semantics, and precise invalidation." (:96-99)
- Symbols/value flow (:107-120): call graph models class bases and method receivers but no general public-symbol relation; Python call summary treats ordinary parameter receiver as open (not annotation-inferred); dynamic expressions need explicit unknown outcomes rather than silent exclusions.
- Historical SQL costs (PR 2539 200-file plan; not fresh) (M, historical) (:165-176): extraction complete 4.907 s / 5.300 s (8 workers); refresh 8.730 / 8.712 s; SQL/report 47.757 / 36.627 s; large save batch 130,663 writes 3.112 / 2.555 s. Plan hypothesized excessive fact volume + repeated broad SQL work; later commits narrow fields and physicalize relation rows (:174-175). Lesson: "demand-driven extraction with shared summaries and early candidate reduction ... Generic facts need not mean every AST field, text fragment, and child relation for every selected rule. Moving Python decisions into SQL does not automatically reduce total work." (:178-180)
- Current Rerail costs (source review, unmeasured) (:188-208): (1) each eligible clause performs its own matching, broad candidate kinds and repeated descendant queries can dominate; (2) "A dirty global check collects witnesses across its partitions. Relation evaluation projects those witnesses again. A small edit can therefore cause repository-wide row work for that check." (:193-195); (3) `require_each` builds hash buckets from equality predicates; without extracted equality, scans candidates (~subject x evidence); hashed path rebuilds buckets every evaluation (:196-199); (4) call adapter tracks graph fingerprints, affected consumers, result stamps, avoids full rebuild per edit; high-fanout changes still affect many consumers (:200-202); (5) process-wide `cpu_slot`, `WORKSPACE_BUSY`, four-thread file worker pool (:203-205); (6) witness projection, serialization, stored-result retrieval, persistence (:206-208). Sources: `workspace/global.rs`, `query/crossfile.rs`, `workspace/evaluate.rs`, `workspace/api.rs`, `lib.rs` (:210-214).
- Required benchmark matrix (D) (:229-238): cold empty store; new workspace populated store; resident unchanged apply; one local function edit; signature/import/export/base-class edit (downstream invalidation); deletion/path change (stale fact removal); complete catalog; many findings/incomplete files. "A faster run with more coverage failures is not a performance improvement." (:217)

---

## data-layer-port-execution-plan.md (206 lines) -- first-batch execution report

- Ten checks, 41 diagnostic clauses (:24-38); native replay 317 calls (234 positive, 83 clean), 297 expected findings (:44-45); scorer preserves duplicates and compares check, code, path, line (+column for backend strings) (:49-50). Empty detector rejected with 297 FN; always-reporting with 1284 FP (:53-54).
- Engine capabilities added: regex lines per Python `str.splitlines()`, line/match filters, first-match; whole-file valid/invalid syntax gates (works in cross-file selection and incremental repair); current/previous line layout predicates (:93-100).
- Measured speed (M, single observations, shared machine; 9,010 Python files, 137,896,473 bytes, four workers; oracle cache disabled; Rerail empty resident workspace, no store) (:109-123):

| Measurement | Time |
|---|---:|
| Python shared fact engine, cold | 9.36 s |
| Native workspace, cold | 11.04 s |
| Native compilation | 53.2 ms |
| Native unchanged-file resend | 3.2 ms |
| Native one-file whitespace edit | 15.9 ms |

- Worker phase aggregates (M): parsing 9.95 s, regex scanning 4.99 s, tape encoding 3.22 s, AST matching 0.20 s (:132-133). Targets: shared text scans, parsing and tape costs, dispatch overhead; "A shared split-line index could avoid rebuilding Unicode line boundaries for every regex clause." (:134-135); more AST matcher shortcuts not supported by profile (:136).
- Note: for batch one (non-relational, regex/AST rules), per-edit cost is 15.9 ms (M), vs. seconds for the relational quality clauses elsewhere.
- Later continuation header: batch six -> 60 candidates, 1,259 source comparisons, 40 edit transitions pass (:1-2).

---

## sql-materialization-shape.md (282 lines) -- "SQL materialization ownership" (status: target design before implementation, :3)

Invariant (D) (:12-29)
- "The workspace owns a canonical producer graph across the complete scheduled rule evaluation. SQL statements and result packets consume that graph. They do not define its sharing boundaries." (:12-14)
- One owner produces each required version and coverage partition exactly once; every consumer reads that owner's rows; no consumer embeds another materialized copy or reconstructs dependencies; "Reading a relation several times is valid. Reconstructing its unchanged covered rows is not." (:16-19)
- Renaming rule/clause/alias/connection/transport batch does not create a new producer; semantic scope + data changes create a new producer version; exactly-once applies to successful publication (cancellation can require retry) (:22-24).
- Materialization selective: existing indexes answer covered paths; cheap single-use expressions stay pipelines; every materialization has named owner, consumers, coverage unit, retirement point (:26-29).

Evidence (M, static SQL placements from captures) (:37-50)

| Workload | Materialized CTE placements | Definitions in multiple statements | Extra placements across statements |
|---|---:|---:|---:|
| State | 10,320 | 248 | 1,672 |
| Ten rules | 19,001 | 420 | 2,745 |

- One demanded-span definition appears in 61 state statements / 90 ten-rule statements; a window frontier 59/60; an empty-set definition 670/1,168 (:42-44). Placements, not CPU (:45-46).
- Result execution allocation reaches 68.5 MiB (state) / 98.6 MiB (ten rules); first state packet contains 1,101 inline materializations; scheduler controls explicit SQL tables only, can't release private intermediates (:49-51).
- Current boundaries (:55-61): rendering embeds live inline definitions into multiple statements; each property fill can reconstruct demand subqueries; each result packet can own another inline working set; **each clause session opens a separate in-memory SQLite database**; local release knows only local consumers.

Target shape (D) (:65-93): all scheduled consumers -> canonical producer + demand graph -> union ready demand by producer version -> build missing coverage once in dependency waves -> workspace SQL relations + existing indexes -> consumers read -> retire after every planned consumer. Conditional consumers remain potential readers until gates resolve; skipped consumers force nothing (:80-81). Demand discovery can depend on earlier results so one producer can get several disjoint missing-key batches; each key one successful materialization; must not force future demand into an earlier wave or create cycles (:86-89). Native producers keep broad count/scan/scatter passes; "does not create one launch or SQL query per rule, node, or lookup" (:91-93).

Identity, scope, coverage (:97-128)
- Identity = versioned contract name + input bindings + typed options + semantic domain; schema identity includes affinities, NULL behavior, multiplicity, ordering; "Negation includes its universe. Correlated producers include their bound key domain." Source/scope filters are semantic inputs; effective execution context (limits, callbacks) affects compatibility (:97-102). Matcher exactness is consumer metadata, can't become stronger through reuse (:105).
- Coverage forms (:113-117): independent input keys (node properties, spans, child fields, pure predicate facts: compute missing keys incl. empty); complete partitions (file-local derived sets, reachability for a requested root: each versioned partition once); complete domains (cross-file aggregates, scoped negation, global choices, non-distributive windows: seal exact input domain before publication).
- "File-local versions include a file identity and generation that survives integer-ID reuse safely. Transport partitions do not enter file-local identity. Domain-dependent versions include actual membership and relevant input generations. A request-table name or path-filter spelling alone does not identify that domain. Vocabulary and semantic-reference dependencies retain their own version stamps." (:119-123)
- Completion metadata distinguishes empty from missing; records covered inputs; "Non-distributive results cannot accept appended keys without a proven incremental algorithm. They require a new domain version or completion of the original sealed domain." (:125-128)

Materialization choices (D) (:132-140): kind/value/vocabulary access -> direct indexed SQL, no expanded posting cache; literal empty -> constant propagation; alias/cheap projection -> binding/pipeline; nontrivial multi-consumer -> one owned SQL materialization; demand fence / algorithm boundary / measured working-set cut -> explicit owner; cheap single consumer -> inline pipeline; recursive algorithm -> one producer owns recursive component + private work tables.
- "A single textual reference inside a correlated loop does not prove single execution." (:143). Admission compares avoided computation with complete storage cost (write, indexes, maintenance, retention, reads); "Expense alone does not justify storing a single-use result." (:147-150)
- Renderer substitutes storage reference for every owned dependency; cannot embed `AS MATERIALIZED` definition in a consumer (:153-155). Final single-use pipeline streams into compact candidate sink (:162-163). Constant propagation preserves bag semantics: `empty UNION bag` still requires dedup; `(producer,fid,pre)` uniqueness only for proven set relations (:167-170).

SQL storage and publication (D) (:174-205)
- Workspace SQLite DB owns reusable facts + selected shared materializations; typed SQL backing relations with consumer-selected indexes; "does not require a universal row pool or JSON payloads"; existing decoded-property tables and compact forest intervals authoritative; expanded root-descendant pairs not required to share forest producer (:174-178).
- Selector relations store keys + necessary computed columns, don't copy fact columns per rule (:180-181). Missing coverage uses batched writes against owner's backing; each index receives rows once (:186-187). Planner collects index access paths across consumers before publication (:189-190).
- Shared backing in workspace DB not connection-local temp; clause connections consume scoped views without copying; one writer + dependency-wave barriers; readers acquire snapshots after waves; generations pinned; close incremental BLOB readers before incompatible writes (:192-197). Reader counts, page caches, snapshot leases contribute to admission and WAL limits (:199). Physical accounting charges shared work once (:204).

Lifetimes (:209-229): retirement after last consumer in complete graph; "Live materializations cannot be evicted and reconstructed to satisfy a RAM limit." SQLite spills or scheduler defers admission (:212-213). "Stable base facts retain their existing workspace invalidation rules. Evaluation-derived rows remain available until their complete consumer set finishes. ... The exactly-once guarantee covers one evaluation and its dependency waves, not every future workspace request or process restart." (:220-224). Dispatch rows repeating a candidate under different rule IDs are outputs, not materializations (:226-227).

Implementation sequence + integration map (:233-255): `graph.rs`/`lower.rs` keep canonical producer refs, coverage classes, consumers, placement; `lower/render.rs` storage refs; `decode.rs` ready demand frontiers; `lower/lifetime.rs` explicit graph edges; `corpus.rs` ownership independent of plan-pointer and packet identities; `relational.rs`, `relational/batch.rs` replace fact imports with scoped bindings; `workspace/global.rs` plan cross-rule consumers + barriers. Enforcement: reject duplicate owners, reads before completion, retirement with pending consumers; "Repeated writes count as attempted writes, even when `INSERT OR IGNORE` hides them." (:259-263). TDD ladder (:267-276) incl. different scopes, generations, types, regex flags, bag semantics, window domains cannot alias; literal empty folding preserves `UNION`, `EXCEPT`, bag.

---

## sql-preparation-architecture.md (297 lines) -- "Incremental SQL preparation for candidate selection"

Scope: the candidate-index subsystem in `native/src/code_rule_dsl/matcher/anchors/sql/` (anchor/candidate selection for the matcher), NOT the authored relational executor, which has different input, capture, budget, and connection lifetimes (:56-59).

Status (:7-13): implemented: shared structural preparation, duplicate-expression removal, fixed request tables, incremental vocabulary matches. **Rejected** (isolated experiments, increase measured CPU): expanded posting sets and generic shared utility sets; their catalog, retirement, retention limits are not production features.

Decision (:17-28): keep reusable candidate relations in the workspace SQLite DB; maintain them in a batched preparation phase; run candidate selection against prepared relations without cache fills or schema changes; request scope and cheap single-consumer intermediates temporary. Rust coordinates dependencies, supplies batched inputs, decodes packed facts via SQLite extensions. DB temporary at workspace level: retained relations survive rule batches and workspace edits but disappear at workspace closure; durable reuse across restarts needs separate design (:26-28).

Evidence (M) (:32-49)

| Workload | Before value access + shared properties | Current |
|---|---:|---:|
| Two rules, twelve files, 61 findings | 1.874 s | 2.504 s |
| Quality rule, five files | 3.004 s | 3.192 s |

- Positive diagnostic executes 550 SQL data statements vs 72 previously; BLOB reads 2,791,126 -> 51,444, but VM steps 111,766,482 -> 130,976,638 (:39-41). I.e. value access + shared properties REGRESSED cold CPU (+33.6% and +6.3%, A mine). Positive diagnostic: 1.299 CPU s materialization, 0.203 s index construction; largest structural fill ~0.390 s (overlapping) (:45-47).
- Current corpus retains base facts + final candidate roots; unchanged file with same plan does not repeat candidate selection; "Intermediate sharing ends at the compiler batch boundary, which permits repeated work across plans and batches." (:51-53)

Execution boundary (:65-83): publish changed files + plan definitions -> prepare missing shared facts in batches -> publish -> read prepared relations -> pack candidate roots -> existing matcher + capture evaluation. Publication retains packed tapes and existing FTS and value indexes (:75). Already complete request bypasses prep and selection; new plan reuses deps; "A file edit replaces that file's facts and invalidates dependent preparation." (:81-83)

Relation lifetimes table (:87-99) -- the fact store of the candidate subsystem:
- Packed tape, positional FTS, value segments, coverage: workspace storage, replaced for changed files.
- Decoded structural properties: retained per file revision, populated for the union of demanded nodes.
- Decoded spans and field-child results: per file revision, sparse unless demand proves full coverage.
- Expanded kind/value hits: only for selected shared paths with measured benefit.
- Regex vocabulary matches: by predicate + language, maintained from vocabulary changes.
- Literal vocabulary matches: direct indexed seeks.
- Shared file-local utility results: by logical relation identity + file revision.
- Pattern anchor choice: request-dependent unless scope/statistics deps explicit.
- File and plan request keys: reusable scratch schema with batched request rows.
- Cheap single-consumer joins: inline / scratch.
- Final candidate roots: existing compact cache, plus logical plan identity.
- Admission: retain decoded facts + multi-consumer relations across the complete requested plan graph; count consumers before splitting into batches; posting expansion needs its own evidence "because retained ordinals can duplicate a compact index many times" (:101-104). No blanket retention; keep dependency boundaries preventing exponential CTE expansion (:106-108).

Logical graph and identities (:112-145)
- Typed relation graph before SQL text: operation, child identities, parameters, column types, dependency class, consumers; aliases and parameter positions excluded (:112-114). Canonical definition includes graph/compiler versions, language + grammar (kind/field mappings), operator, constants, regex semantics, direction, children, NULL/duplicate/set semantics, input domain (:116-123). Hash for lookup, exact equality resolves collisions; children referenced by identity; versioned typed encoding, no per-node JSON; Arc addresses not identities (:125-128).
- Dependency classes (:134-139): File-local (relation identity, file identity, file revision); Vocabulary (predicate, language, vocabulary epoch); Request-scoped (exact domain/request, temporary); Repository-wide (explicit repository epoch, excluded from initial retention).
- "File-local classification requires proof that file restriction commutes with the operation. Negation retains its universe." (:141-142). Pattern anchor selection uses posting counts across the batch -> request-scoped (:143).

Storage contract (D) (:149-188) -- key schema detail
- Monotonically increasing file revision in publication metadata and SQL file row; prevents stale reuse if SQLite reuses a deleted integer file ID; `(file id, revision)` unique; grammar changes require new revision (:149-152).
- Proposed tables (:156-163):
  - `prepared_relation`: relation ID, definition hash, exact definition, dependency class, schema version
  - `prepared_dependency`: parent/child relation IDs
  - `prepared_node`: PK `(relation_id,fid,pre)`, revision in payload
  - `prepared_ready`: PK `(relation_id,fid,revision)`, incl. complete empty sets
  - `predicate_term`: PK `(predicate_id,lang,term)` for regex vocabulary matches
  - `predicate_ready`: predicate, language, completed vocabulary epoch
- Node + completion tables reference current file revision with cascading deletion; ordinal follows file ID directly in key; one current revision per file; WAL snapshots preserve earlier readers (:165-168). Completion markers store a dependency stamp for dynamic inputs outside the file revision, e.g. corpus-dependent coverage changes invalidate value relations and final roots; invalidation removes rows + markers in one batched transaction before selection; "Unchanged source bytes alone do not prove validity for these relations." (:169-172). All ordinal/revision columns explicit INTEGER affinity (:173). Spans, children, structural properties stay in existing typed tables (:176).
- Shared tables + indexes created once per schema version; preparation uses batched INSERT, DELETE, joins instead of CTAS + index build per retained relation; file change deletes via revision key; definition change creates new identity, retires unreachable ones (:179-182). Over-limit retention falls back to bounded scratch; "Selection never repairs an evicted relation." (:184-188)

Preparation protocol (:192-223): collect all plans + files before the existing **32-plan batching boundary**; intern + count consumers; classify; insert demand keys in bounded batches; join against completion markers for missing work; execute in topological frontiers; write completion markers in same transaction; publish and select (:192-199). "No host loop issues a SELECT for each node, file, predicate, or rule." (:201). Structural demand = union of missing node keys; a live all-node demand covers subset demands; span + field-child coverage separate from structural; known missing child = complete NULL (:206-210). Regex maintenance: active predicates vs newly introduced distinct terms in SQL batches; new predicates vs current vocabulary once; deleted terms remove matches; epochs advance after maintenance; coverage/residue confirmation mandatory for values outside indexed vocabulary (:212-217). Corpus mutex + one writer; cancellation rolls back active transaction keeping earlier complete frontiers; retry discovers missing work via markers (:219-222).

Read contract and invalidation table (:227-245)
- Selection does only SELECTs; integration test rejects selection DDL, DML, decode calls (:227-229). Scheduler freezes file revisions + plan definitions; mutex prevents concurrent publication (:233-235).

| Change | Required invalidation |
|---|---|
| File edit, deletion, language change | That file's facts, retained node sets, completion markers, final roots |
| New file | Missing prep for that file + affected vocabulary maintenance |
| Utility or rule change | Changed logical relations + dependents; unchanged deps reusable |
| New/deleted vocabulary term | Affected predicate matches + vocabulary epoch |
| File-filter change | Request scope; file-local retained facts reusable |
| Grammar/compiler/semantic-version change | Affected catalog identities + all incompatible derived rows |
| Corpus-dependent value coverage change | Affected value results, even for unchanged files |

- "Cache validity and admission are separate decisions. Only proven dependencies permit reuse. A measured cost determines whether retention pays for itself." (:247-248)

CPU expectations (:252-270): "SQL B-tree insertion can cost more than a one-use temporary table. Moving the same construction earlier does not establish an improvement." (:256-257). Targets (D): >= 30% less cold CPU on positive control (~1.753 s); quality >= 10% (~2.872 s); no regression >5% of another control (:266-270).
- Broad batches: typed columns + batch of demand keys per op; frontiers not nodes; "CPU SQL statements are not equivalent to GPU kernel launches." (:274-281)
- Implementation map: `sql/plans.rs`, `sql/lower.rs`, `sql/corpus.rs`, `sql/schema.sql`, `sql/store.rs`, `sql/decode.rs`, `sql/access.rs`, `sql/execute.rs`; proposed `graph.rs`, `prepare.rs`, `retained.rs` (:285-295).

---

## sql-execution-stages.md (244 lines) -- "Compiled execution stages" (status: target architecture, before implementation, :3)

Decision (D) (:11-31)
- Compiler builds a general dependency graph and specializes it into a stage plan per scheduled rule set; runtime executes it with a small frontier controller; no general task scheduler, arbitrary callbacks, work stealing, or task per row (:11-13). Specialization follows operation families, schemas, coverage rules, dependencies, never rule names (:15-16). SQLite keeps joins, filters, aggregation, windows, relational recursion (:17).
- "A fixed sequence of preparation, query, and output phases is insufficient. Later candidate results can discover new property demand or activate another clause. The compiled topology stays fixed, but demand and branch outcomes determine the executed frontiers." (:20-22)
- Planning vs runtime table (:24-31): dependencies (graph, read edges, ordering edges, branch gates / which gates resolve); sharing (bindings, one owner / missing coverage); execution (templates, schemas, indexes / batch sizes, keys, admission); lifetime (potential consumers / completion, leases, retirement); memory (placement, partition legality / spill, delay, failure); output (bindings, ordering, set/bag / rows).

Current components (:35-45): candidate lifetime scheduler (`matcher/anchors/sql/lower/lifetime.rs`) produces cached action list, infers deps from generated SQL, controls explicit tables, cannot own inline materializations or see later clause consumers. Clause batch executor (`relational/batch.rs`) gathers native demands into capacity-bounded waves, but each session advances through SQL stages until the next native demand; shared SQL work stays inside separate sessions (:39-41).

Planning representation (:49-77): producers, consumers, demands distinct from storage/transport. Producer declares contract ref + options + inputs; schema/multiplicity/ordering/completion; scope, context compatibility, generation deps; coverage form; representation (indexed access, pipeline, owned materialization); backend op, access paths, private workspace estimate, legal capacity partitions; potential consumers, gates, future demand sources (:54-62). Authored SQL opaque unless declared (:67-68). "Ordering edges also preserve mutable maintenance, callbacks, and required write ordering. ... An ordering edge alone does not keep an unused table alive." (:71-73). "Recursion stays inside one declared algorithm stage. The outer dependency graph is acyclic after recursive components become single producers. Undeclared cycles fail planning" (:75-77).

Compiled stage plan (:81-100): maximal legal pipelines; compact plan with indexed descriptors and adjacency arrays; runtime state = indexed counters, bitsets, cursors, frontier buffers; no heap object per row/key (:81-84). Ops: Discover (batched selector adds demand downstream), Build (missing coverage via SQL template or native op), Evaluate (ready relational pipeline), Emit (stream to output adapter) (:86-91). Index construction belongs to the owning Build (:94). Fusion where no sharing/fence/memory boundary (:95).

Frontier execution (:104-132): advance only affected successors; stable plan order for ties (:104-106). Per frontier: resolve gates + collect demand; union by producer version + coverage form; subtract completed + reserved via batched SQL; admit by retained bytes, reader cost, workspace estimate; execute + publish; advance consumers, close sources; retire (:110-116). "The controller never delays a producer until all future consumers become ready. Such a barrier can deadlock a consumer that discovers more demand for that producer." (:121-122). Native batches combine compatible ops across files and rules; SQL batches use demand relations and set operations, not parameterized point queries; "Batching does not require one giant SQL statement for unrelated algorithms." (:129-132)

Coverage (:136-162): pending/reserved/completed sets per producer version; writer owns reservations; rows, completion, indexes visible at one publication boundary; empty completes key (:136-139). Keyed coverage uses stable input identity + generation (:141). Cancellation rolls back unpublished rows, releases reservations; retry can't repeat successful publication (:146-148). Demand-source leases; static last-use for unconditional regions, lease counters for dynamic (:150-158). "This model avoids one runtime object per potential file-and-rule pair." (:160)

SQL ownership (:166-187): reusable rows in one file-backed workspace DB, one writer; clause connections keep private namespace + scratch; engine-created read-only bindings expose shared DB via scoped views; user SQL can't attach DBs or access internal catalog (:166-169). Publication waves end readers before incompatible writes; generations pinned (:171-174). Shared physical build preserving per-consumer logical accounting; "Budget semantics do not change implicitly through reuse." (:180-183)

Memory/output (:191-202): admission includes retained owners + next op's private peak; "SQLite spill settings alone do not establish a hard bound on those allocations." (:194); "It does not turn every CTE into a table." (:197)

GPU-compatible boundaries (D) (:206-214): frontier carries typed columns, offsets, stable IDs, validity bits, selection domains; native producers do broad count/scan/scatter; outer controller submits one op family over a broad compatible frontier; "cannot promise a fixed number of GPU launches across arbitrary dependency chains or recursive algorithms"; launch count, frontier width, transfer bytes, synchronization count are explicit metrics.

Acceptance (:218-244): steps 1-5 (typed graph; shared owners; unified demand frontier + coverage ledger; workspace writer + scoped readers; admission/retirement/output); TDD ladder of 8 cases. Profiler separates planning CPU, controller CPU, SQLite CPU, native CPU, publication cost (:242).

---

## sql-composition-implementation.md (177 lines)

Forest containment (implemented) (:9-35)
- Forest adapter validates selected edge graph, keeps one preorder position per normalized identity and one interval per requested root; public relation still `(__path, root, node)`; SQL range joins produce rows per consumer; no longer stores every root/descendant pair (enumeration still proportional to result) (:9-14).
- Intervals describe the selected graph incl. disconnected subtrees and dangling endpoints; "Raw CST intervals cannot replace this validation." Cycles/multiple parents error for declared forest; arbitrary graphs keep recursive SQL (:16-19).
- Root query can select `__path,id,expand`; false/NULL permits only reflexive pair; any duplicate permitting expansion expands; blocked root can be intermediate for another root; adds no extra dependency wave (:21-26). Four authored state clauses use it (:28).
- **Join-order lesson**: "The state loop predicate fixes the join order for two small, correlated subtree probes. `CROSS JOIN` keeps each demanded subtree before its identifier lookup. Without that order, SQLite can scan all identifiers before testing interval membership." (:32-34)

Typed syntax intervals (:39-47): syntax sources expose `subtree_end` (exclusive end of raw CST preorder interval); `id` column supports indexed lower/upper bounds; containment `child.id>=root.id AND child.id<root.subtree_end` within same file over visible rows; if excluded interior node disconnects graph, selected-edge reachability still needs forest adapter (:39-43). Syntax producer request record = file, node, expansion as three `u32` scalars; count/scan/scatter passes keep broad wave shape (:45-47).

Demand boundaries (:51-55): candidate compiler keeps a materialized span relation after the demanded-key join; costly source predicates consume that completed selection; "SQLite cannot flatten those predicates into a broader retained-property scan." Vocabulary and value postings answer covered values without source decoding.

Shared policy catalog (:59-77): candidate producers share one definition catalog across the complete policy frontier per grammar; previous 32-plan boundary no longer duplicates producers (:59-60). Physical cuts kept before shared expressions cause excessive preparation expansion; result queries <= 128 output arms (SQLite compound-query limit), share producer relations across packets (:64-67). SQL generation indexes dependencies once per definition; each statement visits only reachable definitions; temp decode definitions borrow catalog; physical cuts remove inline definition before later consumers render (prevents repeated full-catalog copies) (:70-74). Capture format `rerail.candidate-replay/v2` (:76).

Candidate relation lifetimes (:81-121): schedule per cached SQL template; physical table refs create producer deps; retained-property reads/writes add ordering deps; derived from generated SQL identifiers (:84-88). Prepare each producer before first consumer; construct each shared producer once; release temp tables + indexes after last consumer; ordering dep doesn't extend lifetime; iterative traversal (:91-95). Adjacent releases batched into one SQL call; after error release live tables (:99-100). Trace `anchor-working-set` every 128 schedule operations (:105-110).

Measurements (M) (:129-132): cold CPU medians -12.5% (state snapshot), -25.8% (smaller ten-rule snapshot); peak RSS +13.6% and +8.8%; small snapshots, not full repository.

File-scoped kind positions (:138-153)
- Kind selection seeks persistent `file_kind_counts` PK `(fid,code)`; each count row owns a delta-varint posting segment; generic `nera_postings` table expands that segment after file + kind selection; packed-record pass constructs positions alongside counts (:138-141). Sparse files don't need decoded property coverage (:142).
- Lesson: "The bare FTS index remains available. Its vocabulary instance table accepts term constraints but cannot seek document constraints. Joining requested files after that stream repeats corpus-wide work during ingestion. The repository checkpoint exposes this cost; tiny source controls do not." (:144-147)
- Store version 2 adds posting column; transactional backfill from packed records via one ordered file cursor; edits replace counts + postings together; removal cascades (:149-152).

Value lookup file fence (:157-165): exact value terms need file-scoped seeks before posting expansion; order terms, requested files, `vidx` with explicit `CROSS JOIN`; `(lang,term,fid,kind)` PK answers probes; missing-value fallback starts with requested files before `vcov`. Regression (M): 200 unrelated files with the same exact term: before fix 19 progress ticks instead of 1 (tick = 100 SQLite VM instructions).

Retained property read fences (:169-177): decoded property views start from demanded keys; universal node reads start from requested files; keyed node/span/child reads start from selected `(fid,pre)` relation; explicit `CROSS JOIN` prevents retained-table scan replacing probes. Regression (M): unrelated file with 2,000 statements: before fix 806 progress ticks instead of 6. "An isolated hand-written CTE does not reproduce the planner behavior of that schedule." (:177)

Store tables named here: `file_kind_counts(fid,code)` + posting segment, `nera_postings`, `vidx` with PK `(lang,term,fid,kind)`, `vcov` (value coverage), FTS vocabulary instance table.

---

## sql-composition-opportunities.md (179 lines)

- Sibling branches surveyed (:11-16): `Moti-Velz/sql-set-composition` `e1a46a28`; `jeremy/sql-capture-demand-spikes` `f0907707...`; `jbarnes/ner-1562-sql-rerailer` `9bc40b37`.
- Sibling numbers (M, other branches): generated B023 SQL over 138 files 125.553 ms kernel CPU vs older frame compiler 5,478.944 ms vs handwritten 63.349 ms; generated query 51.560 ms in preparation (:24-28). NER sibling: 164,480,024-byte query and > 2 GiB during preparation for `eager_resolve`; "Named CTEs alone do not establish bounded preparation cost." (:30-32)
- 1. Tree intervals: `relational/forest.rs` computed preorder ranges then expanded every root into pairs; proposed `(file, root, lo, hi, expand)`; requires proven tree with contiguous subtrees; excluded edges/nested stop conditions need proof; raw CST bounds alone insufficient (:36-48).
- 2. Push demand: separate semantic node sets from answer construction; kind/field/value predicates restrict keys before joins; semijoins for existence; capture text deferred (:56-59). `removed-calls` shape `call.function -> attribute.attribute -> identifier`: reference decodes all identifier text; candidate decodes only identifiers on demanded path; SQLite can evaluate native predicate before selective joins (control) (:61-66). Covers capture-free conjunction only; not ordered choice, negative captures, unknown results, reached failures (:68-70).
- 3. Share utilities: intern semantic definitions before SQL emission (params, capture scope, source version, outcome behavior); typed demand keys union; one recursive component per positive recursive closure (:74-77).
- 4. Broad terminal batches: immutable file and range descriptors, typed columns via broad passes per wave; no terminal calls back into rule evaluator (:90-93). Batch table decodes <= 65,536 rows per CPU chunk under one reader lock (D) (:97). GPU needs resident input, count/scan/scatter allocation, budgets, cancellation between waves; "GPU launch count must follow terminal dependencies, independently of files, roots, and CPU chunk count." (:102-103). NER sibling stores complete `PackedRow` vectors (:105).

Results (M; six rotated trials; kernel CPU) (:141-146)

| Opportunity | Kernel CPU | Decision |
|---|---|---|
| Tree containment | all-root closure 267.068 ms; demanded closure 73.356 ms; intervals 0.994 ms | Integrate only for proven contiguous forests |
| Selective demand | eager 6.120 ms; selective syntax 2.943 ms; materialized keys 2.267 ms | Explicit boundaries before costly predicates |
| Shared utilities, depth 8 | inline 162.835 ms; named CTEs 10.908 ms; physical cuts 5.353 ms | Extend sharing + physical cuts across policies |
| Broad terminal batches | scalars 24.780 ms; streaming 19.096 ms; copied columns 20.394 ms | Keep streaming on CPU; retain broad producer contract |

- Interval prototype uses numeric CST identities rather than state policy's projected string IDs; production needs typed identity handoff and selected-edge proof (:153-154). Batch prototype column buffers add ~3.3 MiB peak RSS over streaming; CPU rejects the copies; "Future GPU work needs shared producer columns and a separate transfer measurement." (:160-162)

---

## batched-sql-performance.md (130 lines) -- "Batched recursive SQL" (integrated)

- Reachability stage with explicit SQL producers for roots and edges: `{"name":"descendants","reachability":{"roots":"SELECT __path,id FROM requested_nodes","edges":"SELECT __path,src,dst FROM links"}}` (:35-43). Quality rules use it; conditional architecture rules keep repository-wide root producer; primary quality rules request a conservative set of roots from consumer families (:3-6).
- Engine normalizes `(path, identity)` keys once per stage; recursive `UNION` over batches of 256 integer roots; result keeps forward + reverse indexes across batches; consumers read `__path,root,node` via mapping view; "The compact key column has explicit `INTEGER` affinity so SQLite can use its lookup index." (:8-12). Null identities supply no root/edge; cycles terminate; converging paths -> one pair; "Batch size limits the traversal request, not the retained closure size." (:14-20). Logical relation separated from physical impl (:22-24). Phases `sql-reach-prepare`, `sql-reach-walk`, `sql-reach-index` (:32).

Remaining costs (M, five-file traced cold run with plan logging + sampling; leaf spans) (:50-58): ordinary SQL table materialization 4.35 s; ordinary SQL indexes 1.90 s; fact extraction 2.17 s; reachability key/edge/root prep 1.36 s; recursive batches 0.96 s; reverse index + cleanup 0.18 s; fact insertion 0.50 s.
1. Common producers repeat across scopes: across 3 quality groups + conditional scope, `nodes` and `ownership` each materialize 14 times, 1.17 and 1.25 CPU s before indexes; largest file each quality group builds 67,774 node rows and repeats ownership walk (:62-66).
2. Facts wide: three quality node tables copy 29.2 MB of node text for 1.03 MB of source (logical, not physical); derived relations copy `n.*`/`f.*` again (:67-70).
3. Graph prep exceeds traversal: sorting/dedup roots, key assignment, edge conversion 1.36 s; largest file 0.82 s prep vs 0.56 s traversal over 158 batches (:71-73).
4. Candidate selection ~1.3 CPU s in initial anchor selection; samples `FileAnchorIndex::select_complete`, `combine`, sorting, budget polling; witness capture maps, allocations, SQLite parameter binding; fact insertion ~0.50 s (:74-78).
- A diagnostic aggregating all descendants exceeded the SQL work budget (:85).

Native comparison (M; medians of two fresh processes, tracing off; SQLite 3.53.2, opt-level 1, thin LTO, 256 CGU) (:98-103): Previous SQL 15.76 s cold CPU / 1.36 GB RSS; batched recursive 13.60 s / 1.07 GB: ~-14% CPU, ~-22% RSS. Validation 200 upstream, 219 production, 57 transitions, 277 native tests (:110-112).

---

## shared-sql-performance.md (173 lines) -- "Performance after shared SQL grouping" (profiling-only)

Runs (M) (:15-19): tracing+sampling off: cold wall 14.810 s, cold CPU 14.358 s, edited-file wall 3.778 s; attributed tracing: 15.267 / 14.701 / 4.437 s; external sampling: 19.702 / 14.740 / 5.972 s. Unsampled CPU matches previous 14.370 s (:21).

Cost breakdown (M; disjoint leaf spans of 14.701 s) (:34-41)

| Work | CPU | Share |
|---|---:|---:|
| SQL table materialization | 6.578 s | 44.7% |
| SQL index construction | 3.021 s | 20.6% |
| AST input extraction | 1.778 s | 12.1% |
| SQLite input insertion | 0.446 s | 3.0% |
| SQL analysis, validation, results | 0.192 s | 1.3% |
| Outside leaf spans | 2.685 s | 18.3% |

1. Descendant closure largest cost (M) (:55-71): materialization + indexes for `descendants` 3.977 s = 27.1% cold CPU. File-local quality-contract closure 1,095,879 rows for 113,953 syntax nodes; roots include every syntax node. Largest file `commit_branch_steps.py`: 631,463 closure rows for 67,774 nodes; recursive `UNION` all root/node pairs + two index permutations; query plans confirm indexed edge lookup, not a full scan (:59-61). "Stage materialization is eager. A later SELECT cannot push root filters into a temporary table that an earlier statement already constructed. The SQL construction queries must receive those restrictions explicitly." (:63-65). Path, root, node strings total 113,048,228 logical bytes across five closures; repeated paths 61,475,460 bytes = 54.4%; largest file 66,697,780 key-text bytes; both indexes contain these keys (:67-70). Proposals: derive root sets from consumers; avoid repeating constant file path in file-local graph keys; compact node IDs vs `kind:start:end` strings; `UNION ALL` after proving single-parent, acyclic, unique-node (:75-78). Consumers request argument, annotation, binding, loop, block roots, so function-only roots would change behavior (:80-81).
   - Note: node identity at this stage was textual `kind:start:end` strings (:77); later replaced by integer parser ordinals (shared-syntax-implementation.md:28).
2. Common syntax + ownership repeat (M) (:87-98): `nodes` construction + indexes 1.675 s; ownership 1.528 s; together 21.8%. Three source groups independent SQL scopes; in the largest file all three produce 67,774 rows and run the same ownership query; "Each ownership materialization executes exactly 5,299,816 VM instructions." Secondary groups ~2.030 s exposed to sharing. Primary syntax text 9,842,329 logical bytes before downstream copies; doesn't explain 1.335 GB peak RSS by itself (:106-109).
3. Input extraction (M) (:113-125): 355 extractions -> 339,551 input rows; 1.778 s; typed selectors and field projections execute separately; catch-all syntax selector probes many optional fields. `TapeNode::child_by_field_id` searches a node's children for each field lookup -> repeated navigation. Workspace file-local scheduling probes stop after one witness and clear projections.
- Smaller (:129-137): input insertion one execute per row in a transaction with prepared statement, ~3%; parser timer 0.074 s in cold pass -> "A parser replacement is not supported by these measurements." File-local relational execution serial; reduce repeated construction before file concurrency.

---

## content-store-bulk-publication-design.md (212 lines) -- content store publication (workspace content store, not rule facts)

Required properties (D) (:10-19): one SQLite connection per worker thread; WAL read transactions overlap a write; each lookup one bulk query with CTEs + joins; source + tape state from one joined lookup; hash-prefix indexes contain only algorithm + first signed hash lane, SQL compares remaining lanes; Rust completes compression + serialization before write transaction; stale read -> committed correction pass; <= 10 data statements per reconciliation pass; completed-revision marker is the visibility boundary.
- `ContentStore` stores DB path, compression settings, object caches, no connection; thread-local map owns one connection per DB path per thread (maintenance thread, Rayon workers, N-API task threads) (:23-28).
- Hash identity = algorithm + four signed 64-bit lanes; index `source_hash_prefix ON source_objects(hash_algorithm_id, hash_lane0)`; join checks lanes 1-3 (:35-54). Writer serializes with `BEGIN IMMEDIATE`, checks complete identity before insert; no full-width uniqueness index needed (:56-58).
- Publication <= 64 files (D) (:62); one immutable input model: paths + path hashes, canonical source hashes + Git aliases, compressed source frames, tape grammar identities + hashes + compressed frames, candidate catalog + check identities, source and candidate anchor facts, compressed local-result frames, expected overlay + relationship rows (:65-74). JSON parameters for variable-size integer/text identities; bound BLOBs for compressed frames; no JSON/base64 of BLOBs (:76-78).
- Read phase: short deferred transaction with one bulk lookup returning source, alias, tape, path, catalog, check, overlay, candidate, result state (:82-94). Preparation without transaction: identity validation, hashing, Zstandard compression, result-frame serialization, JSON encoding (:101-109).
- Write: `BEGIN IMMEDIATE`; each statement consumes complete prepared input with joins; no per-row ID fetch (:116-118). Object pass: hash identities, strings, paths, candidate catalogs, candidate checks, sources, Git aliases, tapes, source anchor facts (:122-130). Relationship pass: workspace overlays, candidate anchor facts, replace candidate-completion markers, replace candidate file relationships, local results, completed-revision marker (:136-141).
- Stale snapshots: never trust integer IDs from read phase; join by complete identity; commit valid rows, skip unavailable deps, reread, correction pass; monotonic progress; bounded busy backoff (:148-165). Crash recovery: next process publishes only missing rows; marker written only after final lookup proves completion (:169-178). Statement counter fails pass > 10 data statements (:182-184).
- Baseline measurements: "not part of this repository" (:212) -- no numbers.
- Relevance: tapes, source anchor facts, candidate anchor facts, candidate-completion markers, local results are all stored in the content store keyed by content hash identity (:65-74, :122-141).

---

## code-rule-profiling.md (369 lines) -- profiling harness + original investigation

Harness facts
- Deadline default 110 s, max 115 s; 3 s reserved for termination (D) (:67-68). Profile artifacts: `summary.txt`, `profile.json`, `parity.json`, `trace.jsonl`, `trace.chrome.json`, `sample-*.txt` (:99-108). Phases: compilation, cold, unchanged replay, edit (:110).
- Memory census counts shared projection values once across file results and evidence partitions (:148-149). Tracing records the complete projected input queue before SQL admission (rows, unique projections, JSON fields, referenced syntax objects, payload sizes) and AST field lookups, child examinations, failed lookups, max scan length (:155-161).
- `--batch-files` ingestion request size default 32 files, 1 MiB source ceiling (D) (:165-166).
- SQLite input insertion batches up to 32 rows (fixed); respects SQLite parameter limit and 1 MiB payload target; larger row own statement; prepared statements reused within transaction; each row keeps budget + cancellation checks (D) (:202-209). (Contrast: shared-sql-performance.md:129 says "one execute per row" at that earlier time.)
- Materialization spans include SQLite statement counters: VM instructions, full scans, sort operations, automatic-index rows (:215). File spans record final-result cache hits/misses (:216).

Original measured results (M; thin LTO, 256 CGU, system-malloc shim; individual runs; 18 logical CPUs, load above CPU count) (:241-245)

| Workload | Files / bytes | Compile | Cold | Unchanged | Edit | Peak RSS |
|---|---|---|---|---|---|---|
| Dispatch, traced | 1 / 136,810 | 54 ms | 1.218 s | 1.206 ms | 1.488 s | 411 MB |
| State, traced + sampled | 7 / 1,123,712 | 366 ms | 13.638 s | 9.998 ms | 1.334 s | 1,067 MB |
| Quality, traced + sampled | 5 / 1,026,452 | 723 ms | 48.839 s | 8.272 ms | 11.119 s | 1,350 MB |

- Quality later trace-disabled cold run took 88.599 s (load) (:249). State pair 13.133 CPU s with spans vs 12.824 without (:251).
- Quality cold breakdown (worker wall) (:258-264): SQL table materialization 25.769 s (52.8%); SQL index construction 12.392 s (25.4%); AST input extraction 5.137 s (10.5%); SQLite input insertion 1.334 s (2.7%); stats/validation/results 0.535 s (1.1%).

Causes (:271-354)
1. Repeated materialization: 890 materialized stages, 1,362 index builds, 1,155 AST input extractions; `descendants` alone 17.380 s materialize + 8.338 s index = 52.7% of cold elapsed (:273-275). For `commit_branch_steps.py`, six clauses executed the same recursive `descendants` query, each exactly 23,860,081 VM instructions; materialization 9.695 s across the six (:277-278). Original generator split detector groups to satisfy 32-stage and SQL-size bounds, repeating prerequisites in separate scopes (:281-282).
   - "Each relational evaluation creates a fresh SQLite connection and rebuilds its inputs, stages, and indexes." (:285). "The final-result cache is keyed by clause and file version. It does not share common relation prefixes between clauses or rules. The parsed-tape slot avoids repeated tree restoration within a clause, but does not share projected input relations between clauses." (:287-289)
2. All-root transitive closure: seeds from every syntax node; recursive `UNION` dedups; two indexes order relation; keys contain file paths + textual node identities; chain has quadratic worst case (:302-305). Samples (1,594 active on one libuv worker): `sqlite3VdbeRecordCompareWithSkip` 14.1%, `_platform_memcmp` 12.9%, `sqlite3VdbeExec` 8.5%, `_platform_memmove` 8.2%, `sqlite3BtreeIndexMoveto` 7.0%, `vdbeSorterCompareText` 4.6%, `vdbeSorterSort` 4.4% (:310-318). All via `relational::create_stage`; stage construction 62.2-86.4% of active samples in state intervals (:320-323). "Full-scan counters alone do not prove a missing index; recursive seed scans can be necessary." (:330)
3. Global file evaluation serial: `evaluate_file_relations` loops paths synchronously in `workspace/global.rs`; repository-wide clauses run in global pass after deferred input ingestion; four workers don't mean four concurrent relational evals; one active native worker observed (:334-338). Concurrency needs input-byte or retained-memory bound (:342-343).
4. **Changed file repeats pipeline (invalidation)**: "Unchanged replay is fast because the workspace retains final clause results. A changed source version invalidates those results, including for a trailing newline. The changed file then repeats input extraction, SQL materialization, and index creation for its clauses. This explains quality's 11.119-second edit despite an 8.272-millisecond unchanged replay." (:347-350). "Shared input and relation work should therefore precede a general incremental-SQL mechanism." Validate clean->positive, positive->clean, removal, option changes, companion-file edits (:352-353).
- Profiler bug: Rayon can run a one-item iterator on the apply thread; `WorkerProfile` cleared enclosing profile; fixed (:358-360).

---

## data-layer-performance-journal.md (3701 lines) -- chronological digest

The journal is written newest-first (:11 is 2026-10-24 latest; :3463 is the 2026-10-21 baseline; :3570 "Experiment ledger" is oldest-first, 2026-10-20 onward). Below is oldest-first. Workloads: "quality" = 5 files/1,026,452 B; "state" = 7 files/1,123,712 B (later "state, one file" = 108,562 B); "dispatch" = 1 file/136,810 B; "positive control" = two rules, twelve files, 61 findings; "ten rules" = ten sixth-batch rules (12 files/1,984,216 B in one place, 2 files/178,826 B in the "smaller ten-rule snapshot"); repository = 9,021 files/137,924,324 B (first-50) or 9,010 files/137,896,473 B (batch six). Later 2026-10-24 entries use a fixed "small state-tripwire guard" (two original observations through snapshot + workspace paths in one cold process) measured in **engine-submitted SQL statements** against a fixed 100-submission gate. Policy (D, :8-9, :3697-3701): one chosen execution path; experiments in isolated worktrees; no selectable execution paths.

### 2026-10-20 (Experiment ledger :3572-3608)
- Shared SQL + reachability (`d97e1239`): keep SQL as semantic engine; reuse shared facts before comparing backends; "The earlier specialized Rust evaluator did not provide a fair comparison against equivalently shared SQL preparation." Batched recursive SQL for general directed graphs; recursive UNION for cycle termination + dedup; "UNION ALL is valid only with a proved unique-path invariant. Scalar probes do not replace a broad batch interface." (:3574-3582)
- Generic forest traversal and storage (`f52817fc`): native structural primitives only where graph contract explicit; IR-name propagation native is slower, rejected; budget model initially double-charged (:3590-3597).
- Shared syntax preparation (`c4bd76bf`): "remove repeated preparation before adding more execution backends"; immutable syntax facts, direct parser identities, subtree boundaries (:3603-3606).

### 2026-10-21 ledger entries (:3610-3684)
- Broad producer passes (`eec751d0`): count/scan/scatter per dependency wave; resident packed input; pass count constant as roots/files/policies grow; "Batching alone does not establish a large CPU gain." (:3612-3618)
- Complete SQL instrumentation (`e45dd638`): statement capture, CPU counters, plans, bytecode (:3624-3627). "VM instructions, sorts, and cursor visits supply explanations. They do not replace measured CPU."
- Isolated SQL-shape experiments (`b6934ddf`; M, medians of 3, quality workload, process CPU ms) (:3637-3651):

| Experiment | Cold | Edit | Decision |
|---|---:|---:|---|
| Baseline | 2,966 | 1,180 | Control |
| Method identity index | 2,822 | 1,011 | Keep |
| Forced owner-first joins | 2,842 | 1,048 | Reject forced order |
| Index + forced joins | 2,799 | 985 | Keep index alone |
| Return range index + bound | 2,943 | 1,120 | Keep |
| Narrow projections | 2,815 | 1,098 | Keep |
| UNION ALL root seeds | 3,027 | 1,197 | Reject |
| Materialized root CTE | 3,131 | 1,296 | Reject |
| Root unpivot | 3,080 | 1,247 | Reject |
| Virtual publication | 2,412 | 989 | Keep |
| SQL combination w/ UNION ALL roots | 2,548 | 835 | Control |
| That + virtual publication | 1,996 | 718 | Control |
| Accepted combination, original roots | 1,991 | 685 | Promote |

  - Method index lets SQLite choose join order without forced CROSS JOIN; `self_stores` ~8.1M -> 77,000 VM instructions (:3653-3654). Narrow projections keep multiplicity, not physical column removal (:3656-3657). **Virtual publication** replaces output insertion + SQLite index construction with access to completed numeric arrays: diagnostic publication CPU 671 ms -> 19 ms, some extra cost in consumers (:3659-3661). "All root rewrites regress ... UNION ALL removes sorts and lowers VM counts ... Those counter reductions do not offset their other execution costs." (:3663-3666)
- Promotion (`7da151b8`): method + return indexes, column narrowing, completed-output virtual table become normal; bounded 32-row insertion; stages always released after last consumer; three source domains separate; five execution switches removed (:3672-3678).

### 2026-10-21 Current baseline, `7da151b8` (:3463-3568) (M; median of 5 fresh processes; process CPU ms)

| Workload | Files / bytes | Compile | Cold | Unchanged | Edit |
|---|---|---:|---:|---:|---:|
| Quality | 5 / 1,026,452 | 88.2 | 1,940.1 | 1.96 | 710.1 |
| State | 7 / 1,123,712 | 298.3 | 12,512.3 | 1.61 | 1,137.8 |
| Dispatch | 1 / 136,810 | 46.8 | 982.7 | 1.43 | 972.5 |

- Peaks 444 / 1,223 / 390 MiB (:3480). Build opt-level 1, deps 3, thin LTO, 256 CGU, Apple M5 Max 18 logical CPUs, Node 26.8.1, four workers (:3496-3499). Diagnostic span CPU: SQL materialization 1,096 / 5,495 / 675 ms; ordinary indexes 251 / 2,410 / 158; completed numeric publication 20 ms (quality) (:3518-3522). Quality targets `scoped_nodes` ~131 ms, `scope_owners` ~124 ms, root query ~113 ms (:3532-3534). "State is now the largest cold workload ... Those paths still reconstruct facts through the older evidence and SQL representation." (:3536-3539)
- Note dispatch edit (972.5) ~ cold (982.7): edits re-run the whole file's work.

### 2026-10-21 Full repository verification, `3285b48f` (:3358-3461) (M)

| Workload | Cold CPU | Cold elapsed | Unchanged CPU | Edit CPU | Peak RSS | Result |
|---|---:|---:|---:|---:|---:|---|
| First 50 rules | 212.619 s | 233.738 s | 0.801 s | 40.182 s | 6.60 GiB | Pass, 24,820 findings |
| Batch six historical | n/r | 951.004 s | n/r | n/r | 11.14 GiB | Pass, 173 |
| Batch six current | 538.452 s | 645.958 s | 0.087 s | 11.344 s | 21.33 GiB | Fail: 890 engine failures |

- Remembered "1,400 s" = 1,456.276 s for batch six's ten rules with 84 extra findings and 46 failures; no historical CPU recorded (:3391-3395). Source corpus: 3,210 native observations (1,631 positive, 1,579 clean) all pass (:3403).
- Root causes (:3418-3428): 889 `producer capacity exceeded` in `quality-contracts-1`: producer packs ready relations into shared 64 MiB arena and rejects every pending relation when exceeded; driver creates all dirty jobs and SQL sessions before the producer wave; SQL session storage outside arena. One `repository input budget exceeded` in `quality-conditional-1`: `Session::new` checks combined source bytes against 32 MB limit before selecting syntax-gated files. Sample: 789/796 in SQLite stage construction (:3430-3431).

### 2026-10-21 Batch six diagnosis, after `71ea1606` (:3156-3356) (M)

| Combined 60-rule run | Cold CPU | Elapsed | Unchanged | Edit | Peak RSS | Result |
|---|---:|---:|---:|---:|---:|---|
| After capacity admission, before SQL fixes | 1,076.563 s | 1,054.093 s | 2.102 s | 53.083 s | 28.12 GiB | Fail |
| Final | 1,114.637 s | 1,277.989 s | 2.109 s | 63.196 s | 18.30 GiB | Pass 24,993 findings |

- Fix 1, capacity admission: driver now prepares consumers as capacity permits; counts shared packed columns once; SQLite storage via 64-bit status API at first producer suspension; finished consumers close before admission continues; one 100-pass allowance across partitions (:3190-3196). Target 256 MiB SQLite; producer 64 MiB input / 256 MiB numeric output (:3198-3203).
- Fix 2, "A file gate cannot preserve selected AST rows": the conditional rule's authored AST input selects individual rows + ancestry, but native syntax lowering replaced it with a file gate + whole-file syntax; because the selection includes module declarations, every file qualifies -> entire repository's syntax and source demand -> 32 MB guard correctly rejects 137 MB (:3211-3216). Fix restricts native syntax lowering to file-partitioned contract clauses; global conditional clause uses projected AST inputs and ordinary SQLite stages; 20 typed input partitions share one AST selection utility (:3221-3223).
- SQL shape changes (:3241-3289): general column-nulling pass increased VM 9,574,525 -> 10,680,292 (removed); ordinal window sorts narrow keys before joining projected values; declaration selection uses materialized key sets; together 8,723,163 VM (~9% fewer) without CPU gain (:3242-3249). Descendant normalization exhausted work at 784,639,688 VM because normalizer prepared graph keys for every selected file; authored plan now materializes roots once and limits edges to those file domains ("CST edges cannot cross files"); one covering forward edge index replaces two; unused outermost-function ownership index removed (:3251-3258). Graph-key compaction: key sort 67,410,860 VM, two key indexes 18,962,193, compaction 12,967,702 before interruption; fix: declare dense number as `INTEGER PRIMARY KEY`, SQLite assigns during dedup insertion, removing numbering window + redundant index; 5,008 keys normalization ~921,600 -> 699,000 VM (:3260-3273). Prepare union of requested subtrees through one frontier: graph catalog 824,441 -> 88,625 keys; key insertion 67,410,860 -> 4,596,429 VM; conditional-only diagnostic 199.837 s cold CPU, 7.24 GiB (traced, 2,048-file batches) (:3276-3288).
- Quality-only run 140.094 s cold CPU, 2.79 GiB; first partition 439 files, 950,054 roots, three producer passes; 268,454,464 SQLite admission bytes across 450 suspended sessions (:3291-3296). Five-file quality probe 3.888 s cold / 1.068 s edit vs earlier 1.940 / 0.710: correctness cost of preserving selected global evidence (:3299-3301).
- Profiling overhead: `record_workspace_residency` after every ingestion batch walks all retained witnesses (74/96 samples) (:3304-3308). Workspace prepares every dirty file-local job before SQL scheduler; eligibility witnesses are gates not complete inputs; consumer restores tapes, extracts full AST inputs via `relational_rows` and retains rows in queued jobs; 87/89 samples include `relational_rows`/`stateless_rows` (:3313-3323). "The next admission boundary must precede full projection, with queued descriptors holding document versions rather than complete input row vectors." (:3324)
- Conditional forest walk ~56 ms CPU for 46,469 roots in 182 batches vs ~200 s diagnostic: "Faster graph traversal cannot remove the extraction and materialization costs around it." (:3353-3354)

### 2026-10-21 CPU and memory attribution, after `4def200d` (:3012-3154) (M)
- Python vs Rerail, ten sixth-batch rules, 12 files, 1,984,216 bytes, 61 findings: upstream Python shared fact engine (one worker) 4.320 s / 196.70 MiB; Rerail 37.237 s / 1.681 GiB; **8.62x CPU, 8.75x RSS** (:3027-3031). Native: 7.558 s ingestion + 29.675 s global prep + SQL (:3041).
- Queue before admission: 80 jobs, 577,199 projected rows, 9,833,801 JSON fields, ~1,256,398,807-byte referenced heap estimate; RSS peak 1,805,041,664 (:3058-3062). Consumers: state flow 196,089 rows, legacy execution graphs 162,195, state signatures 118,589, remaining 100,326 (:3066-3069). Field names 103,706,252 bytes, string values 82,247,225 bytes (:3072). "The 256 MiB SQL admission target starts after this queue exists. It therefore cannot bound this preparation peak." 64 MiB document cache can't release syntax objects retained by queued jobs (:3077-3079).
- Field navigation (64-file quality): 16,140,707 field lookups; 427,807,885 child examinations; 423,856,845 (99.08%) inside failed lookups; max scan 400 children; `TapeNode::child_by_field_id` searches sibling list per request; 43.0% and 63.7% of self samples in two windows (:3088-3095). "Python's fact engine shares one AST traversal across collectors and accesses AST fields directly." (:3098)
- Profiling quadratic: residency accounting summed ~316 GB estimates across 288 revisions; 74/96 samples; old 1,114.637 s can't establish clean time; removed (64-file control 7.630 -> 7.245 s) (:3106-3117).
- One state-signature descendant materialization 23,860,081 VM instructions (:3125).
- Required order (D) (:3133-3137): 1) queue versioned descriptors before construction, admit capture + syntax leases in same memory boundary as SQL consumers; 2) shared capture columns per file version with row-selection views and union of demanded columns; 3) source text as bounded references; 4) index AST field access, reject impossible fields; 5) reduce demanded descendant roots + repeated intermediates.

### 2026-10-21 Field-aware anchors and shared input indexes, after `a92c126a` (:2923-3010) (M)
- Existing kind + value postings plus sparse postings for requested AST fields (each field posting = first child in that field); stored parent links for reverse navigation; file-local fact extraction uses these before matcher builds captures (:2928-2933). Tape records first-field occupancy in existing padding: navigation record stays 40 bytes (D) (:2935-2936). Candidate indexes remain in Rust because matcher consumes tape before SQL input construction (:2940).
- Ten rules 12 files: 38.155 -> 32.436 s CPU, 2.564 -> 2.474 GiB; quality 64 files (traced) 9.184 -> 6.947 s (:2955-2956).
- Navigation: field lookups 16,140,707 -> 614,645; child examinations 427,807,885 -> 1,403,498 (-99.67%); failed-lookup examinations 423,856,845 -> 808,916; direct known-edge checks 0 -> 2,050,091 (:2971-2978). First impl rebuilt structural index per input: 695 builds over 31,441,607 nodes, rejected; final 76 indexes over 3,466,450 nodes; selection cache keyed by compiled plan + current source identity; source change clears cache (:2979-2990).
- Remaining: 22.052 s SQL batch, 5.785 s input prep (:3002).

### 2026-10-21 SQLite positional kind anchors, after `5616076a` (:2789-2921)
- **Decision: use the positional FTS5 kind index from `ast-index-spike`** (`bench/tape_store.py::_build_kind_fts`): one contentless FTS5 row per file; each preorder node contributes a bare token `k<base36(language * 1024 + kind)>` including anonymous nodes; FTS5 instance vocabulary returns file IDs + preorder positions; contentless deletion for replacement/removal; no field tokens or persistent row per AST node (:2791-2798).
- Storage: real temp file, 1 MiB main page-cache target, file-backed temp storage with 512 KiB target, mmap disabled; a 4,816,896-byte DB -> 1,114,704 bytes SQLite page cache + schema + statements (:2800-2805).
- SQL selects candidates via kind postings, text predicates, field constraints, parent relationships; existing matcher constructs captures and checks semantics (:2807-2809).
- Rejected trials (:2825-2876): loading full tape BLOBs 16.005 s vs Rust 2.631-2.848 s, 76.33% in `pread`; incremental BLOB reads -> 3.867 s. CTE reference expansion -> 178 table-reference-limit errors; fix: temp tables at shared deps with large expansion estimates (physical boundaries). **`SQLITE_ENABLE_MEMORY_MANAGEMENT` forced all connections into one shared page-cache group: 44.84% samples in mutex wakeups; disabled.** Pattern plans scanned postings per candidate root and estimated by kind counts; fixed to actual posting counts + indexed temp tables; materialized span relation computes subtree end once. Ancestor walks replaced by indexed SQL range queries since subtree ordinals form intervals; inside predicates remove nested intervals with a SQL window (:2869-2874).
- Matched (M): ten rules 12 files Rust 32.436 s vs SQL 105.663 s (2.474 vs 2.546 GiB); quality Rust 2.631 vs SQL 3.113 s (:2890-2893). Candidate selection 81.422 s vs Rust 3.722 s; root pairs 5,176,955 vs 1,604,003; 1,258 compiler broadenings ("compiler broadens plans that exceed its CTE bound ... preserves findings but weakens filtering") (:2902-2911).

### 2026-10-21 Corpus index lifetime correction (:2696-2787)
- Previous adapter rebuilt a temp SQL index per file partition (rejected). Workspace now owns **one file-backed corpus database**; file's source digest + grammar determine whether postings need replacement; unchanged documents reuse rows incl. after tape restoration and document-cache eviction; publication of missing files before partitions; removal deletes rows (:2698-2707). Workers lease separate connections; WAL; DB survives workspace revisions, not process restarts (:2709-2719). Kind counts by file support replacement/deletion without scanning vocabulary (:2714).
- 12 files / 379,809 nodes published once vs rejected adapter 88 indexes / 3,846,259 nodes; preparation 0.677 -> 0.062 s (:2726-2729). Ten rules: Rust 32.436 s vs corpus SQL 122.420 s (:2733). Incremental probe: unchanged 0.002788 s CPU; edit 0.816207 s (M) (:2743-2745).
- Selection 93.322 s vs Rust 3.722 s; 1,258 broadenings; 8,990,941 candidate root pairs (:2749-2750). **FTS access-path limitation**: vocabulary table seeks by term then tests file; `fts5VocabBestIndexMethod` exposes no document constraint; 100 requested postings constant + 32 unrelated files: ~700 -> 160,700 VM steps (:2751-2756).

### 2026-10-21 Batch candidate preparation (:2611-2694)
- Previously queried postings separately per file and rule: "That query boundary defeated batching." Now prepare candidate sets before matching; partitions consume without candidate SQL (:2613-2616). Each SQL batch up to 32 plans and 16,384 files of one language (D) (:2619). One workspace corpus retains prepared sets across partitions and revisions; source or grammar change invalidates that file's sets; unchanged request issues no candidate SQL (:2621-2623). Transport stores empty, complete, sparse, or bitmap selections (:2628). Interrupted batch publishes no partial absence (:2634).
- M: positive 2.620 -> 2.198 s, 298.39 -> 376.33 MiB; quality 2.606 -> 3.023 s, 488.83 -> 614.84 MiB (:2655-2658). Unchanged revision executes no candidate statements; edit executes nine (:2670).

### 2026-10-21 Repeated index comparison (:2572-2609) (M, 3 trials)

| Implementation | Positive CPU | Positive RSS | Quality CPU | Quality RSS |
|---|---:|---:|---:|---:|
| Rust index | 0.462 s | 305.12 MiB | 2.648 s | 497.36 MiB |
| SQL, rebuilt per partition | 2.339 s | 303.06 | 3.196 s | 496.27 |
| SQL, shared corpus individual queries | 2.633 s | 301.22 | 3.025 s | 492.66 |
| SQL, batched candidates | 2.136 s | 399.39 | 2.956 s | 614.05 |

- Batched SQL vs Rust index: +362.9% CPU on positive control, +11.6% quality (:2595).

### 2026-10-21 Candidate SQL CPU root cause (:2476-2570)
- "The kind postings match `ast-index-spike`'s encoding, but the value access path is missing." Reference persists value postings in `vidx` and distinct-term vocabulary in `vlex`; evaluates text predicates against vocabulary terms then seeks postings; ours decoded and tested individual occurrences (:2479-2483). Reference has coverage checks and bounded residue to avoid false absence (:2485-2486).
- Positive control: 164,171,085 VM steps, 2,174 sorts; one statement with 710 CTEs and 536 sorts; redundant `s162 = s160 INTERSECT s5` then intersect with `s5` twice more (:2492-2499). Compiler reuses identical relations + canonical intersections (:2500). After cleanup: statements 82 -> 72; VM 164,171,085 -> 111,766,482; sorts 2,174 -> 1,292; BLOB reads 2,858,925 -> 2,791,126; text extractions 519,033 -> 517,994 (:2505-2512). BLOB API transferred 102,258,042 bytes, repeated packed-span blocks 82,960,502, source text 8,637,096; 340,517 AST nodes indexed once (:2516-2519).
- Measurement error: an internal FTS cursor open while other statements run reported 14.5 CPU s lifetime during a 2.547 s cold phase (:2534-2538).
- M: positive 2.216 -> 1.783 s (-19.5%); quality 2.948 -> 2.984 s (:2547-2548).

### 2026-10-21 Value postings, vocabulary access, shared SQL properties (:2392-2474)
- Corpus stores case-exact value postings and a persistent distinct-term vocabulary; value index beside positional kind index; replacement and deletion update both in same transaction (:2394-2396). SQLite B-tree keys keep complete terms (not reference's linear-hash buckets); each `(term, file, kind)` has a delta-varint posting segment; small vtab streams packed ordinals; SQL owns filtering/joins (:2399-2403).
- Coverage: every clean token incl. keywords, numbers, anonymous tokens; clean-token limit 96 Unicode chars; non-clean leaf residue corpus-wide limit 4,096 occurrences per language and kind; residue text <= 384 KiB per kind per publication (D) (:2405-2409). Literal predicates seek terms; regex predicates test distinct vocabulary terms once per batch (:2411).
- **Shared decoded properties live in ordinary SQL columns keyed by file and preorder**; separate tables for structural properties, span offsets, first-child field lookups; batched fills decode missing keys and reuse rows; file replacement invalidates via foreign keys; no source text for interior nodes (:2416-2420).
- **Affinity regression**: first impl regressed positive control 1.854 -> 9.112 s: INTEGER bounds from decoded properties met temp ordinal columns with empty affinity; SQLite omitted ordinal range constraints and scanned complete postings; two statements > 660 million VM steps; fix: explicit INTEGER affinity on materialized numeric columns; 2,048-interval case ~35,664,900 -> 32,700 VM (:2422-2430).
- Repeated base-posting scans + tiny BLOB reads: materialize shared kind postings, enumerate with series vtab; small record/span buffers (:2433-2436). A live all-node demand covers subset demands; hoisted; file completeness recorded in SQL (:2439-2441).
- M: positive 1.874 -> 2.504 s (+33.6%); quality 3.004 -> 3.192 s (+6.2%) (:2446-2449). Text extractions 517,994 -> 13,739; BLOB reads 2,791,126 -> 51,444; VM 111,766,482 -> 130,976,638; data statements 72 -> 550 (:2456-2460). "Shared property construction, temporary relation copying, and index construction now form the next performance frontier." (:2462)

### 2026-10-22 SQL preparation and retained-layout experiments (:2258-2390)
- Decision: keep shared preparation, duplicate-expression removal, incremental vocabulary matches; reject retained expanded postings and generic utility sets (:2260-2261).
- M (medians of 3): positive cold 2.476 -> 1.936 s (-21.8%), edit 0.543 -> 0.418 s; quality cold 3.458 -> 3.268 s (-5.5%), edit 1.007 -> 0.933 s (:2277-2280). Unchanged ~3 ms, no candidate SQL (:2284). Missed the 30%/10% targets (:2289).
- Accepted: universal structural demands collected across all 32-plan groups; fresh files stream typed tape columns into retained SQL properties table; partly prepared files decode missing keys only; changed files invalidate own facts (:2293-2297). Fixed SQL request tables receive file IDs in batches of 128 (:2302). Compiled preparation templates survive file edits (:2303). Regex matches persist by language, expression, term; FKs remove matches when last occurrence disappears; per-term dynamic regex compile regressed to 14.336 s, fixed by caching one predicate per SQL expression (:2306-2311).
- Rejected retention (M; replay thread CPU): two rules temp stages 1.146 s vs retained postings 1.501 s (+31.0%) vs retained shared sets 1.164 s (+1.6%); three rules 1.948 vs 2.427 (+24.6%) vs 2.252 (+15.6%) (:2320-2323). Retained postings 303,285 rows with no reuse; 410,038 rows reusing 32 constructions; shared sets 163,552 rows reusing 38: "The reuse does not offset retained writes, wider keys, completion joins, and subsequent reads." (:2331-2334)
- **Key-order lesson**: retained key with revision between file ID and preorder -> SQLite failed to use the ordinal suffix; corrected PK `(relation_id,fid,pre)` with revision in payload + cascading FK; 2,048-interval range work ~39.9M -> 79,000 VM; membership ~16.9M -> 92,000 (:2338-2343).
- Remaining: 500 candidate statements incl. 480 materializations; structural prep decodes 340,517 nodes once ~155 ms; other materializations ~734 ms, temp indexes 116 ms, final queries 250 ms; VM 141.6M -> 98.9M; main DB 16.97 MB, temp 12.13 MB, WAL 9.36 MB (:2348-2356). "The separate authored relational executor has not acquired a shared relation cache." (:2357)

### 2026-10-22 Typed projected facts and SQL input streaming (:2165-2256)
- Decision: keep typed column storage, SQL column demand, deferred preparation, bounded SQL import; removes per-row JSON maps; no end-to-end improvement (:2167-2168). Projected facts share schemas, source slices, immutable typed column batches (`violations/projection.rs`); SQLite validation records actual column reads; import binds column positions once per schema; scheduler queues consumer descriptions and restores file-local inputs during session preparation; input sink inserts completed column batches and releases them (`relational/inputs.rs`) (:2174-2180).
- M: state one file cold 3.202 -> 3.228 s; peak RSS 395.8 -> 461.6 MiB (+16.6%); RSS after global eval 288.3 -> 222.2 MiB (:2200-2204). Python 0.264 s (state) and 1.429 s (ten-rule): native ~12x and ~16x slower (:2215-2216). Queue: baseline 11,272 witnesses / 198,758 JSON fields / ~19,624,497 B; final 2 consumer descriptions, 0 materialized rows, streams 11,272 rows with <= 256 buffered (:2223-2225). Samples 98-100% SQLite (:2229).

### 2026-10-22 SQL composition opportunities (:2075-2163) -- kernels (see sql-composition-opportunities.md); extra numbers: interval VM 9,529,897 -> 72,996 (694 rows) (:2118-2119); selective SQL still decodes all 4,612 identifiers; materialized demand 244 (:2123-2125); utility depth 4->8 named SQL 709 -> 1,333 bytes but preparation 0.392 -> 6.075 ms, physical cuts 0.274/0.431 ms: "Naming alone does not bound SQLite preparation work." (:2132-2135)

### 2026-10-22 SQL composition promotion (:1951-2073)
- Forest: 1,000-node chain retains 1,000 positions + 1,000 root ranges, exposes 500,500 logical pairs (:1967). 535-output regression -> five packets 128,128,128,128,23 (:1981).
- Integration regressions: sharing full catalog amplified copies/scans during SQL generation; indexed dependency traversal template generation 1,116.586 -> 604.187 ms (old 600.219) (:1985-1988). **SQLite reordered correlated subtree probe to scan all identifiers before interval membership: `direct_events` 774,666 -> 16,328,922 VM; explicit `CROSS JOIN` -> 985,870** (:1990-1993).
- State diagnostic: catalogs 5 -> 1; temp tables 1,784 -> 1,029; materialization+fill stages 3,044 -> 1,712; packets 5 -> 2 for 139-plan frontier (:1996-1999).
- M: state 2.424 -> 2.120 s (-12.5%), RSS 397.2 -> 451.5 MiB (+13.6%); ten rules 13.338 -> 9.903 s (-25.8%), 497.1 -> 540.9 MiB (+8.8%) (:2020-2021). "Fewer SQL constructions do not guarantee lower allocator residency or a smaller live relation frontier." (:2041)

### 2026-10-23 Candidate SQL relation lifetimes (:1829-1949)
- Decision: one dependency schedule, construction before demand, release after last SQL consumer (previously every temp relation built before first packet and retained to end) (:1831, :1843-1844).
- M: peak live tables state 1,029 -> 206, temp storage 14.00 -> 2.66 MiB; ten rules 1,905 -> 437, 48.10 -> 8.99 MiB (:1867-1868). CPU state 2.582 -> 2.597 s; ten rules 13.317 -> 13.106 s; RSS ~flat (:1876-1877).
- Why memory stays high (:1900-1924): state run ends candidate prep with zero live temp tables; allocator in-use 65.9 -> 66.6 MiB, managed space 138.4 -> 210.4 MiB, RSS +71.7 MiB. SQLite execution peak 68.5 MiB (state) / 98.6 MiB (ten rules), preparation peak 10.1/11.8 MiB; the state peak is in the first result packet whose query contains **1,101 inline materialized CTEs, 983 temp B-trees**; compiled statement ~3.3 MiB (:1913-1921).

### 2026-10-23 Design entries (:1757-1827): materialization ownership design (static audit 248/420 multi-statement definitions), compiled stage execution design, named producer discovery (state 1,716 host statements / 10,320 placements / 157 multi-host definitions / 1,240 extra; ten rules 3,216 / 19,001 / 255 / 2,048 -- lower than earlier audit because the tool distinguishes write epochs and CTAS projections) (:1784-1789), shared producer plan.

### 2026-10-23 Aggregate optimization checkpoint (:1422-1755) -- the big regression
- Repository comparisons (M) (:1457-1462):

| Workload | Reference | Cold CPU | High-water RSS | Parity |
|---|---|---:|---:|---|
| First 50 | Pre-SQL | 125.804 s | 6.441 GiB | 24,820, pass |
| First 50 | Final SQL | 2,507.496 s | 6.143 GiB | 24,820, pass |
| 60 rules | Pre-SQL | 709.151 s | 16.102 GiB | match; 23 failures, 2 failed incremental |
| 60 rules | Final SQL | 7,211.657 s | 14.997 GiB | match; 34 failures, 2 failed incremental |

- First-50 SQL = **19.93x** pre-SQL (:1467). 60-rule SQL: 34 relational failures in `serverplan-state-tripwires` (12 regex-budget, 22 query interruptions) (:1479). 60-rule cold split 6,782.080 s ingestion + 429.207 s global (prior 202.550 + 506.184); unchanged 0.517 s; edit 121.587 s vs 62.576 s previously (:1486-1488). 288 ingestion batches, 1,363,392 candidate data statements, 1,359,072 materializations, 197,856 plan memberships = 687 plans/batch vs 377 first-50; 28,230,245 indexed nodes (:1494-1496).
- First-50 phases: file-input applies 73.350 -> 2,456.015 s; global apply 52.144 -> 51.145 s (:1514-1517). Anchor selection counters 2,247.966 s, anchor index construction 125.776 s, parsing 11.087 s (:1519-1520). 492,480 data statements, 489,024 materializations (:1523).
- Broad-run access-path regression (:1542-1576): `fts5VocabBestIndexMethod` accepts term but ignores document constraints; candidate query joins requested files after corpus-wide postings scan -> later batches repeat over growing corpus. Regression test: unrelated 2,000-statement file raises ticks 1 -> 361. A decoded `(fid,kind,pre)` index passed covered test but failed sparse ingestion probe (rejected). Selected: compressed positions on `file_kind_counts` rows with `(fid,code)` PK; atomic version-2 upgrade from persisted records without reparsing. Value: 200 unrelated files 1 -> 19 ticks; fenced term + requested-file inputs before `vidx` PK seek.
- Retained-property join regression (:1616-1626): structural parents for one file + unrelated 2,000-statement file: 806 ticks instead of 6; fix: requested keys as outer input with `CROSS JOIN` fences.
- **Preparation frontier** (:1639-1656): "The workspace gathers anchor plans from every compiled rule before it publishes the eligible source jobs ... then prepares every language-compatible plan for each published file. The corpus API receives language and plan identities, without per-rule file eligibility." One batch: 32 files, 377 plans, 1,710 statements, 1,698 materializations; 128 rule-file entries, 128 rejections. 83 applies with zero roots for every rule-file still execute 140,934 materializations and 697.407 s anchor-selection CPU. `SelectWhen` collects both queries before runtime condition selects consumers.
- Pager pressure (:1662-1673): anchor-selection CPU per indexed node 50.2 -> 115.3 microseconds across four groups of 72 applies; 537/749 stack observations end in `pwrite`, 65 in `pread` (`pagerStress`, `pagerWalFrames`, `subjournalPageIfRequired`); main DB > 1 GiB with 1,024 KiB main / 512 KiB temp cache targets.
- Controlled cache replay (M; 1,886 files, 6,223,803 nodes, `_conditional_compile.py` 18,993-node request; 146,136-row multiset; 59,590,722 VM steps, 3,294,894 full-scan steps, 4,979 sorts) (:1694-1709):

| Main / temp cache | Median SQL CPU | Peak RSS | Misses | Writes | Spills |
|---|---:|---:|---:|---:|---:|
| 1 MiB / 0.5 MiB | 1,531.078 ms | 112.656 MiB | 66,709 | 14,949 | 14,630 |
| 64 MiB / 0.5 MiB | 1,336.781 ms | 143.563 MiB | 10,814 | 5,427 | 1,721 |
| 1 MiB / 16 MiB | 1,402.635 ms | 109.344 MiB | 62,595 | 13,119 | 12,909 |
| 64 MiB / 16 MiB | 1,403.610 ms | 150.484 MiB | 6,700 | 3,597 | 0 |

  - Larger budgets eliminate spills, -8.3% SQL CPU; do not explain 19.93x; "The unchanged VM work remains substantial for one requested file, so demand scope and repeated query work remain the main architectural targets." (:1711-1714)
- Memory (:1719-1737): vmmap 6.6 GiB physical footprint (4.5 resident, 1.9 swapped `MALLOC_SMALL`); 15,992,992 allocations / 6.2 GiB; prepared candidate payloads ~218 MiB; later 31,521,959 allocations ~11.3 GiB, footprint ~12.7 GiB (peak 13.9); no leak established.

### 2026-10-23 SQL execution limit restored (:1362-1420)
- Fixed **100-execution job acceptance limit** from commit `c109a915`; counts complete cold process across workers/connections incl. compilation setup, DDL, publication, transactions, matching, cleanup; each `execute_batch` statement counts; cached executions count; EXPLAIN and native spans don't (:1364-1371).
- First attempt (M): setup incl. DDL + ANALYZE 17,686; transactions 168; matching/publication/cleanup 6,305; **total 24,159 / 100**; 11,544 CREATEs, 5,508 DROPs; execution 101 creates `syntax_21` during compilation schema validation; 1,100 `INSERT INTO kind` and 128 `INSERT INTO field` executions (:1385-1396).

### 2026-10-23 Recovery TDD + checkpoint (:890-1361; read bottom-up within the section)
- Guard launcher bypass fixed (missing report returned zero) (:1139-1152). Execution inventory counts raw SQL events with session, worker, producer, binding, generation, file domain, consumer, stage, SQL CPU, VM steps (:1159-1161). Baseline inventory 24,159: 464 setup, 1,232 catalog publication, 3,339 relational, 19,124 unknown (:1165-1166).
- R2 admission: adding a path-excluded rule grew prepared template from one plan to two; twenty files across two disjoint scopes ran both plans over all twenty files; fix: two batches, one plan + ten files each; path, text, syntax, language exclusions leave only admitted plan; "Source generation remains part of the corpus file identity" (:1172-1188). Count 24,159 -> 22,181 (-8.19%): 936 setup, 1,232 catalog publication, 272 file publication, 16,402 candidate preparation, 3,339 relational (:1210-1212). Snapshot + relational prep admission fixed; "Relational preparation no longer expands every workspace rule over the complete corpus." (:1229-1232)
- R3 publication: four files x 200 identifier values: 275 kind inserts, 32 field, 28 kind-frequency, 28 file-kind, 808 value-posting, 28 coverage inserts = 1,199 executions -> 18 (one language-catalog batch per table, one publication batch per file and table); bindings below SQLite variable limit and a 4,096-value cap; empty batches execute no SQL (:1222-1247).
- FTS audit: one/ten/101 documents execute 9/18/109 SQLite statements via one bulk `INSERT ... SELECT` (internal document-size writes once per document) (:1249-1252). Acceptance boundary fixed at **100 engine-submitted statements, SQLite internal executions reported separately** (user decision) (:1253-1255, :1261). Origin attribution via SQLite trace callback decoration (nested VDBE comment prefix) (:1265-1270). contentless-delete layout rejects `columnsize=0` (:1258).
- FTS bulk experiment (M; 1,024 files x 256 tokens): packet transactions 39.275 ms; packet inserts in one transaction 23.189 ms; one complete bulk insert 11.545 ms (-70.6% / -50.2%); packet forms leave five segments, bulk one (:1288-1296). Supports whole-frontier `INSERT ... SELECT`.
- Cleanup inventory: 10,443 engine submissions + 994 internal: 5,176 candidate construction, 2,534 cleanup, 1,781 relational, 904 setup, 48 publication (:1305-1306).
- Recovery checkpoint (M) (:1319-1326): total SQLite executions 24,159 -> 11,437 (-52.66%); median complete replay CPU 3,441.002 -> 2,567.392 ms (-25.39%); maxRSS 476,144 -> 370,304 KiB.
- Compiler schema (:978-1130): compiler registration via eponymous-only SQLite modules (no CREATE executed); prior addon executed 48 CREATE TABLE for 12 clauses (:1129-1135). `Corpus::prepare_files` sorts plans by Arc allocation address -> changing compiler allocation changes lowering order, reuse, broadening (:1120-1121); schema-adapter slice alone 11,641 submissions (candidate prep +1,088) (:1116-1118). Authored-order lowering: 14,699 submissions, +4.51% CPU (rejected) (:1031-1037); work-estimate order 14,699, +3.36% (rejected) (:1017-1021). Rollback of an over-budget selector cleared earlier reuse caches: 7 -> 8 materializations (:1067-1069) (later found control incomplete, :1047-1053). Compiler connections without runtime PRAGMAs (24 PRAGMAs across 12 prepare-only sessions removed) (:1086-1090). Selected compiler checkpoint: 9,695 submissions; connection setup 904 -> 498; CPU 2,374.190 -> 2,266.036 ms (-4.56%, overlapping) (:985-993). 194 compiler stage CTAS executions contain only 53 distinct SQL strings; "SQLite rejects CTAS through its virtual-table declaration API, so native affinity inference remains necessary on a cache miss" (:970-971). Schema cache keyed by exact SQL + complete ordered schemas of read relations + deferred column names; limits 128 entries / 4 MiB; process-owned; result 9,095 submissions; stage CREATEs 194 -> 68; CPU 2,479.402 -> 2,595.351 ms (+4.68% regression, kept for shape) (:894-921).

### 2026-10-24 Shared producer foundation (:754-888)
- Catalog + exports: 9,113 submissions; 8 manifests, 56 binding groups; one span binding with 117 consumer/selector references; each frontier still 22 span-fill statements; CPU 2,390.952 -> 2,299.499 ms; RSS 412,064 -> 334,976 KiB (:857-867). Protected view fails: SQLite `count(*)` shortcut reports backing read without accessor; CTE can impersonate accessor label (:843-845).
- Span waves: one physical span fill for two rules; balanced compound SELECTs handle 700 demand sources in one statement (:768). First impl repeated generated-SQL token scanning: +10.64% CPU (2,234.975 -> 2,472.817 ms), replaced (:772-774). Selected: 9,481 submissions (up from 9,113); two span fills per frontier vs 22; frontier stage counts unstable (1,333 vs 703) due to allocation-address ordering in `Corpus::prepare_files` (:777-789). Frozen-frontier: 22 split fills 108.291 ms vs two broad fills 104.903 ms SQL CPU (:801).

### 2026-10-24 Frontier determinism (:684-752)
- Permutation audit, 139-plan state policy: forward 1,609 stages / 54 broadened; reverse 1,532 / 64: "earlier producer-cache warmth changes admission under the existing 256-node allowance" (:743-746). Fix: each plan lowered under its own 256-node allowance, failed fragment discarded and fallback built fresh; canonical topological numbering; allocation addresses only local handles (:693-698). All orders 1,503 stages / 65 broadened (:689).
- M: 9,481 -> 8,669 submissions (-8.56%); candidate prep 4,836 -> 4,252; cleanup 2,444 -> 2,216; CPU 2,318.369 -> 1,617.488 ms (-30.23%, all selected runs below all baseline runs); SQLite stepping heap 22,828,464 -> 19,793,328 B; candidate rows 135 -> 161 (broader supersets from independent admission) (:710-729).

### 2026-10-24 Shared child demand (:614-682)
- `(fid, pre, field)` transport dedup; `decoded_children` owns answers + completion; missing-coverage join tests `d.fid IS NULL` not `d.child IS NULL` (:620-622). 8,669 -> 7,733 (-10.80%); first frontier 647 -> 415 stages, child fills 246 -> 11; span fills 2 -> 5 (explicit dependencies); temp selector tables 393; peak live 100 -> 93 (:630-638). CPU nine pairs 1,739.994 -> 1,708.247 ms (-1.82%, overlapping) (:642). Replay: VM 81,781 -> 75,068; cache misses 259 -> 180, writes 401 -> 322, spills 282 -> 207; heap 19,793,328 unchanged (:649-653). Edit test preserves findings `2 -> 1 -> 2` (:659). Remaining candidate prep + cleanup 5,532 submissions (~72%) (:666).

### 2026-10-24 Selector SQL owners (:511-612)
- Target shape (red): 393 indexed `(fid, pre)` tables in first frontier, mostly different logical selectors; one SQL owner per compatible ready group with `(producer, fid, pre)` rows and one nonunique index; selectors read their tag via inline projection; UNION ALL preserves duplicates (:594-605).
- Generic family packing REJECTED: 7,733 -> 4,109 submissions but CPU 1,679.669 -> 1,855.333 ms, then 1,682.382 -> 1,789.405 ms (all five selected trials worse) (:568-573). Nested member scopes heap 53,478,480 B vs 19,793,328 baseline; one shared scope 37,930,224; 128-unit preparation bound returns below baseline (:575-577). Costs: lowering work + SQLite SELECT preparation (:579).
- Physical owner projection (green): final `(fid, pre)` CTE streams into its CTAS owner without ephemeral copy; one fewer `OpenEphemeral`; CPU 1,650.576 -> 1,638.666 ms (-0.72%); VM 75,068 -> 73,183; heap unchanged (:515-549). "Packing complete rendered selector trees repeats compiler processing and increases SQLite preparation expansion." (:561-562)

### 2026-10-24 Original execution reduction planning (:490-509)
- 7,733 submissions include 1,588 candidate table creates, 1,588 index creates, 1,588 drops; 1,028 adjacent retirement groups across four contexts. R5-A target 3,474 net removals (<= 4,259). R5-B provisional >= 2,000 (2,130 relational lifecycle executions, 136 clause-configuration PRAGMAs, 68 empty syntax-schema CTAS) (:496-502).

### 2026-10-24 R5-A Candidate ownership delivery (:392-488)
- 7,733 -> 3,685 (4,048 removed). Owner stores typed `(producer, fid, pre)` rows under one nonunique composite index per execution domain; independent frontiers share a bounded SQL population statement and one WITH scope; batch membership records completion even with no rows; each logical producer keeps its own last-use; adjacent releases share one DELETE (:400-405). Lifecycle 4,764 -> 716 (store creation 4, index 4, population 380, retirement 324, drops 4) (:408-417).
- First impl +4.3% CPU (built two schedules before final, scanned discarded alias SQL); fixed; delivery CPU 1,695.593 -> 1,653.416 ms (-2.49%, overlapping) (:442-458). Replay heap 19,793,328 -> 19,269,664 B; VM 73,349 -> 62,073; peak live bindings 93 -> 110 (:469-472). WITH-before-CREATE alias bug (:435-436).

### 2026-10-24 R5-B Relational storage delivery (:315-390)
- 3,685 -> 2,250 (1,435 removed); SQLite internal 994 -> 181 (:318-319). Readers reuse empty typed tables, indexes, views when complete declarations match; input bindings keep separate row lifetimes; "does not infer shared data from equal SQL text"; retirement deletes populated backing at logical last use; empty slots complete with no DELETE; ANALYZE only for populated tables; generic graph construction stops early when no vertices (:325-331). Idle reader pool <= 4; each <= 128 reusable objects, 4 MiB metadata, 16 MiB SQLite+metadata (D) (:333-334). Runtime CREATE 1,192 -> 489, DROP 628 -> 161, ANALYZE 310 -> 39; replacement INSERTs 42 -> 310, DELETEs 0 -> 43; 68 reader connections -> 4 (:346-348).
- Deferred-CAST bug: typed comparison lost finding after storage reuse; fix: compiler syntax declarations match runtime affinity; SQLite infers public deferred projection affinity once per typed schema and query (:357-362).
- CPU 1,546.054 -> 1,541.322 ms (-0.3%: flat); "The engine submission reduction is 38.9%, but it does not deliver a substantial CPU speedup on this guard." Samples: alloc/free, copying, regex search, regex determinization (:377-383).

### 2026-10-24 R5-C (complete shared spec superseded :298-313; bounded C scope :262-296; delivery :173-260)
- Bounded C: exactly four state frontends + one generic nonempty-input precondition; excludes job-wide storage, general derived sharing, compiler-affinity redesign, replacement scheduler (:265-266). Concrete omission: state frontend's 25 typed AST input partitions reconstructing nodes, edges, ownership, staging unions (:270-271). Frontend prototype 2,250 -> 1,489 (761 removed); CPU 1,060.459 -> 427.484 ms (-59.7%) (:276-289). Nine complete empty-gate sessions = 500 engine statements (:282).
- Delivery: 2,250 -> 975 (1,275 removed); SQLite internal 181 -> 106 (:175-177). Four state frontends -> tape-backed syntax relations, removing 25 AST partitions and five reconstruction stages (:184-185). `relational.requireNonemptyInput` names one declared input; complete empty evidence skips SQL reader and body preparation (:190-191). Repository syntax restores only eligible current documents incl. evicted via bounded metadata/source/tape batches; 257 cold sources read with three SQL statements; warm reads need no SQL (:194-196). Table: setup 244 -> 229; candidate prep 702 -> 308; cleanup 165 -> 24; relational 1,091 -> 366 (:202-210). **CPU 1,586.221 -> 550.052 ms (-65.3%); RSS 337,616 -> 298,784 KiB (-11.5%)** (:242-247).

### 2026-10-24 Post-C survey + spikes (:11-171)
- 975 submissions: candidate execution 312, event clauses 202, flow execution 184, index/reader setup 176, compiler inference 53, publication 48 (:123). Adding `requireNonemptyInput: events` to the other 28 clauses: 975 -> 785, CPU 575.665 -> 587.471 ms (+2.05%) (:132-138). Candidate execution has 110 key fills, 66 retirement DELETEs, 20 span fills, 28 child fills; zero-change 74/26/16/22: "Zero changed rows do not prove safe omission." (:146-147). Index setup 168 = 4 x 42 (:152). Flow execution 92 statements twice, 24 SQL-derived relations + one native reachability boundary (:158).
- Combined spike: 975 -> 718; CPU 585.845 -> 579.437 ms (overlapping), RSS ~flat (:20, :49-51). Storage component: fresh DBs 220 submissions / 20.509 ms; clear-and-reuse 100 / 7.803 ms but replicated 71.157 ms and 29,791 internal (FTS deletion work); **private seed clones of immutable schema/catalog** 127 / 18.043 ms / 63.242 ms replicated -> selected (:64-75). Compiler CTAS inference batching 53 -> 31 statements; CPU flat (:78-83). Set folding 272 -> 252; heap increases; string rewriter not promoted (:85-90). First private-cut heuristic (710) removed required owners and interval indexes, losing range bounds and linear nesting (:102-104).
- Final 100 gate still fails by 618 (:22).

### Journal-level takeaways the journal itself states
- Repeated structural fact reconstruction per scope (nodes, edges, ownership, descendants) was the dominant cost in every early profile (code-rule-profiling.md:273-282; journal :3536-3539, :270-271).
- Statement-count reductions often did not move CPU (R5-B 38.9% fewer, CPU -0.3%; :382), while removing whole reconstructions did (shared syntax -71% cold; R5-C -65.3%).
- SQLite planner hazards recurred: affinity loss (:2422-2430, :3260-3264), FTS vocab no document constraint (:2751-2756, :1542-1545), retained-table scans replacing probes (:1616-1625), correlated probe reordering (:1990-1993), key column order (:2338-2343). Each fix used explicit INTEGER affinity, `CROSS JOIN` fences, or key reorder.

