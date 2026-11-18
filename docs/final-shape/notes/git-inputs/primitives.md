# Native contracts for Git source facts

Status: design contract. See [Git in native rules](../../../rule-syntax/git.md) for the implemented interface.
[design.md](design.md) defines the boundary. [library.md](library.md) gives the public relations.

The interface follows the existing request/answer mechanism from [0065 section 2a](../../decisions/0065-typing-is-one-fixpoint-over-stored-derivations.md).
A rule derives a request. Native code supplies produced facts inside the same fixpoint.
The names below identify operation families, not a requirement for one callback per output relation.

## 1. Values and request ownership

Examples use nominal values named `repo`, `oid`, `content`, and `profile`.
Their persistence encoding is open. Their identities and supported equality are part of the contract.
A repository-dependent request always carries `repo`.
An `oid` includes its hash algorithm and full digest.
A content handle identifies immutable bytes in the ordinary source store.

The invocation supplies `git_root_repo(repo)` and explicit selection requests.
A separate repository-open request can introduce another authorized repository value.
Neither operation changes the meaning of a Git call that already names its repository.
Native repository handles belong to the run context and can be reused by its workers.
A process-global current-repository slot is excluded.

## 2. Primitive families

| Family | Bound request inputs | Produced facts | Native justification |
| --- | --- | --- | --- |
| Repository discovery | Invocation root or explicit locator | Repository value, kind, object format, Git directory, optional worktree | Filesystem and Git administrative layout |
| Revision observation | Repository and reference name or single-object revision expression | Resolved full ID or explicit resolution outcome | Git reference storage and standard revision syntax |
| Object acquisition | Repository and full object ID | Kind, immutable payload content, size, or explicit outcome | Pack indexes, loose objects, compression, delta chains |
| Object decoding | Payload content, object format, decoder version | Commit records, ordered parents, signatures, extra headers, immediate tree entries, tag targets | Git binary and header formats, through `gix` |
| Language capabilities | Engine build and registry | Grammar labels, suffixes, entry points, parser profiles, implementation versions | Existing parser registry and implementations |
| Content operations | Explicit content and decoding policy | Bytes/text views, bounded evidence, or encoding outcome | Ordinary source-store access and byte decoding |
| Generic parsing and placement | Content plus complete parser inputs, then explicit placement | Stored tape, document identity, syntax relations, or parse outcome | Existing parser and tape engine |

The last three families extend shared source capabilities where necessary.
They are not separate Git language detection or Git parsing engines.
Commit ranges are not a native primitive in this proposal.

## 3. Observation versus immutable computation

A reference name, symbolic `HEAD`, revision expression, or abbreviated ID is observed again when requested in another invocation.
The provider publishes the complete result as base facts.
An unchanged result supplies no logical delta.
A changed result retracts the previous answer and supplies the new one.
A fresh invocation number does not enter immutable cache keys.

The observation result includes failures and successful absence where the operation supports absence.
An old error cannot stand in for a new observation after objects or refs change.
The provider uses the common observation mechanism rather than a new Git-specific validity table or `still` callback.

Successful object decoding is a function of immutable payload, object format, and decoder implementation.
A ref move does not change that result.
Availability and authorization remain separate observations. A retained payload does not prove present local storage or access rights.

## 4. Object decoder outputs

One commit-decoder request can produce these answer families:

```text
commit_record(content, format, decoder_v, tree_oid, message_content)
commit_parent(content, format, decoder_v, position, parent_oid)
commit_signature(content, format, decoder_v, role, name, email, seconds, offset_seconds)
commit_header(content, format, decoder_v, position, name, value_content)
```

`role` is `author` or `committer`. Parent positions start at zero and preserve commit order.
Epoch seconds and timezone offsets remain separate numbers.
Message content preserves every byte, including a final newline.
Extra headers preserve their ordering and continuation data.
Invalid textual metadata produces a decoding outcome or an explicit byte-content value.
It does not become replacement characters without a declared decoding policy.

A tree decoder produces immediate entries:

```text
tree_entry(content, format, decoder_v, name, mode, target_oid)
```

It does not walk descendants or read child blobs.
The hash format is an input because binary tree entries contain digests of format-dependent width.
Entry names preserve Git identity. A platform-specific filesystem spelling cannot merge two names.
A tag decoder produces its target ID, target kind, and decoded headers from the same object mechanism.

These schemas describe logical data. Packed storage and batching remain engine choices.
Decoder outcomes identify unsupported or malformed data without publishing incomplete successful answer families.

## 5. Binding modes and demand

Object acquisition requires bound repository and full object ID.
A commit decoder requires bound content, format, and decoder version.
A tree decoder requires the same inputs and enumerates one immediate tree.
A blob request requires a bound ID. It does not enumerate repository history.

The planner can batch distinct bound requests and share their answers.
A query with unbound object IDs needs an explicit finite enumeration source, such as selected refs or reachable commits.
It cannot cause an implicit scan of every object in the repository.

Native answers preserve request identity and completeness.
The decoder can answer several public relations from one request without repeating payload reads.
The ordinary relation store retains those answers through library key/result rules.

## 6. Content and parser contracts

A Git blob's object ID can map to an existing canonical content handle.
Equal bytes share that handle across Git hash formats and source providers.
A known mapping avoids another blob read solely to compute the content digest.
The mapping and any retained decoded result follow the ordinary fact path.

The existing `parse_of(text, grammar, tape)` supports text and grammar selection.
The proposed pipeline also needs explicit entry point, parser profile, parser version, and tape format dependencies.
A version in a cache key alone cannot compensate for a parser that ignores those inputs.

The implementation must either extend the generic parse request or derive a complete versioned parser descriptor.
The same contract serves working-tree, Git, and embedded content.
Unsupported descriptors produce an explicit outcome.
The engine exposes supported descriptors as facts, so a library does not invent parser versions.

Placement supplies source view, path, module identity, grammar, and tape.
Placement can reuse the stored tape without copying it into a Git-specific representation.
Stored tape access preserves the existing [0336](../../decisions/0336-the-tape-reaches-every-reader-as-stored.md) constraints.

## 7. Scope limits

No primitive checks out a tree, writes source files, runs repository hooks, or applies checkout filters.
No primitive fetches from a remote implicitly.
Symlink and gitlink modes are visible data. Following them requires explicit library and access semantics.
The first historical-source scan can report these entries without expanding them.

Current index membership is a separate mutable source from tree membership.
A historical scan must not redefine the index relation to mean membership in its selected tree.
Existing `@git_tracked` compatibility needs an explicit migration choice.
