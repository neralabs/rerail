# Documentation

## Using Rerail
- [installation.md](installation.md): what `rerail setup` writes, removal, and the managed-installation layout
- [distribution-model.md](distribution-model.md): how the package is distributed and updated
- [rule-scoping.md](rule-scoping.md): global, project and repository rules
- [tool-call-rules.md](tool-call-rules.md): writing rules over the commands an agent runs
- [code-rule-dsl-reference.html](code-rule-dsl-reference.html): every construct of the `rerail.policy/v1` code-rule
  DSL. Hand-written prose with generated regions; see `scripts/generate/README.md`.
- [in-session-surfaces.md](in-session-surfaces.md): what Rerail shows inside a coding-agent session

## Rules, packs and languages
- [rule-packs.md](rule-packs.md): rule packs, matching and recommendation, and the shipped packs
- [pack-format.md](pack-format.md): the pack layout and its validation contract
- [language-packs.md](language-packs.md): the language registry, statuses, and how to start a pack

## Repository
- [repository-structure.md](repository-structure.md): the layout rules and the structure gate
  (`npm run check:structure`) that enforces them

## Design notes
- [content-addressed-workspace-store.md](content-addressed-workspace-store.md) and
  [content-store-bulk-publication-design.md](content-store-bulk-publication-design.md): the machine-wide source store
- [sql-rerailer-revival-design.md](sql-rerailer-revival-design.md): the active SQL rule-engine design
- [sql-relational-compiler-design.md](sql-relational-compiler-design.md): the callback-free relational compiler and packed terminal boundary
- [sql-engine-shape-analysis.md](sql-engine-shape-analysis.md): the measured SQL topology problems and proposed corrections
- [ner-1562-landing-pad.md](ner-1562-landing-pad.md): the packed executor port boundary and source refs
- [sql-driven-transducer-execution.md](sql-driven-transducer-execution.md): history of the deleted callback-shaped SQL engine
- [transducer-design.md](transducer-design.md): the tape transducer language
- [in-session-status-line-prd.md](in-session-status-line-prd.md): requirements for in-session visibility
