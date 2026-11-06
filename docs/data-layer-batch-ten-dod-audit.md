# Batch-ten completion review — 2026-11-06

Audited commit: `9ceb4c8ef`.
Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
The review uses the per-batch and migration criteria in the [restart plan](data-layer-port-restart.md#resume-in-bounded-deliveries).
Batch ten still falls short of the full completion criteria.

The [audit record](../fixtures/host/data-layer-port/batch-10-dod-audit-20261106.json) retains inputs, configuration, original findings, native results, source verification, and implementation hashes.
Six reduced scenarios run through both native APIs: twelve evaluations.
The four default-control evaluations pass.
Four evaluations reproduce an optimizer defect. Four show unsupported production configuration.
No native execution reports an engine failure.

## Default rule defect: optimizer document checks

An existing malformed `impl/common/l2065_plan_core/verifier.py` must still activate the configured reporter's document checks.
The original reporter treats that file's fact bundle as present, even when syntax extraction fails.
The port instead requires a valid canonical `Module` row.

| Verifier | Invariants document | Original configured reporter | Rerail, both APIs |
| --- | --- | --- | --- |
| `if True:` followed by a newline | Missing | `PV2-050` | No finding |
| Same malformed source | Contains `` `scope.stale` `` | `PV2-051` | No finding |
| `pass` | Missing | `PV2-050` | `PV2-050` |
| Absent; another optimizer file exists | Missing | Clean | Clean |

The faulty presence gate is in `scripts/dsl/data_layer_ten/optimizer.py:78`.
It tests successful parsing instead of file presence.
The repair needs a file-presence predicate and regressions for malformed, valid, absent, and restored verifier inputs.
This defect affects the shipped default program. It is separate from the earlier missing CLI assertions.

## Missing production contract: matching consumer roots

The original `lint(root, application_roots=...)` discovers explicit consumer roots and treats their files as application code.
For `consumers/probe.py` containing `import impl.common.hidden`, the original emits `MA010`.
The public Rerail runner returns no finding for that snapshot in either mode.

The public runner exposes no application-root argument.
The shipped matching scope also excludes `consumers/`.
The source-test adapter injects `applicationPaths` and broadens scope for its adapted calls.
That proves a test entry point, rather than production configuration support.

The repair must route explicit roots into the rule's candidate scope and application classification.
It must preserve test exclusions, missing-root errors, and exact file selection.
The public API and CLI need equivalent configuration support.
This comparison uses an explicitly configured original consumer root; it is not a failure of original default discovery.

## Missing production contract: introspection approvals

The original configured reporter validates approval entries, verifies their signatures, and suppresses valid approved callsites.
An invalid entry such as `impl/bad.py:1:table_exists` produces `SIDP000` before source findings.
The current public Rerail path has no approval input and returns no finding for the empty snapshot.

The port contains neither the `SIDP000` production branch nor approval-signature verification and approved-callsite suppression.
The native `_approval_entry_errors` unit contract does not supply that reporter integration.
The pinned approval list is empty, so this gap remains dormant under the current defaults.

The repair must preserve invalid-entry diagnostics, invalid-signature failure, verified suppression, and rejection of unapproved callsites.
Signing updates need a separate authorized host workflow.
The demonstrated case covers invalid-entry validation. Signature and suppression omissions follow from production-code inspection.

## Remaining completion evidence

The following requirements also remain open:

- The ten shipped rules lack a permanent clean/positive/restored/removed matrix on persistent workspaces.
  `TenReplay.run()` creates a fresh workspace for each observation.
  Generic text-workspace tests do not establish all ten rules' cache and dependency behavior.
- The complete repository result covers 80 rules, while the branch has 100 authored ports.
  There is no current cumulative 100-rule correctness, cold CPU, wall-time, and RSS result.
  This review performs no full cold execution.
- Full migration also needs the remaining rule-specific command options and configuration workflows.
  Examples include matching phases and optimizer summaries.
  The 75 transferred assertion sites cover their stated upstream cases, rather than all interfaces of all ten commands.

The 16 identified Python-specific assertion sites are accounted exclusions, rather than omitted native rule semantics.
The thirteen banned symbols now have positive, lookalike, other-family, and removal coverage.
This review does not reopen those completed assertion transfers.

The demonstrated gaps do not establish a missing engine capability.
The optimizer fix is a SQL guard change. The configuration gaps require rule and host integration.
No Linear issue or production-rule change forms part of this review.
