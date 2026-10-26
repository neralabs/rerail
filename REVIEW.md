# Review bar for rerail

The automated reviewer ([neralabs/reviewer](https://github.com/neralabs/reviewer)) reads this file from the base branch on every review, on top of the team-wide bar in that repository (`skill/review/team-context.md`). Human reviewers can use it too. Changes to it change what blocks a merge, so review them like code; it is a high-risk path, so a PR that touches it is never auto-approved.

## What rerail is

Rerail runs coding rules inside coding agents (Claude Code, Codex, Cursor, OMP, Pi): a TypeScript package with one host per agent (`src/hosts/`), a Rust engine (`native/`), rule packs (`packs/`), and one user-level SQLite database holding the rule catalog and execution history. It runs on users' machines inside their agent sessions, so a bug in a host or the engine breaks someone's coding session, and a bad pack publish ships to npm. The domain terms (Rule, Clause, Violation, Rule Mode, Audit, Engine Adapter, Language Pack and the rest) are defined in [CONTEXT.md](CONTEXT.md); use them as defined there.

## What blocks a merge here

In addition to the team-wide blockers:

- **Third-party material without provenance.** Code, data or rules from another project need an `UPSTREAM.md` entry beside them (source, version, files, verbatim/derived/excerpt), the upstream licence text, and a `REUSE.toml` annotation ([CONTRIBUTING.md](CONTRIBUTING.md)). Only licences on `scripts/check/licence-allowlist.json` are accepted. CI's licence job checks that licences are declared and permissive, not that the `UPSTREAM.md` entry is accurate; check that.
- **Upstream text in port readiness recordings without its licence.** A recording (`recording.tar.zst`, the one archive per tool-pack a `packs/*/upstream/*/recording.lock` names by sha256, kept in S3 at `rerail/recordings/<pack>/<tool>/<sha256>.tar.zst` and cached by CI) holds upstream's test text inline (as `texts/<sha256>.json`, indexed by `tool/<config-id>/fixtureTexts.json`), so it is third-party material though the tree holds only its lock. A PR that records a new upstream or changes what a recorder inlines needs the recording note in that upstream's `licenses/<project>/UPSTREAM.md`, the recorder still writing upstream's licence text into the recording (`licenseText`), and no text from an upstream path an `UPSTREAM.md` lists as "Not copied". The recording is not in the diff; review the recorder change and the per-rule summary under "Recorded by this run" in the port readiness comment: CI checks every tool-pack whose `recording.lock` the PR changes against upstream and computes that summary itself, so it, not a summary in the PR body, is what upstream answers ([docs/port-readiness.md](docs/port-readiness.md#the-recordings-are-not-in-the-tree)).
- **A pack change without its test data, or with test data still tracked.** Pack test data (a tool's cases and recorded answers under `packs/*/upstream/<tool>/`, `PACK_FIXTURE_PATTERNS` in `scripts/s3/pack-fixtures.mjs`) is not in git: each tool-pack's data is one archive in S3 that its tracked `test-data.lock` names by sha256 ([CONTRIBUTING.md](CONTRIBUTING.md#pack-test-data)). Derived data (upstream's tests, copied or derived, and the recorded answers) is regenerated from the pins by a maintainer (`npm run fixtures:derive`) or the nightly's bump workflow: a PR changes it by changing the pins, a generator or a recipe under `scripts/pack-data/recipes/`, so review that change; it is the code a maintainer then runs to derive, in a sandbox without credentials. Authored data (cases Rerail wrote for a tool, listed in its `UPSTREAM.md`, or a code corpus with its ground truth under `upstream/<tool>/authored/`) is checked in with `git add -f` for review ([CONTRIBUTING.md](CONTRIBUTING.md#adding-or-changing-a-pack)); removing one is removing its `UPSTREAM.md` row. Review it like other third-party material. Before merge a maintainer archives each changed tool-pack and pushes the recordings a changed `recording.lock` names from a trusted worktree of main (`npm run fixtures:push -- --from <ref>`, which never runs the PR's code; [CONTRIBUTING.md](CONTRIBUTING.md#merging-maintainers)), and the PR commits the new `test-data.lock` and untracks the data. A PR that changes a lock must pass **Pack test data is in S3** (S3 holds what every lock names) and, for a `test-data.lock`, **Test data reproduces from the pins** (CI derives the tool-pack again and finds the archive's data byte for byte). Structure metric #3 fails while any pack test data is tracked or a tool directory lacks a well-formed lock; that is cleared by the push, never by an exemption.
- **Hand edits to generated files.** Files listed in [scripts/generate/README.md](scripts/generate/README.md) change through their generator, in the same PR as the generator change. A diff to a generated file with no matching generator change, or the reverse, is a blocker.
- **Language branching outside an Engine Adapter.** Apart from each family's Engine Adapter, nothing branches on a language (CONTEXT.md, Language Registry). A new `match` or `if` on a language or family anywhere else is a blocker.
- **Rule documents or fixtures in the wrong place.** The layout rules in [docs/repository-structure.md](docs/repository-structure.md) are checked by `npm run check:structure` in CI's lint job; an exemption added to a metric module to let a PR pass needs its reason stated and must be justified by the change.
- **Engine behaviour without a Rust test.** A change to what the engine parses, resolves or reports needs a test in `native/` that fails without it.
- **Vendored grammar changes not recorded.** A change under `native/grammars/` needs its patch in that grammar's `PATCHES.md`, and the regenerated `grammar.json` must match it.
- **Unevaluated rules shipped.** A pack's `candidates/` holds ports under evaluation and is never shipped ([packs/README.md](packs/README.md)). A rule that moves into `rules/` needs the evidence for it in the PR: its conformance results against the upstream tool's recorded findings, and any divergence declared.

## Known gaps

- `real-sessions` and `provider-smoke` (real agent sessions) don't run for PRs from forks or from Dependabot, so a host change from either has no real-session evidence in CI. Say so in the review; mark claims about host behaviour `needs-run`.
- The Rust job runs only when the paths filter in `.github/workflows/ci.yml` matches. A change that affects the engine through another path gets no Rust run.
- Pack test data a PR does not track is not in the review checkout; once pushed, its diff is the `test-data.lock` change (the archive's sha256, file count and bytes). To see what a lock change does to the data, pull the base's and the head's archives (`npm run fixtures:pull` on each) and compare them. CI runs on the archives the PR's locks name, so a PR that changes pins, a generator or a recipe runs on the old derived data until a maintainer derives and pushes it; only then does **Test data reproduces from the pins** derive it in CI.
- Git LFS files (`model/model.safetensors`, `native/src/code_rule_dsl/value/names/names.bin`) are pointer files in the review checkout; their diff is the pointer, not the content.

## High-risk areas

- Migrations: `src/engine/storage/` (the Rerail database in users' repos, its schema and `migrations.ts`) and `native/src/content_store/schema.rs` (the content store's schema and version floor). Check that existing databases still open, nothing is lost, and an older binary still refuses a store it can't read.
- CI and deployment: `.github/` (workflows, actions, Dependabot; always high-risk for the reviewer, except the issue forms and the pull request template, which are text people fill in), `scripts/release/` and `.changeset/config.json`. Check that checks still gate and that a release publishes what it should, where it should.
- Packaging: the root `package.json` and `Cargo.toml`, `native/Cargo.toml`, `native/build.rs`, `scripts/build/`, `tsconfig.build.json`, `packs/*/package.json`, and the agent plugin manifests (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`). Check what the packages contain, their versions and their entry points.
- `src/setup/`: installs rerail into users' agents.
- `LICENSES/` and `scripts/check/licence-allowlist.json`: licensing.

## Reviewer settings

Machine-readable. Paths use CODEOWNERS syntax. A PR touching a high-risk path is never auto-approved. PR size doesn't affect approval. A PR over the summary-only threshold (50,000 changed lines by default; set `largePrLines` here to change it) gets a summary-only review unless someone comments `@neralabs-ci review full`. Generated paths don't count toward that threshold; they are the files in `scripts/generate/README.md`, the packs' upstream recordings (the fixtures, tests, expected answers and lock files under `packs/*/upstream/<tool>/`; not its `gates.yaml`, which pins the upstream a recording runs, nor the scripts and `UPSTREAM.md` beside them), and the regenerated grammar tables.

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
