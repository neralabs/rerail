# Store frame dictionaries

These are zstd dictionaries that are compiled into the engine
(`content_store::compression`). The store compresses each source and
each tape as its own zstd frame. A frame under 64 KB is also
compressed with its language's dictionary, and the store keeps
whichever frame is smaller. Small files have little text of their own
for zstd to copy from, and the dictionary supplies it. Note t28 has the
measurements.

Each frame names its dictionary in the frame header by the dictionary's
id. The list is append-only:

- A retrained dictionary gets a new id and a new file.
- The old dictionary stays, so frames already in a store can still be
  read.
- Writers use the last dictionary listed for their language.

| File | Id | Trained on |
| -- | -- | -- |
| `python-source.zdict` | 0x8001 | CPython 3.13 standard library sources (PSF-2.0) |
| `python-tape.zdict` | 0x8002 | v12 tapes of those sources |
| `typescript-source.zdict` | 0x8003 | this repository's TypeScript |
| `typescript-tape.zdict` | 0x8004 | v12 tapes of those sources |
| `javascript-source.zdict` | 0x8005 | this repository's JavaScript |
| `javascript-tape.zdict` | 0x8006 | v12 tapes of those sources |
| `rust-source.zdict` | 0x8007 | this repository's Rust |
| `rust-tape.zdict` | 0x8008 | v12 tapes of those sources |
| `php-source.zdict` | 0x8009 | this repository's PHP fixtures (32 KB) |
| `php-tape.zdict` | 0x800A | v12 tapes of those sources (32 KB) |
| `sql-tape.zdict` | 0x800B | v12 tapes of this repository's SQL |

Notes on the table:

- There is no SQL source dictionary. This repository has too little
  SQL to train one, and a dictionary trained on it saved nothing on
  other SQL.
- Every dictionary was trained only on text that can be redistributed:
  this repository's own files and CPython's standard library. No
  customer code is in a shipped dictionary.

How to train one:

```
zstd --train -3 --maxdict=112640 --dictID=<next id> <files…> -o <lang>-<kind>.zdict
```

For tapes, the files are stored v12 images. `RERAIL_DUMP_TAPES=<dir>`
on the ignored test `tape::tokens_tests::named_only_tapes_on_corpora`
writes them.
