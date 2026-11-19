# Git in native rules

Include `git.rr` in a program that runs with `rerail scan --rr`, `rerail-engine logic`, `run-rr`, or `run-policies`.
The invocation supplies `git_root_repo(repo)` when the source root belongs to a local Git repository.
Treat `repo` as an opaque value. Pass it to each request and join on it in each answer.
Linked worktrees share the repository value. Use `git_root_view(repo, view)` to identify the invocation's Git view.
Each linked worktree has its own view for HEAD, its reflog, and its index. A bare repository also has a view for resolving revisions.
An open source view tracks the Git directories it opened. If another checkout takes their place, evaluation fails.
Changing a ref inside the same checkout remains visible to a later evaluation.

Use `git_repository_dir(repo, path)` for the common Git directory. Use `git_worktree(view, path)` for the working directory of the opened view.
These relations return absolute paths on the host that runs the policy. Linked worktrees share a common directory. A bare view has no working directory.
During a revision scan, these paths describe the local repository that supplies the objects.

```rr
#include "git.rr"
.decl checkout(repo: symbol, view: symbol, common: symbol, work: symbol)
checkout(r, v, common, work) :- git_root_view(r, v), git_repository_dir(r, common), git_worktree(v, work).
.output checkout
```

Both path relations use the `text` rows of `git_location_of(repo, view, role, encoding, value)`. The roles are `common-dir` and `worktree`.
For a path outside Unicode, the input supplies lowercase hex with encoding `unix-bytes` on Unix or `utf16le` on Windows.
If the host cannot resolve a directory, its row uses `error` and gives the cause.

Use a view for a mutable name such as HEAD. This program returns the repository, worktree view, and commit together.

```rr
#include "git.rr"

.decl view_tip(repo: symbol, view: symbol, commit: symbol)
git_view_revision_request(v, "HEAD") :- git_root_view(_, v).
view_tip(r, v, c) :- git_root_view(r, v), git_view_revision(v, "HEAD", c).
.output view_tip
```

`git_view_revision_error(view, expression, reason)` retains the requested view on failure.
The repository convenience relation `git_revision(repo, expression, oid)` uses the invocation's root view.
For immutable data such as a commit message, pass the repository and the resolved object ID.

## Read one analysis scope's history

Include `analysis.rr` and derive `analysis_history_request(kind, key)` for each scope you want to read.
Use `"named"` with the name you passed to `--analysis-scope`. Use `"scope"` with an opaque scope ID from this store.
The answer includes the request key, scope ID, run ID, scan ID, directory, repository, tree, commit and workspace snapshot.
An absent scope has zero rows. Store read errors fail the query.

```rr
#include "analysis.rr"
analysis_history_request("named", "release").
.decl release_scan(scan: symbol, tree: symbol)
release_scan(id, t) :- analysis_history_of("named", "release", _, _, id, _, _, t, _, _), t != "".
.output release_scan
```

The store looks up completed runs by scope. Publishing or removing another scope preserves this query's cached result.
To enumerate every published run, read `analysis_run_record` or its projections such as `analysis_scope_run`.

## Find the Git blob behind a parsed document

Include `git/documents.rr` to join a parsed file to its source in the selected revision.

```rr
#include "git/documents.rr"
.language python
.decl origin(path: symbol, blob: symbol, grammar: symbol, parsed: tape, document: number)
origin(p, b, g, t, d) :-
    git_source_view(v, _, _, _, _), git_source_file(v, p, _, b),
    source_document(v, p, _, g, _, t, d).
.output origin
```

`git_source_view(view, repo, tree, prefix, config)` identifies the selected tree, directory within it, and file selection. The root prefix is empty.
`git_source_file(view, path, mode, blob)` lists the selected regular files, including executables. Paths use Git separators and start at the view's directory.
The inventory applies the invocation's path filter and the selected tree's ignore rules. Listing files requires their tree entries.

`source_document(view, path, module, grammar, profile, tape, document)` identifies a primary document the policy actually parsed.
The grammar comes from the parser. The profile identifies the grammar build and tape format.
A policy's document filter can exclude a file that the Git inventory lists.
Two paths containing the same bytes can share a blob and a tape while retaining different document IDs.
Document IDs belong to one evaluation. Use the view and path to identify a file across rules.

## Read the selected commit

```rr
#include "git.rr"

.decl selected_message(repo: symbol, commit: symbol, message: symbol)
git_object_request(r, c) :- git_source_commit(r, c).
selected_message(r, c, msg) :- git_source_commit(r, c), git_message(r, c, msg).

.output selected_message
```

`git_source_commit` holds the resolved commit for a commit snapshot.
`git_source_tree` holds the root tree for a commit or tree snapshot.
For a tree snapshot, `git_source_commit` has zero rows.

Request the object before using the absence of a metadata row. For example, a root commit has a readable commit record and zero parents.

```rr
#include "git.rr"

.decl parentless_commit(repo: symbol, commit: symbol)
git_object_request(r, c) :- git_source_commit(r, c).
parentless_commit(r, c) :- git_source_commit(r, c), git_commit(r, c, _), !git_parent(r, c, _, _).
.output parentless_commit
```

The `git_commit` join checks that decoding succeeded. A failed object request has a `git_object_error` row.
Without an object request, the parent relation can be empty even for a commit with parents.

### Read commit trailers

`git_trailer(repo, commit, position, name, value)` returns one row per trailer in a decoded UTF-8 commit message.
Positions start at zero. Repeated trailers retain their own positions, including duplicates with the same value. Names preserve their case.
Reading trailers requires the commit object. A metadata-only policy can read them when the source tree is unavailable.

```rr
#include "git.rr"
git_object_request(r, c) :- git_source_commit(r, c).
.decl reviewer(commit: symbol, position: number, person: symbol)
reviewer(c, i, who) :- git_source_commit(r, c), git_trailer(r, c, i, "Reviewed-by", who).
.output reviewer
```

For example, two `Reviewed-by` lines after a blank line give two rows. An indented continuation contributes to the preceding value.
The library trims the value and replaces each continuation newline and its indentation with one space.
The subject and earlier body paragraphs cannot supply trailers. A final paragraph containing only ordinary text has zero rows.

The library uses Git's default colon separator and comment character. It reads the stored message independently of local trailer aliases and commands.
Compare the rows with `git interpret-trailers --parse --no-divider` on the same message using default configuration. [Git trailer syntax](https://git-scm.com/docs/git-interpret-trailers)

### Standalone command results

`rerail-engine logic` exports every row of each declared output. A literal fact remains present even when the selected tree contains no documents.
A relation can contain both literal facts and rows derived from a file in the selected tree.

`rerail-engine run-policies` writes one JSON report to stdout. Its `runs` array follows the policy argument order.
A policy that fails has a run entry with its `path` and `error`. The other policies still contribute their findings to `diagnostics`.
A failed batch exits with status 1. A working-tree batch stages all its policies under one directory scope.
A revision batch stages them under its named scope. Both publish cache ownership after every policy and required source read succeeds.
A failed batch keeps the scope memberships from earlier successful runs.

If a required Git subtree or file is unreadable, `run-rr` and `run-policies` keep the available findings and list the affected paths in `sourceFailures`.
The report has `status: "failed"` and the command exits with status 1.
`logic` also exits with status 1 and identifies each unreadable path on stderr. Its output relations can contain partial results.
A policy that only reads commit metadata can succeed when the commit's tree is unavailable.
A cache warning alone leaves a successful analysis at exit status 0.

## Walk from HEAD

```rr
#include "git.rr"

.decl reachable(repo: symbol, commit: symbol)
git_revision_request(r, "HEAD") :- git_root_repo(r).
reachable(r, c) :- git_revision(r, "HEAD", c).
git_object_request(r, c) :- reachable(r, c).
reachable(r, p) :- reachable(r, c), git_parent(r, c, _, p).

.output reachable
.output git_object_error
```

The requests form a fixpoint. Each decoded commit can supply the IDs of more commits to request.
This example finds every parent it can read. Check `git_object_error` before treating the set as complete.
A shallow clone can lack some parents. Git requests use local objects and do not fetch.

To query `HEAD` when scanning yesterday's source, request `"HEAD"` as above.
The selected source has no effect on what that expression resolves to.
Explicit relative paths in rule expressions start at the worktree root of the named repository.
Paths in a bare repository can name tree entries from the tree root.
After a branch switch, `@{-1}@{0}` selects the previous branch's latest reflog entry.
A detached previous checkout supplies a commit ID and has no branch reflog for this suffix.
`@{upstream}` and `@{push}` resolve local tracking refs. With the default `simple` push mode, Git requires both tracking destinations to agree.
An explicit push refspec or a mirror remote supplies its own destination.
When a branch uses the local repository as its remote (`.`), its upstream can be another local branch.
Git uses the first configured merge ref as the upstream. A named remote maps that ref through its fetch refspecs.
Revision lookup reports invalid `push.default` configuration, including when the expression is `HEAD`.
A new evaluation on an existing source view reads the current Git configuration. Its selected source commit stays pinned.
Rerail accepts a single object wherever a revision is required. Git can interpret two dots in a path expression as a revision range.
For example, if `HEAD:./src/` exists, `HEAD:./src/..` names a range. Use `HEAD:./src/../` to select the parent tree.
Commit message searches accept POSIX extended regular expressions.
For example, `:/^release [[:digit:]]+` selects a commit by its message.
When multiple tips match at the same committer time, a global message search tries HEAD first.
It then tries the other starting refs in reverse ref-name order. An anchored search starts at its named commit.
The package tests compare message selectors with the installed Git on each host platform.
The `git_revision_error` reason is one of these symbols.

| Reason | Meaning |
| --- | --- |
| `missing-revision` | No name, object prefix, or commit message matched |
| `ambiguous-revision` | The expression matched more than one object |
| `invalid-revision` | Git could not interpret the expression or follow its navigation |
| `missing-object` | A required object is absent from local storage |
| `malformed-object` | A navigation step encountered invalid commit, tag, or tree data |
| `missing-path` | A tree lacks the requested entry |
| `wrong-object-kind` | A step requires a different Git object kind |
| `unreadable-object` | A Git storage read failed |
| `corrupt-object` | The bytes failed the object checksum |
| `repository-not-open` | The repository argument has no open repository |

The resolver checks the objects it reads during navigation before it follows their IDs.
A parent step requires a commit. A commit's tree link requires a tree.
Direct and tagged commit scans check the kind of a locally available root tree before reporting its ID.
A policy can still inspect commit metadata when the root tree is absent.
When an expression ends at a tree entry, the resolver compares the entry mode with the local object's kind.
It can still report the recorded ID for an absent final entry, including a commit in a submodule.
Revision navigation and object facts use the recorded Git objects, as with `git --no-replace-objects`.
To select a replacement explicitly, request its `refs/replace/<original-id>` ref.
Message searches report unreadable parent links on standard error and continue through readable history.
Global searches also skip unreadable starting refs, including a loose ref whose contents Git cannot parse.
If the readable history has no match, the search reports its read failure.

## Relations

Include `analysis.rr` to look up completed scans in the active content store.
Named and directory scope facts also include scopes with published cache ownership from an older store.
Those scopes can have zero recorded runs. A pending scan alone does not create a published scope fact.
A rule that reads run history requires a readable store. If the store is unavailable, that rule reports an error.
Other policies in the scan can still report findings. A readable store with no matching runs supplies zero rows.
This query returns the commits scanned under `release` in the current repository.

```rr
#include "analysis.rr"
#include "git.rr"
analysis_history_request("named", "release").
.decl reviewed_commit(scan: symbol, commit: symbol)
reviewed_commit(id, c) :-
    git_root_repo(repo), analysis_history_of("named", "release", _, _, id, _, repo, _, c, _), c != "".
.output reviewed_commit
```

Use `analysis_run_tree` to include scans that selected a tree hash. Use `analysis_run_snapshot` to find runs sharing a retained workspace snapshot.
The query sees earlier publications. Its own run appears after successful publication. [Run and scope relations](../final-shape/notes/git-inputs/analysis-scopes.md#retention-and-invalidation)

| Relation | Columns after `repo` |
| --- | --- |
| `git_repository` | Kind (`worktree` or `bare`), Git hash format |
| `git_revision_request` | A single-object Git expression, such as `HEAD` or `HEAD~2` |
| `git_revision` | Expression, full Git object ID |
| `git_revision_error` | Expression, reason |
| `git_object_request` | Full Git object ID in the repository's hash format |
| `git_object_kind` | Object ID, kind (`commit`, `tree`, `blob`, or `tag`) |
| `git_object_error` | Object ID, reason |
| `git_commit` | Commit ID, tree ID |
| `git_parent` | Commit ID, position from zero, parent ID |
| `git_message` | Commit ID, UTF-8 message including its trailing newline |
| `git_subject` | Commit ID, first line of the UTF-8 message |
| `git_author`, `git_committer` | Commit ID, name, email, Unix seconds, UTC offset in seconds |
| `git_signature_time` | Commit ID, role (`author` or `committer`), Unix seconds, UTC offset in seconds |
| `git_header` | Commit ID, position from zero among extra headers, name, hex value |
| `git_header_bytes` | Commit ID, position, hex name, hex value for an extra header with a non-UTF-8 name |
| `git_tree_entry` | Tree ID, immediate entry name, numeric Git mode, target ID |
| `git_tree_walk_request` | Root tree ID to walk |
| `git_tree_member` | Root tree ID, reached tree ID |
| `git_tree_walk_error` | Root tree ID, tree ID, reason |
| `git_tree_walk_complete` | Root tree ID |
| `git_tag_target` | Tag ID, target kind, target ID |
| `git_blob_digest` | Blob ID, SHA-256 of its bytes |

Request a walk with `git_tree_walk_request(repo, tree)`. The walk requests
subtrees through the same object relations.

```rr
#include "git.rr"

git_tree_walk_request(r, t) :- git_source_tree(r, t).
.output git_tree_directory
.output git_tree_file
.output git_tree_walk_complete
.output git_tree_walk_error
```

`git_tree_directory(repo, root, path, tree)` gives each directory path.
The root directory has the empty path.
Each directory fact requires a readable tree at that path. The walk still reports an unreadable or wrong-kind child by its recorded object ID.
`git_tree_file(repo, root, path, mode, blob)` gives each regular file.
Paths are relative to the requested tree. Raw `git_tree_entry` facts include
symlinks and gitlinks. Join on `git_tree_walk_complete(repo, root)` if your rule
requires all paths in that tree. A missing tree, an object of the wrong kind, or
a path with invalid UTF-8 prevents this fact. `git_tree_walk_error` identifies the cause.

An explicit walk reads the whole requested tree. The analysis inventory applies
the scan path selection and ignore rules before it requests child trees. A
policy that only requests commits can run without a source inventory.

Use `git_message_bytes` for messages with invalid UTF-8.
`git_message_encoding` gives the declared encoding as hex when the header exists.
`git_signature_bytes` and `git_tree_entry_bytes` expose signatures and names with invalid UTF-8 as hex.
Use `git_signature_time` to read the date even when the signature contains a non-UTF-8 name or email.
A non-UTF-8 extra header name appears in `git_header_bytes`. Header positions retain their original order across both header relations.
A successful object-kind row gives the completeness signal for that object's decoded rows.
A root commit has a successful kind row and zero parent rows.
A request for a missing or unreadable object has an error row and zero successful rows.

The first implementation opens the invocation's repository. It can read any explicitly requested object in that repository.
The legacy `git_rev`, `@git_commit`, `@git_has`, and `@git_tracked` still work.
`@git_tracked` reads the current index. Use `git_tree_entry` for historical tree membership.
