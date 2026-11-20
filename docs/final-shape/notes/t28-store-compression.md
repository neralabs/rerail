# T28: store compression, zstd dictionaries and a suffix-array bound

## Intro

The store compresses every source and every tape as its own zstd
frame at level 3, with no dictionary. A frame knows nothing about the
files around it. So text that appears in nearly every file is paid for
again in each frame: `from __future__ import annotations`, a tape's
code table, the start of every code column. For small files this
repeated text is most of the frame.

A zstd dictionary is a block of bytes that both sides know in advance.
The compressor can copy from it as if it came before the file. This
note measures four things:

- where the store's bytes actually go;
- what three kinds of dictionary save per language, for sources, full
  tapes and named-only tapes (RER-131);
- an upper bound on what any dictionary could save, from a suffix
  array over many files;
- what it would take to ship this, and which wins are easy.

## 1. Where the store's bytes go

`dbstat` over three real stores on this machine, in MB:

| Store | File | Sources | Tapes | Largest other table |
| -- | -- | -- | -- | -- |
| Python repository, 179k documents | 404 | 39 | 91 | `atom_segment` 239 |
| SQL-heavy repository, 3.7k documents | 430 | 101 | 83 | `facts_results` 216 |
| Rule-test store | 53 | 3 | 3 | `facts_results` 44 |

- Sources and tapes are 11–43% of a store. The single largest table is
  never sources or tapes: it is `atom_segment` or `facts_results`. That
  is where most of the storage cost is, and it is outside this note.
- In the Python store, 152k of the 179k tapes are under 512 bytes.
  zstd shrinks those only from 46 MB to 28 MB (1.65×), because a small
  frame has no history to copy from. These are the frames a dictionary
  helps.
- Every tape frame carries the 32-byte SHA-256 of its source, and a
  hash cannot be compressed. The store already holds that hash in
  `hashes`. Across 179k tapes this is 5.7 MB, about 7% of the
  compressed tapes.

## 2. Dictionaries, per language

Method:

- Files alternate between a training half and a test half. Sizes are
  over the test half only.
- Frames are written the way the store writes them: level 3, no
  checksum, no content size. There is no dictionary id either, since
  the store would keep that in a column.
- The kinds of dictionary:
  - **hand**: raw content, written by hand. For sources it is the
    language's keywords and operators plus a hardcoded footprint of its
    standard library. For tapes it is a header template plus the code
    tables of the training tapes.
  - **trained**: `zstd --train` on the training half, at 16 KB and at
    110 KB.

Size of the test half, compressed, as a fraction of plain zstd:

| Data | Plain zstd | hand | trained 16 KB | trained 110 KB |
| -- | -- | -- | -- | -- |
| Python store, sources | 1.89 MB | 1.01 | 0.89 | 0.85 |
| Python store, tapes | 4.66 MB | 0.92 | 0.80 | 0.79 |
| Python, sources | 1.74 MB | 1.01 | 0.91 | 0.87 |
| Python, full tapes | 3.44 MB | 0.97 | 0.95 | 0.92 |
| Python, named-only tapes | 2.81 MB | 0.97 | 0.95 | 0.93 |
| TypeScript, sources | 1.36 MB | 1.01 | 0.91 | 0.87 |
| TypeScript, full tapes | 2.83 MB | 0.97 | 0.95 | 0.93 |
| TypeScript, named-only tapes | 2.20 MB | 0.97 | 0.95 | 0.93 |
| JavaScript, sources | 0.37 MB | 1.00 | 0.92 | 0.87 |
| JavaScript, named-only tapes | 0.59 MB | 0.97 | 0.95 | 0.93 |
| Rust, sources | 1.72 MB | 1.02 | 0.95 | 0.92 |
| Rust, named-only tapes | 3.05 MB | 0.98 | 0.98 | 0.96 |
| PHP, sources | 0.079 MB | 0.99 | 0.85 | 0.77 |
| PHP, named-only tapes | 0.132 MB | 0.95 | 0.91 | 0.87 |
| SQL, sources | 8.88 MB | 0.89 | 0.95 | 0.68 |
| SQL, named-only tapes | 2.20 MB | 0.97 | 0.87 | 1.23 |

The Python store sample is one tape in nine, taken at random. The other
rows use the fixture corpora: Rerail's own files and the data-layer
repository.

What the table says:

- **Hand-written keyword dictionaries do almost nothing for sources**
  (0.99–1.02×). A keyword is a few bytes, and zstd only gains from a
  copy longer than about 4–6 bytes. A source already has its keywords
  earlier in the same file. SQL is the exception (0.89×), because its
  files are long runs of the same few statements.
- **Trained dictionaries work**: 8–32% for sources and 2–20% for tapes.
  The biggest gain is on the store's many small tapes (0.80×).
- **Size matters by file size.** Ratios for the 110 KB dictionary, by
  the file's raw size:

  | Data | < 1 KB | 1–8 KB | 8–64 KB | ≥ 64 KB |
  | -- | -- | -- | -- | -- |
  | Python store, sources | 0.73 | 0.80 | 0.90 | 0.97 |
  | Python store, tapes | 0.57 | 0.81 | 0.93 | 0.99 |
  | Python, sources | 0.47 | 0.74 | 0.90 | 0.95 |
  | Python, full tapes | 0.60 | 0.76 | 0.92 | 0.98 |
  | SQL, named-only tapes | 0.76 | 0.96 | 1.02 | 1.40 |

  Under 1 KB, a dictionary removes 25–55% of a frame. Above 64 KB it
  saves nothing, and for SQL tapes a large dictionary makes the frame
  40% bigger: the compressor tunes its parameters for the dictionary,
  not for the large input.
- **Dictionaries carry across repositories.** A dictionary trained on
  the fixture corpora, used on the Python store, saves 0.84× on tapes
  (0.80× with the store's own) and 0.86× on sources (0.85× with its
  own). Tape dictionaries learn the grammar's code table, which is the
  same in every repository. A per-language dictionary can therefore
  ship with the binary, and no training is needed in the store.
- **Named-only tapes need their own dictionary.** The full-tape
  dictionary gains only 0.97× on them.
- **Named-only and a dictionary add up.** For Python, named-only tapes
  with a 110 KB dictionary come to 0.75× of today's full tapes.
- **Speed:** with or without a dictionary, decoding the test halves
  took the same CPU time to within noise. That was 0.1–0.3 s for
  thousands of frames, and process start-up was most of it.

## 3. The bound: what any dictionary could reach

A suffix array over about 3 MB of files per language gives two lengths
at each byte position:

- the longest match elsewhere in the same file, which a single frame
  can already use;
- the longest match in any other file, which only a dictionary can
  supply.

A greedy parse with matches of 8 bytes or more then counts the bytes
covered using the same file alone, and using the other files too. The
difference is the most a dictionary could ever add in coverage, with
an unlimited dictionary that holds every other file. It is a bound on
coverage, not on compressed bytes. The program is
`scratchpad/compress/sa.c`.

| Data | Covered in the file | Plus other files | Gain |
| -- | -- | -- | -- |
| Python store, sources | 84% | 96% | 12 points |
| Python store, tapes | 70% | 93% | 23 |
| Python: sources / full / named-only | 90 / 84 / 73% | 97 / 95 / 92% | 8 / 11 / 19 |
| TypeScript: sources / full / named-only | 80 / 84 / 69% | 95 / 95 / 92% | 15 / 11 / 23 |
| JavaScript: sources / full / named-only | 78 / 85 / 72% | 94 / 96 / 93% | 17 / 11 / 21 |
| Rust: sources / full / named-only | 85 / 87 / 72% | 95 / 95 / 92% | 10 / 8 / 19 |
| PHP: sources / full / named-only | 73 / 80 / 68% | 92 / 94 / 90% | 19 / 14 / 22 |
| SQL: sources / full / named-only | 97 / 88 / 90% | 100 / 95 / 96% | 3 / 7 / 6 |

- **Named-only tapes leave twice the headroom of full tapes**: 19–23
  points against 8–14. Dropping the tokens removes repetition inside
  the file, and what is left is mostly shared structure between files.
- **Trained dictionaries reach part of the bound.** On the store's
  tapes, the bound adds 23 points of coverage and the dictionary saves
  20% of the bytes, which is close. On the corpora's named-only tapes,
  the bound is about 20 points but the dictionary saves only 5–7%.
  There, the shared material is not in a form a 110 KB dictionary can
  hold.

The repeats found in the most files show why (32-byte windows, by the
number of files they appear in):

- Sources:
  - `from __future__ import annotations` (Python, 45% of files);
  - `import { join } from "node:path"` (TypeScript);
  - indentation runs and `.iter()` (Rust);
  - UUIDs and timestamps shared across generated SQL fixtures.
- Tapes:
  - **The start of the code column**, `00 00 01 00 02 00 03 00 …`, is
    in 95–99% of tapes. Codes are numbered per tape in order of first
    use, so the first nodes of every tape count up from 0.
  - **Code-table entries:** a kind, a field, `ff ff` (no alias) and a
    class count, in nearly the same order in every tape. They appear
    in about 40–75% of files.

Both come from the tape numbering its codes per file. If one table per
grammar numbered the codes, two tapes with the same kinds would write
the same bytes in their code columns, and no code table would be
stored. In the store's small tapes, the code table alone compresses to
77 B of a 393 B frame.

Each part's share of a tape. The last three columns are each part
compressed on its own, in bytes per tape:

| Tapes | Raw, average | Frame, average | Header | Digest | Code table |
| -- | -- | -- | -- | -- | -- |
| Python store | 1,102 B | 393 B | 43 | 41 | 77 |
| Python, full | 21,698 B | 5,252 B | 46 | 41 | 325 |
| Python, named-only | 14,933 B | 4,271 B | 46 | 41 | 306 |
| TypeScript, named-only | 16,737 B | 5,507 B | 46 | 41 | 422 |

The fixed parts cost little in a large tape. In a small tape, the
header, digest and code table are a third of the frame or more.

## 4. What shipping it would take

- **A dictionary table.** `zstd_dicts(dict_id, grammar_id, kind,
  bytes)`. It is append-only, and a dictionary is never removed while a
  frame uses it.
- **One id per frame.** `codec_id` already exists: 1 is plain zstd.
  New values can mean zstd with a dictionary, for example
  `0x100 | dict_id`. Old frames stay readable, and no migration is
  needed. A frame is rewritten only when its document is re-encoded
  anyway.
- **Shipped dictionaries.** Each language gets a dictionary for
  sources and one for named-only tapes, built from fixture corpora and
  compiled into the binary (16–110 KB each, about 1.5 MB for all
  languages). They are versioned by grammar version and tape format
  version, like tapes. A store can train its own later, but the
  transfer result says shipped ones are close.
- **Use a dictionary only for frames under 64 KB.** Larger frames gain
  nothing, and for SQL a dictionary hurts them.
- **Speed and memory.** The compressor and decompressor keep one
  prepared dictionary per (language, kind), built once per process.
  Each is the dictionary's size plus a few tens of KB of tables. That is
  a few MB in all, within the 1 GB budget. Decoding a frame costs the
  same as without a dictionary.
- **Determinism.** Tapes are compared by digest, so a frame must
  stay byte-for-byte reproducible. With a fixed dictionary and level,
  zstd output is deterministic, as today.

## 5. Recommendation: the easy wins

In order of effort:

1. **Drop the source hash from the stored tape frame.** Keep it in the
   tape's in-memory image, and check it against `hashes` when the tape
   is read.
   - Saves 32 B per tape; on the Python store, 5.7 MB (7% of tapes).
   - This changes the tape format, so it goes with RER-131's version
     bump.
2. **Ship trained per-language dictionaries for small frames**, with
   the codec id in `codec_id`. On the Python store:
   - tapes 78.5 MB → about 63–66 MB;
   - sources 32.5 MB → about 28 MB;
   - together 15–18% of sources and tapes.

   Corpora of large files gain 5–10%.
3. **Named-only tapes (RER-131)**, already planned: 0.77–0.81× of tape
   frames on their own, and about 0.75× together with a dictionary.
4. **Not worth it:** hand-written keyword or standard-library
   dictionaries. A trained dictionary already contains whatever
   keywords and library text pay off, and the hand-written ones measure
   0.99–1.02× on sources.
5. **Larger, later:** number codes per grammar, not per tape. This
   removes the per-tape code table and makes code columns match across
   files, which is the largest cross-file repeat in tapes. It is a
   format change, and it needs its own measurement.

Beyond these, most of a store's bytes are in `atom_segment` and
`facts_results` (50–82% of each store above). They should be measured
the same way before more effort goes into tapes.

## 6. What shipped (commit 766f2baf2)

Recommendations 1 to 3 are done:

- The v12 tape drops the source hash.
- The store has per-language dictionaries.
- Tapes are named-only.

How the dictionaries work:

- One codec module, `content_store::compression`, writes and reads
  every store frame.
- A source or tape under 64 KB is compressed twice, plain and with its
  language's dictionary. The store keeps the smaller frame.
- A dictionary frame names its dictionary in the zstd header, in 2
  bytes. `codec_id` stays 1.
- Dictionaries are compiled in and append-only
  (`native/dictionaries/README.md`).
- A tape's language comes from its header. A source written without
  its tape gets its language from its path.

The shipped dictionaries were trained only on text that can be
redistributed: this repository, and CPython's standard library for
Python. They were measured on repositories they were not trained on.
Each figure is the frames as stored, as a fraction of plain zstd:

| Data | Files | All frames | Frames < 1 KB |
| -- | -- | -- | -- |
| Python sources, data-layer repository (1 in 10) | 1,308 | 0.957 | 0.78 |
| Python v12 tapes, same files | 1,308 | 0.961 | 0.68 |
| Python sources, the 55k-source store (1 in 9) | 5,993 | 0.974 | 0.79 |
| Rust sources, data-layer repository | 103 | 0.944 | 0.74 |
| Rust v12 tapes | 103 | 0.960 | 0.71 |
| JavaScript sources, data-layer repository | 35 | 0.965 | 0.67 |
| JavaScript v12 tapes | 35 | 0.971 | 0.60 |
| SQL v12 tapes, data-layer workspaces (1 in 4) | 1,602 | 0.973 | 0.74 |

What the table says:

- Small frames gain 20–40%, as §2 found.
- Overall, the shipped dictionaries save 3–6%, not the 15–20% of §2.
  §2 trained on the measured repositories' own files. A shipped
  dictionary only knows the language, not the repository's
  vocabulary.
- On the 55k-source store, Python sources by size:

  | Size | < 1 KB | 1–8 KB | 8–64 KB | ≥ 64 KB |
  | -- | -- | -- | -- | -- |
  | Stored / plain | 0.79 | 0.91 | 0.965 | 1.00 |

- **Cost.** On the slice backends test (S7), CPU time was 131.6 s
  with the dictionaries and 134.0 s without. That is within noise:
  the machine's load average was 106–162. A small frame is compressed
  twice when it is written; reads cost the same.
- **Compatibility.** Plain frames still read as before. A binary
  without these dictionaries cannot read a dictionary frame, and fails
  with "frame names unknown dictionary".

Next steps, if storage cost still matters:

- **Per-store dictionaries.** Train a dictionary on a store's own
  small frames, and keep it in the store under ids from 0x10000. §2
  measured 0.80× on tapes and 0.85× on sources for the Python store
  with its own dictionary. The codec already selects the dictionary
  by frame id, so only training and a dictionaries table are new.
- **Customer-trained shipped dictionaries.** Ruled out: they would
  put customer code in the binary.
- **The big tables.** `atom_segment` and `facts_results` are still
  the largest share of a store (§1).

## Files

In the session scratchpad, `compress/`. None of them is in the
repository.

- `study.py`: the dictionary comparison.
- `sa.c` and `sa_run.py`: the suffix-array bound and the top repeats.
- `parts.py`: each tape part's share of the bytes.
- `hand/*.txt`: the hand-written dictionaries (keywords and
  standard-library footprint per language).
- `work/*.json` and `work/*-repeats.txt`: the raw results.
