# 0264. Rule images ship with the engine; a store per build holds the rest

Status: decided (2026-11-15, the user's choice). Builds on 0246 (rules load as module images), 0263 (a library's
relations are closed), 0173 (the program image) and 0131 (a store per project and build).

## Intro

Loading a rule pack turns its `.rr` text into program images: each library module's, the linked library's, each
program's (0246). A run keeps them in a store and later runs read them instead of lowering again. But a store
belongs to one project (0131), so the first run in every new project made every image again: 2.9 s of CPU on the
Python pack, nearly all of it the same work for every project.

The library and the packs that come with the engine are fixed for a build. So are their images. They are now made
once, when the engine is built, and shipped with it:

```
rerail-engine                  the engine
rerail-library.images          its library's and packs' images, mapped by every run
```

What the bundle does not hold, because a library file or a pack was edited after the engine was built, is made by
the first run that needs it and kept in a store that every project on the same engine build shares.

## The decision

1. **A bundle beside the engine** (`rerail-library.images`, or `RERAIL_SHIPPED_IMAGES=<path>`, `off`): a sorted
   table of image keys, then the images. A run maps the file and reads only the pages of the images it opens.
   Each image starts at a 16-byte boundary, as `program_image::Image` keeps its words. Today `Image::open` copies
   an image out of the mapping. Once an image can be read from memory it does not own (the flat structures the
   typing work is building), it will be read where it lies.
2. **Keys decide, nothing else.** An image's key names the engine build, the image format and the bytes it was made
   from (0246 §6). A bundle made by another build, or from other files, has no key a run asks for, so it is never
   read. No version check or invalidation step is needed.
3. **A store per engine build for the rest** (`RERAIL_IMAGE_STORE=<path>`, or `project` for the run's own store).
   By default it is `rerail-images/<build>.sqlite` under the temporary directory, pruned with old builds as the
   project stores are. A lookup reads the bundle first and then this store, so the bundle is the store's seed:
   the store only ever holds images that differ from the shipped ones. Images there are kept by their lease
   (renewed by each run that reads one), since no run's manifest lives in that store.
4. **Building the bundle**: run the engine once over each shipped pack, on empty stores, with
   `RERAIL_EXPORT_IMAGES=<dir>`. Every image it keeps is also written to `<dir>`. Then
   `rerail-engine images pack <dir> rerail-library.images`.
5. Tests read and write their own store, as before. Under test the image store is the test's.

## Consequences

- A first run in a new project reads its library and pack images from the bundle, if the engine shipped them.
- Editing a library file makes new images for that file's module and the modules that include it (their keys name
  their scope's). Those go to the build's store and are shared by every project.
- Stage 5 of 0246, preparation derived from the linked library, can take its base from the bundle. Preparation
  done once per build ships too.

## Measured (2026-11-15)

The Python pack, one file, a fresh project store, release, one thread, M3 Max. The bundle holds 362 images
(the library's modules, the linked library, 317 programs, and the combined program's entry and image): 10.0 MB.

| Run | Combining | Preparing | The whole run |
|---|---|---|---|
| No bundle, a fresh store for the build | 9.38 G | 9.32 G | 38.74 G instructions, 2.91 s |
| No bundle, the build's store holding the images (a second project) | 5.79 G | 9.34 G | 35.16 G, 2.60 s |
| The bundle | 0.84 G | 9.26 G | 30.93 G, 2.29 s |

Every output relation is the same in all three. With the bundle, combining is a read. Preparing is not yet shipped:
the prepared program holds atoms of the store that prepared it (`prepared_interned`, checked by `holds`), and a new
project's store numbers its atoms in its own order.

## The text tests' automata ship too (2026-11-15)

With the prepared programs shipped (0270), compiling the text tests was half of what preparing still cost: 0.14 s
of 0.27 s on the Python pack's first run in a project. Each of its three groups of automata (decision 0075) was
determinized again, because a group was kept in the project's store alone.

A group is now an image of the engine's build (`rr::ImageDfas`): read from the bundle, else the build's image store,
and kept there by the store's writer (`keep_image`, with its lease), so a run exporting the bundle exports the groups
too. A group's DFA is regex-automata's bytes in the build's byte order, which is the build's as every image is. The
project store's copy (fact schema 103) goes, and the schema is retired, so the collector sweeps its rows.

Measured, the Python pack, one file, release, one thread, M3 Max, fresh stores:

| Run | Prepare | Whole run |
|---|---|---|
| The bundle, automata determinized | 4.48 G | 26.08 G |
| The bundle with the automata | 1.71 G | 23.44 G |

No bundle and the build's store: unchanged (9.83 G, 40.27 G) and lower (7.06 G, 33.64 G: the export run kept the
groups there). Every output relation matches the baseline. What preparing costs with the bundle now (0.145 s): the
memo's program and the components' plan 0.048 s, folding the guests' copies 0.024 s, reading the prepared program
0.014 s, the projection 0.013 s, statistics 0.012 s, the split's parts 0.011 s.
