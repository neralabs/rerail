# Batched producer execution contract

SQL owns rule decisions, joins, inference, and diagnostics.
Structural producers operate on immutable packed data through an explicit batch boundary.
This contract applies to the CPU backend and every future GPU backend.

## Required shape

1. Collect document, field, owner, and structural demands across ready files and clauses.
2. Prepare each exact document version once and freeze its numeric columns and posting ranges.
3. Advance SQL consumers until their next structural dependency.
4. Gather all ready requests into one producer wave.
5. Count results, scan counts, and scatter numeric output columns across the complete wave.
6. Publish only completed producer relations, then resume dependent SQL consumers.

Virtual-table callbacks only read prepared data and CPU indexes.
They cannot dispatch producers, submit device commands, or wait for device completion.
Source text remains an immutable side array with checked byte ranges.
Paths and grammar names stay in host metadata rather than repeated device records.

Node identities retain document version and file qualification.
Output order uses logical request identities rather than physical execution order.
Each requested relation receives an explicit completion or failure outcome.
Failed producers expose no partial rows and cannot prove absence.
One failed relation cannot spend another relation's work allowance.
Suspension retains work counters and cancellation state.

Resident input and result capacity determine execution partitions.
SQL insertion batches, publication pages, file boundaries, and exact signatures cannot determine dispatch boundaries.
Publication consumes completed output without replaying semantic execution.
Every partition shares one evaluation stage allowance.
A backend must reject capacity or stage exhaustion explicitly.

## Enforcement

For fixed dependencies and sufficient capacity, 100 times more independent roots or files must retain the same producer stage count.
Thread or row counts increase with input volume.
A genuinely dependent producer requires a subsequent wave.
Repeated SQL scans and changes to insertion batch size cannot create more producer stages.
Tests cover source gates, mixed files, edits, cancellation, failures, and zero-result completion.

CPU stage traces describe executed CPU passes.
They do not claim GPU launches or establish GPU throughput.
A future GPU backend must record actual kernel launches, command submissions, and intermediate readbacks separately.
It must preserve GPU-owned counts between dependent kernels.
General recursive inference can require additional semantic waves.

## Provenance

Landing NER-1562 at `ce7d2da32` supplies the execution-shape reference.
Its broad classes, homogeneous signature blocks, compact frontiers, and shared-command counts inform this contract.
Its dynamic production integration remains incomplete.
Rerail does not import Landing's complete rule engine or persisted tape ABI.

## Current CPU implementation

The workspace driver collects dirty relational work across policies, clauses, and files.
Stateless evaluation collects all relational partitions in the requested policy.
SQL sessions retain their connections, stage positions, work counters, and diagnostic anchors across suspension.
Existing file-local result caches retain their exact document-version keys.
Workspace eligibility witnesses remain gates rather than projected input relations.
Preparation extracts the full declared inputs before consumers run.

The syntax backing uses fixed-width node columns, field ranges, owner columns, and CSR postings.
Names and source text remain host dictionaries and immutable source storage.
The producer packs only the columns required for subtree execution.
It shares document columns and visible-node selectors across all ready relations.
Requests, counts, offsets, and result columns use numeric identities.

The CPU backend performs one count pass, one segmented scan, and one scatter pass for each ready wave.
A completed output owns the result columns until SQL consumes them.
SQL insertion and index construction remain publication work.
An empty successful relation still receives a completion token.
Only a successful token permits the dependent SQL stages to advance.

The producer arena permits 64 MiB of packed input and 256 MiB of result columns.
The complete batch permits 100 producer passes across dependent waves.
Oversized waves currently fail explicitly. Automatic memory partitioning is not implemented.
SQL session memory remains separate from the producer arena and existing document-cache reservations.
These limits describe the CPU reference boundary, not measured GPU allocations or kernel launches.

## Required check

`npm run test:producer-shape` runs the execution-shape gate with a profiling-enabled native build.
The gate fails if profiling support is absent, rather than silently skipping its assertions.
The Rust test job builds a separate profiling addon and runs this gate in CI.
It covers 100 times more roots, files, and policies, independent insertion sizes, and genuinely dependent waves.
A dependent wave must reuse all resident document input.
The normal native suite also checks complete descendant rows against the SQL forest oracle.
Empty completion, peer failures, Unicode, exclusions, edits, and source gates retain their existing contracts.
Failed or interrupted outputs never enter the file-result cache.

## Recorded results

The [evidence](../fixtures/host/data-layer-port/profiling/batched-producer-evidence.json) records the source hashes, binary hashes, shape receipts, parity checks, and matched measurements.
The production five-file workload issues 78,911 subtree requests across six relations in one three-pass wave.
It prepares five syntax documents once.

Three matched rounds compare commit `c4bd76bf` with this change using identical policies, input, compiler settings, and allocator.
The profiler's `--allow-engine-changes` option permits the intentional binary change while retaining workload, policy, and parity guards.

| Median | Before | Batched producers |
| --- | ---: | ---: |
| Cold process CPU | 2.898 s | 2.553 s |
| Edit process CPU | 1.192 s | 1.036 s |
| Peak process RSS | 572 MiB | 652 MiB |

The combined storage and scheduling change reduces cold CPU by 11.9 percent and edit CPU by 13.0 percent.
Peak RSS increases by 13.9 percent because the driver retains more ready SQL sessions.
These results do not isolate batching from the packed-storage change.
CPU is the primary timing comparison on this shared host.

All 406 native tests and 52 Python tests pass.
All 200 upstream corpus cases, 219 production comparisons, and 57 workspace transitions pass.
Clippy covers production and profiling configurations.
The source contract retains file-local SQL semantics through separate sessions; producer execution coalesces their ready requests.

Numeric kind slots belong to each document's host dictionary.
A future accelerator adapter must bind those slots to its verified grammar catalog during preparation.
It cannot reinterpret them as another executor's persisted kind codes or tape ABI.
