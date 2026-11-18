# Cache contracts for Git sources and parsing

Status: design contract. See [Git in native rules](../../../rule-syntax/git.md) for the implemented interface.
The examples describe dependencies. They are not executable against the current builtin declarations.
[0103](../../decisions/0103-a-cache-is-two-rules.md) and [0104](../../decisions/0104-everything-kept-is-facts.md) govern their storage and maintenance.

## 1. One fact path

Each retained expensive result has an ordinary key rule and an ordinary result rule.
A key carries a literal version tag. The result reads that key as a positive atom.
The result carries the version where it identifies its producer contract.
The existing relation-identity mechanism also records rules and their dependencies.
The examples do not add a second digest of the whole program to every key.

The common relation store retains these facts, supports deltas, writes them back, and handles collection.
A `.cache` declaration identifies a cache for the existing lint and retention contract.
It does not authorize a special evaluator or another cache table.

Transient handles and buffers are implementation data structures, bounded by the run.
A retained native map of computed Git answers is not an alternative implementation of these relations.
Existing legacy caches can remain during their documented migration. New Git behavior does not expand that exception list.

Tests that inspect the native content store with another SQLite binding must open it in a separate process.
Two SQLite libraries in one process have separate lock records for the same database.
Closing a connection through one library can drop locks still in use by the other.
See [SQLite's guide to file locking problems](https://www.sqlite.org/howtocorrupt.html#_multiple_copies_of_sqlite_linked_into_the_same_application).

## 2. Keys by layer

| Result | Key dependencies | Excluded unless actually read |
| --- | --- | --- |
| Successful object payload | Authorized repository request, full algorithm-qualified object ID, acquisition contract version | Ref spelling, commit selection, invocation number |
| Decoded commit or tree | Payload content, Git object format, decoder version | Checkout state, source path, current ref |
| Raw recursive subtree entries | Immutable tree identity and relevant decoder relations | Parent commit, placement prefix, source policy |
| Selected source paths | Tree entries, root prefix, exclusions, relevant configuration | Unrelated configuration and unrelated trees |
| Text view | Content identity, encoding and normalization policy, decoder version | Repository, commit, filename |
| Effective language | Path, relevant registry/override/configuration facts, demanded content evidence | Whole repository digest by default |
| Parsed tape | Parser input content, grammar, entry point, effective parser profile, parser version, tape format | Repository, commit, source path, module name |
| Document placement | Source view, path, module, effective grammar/profile, tape | Unrelated files |
| Semantic analysis | Document facts and actual configuration/import dependencies | Unrelated commits or unrelated project settings |

A canonical content identity unifies equal bytes from different Git hash formats.
The initial mapping can require one blob read and digest computation.
A retained successful mapping avoids repeating that read solely to recover the digest.

A decoder can use a compressed source-store value without an extra Git-specific payload encoding.
The parsed tape stays the ordinary stored tape.

## 3. Object acquisition and decoding

Conceptual successful acquisition cache:

```prolog
.decl git_object_key(r: symbol, oid: symbol, v: symbol)
git_object_key(r, oid, "object-v1") :- git_object_request(r, oid).

.decl git_object_result(r: symbol, oid: symbol, v: symbol, kind: symbol, content: symbol)
git_object_result(r, oid, v, kind, content) :-
    git_object_key(r, oid, v), git_object_of(r, oid, kind, content).
.cache git_object_result by git_object_key
```

`git_object_of` denotes the native request contract from [primitives.md](primitives.md).
It can produce an explicit error instead of a successful payload.
This cache stores successful immutable results. It does not make failed reads permanent facts of immutable absence.
The provider's failed-read observation refreshes on the next invocation that requests it.
A later successful answer derives the payload relation through the ordinary fact path.

Repository authorization remains an input to acquisition.
A cache hit cannot introduce another repository capability.
Local object presence is a different observation from previously acquired immutable content.
The library must not derive present local storage merely from `git_object_result`.
The provider records the storage state that it checked when it read an object.
It checks loose file metadata and the pack and index files in the repository and its alternates.
If the storage state changes, the provider reads and checks the object's checksum before it returns successful facts.
Failed object reads are checked again when a later run requests the object.
Verify the selected commit or tree before opening the source view.
The source view also verifies each annotated tag that it follows.
Revision expressions can make gix read a tag, commit, or tree before it returns an object ID.
Verifying the final ID cannot detect a change to an earlier object that redirected the expression.
Revision resolution must verify each object before it uses that object's contents to find the next ID.
A loose object's storage observation depends only on the loose copies that Git can use when no pack index lists the object.
Use a conservative pack observation when an index cannot prove whether it contains the object.
A source request checks the same storage observation before it uses saved content.
It verifies bytes from changed storage against the Git object ID.
The scan reports unreadable selected sources by path and continues to check readable sources.
Listing files and evaluating metadata do not demand the source blobs.

A tree decoder's cache key includes hash format:

```prolog
tree_decode_key(content, format, decoder_v, "tree-v1") :-
    wanted_tree_decode(content, format), git_decoder_version(decoder_v).

tree_decode_result(content, format, decoder_v, version, name, mode, target) :-
    tree_decode_key(content, format, decoder_v, version),
    tree_entry_of(content, format, decoder_v, name, mode, target).
.cache tree_decode_result by tree_decode_key
```

The empty tree has a complete successful decoder outcome with zero entry rows.
That outcome is retained too. Zero rows alone do not encode completeness.
The explicit walk, the source inventory, and direct path lookup use the same verified tree entries within a scan.
Count a decode where the provider reads the entries from the Git object.
Read the root's verified facts before a revision run looks up its settings or its ignore file.
The root request uses the ordinary Git rules and can restore the retained tree entries.
Later path lookups use verified inventory entries for directories the inventory has already read.
The ignore selection reads the ignore blob's storage observation. A changed blob must pass verification before it can filter files.
Commit decoding uses the same form for commit records, ordered parents, signatures, and extra headers.
One native decoder request can answer all these families.

## 4. Language selection

Language selection is a computation over path and applicable evidence.
The registry exposes supported grammars, suffixes, and entry points as facts.
Library rules express longest-suffix precedence, explicit overrides, and any content fallback.

The key is the effective set of inputs for that decision.
A path-only decision does not depend on the entire blob body.
A shebang decision depends on its content evidence and the decoding rule that produced that evidence.
A project override depends on the applicable configuration facts.
A rule change reaches the result through ordinary relation identity and dependencies.

The implementation can use a canonical decision-input value when a variable set of evidence needs one key value.
That value must derive from facts and remain inspectable.
It cannot conceal a Rust hash of unspecified context.
The following examples separate suffix, override, and content-fallback dependencies.

A whole-tree ID is not the default detection key.
An unrelated file change must not invalidate every filename's language decision.
A blob-only key is also insufficient because one blob can appear at different suffixes.

The selected grammar and effective parser profile are separate outputs.
A mode change can alter parsing without changing the language label.
Configuration that affects later name resolution alone does not enter the parse key.

### 4.1 Suffix selection example

A path-only selector has a path and registry key, without a blob or commit key:

```prolog
.decl suffix_key(path: symbol, registry: symbol, v: symbol)
suffix_key(path, registry, "suffix-v1") :-
    language_path_request(path), language_registry_version(registry).

.decl suffix_result(path: symbol, registry: symbol, v: symbol, grammar: symbol)
suffix_result(path, registry, version, grammar) :-
    suffix_key(path, registry, version),
    suffix_match(path, grammar, length),
    !longer_suffix_match(path, length).
.cache suffix_result by suffix_key
```

`suffix_match` is a library relation over path and registry suffix facts.
`longer_suffix_match` compares the lengths of matching suffixes for that path.
Their implementation preserves the current ASCII case behavior and longest-suffix rule.
Equal-specificity candidates require registry precedence or an explicit ambiguous outcome.
They do not receive an arbitrary winner from native iteration order.
The registry identity reflects selection data and supported grammars, separately from parser implementation versions.

### 4.2 Explicit override example

An applicable override is ordinary contextual data:

```prolog
override_language(view, path, grammar, profile) :-
    applicable_language_override(view, path, grammar, profile).
```

The library derives applicability from the selected configuration, path, and declared precedence.
It can select the override before requesting suffix or content evidence for that file.
This projection needs no separate expensive-result cache.
A changed override retracts the affected selection and requests the new effective parser key.

### 4.3 Content fallback example

Content-based fallback has a content key and explicit decoding dependencies:

```prolog
shebang_key(content, encoding, normalization, decoder_v, registry, "shebang-v1") :-
    wanted_shebang(content, encoding, normalization),
    text_decoder_version(decoder_v), language_registry_version(registry).

shebang_result(content, encoding, normalization, decoder_v, registry, version, grammar) :-
    shebang_key(content, encoding, normalization, decoder_v, registry, version),
    decoded_text(content, encoding, normalization, decoder_v, text),
    shebang_interpreter(text, interpreter), interpreter_grammar(interpreter, grammar).
.cache shebang_result by shebang_key
```

`shebang_interpreter` and `interpreter_grammar` are library relations over text and registry data.
The complete text-view key is content, encoding, normalization, decoder version, and a literal text-contract version.
It retains decoded text through the same key/result pattern.
A bounded evidence view can replace full text when its byte-limit and truncation semantics are explicit inputs.
The physical cost of reading a compressed Git blob prefix still needs measurement.

The final selector joins override, suffix, and fallback results under explicit precedence.
A file needs one effective grammar/profile or an explicit unsupported/ambiguous outcome.
The default proposal is override first, suffix second, and content fallback only when suffix selection gives no candidate.
A different precedence is a library policy change, with targeted invalidation through its dependencies.

## 5. Parsing content once

Proposed cache shape:

```prolog
.decl parse_key(c: symbol, g: symbol, entry: symbol, profile: symbol, pv: symbol, fv: symbol, v: symbol)
parse_key(c, g, entry, profile, pv, fv, "parse-v1") :-
    wanted_parse(c, g, entry, profile),
    parser_version(g, entry, profile, pv), tape_format(fv).

.decl parsed_content(c: symbol, g: symbol, entry: symbol, profile: symbol, pv: symbol, fv: symbol, v: symbol, t: tape)
parsed_content(c, g, entry, profile, pv, fv, v, t) :-
    parse_key(c, g, entry, profile, pv, fv, v),
    parse_content_of(c, g, entry, profile, pv, fv, t).
.cache parsed_content by parse_key
```

`parse_content_of` is a proposed generic parser contract, not an existing builtin.
The implementation can extend the current parser request or use a complete parser descriptor with the same dependencies.
The parser must honor the profile and entry point represented in the key.
The engine exposes supported parser versions and tape formats as facts.
A library version column does not replace those implementation identities.

The content here is the exact parser input after any explicit text-decoding transformation.
The text-view cache records the original bytes and its decoding policy separately.
Git checkout filters and smudge operations do not run.

One blob at several paths can share one tape under the same parser inputs.
A rename that changes the selected grammar requests another tape.
A rename that preserves the effective parser inputs reuses the tape and changes only placement-dependent results.
A commit ID or invocation counter in this parse key violates the contract.

Across processes, stored result facts and their referenced tapes must restore together.
Retention must keep the content and tape payloads that live facts name.
If collection removes a result, its next demand derives it again through the same rules.
It does not return a dangling tape handle or depend on a process-local map.

## 6. Placement and semantic reuse

A document places a shared tape in a source view.
Its contextual inputs include repository/tree/prefix, path, module identity, and relevant configuration.
The path and module can change while the tape remains equal.
Persisted node IDs from a previous placement cannot be reused as if they were stable document identities.

Later phases consume source-phase exports, as they do in `sources.rr`.
They do not independently list the same tree, read the same blob, and parse it again.
The first content read is handed through the run using the existing source ownership flow.
Within a run, a borrowed immutable content value needs no filesystem stat for validity.

Import resolution can depend on other files and package metadata.
Equal tape identity does not justify reuse of semantic results whose import or configuration inputs changed.
The existing component and relation dependency mechanisms retain that distinction.
Embedded-language parsing uses the same content/grammar cache and preserves its existing source map.

The Python reference extractor currently takes the file's tape and effective module as inputs.
Its local result describes definitions and reference names. It contains no resolved edges to other files.
Retain those local facts by tape, effective module, and extractor version.
Include whether the module has a containing package, because that changes relative imports.
Include the stub setting, which changes which bindings the extractor reads.
The source path locates the tape on a miss. The local result key uses the source content and parser versions.
Use file-local definition ranges to join the summary to each placement's current nodes.
The link step reads the current placements, import names, and rule settings.
A change in another file can change those links without rebuilding this file's summary.

## 7. Mutable observations and failure retry

Ref and revision resolution are observed inputs.
An unchanged resolved full ID leaves dependent immutable keys unchanged.
A changed ID updates selection and placement facts, while unchanged content and tapes remain reusable.

Failed object reads need a renewed observation while demanded.
A missing object that appears later must change the answer without a library version bump or cache deletion.
The provider reports this change through ordinary input facts.
It does not compare a new Git-history digest with a private Rust record.

An outcome includes operation, request key, and complete/error status.
Shallow ancestry and unavailable objects prevent complete reachability conclusions.
Negation and aggregates wait for a complete producer domain or retain an explicit unknown result.
If a program defines a range over equal endpoints, complete endpoint data yields an empty set.

## 8. Acceptance cases

Counters below refer to logical retained work and native payload or parser calls.
Opening repository handles and probing requested mutable refs are separate costs.
Tests must state whether the store is cold, retained, collected, or reopened in another process.

| Case | Required result | Required reuse or invalidation |
| --- | --- | --- |
| Warm unchanged tree | Identical findings and provenance | Zero repeated payload reads, object decodes, or parses for retained demanded results |
| New commit with an unchanged tree | New commit metadata, same source findings | Existing content and tapes reused |
| Separate scans of commits share a blob | Correct findings in each scan’s own report | One cold content acquisition per retained object mapping and one parse per effective parser key |
| Same blob at two paths | Correct path and module identities | Shared tape, distinct placements |
| Rename `.js` to `.ts` | New effective grammar when registry rules require it | Detection changes, correct grammar tape selected |
| Rename preserving parser inputs | New path provenance | Tape reused |
| Equal bytes in two authorized repositories | Independent repository provenance | Canonical content and compatible tape can be shared |
| Equal bytes under SHA-1 and SHA-256 | Correct typed object IDs | Canonical content and compatible tape shared |
| Relevant parser configuration changes | Findings reflect the effective mode | Affected parser keys change |
| Unrelated configuration changes | Unchanged unaffected findings | No unrelated language or parse work |
| Rule-only edit | New rule results | Unchanged parse keys and tapes reused |
| Current policy changes during historical checks | Another scan uses the current rule version on the same tree | Historical tape remains reusable |
| Grammar or parser implementation changes | New parser results where demanded | Affected parser keys change |
| Tape format changes | Compatible stored values only | Correct format requested, no stale or dangling handle |
| Source file disappears from selection | Its document and findings retract | Shared retained content can remain for other placements |
| Branch ref moves | Selection and provenance update | Only dependent derived facts change |
| Missing object later appears | Read error becomes success | Retry without manual cache clearing |
| Corrupt object | Explicit read/decode error | No partial successful family or false empty result |
| Shallow boundary | Explicit incomplete ancestry | No false root or complete negative range conclusion |
| Empty tree | Complete successful empty answer | Empty outcome retained correctly |
| Cold process with retained store | Same findings | Restored result facts reuse stored content and tapes |
| Collection followed by demand | Same valid findings or explicit unavailable-source outcome | Missing retained work recomputed, no dangling payload references |
| Metadata-only rule | Correct messages and identities | Zero source-blob parses or unconditional source traversal |
| Unrequested missing blob | Other requested reads succeed | No eager read of that blob |
| Case-distinct Git paths on a case-insensitive host | Both names remain distinct | No filesystem extraction or name conflation |
| Nested and linked repositories | Correct repository and `HEAD` context | Explicit values prevent cross-repository cache contamination |
| Unknown input under negation | No confident negative finding | Existing unknown/error behavior preserved |

The first implementation must include direct counts for a cold run, a warm run, and a new process.
Output equality alone does not prove cache reuse.
The cache-architecture guard must also pass, including its prohibition on new private retained mechanisms.
