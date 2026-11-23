# 0656: A tape's sidecar: its names, vocabulary, places and counts, read in place in Rust and in UKL

Status: decided (the user, 2026-11-23: "If we took an interim "sidecar" before changing the tape format, what would it
look like in its most optimal form? … cheap to build, use little memory and buildable on a GPU via a ukl kernel, and
include the names stream idea, and include enough of the indexes that we could answer the rules that were active from
it. It should also be extensible …"; of the design page that answered it: "okay please go ahead make sure it's done in
a manner that we can read and look it up in both Rust and UKL").

## Intro

**Why.** Since t54, a batch makes rows to answer questions the tape cannot answer alone. The worst is `text` by a
string, about 26 s on PostHog: the relation is routed by node, so a lookup by string makes the whole chunk first. Next
come node rows, places and counts. Three of these are format changes that 0156 designed but never built:

- §6's names stream;
- a sibling ordinal on every 8th sibling;
- per-document code counts beside presence.

The rest need an index the format will never hold: text to nodes.

**What.** A sidecar is one per tape, a pure function of the tape and its source, kept beside the tape as presence is,
and read in place. The rules choose which sections a run reads, never which keys a section holds: the SQL index
prototype's rule, "never key postings to known rules".

## The decision

1. **The format** (`tape_format::sidecar`):
   - a header (magic, version, nodes, leaves, blocks, terms, the tape's digest);
   - a directory of sections (kind, version, parameters, offset, length);
   - each section 16-byte aligned, after four u32s of its own.

   A reader skips a kind it does not know, and answers `NONE` where a section is missing. The sections:
   - **LEAVES**: a leaf bitmap per 256-node block, and the leaves before each block. A node's leaf rank, and a
     rank's node.
   - **NAMES**: a byte per leaf, its term in the document, the most frequent first. 255 escapes to a per-256-leaf
     run of (leaf, term).
   - **VOCAB**: each term's 64-bit hash (`text_hash`) and its first occurrence's span in the source, so no text is
     copied. Then the terms in hash order.
   - **SIBLINGS**: every 8th child of a family, as (parent, node).
   - **COUNTS**: (code, count), by code.
   - **RARE**: the nodes of each code with at most one node in 256, by code.
2. **One set of reads, written twice and held equal.**
   - Rust: `sidecar::Sidecar`, plus `place` and `child` over a `Tape`.
   - UKL: `<ukl/reads/sidecar.h>` (`ukl_sc_*`) and `<ukl/reads/sidecar_tape.h>` (`ukl_sc_place`, `ukl_sc_child`,
     over the tape's `clif_tape_*`).

   `rerail-ukl-vec`'s `the_sidecars_ukl_reads_are_its_rust_reads` runs every UKL read on the CPU's vector path,
   interpreted and compiled, against the Rust read. It covers every node, leaf, term, code and section of a format
   16 tape with escaped terms, families up to 300 wide, and rare codes. The headers are in the UKL library, so every
   backend's preprocessing and the OpenCL gate check them.
3. **Not a copy (0353).** A sidecar is an index derived from the tape, written once where it is computed and loaded
   as its exact bytes. Nothing reads a row of it that the tape holds: a term is a span of the source, a place is
   ≤ 7 steps of the tape from a stored eighth.

## Steps

1. The format, the Rust builder (`sidecar::build`, over any `Tape`), the reads in Rust and UKL, and the tests that
   hold them equal. Done here.
2. The builder in UKL, on the device and on the CPU's vector path: one plan per batch of tapes, with a bounded
   number of launches. Its passes:
   - nodes: counts, leaves and presence;
   - families: the 8th siblings;
   - leaves: hashes;
   - a radix sort for the terms;
   - a second sort for their frequency order;
   - a compaction for rare codes.

   Its bytes are held equal to `sidecar::build`'s.
3. The store keeps the sidecar beside the tape, keyed by the tape's content and the sections' versions, and makes a
   missing section on demand.
4. The engine reads from it: `text` by string and by node, a child's place, kind counts, and rare kinds. The rows
   these replace are removed, and so is the work that made them.
