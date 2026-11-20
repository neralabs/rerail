# Git tree facts and analysis scopes

## Source selection

A revision scan uses today's policy and the selected Git tree. Use the same
named analysis scope for each invocation in a series.

```sh
rerail scan --revision HEAD~1 --analysis-scope release-review
rerail scan --revision HEAD --analysis-scope release-review
rerail scope delete release-review
```

An analysis scope holds cache entries for completed runs. A report still covers
one revision. The selected tree and commit remain explicit inputs to rules.
The existing `--scope repository|git-diff` option controls which paths an audit
checks.

The default analysis scope for a filesystem scan is its canonical working
directory or the directory given by `--repo`. A watch uses the directory it
watches. Stopping the watch releases cache ownership for that directory and
its descendants. A named scope remains until its owner deletes it. Names
identify scopes within one content store. The same name can hold runs from
more than one repository.

## Tree facts for rules

`git_tree_walk_request(repo, tree)` requests one walk. The Git provider answers
requests for individual tree objects. It returns each immediate entry's name,
mode, and target object ID. The library walks the tree and gives each occurrence
its repository-relative path.

| Relation | Columns |
| --- | --- |
| `git_tree_entry` | Repository, parent tree, name, mode, target |
| `git_tree_directory` | Repository, walk root, path, directory tree |
| `git_tree_file` | Repository, walk root, path, mode, blob |

The root directory has the empty path. The same subtree can occur at several
paths, with the same tree ID at each path. Regular files include executable
files. Raw entry facts also include symlinks and gitlinks. The source inventory
excludes those entries. Its `.rerail` rules request each child through the path and
ignore filter before they ask Git for the child tree. The filter uses the
selected tree's `.rerailignore` and Rerail's mandatory ignore patterns.
The filter can skip an ignored tree if no descendant can match a rule that
includes it after the later rules apply. It checks files and directories
separately because a rule can apply only to directories.
If the filter cannot prove that the tree is excluded, it keeps the tree.
For each selected child, the inventory rules follow its parent placements back to the root.
They admit the child if its tree ID differs from every ancestor on that path.
Two siblings can therefore share one tree. The inventory returns its rows through an in-memory fact image.

The library first derives the set of reachable tree IDs. That set bounds the
depth of path construction. A valid Git tree graph is acyclic. The number of
reachable trees is therefore a bound even when multiple paths share one tree.
If a referenced tree cannot be read, `git_tree_walk_error` reports the failure.
`git_tree_walk_complete(repo, root)` holds after a successful explicit walk.
The source inventory requires every tree that passes its filter to be readable.
When a policy asks for a tree that the inventory read, it receives those same
decoded entries. The request can still reach trees outside the inventory.

A revision view keeps up to 8 MiB of decoded Git directory entries for later lookups.
The limit counts entry records, filename storage, object IDs, and storage stamps. The view evicts the least recently used trees first.
A larger tree remains readable. The view gives its entries to the caller and omits that tree from the cache.
Changing the path selection can reuse recent trees. Eviction leaves the scope's stored Datalog facts available to later runs.
The current inventory and entries that a caller is still reading have their own lifetimes.
The inventory also uses verified tree entries from earlier policy requests in the same scan.
An explicit path under a missing tree gives a source error for that path.
The scan still checks other requested paths whose trees it can read.

## Retention and invalidation

The content store records a pending acquisition before publication. A successful transaction records the scope, run, and snapshot memberships.
The scope supplies the stop ancestors. An insertion trigger indexes those ancestors, and deleting the scope removes its index rows.
A failed publication rolls back those rows together. The pending acquisition can then retry or abort.
The object and frame segment tables identify the storage files that hold fact payloads.

The build computes the suffix registry version from the same ordered dialects and enabled grammars it supplies to rules.
Each embedded rule program has an identity derived from its library text and the engine build.
The phase reads the compiled program from the selected store and names its entry in the consuming run.
Another process can reuse that entry while a scope owns the run.

Nested evaluations that fold constant relations return their result as an in-memory output image.
Library rules that read a typing program's outputs use the same in-memory interface.
They must preserve existing files in the invocation directory and in the caller's output directory.
The stored result contains only the requested constant relations.

Several evaluations can request Git facts at the same time. Each evaluation keeps its own repository and answers.
While a worker waits for an answer, it must allow queued work that produces that answer to run.
This applies even when simultaneous evaluations outnumber the CPU workers.
A revision scan can run its Git inventory and the policy's typing rules at the same time.
Each nested evaluation can run queued parser jobs while it waits for its next batch of documents.
An evaluator that exchanges requests and replies with its caller needs a separate stack.
It first offers that work to the CPU pool. If no worker claims it within one millisecond, a service thread takes it.
The caller can then run queued work while it waits for a request.

A policy can read a Git revision and a later policy can use that same answer to request a commit.
When the engine saves an answer for reuse, it saves the rows even if the first policy uses them only in its output.
On upgrade, the engine rechecks entries written with the earlier native-answer format before it reuses them.

Include `analysis.rerail` to query completed scans in the selected content store.
On Unix, a directory scope preserves the bytes of each physical path component.
Most components keep their UTF-8 spelling. A component with invalid UTF-8 or the literal replacement character uses `./` followed by its lowercase hex bytes.
The dot component cannot occur in a canonical path. Slashes between components preserve directory ancestry for parent stops.
Use this spelling to join scope and history facts. Use the original path or its symlink for filesystem operations.
Registration captures this key before publication. A queued stop keeps its captured key even if a symlink changes.
Older releases could merge these names into one replacement character. An individual stop leaves that ambiguous history for a stop on the common parent.
The headless CLI requires a UTF-8 checkout path. If a symlink leads to an unsupported path, source selection reports the error before opening a checkout.
Scope presence comes from published ownership or recorded history. A scope imported from an older store can own manifests or snapshots without a recorded scan.
The engine reads the requested scope and run relations from one SQLite snapshot.
For a lookup by scope, derive `analysis_history_request("named", name)` or `analysis_history_request("scope", scope)`.
Each lookup uses the scope key in the store's primary index. The cache compares the rows for that scope.
A publication or removal in another scope preserves the answer. Whole-store enumeration still observes all published runs.
A run has an opaque key. Its scan ID is the value that the caller supplied or the standalone command assigned.
Git run records use the clone ID shared by the repository's worktrees. A rename preserves the ID.
A copy that includes the existing clone ID can read the same history from a shared store.
A new `git clone` has its own identity, even if the two clones have identical commits and remotes.
Older records contain only a hash of the Git directory pathname. Their original repository remains unknown.
Rules can still enumerate those records by scope and scan ID.
Passive catalog lookup checks the same directory identity before using saved checkout settings.
A fresh clone can still inherit repository-wide settings through its remote.
The catalog checks the current checkout and its effective remote when the daemon looks up settings.
If a different linked worktree takes the old path, the next lookup uses the replacement's repository.
Registration and pruning use the same Git worktree list. A newline in a path belongs to its name.
On older Git versions, Rerail reads the linked paths from the common directory's worktree registrations.
If Rerail cannot read the registry, pruning keeps the recorded checkout.
The native `scanRrPoliciesAsync` call acquires one run for a durable batch when the caller omits the run handle.
The batch publishes once all policies and required source reads succeed. A failed batch preserves the earlier owners of its scope.
An explicit run handle lets the caller group several evaluations before publication.
The home registry records a pending route for each named scan. Deletion can reach a scan while it runs.
Successful publication promotes the route. An abort removes only that acquisition's route.
After an interrupted update, deletion checks the acquisition in the identified content store. An offline store keeps its route until it can answer.
When a process exits during a scan, cache collection discards abandoned staging rows. It also checks the pending routes for that store.
Older registries recorded a route before a scan finished. During an upgrade, deletion checks the identified store for owners and completed runs.
If an earlier writer can still publish, deletion retains its route. The old store format cannot identify which named scope a pending writer used.
Use the run key to join facts. Two calls can supply the same scan ID.

```rr
#include "analysis.rerail"
analysis_history_request("named", "release").
.decl reviewed(scan: symbol, repository: symbol, tree: symbol)
reviewed(id, repo, tree) :-
    analysis_history_of("named", "release", _, _, id, _, repo, tree, _, _), tree != "".
.output reviewed
```

| Relation | Meaning |
| --- | --- |
| `analysis_named_scope(name, scope)` | A named scope with a completed run |
| `analysis_directory_scope(path, scope)` | A directory scope with a completed run |
| `analysis_scope_run(scope, run)` | A run that the scope published |
| `analysis_run_scan_id(run, id)` | The caller's scan ID |
| `analysis_run_directory(run, path)` | The working-tree source directory |
| `analysis_run_snapshot(run, snapshot)` | Each stored workspace snapshot that the run published and the store still retains |
| `analysis_run_tree(run, repo, tree)` | The selected Git tree and the repository key used by `git.rerail` |
| `analysis_run_commit(run, repo, commit)` | The selected commit, when the caller selected a commit |

The store publishes a run record in the transaction that publishes its cache ownership.
Finishing a run fixes its publication outcome and warning. Repeated finishes return that result, including after a worker reports a late warning.
Two runs using the same stored snapshot have the same snapshot key. A metadata-only run can have no workspace snapshot.
Run history does not extend a snapshot's lifetime. Collection removes the link when it removes the snapshot.
A failed run and a run that loses its publication permit produce zero rows. A metadata-only scan can publish a run.
Deleting a scope removes its run records. An older store keeps its existing cache owners when it gains this table.
The store indexes each directory scope under the ancestors that can stop it. Delivery uses this index to find the affected history.
On upgrade, the store builds the index from the recorded ancestors. Older writers also update it when they publish a new directory.
Run history starts with scans that publish a record. Historical working-tree paths identify the directory as it was named at publication.
The history query observes the committed records it reads. A later publication or deletion invalidates results that used those records.

Scope membership is a relation. `analysis_scope_manifest(scope, manifest_id)`
connects a scope to each corpus run manifest. Existing manifest facts connect
the run to its sources and plans. `analysis_scope_snapshot(scope, snapshot_id)`
connects a named scope to each of its audit snapshots. A directory scope keeps
the latest snapshot for each workspace in `analysis_scope_workspace_snapshot`.
These cache ownership relations live in the content store's SQLite schema. The rule relations above identify the scans that published under each scope.
If an index restores a file's results, the current snapshot names its saved source and compatible tape.
This ownership protects the tape when collection removes an older snapshot.

The collector retains data reachable from any live owner. Deleting a scope
deletes its ownership rows. Shared sources, tapes, and cache frames remain
reachable through the other owners. Active run leases protect data during
publication and collection. A scan stages ownership and publishes it after the
whole report succeeds and every cache write succeeds. If a cache write fails,
the report keeps its findings and reports a cache warning. Collection releases
abandoned staged rows after their process drops the lease. A scope deletion
advances the scope's generation. Each run has its own staging key in the store where it started.
Finishing the run closes that key. Its workers can no longer add memberships.
Each evaluation that uses a Git inventory records the root and inventory programs in its selected store.
A reused source view can supply their verified tree entries. The programs publish their manifests under the current acquisition.
Repeated inventory requests within that evaluation can use the same materialized result.
When a rule derives a constant from a file in a revision, its cache entry follows the program and the file it read.
Another revision or scope can reuse the result when the path still supplies the same content. Changing the file requires a new derivation.
Each scope that publishes a run retains the constant that run used. Collection can reclaim it after the last owner releases it.
For a Git file, the reader records the blob ID, the digest of its bytes and an observation of its storage.
After a repack, the reader verifies the bytes before reusing the result. A missing blob supplies no cached lines or digest.
After scope deletion, a new acquisition must retain the same plans as a fresh view of that source.
Each store that consumes a compiled policy needs its input record and the graph of its includes.
Policy includes use forward slashes on Windows and Unix. The reader checks each included file and missing candidate through a native filesystem path.
When the process already holds a validated local policy, it checks the destination store's recorded inputs and compiled program.
Matching inputs and an available program let it reuse the policy in memory. If collection removed either entry, it records the policy again.
Repeated inspection in one catalog checks the files again. A fresh catalog also checks the store, even when another catalog loaded the same program.
The inspection profile measures file checks, store access, read-record checks, image availability and program decoding separately.
The process can reuse a checked graph from another store. It must save that graph in the selected image store.
On restart, the engine checks the recorded file digests and missing include candidates before it reuses a graph.
A scan can execute a policy inspected for another store. It retains the program there while the inspected inputs still match.
If an include changes after inspection, the scan executes the inspected version. The next inspection reads the new version.
A native policy catalog can inspect the same relative path in several Git revisions.
An explicit policy source selects the inspected entry for that repository, revision, and directory.
A historical source scan still uses today's policy unless the caller supplies a separate Git policy source.
For an explicit Git policy, the compiled program depends on its bytes, its resolved includes, the selected rule library and the compiler.
The stored include graph uses repository-relative paths for Git inputs. A new process binds those paths to the policy revision it opened.
A change elsewhere in the tree preserves the compiled program. A change in an included rule can produce different findings.
A failed read of a required Git blob fails inspection, including after a successful inspection of that policy.
Inspection checks the targets of includes and the library paths where it previously found no file.
If a library file appears there, it takes precedence over the file beside the policy.
Each consuming scope retains the compiled version it used, including when the catalog opened a different store.
Restoring earlier include bytes can reuse an earlier compiled version while a scope still retains it.
Catalog lookup accepts the same file name with repeated separators or a `.` component.
A Git policy path must name a file inside its selected source. A trailing separator requires a directory and gives a policy path error.
Without a policy source, evaluation uses the inspected working-tree policy, or the only inspected Git policy for that path.
If several Git entries match the path, the caller must select a policy source.
The collection check gives a second scope its own policy and memo rows. After releasing that scope, it checks which rows collection removed.
The inventory key uses the set of selected paths. Reordering paths, repeating one, or adding a child of a selected directory preserves the key.
Selecting the root has the same key as an unrestricted walk. An empty selection covers no files.
Native path restrictions preserve a trailing directory requirement. A read of a regular file with a final `/.` fails.
Membership, listing, and file reads accept the same repository-relative spellings.
For example, `src/` lists a directory, and `src/../src/a.py` reads the file at `src/a.py`.
The native checked file reader accepts integer byte limits from zero through 4,294,967,295.
Within one generation, the directory keeps the snapshot from the most recently acquired successful run.
A run in that generation can finish its report. Only the current generation can
publish ownership.
Collection also checks whether another workspace still has a live lease on the
snapshot. A managed stop queues the release in the same transaction as the stop.
The stop advances a durable generation. A scan or watch reads the generation
before discovery. The release affects the checkout and its descendants in
ended generations. The canonical control database path identifies the home
that owns a directory scope. Homes can share a store and keep independent
directory scopes. A directory scope keeps the latest snapshot for each
workspace. It protects the latest manifest for each policy set. An edited
policy keeps its policy set and gets a new cache key.
Each directory membership records its acquisition sequence and the manifest's order within that run.
Only successful publication can advance that order. An aborted run or a read in another scope can still use the same manifest.

Deleting a named scope releases its memberships and removes its generation row.
The store allocates a fresh generation when a caller reuses the name. A writer with the earlier generation cannot publish after reuse or restart.
Older stores can contain generation rows without evidence of whether a scope is still active. The next explicit release removes such a row.
For each new directory scope, the store records the highest control generation that any caller acquired.
A numbered parent stop can remove a child's generation row when its cutoff covers this value.
It must keep the row if a writer already acquired a later control generation, even before that writer publishes.
A late acquisition below the cutoff cannot recreate the removed row. A new watch gets a fresh store generation.
Older stores and older writers can leave a directory without a known upper bound.
A numbered stop preserves that generation row. An unconditional release of the same scope removes it and revokes its writers.
After a numbered stop, the home removes the directory registrations that the delivery covered.
A concurrent acquisition in a later generation keeps its registration.
The home keeps one maintenance address per store so the collector can still visit a released store.

The home records each directory and its analysis stores in a separate SQLite database.
A scan or watch opens the store and registers it before acquiring a generation.
The registry remembers the highest generation issued for this home. Each stop reserves
the next generation there while it holds the control database's write transaction.
Rolling back that transaction can leave a gap between generations.
Acquisition also checks the store's numbered release receipts. A scan after restoration
of an older control database must acquire a generation above those cutoffs.
Already running scans keep their original generations.
A numbered stop from a home also releases older directory owners that have no home.
The store records its last acquisition number at delivery. An earlier writer without a home cannot publish under that stop's cutoff.
A later acquisition can use the directory again. Replaying the receipt keeps the original acquisition boundary.
A numbered parent stop covers earlier child stops in the same home. The store can then discard their receipts.
It keeps a child receipt if the child has a higher cutoff. It also keeps receipts for other homes.
Replaying a covered child stop after restart must preserve all work acquired after the parent stop.
During upgrade, an older child receipt can cover more unqualified acquisitions than its parent.
The store keeps that receipt until another stop covers both its generation and its acquisition number.
An older receipt has no acquisition number. Upgrade uses the store's current acquisition number to fence outstanding writers before accepting new ones.
The native run and workspace APIs accept nonnegative safe integer generation numbers.
A numbered stop receipt contains its cutoff, a delivery ID, and an optional home, separated by colons.
A malformed numbered receipt is an error. An older unnumbered delivery ID still identifies a single unconditional stop.
It must register the store before it can publish ownership.
This order lets a stop find every store that can hold an ended generation.
If the store fails to open, the run uses memory and adds no registration.
The stop remains pending until each store for the directory accepts its release.
Named revision scans also register their content store for maintenance before evaluation.
Maintenance can reach that store after a restart or a change to the store override, including after deletion of its last named scope.
The stop records the physical directory path when it enters the queue.
Delivery and later probes use that path as a stored scope key, even if a symlink replaces the directory.
The generation cutoff preserves a new watch that acquires the same path after the stop.
Directory registrations record the store's SQLite identity as well as its path.
Maintenance also records the store identity. Collection checks the open database before it can change its schema or data.
The collector uses that same connection for its work. A missing store remains eligible for the next maintenance pass.
Maintenance also records which lock file belongs to the open store. If that file disappears or changes, collection defers until the original pair returns.
After a scan or named release, a missing expected store produces a collection error.
Registrations from older versions can contain only a path. The collector checks for an existing content-store schema at those paths.
A numbered release opens an existing database and checks its identity on the connection that delivers the release.
If a different store occupies that path, the original store keeps its pending release.
An older registration contains only the path. It cannot identify a store replaced before identity registration.
For a store from before directory registration, the stop first checks for matching ownership rows.
A store can hold these rows for one directory and have registrations for other directories.
A legacy candidate outside the current process's store access does not delay a stop.
If access is unavailable or the ownership query fails, the stop records a deferred probe for that store.
This also covers a damaged database header. Repairing the file lets the next probe find its owners.
A permanently invalid legacy path therefore continues to receive probes.
A configured store can be absent at the time of the stop. The stop records a deferred probe for that path too.
The probe keeps the original stop generation, home, and directory.
The first failure schedules a retry after five seconds. Each subsequent failure doubles the delay up to one hour.
The control database records the number of failed attempts, the latest error and the next attempt time.
These values survive a daemon restart. A successful probe removes its retry record.
If the query finds matching ownership, the daemon delivers the original release.
If the query proves that the store has no matching ownership, the daemon deletes the probe.
A store with a directory registration remains a release target while it is unavailable.
The daemon handles due releases in order of their next attempt time.
If a store is unavailable, the daemon saves a later attempt time for that release.
Other releases can make progress while the store is unavailable.

Before a test deletes a store, it waits for active upkeep steps to close their SQLite connections.
It removes the store's upkeep jobs before it deletes the files.
The retirement test uses the frame collector's read connection with a transaction that reads from a populated WAL.

The revision is an input to selection and provenance. It is absent from an
immutable tree record's cache key. That key includes the repository, object
ID, and decoder version. Document caches continue to depend on bytes, grammar,
and parser and tape versions. Path and module identity remain inputs to
context-sensitive rules. An unchanged file retains its cache key across
revisions. A changed file requests new content, and changes to the imports or
configuration invalidate their dependents.
Each language key describes the registry entries that match the file name.
On Windows, the parser looks up the selected grammar using the same slash-separated path that the Git inventory emitted.
The cached result selects the grammar from these entries. Files with the
same entries can use the same classification. A file without a match uses
the empty set of entries. An unrelated registry entry leaves the key intact.

Concurrent requests for the same source bytes share a tape build.
The source phase retains recent builds within the existing memory budget.
Each path still has its own document and module facts.
Python reference analysis reads the same tape.
It then links each document's references with the package and path facts for its placement.

The Git view stays pinned during a run. Different revisions can use the same
cache space, provided the engine compares their input facts before it reuses
derived rows. Each report uses the inputs and source view of its own run.

A rule evaluation checks the pack layout when it starts and again when it finishes.
Its workers share the first check. Direct reads through the native source API check the layout on entry.
In a fixture with 12 packs, both 4 and 64 unchanged paths took two checks of 26 storage paths.
Both warm profiles recorded zero tree decodes.

A pack layout records which indexes can supply each object ID prefix.
Each lookup still checks the loose object paths.
An in-place rewrite can preserve a Git object's length and modification time.
The storage observation must detect the rewrite before a view or persisted inventory reuses that object.
On Windows the observation also needs the file's change time. An unavailable change observation requires verification of the object.
Small indexes list the first bytes of the object IDs they contain. A lookup can skip a small
index when the requested ID starts with a different byte. Every candidate index contributes
its supplier, including when several packs contain the same object.

With 12 unrelated single-object packs, a warm scan of 4 directories went from 60 index probes to zero.
With 64 directories, the count fell from 780 to three. Both scans still returned one finding per directory and decoded zero trees.
For a larger index, the observer first uses ordinary Git lookups.
After one unsuccessful lookup per 32 indexed objects, it can keep a bitmap of ID prefixes.
A clear bit rules out that pack. A set bit still requires the full ID lookup.
The bitmap uses at most 64 KiB per index. Indexes too dense for this bitmap keep ordinary lookups.

A 64-directory scan also ran against 12 unrelated packs of 257 objects each.
Three pairs of cold and warm scans used each implementation. The implementations alternated on the same macOS machine.
Each scan found 64 violations. Cold scans decoded 65 trees. Warm scans decoded zero.

| Measurement | Ordinary index lookup | Adaptive prefix lookup |
| --- | ---: | ---: |
| Warm index probes | 780 | 149–152 |
| New bitmaps per process | 0 | 6,144 bytes for 12 bitmaps |
| Median cold scan | 451 ms | 427 ms |
| Median warm scan | 178 ms | 169 ms |
| Median warm process peak RSS | 309,040 KiB | 308,880 KiB |

The one-minute load average at the end was 20. The first ordinary cold scan took 8 seconds.
With the project object store as an alternate, warm probes fell from 1,281 to 804–811.
Those scans allocated 11,264 bitmap bytes. Their median warm times were 189 ms and 214 ms in separate batches.
The probe reduction alone does not establish an end-to-end speedup.

A separate benchmark opened 21 pack indexes containing 252,499 entries. One in 16 requests named an indexed object.
The measured interval included opening indexes, constructing routing data, and finding all suppliers.
Median times for 4,096 lookups fell from 3,226 to 2,129 microseconds.
For 16,384 lookups, the medians were 11,149 and 5,773 microseconds.

In a two-file Python fixture, three separate native processes used the same
store and scope. The initial scan had zero file memo hits. A new commit with
the same tree had two hits. After one file changed, the next scan had one hit
and reported the changed finding for that file.

A TypeScript AST policy run through `rerail scan` on a separate two-file fixture
stored two tapes and two check results. A new commit with the same tree added
neither. Editing one file added one tape and one check result. Only the untouched
file still had a finding.

In a separate test, each run opened the same store in a new process.
An unchanged scan ran zero classification strata and zero source parsers.
A registry edit selected a new file for parsing.
A parser version change with the same registry parsed the two selected files.
It ran zero classification strata.

The headless runner uses the selected content store when it first reads the policy and its includes.
In a two-file Python reference fixture, the cold profile counted nine tape parses.
A second process using the same scope and store reported the same finding with zero tape parses.

Five separate processes checked an unchanged Python file with a policy that reports the file.
Each process reported `a.py`. The fixture used two named scopes and a separate empty store for comparison.

| Run | Embedded programs compiled | Embedded programs reused | Tape parses across all inputs |
| --- | ---: | ---: | ---: |
| First scope in an empty store | 3 | 0 | 8 |
| Second scope in the same store | 0 | 3 | 0 |
| Second scope after deletion and collection of the first | 0 | 3 | 0 |
| Independent empty store | 3 | 0 | 8 |
| Original store after deletion and collection of its last scope | 3 | 0 | 1 |

The last run compiled all three programs again. The tape counter also includes policy and library reads.

## Recovery contract for a stopped directory

The control database's `.analysis-releases.sqlite` file keeps the committed release ID, directory, acquisition cutoff and registered store identities until delivery completes.
It uses SQLite's rollback journal so its attached connection uses full synchronization. The existing store registry continues to use WAL.
A rolled-back stop leaves no release to deliver. An acknowledged stop must survive restoration of an older control database.
After a restart, the worker rebuilds missing delivery work from the registry. A repeated delivery uses the original release ID and cutoff.
A store that received the release before the worker stopped can acknowledge it again. A run acquired after the cutoff keeps its ownership.
A database that takes another store's pathname cannot acknowledge a release addressed to the original store.
When restoring only the control database, keep both the release journal and the `.analysis-stores.sqlite` registry.
For a full backup, include consistent SQLite backups of all three databases and the content stores you intend to restore.
The worker imports pending receipts from earlier versions the first time it opens their queue.
