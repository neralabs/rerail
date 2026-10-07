# Rule Scoping

Rerail keeps rule definitions and execution history in one user-level SQLite database, `$RERAIL_HOME/rerail.sqlite` (`~/.rerail/rerail.sqlite` when `RERAIL_HOME` is unset). Rules are resolved for a repository through its project; sessions, events, work links, and policy runs are attributed to the repository identity of the checkout that recorded them.

## Identities

A **project** is a machine-local stable identity grouping an explicit set of repositories. A **repository** is a stable local identity. A **checkout** is one working copy of that repository, including a linked Git worktree. A **session** is execution history recorded by one checkout and attributed to its repository.

For Git repositories, Rerail atomically creates `rerail-repository-id` in Git's common metadata directory. Linked worktrees therefore inherit the same identity after path moves, while each clone or fork receives a random identity even when a filesystem path is reused. An older `rerail.repository-id` Git config value is adopted on first migration. Non-Git workspaces store an equivalent private identity in `.rerail/repository-id`.

Repository membership is explicit. Rerail never infers shared ownership from a remote URL, directory ancestry, repository name, or account. A repository belongs to exactly one project at a time; assigning it to another project moves the membership. A newly discovered repository starts in a private default project named from its checkout. Project membership is local catalog data and is not written into the repository.

## Applicability and precedence

Rule applicability is independent from evaluation semantics:

- `global`: applies to every repository on the machine.
- `project`: applies to every repository explicitly assigned to the current project.

Policy execution `scope` and `mode`, and cosine sample `source` and `boundary`, continue to determine what and when a rule evaluates. They do not determine where the rule is installed.

**Rule Path Criteria** are repository-relative `scope.include` and `scope.exclude` patterns on a policy. They filter candidate paths after the rule's applicability has selected it; excludes win. They are distinct from applicability, and from policy execution `scope` and `mode`.

The effective rule set contains one definition per logical rule ID:

1. A project definition overrides a global definition with the same ID.
2. A disabled project definition masks an enabled inherited global definition.
3. Deleting the project definition reveals the global definition again.
4. Deleting the global definition removes it for every project that does not override it.

A logical ID cannot change rule kind while any global or project definition with that ID exists. Remove the existing definition before changing between `policy` and `cosine`.

## Propagation and concurrency

Every daemon, extension, and hook opens the shared catalog and resolves its effective rules from its repository identity. SQLite transactions serialize edits. Live policy and cosine catalogs use change tokens combining SQLite `data_version` with connection-local revisions, so commits through the same process or another connection become effective on the next refresh or event without restarting. Restarting reconstructs the same effective set from the catalog.

A write records applicability, owner, revision, creation/update timestamps, a deterministic payload hash, catalog origin, sync state, and optional upstream rule ID. These fields make local provenance explicit and reserve the synchronization boundary for NER-1453 and NER-1457; this change does not add a cloud transport or change Nera's future source-of-truth role.

Each policy run snapshots the effective rule ID, revision, payload hash, applicability, project identity when relevant, catalog origin, sync state, and upstream ID from the compiled workspace that performed the evaluation. Later edits or deletions do not rewrite historical runs.

## Migration

On first use for a repository and legacy source, Rerail copies checkout-database rules and configured host/JSON cosine rules into the current project, then records the source in `rule_migrations`. A source claim and its writes commit atomically, so concurrent first use is idempotent. Legacy rows marked `global` become project rules deliberately: old checkout-local storage could not establish machine-wide intent. Original revisions, timestamps, enabled state, source digest, and rule bodies are retained. Existing catalog definitions win import conflicts.

Deleting a migrated project rule records a catalog tombstone. A previously unseen linked worktree can therefore import unique legacy rules without resurrecting a rule the project already deleted.

Migration does not move sessions, events, work links, or policy runs. A pre-catalog checkout database remains intact as an archival copy of its history, but runtime resolution and all new writes use the Rerail database.

`rerail rules export` writes a version 2 dump containing both global definitions and current-project definitions, including shadowed global rows. `rerail rules import` preserves applicability and provenance, attaching project definitions to the destination repository's current project. `rerail reset` deletes only the current repository's execution history and leaves the catalog untouched.

## Management surfaces

The HTTP/OpenAPI and MCP operation registry exposes:

- `rule_context_get`: current repository and project identity.
- `project_repository_assign`: assign or move an explicit repository identity into a project.
- `project_repository_remove`: remove membership; removing the current repository places it in its private default project.
- Existing rule list/get/upsert/enable/remove operations, with `applicability` and origin/provenance in rule responses.

The same operation definitions drive HTTP and MCP, so filters and writes have one contract. CLI import, export, seed, and reset always use `$RERAIL_HOME/rerail.sqlite`.

## Operational scenarios

- Two linked worktrees resolve the same repository ID and therefore the same project rules.
- Two unrelated repositories assigned to one project share project edits, disables, and deletions.
- A third unassigned repository receives global rules but no rules from that project.
- Removing a repository from a multi-repository project immediately removes that project's rules and assigns the repository to its private default project.
- Moving a checkout path updates only `last_known_path`; rule identity and history location semantics remain unchanged.
- Clone and fork identities are isolated until explicitly assigned.
- A catalog edit survives daemon and harness restarts because applicability is catalog state, not process state.
