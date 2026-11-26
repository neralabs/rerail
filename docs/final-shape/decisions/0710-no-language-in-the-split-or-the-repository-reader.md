# 0710: No language in the split or the repository reader

Status: building (2026-11-26). Supersedes the parts of [0691](0691-what-crosses-to-the-cross-file-part-is-checked-on-main.md)
named below.

## The fault

0691 made the split, the repository reader and the engine's phase programs name languages, frameworks and packs:

- The split (`typing/file_local.rs`) listed 47 relations by name: `SETTINGS_BY_PATH` (14: `tsconfig_for`,
  `tsx_names__target_at`, `py_unpackaged`, seven `points_to__*`, …), `SETTINGS_BY_FOLDER` (33, 13 of them Django's
  `py_names__dj_*`) and a `matches!` of `package`, `tsconfig`, `env` and the rest.
- The repository reader (`facts/repository.rs`), the core's, made `tsconfig`, `tsconfig_for`, `tsconfig_path`,
  `npm_ignored` and `py_unpackaged`, each listed again in `facts/core.rs` and in the reader's `reads` check.
- The engine's own programs (`typing/programs/`) held `py_names` (with Django's models, managers, hooks and Celery
  tasks), `tsx_names`, `points_to` and `node_resolve`.
- An input a library declared and nothing produced was bound empty, so a rule read nothing where a producer was
  missing.

Each new language, phase, or framework helper meant an edit of the engine's core. REVIEW.md: nothing outside a
family's Engine Adapter branches on a language.

## The shape

1. **A relation's placement is declared where it is declared.** Three `.decl` qualifiers, read by the split and the
   corpus memo from the program (`Program::qualified`, `About::qualified`):
   - `setting`: the project's, one value for the run, the same beside every chunk (`env`, `file`, `package_export`).
     A per-file rule joins it and stays per-file.
   - `keyed`: a setting whose column 0 names a folder, target or module (`package(dir, name)`,
     `py_names__exports(m, x, q)`); a file's memo entry is keyed by the rows of the keys its own rows name.
   - `by_path c`: a setting with a row per file, its path in column 0; column `c`, if given, names the key whose
     `keyed` rows the file reads (`package_for(path, dir) by_path 1`).
   No relation is named in the engine.
2. **`file` is a setting.** Its rows change when a file is added or removed, never when one is edited. Python's
   module privacy reads `file(cat(dir, "/__init__.py"))` at its own path, per file, as main's rule did; `py_unpackaged`
   is removed.
3. **The repository reader is generic.** The core makes `file`, `option`, `option_item` and main's `package*`. A
   language's configuration files (`tsconfig`, npm's publication) are read by a reader its adapter registers at load
   (decision record of the crate split: the core defines the interface, a language registers its implementation). The
   reader states its relations' columns; the core lists none. Their declarations are the language library's.
4. **A language's phases are the library's.** `py_names`, `tsx_names`, `points_to` and `node_resolve` move to the
   rule library's `phases/`, which phase lookup searches after `phase_dir` and before the engine's own programs. The
   engine's own (`struct`, `types`, `values`, `sources`, the `calls` programs) stay. Django's relations leave
   `py_names` for a phase `dj_names` that reads `py_names`' exports.
5. **An input nothing produces is an error**, naming the input and its library. An `extensible` relation nothing
   states is still stated empty: that is an extension with no members, not a missing producer.

## What stays

0691's three checks, its lists and their entries, and its rule rewrites stand. The split's special cases (0691's
"what else changes") are kept where a declaration does not replace them, and named in a later decision where one
does.
