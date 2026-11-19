# T29: dictionaries trained on each store (plan, not built)

## Intro

The store compresses each source and each tape as its own zstd frame.
Since commit 766f2baf2, a frame under 64 KB can use a zstd dictionary
compiled into the engine, one per language (t28 §6). Those dictionaries
were trained only on text we can redistribute, so they know a language
but not a repository's vocabulary. They save 3–6% of sources and tapes
overall.

Training a dictionary on a store's own frames does much better. t28 §2
measured 0.80× on tapes and 0.85× on sources for the Python store with
its own dictionary. The gain is on small frames, which are most of a
store's tapes: 152k of 179k tapes are under 512 bytes.

This note plans a background process that trains, stores and applies
those dictionaries. Nothing here is built yet.

## 1. What already exists

The codec (`content_store::compression`) already does most of what is
needed:

- Each dictionary frame names its dictionary by id in the zstd frame
  header. A reader picks the dictionary from the frame, and
  `codec_id` stays 1.
- Every store reader goes through one of `decompress`,
  `decompress_into`, `decompress_head` or `decompress_chunks`.
- A writer compresses a small frame twice, plain and with a
  dictionary, and keeps the smaller one. So a new dictionary can never
  make a frame bigger.
- Shipped dictionaries use ids 0x8001–0x80FF.

What is missing:

- training;
- a table that holds a store's dictionaries;
- the codec reading dictionaries from that table;
- re-encoding the frames already stored.

## 2. Design

### Where dictionaries live

They live in a new table in the store:

```sql
CREATE TABLE frame_dictionaries (
  dictionary_id INTEGER PRIMARY KEY,  -- the zstd id, 0x10000 and up
  tape_tag INTEGER NOT NULL,          -- language (language::tape_tag)
  content INTEGER NOT NULL,           -- 0 source, 1 tape
  bytes BLOB NOT NULL,                -- the dictionary, uncompressed
  frames INTEGER NOT NULL,            -- frames that name it
  created_generation INTEGER NOT NULL
);
```

- Ids come from a sequence local to the store, starting at 0x10000.
  The shipped range stays separate, and a 3-byte id field costs one
  byte more per frame than a shipped id.
- The writer uses the newest store dictionary for its language and
  content. If there is none, it uses the shipped one.
- The codec looks an id up in the compiled table first, then in this
  table. It keeps loaded contexts per thread, as it does today, keyed
  by the store and the id.

### The trainer

The trainer is a background task on the store, run at low priority,
with the same footing as reclamation:

- **When it runs.** It runs for a language and content when the store
  has at least N small frames of it (say 2,000) and either:
  - there is no store dictionary yet; or
  - the newest dictionary's gain has fallen, measured on a fresh
    sample, by more than a margin (say 3 points). That catches a
    repository whose code has drifted.
- **Sampling.** It reads a random sample of frames under 64 KB, about
  100× the dictionary size: around 11 MB for a 110 KB dictionary. It
  decompresses them and trains in process with
  `zstd::dict::from_samples`. There is no CLI and no Node.
- **Check before keeping.** It compresses a second sample held out
  from training, with the new dictionary and with the current one. It
  keeps the new dictionary only if that sample comes out at least 2%
  smaller.
- **Budgets.**
  - Memory: about 12 MB of samples plus zstd's training tables. That
    is well under the 1 GB budget.
  - CPU: training 110 KB on 11 MB is seconds at most.
  - It runs in short transactions under `retry_busy`, so it never
    blocks a push.

### Re-encoding stored frames

New writes use the new dictionary at once. Stored frames are
re-encoded in the background, in batches:

- A batch reads frames under 64 KB whose dictionary is not the newest
  for their language. It recompresses each with the newest dictionary
  and rewrites the frame only if the result is smaller.
- Each batch is a short transaction of about a thousand frames, which
  can be resumed. It moves the `frames` counts of both dictionaries.
- Rewriting a frame does not change its content. A frame's identity is
  the hash of its content, not of its bytes, so no index, memo or
  result changes. Readers holding an old frame still decode it,
  because both dictionaries are still there.

### Retiring a dictionary

A dictionary can be deleted only when its `frames` count is 0. When
re-encoding has moved every frame off a dictionary, reclamation drops
it. Old dictionaries therefore never pile up beyond one per language
and content per drift.

### Copies between stores

Anything that copies raw frames from one store to another must copy
the dictionaries those frames name, or re-encode the frames on the way.
Today only `capture_prefilter` copies between stores, and it does not
copy tape or source bytes. A guard should still refuse to write a frame
whose dictionary id the target store does not hold. That turns a silent
loss into an error at write time.

## 3. Correctness and failure modes

- **An unknown id on read** fails with "frame names unknown
  dictionary". A missing store dictionary means the store is damaged,
  so the error is reported as `StoreError::Corrupt`.
- **Older binaries.** A binary without the table cannot read frames
  that use store dictionaries. That is the same rule as for shipped
  dictionaries, and it goes with a store schema bump.
- **Determinism.** Training samples at random, so two stores of the
  same repository get different dictionaries. Frames are compared by
  content hash, never by bytes, so this is harmless.

## 4. What to measure when it is built

- Bytes stored for sources and tapes, before and after a full
  re-encode, on the 179k-document Python store and the SQL-heavy
  store. t28 §2 predicts about 0.80× for tapes and 0.85× for sources.
- Trainer CPU and peak memory, and the re-encoder's CPU per thousand
  frames.
- Read CPU on S7 with store dictionaries against shipped ones. This
  should be the same, as it was for the shipped dictionaries.

## 5. Open questions

- The thresholds: the frame count that starts a training, and the
  drift margin that starts a retraining.
- Whether to re-encode stored frames at all, or only use the new
  dictionary for new writes. Re-encoding gets the gain on the frames
  already in a store. Without it, the gain only arrives as the
  repository is written again.
- Whether one dictionary per language is enough for a repository
  that mixes very different code, such as generated and hand-written
  files.
