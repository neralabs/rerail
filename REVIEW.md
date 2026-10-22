# Review bar for rerail

The automated reviewer ([neralabs/reviewer](https://github.com/neralabs/reviewer)) reads this file from the base branch on every review, on top of the team-wide bar in that repository (`skill/review/team-context.md`). Human reviewers can use it too. Changes to it change what blocks a merge, so review them like code; it is a high-risk path, so a PR that touches it is never auto-approved.

## What rerail is

Rerail runs coding rules inside coding agents (Claude Code, Codex, Cursor, OMP, Pi): a TypeScript package with one host per agent (`src/hosts/`), a Rust engine (`native/`), rule packs (`packs/`), and one user-level SQLite database holding the rule catalog and execution history. It runs on users' machines inside their agent sessions, so a bug in a host or the engine breaks someone's coding session, and a bad pack publish ships to npm. The domain terms (Rule, Clause, Violation, Rule Mode, Audit, Engine Adapter, Language Pack and the rest) are defined in [CONTEXT.md](CONTEXT.md); use them as defined there.

## What blocks a merge here

In addition to the team-wide blockers:

- **Third-party material without provenance.** Code, data or rules from another project need an `UPSTREAM.md` entry beside them (source, version, files, verbatim/derived/excerpt), the upstream licence text, and a `REUSE.toml` annotation ([CONTRIBUTING.md](CONTRIBUTING.md)). Only licences on `scripts/check/licence-allowlist.json` are accepted. CI's licence job checks that licences are declared and permissive, not that the `UPSTREAM.md` entry is accurate; check that.
- **Upstream text in port readiness recordings without its licence.** A recording (`recording.json`, a release asset that a `packs/*/upstream/*/recording.lock` names) holds upstream's test text inline (`fixtureTexts.inline`), so it is third-party material though the tree holds only its lock. A PR that records a new upstream or changes what a recorder inlines needs the recording note in that upstream's `licenses/<project>/UPSTREAM.md`, the recorder still writing upstream's licence text into the recording (`licenseText`), and no text from an upstream path an `UPSTREAM.md` lists as "Not copied". The recording is not in the diff; review the recorder change and the per-rule summary the recording job posts, or, for a tool-pack a port readiness gate leg recorded itself because the release held no recording, the one in that leg's job summary and report artifact.
- **Hand edits to generated files.** Files listed in [scripts/generate/README.md](scripts/generate/README.md) change through their generator, in the same PR as the generator change. A diff to a generated file with no matching generator change, or the reverse, is a blocker.
- **Language branching outside an Engine Adapter.** Apart from each family's Engine Adapter, nothing branches on a language (CONTEXT.md, Language Registry). A new `match` or `if` on a language or family anywhere else is a blocker.
- **Rule documents or fixtures in the wrong place.** The layout rules in [docs/repository-structure.md](docs/repository-structure.md) are checked by `npm run check:structure` in CI's lint job; an exemption added to a metric module to let a PR pass needs its reason stated and must be justified by the change.
- **Engine behaviour without a Rust test.** A change to what the engine parses, resolves or reports needs a test in `native/` that fails without it.
- **Vendored grammar changes not recorded.** A change under `native/grammars/` needs its patch in that grammar's `PATCHES.md`, and the regenerated `grammar.json` must match it.
- **Unevaluated rules shipped.** A pack's `candidates/` holds ports under evaluation and is never shipped ([packs/README.md](packs/README.md)). A rule that moves into `rules/` needs the evidence for it in the PR: its conformance results against the upstream tool's recorded findings, and any divergence declared.

## Known gaps

- `real-sessions` and `provider-smoke` (real agent sessions) don't run for PRs from forks or from Dependabot, so a host change from either has no real-session evidence in CI. Say so in the review; mark claims about host behaviour `needs-run`.
- The Rust job runs only when the paths filter in `.github/workflows/ci.yml` matches. A change that affects the engine through another path gets no Rust run.
- Git LFS files (`model/model.safetensors`, `native/src/code_rule_dsl/value/names/names.bin`) are pointer files in the review checkout; their diff is the pointer, not the content.

## High-risk areas

- Migrations: `src/engine/storage/` (the Rerail database in users' repos, its schema and `migrations.ts`) and `native/src/content_store/schema.rs` (the content store's schema and version floor). Check that existing databases still open, nothing is lost, and an older binary still refuses a store it can't read.
- CI and deployment: `.github/` (workflows, actions, Dependabot; always high-risk for the reviewer), `scripts/release/` and `.changeset/config.json`. Check that checks still gate and that a release publishes what it should, where it should.
- Packaging: the root `package.json` and `Cargo.toml`, `native/Cargo.toml`, `native/build.rs`, `scripts/build/`, `tsconfig.build.json`, `packs/*/package.json`, and the agent plugin manifests (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`). Check what the packages contain, their versions and their entry points.
- `src/setup/`: installs rerail into users' agents.
- `LICENSES/` and `scripts/check/licence-allowlist.json`: licensing.

## Reviewer settings

Machine-readable. Paths use CODEOWNERS syntax. A PR touching a high-risk path is never auto-approved. PR size doesn't affect approval. A PR over the summary-only threshold (50,000 changed lines by default; set `largePrLines` here to change it) gets a summary-only review unless someone comments `@neralabs-ci review full`. Generated paths don't count toward that threshold; they are the files in `scripts/generate/README.md`, the packs' upstream recordings (the fixtures, tests, expected answers, goldens and lock files under `packs/*/upstream/<tool>/`; not its `gates.yaml`, which pins the upstream a recording runs, nor the scripts and `UPSTREAM.md` beside them), and the regenerated grammar tables.

```json reviewer
{
  "highRiskPaths": [
    "src/engine/storage/",
    "native/src/content_store/schema.rs",
    "scripts/release/",
    ".changeset/config.json",
    "/package.json",
    "/Cargo.toml",
    "native/Cargo.toml",
    "native/build.rs",
    "scripts/build/",
    "tsconfig.build.json",
    "packs/*/package.json",
    ".claude-plugin/",
    ".codex-plugin/",
    ".cursor-plugin/",
    "src/setup/",
    "LICENSES/",
    "scripts/check/licence-allowlist.json"
  ],
  "generatedPaths": [
    "*.generated.*",
    "native/src/code_rule_dsl/value/names/names.bin",
    "native/grammars/**/grammar.json",
    "model/",
    "packs/*/upstream/*/fixtures/",
    "packs/*/upstream/*/tests/",
    "packs/*/upstream/*/*.json",
    "packs/*/upstream/*/*.tsv",
    "packs/*/upstream/*/*.lock",
    "packs/node/rules/",
    "packs/node/candidates/",
    "packs/typescript/candidates/rules/vitest-*.json"
  ]
}
```
