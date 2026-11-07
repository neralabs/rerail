# Documentation

## Using Rerail
- [installation.md](installation.md): installing Rerail: requirements, what set-up asks and changes, updating,
  starting and stopping, removal, and what Rerail runs on
- [how-setup-works.md](how-setup-works.md): each set-up step and what it writes, the managed-installation layout, the
  database, updates, and removal in detail
- [troubleshooting.md](troubleshooting.md): what you see when set-up or Rerail goes wrong, and what to do
- [reference.md](reference.md): how rules and their configuration work, the database, updates, the daemon and its API,
  and audits
- [distribution-model.md](distribution-model.md): how the package is distributed and updated
- [writing-rules.md](writing-rules.md): which rules are worth having, and suggesting, writing, testing and tuning one
  with `/rerail-create` and `rerail rules evidence|test|add`, and keeping a rule to one worktree with `.rerail/rules/`
- [rule-scoping.md](rule-scoping.md): global, project and repository rules
- [tool-call-rules.md](tool-call-rules.md): writing rules over the commands an agent runs
- [path-and-text-rules.md](path-and-text-rules.md): writing code rules over files' text and paths, which files they
  read, and rules that say a file should not exist
- [code-rule-dsl-reference.html](code-rule-dsl-reference.html): every construct of the `rerail.policy/v1` code-rule
  DSL. Hand-written prose with generated regions; see `scripts/generate/README.md`.
- [in-session-surfaces.md](in-session-surfaces.md): what Rerail shows inside a coding-agent session
- [privacy.md](privacy.md): what Rerail sends over the network, what it stores locally, and how to delete it
- [internal-testing.md](internal-testing.md): features in the code that are not ready for public use, and where each
  surfaces
- [headless-scan.md](headless-scan.md): `rerail scan` of a checkout with no daemon or session, for CI, and its
  `rerail.scan/1` report (internal testing)
- [remote-sink.md](remote-sink.md): sending this machine's Rerail data to a remote you add, its protocol, and what is
  kept on disk (internal testing)

## Rules, packs and languages
- [rule-packs.md](rule-packs.md): rule packs, matching and recommendation, the shipped packs, and a worktree's own rules
- [pack-format.md](pack-format.md): the pack layout, its pack data and provenance records, and its validation contract
- [upstream.md](upstream.md): upstream snapshots and pack data: samples and their licences, new snapshots, the
  publish workflow, upstream releases and bumps, and CI
- [conformance/README.md](../conformance/README.md): the conformance check every shipped ported rule passes, its pull
  request comment, divergences, and promoting a candidate
- [language-packs.md](language-packs.md): the language registry, statuses, and how to start a pack
- [adding-rules-to-a-pack.md](adding-rules-to-a-pack.md): adding rules to a pack, written for Rerail or ported from
  a tool: who does what, the steps for each kind, the Conformance comment, promotion and demotion, and the
  maintainers' publish steps

## Release
- [release.md](release.md): how core is built, versioned and published
- [publishing-a-new-pack.md](publishing-a-new-pack.md): runbook for a pack's first publish to npm and its trusted
  publisher

## Repository
- [GOVERNANCE.md](GOVERNANCE.md): who takes part and how, who the maintainers are, and how decisions are made
- [sql-executor.md](sql-executor.md): SQL sessions over facts, rule lowering, wave execution, and extension queries
- [repository-structure.md](repository-structure.md): the layout rules and the structure gate
  (`npm run check:structure`) that enforces them

## Design notes
- [data-layer-rule-port.md](data-layer-rule-port.md): current data-layer linter inventory, engine capability gaps,
  historical performance evidence, and migration order
- [data-layer-port-execution-plan.md](data-layer-port-execution-plan.md): upstream test recording, strict parity gates,
  native replay requirements, and the complete batch plan
- [data-layer-batch-two.md](data-layer-batch-two.md): the next ten candidate ports, upstream and supplemental parity, and cumulative measurements
- [data-layer-batch-three.md](data-layer-batch-three.md): the third ten ports, direct AST visitor recording, configuration parity, and cumulative measurements
- [data-layer-batch-four.md](data-layer-batch-four.md): Completed SQL grammar candidates, Rust coverage, upstream ambiguities, and performance measurements
- [data-layer-batch-four-root-causes.md](data-layer-batch-four-root-causes.md): Failure analysis, source requirements, and TDD ladders
- [data-layer-engine-gaps.md](data-layer-engine-gaps.md): demonstrated engine features, withdrawn issues, and required approval before new Linear issues
- [content-addressed-workspace-store.md](content-addressed-workspace-store.md) and
  [content-store-bulk-publication-design.md](content-store-bulk-publication-design.md): the machine-wide source store
- [sql-driven-transducer-execution.md](sql-driven-transducer-execution.md) and
  [transducer-design.md](transducer-design.md): the tape transducer language
- [in-session-status-line-prd.md](in-session-status-line-prd.md): requirements for in-session visibility
