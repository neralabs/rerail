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

## Language Conformance Layer

The validation of how well the engine reads each language, on upstream's tests and on real code: grammar parity, capability oracles, corpus differentials, host routing, the Language Registry, and its Known Parser Gaps. It judges the engine and its languages, never a single rule, and passing it is not enough for a rule to ship. It complements the Rule Validation Layer; the two overlap only where check 4 scores a port with the conformance runner.

## Rule Validation Layer

The validation every Pack Rule passes before it ships: checks 1 to 9 of the pack validation contract (`docs/pack-format.md`), each made of Sub-checks. Checks 1 to 6 are a pack's own bar; checks 7 to 9 (provenance, generality, size) and the upstream-test Sub-checks of check 4 judge a port against its Upstream's Source Facts and Recordings.

## Pack Rule

A Rule a pack's `pack.json` lists, with its `rerail.policy/v1` document in `packs/<id>/rules/`. It is proven by examples it must catch and let pass, or, as a port, by reproducing one Upstream rule, recorded in the pack's `conformance.json`.

## Upstream

The public tool a port reproduces (Ruff, Bandit, ESLint and its plugins, PHPStan, TLint, …), pinned in the pack's `conformance.json` to one release, with its source repository at one commit. Upstream's own tests and output at that pin are the only evidence of what a port should report; nobody writes upstream's expected findings by hand.

## Recording

An Upstream tool's findings, recorded offline by the pack's recorder from the pinned tool and committed under `upstream/<tool>/`: on its own tests copied into the pack (`fixtures.expected.json`) and on each pinned corpus (`corpus-<name>.expected.json`). Each finding carries the parts of its position the tool reports: its start and, where reported, its end. Validation reads Recordings and never runs the tool.

## Source Facts

What an Upstream tool's own source says about each ported rule, recorded with its Recordings in `upstream/<tool>/source.json`: the pins, the file types the Approved Configurations lint, the rule's implementation and test files and which of those tests its Recordings leave out, whether upstream deprecates it, the Approved Configuration that turns it on, the names its code checks, and every word in upstream's tests. Checks 7 to 9 and check 4's coverage read them. For a tool without Source Facts those checks are not judged, which never counts as a pass.

## Approved Configuration

An Upstream configuration a pack counts as recommended for a tool, listed in order in the tool's `recommended` in `conformance.json` (Ruff's and Bandit's `default`, ESLint's `recommended`). A port is recorded with the options of the first one that turns its rule on, and a rule none turns on fails `provenance:recommended`. Listing them declares the tool's Source Facts.

## Sub-check

One named part of a validation check, under which every failure is filed: the check's own name for its original bar (`rules`, `examples`) or `<check>:<name>` (`ports:span`, `provenance:recommended`). It judges each Pack Rule, each tool or the whole pack; only some can be covered by a Scrub List. A Sub-check belongs to validation, a Clause to a rule document.

## Scrub List

The committed list of (Pack Rule, Sub-check) failures a pack's existing rules have today, `packs/<id>/scrub.json`, to clear before those rules go live. A covered failure makes the rule `scrubbed` instead of `failed`, an entry the rule now passes fails as stale, and a guard refuses entries for new or changed rules, so the list only shrinks. Its entries are either fidelity defects of a port or upstream-policy exclusions (upstream deprecates the rule, or no Approved Configuration turns it on).

## Known Parser Gap

Source a language accepts but the engine's grammar cannot parse, listed on purpose in the ledger in `conformance/t1/` with the grammar, its version and the smallest probe that fails to parse. A test proves every probe still fails, so a gap a grammar fix closes is stale and must be removed. It belongs to the Language Conformance Layer and excuses no failure in the Rule Validation Layer.
