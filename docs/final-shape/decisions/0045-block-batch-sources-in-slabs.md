# 0045: A batch's files are read in place in residency slabs

Status: accepted (user, 2026-11-07: the list item "Binding residency slabs as the source
buffer; today the batch copies files into one leased block")

## Intro

A block-program batch (decision 0044) reads each file's bytes through a file table of offsets.
Until now the host copied every file into one new device buffer per launch. A wave parses the
same files again and again: under each language's program, after an edit elsewhere, after a
fallback. This record makes the bytes resident: copied once into a slab, then read in place by
every later launch.

```
before:  files --copy--> one leased block per launch --> kernel
after:   files --copy once, by hash--> slab (resident) --> kernel binds the slab
```

## Prior work consulted

- **ast-index-spike indexes:** the spike ran no GPU experiments (prior-work `README.md`, "Known
  differences"). Its mldb tape row ("per-entry tape over the source buffer, no text copy")
  **agrees**: the reader works over the source where it lies.
- **Decision 0032 (slabs):**
  - **Agrees:** anonymous-mapping slabs, each with one retained no-copy `MTLBuffer`, a 16-byte
    bump, retirement of a whole slab, the two-minute drop, at most `MAX_SLABS` (four) slabs a
    batch.
  - **Agrees with its evidence:** a first-use Metal mapping cost 213 ms against 6.1 ms of GPU
    time; retained buffers cut the wait 30–60×. A buffer per launch is what this record removes.
  - **Departs:** 0032's resident unit is a stored tape and its source, keyed by the tape's hash.
    A parser's input has no tape yet. So a second kind of entry, a source alone, is keyed by the
    source's SHA-256. It shares the slabs, the budget, the pools (`source.resident`) and
    retirement with the documents.
- **Lessons ledger #26** (a fixed budget does not scale). **Agrees:** a batch that spans more
  than four slabs is refused with `BackendCapacity`, and the caller splits it, as a label batch
  is split. It is never a silent partial result.
- **Lessons ledger #10** (measure the code that is running). The admission is counted
  (`residency.source_hits`, `residency.source_misses`), so a run shows whether it copied.

## Decision

1. **`residency::slabs::admit_sources(files)`** returns one `ResidentSource` per file, in order.
   - A hit is found by the bytes' SHA-256, and its slab is marked used.
   - A miss is copied once into the newest slab with room, at a 16-byte offset, taking at least
     16 bytes, so a 16-byte read of an empty file stays inside it.
   - The slabs of every source asked for are kept while the misses are placed.
2. **`source_slabs(sources)`** gives the slabs a batch binds and each source's binding, or
   `None` past `MAX_SLABS`.
3. **`run_batch` binds the slabs** at buffers 0, 16, 17 and 18. An unused binding repeats the
   first. A file's row carries its slab and its offset in 16-byte units.
4. **The kernels pick the slab per group.** The workers' loop chooses the pointer from the
   file's row, the same in every lane. The program functions read `bytes` as before.

## Not decided here

- When a file's tape is admitted later, its document copies the source again. It could
  reference the resident source instead.
- Hashing every file on every launch costs one SHA-256 pass. The engine already knows a file's
  digest from the store; the API could take it.
