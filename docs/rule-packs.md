# Rule packs

A rule pack is a named set of rules that come with Rerail (origin **By Rerail**). Rules you write yourself are **Created by you** and belong to no pack. A pack holds no rules of its own: it lists rules that already ship with Rerail, and adds the metadata the dashboard shows (a title and a group for each rule), a match definition, and whether its rules start on.

**Packs are not attached to repositories yet.** This release defines packs, validates them and recommends the ones that fit a repository. Choosing packs for a repository, and seeding their rules in place of the bundled rules, waits on deciding how rules attach to repositories, clones, worktrees and projects. Until then a repository gets the bundled rules as before.

Rerail recommends packs for a repository by matching the repository's files against each pack's definition. It does not read the code for violations to do this, it uses no model and no network, and it says nothing about what no pack covers. A stack no pack matches gets no recommendation.

## The packs

Only rules that already exist ship as packs. Rerail ships one:

| Pack | Category | Rules | Where it applies | What it is |
|---|---|--:|---|---|
| TSLint | language | 74 | the `typescript` dependency, `*.ts` or `*.tsx` files | The golden TSLint ports in `policies/executable-policy-dsl/rules/opt-in/tslint/`: each passed TSLint's own test suite in the conformance harness, with TSLint's default options. Group: TSLint. |

The rule list is the pack file itself: `policies/rule-packs/tslint.json`. A rule's title comes from its document's description.

A pack for a project's own conventions is added the same way: its rule documents under `policies/executable-policy-dsl/rules/` and a `repository` pack file in `policies/rule-packs/` that lists them, with a match definition naming the files or dependencies that identify the project. Linter packs for other languages and frameworks wait until their rule ports exist.

Every rule is listed in the mode its own document has (all `steer` today), and the pack lists its rules as on.

**A pack's `mode`, `on` and `honourSuppressionComments` take effect only once packs are attached to repositories.** Until then they describe how the pack would start its rules; they change nothing in the rule catalog.

A rule honours its linter's suppression comments only where its document says so (`suppress`); none of the shipped rules does today.

## Format

A pack is one file, `policies/rule-packs/<id>.json`, kept apart from the rule documents it lists:

```json
{
  "format": "rerail.rule-pack/v1",
  "id": "tslint",
  "name": "TSLint",
  "category": "language",
  "covers": "The 74 TSLint core rules ported to Rerail that pass TSLint's own test suite, with TSLint's default options.",
  "reason": "matches TypeScript",
  "match": {
    "any": [
      { "npmDependencies": ["typescript"] },
      { "files": ["**/*.ts", "**/*.mts", "**/*.cts"], "exclude": ["**/*.d.ts"], "label": "*.ts files" },
      { "files": ["**/*.tsx"], "label": "*.tsx files" }
    ]
  },
  "rules": [
    { "id": "tslint:triple-equals", "title": "Requires === and !== in place of == and !=", "group": "TSLint" }
  ]
}
```

- `category` is `language`, `framework` or `repository`. It drives the dashboard's filter chips. Packs are listed languages first, then frameworks, then repository packs.
- `match` is `{ "any": [signal, …] }`. A signal is either:
  - `{ "files", "exclude"?, "label" }`: repository-relative globs. `**` crosses folders, `*` and `?` stay within one, and `{a,b}` gives alternatives.
  - `{ "npmDependencies": [...] }`: any `package.json` in the repository that lists one of the packages under `dependencies`, `devDependencies`, `peerDependencies` or `optionalDependencies`.
- `reason` is the pack's own summary. The reason shown for a repository is built from what matched: `matches the typescript dependency in package.json and *.ts files in src`. It names at most two signals. A file signal names the folder its files share, which lets a person see when a pack was picked only because of a stray folder.
- `honourSuppressionComments` (optional, default `true`): whether the pack's rules honour their linters' suppression comments. Takes effect once packs are attached.
- `rules` lists bundled rules by id; each rule belongs to exactly one pack, and a pack adds no rule documents.
  - `title` and `group` are what the dashboard shows.
  - `mode` (optional) is the rule mode the pack starts the rule in; without it the rule keeps its document's mode. The shipped pack sets none. Takes effect once packs are attached.
  - `on` (optional, default `true`) is whether the pack starts the rule on. Takes effect once packs are attached.
  - `examples` (optional) is one caught and one passing example, `{ "caught": { "path"?, "text" }, "passes": { … } }`, with a path for a file rule and none for a conversation rule. The shipped pack has none: the TSLint ports are proved by the conformance harness (`golden.json`).

`test/rule-packs.test.ts` holds the packs to this: TSLint lists exactly the golden ports, and every rule starts in its document's mode.

## Matching

`recommendRulePacks(repositoryRoot)` lists the repository's files once and matches every pack against that list:
- Under Git it uses `git ls-files --cached --others --exclude-standard`, less files deleted from the worktree and files a sparse checkout leaves off disk (skip-worktree entries). Outside Git it walks at most 200,000 files.
- Dependency and build-output folders (`node_modules`, `dist`, `.venv`, `vendor`, …) and test-data folders (`fixtures`, `__fixtures__`, `testdata`, `test-fixtures`) are left out.

It then reads every `package.json` that is left and is a regular file inside the repository of at most 1 MiB; a symlinked manifest (which Git lists like any tracked file) or a larger one is skipped unread. It returns each fitting pack with its rule count and reason. No source text is read. On a repository of about 4,700 source files a match takes about 90 ms; on cal.com (6,700) about 500 ms.

## API

The daemon serves the packs read-only, and each call is also an MCP tool:
- `GET /api/rule-packs` (`rule_packs_list`): every pack with its rules.
- `GET /api/rule-packs/recommendations` (`rule_packs_recommend`): the packs that fit the daemon's repository, with rule counts and reasons. The scan runs synchronously in the daemon (about 0.3 to 0.5 s on a large repository), so the daemon reuses a repository's result for a minute before scanning again.
- `GET /api/rules` gains `packId` on every rule: the pack that lists it, or `null`.

Choosing packs for a repository is not part of the API yet.
