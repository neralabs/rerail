# 0336. The tape reaches every reader as stored: blocks decompressed on the GPU as it goes, full decompression the planner's choice, format 15 retired after a side-by-side run

Status: decided in part (the user, 2026-11-17): §2 and §3 decided, §1 settled by 0156, §4 open but for the header's
starts (built), §5 built. Survey and
scenarios: notes/t42. Builds on 0156 (format 16), 0293 (one tape API, read in place), 0310 (the reads written once
in CLIF for Rust, MSL and the JIT) and 0289 §4 (a document's offsets count from its place in its block).

## Intro

A rule's reads ask a document's tape questions: a node's code, its span, its children, its line. The user's
invariants: the stored tape is written and read as it is, the same bytes on the CPU and the GPU, with no decoding
into another format on the way, through accessors defined once and inlined where a backend can.

On every branch today (t42), four things stand between a stored tape and its readers: a view computed from the
header at load; a 16z tape decoded into format 16 on the host when it is placed on the device; line starts
counted byte by byte on the host at each placement; and a per-document copy of the code table in the device's
form. This decision says which of them go and what replaces them.

## Decision

1. **Line starts and the code table come with format 16.** Settled by 0156: the newline offsets are an
   Elias–Fano index in the tape (§1.6, about 0.19 B a node; a node's line is the rank of its start), and codes
   are numbered in the language's table (items 6–9). Neither is built yet: format 16's writer has nine regions and
   no newline index, and the residency makes line starts and a per-document `DeviceCode` table at placement. Both
   are 0156 work still owed. The placement's line count and per-document table go when it lands. A language's
   table is placed once per batch per language.
2. **The GPU decompresses 16z blocks as it reads; any full decompression is the planner's choice and runs on the
   GPU** (the user: "I want the GPU to be able to decompress blocks as it goes, and for any full decompression to
   be chosen by the planner and done on the GPU (the planner will need to tradeoff CPU and memory depending on the
   job)").
   - A 16z tape is resident as stored. A read reaches a node through its block: a decoded block (the format 16
     bytes of its nodes, as 0156 §2.1 lays it) is read by the same CLIF reads as a whole format 16 tape.
   - Decoding a whole document to format 16 is a plan step, priced and placed like any other: chosen when a job's
     reads touch most blocks more than once, or memory allows it and the job runs again. It is a kernel, not host
     work (and the host's `unblock_into` at placement goes, as "no host work on Metal" asks).
   - The planner weighs device memory (4.5 B a node decoded against 1.2–2.4 compressed), the decode's work (about
     5 ns a node on a CPU core for LZ4), and the job's reads (one pass, or random access across blocks).
3. **Format 15 is retired for documents, after every workload has run on 15 and 16 side by side and every
   difference is fixed** (the user, 2026-11-17). Until then both formats stay, and the comparison is the gate: the
   same finding keys (the loose findings contract), and each workload's costs, for 15, 16 and 16z. `.rr` needs a
   format 16 table first. Not now (the user, 2026-11-17): "I don't want to do it right now, as we still haven't
   proven that format 16 dominates it in all of our required ways". Retiring 15 waits on that proof.
4. **Built in part (2026-11-17): the header holds each region's start.** Format 16's header held each region's
   length (nine u64s), and every reader summed them. It now holds ten u32 starts from the tape's first byte (the
   last is the end), and the slot, gap and length widths (as 16z's did), in 80 bytes where it took 112. A reader
   reads the placement; it checks each fixed region's length against the node count and the widths. A tape past
   4 GiB is not written (the reads' offsets are u32s already). **Still open: whether the stored header is the
   view** (t42 axis A). Today `TapeView` is built from the header at
   load (a prefix sum of the regions' lengths and the language's numbers). The alternative stores the region
   starts in the header in the layout the reads take, the language's numbers stay the language's (placed once
   with its table, checked by the revision mark), and the batch's per-document entry holds the batch's own facts
   (where the document lies, its first byte and line in the batch's numbering).

5. **16z is 0156 §2.1's blocking (2026-11-17).** It compressed all nine regions of a block's nodes as one LZ4
   block, checkpoints too, and a full decode scattered each block's regions into the tape's. Now:
   - a 16z tape is the header and part starts of the format 16 tape it decodes to (its magic apart), its
     checkpoints raw, a piece index (a u32 start and a kind byte a piece), and the payload;
   - a block is four pieces, one an array (codes, slots, gaps, lengths), each the block's bytes of the array and
     then its side list's part for the block, so one decode gives both;
   - a piece is stored raw, as one LZ4 block (HC 9) when that shrinks it by a tenth, or as a repeat (the index of
     an earlier piece with the same bytes, at least 8 of them);
   - a block decodes in place, alone, into the format 16 tape (`decode_block_into`), whose head (header, part
     starts, checkpoints, rooms) is copied once (`decode_head_into`).
   Stored tapes are keyed by layout (`F16_LAYOUT` = 2: keys 216 and 1216, were 16 and 1016), so a store built before
   reads cold. No dictionary yet (0156 §2.3). §2's kernel and planner step are Q65; §1 is Q66; §3's gate is Q67.

6. **Format 16 holds its newline index (2026-11-17, §1's first half).** Region 9, after the side lists, is 0156
   §1.6's Elias–Fano index of the source's `\n` offsets (`tape_format::newlines`): a header of five u32s, a sample
   (word, count before) of every 256th zero and one of the high bits, the high bits, then the low parts. The
   header holds the newline count (byte 12). A byte's line is the newlines before it (`tape_line`: the bucket's
   bounds by select on the zeros, then a scan of its low parts); line `k` starts one past newline `k - 1`
   (`tape_line_start`: select on the ones). Both are CLIF reads, so the CPU, the JIT and Metal run the same body;
   format 15 answers them from its line-start array. The writer takes the source, not its length. 16z stores the
   index raw after its checkpoints; a block decoded alone carries none (the whole tape's view answers lines).
   Stores are keyed by layout 3 (316 and 1316). The residency places no line starts for format 16: the view's
   `lines` is the tape's index, and Metal's batch runtime (`LineTable`: a node's line, a byte's line, a row and
   column's byte) answers through the same reads. Format 15 keeps its placed line starts.

7. **A language's code table is placed once a slab (2026-11-17, §1's second half).** A format 16 document's codes
   are its language's table (`DeviceCode`s, made from the language's numbering and the grammar). A slab holds it
   once per key (the tape's tag and the table's revision), placed after the first document of the language that
   lands there (`rerail_metal::slabs::Slab::table`), and every format 16 document's view points at it: `codes` is
   the table's offset from the document's `at` modulo 2^32, so `tape_code` reads `chunk + (at + codes)` whether
   the table lies before or after it. A batch gathered into one block copies a language's table there once
   (`gather`), and the copies' views point at it. Format 15 keeps a table a document. The residency's derived bytes
   for a format 16 document are now none: no codes, no line starts.

8. **The device decodes 16z's pieces (2026-11-17, §2's first step).** The residency no longer decodes a 16z tape
   on the host. Where it is placed, the host copies its head (header, part starts, checkpoints, newline index:
   stored as they are, and what the placement's checks read), and every piece of the admission's 16z documents is
   decoded on the GPU in one command before any document is published (`residency/slabs/decode16z.metal`): one
   SIMD group a piece, E26's shape, straight into the slab (no threadgroup room, so no piece is too long), each
   read and write bounds-checked, a piece that does not decode to its length refusing the admission. A repeat is
   decoded from the earlier piece's stored bytes (`Tape16Z::piece_jobs`). The device's bytes are checked equal
   to the host's decode. What is not built yet: a 16z tape resident as stored with blocks decoded as a job's reads
   reach them, and so the planner's choice between that and the whole decode (Q65). Today every 16z document is
   decoded whole, on the GPU, when placed.
   - **Superseded (2026-11-18, the merge of Native-and-GPU-strata into oscar):** decision 0353 step 4b builds
     what this item left open, and goes further: a 16z tape is resident as stored, and every reader (the CPU, the
     JIT and the kernels) decodes a block where a read falls, into a rolling cache. The whole decode on placement
     is a copy (0353 row 15), so it is gone: `decode16z.metal`, `Decodes` and `Tape16Z::piece_jobs` are deleted.
     The cache's decoder (`z_decode`, CLIF) decodes 0434's four pieces a block.

## Consequences

- The roll-up of 2026-11-17 merges the branches' views as they are (A1, a view computed at load), declared once.
  §1–§3 are work queue items, not merge steps.
- 16z's block index and a decoded block's layout become what the reads address through: whichever answer §4
  gets, a view says where a node's bytes are, whole or by block.

## Kind tests on the language's table (2026-11-17)

On format 16 a document's codes are numbers of its language's table (§7), about 140k entries for Python. Format
15's table is the document's own, so a kind test of a document without the kind found an empty set and stopped.
On format 16 every test scanned the document, and each built its set from the whole table. After the merge of
Native-and-GPU-strata, the B-now rules on 10 Python files took 1,150 G instructions on format 16 against 135 G on
15. What the reads do now:

- **The table indexes its codes** (`CodeTable`'s index, made once per table): by kind (those without the ERROR
  flag), by class, ERROR, MISSING, and the two together, each as the `CodeSet` a test reads. A test borrows it.
- **A document holds a summary of what it holds** (`Facts::held`, on the first kind or class test): its codes,
  their kinds and their classes, ascending. This is per document, not per node. A test of a kind or class the
  document lacks is one binary search and no scan, as on format 15.
- **A scan reads codes past the cutoff in order.** Below the cutoff a code byte is looked up in a 256-entry table,
  built from the set's members. Past the cutoff the scan walks side list 0's entries for each part
  (`f16::part_codes`): each entry once, in node order, instead of a search for each node. A part with no entries
  is no bytes and is skipped.

B-now on 10 files is now 113 G on format 16 against 110 G on 15. On 50 files it is 317 G against 316 G, with the
same 213 findings on 15, 16 and 16z, cold and warm.
