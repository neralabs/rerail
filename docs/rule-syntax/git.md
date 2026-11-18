# Git in native rules

Include `git.rr` in a program that runs with `rerail scan --rr`, `rerail-engine logic`, `run-rr`, or `run-policies`.
The invocation supplies `git_root_repo(repo)` when the source root belongs to a local Git repository.
Treat `repo` as an opaque value. Pass it to each request and join on it in each answer.

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
Commit message searches use Git's regular expression syntax.
The `git_revision_error` reason is one of these symbols.

| Reason | Meaning |
| --- | --- |
| `missing-revision` | No name, object prefix, or commit message matched |
| `ambiguous-revision` | The expression matched more than one object |
| `invalid-revision` | Git could not interpret the expression or follow its navigation |
| `missing-object` | A required object is absent from local storage |
| `missing-path` | A tree lacks the requested entry |
| `wrong-object-kind` | A step requires a different Git object kind |
| `unreadable-object` | A Git storage read failed |
| `corrupt-object` | The bytes failed the object checksum |
| `repository-not-open` | The repository argument has no open repository |

The resolver checks the objects it reads during navigation before it follows their IDs.

## Relations

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
| `git_header` | Commit ID, position from zero among extra headers, name, hex value |
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
A successful object-kind row gives the completeness signal for that object's decoded rows.
A root commit has a successful kind row and zero parent rows.
A request for a missing or unreadable object has an error row and zero successful rows.

The first implementation opens the invocation's repository. It can read any explicitly requested object in that repository.
The legacy `git_rev`, `@git_commit`, `@git_has`, and `@git_tracked` still work.
`@git_tracked` reads the current index. Use `git_tree_entry` for historical tree membership.
