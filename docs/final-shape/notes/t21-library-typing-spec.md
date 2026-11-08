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
- the typing program that reads the workspace reads the libraries too, in the same run and the same way;
- a library has no summary: its facts are that program's rows for its documents, stored and reused as any file's
  are, because its files are unchanged.

What is Python-specific is written in `.rr`. That covers where the libraries are, which file a module name
resolves to, and what a stub's conditions and re-exports mean. The engine gets a few generic abilities, none of
which knows Python: list a directory, read a small file's lines, load a file as a document on demand.

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
- Whatever it costs, it is paid once: a stub's facts are stored like any file's (§2.5).
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
| loading | which files become documents | `python/modules.rr` asks | `load` requests (E2, E3) |
| stub meaning | conditions, re-exports, overloads, `...` | `python/stubs.rr` | — |

Nothing in these layers says how a library is evaluated. Once loaded, its files are documents like the workspace's,
and their facts are stored like any file's (§2.5).

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
`resolves` instead of from the path. Then `ext_import`, `isqmod` and the cross-file rules work unchanged
whichever root a file is under. Namespace packages (a directory with no `__init__`) are one rule.

### 2.3 Loading

Only the files a resolution reaches are read. A rule asks for a file with `load(path, module, grammar)`. The
module is the resolution's answer. The engine names the document by it (its `doc` row's module and its `pkg`), as it
names a workspace file by its path, so `modname` and relative imports work in a stub unchanged:

```text
// An import the workspace makes, resolved into a library: read that stub, and later what it imports.
load(p, q, "python") :- want(q), resolves(q, p, layer), layer != "workspace".
// Every module reads builtins implicitly.
load(p, "builtins", "python") :- resolves("builtins", p, _).
```

The engine reads each requested file as a document, named by its canonical path, so its key and memo entries are
the same in every workspace. `loaded(path, module, grammar)` lists the documents loaded. The engine then runs
the requesting rules again over the new documents, until no new request appears or a budget is spent (E2). A
loaded document is evaluated like every other; findings are reported for the files the run was asked to check. This is 03 §13's `demand module(M2) :- module(M), reexport(M, _, M2)`,
written as a rule.

Built (`corpus/world.rs`): a round runs the program cut to its requests (`load`, and the `rr_dir_req` and
`rr_file_req` that reading `dir_entry` and `file_line` derive), answers them, and runs again. The run proper then
reads the answers as facts. Built with a fourth column, `role` (`"library"`), that no rule or engine path
reads; it goes. A recursion that makes values is refused (O11), so a module's path is
`@replace_all(q, "\\.", "/")` and its prefixes are `@pkg_up(q, 1)`, not a walk.

### 2.4 Stub meaning (`python/stubs.rr`)

A stub is Python with a few conventions. The rules that give them meaning are Python's, so they live in `.rr`:

- **Conditions.** `if sys.version_info >= (3, 11):` and `if sys.platform == "win32":` are decided against
  `py_version` and `env("python.platform", …)`. A binding under a false branch does not exist. Under an
  undecided one, it is a possible binding, as `calls.rr` treats any `if` today.
- **Re-exports.** In a stub, `import x as x`, `from m import y as y`, a star import and the names in `__all__`
  are exported; a plain `import x` is not. This is `stub_export(d, name)`, and the program's module attribute
  lookup reads it for stub documents, whichever root they are under.
- **Bodies.** `...` bodies carry no flow. Types come only from annotations, which `calls.rr`'s declared-type
  section already reads (`rann`, `pann`, `adeclared`).
- **Overloads.** `@overload` defs give one declared return per overload. Until arguments select among them, their
  union is the possible type (a lower bound, as everywhere).
- **Accelerators.** `try: from _x import f` followed by `except ImportError: def f(...)`: both bindings are
  possible. This explains 130 of the Jedi divergences in t20 gap 6.

### 2.5 Stored facts: the same as any file's

A stub's facts are made and kept by the mechanisms that make and keep every file's. Nothing here knows which files
are libraries.

- **File-local rows**: the per-file memo (decision 0052) keys a document's rows by its content, its path and the
  plan. A stub's rows are therefore replayed from the memo after the first run that reads it, in any workspace on
  the machine.
- **Cross-file rows**, such as a class's inherited attributes or a re-export's target: every one is a derivation
  (03 §14, decision 0065). It is valid while the files it read are unchanged, and a run reads the valid ones and
  derives only what is missing or invalid. A stub's file does not change, so its facts stay valid. What a new
  workspace adds are new facts, such as the instantiations of a generic with that workspace's types, not
  rewritten ones. A changed stub invalidates what read it, as any changed file does.

## 3. Engine changes (generic, minimal)

| # | change | why generic |
|---|---|---|
| E1 | `dir_entry(dir, name, is_dir)` and `file_line(path, i, text)`: read-only, computed only for a bound `dir` or `path` (demand, 0048), and never for paths outside the configured roots | any language's lookup is a directory walk; config files are lines |
| E2 | `load(path, module, grammar)` requests: the engine parses each requested file as a document, in the demand loop 0055 runs for `parsed`, with a budget per run and per root | a guest without a host: 0055's embed loop over files instead of strings |
| E3 | file extensions per language in the registry (`languages/python.json`: `.py`, `.pyi`), so the corpus reader and E2 take `.pyi` | data, not code |
| E4 | `env(key, value)` facts from `--env k=v` and a project config file | settings as facts, read by rules |
| E5 | cross-file facts stored as derivations and read while valid, for every file alike (0065) | the derivation graph of 03 §14, not a new cache |
| E6 | *withdrawn:* a per-document cutoff in the rounds. The rounds go instead: the cross-file part is one fixpoint (0065) | — |

E1–E4 are small. E5 reuses the store's derivations and the per-file memo, which already make a file's facts once.
None of them names libraries.

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
2. **E5: cross-file facts stored, as every fact is (0065).** The goal is that a run whose stubs are unchanged
   costs at most 10% more than today's run without them. *A first form (0059) ran the libraries as a separate stage and
   replayed their rows into each round. It cost more than it saved, and its code is removed.*
3. **Site directories, stub packages, `py.typed`, `.pth`, user declarations as `.pyi`.** Retire `deps.rs`,
   `declare.rs` and the `declared_ret` facts. *Built: `python/modules.rr`'s layers in §2.2's order, the venv's site
   directory and `.pth` lines (`python/env.rr`); fixtures `world_site` and `world_decl`. `calls.rr` reads no
   `declared_ret`: the inputs of the `deps`/`declare` test type the same calls from `.pyi` declarations and installed
   sources. Not yet: a workspace `.pyi` beside its `.py` (the corpus reads `.py` files); `partial` stub packages.
   `deps.rs`, `declare.rs` and `corpus/typing.rs` remain for `workload.rr` and its test, unreached by `calls.rr`;
   removing them is the user's call.*
4. **§5.1–§5.3**, ordered by what the score says is left. *Built: §5.1's first version and generic returns, §5.2,
   §5.3 (§6b).*
5. **A second language's `env.rr` and `modules.rr`** (TypeScript: `.d.ts` is the same shape), as the test that
   the blueprint is generic.

## 6a. Checkpoint: step 1 measured (2026-11-07)

Built: E1–E4 (`corpus/world.rs`, `rerail.env`/`--env`), `python/env.rr`, `python/modules.rr` (typeshed's `stdlib`
with `VERSIONS`, and `builtins`), and the stub conventions in `calls.rr` that the score asked for first:
`@overload` (transparent, overloads do not shadow each other), generic and special-form bases, and module-level
`x: T` in a stub.

On the 84 files (dl-small), mypy's typeshed, debug build, against Jedi 0.19.2 (8,882 calls, 1,445 Jedi leaves
undecided):

| run | documents | rounds | agree | program unknown, Jedi decided | CPU |
|---|---|---|---|---|---|
| no typeshed, k = 4 | 84 | 6, pending | 5,477 | 1,548 | 10 s |
| typeshed, k = 4 | 263 | 6, pending | 5,457 | 1,568 | 39 s |
| no typeshed, k = 12 | 84 | 7, converged | 5,821 | 1,204 | 14 s |
| typeshed, k = 12 | 263 | 10, converged | 5,802 | 1,223 | 104 s |

No run names a wrong definition; 412 calls get no edge where Jedi names one, in every run.

What it says:
- **Convergence, not the library, moved the score.** At k = 4 the pending round leaves every target-less call
  unknown. Converged, the standard library is a wash so far: 1,208 calls reach a stub, and the unknowns are 19
  more, not fewer.
- **The library's ceiling here is about 308 calls.** Of the 1,204 unknowns at convergence, Jedi names a workspace
  function for 743 and a workspace class for 153. Only 308 are external: `ContextVar` methods (94), `str` and
  `list` methods (71), `Path` (54), `Mapping` (25), `IO`, `Logger`, … The §2.3 estimate of about 760 was too high
  for this corpus.
- **Cost is the rounds over library documents.** 263 documents take 7.5× the CPU of 84, almost all of it in the
  final run's local and cross parts (discovery is about 6 s). The rounds rerun the whole cross-file part per file
  crossing. 0065 replaces them with one fixpoint, whose facts are stored like any others.

Next, in that order: literal values as instances of `builtins` classes (`"".join`, `[].append`), the per-call
difference between the two converged runs (which calls the stubs decide, which they un-decide, and why), then
0065.

## 6b. Checkpoint: step 4, §5.1 and §5.2 (2026-11-07)

Built, in `calls.rr`:

- **Generic substitution (§5.1).** A class's parameters are read as written (`Generic[K, V]`, `Protocol[T]`, else
  the type variables its first generic base is given). An annotated instantiation (`x: dict[str, V]`) is a generic
  value. A method declared to return its class's parameter returns the receiver's binding of it. A function
  declared to return a generic (`-> Iterator[_T]`) returns a generic value of the declared class, its parameters
  bound at the call. That class is exact: a subclass's method is typed by it.
- **Implicit calls (§5.2).** A subscript `o[i]` is `o.__getitem__(i)`. `for x in e` binds x to
  `e.__iter__().__next__()`, and `with e as x` binds x to `e.__enter__()`. Their `async` forms stay unknown (the
  syntax producer marks them `op "async"`, as it marks `async def`).
- **Class decorators.** One declared `(cls: type[T]) -> type[T]` gives back the class. Before, every subclass of a
  decorated class had an open base, so a method call on a receiver that might be one was unknown.

The 84 files, k = 4, debug build (Jedi on the whole repository):

```text
                                agree   unknown, Jedi decided   no edge, Jedi a definition   another definition
before §5.1                     5,656   1,369                   412                          0
generics, first version         5,895   1,130                   412                          0
+ implicit calls, generic
  returns, class decorators     6,149     874                   412                          2
```

The two "another definition" calls are Jedi's error: `LazyWriteTransaction(LazyReadTransaction,
WriteTransactionLazyMixin)` finds `query_lazy` on WriteTransactionLazyMixin by C3, where Jedi takes
ReadTransactionLazyMixin's.

Of the 874 unknowns, 305 are calls Jedi resolves to definitions outside the 84 files (it read the whole
repository): no rule decides them on this corpus. 443 are workspace definitions in the corpus, 126 external.

Then, also in `calls.rr`:

- **ParamSpec wrappers.** A decorator declared `(func: Callable[P, A]) -> Callable[P, B]` (`contextmanager`) makes
  a call of the decorated function hold a B. B's type variables are bound by matching A against the function's
  declared return, through the return's class's bases. So `with opened() as v` gives v what `opened` yields.
  A class with no `Generic[...]` base takes its parameters from its first base that has a type variable as an
  argument (`_GeneratorContextManager`'s first base is given `Generator[...]`).
- **A function's own type variable.** It is bound from the argument's generic value or its class's bases
  (`sorted(iterable: Iterable[_T]) -> list[_T]`, `min`). For a protocol parameter, it is bound from what the
  argument's method returns, the method the protocol declares to return the variable
  (`next(i: SupportsNext[_T]) -> _T` is `i.__next__()`).
- **Overloads by arity.** An overload types a call only if the call's positional arguments fit it. The fit takes
  in required and keyword-only parameters, `*args`, and the `self` an attribute call binds. Every overload is
  still a call target.
- **`modules.rr`.** A VERSIONS line with a comment after an open range (`_typeshed: 3.0-  # ...`) had kept its
  module from loading: `_typeshed`, the home of `SupportsNext`, `SupportsIter` and `SupportsRichComparison`.

`next(iter(xs))` on a `list[V]` needs 8 rounds on typeshed, where k is 4. Under one fixpoint (0065) it needs no
bound.

## 7. Open questions

- **Rounds over library documents.** *Answered by 0065:* there are no rounds, and nothing is particular to
  libraries. Their files' facts come from the same fixpoint as every file's and are stored the same way.
- **Kinds across grammars.** A `.pyi` is the Python grammar, so no new kinds are needed. TypeScript's `.d.ts` is
  likewise its own grammar.
- **Where typeshed comes from by default.** Options: a pinned pack fetched by tooling, or one found inside an
  installed mypy, pyright or Jedi. The engine must never need the network during a run.
- **Untyped installed packages.** Reading their `.py` means bodies, which costs as much as the workspace. Keep
  this off by default, with a per-package switch.
