# 0336. The tape reaches every reader as stored: blocks decompressed on the GPU as it goes, full decompression the planner's choice, format 15 retired after a side-by-side run

Status: decided in part (the user, 2026-11-17): §2 and §3 decided, §1 settled by 0156, §4 open. Survey and
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
   format 16 table first.
4. **Open: whether the stored header is the view** (t42 axis A). Today `TapeView` is built from the header at
   load (a prefix sum of the regions' lengths and the language's numbers). The alternative stores the region
   starts in the header in the layout the reads take, the language's numbers stay the language's (placed once
   with its table, checked by the revision mark), and the batch's per-document entry holds the batch's own facts
   (where the document lies, its first byte and line in the batch's numbering).

## Consequences

- The roll-up of 2026-11-17 merges the branches' views as they are (A1, a view computed at load), declared once.
  §1–§3 are work queue items, not merge steps.
- 16z's block index and a decoded block's layout become what the reads address through: whichever answer §4
  gets, a view says where a node's bytes are, whole or by block.
