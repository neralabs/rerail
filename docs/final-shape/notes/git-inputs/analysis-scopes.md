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
excludes those entries. Its `.rr` rules request each child through the path and
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
The inventory also uses verified tree entries from earlier policy requests in the same scan.
An explicit path under a missing tree gives a source error for that path.
The scan still checks other requested paths whose trees it can read.

## Retention and invalidation

Scope membership is a relation. `analysis_scope_manifest(scope, manifest_id)`
connects a scope to each corpus run manifest. Existing manifest facts connect
the run to its sources and plans. `analysis_scope_snapshot(scope, snapshot_id)`
connects a named scope to each of its audit snapshots. A directory scope keeps
the latest snapshot for each workspace in `analysis_scope_workspace_snapshot`.
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

The home records each directory and its analysis stores in a separate SQLite database.
A scan or watch opens the store, registers it, and then reads the generation.
It must register the store before it can publish ownership.
This order lets a stop find every store that can hold an ended generation.
If the store fails to open, the run uses memory and adds no registration.
The stop remains pending until each store for the directory accepts its release.
For a store from before directory registration, the stop first checks for matching ownership rows.
A store can hold these rows for one directory and have registrations for other directories.
A legacy candidate outside the current process's store access does not delay a stop.
If access is unavailable or the ownership query fails, the stop records a deferred probe for that store.
A configured store can be absent at the time of the stop. The stop records a deferred probe for that path too.
The probe keeps the original stop generation, home, and directory.
The daemon tries the probe again after a delay, independently of release delivery.
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
