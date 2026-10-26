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

