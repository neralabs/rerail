# Rule Domain

## Rule

A named coding constraint with scope, enforcement, detection, repair guidance, and an independent ground-truth check.

## Generation Check

A rule evaluation performed against generated content before the generation is accepted. The Audit that runs it records its Violations.

## Exact Generation Check

A Generation Check whose result is equivalent to the Ground-Truth Oracle for the same content and context.

## File Projection

A Generation Check evaluated from one generated file without the complete repository context. It is fast but is not Exact until parity is proven for the rule.

## Ground-Truth Oracle

The authoritative source check used to determine whether another representation preserves a Rule's behavior. It is a checkpoint check, not the generation-time implementation.

## JIT Gap

Context or behavior required by a Ground-Truth Oracle that is unavailable or not yet proven equivalent in a Generation Check.

## Code Rule

A Rule over repository source files (kind `code`): one versioned, customer-authored `rerail.policy/v1` document holding its Clauses, its Where It Applies path patterns, rationale, and Rule Mode. Its applicability is stored separately in the machine rule catalog.

## Clause

One constraint inside a Code Rule: an `id`, exactly one form (`forbid`, `require_each`, `budget`, `llm`, or `define` for shared definitions), and, for every form except `define`, the Violation it reports.

## Violation

What a Clause reports where code breaks it: a stable id, the `ruleId` and `clauseId` that produced it, code, message, path, span, and the Rule Mode of its rule. An execution failure is not a Violation.

## Conversation Rule

A Rule over the agent conversation (kind `conversation`): a standing semantic query over one event source and boundary, with a Rule Mode and an action. Stored in the machine rule catalog and refreshed by running engines after catalog commits, including writes made through the same process or another connection.

## Audit

Any run of rules over a session, live or re-run: the automatic evaluation of generated edits, an explicit audit request, or a full repository audit. An Audit has a scope (`repository`, `touched-files`, or `ai-output`) and a run mode (`steer` or `log`) and records its Violations and execution failures.

## Rule Mode

What Rerail does when a rule of either kind fires, set once on every rule document: `log` records the match or Violation for review and never sends guidance, `steer` sends the rule's guidance to the agent, and `block` stops the action. Until block enforcement lands, a `block` rule is delivered exactly like a `steer` rule. Violations carry the mode of the rule that produced them. An Audit's run mode (`steer` or `log`) is a separate setting with overlapping words: it decides whether a whole Audit sends guidance at all.

## Rule Applicability

Where a rule definition applies: `global` across the user's machine or `project` across the repositories explicitly assigned to one project. Applicability is independent from a Code Rule's path criteria and a Conversation Rule's sample boundary.

## Where It Applies

Repository-relative `scope.include` and `scope.exclude` patterns on a Code Rule. After Rule Applicability selects a rule, these criteria filter its candidate paths; excludes win. They are distinct from applicability and from an Audit's scope and run mode.

## Project

A machine-local, stable identity grouping one or more explicit repository identities for shared rule applicability. A repository belongs to exactly one project at a time.

## Repository and Checkout

A repository has a stable local identity stored in Git's common metadata directory (`rerail-repository-id`). Its linked worktrees are separate checkouts that inherit that identity; clones and forks receive new identities and require explicit project assignment.

## Rerail Database

The one user-level SQLite file (`$RERAIL_HOME/rerail.sqlite`, normally `~/.rerail/rerail.sqlite`) holding the rule catalog and every repository's execution history.

## Rule Catalog

The projects, repository membership, global rules, and project rules in the Rerail database. The engines in each coding agent evaluate effective rules but do not own definitions.

## Execution History

The sessions, events, work links, and Audit provenance in the Rerail database. Each session carries the repository identity it ran in; per-repository views and `rerail reset` select by it.

## Work Link

A loose, un-enforced association from a session to an external reference such as a git branch, pull request, or ticket.

## Semantic Program

A language-independent, versioned representation of source structure whose stable node kinds, fields, identities, and source spans support the same policy concepts across parser grammars.

## Resolved Fact

A versioned, provenance-bearing semantic assertion attached to a node in a Semantic Program, such as a resolved symbol, framework role, inferred query kind, or transaction relationship.

## Structural Retrieval Policy

A customer-authored, Semgrep-style expression over a Semantic Program and its Resolved Facts. It uses structural patterns, metavariables, ancestry and descendant constraints, fact predicates, and boolean composition without exposing parser-native node shapes.

## Compiled Retrieval Policy

A deterministic, versioned executable form of a Structural Retrieval Policy for one Semantic Program schema.

## Selected Policy Context

The bounded, deterministic set of matched Semantic Program nodes and Resolved Facts supplied to a policy evaluator.

## Language Registry

The one list of the languages Rerail knows (`languages/<id>.json`): for each, its family, dialects, file suffixes, per-capability support and status, `supported` or `planned`. The engine and the host both read it; nothing else branches on a language.

## Language Pack

A language's registry entry, its engine adapter, and the rule packs written in it. A Code Rule for a `planned` language, or one reading a capability its language does not support yet, is stored but inactive: compiling it is refused as `language-not-supported`.
