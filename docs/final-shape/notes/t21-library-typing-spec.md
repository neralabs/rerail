# t21. Typing against libraries: the standard library, installed packages and stubs (spec, proposed)

Status: proposed, 2026-11-07. A blueprint: Python first, the same shape for every language after it.

## Intro

The typing program (`typing/programs/calls.rr`) knows only the files of the workspace. A call into anything else
is unknown:

```python
p = Path(src).resolve()      # pathlib is not in the workspace: p's type is unknown
with open(f) as fh:          # open() and its file object: unknown
    fh.read()
```

On 84 files of the data layer, Jedi resolves 1,571 calls that the program leaves unknown. About half of
them need the standard library: calls of outside functions (203), outside parameter types (312), the pending
round's outside targets (246), and part of the classes with an unknown base (308).

Python describes its own libraries in stub files (`.pyi`). They hold signatures, classes and annotations, with no
bodies. The standard library's stubs are typeshed's. Installed packages ship their own stubs, a `<pkg>-stubs`
package, or inline annotations marked by `py.typed` (PEP 561). Which files a module name means depends on the
environment: the interpreter's version, the virtual environment and its site directories.

This note specifies how rerail reads them. The principle is that **library code is code**:
- a stub is a Python document, parsed like any other;
- the typing program that reads the workspace reads the libraries too;
- a library's "summary" is that program's rows for the library's documents.

What is Python-specific is written in `.rr`. That covers where the libraries are, which file a module name
resolves to, and what a stub's conditions and re-exports mean. The engine gets a few generic abilities, none of
which knows Python: list a directory, read a small file's lines, load a file as a document on demand, and
cache a library run.

## 1. Where things stand

**Measured** (2026-11-07, debug build, load about 18–66):

| | files | nodes | CPU |
|---|---|---|---|
| data layer, 84 files (`calls.rr`) | 84 | 250,862 | 12.0 s |
| typeshed stdlib reached from those 84 files' imports | 199 of 742 | 446,466 | 28 s before `88cda9248`, then about 21 s at load 95; rounds not converged |
| … from the 489-file data layer | 214 of 742 | — | — |

- The import closure into the standard library saturates. It covers 199 stubs for 84 files and 214 for 489
  files (about 1.5 MB and 40k lines). That is close to a fixed set per interpreter version.
- That first figure was mostly the program's own fault, not the stubs'. Two declared-type rules multiplied out:
  14M of the 28M join rows. With them fixed, the closure takes 12M join rows (`88cda9248`). The timings are from
  an unoptimised build of the engine itself, so its Rust loops run several times slower than shipped.
- Whatever it costs, it is paid once. The library's facts are made by the existing memo and derivations (§2.5).
- The corpus reader skips `.pyi` files today. The measurement above copied the stubs as `.py`.

**Built earlier, not reached in production** (the CLI and `corpus::run` pass `Typing::default()`):
- `typing/deps.rs`: `RECORD` scanning, lockfile versions, a summary cache under `~/.cache/rerail/summaries`,
  and a demand worklist with a per-package budget. It finds imports with a line scanner.
- `typing/declare.rs`: the `rerail.types/v1` stub dialect.
- `corpus/typing.rs::facts`: these become `declared_ret(q, t)` facts, return names only.

These are Rust and Python-specific, and they summarise only returns. This spec replaces them with `.rr`
(§6, step 3). The tests that use them move to the new path, and the copies are retired.

**How `calls.rr` sees outside code today**:
- The fact that a module is outside is known only by absence: `ext_import`, an import of a module that no
  workspace document is.
- Builtins are a hand table (`builtin`, `bclass`: 13 classes).
- `proto_q` names `typing.Protocol`.

All three become reads of the loaded stubs, with the hand tables kept as the fallback when no stub is found.

## 2. Design

There are five layers. The engine owns two of them, the generic ones. Each language owns the other three, in
`.rr`.

| layer | what it decides | Python | engine |
|---|---|---|---|
| environment | the search roots and their order, the interpreter version, the platform | `python/env.rr` | `env`, `dir_entry`, `file_line` facts (E1, E4) |
| resolution | which file a module name means | `python/modules.rr` | — |
| loading | which files become documents | `python/modules.rr` asks | `load` requests, the library role (E2, E3) |
| stub meaning | conditions, re-exports, overloads, `...` | `python/stubs.rr` | — |
| summaries | rows a workspace run reuses | the program's own rows | the per-file memo and the derivation store (E5) |

### 2.1 Environment (`python/env.rr`)

The run is given a few settings as facts: `env(key, value)`. Each has a default that a rule can derive.

```text
// rerail-engine logic … --env python.venv=.venv --env python.version=3.13
env("python.venv", ".venv").
// No version given: read it from the venv's pyvenv.cfg ("version = 3.13.1").
py_version(v) :- env("python.version", v).
py_version(v) :- !env("python.version", _), env("python.venv", e), file_line(cat(e, "/pyvenv.cfg"), _, l),
    substr(l, 0, 10) = "version = ", v = substr(l, 10, strlen(l) - 10).
```

The roots, in the order a lookup tries them:
1. The workspace's source roots: the repository root, plus `src/` where it holds packages (`env("python.roots", …)`
   overrides).
2. User declarations: a directory of `.pyi` files (`env("python.declarations", …)`). These are `rerail.types/v1`
   written as ordinary stubs (I17: "in the language's own syntax"). They fill gaps only, as decided.
3. The venv's site directory, `<venv>/lib/python<X.Y>/site-packages`, found with `dir_entry`. Also each line of
   its `.pth` files that is a path; lines starting with `import` are ignored, since nothing is executed.
4. Typeshed's `stdlib`: `env("python.typeshed", …)`, or a pinned copy that rerail ships as a content-addressed
   pack. Its `VERSIONS` file gives each module's range of versions, read with `file_line`.
5. Typeshed's `stubs/` (third-party stubs), after installed packages, as PEP 561 orders them.

### 2.2 Resolution (`python/modules.rr`)

`resolves(q, path, layer)`: the file that module `q` means. The first layer that has it wins, in PEP 561's
order:

| layer | file |
|---|---|
| workspace stub | `a/b.pyi` beside `a/b.py`: the stub gives the interface, and the `.py` keeps the findings |
| workspace | `a/b.py`, `a/b/__init__.py` |
| declarations | `<decl>/a/b.pyi` |
| stub package | `<site>/a-stubs/b.pyi` |
| typed package | `<site>/a/b.pyi` or `.py`, when `<site>/a/py.typed` exists |
| stdlib | `<typeshed>/stdlib/a/b.pyi`, if `VERSIONS` admits `a` at `py_version` |
| third-party stubs | `<typeshed>/stubs/*/a/b.pyi` |
| untyped package | `<site>/a/b.py`: off by default (cost), `env("python.untyped", "read")` turns it on |
| none | `ext`: unknown, as today |

`modname(m, q)` is the relation `calls.rr` reads for "document is module q" today. It is derived from
`resolves` instead of from the path. Then `ext_import`, `isqmod` and the cross-file rounds work unchanged
across the workspace/library boundary. Namespace packages (a directory with no `__init__`) are one rule.

### 2.3 Loading

Only the files a resolution reaches are read. A rule asks for a file with `load(path, module, grammar, role)`. The
module is the resolution's answer. The engine names the document by it (its `doc` row's module and its `pkg`), as it
names a workspace file by its path, so `modname` and relative imports work in a stub unchanged:

```text
// An import the workspace makes, resolved into a library: read that stub, and later what it imports.
load(p, q, "python", "library") :- want(q), resolves(q, p, layer), layer != "workspace".
// Every module reads builtins implicitly.
load(p, "builtins", "python", "library") :- resolves("builtins", p, _).
```

The engine reads each requested file as a document, named by its canonical path, so its key and memo entries are
the same in every workspace. `loaded(path, module, grammar, role)` lists the documents loaded. The engine then runs
the requesting rules again over the new documents, until no new request appears or a budget is spent (E2), and
writes no output rows for loaded documents. This is 03 §13's `demand module(M2) :- module(M), reexport(M, _, M2)`,
written as a rule.

Built (`corpus/world.rs`): a round runs the program cut to its requests (`load`, and the `rr_dir_req` and
`rr_file_req` that reading `dir_entry` and `file_line` derive), answers them, and runs again. The run proper then
reads the answers as facts. A recursion that makes values is refused (O11), so a module's path is
`@replace_all(q, "\\.", "/")` and its prefixes are `@pkg_up(q, 1)`, not a walk.

### 2.4 Stub meaning (`python/stubs.rr`)

A stub is Python with a few conventions. The rules that give them meaning are Python's, so they live in `.rr`:

- **Conditions.** `if sys.version_info >= (3, 11):` and `if sys.platform == "win32":` are decided against
  `py_version` and `env("python.platform", …)`. A binding under a false branch does not exist. Under an
  undecided one, it is a possible binding, as `calls.rr` treats any `if` today.
- **Re-exports.** In a stub, `import x as x`, `from m import y as y`, a star import and the names in `__all__`
  are exported; a plain `import x` is not. This is `stub_export(d, name)`, and the program's module attribute
  lookup reads it for library documents.
- **Bodies.** `...` bodies carry no flow. Types come only from annotations, which `calls.rr`'s declared-type
  section already reads (`rann`, `pann`, `adeclared`).
- **Overloads.** `@overload` defs give one declared return per overload. Until arguments select among them, their
  union is the possible type (a lower bound, as everywhere).
- **Accelerators.** `try: from _x import f` followed by `except ImportError: def f(...)`: both bindings are
  possible. This explains 130 of the Jedi divergences in t20 gap 6.

### 2.5 Summaries: the existing derivation store, no new cache

A library file's facts are made once, by the mechanisms that already make a workspace file's facts once:

- **File-local rows**: the per-file memo (decision 0052) keys a document's rows by its content, its path and the
  plan. A stub is a document, so its rows are replayed from the memo after the first run that reads it, in any
  workspace on the machine.
- **Cross-file rows**: these are the rows only the rounds make, such as a library class's inherited attributes
  or a re-export's target. Here the derivation graph (03 §14, `content_store::derivation`) is the mechanism.
  - A library never imports the workspace, so the cross-file rows of a set of library documents closed under
    their imports are a derivation. Its inputs are those files' digests, the settings the rules read (`env`)
    and the program.
  - The workspace run reads that derivation's output as fixed rows and does not recompute them. A changed stub
    rebinds its name and invalidates the derivation, as any changed file does.
  - What is new is the granularity: one derivation per import-closed set of documents, not per file. 0052
    memoises only the file-local part today, because the cross-file part was 0.01 s for its rules. For a
    typing program it is not.

## 3. Engine changes (generic, minimal)

| # | change | why generic |
|---|---|---|
| E1 | `dir_entry(dir, name, is_dir)` and `file_line(path, i, text)`: read-only, computed only for a bound `dir` or `path` (demand, 0048), and never for paths outside the configured roots | any language's lookup is a directory walk; config files are lines |
| E2 | `load(path, module, grammar, role)` requests: the engine parses each requested file as a document with `doc_role(d, role)`, in the demand loop 0055 runs for `parsed`, with a budget per run and per root | a guest without a host: 0055's embed loop over files instead of strings |
| E3 | file extensions per language in the registry (`languages/python.json`: `.py`, `.pyi`), so the corpus reader and E2 take `.pyi` | data, not code |
| E4 | `env(key, value)` facts from `--env k=v` and a project config file | settings as facts, read by rules |
| E5 | the cross-file rows of an import-closed set of library documents recorded as a derivation in the existing store (03 §14), and replayed into the workspace run as fixed rows; no findings for non-primary documents | the derivation graph at one more granularity, not a new cache |
| E6 | the per-document cutoff in the rounds (O41, O86): needed in any case for cost; more so while E5 is missing | already open |

E1–E4 are small. E5 reuses the store's derivations and the per-file memo, which already make a file's facts once.

No Rust in this list knows Python. The Python-specific Rust that exists (`deps.rs`, `declare.rs`,
`corpus/typing.rs::facts`) is retired once step 3 is in. `RECORD` and lockfiles are not needed to resolve a
module. They remain useful only to name a package version in a report, so identity is the files' content digest,
not the lockfile. That is a departure from I18 ("identity from lockfiles and `RECORD`"), to be recorded when this
note is adopted.

## 4. The blueprint for other languages

Each language writes the same three `.rr` layers over the same engine abilities:

| language | roots (`env.rr`) | resolution (`modules.rr`) | declaration files |
|---|---|---|---|
| Python | venv, site, `.pth`, typeshed | PEP 561 order | `.pyi` |
| TypeScript | `node_modules`, `tsconfig` `paths`/`baseUrl`, the bundled `lib.*.d.ts` | Node resolution, `types`/`typings`, `@types/*` | `.d.ts` |
| Rust | `Cargo.lock`, the registry source cache, the sysroot's `library/` | `extern crate` / `use` paths, crate roots | the source itself (public items only) |
| PHP | `composer.json` autoload, `vendor/`, phpstorm-stubs | PSR-4 | stub `.php` files |

The memo and derivations, `load`, `env`, `dir_entry` and `file_line` are shared. 03 §13's measurement for Rust
(3 hops reach 14% of registry bytes) is the same saturation as typeshed's 199 of 742 above.

## 5. What it needs from the typing program

Most of what a stub says, `calls.rr` already reads, because stubs are annotations. That is the O84 work: declared
returns, parameter types, classes, bases and the MRO. Three capabilities are missing, and the standard library
needs them:

1. **Generic substitution.** Much of typeshed returns a type variable: `def get(self, key: _KT) -> _VT | None`,
   `def __iter__(self) -> Iterator[_T]`, and `contextmanager`'s
   `Callable[_P, Iterator[_T]] -> Callable[_P, _GeneratorContextManager[_T]]`. Without binding `_T` from the
   receiver's or argument's type, these stay unknown. A first version binds a class's parameters from its
   annotated instantiation (`list[str]`) and from one argument position. This is the largest item.
2. **Protocols for loops and `with`.** `for x in e` takes `__iter__`'s return and then its `__next__`'s; `with e as
   x` takes `__enter__`'s return. These rules are small once the methods have declared returns.
3. **Instance attributes declared in a class body** (`x: int` in a class), which stubs use for every attribute.

## 6. Steps, with what each buys

Each step is scored against the Jedi corpus on the same 84 files, and its cost measured as O86 does.

1. **E3, E1, E4, E2; `env.rr` and `modules.rr` for typeshed's stdlib and `builtins` only.** No cache yet.
   This finds out how many of the 1,571 calls become decided. The ceiling is about 760 (the categories above),
   less what needs §5.1.
2. **E5: library cross-file rows as a derivation.** The goal is a warm workspace run that costs at most 10% more
   than today.
3. **Site directories, stub packages, `py.typed`, `.pth`, user declarations as `.pyi`.** Retire `deps.rs`,
   `declare.rs` and the `declared_ret` facts.
4. **§5.1–§5.3**, ordered by what the score says is left.
5. **A second language's `env.rr` and `modules.rr`** (TypeScript: `.d.ts` is the same shape), as the test that
   the blueprint is generic.

## 7. Open questions

- **Rounds over library documents.** The typeshed closure does not converge by k = 4 either. Should the library
  stage run to its fixpoint (it is cached, so cost matters less), or be bounded like the workspace?
- **Kinds across grammars.** A `.pyi` is the Python grammar, so no new kinds are needed. TypeScript's `.d.ts` is
  likewise its own grammar.
- **Where typeshed comes from by default.** Options: a pinned pack fetched by tooling, or one found inside an
  installed mypy, pyright or Jedi. The engine must never need the network during a run.
- **Untyped installed packages.** Reading their `.py` means bodies, which costs as much as the workspace. Keep
  this off by default, with a per-package switch.
