# Documentation

## Using Rerail
- [installation.md](installation.md): what `rerail setup` writes, removal, and the managed-installation layout
- [troubleshooting.md](troubleshooting.md): what you see when set-up or Rerail goes wrong, and what to do
- [reference.md](reference.md): how rules and their configuration work, the database, updates, the daemon and its API,
  and audits
- [distribution-model.md](distribution-model.md): how the package is distributed and updated
- [writing-rules.md](writing-rules.md): which rules are worth having, and suggesting, writing, testing and tuning one
  with `/rerail-create` and `rerail rules evidence|test|add`
- [rule-scoping.md](rule-scoping.md): global, project and repository rules
- [tool-call-rules.md](tool-call-rules.md): writing rules over the commands an agent runs
- [code-rule-dsl-reference.html](code-rule-dsl-reference.html): every construct of the `rerail.policy/v1` code-rule
  DSL. Hand-written prose with generated regions; see `scripts/generate/README.md`.
- [in-session-surfaces.md](in-session-surfaces.md): what Rerail shows inside a coding-agent session
- [privacy.md](privacy.md): what Rerail sends over the network, what it stores locally, and how to delete it
- [internal-testing.md](internal-testing.md): features in the code that are not ready for public use, and where each
  surfaces

## Rules, packs and languages
- [rule-packs.md](rule-packs.md): rule packs, matching and recommendation, and the shipped packs
- [pack-format.md](pack-format.md): the pack layout, its pack data and provenance records, and its validation contract
- [upstream.md](upstream.md): upstream snapshots and pack data: samples and their licences, new snapshots (paused), the
  publish workflow, upstream releases and bumps, and CI
- [conformance/README.md](../conformance/README.md): the conformance check every shipped ported rule passes, its pull
  request comment, divergences, and promoting a candidate
- [language-packs.md](language-packs.md): the language registry, statuses, and how to start a pack

## Release
- [release.md](release.md): how core is built, versioned and published
- [publishing-a-new-pack.md](publishing-a-new-pack.md): runbook for a pack's first publish to npm and its trusted
  publisher

## Repository
- [repository-structure.md](repository-structure.md): the layout rules and the structure gate
  (`npm run check:structure`) that enforces them

## Design notes
- [content-addressed-workspace-store.md](content-addressed-workspace-store.md) and
  [content-store-bulk-publication-design.md](content-store-bulk-publication-design.md): the machine-wide source store
- [sql-driven-transducer-execution.md](sql-driven-transducer-execution.md) and
  [transducer-design.md](transducer-design.md): the tape transducer language
- [in-session-status-line-prd.md](in-session-status-line-prd.md): requirements for in-session visibility
