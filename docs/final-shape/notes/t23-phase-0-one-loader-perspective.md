# t23. Phase 0 as one loader for rule programs and sources: a perspective

A perspective for the session designing phase 0 (file management, change detection, and loading rule and source
files the same way). It is not a decision and claims no number. Where it says "Decision" below, read "proposal".
It fills the "P0 files" phase that 0083 §1 names but does not design. It keeps 0065 §2a (requests are answered
inside a fixpoint, with no pass before the run) and 0089 §1 (a module is a path plus the digest of its text).

## Intro

Before any rule runs, the engine has to know which files there are, which ones changed since the last run, and
which further files they pull in. Today it answers those questions twice, in two different ways.

- **Rule programs.**
  - `rr::program` follows `#include` eagerly, before the run, through a search path: the library directory, then
    the includer's directory.
  - It checks each file by reading and hashing it again in every process. It never uses `stat`.
  - What it found is not stored, except as the flattened text of the whole pack. That text is the key of the
    prepared program, so editing one rule re-prepares all 546 programs (O135 on `final-rerail-shape`).
- **Analysed sources.**
  - `corpus_files` walks the root with `read_dir`. It does not use git or `.gitignore`.
  - A stat index (size, mtime, inode, with git's racy rule) skips reading files that are unchanged.
  - Libraries are loaded inside the fixpoint: rules in `modules.rr` ask for directory listings and files, and
    `world.rs` answers.
  - Those answers are not stored. On every warm run, typeshed's 12 rounds of requests (183 stubs, 5.8 s of CPU)
    run again.
- **A third copy.** `module_graph.rs` keeps its own stat stamps, and the workspace engine has a separate gix-based
  index.

This decision makes one loader, phase 0, that does both jobs the same way:
- A path is a fact.
- Resolution rules turn the facts of loaded files into requests for more paths. An `#include`, a Python import
  and, later, a Rust `mod` are all such rules.
- The loader answers each request from the cheapest trustworthy source: the git tree, a `stat`, or later a
  file-system event stream.
- It stops at a fixed point, when every library is loaded.
- It records what it read, so a warm run with nothing changed costs one check of that record.

The rest of this record is: what P0 is (§1), its two passes (§2), how it knows a file is unchanged (§3), the
record of reads and early cutoff (§4), keys (§5), storage (§6), and what it replaces (§7).

## Proposal

### 1. P0 is a fact space whose rules ask for paths

P0 runs on the same engine as P1–P3, in its own namespace, to its fixed point (0083 §1). Its relations:

| relation | meaning | made by |
|---|---|---|
| `p0.root(r, path, provider)` | a root the run was given: the corpus, a program path, a library root from `env` | the run |
| `p0.want_dir(path)`, `p0.want_file(path)`, `p0.want_load(path, lang)` | requests | resolution rules |
| `p0.entry(dir, name, type)` | a directory's listing | the loader |
| `p0.absent(path)` | a probe that missed | the loader |
| `p0.content(path, source)` | the file's content, by its content source (sha256) | the loader |
| `p0.document(path, lang, source, doc)` | a loaded document, with its document key | the loader |
| `p0.module_edge(from, to)` | the include or import graph, as resolved | resolution rules |

- **Resolution rules are ordinary `.rr` rules over P0's facts and a document's own facts.** Examples:
  - `#include "x"` resolves to `library/x` if it exists, else to `x` beside the includer;
  - PEP 561's order in `modules.rr`;
  - the `python.typeshed`, `python.venv` and `pyvenv.cfg` settings in `env.rr`.
- **The loader is native.** It answers requests and never decides what to request.
  - This is the split `world.rs` already has: rules ask, Rust answers.
  - It also confines paths to the run's roots, and it keeps the budget.
- **Outputs.** P0 hands P1 the loaded documents, as `p0.document` rows, and the module graph.
- **A delta to P0 is a delta of these rows.** An added file inserts `entry` and `document` rows; a deleted one
  deletes them. Downstream phases see an ordinary delta (0090), and an empty one stops propagation.

### 2. Two passes of the one loader: rules first, then sources

A rule program has to be compiled before its resolution rules can run. So P0 runs twice, in two namespaces, with
the same code:

1. **P0r, rule modules.**
   - Its seeds are the program paths given to the run.
   - Its resolution rules are a fixed kernel: `#include` and the library search path. The kernel is held by the
     engine like `core.rr`: compiled into it, and checked by its digest.
   - Its fixed point is the include graph of every `.rr` module, each module keyed by path plus digest (0089 §1).
   - The compiler then prepares the program from that graph, module by module, not from flattened text (§5).
2. **P0s, sources.**
   - Its seeds are the corpus root and the library roots from `env`.
   - Its resolution rules are those the compiled program brings: `env.rr`, `modules.rr`, and any other language's
     libraries.
   - Its fixed point is every document the run reads, typeshed and site-packages included.

Why one loader matters later:
- A new language plugs in by shipping resolution rules as a `.rr` library. It needs no new loader.
- A rule that spans languages (for example, "every public Rust API has a Python equivalent") reads the P1 facts of
  both languages' documents. Both come from one P0s.
- Scope stays a filter over P0's documents and findings, not part of any key (0089 §5). The prior work found path
  scoping to be "a caller join, not a query concern" (F6). It also found that sharing over union-scoped relations
  had measured scope, not sharing (T2).

### 3. Knowing a file is unchanged: one interface, three providers

The loader asks a provider: "for this root, what changed since the stored state?" Every provider gives the same
two kinds of answer:
- a set of **unchanged prefixes**: whole directories whose stored P0 rows are still valid;
- a set of **changed paths**, each with its new content source or `absent`.

The loader turns the answer into a delta of `entry`, `content` and `document` rows. A provider is chosen per root:
git where the root is a git work tree, stat otherwise. Inside a git root, an untracked path or a path with a
filter attribute uses stat.

**a. Git (gix, in-process; no subprocess).** This is what NER-1562 built in the `tempo` engine
(`native/src/git_workspace.rs` at `8fe9ffed4`, before `f671d7798` removed subtree reuse):
- **The tree.** The provider reads HEAD's tree and keeps each tree object id by path prefix. A prefix whose tree
  id matches the stored one, with no status entry below it, is unchanged. Its rows are reused without
  enumerating it.
- **Changed paths.** `status` gives them: staged, unstaged, deleted and untracked, with the index's own stat cache
  and racy-git handling.
- **Blob ids are aliases.** A git blob id maps to the canonical content source, which stays sha256 of the bytes.
  The alias is trusted only for a clean path, or after the bytes are hashed once as git hashes them.
- **What it bought there.** On `nera-data-layer`, 5,725 files: the cold index took 11.59 s. A warm attach went from
  668.6 ms to a 34.1 ms median with subtree pages. Over 47 workspaces (341k files), elapsed time went from 48.7 s
  to 13.6 s.
- **Known pitfalls, all handled by falling back to stat for the path:**
  - a tree id alone does not prove the work tree is clean;
  - filters, line-ending conversion and `working-tree-encoding` make the bytes differ from the blob;
  - an uninitialised submodule made gix rediscover the parent repository and crash. Submodules are entered only
    with a `.git` marker and are never followed through symlinks.

**b. Stat (the default, and every non-git root).** This extends today's `corpus_stats` index:
- **Fields.** dev, inode, size, mtime_ns and ctime_ns. An entry is trusted only when its mtime is at least
  `RACY_NS` before the time it was recorded (git's racy rule).
- **Directories as well as files.** A directory's listing is reused while the directory's own stat is unchanged.
  This is what lets typeshed's and site-packages' listings be skipped.
- **Rule modules and library roots too.** The rule loader stops rehashing every file in every process.
- **A validation scan.** Each run rehashes a rotating slice of trusted files, about 1/64 of them, capped by a
  byte budget. A whole root is rehashed after a clock jump, or on a file system with coarse mtimes.
  - A mismatch is reported, and the root drops to rehashing until the next full pass.
  - This catches a `stat` that lies, such as a copy that keeps the mtime, or a coarse file system.
- **A library root fingerprint (0083 §6).** pyvenv.cfg, the `.dist-info` names, versions and `RECORD` hashes, and
  the pinned typeshed. It is a shortcut: when it matches, the stat check covers only that root's recorded reads.

**c. Events (watchman, FSEvents, inotify): later, not now.** The provider keeps a "since" token and returns the
paths changed since it. When the token is lost, as after a restart or an overflow, it falls back to stat. Its
answers have the same form, so nothing above the provider changes.

### 4. P0 records what it read, so a warm run with no change is one check

- **The record.** For each pass, P0 stores its read set:
  - the directories it listed, and the files it read, with what each provider saw;
  - the probes that missed (`p0.absent`), recorded as reads of the parent directory's listing.
  - A file that later appears where a probe missed, and shadows a later candidate in PEP 561's order or the
    library search path, must invalidate the probe.
- **A warm run.** It asks the providers about the read set only.
  - If nothing in it changed, P0's outputs are the stored ones: no rounds and no requests. Typeshed's 12 rounds
    become about 300 `stat` calls.
  - If something changed, the changed reads are a delta, and P0 is maintained from it like any phase.
- **Reads are recorded here, and only here.** 0090 refuses to record reads, because recording them per row cost
  +60% (0064). P0's reads are files and directories, hundreds of them, so recording them costs nothing that
  matters. They are also the only reads of anything outside the store.

### 5. Keys

- **Content.** sha256 of the bytes, as a `SourceId`, as today. Git blob ids are aliases (§3a).
- **Document.** It is made from the path, as today (decision 0052), and does not change when the content does.
  - A loaded library document is keyed by its canonical path (t21 §2.3), so its key and its memo are the same in
    every workspace.
  - The document key is the high bits of every node id. Node ids within it are numbered by definition (0092), and
    P0 does not depend on how.
- **Rule module.** Path plus digest of the text (0089 §1).
- **Prepared program.** The module graph from P0r: the module digests and their edges, plus the facts, inputs and
  outputs.
  - It is no longer the whole pack's flattened text. So editing one module re-prepares the modules that depend
    on it, not all 546 programs.
  - This is the cost O135 found behind 0089's per-unit memo.
- **No key is corpus-wide.** Each provider's state is per root, and each statistic is per file (S124).

### 6. Where P0's state lives

As already decided: SQLite is the durable copy, and the in-memory state may be ahead of it while its writes lag.
- **Stored.** The stat index, git aliases and tree ids by prefix, directory listings, the module graph, the read
  set, and each library root's fingerprint.
- **Deletes are written as deletes.** An incremental load must return its dead rows, or it fills the disk (S134).
  A run that drops a file removes its `entry` and `document` rows and unpins its sources in the manifest
  (decision 0053).

### 7. What this replaces

| today | becomes |
|---|---|
| `rr::program`: eager `#include` search, rehashing in each process | P0r, with the kernel's resolution rules and the stat provider |
| `tools::program_text`: a flattened pack as the prepared key | the module graph as the key (§5) |
| `corpus_files`: `read_dir` and `.scope` globs | P0s's seed request on the corpus root, through the git or stat provider |
| `world.rs` answers, not stored | P0s's loader answers, stored with the read set |
| `module_graph.rs` stamps and the workspace engine's gix index | the stat and git providers |

`world.rs`'s confinement to the run's roots and its load budget stay, inside the loader.

## Order of work

Built and measured: t24 (steps 1, 3 and 4, and half of step 2).

Each step is measured on the 84 files with typeshed, and the git step also on the data layer's 9,010 files.

1. **Store the loader's answers and the read set, with the stat provider.** This is the first win: the warm run's
   4.3 s "stats" phase (t22). Target: with nothing changed, P0s under 50 ms CPU.
2. **Move `#include` resolution into P0r.** Use the same request and answer path, checked by stat, and make the
   module graph the key of the prepared program. Measure one rule edit against today's re-preparing.
3. **The git provider.** Unchanged prefixes by tree id, changed paths from status, and blob aliases. Target: an
   unchanged 9,010-file tree in milliseconds.
4. **The validation scan.** Add a test with a mtime-preserving copy that changes the bytes.
5. **Events**, when asked for.

## Prior work consulted

- **The ast-index-spike indexes:**
  - S124: incremental indexing allows no corpus-wide statistic. Agrees.
  - S134: an incremental load must return its dead rows. Agrees (§6).
  - The linter survey's F3 (a fixed cost per file, all-or-nothing cache invalidation) and F6 (path scoping is a
    caller join). Agrees: scope is not a key.
  - T2: a union-scoped shared relation measured scope, not sharing. Agrees: no scope in keys.
- **NER-1562's workspace store:**
  - the `tempo` journal, entries 25 and 31–34;
  - `docs/content-addressed-workspace-store.md` ("Git aliases", "Git tree reuse", "Git limitations").
  - It agrees, and is adopted for §3a, including its pitfalls.
- **Rerail decisions and notes:**
  - 0052 (document keys), 0053 (the manifest), 0065 §2a (requests inside the fixpoint), 0083 §1 and §6, 0089 §1
    and §5, 0090 (deltas and cutoff), 0092 (definitions as the unit of change);
  - t21 §2.3 (canonical paths), t22 (the warm run's file-system rounds).

## Not decided here

- **Watchman or another event service.** The interface is fixed by §3c; the service is not chosen.
- **Network sources.** None: a run never uses the network (t21 §7).
- **Git LFS, sparse checkouts beyond dropping absent blobs, and worktrees that share an object store.** These use
  stat until measured.
- **How node ids number definitions.** That is 0092's. P0 gives only the document key.
