# Review bar for rerail

The automated reviewer ([neralabs/reviewer](https://github.com/neralabs/reviewer)) reads this file from the base branch on every review, on top of the team-wide bar in that repository (`skill/review/team-context.md`). Human reviewers can use it too. Changes to it change what blocks a merge, so review them like code; it is a high-risk path, so a PR that touches it is never auto-approved.

## What rerail is

Rerail runs coding rules inside coding agents (Claude Code, Codex, Cursor, OMP, Pi): a TypeScript package with one host per agent (`src/hosts/`), a Rust engine (`native/`), rule packs (`packs/`), and one user-level SQLite database holding the rule catalog and execution history. It runs on users' machines inside their agent sessions, so a bug in a host or the engine breaks someone's coding session, and a bad pack publish ships to npm. The domain terms (Rule, Clause, Violation, Rule Mode, Audit, Engine Adapter, Language Pack and the rest) are defined in [CONTEXT.md](CONTEXT.md); use them as defined there.

## What blocks a merge here

In addition to the team-wide blockers:

- **Third-party material without provenance.** Code, data or rules from another project need an `UPSTREAM.md` beside the upstream licence text (per repository: the source and its commit, the pack-path patterns of what is taken and how, and what is not copied), and a `REUSE.toml` annotation ([CONTRIBUTING.md](CONTRIBUTING.md)). Pack test data's per-file record is `provenance.json` inside each test-data archive, for audit; nothing reads it. Only licences on `scripts/check/licence-allowlist.json` are accepted. CI's licence job checks that licences are declared and permissive, and pack check 6 that every copied file falls under a declared pattern, not that the `UPSTREAM.md` is accurate; check that. Real-code samples (`samples.lock`, `SAMPLES.md`, a staged `publish/samples/`) are whole files of other repositories: sample publish refuses a source repository with no entry in `scripts/samples/licences.ts`, a declared `source.license` that is not its entry, and a copyleft source, so publish trusts the entry, not the sample's label. When a PR adds or changes an entry, open the repository's licence file at the samples' commits and check it states that licence; ESLint test-set entries carry the licence the test set recorded, and their archives hold no licence text. In the publish summary and `SAMPLES.md`, check that every repository a samples archive takes files from is one you expect, under its entry's licence.
- **A pack change without its test data, or with test data still tracked.** Pack test data (a tool's cases and expected findings under `packs/*/upstream/<tool>/`, `PACK_FIXTURE_PATTERNS` in `scripts/s3/pack-fixtures.mjs`) is not in git: each tool-pack's data is one archive in S3 that its tracked `test-data.lock` names by sha256 ([CONTRIBUTING.md](CONTRIBUTING.md#pack-test-data), [docs/upstream.md](docs/upstream.md)). Derived data (upstream's tests, copied or derived, their rename variants and the expected findings) is regenerated from the pins by a maintainer (`npm run fixtures:derive`) or the bump workflow: a PR changes it by changing the pins, a generator or a recipe under `scripts/pack-data/recipes/`, so review that change; it is the code a maintainer then runs to derive, in a sandbox without credentials. New or changed data, derived or authored (cases Rerail wrote for a tool, listed in its `UPSTREAM.md`, or a code corpus with its ground truth under `upstream/<tool>/authored/`), is staged into the tool-pack's tracked `publish/` for review ([CONTRIBUTING.md](CONTRIBUTING.md#adding-or-changing-a-pack)); removing an authored file is removing its `UPSTREAM.md` row. Review it like other third-party material. The **Publish pack data** workflow then puts it in S3 after a code owner approves, and commits the new locks and removes `publish/` ([CONTRIBUTING.md](CONTRIBUTING.md#merging-maintainers)). A PR that changes a lock must pass **Pack test data is in S3** (S3 holds what every lock names). Structure metric #3 fails while any `publish/` or pack test data in the pulled layout is tracked, or a tool directory lacks a well-formed lock; that is cleared by the publish, never by an exemption.
- **A data publish approved unread.** Before approving a **Publish pack data** run's `test-data-upload` deployment, read its prep summary (each archive's sha256, files, bytes, licences, a samples archive's source repositories with their licences, and what it adds, changes and drops) against the PR's `publish/` diff: the approver approves the bytes that reach S3.
- **Hand edits to generated files.** Files listed in [scripts/generate/README.md](scripts/generate/README.md) change through their generator, in the same PR as the generator change. A diff to a generated file with no matching generator change, or the reverse, is a blocker.
- **Language branching outside an Engine Adapter.** Apart from each family's Engine Adapter, nothing branches on a language (CONTEXT.md, Language Registry). A new `match` or `if` on a language or family anywhere else is a blocker.
- **Rule documents or fixtures in the wrong place.** The layout rules in [docs/repository-structure.md](docs/repository-structure.md) are checked by `npm run check:structure` in CI's lint job; an exemption added to a metric module to let a PR pass needs its reason stated and must be justified by the change.
- **Engine behaviour without a Rust test.** A change to what the engine parses, resolves or reports needs a test in `native/` that fails without it.
- **Vendored grammar changes not recorded.** A change under `native/grammars/` needs its patch in that grammar's `PATCHES.md`, and the regenerated `grammar.json` must match it.
- **Unevaluated rules shipped.** A pack's `candidates/` holds ports under evaluation and is never shipped ([packs/README.md](packs/README.md)). A rule that moves into `rules/` needs the evidence for it in the PR: its conformance results against the upstream tool's expected findings (rename variants and samples included), and any divergence declared.

## Known gaps

- `real-sessions` and `provider-smoke` (real agent sessions) don't run for PRs from forks or from Dependabot, so a host change from either has no real-session evidence in CI. Say so in the review; mark claims about host behaviour `needs-run`.
- The Rust job runs only when the paths filter in `.github/workflows/ci.yml` matches. A change that affects the engine through another path gets no Rust run.
- Pack test data a PR does not track is not in the review checkout; once published, its diff is the `test-data.lock` change (the archive's sha256, file count and bytes). To see what a lock change does to the data, pull the base's and the head's archives (`npm run fixtures:pull` on each) and compare them. CI runs on the archives the PR's locks name, with any `publish/` laid over them; it never derives data again, so a PR that changes pins, a generator or a recipe runs on the old derived data until the new data is derived and staged into `publish/`.
- Git LFS files (`model/model.safetensors`, `native/src/code_rule_dsl/value/names/names.bin`) are pointer files in the review checkout; their diff is the pointer, not the content.

## High-risk areas

- Migrations: `src/engine/storage/` (the Rerail database in users' repos, its schema and `migrations.ts`) and `native/src/content_store/schema.rs` (the content store's schema and version floor). Check that existing databases still open, nothing is lost, and an older binary still refuses a store it can't read.
- CI and deployment: `.github/` (workflows, actions, Dependabot; always high-risk for the reviewer, except the issue forms and the pull request template, which are text people fill in), `scripts/release/` and `.changeset/config.json`. Check that checks still gate and that a release publishes what it should, where it should.
- Packaging: the root `package.json` and `Cargo.toml`, `native/Cargo.toml`, `native/build.rs`, `scripts/build/`, `tsconfig.build.json`, `packs/*/package.json`, and the agent plugin manifests (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`). Check what the packages contain, their versions and their entry points.
- `src/setup/`: installs rerail into users' agents.
- `LICENSES/` and `scripts/check/licence-allowlist.json`: licensing.

## Reviewer settings

Machine-readable. Paths use CODEOWNERS syntax. A PR touching a high-risk path is never auto-approved. PR size doesn't affect approval. A PR over the summary-only threshold (50,000 changed lines by default; set `largePrLines` here to change it) gets a summary-only review unless someone comments `@neralabs-ci review full`. Generated paths don't count toward that threshold; they are the files in `scripts/generate/README.md`, the packs' upstream test data (the fixtures, tests, expected findings and lock files under `packs/*/upstream/<tool>/`; not its `gates.yaml`, which declares the upstream pins and facts, nor the scripts and `UPSTREAM.md` beside them), and the regenerated grammar tables.

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
