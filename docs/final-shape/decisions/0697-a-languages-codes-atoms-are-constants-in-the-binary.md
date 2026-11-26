# 0697: A language's codes' atoms are constants in the binary

Date: 2026-11-25 (the user's request: "make the tables constants per language, shipped as part of the binary, so that
we don't have to allocate anything for language tables"; the device's reads by "Directory, like atoms"). Follows
decisions 0353 (blocks are never copied), 0631 (format 16's shipped tables) and 0683 (the corpus batch shares the
chunks' projections).

## Intro

A view's `node` row holds its node's kind atom, and a `field` row its field atom. Both are pure functions of the
language: a code's kind atom is `tag * 1024` plus its name's canonical code (ERROR's is the dialect's 0), and its field
atom is `tag << 16 | field`. Until now each document's view entry (`ViewDoc`) pointed at two tables of them, one u32 a
code each. Every document made its own pair in its batch's block, about 16 KB for a Python document's 2,045 codes.

On PostHog's run (57,813 documents) those tables were 900 MB of the 1,173 MB of CHUNK blocks at the peak, 450 MB for
the kinds and 450 MB for the fields. A spike that made one pair per batch and language brought the peak from 5.6 GB to
5.0 GB. Even that is still an allocation per batch of what never changes.

## Decision

1. **The codes' directory is built into the binary.** `rerail-tape-format`'s build script reads the shipped tables
   (`tables/<tag>.tsv`). It writes each language's kind atoms and field atoms into one image. Its first 2 KB hold each
   tag's place (`CodesLang`: the kinds' and the fields' offsets), and the runs follow. The image is 16 KiB-aligned and a
   whole number of pages (`codes_dir`). A code's canonical kind is its name's lookup as a named kind, from the table's
   own `kind` and `lookup` lines (the grammar's `id_for_node_kind`, as `Dialect::new` reads it).
2. **One definition of each rule.** The table's lines (`codes::line`), the atoms (`kind_atom`, `field_atom`,
   `KIND_STRIDE`, `ERROR_SYMBOL`), the code flags and the image are in `tape_format/src/codes.rs`. That file depends on
   `std` alone, and the build script includes it. `f16::Lang::from_text` reads the tables through the same lines. The
   engine's `kind_atom` and `field_atom` are these. A test holds every shipped language's directory equal to its
   dialect's atoms, code by code (`the_codes_directory_is_the_dialects_atoms`).
3. **The entry no longer has the tables.** `ViewDoc` loses `kinds` and `fields` (176 bytes, from 208), and no document
   or batch makes them. The host's view reads read `codes_dir::kinds(tag)` and `codes_dir::fields(tag)`.
4. **The JIT's and the kernels' reads are passed the directory.** A new family of reads, `Codes` (`codes_kind`,
   `codes_field`, written once in CLIF: `<ukl/reads/codes.h>`), reads a tag's run at a code. A view read takes a fifth
   pointer, the directory (`cd` in UKL), and calls them. The views' directory (`ViewDir`) names the codes as one more
   home, `codes` (`2n + 1`). Its host address is the binary's directory. Its device address is a wrap of that same
   memory in place (`wrap_host_memory`: no copy), made once a batch where the device's reads are set up
   (`device_chunk::build_views`) and made resident with the slabs. A device that refuses the wrap is an error, never a
   copy. A layered directory (`views_over`) names the chunk's batch's home.

## Measured

PostHog's `len-_py` run on the M3 (its 29,965 findings each time), two runs each in turns, at loads of 15 to 24:

| | Instructions | Peak footprint |
|---|---|---|
| Each document's own tables (before) | 2,143 G | 5.58–5.61 GB |
| One pair a batch and language (the spike) | 2,116–2,126 G | 4.96–5.00 GB |
| The binary's directory (this decision) | 2,119–2,125 G | 4.99–5.02 GB |

The directory keeps the spike's 0.6 GB, with no allocation at all: its peak and instructions are the spike's, within
the runs' noise. The binary holds every shipped language's atoms, 61,364 codes, in 0.5 MB.

## Not yet run

The Metal reads compile from the generated headers, but no Metal run has been made: Metal runs are paused (2026-11-24).
Metal's no-copy wrap of the binary's read-only pages is what the device side relies on. When Metal runs resume, the
first check is `the_view_reads_are_the_hosts_rows` on a Mac. That test wraps the directory and reads every column on
the device.
