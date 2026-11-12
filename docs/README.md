# Documentation

## Using Rerail
Install Rerail, write rules, and see what it does in a session.

- [installation.md](installation.md): requirements, what set-up asks and changes, updating, starting, stopping,
  removal, and what Rerail runs on
- [how-setup-works.md](how-setup-works.md): each set-up step and what it writes, the managed-installation layout, the
  database, updates and removal
- [troubleshooting.md](troubleshooting.md): what you see when set-up or Rerail goes wrong, and what to do
- [reference.md](reference.md): rules and their configuration, the database, updates, the daemon and its API, audits
- [distribution-model.md](distribution-model.md): how the package is distributed and updated
- [writing-rules.md](writing-rules.md): which rules are worth having; suggesting, writing, testing and tuning one with
  `/rerail-create` and `rerail rules evidence|test|add`; keeping a rule to one worktree with `.rerail/rules/`
- [rule-scoping.md](rule-scoping.md): global, project and repository rules
- [tool-call-rules.md](tool-call-rules.md): rules over the commands an agent runs
- [path-and-text-rules.md](path-and-text-rules.md): code rules over file text and paths, which files they read, and
  rules that a file should not exist
- [architecture-rule-examples.md](architecture-rule-examples.md): design rules that maintainers of Sentry, Home
  Assistant, Zulip, Immich, n8n, VS Code, rust-analyzer, Zed, uv, Nextcloud, Laravel apps and Synapse enforce in
  review, each as a tested rule, plus React wrapper rules and what a rule can and cannot see
- [code-rule-dsl-reference.html](code-rule-dsl-reference.html): every construct of the `rerail.policy/v1` code-rule
  DSL. Hand-written prose with generated regions; see `scripts/generate/README.md`.
- [in-session-surfaces.md](in-session-surfaces.md): what Rerail shows inside a coding-agent session
- [privacy.md](privacy.md): what Rerail sends over the network, what it stores locally, and how to delete it
- [internal-testing.md](internal-testing.md): unreleased features and where each surfaces
- [headless-scan.md](headless-scan.md): `rerail scan` of a checkout for CI, with no daemon or session, and its
  `rerail.scan/1` report (internal testing)
- [remote-sink.md](remote-sink.md): sending this machine's Rerail data to a remote you add, its protocol, and what
  stays on disk (internal testing)

## Rules, packs and languages
How rule packs are built, ported, checked and extended.

- [rule-packs.md](rule-packs.md): packs, matching and recommendation, the shipped packs, and a worktree's own rules
- [pack-format.md](pack-format.md): pack layout, pack data, provenance records and the validation contract
- [upstream.md](upstream.md): upstream snapshots and pack data, samples and their licences, new snapshots, the
  publish workflow, upstream releases and bumps, and CI
- [conformance/README.md](../conformance/README.md): the conformance check every shipped ported rule passes, its pull
  request comment, divergences, and promoting a candidate
- [language-packs.md](language-packs.md): the language registry, statuses, and how to start a pack
- [adding-rules-to-a-pack.md](adding-rules-to-a-pack.md): adding rules written for Rerail or ported from a tool, who
  does what, the steps for each kind, the Conformance comment, promotion and demotion, and maintainers' publish steps

## Release
How core and packs are published.

- [release.md](release.md): how core is built, versioned and published
- [publishing-a-new-pack.md](publishing-a-new-pack.md): runbook for a pack's first npm publish and its trusted
  publisher

## Repository
How the project is run and laid out.

- [GOVERNANCE.md](GOVERNANCE.md): participation, maintainers and decision-making
- [repository-structure.md](repository-structure.md): layout rules and the structure gate (`npm run check:structure`)

## Design notes
Internal designs for contributors.

- [content-addressed-workspace-store.md](content-addressed-workspace-store.md): the machine-wide content store
- [transducer-design.md](transducer-design.md): the tape transducer language used by tests and benchmarks
- [merge-assist-v1-build.md](merge-assist-v1-build.md): Merge Assist (internal testing), which detects in-flight
  conflicts, where this checkout's work overlaps other branches and open pull requests, and tells the person and their
  coding agent before they merge; [merge-assist-prd.md](merge-assist-prd.md) states its delivered scope and keeps the
  design and evaluation record of what was built and removed, and
  [branch-coordination-amp-comparison.md](branch-coordination-amp-comparison.md) compares the earlier approach with Amp
