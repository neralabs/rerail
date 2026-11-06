# Complete the batch-ten DoD

The user requested the full repair after the [completion review](data-layer-batch-ten-dod-audit.md).
The delivery includes the following work:

1. Preserve optimizer document checks for malformed, valid, absent, and restored verifier files.
2. Route matching consumer roots through discovery, native scope, application classification, and CLI configuration.
3. Implement schema-approval validation, signature verification, verified suppression, and rejected unapproved callsites.
   Keep signing updates in an explicit user-invoked workflow. Use isolated fixture keys in tests.
4. Preserve matching phase selection and optimizer summaries through the production command.
5. Exercise all ten shipped rules on persistent workspaces, including edits, restoration, removals, and cross-file dependencies.
6. Extend the cumulative oracle and native runner to all 100 authored ports.
   Record exact finding parity, compilation, cold CPU, wall time, and RSS over the complete repository and required artifacts.
7. Run the upstream assertion, positive, configured-default, helper, transfer, and integration gates.
   Refresh receipts and CI, record failures and results, then commit and push.

Ordinary commands retain their 115-second cap.
The requested complete repository measurement runs to completion with progress updates.
The user authorized that measurement through the full DoD request.

## Progress

The malformed-verifier regression first passed four controls and failed four positive evaluations.
Commit `bafbaccd4` retains that failure. Commit `ebe9efc36` changes the gate to file presence.
All eight optimizer regressions pass.

Commit `812967d7e` retains two failing production consumer-root cases.
The implementation now discovers explicit roots, checks missing directories, and merges source buffers with discovered files.
It passes root metadata into SQL application classification and source selection.
The public command accepts repeated `--application-root` arguments.
Further completion work remains in progress.

Approval reporter comparisons pass: 28/28. The three command tests pass.
The permanent lifecycle matrix passes for all ten rules and submits only changed files.
The focused DoD run passes 52 tests on the pre-rebuild addon.

The cumulative twenty-check preflight fails with 3,122 producer-capacity errors.
It exposed an aggregate 8 MiB canonical-producer transport limit during coalescing.
The engine repair partitions complete documents and preserves logical scopes.
The first Rust test command reached its compilation deadline before test execution.
A fresh release addon build and a complete original 100-check run are in progress.
All current native evidence will be refreshed against the rebuilt addon.

The complete original run covers 9,021 code files and seven required documents.
It records 25,262 findings across all 100 checks, with no source mismatches.
The original measured phase takes 392.407 CPU seconds and 468.218 wall seconds.
The external process-family receipt records 429.978 CPU seconds and 511.261 wall seconds.
Its maximum single-process RSS is 4,859,510,784 bytes.
These measurements include concurrent build activity in wall time.

The release build completed in 6m44s. The selected addon is
`3d8e67fda55bcc4b410095e11a96ea5451c840d143c55442793e7f2d25b82724`.
All eleven canonical producer Rust tests pass.
The focused native DoD matrix passes 53 tests on that addon.
The complete native 100-rule run is in progress.

A further command regression fails before its fix: invalid approval configuration returns zero without `--check`.
The repaired command returns one for `SIDP000` independently of ordinary report-only findings.
The isolated signing/command regression passes after that fix.
