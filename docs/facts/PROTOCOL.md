# The facts daemon's protocol

Status: Phase 3, P3c (`docs/facts/PHASE-3.md`). Implementation: `spikes/facts-a/src/daemon/` (`facts-a daemon`); client: `conformance/data-layer/facts-daemon.ts`; end-to-end run: `just rerail-workspaces run --engine facts` (`benchmark/workspaces/facts.ts`).

## Process

One daemon serves every worktree of a machine. It compiles the policy set once (`--policies DIR`, every `data-layer-*.json`, or `--checks`), keeps the memo in the content store (`--store FILE`; `native/src/content_store/facts_memo.rs`), and listens on a Unix stream socket (`--socket PATH`). `--threads N` bounds evaluation fan-out; `--cross none|scratch` selects the cross-file side.

Threads: an acceptor, one reader per connection, one engine thread that owns all state and handles requests in arrival order, a persistent pool of `--threads` evaluation workers (persistent because regex match caches are per thread: threads spawned per request would each add theirs), one cross-file thread, and one thread that returns the workers' freed pages to the system after each evaluating request. Several clients may connect; each worktree's cross-file pushes go to every connection that named it.

## Framing

Each message, both directions, is one frame:

```
u32 header length (little-endian) | header: UTF-8 JSON object | u32 body length (LE) | body: raw bytes
```

The header holds `op` and the message's fields. The body is empty except for an edit carrying a file's bytes. A request may carry `seq` (any JSON value); its partial frames and its reply echo it and add `ms` (time spent in the daemon so far). A frame part above 256 MiB is refused. A message without a `seq` the client sent is a push.

## Identities

- **Content**: blake3 of the bytes (`hash`, 64 hex digits) or the Git blob id (`git`, 40 hex digits: SHA-1 of `blob <len>\0` + bytes). The store maps both to one stored source; the daemon checks bytes against the hash they arrive with.
- **Path**: repository-relative, `/`-separated. The per-file engine reads `.py` files and, for regex clauses, `.rs` files.
- **Worktree**: a client-chosen name. Its membership is path → content.
- **Epoch**: per worktree, incremented by every membership change. Per-file replies carry the epoch they produce; cross-file pushes carry the epoch they were computed at, so a client drops a push older than the epoch it holds.
- **Finding**: `[check, clause, path, line, code, column|null]` (line 1-based; column only for regex clauses). **Failure**: `[check, clause, path, message]` (check and clause empty for a file-level failure: not UTF-8, not parsed).

## Requests

| `op` | Fields | Reply |
|---|---|---|
| `hello` | | `hello`: `protocol` (2), rules, compile refusals, clause counts, the per-file clauses `[[check, clause]…]`, the cross-file model, the store, threads |
| `sync` | `worktree`, `git_dir`, and `commit` (full hex id) or `entries` (`[[path, blob id]…]`, an index snapshot) | partial `findings` frames when it evaluates (§ Streaming), then `findings` (below) for every path whose content changed; a new worktree's frames introduce all its findings |
| `edit` | `worktree`, `path`, `hash` or `git` (neither: the path is deleted); body: the bytes, only after `need_bytes` | partial `findings` frames when it evaluates, then `findings`; or `need_bytes` (`worktree`, `path`, `hash`/`git`) when the daemon does not know the content and no body came |
| `findings` | `worktree` | `all_findings`: `epoch`, every per-file finding and failure of the membership |
| `rebalance` | | `rebalanced`: recomputes the plurality base and cluster bases over every worktree (below); cost, clusters, state |
| `close` | `worktree` | `closed`: the worktree's pairs released |
| `stats` | | `stats`: resident set, live heap, `memory` (jemalloc's allocated, active, resident, mapped, retained and metadata bytes; the system malloc zones' bytes in use, where SQLite allocates), store bytes, state (worktrees, base and cluster sizes, delta paths per worktree, held pairs, resident findings, held contents, cross-file runs, the largest worker scratch), work totals |
| `shutdown` | `attribute` (optional, boolean) | `bye`, then the daemon exits and removes the socket. With `attribute`, `bye.memory` is the engine torn down one component at a time (cross-file worker, held pairs' results, membership, interner, Git object databases, memo index, content store, compiled rules), each with the Rust heap and C heap bytes its drop returned, and what stays live after the last |

`findings` reply (per-file): `worktree`, `epoch`, `cross: false`, `partial: false`, `introduced`, `removed` (findings), `failures_introduced`, `failures_removed`, `changed_paths`, `work` (`evaluated_clauses`, `memoised_clauses`, `evaluated_pairs`, `blobs_read`), `frames` (partial frames sent before it), `tasks` and `workers` (the evaluation's queue and fan-out), `scratch_peak` (the largest heap one worker held). An edit or sync that changes no content is no work and an empty delta.

Hash first: a client sends the hash alone; the daemon answers from the store when it knows the content (another worktree, an earlier session) and asks for bytes otherwise. Bulk sync never transfers bytes over the socket: the daemon reads unknown blobs from the Git object database at `git_dir` and skips blobs the store already holds under their Git id.

## Streaming

A `sync` or `edit` whose change evaluates some clause (a content the memo lacks for some in-scope clause) answers in several frames: zero or more partial `findings` frames (`partial: true`; `worktree`, `epoch` (the epoch the request produces), `cross: false`, `introduced`, `removed`, `failures_introduced`, `failures_removed`, `seq`, `ms`), then the `findings` reply (`partial: false`). The delta of the request is the union of its frames: a client applies each frame as it arrives and holds the worktree's exact per-file findings after the reply. Order:

1. Before any clause is evaluated: the changed paths whose new (path, content) is resident (another worktree holds it) or deleted, and every clause the memo serves, as one frame.
2. As the workers finish: each batch of evaluated clauses (one rule's clauses on one path, or a run of them), as one frame per batch the engine collects; a clause's new findings and failures come in the same frame that removes the path's old findings and failures of that clause.
3. The reply: what no frame carried (file-level failures, the old content's results for clauses no frame replaced).

A change that evaluates nothing (every content resident or memoised: re-applies, contents known from another worktree or an earlier session) sends no partial frame; the reply carries the whole delta. Evaluation: the request's pairs' memo lookups over the workers, then one queue of tasks, largest files first (`daemon/perfile.rs`): a whole pair per task when there are at least two pairs per worker, else one rule's clauses on one pair, split into runs when a few large files would leave workers idle. The engine thread evaluates nothing: it forwards each batch the workers finish and stores each pair's fresh memo entries when its last task is in.

## Pushes

`findings` with `cross: true`, `model` (the cross-file implementation), `epoch`, `introduced`, `removed`, `failures` (the cross-file clauses' current failures), `members`, `changes` (path changes folded into the run), `files_read`, `ms`. The cross-file side runs on its own thread: after every membership change the engine hands it the worktree's membership snapshot, the epoch, and the changed paths, and goes on with the next request. The thread takes everything queued at once and runs once per worktree with the latest snapshot and every change since the last run, so a burst of edits yields one push per worktree; per-file replies never wait for it, and a push's epoch may be older than the client's.

## Per-file and cross-file

A clause is per-file when it is a pattern (`forbid` AST or regex) or a per-file relational clause of a rule without `report_when`; its findings for a (path, content) are a function of that pair and are answered synchronously. Budgets, repository relational clauses and every clause of a `report_when` rule are cross-file: they depend on the membership.

The cross-file side is `crossfile::CrossFile: Send { fn apply(&mut self, worktree, members, delta: &[PathChange]) -> FindingsDelta }`, owned by `crossfile::Worker` (the cross-file thread). Implementations: `FromScratch` (`--cross scratch`): `eval::finish` over every member's stored rows (the memo holds every evaluated clause's repository and budget rows), the reference an incremental model is compared with. P3a's model plugs in behind the same trait (an in-process entry point in facts-b).

## State

- **Membership** (`daemon/membership.rs`, copied from Spike B): a per-path base, cluster bases chosen by an exact dynamic program over contiguous runs of worktrees in name order, and per worktree only its delta from its parent level. Until the first `rebalance` the first worktree synced is the base; later worktrees attach to the level they differ least from.
- **Pairs**: a (path, content) some level's records hold is resident with its per-file findings (compact: clause ordinal, path, line, column, code id); `Resident` reference-counts pairs and releases those no record holds. A worktree's findings are its members' pair results, never a per-worktree copy.
- **Store**: per-file results live in the content store under `(content, plan digest, fact schema[, path])` (`facts_memo.rs`). The daemon pins the sources of held contents (owner `rerail-facts-daemon`) and unpins them when no pair holds them; the store's collection then reclaims them, their memo rows cascading.
