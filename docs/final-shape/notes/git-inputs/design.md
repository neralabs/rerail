# Git inputs and the rule library

Status: design and implementation notes. Section 1 records the agreed scope. The last section describes the implemented interface.

Git trees supply source content through `gix`. A `.rr` library selects commits, traverses trees, selects languages, and requests parsing.
The engine maintains expensive results through ordinary key/result facts.
Repository identity, source selection, content identity, and document placement remain separate values.

## 1. Constraints from the discussion

- A scan can use a commit or tree without a checkout or source extraction.
- Commit messages and other Git metadata are available to rules.
- A historical scan applies today’s policy to one selected commit or tree.
- External drivers or `.rr` programs can compose range checks and analyze findings across revisions.
- The normal command discovers the repository from the run root.
- `git_root_repo(repo)` gives rules an explicit repository value.
- Repository-dependent Git predicates take that value as an argument.
- Most semantics live in a `.rr` library over a small native interface.
- Parsing, language selection, and other retained results use the Datalog cache pattern.
- Documentation precedes further implementation.


## 2. Existing mechanisms and proposed changes

| Mechanism | Existing behavior | Proposal |
| --- | --- | --- |
| Cache ownership | [0103](../../decisions/0103-a-cache-is-two-rules.md) defines ordinary key/result rules | Use those rules for Git-derived results |
| Maintained facts | [0104](../../decisions/0104-everything-kept-is-facts.md) specifies one fact path | Add Git observations and answers to that path |
| Source phase | [sources.rerail](../../../../native/src/code_rule_dsl/typing/programs/sources.rerail) requests documents and retains tapes | Accept explicit source values as well as filesystem sources |
| Tapes | [0055](../../decisions/0055-embedded-languages-and-tapes-as-values.md) separates content from placement | Share content tapes across commits and paths |
| Source reads | [0300](../../decisions/0300-a-runs-source-files-read-once.md) hands one read through the run | Hand Git content through the same flow |
| Language selection | [language.rs](../../../../native/src/code_rule_dsl/language.rs) selects the longest matching suffix in native code | Expose registry facts and derive selection in `.rr` |
| Git helpers | [0142 section 6](../../decisions/0142-artifacts-cached-by-what-they-depend-on.md) describes bound native object questions | Keep bound access, with explicit repository arguments |
| Retained native mechanisms | [t33](../t33-every-cache-moved-to-facts.md) lists mechanisms that must move to facts | Add no new native freshness mechanism |

The existing parser backend stores tapes by source digest, grammar tag, grammar version, and tape format.
That backend does not establish that language selection or Git-derived results already follow the proposed logical cache contract.
Both the relation rules and the physical tape store need inspection during implementation.

The nera-data-layer library importer walks trees and reads blobs by object ID through `pygit2`.
Its useful precedent is direct object access and reuse by blob identity.
Its Python dictionaries are not the cache architecture for rerail.

## 3. User operations

The catalog keeps a live main checkout with a separate Git directory. It also keeps a registration while the checkout is inaccessible.
Before pruning, the writer checks that the same registration still exists and the checkout is still absent or unlisted.
A replacement directory needs its own identity to use checkout settings.
Each catalog records remote adoption independently. If an adoption transaction fails, registration can retry it.

### 3.1 One source snapshot

Proposed command behavior:

```sh
rerail scan
rerail scan --revision HEAD~1 --analysis-scope review
rerail scan --revision <tree-id> --analysis-scope review
```

The first command uses the existing working-tree behavior.
The other commands select immutable source from the repository containing the run root.
A user does not need to supply a `.git` path.
The existing project-directory selector also determines repository discovery for an explicit target.

The runner resolves a revision once and records its full object ID.
For a commit, the source tree is its tree. A raw tree has no invented commit, message, or ancestry.
The native rule runner preserves a subdirectory argument as a prefix in the selected tree.
The headless scan discovers the checkout root, as it does for a working-tree scan.
A missing historical prefix in a native run produces an error. It does not fall back to local files.

A bare repository is an explicit target. It has no implicit working-tree source.
A linked worktree keeps its own `HEAD` and index context.
A root inside a submodule selects the submodule repository.
Entering other repositories requires explicit repository values and source requests.

### 3.2 Policy and source configuration

A historical scan uses the rules, packs, settings, and approvals that the normal scan selects today.
Existing explicit rule-source overrides, such as `--rules-ref`, remain independent of `--revision`.
Picking a source revision never loads its `.rerail/rules/` as the policy under evaluation.

When a scan uses the home database, it reads eligibility, rule attachments, policy documents, and approvals from one committed SQLite snapshot.
A policy change during setup takes effect on the next scan.
The scan releases the read transaction before evaluating the source tree.
See the existing headless scan’s rule selection. [Headless scan](../../../headless-scan.md#which-rules-run)

Files that describe the source, such as `package.json` for module resolution, come from the selected tree.
Rule inputs that read repository files also read that tree.
Explicit external inputs retain their provenance.
An absent historical file remains absent even when the checkout contains a file at that path.

For example, a current rule can flag syntax in `src/a.ts` from `HEAD~1` even when the checkout has already removed it.
If that commit declares a different package mode, the parser and import resolver use the mode from that commit.
The user sees one finding at `src/a.ts` under the selected revision, attributed to the current rule’s exact version.

### 3.3 Report and composition

One historical scan selects one revision and produces one report.
The report records the resolved tree, the commit if the selection has one, and the current policy’s provenance.
The checkout’s `HEAD` and the scanned revision are distinct fields when both are present.
File paths and spans refer to the selected tree.

Rules can request metadata or tree entries for other explicit Git objects in the same evaluation.
Those reads supply evidence to the rule. The scan still has one source selection.
External drivers can invoke one scan per revision. `.rr` programs can join Git facts across objects.
Range invocation, repeated source scans, and cross-revision finding grouping are outside the product scope.

## 4. Identity and provenance

The following are different identities:

| Value | Meaning | Required distinction |
| --- | --- | --- |
| `repo` | The local clone | Linked worktrees share its immutable objects and run history |
| `view` in `git_root_view(repo, view)` | The Git context for mutable lookups | Linked worktrees keep their own HEAD, reflog, and index |
| `oid` | Full Git object ID, including hash algorithm | SHA-1 and SHA-256 IDs are distinct |
| `content` | Immutable bytes in the ordinary source store | Equal bytes can share content across repositories and Git formats |
| `tape` | Parsed content under an effective parser configuration | A grammar or parser change can produce another tape |
| `document` | A tape placed in a source view | Equal tapes can have different paths and module meanings |
| `request` | An explicit invocation or rule request | Selection does not change unrelated Git predicates |

Repository values are stable, serializable references to repository records.
They are not process addresses or mutable global defaults.
Their display facts include repository kind, object format, Git directory, and an optional worktree root.
The internal encoding needs agreement with the existing store identity model.

Object IDs in durable keys are full and algorithm-qualified.
Refs, abbreviated IDs, and revision expressions are mutable observations until resolution produces full IDs.
An abbreviation can become ambiguous after new objects arrive.
A successful resolution does not make that abbreviation an immutable key.

A source view includes repository, tree, root prefix, and effective source configuration.
A document also records its path, module name, grammar, parser profile, and tape.
The tape excludes placement facts. Name resolution and findings include the contextual facts they actually read.

## 5. Public library surface

The proposed public surface is in [library.md](library.md).
The native request contracts and supported binding modes are in [primitives.md](primitives.md).
All signatures in those files are proposed contracts.

`git_root_repo(repo)` is a declared input supplied by the invocation.
In a daemon, its value comes from the request root.
It does not come from the daemon process's working directory.
An unavailable root repository produces an explicit input error.

Every repository-dependent operation names `repo` in its arguments.
Pure operations over a content or tape value need no repository argument because the value identifies their complete input.
A public Git convenience relation still keeps repository provenance.

The library owns:

- Commit ancestry and other history queries that rules derive from explicit parent edges.
- Tag peeling from decoded target edges.
- Recursive tree paths and source-file selection.
- Commit subjects, message trailers, and policy interpretations of signatures.
- Language selection, overrides, and effective parser profiles.
- Parse demand, cache key/result rules, and document placement requests.
- Joins that attach source and commit findings to their provenance.


## 6. Content and parsing cache contracts

The key/result rules and invalidation examples are in [cache-contracts.md](cache-contracts.md).
These contracts are requirements for implementation, including cold processes that read an existing store.

The pipeline is:

```text
explicit repository and selection requests
  -> observed refs and resolved full IDs
  -> decoded commits and immediate tree entries
  -> library-derived files and configuration
  -> effective language and parser profile
  -> requested immutable content
  -> shared parsed tape
  -> document placement
  -> rule-derived results and findings
```

The arrows represent relation dependencies. They are not separate cache managers or unconditional preparation passes.
Demand controls object reads, content decoding, and parsing.
A metadata-only check does not request every source blob.
A known suffix does not require content sniffing unless a relevant rule or override requests it.

Successful immutable payloads can remain reusable after their original invocation ends.
Transient repository handles and buffers can remain native and bounded by the run.
Retained computed answers and their validity belong to facts.

## 7. Failure and completeness

A successful empty answer differs from an unreadable input.
Native requests publish explicit outcomes for missing objects, malformed objects, ambiguous revisions, and unsupported encodings.
They also identify which answer family is complete.

A shallow boundary is incomplete ancestry, not evidence of a root commit.
A missing blob is not an empty file.
A failed decoder is not an empty set of parents or tree entries.
Dependent negation and aggregates require complete input or produce the engine's existing unknown behavior.

The provider does not fetch from remotes implicitly.
A user can make missing objects available and rerun the request.
Failed observations must refresh under that rerun. They cannot become permanent immutable negative cache entries.

An already retained payload and present local Git storage are different claims.
A content read can reuse a verified retained payload with recorded provenance.
A question about current local object presence requires a fresh repository observation.
Whether public rules need that presence relation is open.

Root discovery and source capabilities bound access to repository data.
A content-store hit must not establish access to an otherwise unauthorized repository.
Git replacements, grafts, and shallow metadata also need an explicit interpretation.
The proposed default for immutable object relations is the object's recorded bytes and parent IDs.
An alternative logical history view must identify its additional inputs in the relation and its keys.

## 8. Library structure

Proposed files, introduced only after the contracts are settled:

| File | Responsibility |
| --- | --- |
| `lib/git/core.rr` | Public types, primitive declarations, request/result projections |
| `lib/git/objects.rr` | Successful object/decoder key and result relations |
| `lib/git/history.rr` | Reusable ancestry rules over explicit parent edges |
| `lib/git/tree.rr` | Immediate-entry projections, recursive paths, subtree reuse |
| `lib/git/messages.rr` | Subjects, trailers, identity convenience relations |
| `lib/language/select.rr` | Registry facts, suffix precedence, overrides, detection evidence |
| Existing `sources.rr` | Generic source requests, shared tapes, contextual document placement |

The source and language layers also serve filesystem and embedded sources.
They are not separate Git parser implementations.
The native boundary adds no hidden root argument to the existing Git functors.
Existing helpers need explicit migration or compatibility rules before removal.

## 9. Migration from the current prototype

The following prototype pieces are candidates for reuse:

- Direct `gix` object access and the repository fixtures.
- Tests for dirty-checkout isolation, bare repositories, raw trees, and ordered merge parents.
- Tests for language-sensitive source reads and unrequested missing blobs.

The following pieces require replacement or redesign:

- Global root-based lookup for repository-dependent functors.
- Synthetic filesystem paths as the primary source identity.
- Source-reader dispatch spread through the corpus runner.

An explicit source value and document placement must reach configuration, loading, parsing, and reporting.
A compatibility adapter can serve existing path-based consumers during migration.
It cannot become the owner of source identity or an independent cache.

The migration does not require removal of useful object-access code or working tests.
It also does not require a wholesale rewrite of existing caches listed for migration in 0103.
New Git behavior must not add another retained mechanism to that list.

## 10. Acceptance and measurements

[cache-contracts.md](cache-contracts.md#8-acceptance-cases) defines the cache acceptance cases.
Each case checks both output and expensive-operation counts.
Required counters include object payload reads, object decodes, detection derivations, parses, tape loads, and document placements.
A query profiler can report counts. A correctness test must assert the relevant counts directly.

The comparison covers cold runs, warm runs, new processes, and repeated commits with shared trees or blobs.
Reports include bytes read, peak memory, and store growth.
Large-history measurements include machine load, as required by the repository build instructions.

Parsing reuse does not imply that all semantic analysis is reusable.
Import resolution and project-wide rules can depend on changed contextual facts despite an unchanged tape.
Measurements must separate these costs.

## 11. Engineering questions

| Question | Proposed starting point | Evidence needed |
| --- | --- | --- |
| Source value representation | Serializable repository/tree/prefix and content handles | Exercise document loading and contextual placement with an explicit source value |
| Parser interface | Extend the generic content parser contract | Check which parser inputs the existing descriptor can represent |
| Detection inputs | Path and relevant facts, with bytes on demand | Count work after a grammar-changing rename and after an unrelated edit |
| Bound requests through library rules | Reuse existing demand propagation | Resolve `git_message(repo, commit, text)` without enumerating the object store |
| Failure retry | Publish refreshed read outcomes as facts | Read a missing object, add it, then repeat the same request with the store retained |
| Commit diagnostic location | Repository and commit, with an optional message span | Check which existing location and report types can carry that provenance |

A first slice applies a current rule to one historical file. It selects a language, requests a tape, and places the document.
The slice also reads a commit message through the repository-qualified library.
It measures a warm rerun and a reopened store, then exercises retry after a missing object becomes available.

## 12. First implementation

The headless scan keeps its existing policy catalog and rule selection. It opens
the revision once with gix and passes that source to the repository index. The
index uses blob IDs and reads source bytes on demand. It resolves package markers
and manifests against the historical source. The JSON report identifies the
source tree and, when present, the commit. Its checkout fields still describe
the checkout used to select policy. Path-based readers use a virtual root
to identify the Git view. Rerail creates no source directory.

The first rule library uses explicit request relations for revision resolution
and object decoding. Request rows carry the repository and the expression or
object ID. This gives a rule a finite domain, including a domain derived by
following parents. The provider answers through the ordinary request fixpoint.
The library projects decoded rows into named predicates. The selected source
does not change the meaning of a lookup for HEAD or another object.

The rule library uses a key and a result tagged with the decoder version.
The request fixpoint rechecks refs and checks the availability of kept objects.
The engine can carry successful decoded rows forward through its ordinary fact frames.
See [Git in native rules](../../../rule-syntax/git.md) for the implemented relation names.

The source phase derives the suffix result from the grammar registry through
`source_language_key` and `source_language_result`. The phase's tape key
includes the parser build and tape format. The native corpus still uses the
registry for its initial language partitioning. The product scan keeps its
existing language routing and uses the shared content and tape store.
The versioned store for headless revision scans lives beside the Rerail
home database. It can reuse parsed content on a later scan.

The current rule interface opens the invocation repository. For independent
ref resolution and object acquisition, a program supplies each request row.
The JSON policy evaluator uses the revision as its document source and
the native `.rr` runner exposes the repository-qualified Git relations.
