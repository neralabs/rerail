# Proposed Git library relations

Status: design contract. See [Git in native rules](../../../rule-syntax/git.md) for the implemented interface.
They describe the intended data and dependencies before implementation.
[design.md](design.md) gives the user operations. [primitives.md](primitives.md) gives the native boundary.

## 1. Repository and invocation inputs

The signatures use `symbol` for proposed opaque values until their nominal types are settled.
A value's contract still distinguishes repository IDs, algorithm-qualified object IDs, content IDs, and source views.

```prolog
.decl git_root_repo(repo: symbol)
.decl git_root_view(repo: symbol, view: symbol)
.decl git_repository(repo: symbol, kind: symbol, format: symbol)
.decl git_repository_dir(repo: symbol, path: symbol)
.decl git_worktree(view: symbol, path: symbol)

.decl git_snapshot_request(request: symbol, repo: symbol, revision: symbol, prefix: symbol)
```

`git_root_repo` contains one repository for an invocation with a Git root.
`git_repository_dir` names the common Git directory shared by linked worktrees. `git_worktree` names the opened view's working directory and has no row for a bare view.
The repository identifies the clone. The view identifies the opened worktree's reference context.
No request changes the repository argument of another predicate.
A scan supplies one snapshot request. A rule can issue additional object lookups for its own evidence.

The ordinary invocation does not require a programmer to write a repository path:

```rr
#include "git.rerail"

.decl empty_head_message(repo: symbol, view: symbol, commit: symbol)
git_view_revision_request(v, "HEAD") :- git_root_view(_, v).
git_object_request(r, c) :- git_root_view(r, v), git_view_revision(v, "HEAD", c).
empty_head_message(r, v, c) :- git_root_view(r, v), git_view_revision(v, "HEAD", c), git_message(r, c, "").
.output empty_head_message
```

The executable example requests the commit explicitly. Section 6 describes the remaining work on direct bound lookup.

## 2. Mutable names and immutable objects

```prolog
.decl git_view_revision(view: symbol, expression: symbol, oid: symbol)
.decl git_revision(repo: symbol, expression: symbol, oid: symbol)
.decl git_object(repo: symbol, oid: symbol, kind: symbol, content: symbol)

.decl git_commit(repo: symbol, commit: symbol, tree: symbol)
.decl git_parent(repo: symbol, commit: symbol, position: number, parent: symbol)
.decl git_tag_target(repo: symbol, tag: symbol, kind: symbol, target: symbol)
```

`git_view_revision` resolves a name or a single-object expression in the requested view. A fully qualified ref can be its expression.
`git_revision` resolves a standard single-object Git expression.
It does not interpret a two-dot expression as an arbitrary single object.
Rules that compare endpoints resolve each endpoint explicitly.

`git_object` records successful acquisition of an object payload from an authorized source.
It does not assert that the object remains in the local Git directory forever.
The object kind is `commit`, `tree`, `blob`, or `tag`.
The payload's bytes remain unchanged under the full object ID.
A content handle refers to those bytes in the ordinary source store.

The library derives commit trees, parents, and tag targets from decoded object data.
A commit has one tree. A root commit has a complete empty parent family.
Parent positions start at zero and preserve commit order.
A rule can query a parent independently of the source snapshot.

## 3. Metadata

```prolog
.decl git_message(repo: symbol, commit: symbol, text: symbol)
.decl git_subject(repo: symbol, commit: symbol, text: symbol)
.decl git_author(repo: symbol, commit: symbol, name: symbol, email: symbol, seconds: number, offset_seconds: number)
.decl git_committer(repo: symbol, commit: symbol, name: symbol, email: symbol, seconds: number, offset_seconds: number)
.decl git_header(repo: symbol, commit: symbol, position: number, name: symbol, content: symbol)
.decl git_trailer(repo: symbol, commit: symbol, position: number, name: symbol, value: symbol)
```

`git_message` exposes the full message, including its final newline, after the selected text-decoding policy succeeds.
Raw message bytes remain available through the decoded content handle.
`git_subject` and `git_trailer` are library rules over message content.
The trailer relation preserves ordered duplicates and unfolds continuation lines from the final trailer block.
The [rule reference](../../../rule-syntax/git.md#read-commit-trailers) shows how to inspect it.

Signatures preserve name, email, epoch seconds, and timezone offset separately.
Header content preserves unknown headers and signature payloads without introducing a builtin for every header name.
No field silently substitutes replacement text for invalid bytes.

Reusable metadata rules keep repository and selection explicit:

```prolog
check_commit(repo, commit) :-
    git_root_view(repo, view), git_view_revision(view, "HEAD", commit).

empty_message(repo, commit) :-
    check_commit(repo, commit), git_message(repo, commit, "").
```

Another rule can derive `check_commit` rows from a range or another explicit repository.
The relation selects the domain of this check. It does not change what `git_message` means.
An unbound query needs a finite domain from refs, a range, or explicit object requests.
A compiler must reject an unsupported enumeration mode or report the required domain.
It must not silently treat undemanded objects as absent.

## 4. Authoring history queries

```prolog
.decl git_walk_request(request: symbol, repo: symbol, start: symbol)
.decl git_reachable(request: symbol, repo: symbol, start: symbol, commit: symbol)
.decl git_history_complete(request: symbol, repo: symbol, start: symbol)
.decl git_range_member(request: symbol, repo: symbol, commit: symbol)
```

These are example relations a program can derive from the Git primitives. The scan command does not emit range requests.
A reachable set includes its start commit.
A walk follows every recorded parent. First-parent behavior is an explicit projection over parent position zero.
The library requests parent commits as it discovers their IDs.
Reachability does not use timestamps to decide ancestry.

The positive recursion is conceptually:

```prolog
git_reachable(q, r, start, start) :-
    git_walk_request(q, r, start), git_commit(r, start, _).

git_reachable(q, r, start, parent) :-
    git_reachable(q, r, start, child), git_parent(r, child, _, parent).
```

The request rules for parent objects are part of the same library phase.
These two rules alone do not establish completeness.
The phase must settle its positive request closure and account for every requested commit outcome.
It can then publish `git_history_complete` or an incomplete outcome.
Negation does not participate in a cycle that decides whether more parents must be fetched.

A program can derive two-dot range membership from complete endpoint histories:

```prolog
git_range_member(q, r, c) :-
    git_range_endpoints(q, r, base, tip),
    git_history_complete(q, r, base),
    git_history_complete(q, r, tip),
    git_reachable(q, r, tip, c),
    !git_reachable(q, r, base, c).
```

The program supplies or derives `git_range_endpoints` from its chosen endpoints.
A shallow boundary or missing parent prevents a complete result.
The library can expose discovered commits as partial evidence, with an explicit incomplete outcome.
It cannot publish that evidence as a complete checked range.

A range query is a set. Commit order is a separate requested relation.
An external driver can use its own order when it invokes one source scan per revision.

## 5. Trees and source placement

```prolog
.decl git_tree_entry(repo: symbol, tree: symbol, name: symbol, mode: number, target: symbol)
.decl git_tree_file(repo: symbol, root_tree: symbol, path: symbol, mode: number, blob: symbol)
.decl git_source_view(view: symbol, repo: symbol, tree: symbol, prefix: symbol, config: symbol)
.decl git_source_file(view: symbol, path: symbol, mode: number, blob: symbol)
.decl git_blob_content(repo: symbol, blob: symbol, content: symbol)
.decl source_document(view: symbol, path: symbol, module: symbol, grammar: symbol, profile: symbol, tape: tape, document: number)
```

`git_tree_entry` lists one immediate tree. It preserves entry names and numeric modes.
`git_tree_file` is a recursive library view over regular-file entries.
A raw subtree traversal can be retained by subtree identity, then placed under several path prefixes.
Path placement and exclusions do not require another native tree traversal algorithm.

The recursive path rules compose relative names using the existing path operations.
They distinguish tree, regular-file, executable-file, symlink, and gitlink modes.
Ignored paths and source scope are separate library filters.
Repository inputs such as package configuration come from the selected tree. The current policy defines what to check.
A tracked file's `.gitignore` match alone does not make it untracked or absent.

`git_blob_content` acquires bytes only for requested regular files.
The language library can select many files by suffix before any source content is needed.
Binary and unsupported-encoding outcomes remain explicit.

`source_document` binds a shared tape to contextual source identity.
Its document ID is local to the engine's numbering scheme and is not a durable cache key.
Its durable provenance is view, path, module, grammar, parser profile, and tape.
The existing `doc`, node, and span relations consume that placement.
A diagnostic can then recover repository, tree, path, and any selected commit through explicit joins.

## 6. Demand and completeness are part of the interface

A relational declaration alone does not specify when its native source is requested.
The implementation needs a binding-mode contract for each keyed public view.
For example, bound `(repo, commit)` in `git_message` must reach an object request through the library rules.

There are two compatible implementation techniques:

1. Existing demand translation propagates bound keys through the library to its request relations.
2. A library phase derives explicit object requests from finite selection inputs, then exports complete result families.

The first technique gives ergonomic direct lookup. The second makes whole-selection analysis explicit.
A first implementation must demonstrate both access needs with existing engine mechanisms.
A new implicit repository slot or a native implementation of each public relation does not satisfy this contract.

Each requested key has a complete success family or an explicit failure/incomplete outcome.
A complete zero-parent family permits root-commit reasoning.
An undemanded key or failed read does not permit the same reasoning.
Errors retain repository, operation, object or expression, and request provenance.

## 7. Compatibility and reporting

Existing `git_rev` and implicit-root functors remain implementation facts of the current engine.
Their compatibility treatment needs explicit agreement.
New public library signatures do not acquire hidden repository arguments to mimic those functions.
Tree membership and current index membership remain different relations.

A commit diagnostic refers to repository and commit, with an optional metadata field.
It does not invent a file named `commits` or `COMMIT_EDITMSG`.
A source diagnostic refers to a source view, path, and span.
The scan report records its one selected tree and the commit if the revision names one.
The report schema and display form remain open, rather than overloading the existing path field silently.
