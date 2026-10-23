# The facts daemon's protocol

Status: Phase 3, P3c (`docs/facts/PHASE-3.md`). Implementation: `spikes/facts-a/src/daemon/` (`facts-a daemon`); client: `conformance/data-layer/facts-daemon.ts`; end-to-end run: `just rerail-workspaces run --engine facts` (`benchmark/workspaces/facts.ts`).

## Process

One daemon serves every worktree of a machine. It compiles the policy set once (`--policies DIR`, every `data-layer-*.json`, or `--checks`), keeps the memo in the content store (`--store FILE`; `native/src/content_store/facts_memo.rs`), and listens on a Unix stream socket (`--socket PATH`). `--threads N` bounds evaluation fan-out; `--cross none|scratch` selects the cross-file side.

Threads: an acceptor, one reader per connection, and one engine thread that owns all state and handles requests in arrival order. Several clients may connect; each worktree's cross-file pushes go to every connection that named it.

## Framing

Each message, both directions, is one frame:

```
u32 header length (little-endian) | header: UTF-8 JSON object | u32 body length (LE) | body: raw bytes
```

The header holds `op` and the message's fields. The body is empty except for an edit carrying a file's bytes. A request may carry `seq` (any JSON value); its reply echoes it and adds `ms` (time spent in the daemon). A frame part above 256 MiB is refused. A message without a `seq` the client sent is a push.

## Identities

- **Content**: blake3 of the bytes (`hash`, 64 hex digits) or the Git blob id (`git`, 40 hex digits: SHA-1 of `blob <len>\0` + bytes). The store maps both to one stored source; the daemon checks bytes against the hash they arrive with.
- **Path**: repository-relative, `/`-separated. The per-file engine reads `.py` files and, for regex clauses, `.rs` files.
- **Worktree**: a client-chosen name. Its membership is path → content.
- **Epoch**: per worktree, incremented by every membership change. Per-file replies carry the epoch they produce; cross-file pushes carry the epoch they were computed at, so a client drops a push older than the epoch it holds.
- **Finding**: `[check, clause, path, line, code, column|null]` (line 1-based; column only for regex clauses). **Failure**: `[check, clause, path, message]` (check and clause empty for a file-level failure: not UTF-8, not parsed).

## Requests

| `op` | Fields | Reply |
|---|---|---|
| `hello` | | `hello`: rules, compile refusals, clause counts, the per-file clauses `[[check, clause]…]`, the cross-file model, the store, threads |
| `sync` | `worktree`, `git_dir`, and `commit` (full hex id) or `entries` (`[[path, blob id]…]`, an index snapshot) | `findings` (below) for every path whose content changed; a new worktree's reply introduces all its findings |
| `edit` | `worktree`, `path`, `hash` or `git` (neither: the path is deleted); body: the bytes, only after `need_bytes` | `findings`, or `need_bytes` (`worktree`, `path`, `hash`/`git`) when the daemon does not know the content and no body came |
| `findings` | `worktree` | `all_findings`: `epoch`, every per-file finding and failure of the membership |
| `rebalance` | | `rebalanced`: recomputes the plurality base and cluster bases over every worktree (below); cost, clusters, state |
| `close` | `worktree` | `closed`: the worktree's pairs released |
| `stats` | | `stats`: resident set, live heap, store bytes, state (worktrees, base and cluster sizes, delta paths per worktree, held pairs, resident findings, held contents), work totals |
| `shutdown` | | `bye`, then the daemon exits and removes the socket |

`findings` reply (synchronous, per-file): `worktree`, `epoch`, `cross: false`, `introduced`, `removed` (findings), `failures_introduced`, `failures_removed`, `changed_paths`, `work` (`evaluated_clauses`, `memoised_clauses`, `evaluated_pairs`, `blobs_read`). An edit or sync that changes no content is no work and an empty delta.

Hash first: a client sends the hash alone; the daemon answers from the store when it knows the content (another worktree, an earlier session) and asks for bytes otherwise. Bulk sync never transfers bytes over the socket: the daemon reads unknown blobs from the Git object database at `git_dir` and skips blobs the store already holds under their Git id.

## Pushes

`findings` with `cross: true`, `model` (the cross-file implementation), `epoch`, `introduced`, `removed`, `failures` (the cross-file clauses' current failures), `files_read`, `ms`. The engine runs the cross-file side for every worktree changed since its last run when no request is waiting, so a burst of edits yields one push per worktree.

## Per-file and cross-file

A clause is per-file when it is a pattern (`forbid` AST or regex) or a per-file relational clause of a rule without `report_when`; its findings for a (path, content) are a function of that pair and are answered synchronously. Budgets, repository relational clauses and every clause of a `report_when` rule are cross-file: they depend on the membership.

The cross-file side is `crossfile::CrossFile { fn apply(&mut self, worktree, members, delta: &[PathChange]) -> FindingsDelta }`. Implementations: `FromScratch` (`--cross scratch`): `eval::finish` over every member's stored rows (the memo holds every evaluated clause's repository and budget rows), the reference an incremental model is compared with. P3a's model plugs in behind the same trait (an in-process entry point in facts-b).

## State

- **Membership** (`daemon/membership.rs`, copied from Spike B): a per-path base, cluster bases chosen by an exact dynamic program over contiguous runs of worktrees in name order, and per worktree only its delta from its parent level. Until the first `rebalance` the first worktree synced is the base; later worktrees attach to the level they differ least from.
- **Pairs**: a (path, content) some level's records hold is resident with its per-file findings (compact: clause ordinal, path, line, column, code id); `Resident` reference-counts pairs and releases those no record holds. A worktree's findings are its members' pair results, never a per-worktree copy.
- **Store**: per-file results live in the content store under `(content, plan digest, fact schema[, path])` (`facts_memo.rs`). The daemon pins the sources of held contents (owner `rerail-facts-daemon`) and unpins them when no pair holds them; the store's collection then reclaims them, their memo rows cascading.
