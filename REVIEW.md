# Review bar for rerail

The automated reviewer ([neralabs/reviewer](https://github.com/neralabs/reviewer)) reads this file from the base branch on every review, on top of the team-wide bar in that repository (`skill/review/team-context.md`). Human reviewers can use it too. Changes to it change what blocks a merge, so review them like code; it is a high-risk path, so a PR that touches it is never auto-approved.

## What rerail is

Rerail runs coding rules inside coding agents (Claude Code, Codex, Cursor, OMP, Pi): a TypeScript package with one host per agent (`src/hosts/`), a Rust engine (`native/`), rule packs (`packs/`), and one user-level SQLite database holding the rule catalog and execution history. It runs on users' machines inside their agent sessions, so a bug in a host or the engine breaks someone's coding session, and a bad pack publish ships to npm. The domain terms (Rule, Clause, Violation, Rule Mode, Audit, Engine Adapter, Language Pack and the rest) are defined in [CONTEXT.md](CONTEXT.md); use them as defined there.

## What blocks a merge here

In addition to the team-wide blockers:

- **Third-party material without provenance.** Code, data or rules from another project need their licence text, a record of where they came from, and a `REUSE.toml` annotation ([CONTRIBUTING.md, "Third-party material"](CONTRIBUTING.md#third-party-material)). In a pack the record is per upstream repository: `packs/<id>/licenses/<project>/UPSTREAM.md`, with its `License:` SPDX expression, its `Source` row, the `Taken from it` globs with how each was copied, and the `Not copied` lines; the files Rerail wrote in a tool directory are listed one per row in that tool's `upstream/<tool>/UPSTREAM.md` ([docs/pack-format.md, "UPSTREAM.md"](docs/pack-format.md#upstreammd)). Only licences on `scripts/check/licence-allowlist.json` are accepted, and an expression passes only when some way to satisfy it uses listed licences alone. Pack check 6 and the publish licence check classify every file the same way and check that each is covered, permitted and not under a `Not copied` glob; they cannot check that the record is true. Check that: the `Source` commit and licence against the repository, each glob against what the generator copies there, and that a `Not copied` line exists for every part of the repository under another licence. An archive's `provenance.json` and `LICENSES.md` are for audit; nothing reads them, so they prove nothing about a PR.
- **Samples from an unrecorded or wrongly recorded repository.** Real-code samples (`samples.lock`, `SAMPLES.md`, a staged `publish/samples/`) are whole files of other repositories. Publish refuses a repository with no entry in `scripts/samples/licences.ts`, a sample whose declared `source.license` is not its repository's entry, and a copyleft repository, so it trusts the entry, not the label ([docs/upstream.md, "Sample licences"](docs/upstream.md#sample-licences)). When a PR adds or changes an entry, open the repository's licence file at the samples' commits and check that it states that licence. In `SAMPLES.md` and the publish summary, check that every repository a samples archive takes files from is one you expect, and that `SAMPLES.md` claims licence texts only for the repositories whose text the archive holds (ESLint test-set samples carry the licence the test set recorded, and no text).
- **A pack change without its test data, or with test data still tracked.** Pack test data (a tool's cases and expected findings under `packs/*/upstream/<tool>/`, `PACK_FIXTURE_PATTERNS` in `scripts/s3/pack-fixtures.mjs`) is not in git: each tool-pack's data is one archive in S3 that its tracked `test-data.lock` names by sha256 ([CONTRIBUTING.md, "Pack test data"](CONTRIBUTING.md#pack-test-data), [docs/pack-format.md, "Pack data"](docs/pack-format.md#pack-data)). New or changed data, authored or derived, is staged into the tool-pack's tracked `publish/` and shows in the diff; removing an authored file is removing its row from the tool's `UPSTREAM.md`, which the publish request comment counts ([CONTRIBUTING.md, "Adding or changing a pack"](CONTRIBUTING.md#adding-or-changing-a-pack)). Review that data like other third-party material. Data a generator makes (upstream's cases, expected findings recorded by running the tool) comes with the pin, generator or recipe change that made it, and a maintainer can derive it again from the pins ([docs/upstream.md, "Derive in a sandbox"](docs/upstream.md#derive-in-a-sandbox)). **Publish pack data** puts the data in S3 once a code owner approves, then commits the new locks and removes `publish/` ([CONTRIBUTING.md, "Merging (maintainers)"](CONTRIBUTING.md#merging-maintainers)). Structure metric #3 fails while any `publish/` or pack test data in the pulled layout is tracked, or a tool directory lacks a well-formed lock; the publish clears it, never an exemption. A PR that changes a lock must pass **Pack test data is in S3**. A lock change that no **Publish pack data** run on the PR made (its comment links the run) was uploaded with `npm run fixtures:push`, which skips prep's licence check and the approval: ask which maintainer pushed it and why, and review the data as you would a publish summary.
- **A data publish approved unread.** The approver approves the bytes that reach S3; see [Approving a pack data upload](#approving-a-pack-data-upload).
- **Hand edits to generated files.** Files listed in [scripts/generate/README.md](scripts/generate/README.md) change through their generator, in the same PR as the generator change. A diff to a generated file with no matching generator change, or the reverse, is a blocker.
- **Language branching outside an Engine Adapter.** Apart from each family's Engine Adapter, nothing branches on a language (CONTEXT.md, Language Registry). A new `match` or `if` on a language or family anywhere else is a blocker.
- **Rule documents or fixtures in the wrong place.** The layout rules in [docs/repository-structure.md](docs/repository-structure.md) are checked by `npm run check:structure` in CI's lint job; an exemption added to a metric module to let a PR pass needs its reason stated and must be justified by the change.
- **Engine behaviour without a Rust test.** A change to what the engine parses, resolves or reports needs a test in `native/` that fails without it.
- **Vendored grammar changes not recorded.** A change under `native/grammars/` needs its patch in that grammar's `PATCHES.md`, and the regenerated `grammar.json` must match it.
- **Unevaluated rules shipped.** A pack's `candidates/` holds ports under evaluation and is never shipped ([packs/README.md](packs/README.md)). A shipped rule (`rules/`) that fails a blocking check fails CI, and the **Conformance** comment lists it; a candidate is only reported, and "ready to promote" means it passes every check, not that it should ship ([conformance/README.md, "The pull request comment"](conformance/README.md#the-pull-request-comment)). A rule that moves into `rules/` needs the evidence for it in the PR: its results against the upstream tool's expected findings, its rename variants and its samples, its `pack.json` `on` as upstream-status sets it (or the reason it differs), and every divergence it declares checked ([conformance/README.md, "Candidates and promotion"](conformance/README.md#candidates-and-promotion), ["Divergences"](conformance/README.md#divergences)). A check the comment lists under "Passing without evidence" (a `holds-under-rename` whose rename variants kept no finding of the rule) is reported, not failed: say whether the rule's other evidence is enough.

## Approving a pack data upload

The code owners are the required reviewers of the `test-data-upload` environment, and **Publish pack data** puts nothing in S3 without an approval on record; the approver's login is tagged on every object. No CI job derives the data again, so the approval is the check that the bytes are what the PR says. Before approving, open the run's **prep** job summary and check it against the PR ([CONTRIBUTING.md, "How pack data is published"](CONTRIBUTING.md#how-pack-data-is-published)):

- **The run.** The summary's heading names the commit prep packed: it is the PR's head. It says `overlay` or `replace`; `replace` only when the PR stages each tool-pack's whole set (`fixtures:stage --all`), since it drops every file `publish/` lacks.
- **The archives.** One row per archive (kind, tool-pack, sha256, files, bytes, licences, sources), for exactly the tool-packs the **Pack data waiting to publish** comment lists. A test data archive's licences are its repositories' `licenses/*/UPSTREAM.md` expressions and Apache-2.0 for Rerail's files; a samples archive's Sources column names every repository it takes files from, with its recorded licence, and each is one you expect.
- **The changes.** Every path added, changed or dropped follows from the PR: added and changed paths from its `publish/` diff, dropped paths from the authored rows it removes (overlay) or the files its staged set lacks (replace). Each list shows at most 50 paths and a replace's dropped files appear nowhere else, so for a longer list pull the base and the head (`npm run fixtures:pull` on each) and compare. Data that no change in the PR explains is a reason to reject.
- **The judgement.** The PR's CI judged the data as published (`publish/` laid over the locked archives): the conformance check passes, or its failures are explained. A fork's change is published only from the PR that ports it, where CI runs.

Reject the deployment when anything does not follow from the PR: the run then commits nothing and changes no lock, and the comment says the publish failed.

## Known gaps

- PRs from forks run no CI: maintainers don't approve fork runs, since CI is costly to run. Review a fork's PR as a proposal; what goes in is ported to a branch here, where CI runs ([CONTRIBUTING.md](CONTRIBUTING.md#proposing-a-change-from-your-fork)).
- `real-sessions` and `provider-smoke` (real agent sessions) don't run for PRs from Dependabot, so a host change from it has no real-session evidence in CI. Say so in the review; mark claims about host behaviour `needs-run`.
- The Rust job runs only when the paths filter in `.github/workflows/ci.yml` matches. A change that affects the engine through another path gets no Rust run.
- Pack test data a PR does not track is not in the review checkout; once published, its diff is the `test-data.lock` change (the archive's sha256, file count and bytes). To see what a lock change does to the data, pull the base's and the head's archives (`npm run fixtures:pull` on each) and compare them. CI runs on the archives the PR's locks name, with any `publish/` laid over them; it never derives data again.
- PRs from Dependabot get no OIDC token, so the jobs that read pack test data (`test-data`, `unit`, `rust`, `tools` with the conformance check, the **Conformance** comment and **Pack test data is in S3**) skip, and run when the change reaches `main`. Only structure metric #3 gates such a PR's pack data. Say so in the review, and mark claims about conformance and the data `needs-run` until a maintainer has run `npm run fixtures:pull`, `npm test` and `npm run test:conformance` on the head.
- Git LFS files (`model/model.safetensors`, `native/src/code_rule_dsl/value/names/names.bin`) are pointer files in the review checkout; their diff is the pointer, not the content.

## High-risk areas

- Migrations: `src/engine/storage/` (the Rerail database in users' repos, its schema and `migrations.ts`) and `native/src/content_store/schema.rs` (the content store's schema and version floor). Check that existing databases still open, nothing is lost, and an older binary still refuses a store it can't read.
- CI and deployment: `.github/` (workflows, actions, Dependabot; always high-risk for the reviewer, except the issue forms and the pull request template, which are text people fill in), `scripts/release/` and `.changeset/config.json`. Check that checks still gate and that a release publishes what it should, where it should.
- Packaging: the root `package.json` and `Cargo.toml`, `native/Cargo.toml`, `native/build.rs`, `scripts/build/`, `tsconfig.build.json`, `packs/*/package.json`, and the agent plugin manifests (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`). Check what the packages contain, their versions and their entry points. Adding or removing `"private": true` in a pack's `package.json` decides whether it ever reaches npm: making one public needs its changeset, `publishConfig` and the [first-publish runbook](docs/publishing-a-new-pack.md); making a published one private strands its npm versions.
- `src/setup/`: installs rerail into users' agents.
- `LICENSES/` and `scripts/check/licence-allowlist.json`: licensing.

## Reviewer settings

Machine-readable. Paths use CODEOWNERS syntax. A PR touching a high-risk path is never auto-approved. PR size doesn't affect approval. A PR over the summary-only threshold (50,000 changed lines by default; set `largePrLines` here to change it) gets a summary-only review unless someone comments `@neralabs-reviewer review full`. Generated paths don't count toward that threshold; they are the files in `scripts/generate/README.md`, the packs' upstream test data (the fixtures, tests, expected findings and lock files under `packs/*/upstream/<tool>/`; not its `gates.yaml`, which declares the upstream pins and facts, nor the scripts and `UPSTREAM.md` beside them), and the regenerated grammar tables.

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
