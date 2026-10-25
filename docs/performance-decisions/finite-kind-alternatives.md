# Fold finite kind alternatives before SQL emission

Status: retained after regression tests and bounded CPU controls, 2026-10-25. Full-repository acceptance remains open.

## Evidence

The guarded-stream capture uses the original generated-facades file and all 60 rules.
Query Q224 spends 1.247 CPU seconds and 50,792,117 VM instructions partitioning and reuniting one candidate scope.
Its nested alternatives test only finite sets of node kinds.
The serverplan-quality syntax exclusion and related data-layer inputs contain this shape.
The capture is `.rerail/profiles/anchor-planning-20261025/stream-sql-diagnostic-one/sql/profile.json`.
This diagnostic omits findings parity and cannot establish aggregate performance acceptance.

## Prior work

The shared direction requires fused narrow statements and identical candidate scopes.
The lessons ledger entries 19, 28, 30, and 33 cover embeddable fragments, measurement proxies, full volume, and excess relation construction.
The ast-index design index topic 2 cites `DESIGN.md` S066, lines 3855–3874.
The locally available source is `/tmp/ast-index-seadragon-design.md`; those source lines were read directly.
S066 retains a single SQL integer `IN` list after a bitmap alternative loses, 123 versus 110 milliseconds.
This change agrees with that result. It adds no bitmap, native evaluator, or new index.
The shared indexes contain no directly measured finite-kind Boolean folding experiment.

## Decision

Recognize exact finite kind alternatives before emitting their individual SQL branches.
Union their kind sets, intersect any enclosing kind restriction, and emit one filter over the original scope.
Follow utility references with memoization so shared definitions do not multiply compiler work.
Keep approximate or non-kind conditions on the existing conservative path.
Negation remains caller SQL and may subtract only an exact result.
No scope is widened and no data is shared between different consumer scopes.

## Validation

A red regression gives nested alternatives only the space needed for one decoder, one filter, and one subtraction.
It checks positive selection and negation against the tape, including repeated alternatives and a utility reference.
The existing arbitrary-union test must still exercise union construction independently of kind folding.
Run source parity, bounded unprofiled ingestion controls, and a separate SQL diagnostic after implementation.
Keep full-repository acceptance open until complete findings and CPU, wall time, and RSS are available.

## Result

All 1,516 DSL tests, 42 production-shape tests, 2,350 original executable observations, and 120 supplemental controls pass.
Three-run median ingestion CPU falls from 25.675 to 21.827 seconds on the 128-file, 60-rule control.
Peak RSS remains effectively unchanged. Retain the fold; do not claim full-repository recovery.
The performance journal records every control, capture, hash, and remaining coverage boundary.
