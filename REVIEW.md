# Review bar for rerail

The automated reviewer ([neralabs/reviewer](https://github.com/neralabs/reviewer)) reads this file from the base branch on every review, on top of the team-wide bar in that repository (`skill/review/team-context.md`). Human reviewers can use it too. Changes to it change what blocks a merge, so review them like code; it is a high-risk path, so a PR that touches it is never auto-approved.

## What rerail is

Rerail runs coding rules inside coding agents (Claude Code, Codex, Cursor, OMP, Pi): a TypeScript package with one host per agent (`src/hosts/`), a Rust engine (`native/`), rule packs (`packs/`), and one user-level SQLite database holding the rule catalog and execution history. It runs on users' machines inside their agent sessions, so a bug in a host or the engine breaks someone's coding session, and a bad pack publish ships to npm. The domain terms (Rule, Clause, Violation, Rule Mode, Audit, Engine Adapter, Language Pack and the rest) are defined in [CONTEXT.md](CONTEXT.md); use them as defined there.

## What blocks a merge here

In addition to the team-wide blockers:

- **Third-party material without provenance.** Code, data or rules from another project need an `UPSTREAM.md` entry beside them (source, version, files, verbatim/derived/excerpt), the upstream licence text, and a `REUSE.toml` annotation ([CONTRIBUTING.md](CONTRIBUTING.md)). Only licences on `scripts/check/licence-allowlist.json` are accepted. CI's licence job checks that licences are declared and permissive, not that the `UPSTREAM.md` entry is accurate; check that.
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

- `src/hosts/` and `hooks/`: run inside each coding agent's session. Check failure handling: a host must not break the agent's session when rerail fails.
- `src/engine/storage/` and `src/runtime/`: the Rerail database and its repair. Check that existing databases still open and nothing is lost.
- `src/setup/`: installs rerail into users' agents.
- `packs/*/package.json` and the release workflows: what gets published.
- `languages/`, `LICENSES/`, `REUSE.toml`, `scripts/check/licence-allowlist.json`.

## Reviewer settings

Machine-readable. Paths use CODEOWNERS syntax. A PR touching a high-risk path is never auto-approved. PR size doesn't affect approval. A PR over the summary-only threshold (50,000 changed lines by default; set `largePrLines` here to change it) gets a summary-only review unless someone comments `@neralabs-ci review full`. Generated paths don't count toward that threshold; they are the files in `scripts/generate/README.md`, the packs' upstream recordings, and the regenerated grammar tables.

```json reviewer
{
  "highRiskPaths": [
    "src/hosts/",
    "hooks/",
    "src/engine/storage/",
    "src/runtime/",
    "src/setup/",
    "packs/*/package.json",
    "languages/",
    "LICENSES/",
    "REUSE.toml",
    "scripts/check/licence-allowlist.json"
  ],
  "generatedPaths": [
    "*.generated.*",
    "native/src/code_rule_dsl/value/names/names.bin",
    "native/grammars/**/grammar.json",
    "model/",
    "packs/*/upstream/",
    "packs/node/rules/",
    "packs/node/candidates/",
    "packs/typescript/candidates/rules/vitest-*.json"
  ]
}
```
