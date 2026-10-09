# Rule packs

A rule pack is a named set of rules that come with Rerail (origin **By Rerail**). Rules you write yourself are **Created by you** and belong to no pack. A pack is a `packs/<id>/` directory: its rule documents in `rules/`, and a `pack.json` that lists them with the metadata the dashboard shows (a title and a group for each rule), an optional match definition, and whether its rules start on.

**Packs are not attached to repositories yet.** This release defines packs, validates them and recommends the ones that fit a repository. Choosing packs for a repository, and seeding their rules in place of the bundled rules, waits on deciding how rules attach to repositories, clones, worktrees and projects. Until then a repository gets the bundled rules as before.

Rerail recommends packs for a repository by matching the repository's files against each pack's definition. It does not read the code for violations to do this, it uses no model and no network, and it says nothing about what no pack covers. A stack no pack matches gets no recommendation, and a pack without a match definition is never recommended: it is chosen by hand.

## The packs

Only rules that already exist ship as packs. Rerail ships two:

| Pack | Category | Rules | Where it applies | What it is |
|---|---|--:|---|---|
| Cyclomatic complexity (limit 6) | language | 3 | never recommended; chosen by hand | One rule each for Python, TypeScript and JavaScript that flags a function whose cyclomatic complexity, with lizard 1.24.0's decision points, is above 6. The documents are in `packs/complexity/rules/`, seeded off. Group: Complexity. See [Cyclomatic complexity](#cyclomatic-complexity). |
| TSLint | language | 74 | the `typescript` dependency, `*.ts` or `*.tsx` files | The TSLint ports in `packs/tslint/rules/`, with TSLint's default options, each proven against TSLint's own test suite (see `packs/tslint/README.md`). A violation's code is the TSLint rule (`triple-equals`). Group: TSLint. |

The rule lists are the pack files themselves: `packs/complexity/pack.json` and `packs/tslint/pack.json`. A rule's title comes from its document's description. Core ships both packs inside its own package (`CORE_RULE_PACKS` in `src/engine/catalog/rule-bootstrap.ts`, and `package.json` `files`) and seeds their rules disabled; the other packs under `packs/` are not shipped yet.

A pack for a project's own conventions is added the same way: a `repository` pack directory with its rule documents in `rules/` and a `pack.json` that lists them, with a match definition naming the files or dependencies that identify the project. Linter packs for other languages and frameworks wait until core loads installed packs.

Every rule is listed in the mode its own document has (all `steer` today), and every pack lists its rules as on.

**A pack's `mode`, `on` and `honourSuppressionComments` take effect only once packs are attached to repositories.** Until then they describe how the pack would start its rules; they change nothing in the rule catalog.

A rule honours its linter's suppression comments only where its document says so (`suppress`); none of the shipped rules does today.

## Format

This is the pack format, `rerail.rule-pack/v1`. Where packs live in this repository and what each must prove is
specified in [pack-format.md](pack-format.md).

A pack is `packs/<id>/pack.json`, beside the rule documents it lists in `packs/<id>/rules/`:

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
    { "id": "tslint:triple-equals", "document": "rules/triple-equals.json", "title": "Requires === and !== in place of == and !=", "group": "TSLint" }
  ]
}
```

- `category` is `language`, `framework` or `repository`. It drives the dashboard's filter chips. Packs are listed languages first, then frameworks, then repository packs.
- `match` (optional) is `{ "any": [signal, …] }`. Without it the pack is never recommended. A signal is either:
  - `{ "files", "exclude"?, "label" }`: repository-relative globs. `**` crosses folders, `*` and `?` stay within one, and `{a,b}` gives alternatives.
  - `{ "npmDependencies": [...] }`: any `package.json` in the repository that lists one of the packages under `dependencies`, `devDependencies`, `peerDependencies` or `optionalDependencies`.
- `reason` is the pack's own summary. The reason shown for a repository is built from what matched: `matches the typescript dependency in package.json and *.ts files in src`. It names at most two signals. A file signal names the folder its files share, which lets a person see when a pack was picked only because of a stray folder.
- `honourSuppressionComments` (optional, default `true`): whether the pack's rules honour their linters' suppression comments. Takes effect once packs are attached.
- `rules` lists the pack's rules; each rule belongs to exactly one pack.
  - `title` and `group` are what the dashboard shows.
  - `mode` (optional) is the rule mode the pack starts the rule in; without it the rule keeps its document's mode. No shipped pack sets one. Takes effect once packs are attached.
  - `on` (optional, default `true`) is whether the pack starts the rule on. Takes effect once packs are attached.
  - `examples` (optional) is one caught and one passing example, `{ "caught": { "path"?, "text" }, "passes": { … } }`, with a path for a file rule and none for a conversation rule. The complexity pack has one pair per rule; the TSLint ports are proved against TSLint's recorded tests (`packs/tslint/conformance.json`), and the three whose rule TSLint never tests with its default options also have examples.
  - `document` is the rule's document, `rules/<name>.json`, relative to the pack; it must carry the rule's id
    ([pack format](pack-format.md#packjson)).

`test/engine/catalog/rule-packs.test.ts` holds the packs to this: TSLint lists exactly its shipped ports, the complexity pack exactly its three rules, every rule starts in its document's mode, and every example is caught or passes as it says.

## Matching

`recommendRulePacks(repositoryRoot)` lists the repository's files once and matches every pack against that list:
- Under Git it uses `git ls-files --cached --others --exclude-standard`, less files deleted from the worktree and files a sparse checkout leaves off disk (skip-worktree entries). Outside Git it walks at most 200,000 files.
- Dependency and build-output folders (`node_modules`, `dist`, `.venv`, `vendor`, …) and test-data folders (`fixtures`, `__fixtures__`, `testdata`, `test-fixtures`) are left out.

It then reads every `package.json` that is left and is a regular file inside the repository of at most 1 MiB; a symlinked manifest (which Git lists like any tracked file) or a larger one is skipped unread. It returns each fitting pack with its rule count and reason. No source text is read. On a repository of about 4,700 source files a match takes about 90 ms; on cal.com (6,700) about 500 ms.

## Cyclomatic complexity

The complexity pack flags a function whose cyclomatic complexity is above 6: one plus the decision points inside it, with lizard 1.24.0's decision points and without its tokenizer defects. On Python it gives lizard's number for every function lizard reads correctly, checked function by function on jinja, click and marshmallow. Each finding names the number, for example `CYCLOMATIC_COMPLEXITY src/grade.py:1: Cyclomatic complexity 8 (limit 6). Split this function into smaller ones or simplify its branching.` What counts in each language, where it deliberately differs from lizard, the real-code comparison and the corpus that proves the rest are in `packs/complexity/README.md`. Test files are not scored.

It reproduces the complexity feedback of the Metrics-in-the-Loop study (the C arm): the same tool semantics, the same limit, and only the functions the agent changed. To use it:

1. Turn on `complexity:cyclomatic-python`, `complexity:cyclomatic-typescript` and `complexity:cyclomatic-javascript` on the Rules page, or with the `rules_set_enabled` MCP tool. Once packs can be attached to repositories, choosing the pack does this.
2. Report only the functions the agent changed: set the workspace's audit scope to `ai-output`, with `rerail settings --session <session-id> --scope ai-output`. A finding covers its whole function, so under `ai-output` a function is reported when an edit touches any of its lines. Under the default `touched-files` scope every function over the limit in an edited file is reported, including ones the agent never touched.

It differs from the study in when and how often the agent hears about it:

- Rerail checks after each edit and speaks only when a function is over the limit. The study measured once after each turn that changed a file, and sent a message even when nothing was over the limit.
- The study's every message also carried a reminder to keep code simple and tested; Rerail sends only the finding.
- The study's coverage feedback (the T and CT arms) is not reproduced: it needs the agent's own tests run under coverage after every turn, and Rerail does not run tests.

## API

The daemon serves the packs read-only, and each call is also an MCP tool:
- `GET /api/rule-packs` (`rule_packs_list`): every pack with its rules.
- `GET /api/rule-packs/recommendations` (`rule_packs_recommend`): the packs that fit the daemon's repository, with rule counts and reasons. The scan runs synchronously in the daemon (about 0.3 to 0.5 s on a large repository), so the daemon reuses a repository's result for a minute before scanning again.
- `GET /api/rules` gains `packId` on every rule: the pack that lists it, or `null`.

Choosing packs for a repository is not part of the API yet.
