# 0324. The grid machine reads the tape

Status: building (2026-11-16). Step 5 of 0293, after Metal's packed interpreter and JIT (5e91e374d). Under the
user's terms for the track: "I want the kernels to operate over the tape directly, either compressed or
decompressed, and format 15 or 16. There must be no intermediate 'decompress to an array of nodes' step."

## Intro

Metal runs a batch of programs in one of three ways. Two of them, the packed interpreter and its JIT
(`metal_program_backend`), now read each document's tape in the slabs. The third is the grid machine
(`ExecutionBackend::Metal`: `metal_grid`, the register stages, the frontier, successor, closure and terminal
kernels). It still reads a decoded copy of every document, built on the host for every batch
(`metal_register_execution::upload`), which the kernels read through `metal_resident.metal`'s `View`:

- **`nodes`:** a 44-byte record per node (`BatchNode`: parent, next sibling, next named sibling, child range,
  subtree end, span, kind, field, flags).
- **`children`:** each node's children, listed.
- **Tables derived from those two:**
  - `relations`: each parent's first and last named child.
  - `fields`: the (parent, field) pairs present.
  - `supertypes`: the (node, class) pairs.
  - `named_order`: each node's place among its named siblings, and its preorder.
  - The child index's `field_nodes` and `kind_nodes` (`metal_child_index`): children sorted by parent and key,
    for `unique_child`.
- **`source` and `line_starts`:** the documents' bytes and lines, copied once more.

Every one of these is a node array, which the user's terms exclude.

## Decision

1. **The documents are the slabs'**, as for the packed interpreter: `admit_docs`, then `DocTable`. The grid binds
   the doc views and the slabs where its image held the records. The image keeps the batch's own tables (roots,
   selectors, spans, blocks, programs, strings, values and the derived classes' bits) and loses every node table.
2. **A node is a handle.** `v.nodes[g]` becomes a handle holding the node's document and its local number: the
   document is found once, from the batch-global number (a binary search over the doc views' node bases). Every
   field and every hop then reads that document's tape through the generated reads (0310), and stays in it. Node
   numbers stay batch-global wherever kernels store them (frontiers, captures, results), so no buffer's layout
   changes.
3. **Children are hops.** A loop over `children[child_start + k]` walks the children from the first one by
   sibling hops (`tape_next_in`); one child at a known place is `tape_child`. No loop asks for the k-th child k
   times.
4. **The derived tables are reads:**
   - **First and last named child:** hops (the last is a walk over the children).
   - **(parent, field) present and `unique_child`:** a walk over the parent's children comparing codes.
   - **Classes:** the code's classes (`tape_has_class`), as the packed interpreter reads them.
   - **Named order:** the sibling place by hops, and the preorder is the node's number.
   - **Source and lines:** the slab's (`DocView.source`, `DocView.lines`).
   A table's lookup becomes a walk over one parent's children; that cost is measured with the rest of step 5,
   not before.
5. **Checks move with the data.** `metal_grid_contract` and the child index's `validate` check the records the
   host built, so they go with those records. The tape's own checks run when it is admitted to the slabs
   (`residency::slabs`, its SHA-256 and header checks). The grid conformance tests that corrupt the records
   (`metal_register_conformance`'s malformed batches) go too: those records no longer exist.

## Order

1. **The handle and the image.** `metal_resident.metal` reads nodes through the handle, and the upload drops the
   node tables. Kernels keep compiling, through the handle's fields.
2. **The kernels one family at a time:** register stages, frontier, successor, closure, terminal, output. Each
   family's child loops become hops, and its derived-table reads become tape reads. Each family's tests run on
   the device (`metal-tests`) before the next.
3. **The code generators** (`metal_register_codegen`, `metal_successor_codegen` and the others) emit the same
   reads.
4. **The host's builders go:** `PackedBatch`'s `Layout::Packed`, `metal_child_index`, `metal_named_order` and the
   grid contract. No caller of `PackedBatch::new` is left.

## Progress

- Not started.
