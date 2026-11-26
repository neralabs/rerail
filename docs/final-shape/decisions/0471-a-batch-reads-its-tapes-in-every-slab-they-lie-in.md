# 0471. A batch reads its tapes in every slab they lie in, and the device joins' caches share one cap

Status: built (audited 2026-11-23). Accepted (2026-11-19). Follows 0465 §3 and 0476 (Q334's third item).

## Intro

0465 found the large program's label runs on all (3,089 files, 9.6 M nodes) to be 283 runs with a median of 6 files,
where s1000's were 6 runs with a median of 36. Each run pays a run's fixed costs. 0465 also saw a cold Metal run of
all exit 1. The user: "please do all 3. Our parsing should be lightning fast".

Two causes, both of the Metal budget's holders:

1. **A wave was split until its files shared a slab.**
   - A loaded tape's one home is a residency slab (0353 step 4a). Tapes go into the newest slab as they load, so a
     chunk of 262,144 nodes (about 7 MB of tapes) mostly straddles a slab boundary (a slab is 64 MB).
   - A batch read one block (0289 §4). A batch over two slabs was copied into one (`gather`, a listed copy: 0353
     row 13). With the residency's share of the budget full of tapes' homes, which cannot be retired while their
     tapes live, the copy found no room and refused.
   - The wave then halved the batch (`roots_metal_split`) until each half lay in one slab: about 4.5 runs a wave.
     The profile named it once `capacity()` named its file: `capacity.function.rs:250` 118–127 times a run, that is
     `ok_or_else(capacity)` after `DocTable::one_block`.
2. **The device joins' caches had a cap per thread.**
   - `setfix::device::Resident` keeps relations on the device, each thread its own, up to 80 MB (at 512 MB). Two to
     four threads held 127 MB of the 512 MB budget, which nothing else can take back.
   - With the tapes' slabs (256 MB) and the rows on the device (112 MB), a wave halved down to one file was refused
     too, and the run stopped (`ast_wave.rs`: "the ast matcher wave failed on Metal: BackendCapacity"). That was the
     cold run's exit.

## Decision

1. **A batch reads its documents where they lie, in as many slabs as they are in** (0353 step 4's plan: "a
   document's descriptor names its slab and its tape's offset there, and each backend gives the reads the slabs'
   bases").
   - `DocTable::new` binds the first document's slab as the block and lists the batch's other slabs
     (`DocTable::slabs`).
   - A view's `at` and its new `at_high` are an i64: the document's first byte from the bound block. For a document
     in another slab it is the slabs' GPU addresses apart plus its place there, as the 16z cache is already reached
     (`side`).
   - The device's one read of it, `tape_doc` (`device_tape.h`), adds the i64. `tape_code` takes the table's `codes`
     from the document as an i32, so a table before the document is reached as before (0336 §7).
   - Every encoder that binds the block makes the other slabs resident (`use_resource`, read): the label kernels
     (`make_resident`, with the 16z cache), the interpreter's runs, and the packed program batch.
   - `gather` and `Resident::copied` are deleted: 0353 row 13 loses its copy between slabs.
   - The view is 124 bytes (31 u32s). `ViewRow` stays 184 bytes.
2. **The device joins' caches share one cap, every thread's together.** `HELD` counts the bytes of every thread's
   `Resident`. A thread's `trim` counts the others' bytes against its room. A thread's cache leaves the count when the
   thread goes. The Metal budget is the device's, one for the process, as the buffer pool's ledger is.
3. **A Metal wave that fails still stops the run** (0382 §6). A CPU wave in its place for a one-file refusal was
   tried here and removed: the cause is fixed instead.
4. **`capacity()` names its file and line** (`capacity.<file>:<line>`), and is called in a closure: passed as a
   function, `#[track_caller]` saw core's `FnOnce::call_once`.

## Measured

All, `internal_dead_code`, Metal pinned to the GPU, an M1 Max farm host. The rows were ed502024 (native's) in every
run.

| | Before (the cap only) | After |
|---|---|---|
| label runs (`label_parents` encoders) | 197–232 | 78 (one a wave) |
| waves split (`capacity.*`) | 118–127 | 0 |
| dispatches (`run.launches`) | 7,209 | 1,878 |
| `label_phase` | 403–451 ms | 193 ms |
| the GPU's time | 932–1,051 ms | 439 ms |
| instructions | 429–453 G | 403 G |
| wall | 18.4 s at load 11; 23–36 s at load 18–38 | 16.6 s at load 5 |

- Two cold runs of the new binary passed (rc=0) once the caches shared their cap. Before it, 0465's cold run and this
  session's first pinned runs on all exited at the panic above.
- Tests: `the_device_reads_every_format_as_the_document_does` runs every format twice, as admitted and over two slabs
  (a source past a slab's size admitted between the first document and the rest), and compares every node's 14 reads
  on the device with the document's. `a_batch_over_several_slabs_reads_each_document_where_it_lies` holds that
  nothing is copied or moved, and that each view counts from the bound slab. The Metal wave tests
  (`compile_matcher_tests`, `compilation::tests`) and `rerail-clif`'s pass.

## What is next

Q334's first two items: the parse workers' CPU (W's reference graph, 0476's next steps) and the run's serial phases.
