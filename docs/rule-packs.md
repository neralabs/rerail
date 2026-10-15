# Rule packs

A rule pack is a named set of rules that come with Rerail (origin **By Rerail**). Rules you write yourself are **Created by you** and belong to no pack. A pack is a `packs/<id>/` directory: its rule documents in `rules/`, and a `pack.json` that lists them with the metadata the dashboard shows (a title and a group for each rule), an optional match definition, and whether its rules start on.

A rule pack runs only where a repository adds it. Setting a repository up adds the packs recommended for it; after that, packs are added, removed, turned off or given another mode per repository, per folder and per checkout on the rules page or through the API (see [Rule scoping](rule-scoping.md)).

Rerail recommends packs for a repository by matching the repository's files against each pack's definition. It does not read the code for violations to do this, it uses no model and no network, and it says nothing about what no pack covers. A stack no pack matches gets no recommendation, and a pack without a match definition is never recommended: it is chosen by hand.

## The packs

Only rules that already exist ship as packs. Rerail ships one:

| Pack | Category | Rules | Where it applies | What it is |
|---|---|--:|---|---|
| Cyclomatic complexity (limit 6) | language | 3 | never recommended; runs only in repositories that add it | One rule each for Python, TypeScript and JavaScript that flags a function whose cyclomatic complexity, with lizard 1.24.0's decision points, is above 6. The documents are in `packs/complexity/rules/`. Group: Complexity. See [Cyclomatic complexity](#cyclomatic-complexity). |

The rule list is the pack file itself: `packs/complexity/pack.json`. A rule's title comes from its document's description. Core ships the pack inside its own package (`CORE_RULE_PACKS` in `src/engine/catalog/bundled-rule-documents.ts`, and `package.json` `files`); its rules run only in repositories that add it. It has no match definition, so setting a repository up adds no pack. The other packs under `packs/` are not shipped yet.

A pack for a project's own conventions is added the same way: a `repository` pack directory with its rule documents in `rules/` and a `pack.json` that lists them, with a match definition naming the files or dependencies that identify the project. Linter packs for other languages and frameworks wait until core loads installed packs.

Every rule is listed in the mode its own document has (all `steer` today). A ported rule starts on, unless no approved upstream configuration turns it on or upstream deprecates it: such a rule is experimental and is listed with `"on": false`.

A pack's `mode` and `on` are the values its rules start with in a repository that adds it; a rule or pack setting in the repository or a checkout overrides them.

A rule honours its linter's suppression comments only where its document says so (`suppress`), and only while the repository's `suppressions` setting for it is `honour` ([rule scoping](rule-scoping.md#rules-attachments-and-settings)); the pack's `honourSuppressionComments` is the value its rules start with. None of the shipped rules reads a linter's comments today.

## Format

This is the pack format, `rerail.rule-pack/v1`. Where packs live in this repository and what each must prove is
specified in [pack-format.md](pack-format.md).

A pack is `packs/<id>/pack.json`, beside the rule documents it lists in `packs/<id>/rules/`. The TypeScript pack (`packs/typescript/`, not shipped yet), with one of its rules:

```json
{
  "format": "rerail.rule-pack/v1",
  "id": "typescript",
  "name": "TypeScript",
  "category": "language",
  "covers": "21 ESLint and typescript-eslint rules ported to Rerail, each scored against the tool's own recorded findings on its test fixtures.",
  "reason": "matches TypeScript",
  "match": {
    "any": [
      { "npmDependencies": ["typescript"] },
      { "files": ["**/*.ts", "**/*.mts", "**/*.cts"], "exclude": ["**/*.d.ts"], "label": "*.ts files" },
      { "files": ["**/*.tsx"], "label": "*.tsx files" }
    ]
  },
  "rules": [
    { "id": "eslint-eqeqeq", "document": "rules/eslint-eqeqeq.json", "title": "ESLint eqeqeq: Require the use of === and !==", "group": "ESLint" }
  ]
}
```

- `category` is `language`, `framework` or `repository`. It drives the dashboard's filter chips. Packs are listed languages first, then frameworks, then repository packs.
- `match` (optional) is `{ "any": [signal, …] }`. Without it the pack is never recommended. A signal is either:
  - `{ "files", "exclude"?, "label" }`: repository-relative globs. `**` crosses folders, `*` and `?` stay within one, and `{a,b}` gives alternatives.
  - `{ "npmDependencies": [...] }`: any `package.json` in the repository that lists one of the packages under `dependencies`, `devDependencies`, `peerDependencies` or `optionalDependencies`.
- `reason` is the pack's own summary. The reason shown for a repository is built from what matched: `matches the typescript dependency in package.json and *.ts files in src`. It names at most two signals. A file signal names the folder its files share, which lets a person see when a pack was picked only because of a stray folder.
- `honourSuppressionComments` (optional, default `true`): whether the pack's rules start out honouring their linters' suppression comments, the pack's start for the `suppressions` rule setting. With `false`, no linter's comment silences them until a repository or checkout sets `suppressions: "honour"`; Rerail's own `rerail-ignore` always does.
- `rules` lists the pack's rules; each rule belongs to exactly one pack. A pack may list no rules while all its ports are still candidates, and such a pack is never recommended; neither is a pack whose rules all start off.
  - `id` is lowercase letters, digits and dashes, the upstream tool's name then its rule (`eslint-eqeqeq`, `ruff-b011-assert-false`); every pack uses this one style. It names the rule's document and its port in `conformance.json`, and people turn the rule on and off by it, so it does not change once the pack is published.
  - `title` and `group` are what the dashboard shows.
  - `mode` (optional) is the rule mode the pack starts the rule in; without it the rule keeps its document's mode. No shipped pack sets one.
  - `on` (optional, default `true`) is whether the pack starts the rule on.
  - `examples` (optional) is one caught and one passing example, `{ "caught": { "path"?, "text" }, "passes": { … } }`, with a path for a code rule and none for a conversation rule. The complexity pack has one pair per rule.
  - `document` is the rule's document, `rules/<id>.json`, relative to the pack; it must carry the rule's id
    ([pack format](pack-format.md#packjson)).

`test/engine/catalog/rule-packs.test.ts` holds the shipped packs to this: complexity is the only one and lists exactly its three rules, every rule starts in its document's mode, and every example is caught or passes as it says.

## Matching

`recommendRulePacks(checkoutRoot)` lists the checkout's files once and matches every pack against that list:
- Under Git it uses `git ls-files --cached --others --exclude-standard`, less files deleted from the worktree and files a sparse checkout leaves off disk (skip-worktree entries). Outside Git it walks at most 200,000 files.
- Dependency and build-output folders (`node_modules`, `dist`, `.venv`, `vendor`, …) and test-data folders (`fixtures`, `__fixtures__`, `testdata`, `test-fixtures`) are left out.

It then reads every `package.json` that is left and is a regular file inside the repository of at most 1 MiB; a symlinked manifest (which Git lists like any tracked file) or a larger one is skipped unread. It returns each fitting pack with its rule count (the rules that start on), reason and folder. No source text is read. On a repository of about 4,700 source files a match takes about 90 ms; on cal.com (6,700) about 500 ms.

The folder is where set-up adds the pack. A language or framework pack goes in the folder that holds everything it matched (`web` when only `web/` has TypeScript; the whole repository when matches are spread across it). A repository pack goes in the whole repository.

## Cyclomatic complexity

The complexity pack flags a function whose cyclomatic complexity is above 6: one plus the decision points inside it, with lizard 1.24.0's decision points and without its tokenizer defects. On Python it gives lizard's number for every function lizard reads correctly, checked function by function on jinja, click and marshmallow. Each finding names the number, for example `CYCLOMATIC_COMPLEXITY src/grade.py:1: Cyclomatic complexity 8 (limit 6). Split this function into smaller ones or simplify its branching.` What counts in each language, where it deliberately differs from lizard, the real-code comparison and the corpus that proves the rest are in `packs/complexity/README.md`. Test files are not scored.

It reproduces the complexity feedback of the Metrics-in-the-Loop study (the C arm): the same tool semantics, the same limit, and only the functions the agent changed. To use it:

1. Add the `complexity` pack to the repository with `repository_attachments_set` (`PUT /api/repository/attachments`, body `{ "checkout": "<absolute path of a checkout>", "add": [{ "sourceKind": "pack", "sourceId": "complexity" }] }`, or `"repository": "<repository key>"` in place of `checkout`; the whole repository unless the entry names a `folder`). Set-up never adds it, because it has no match definition. A coding agent may make this call only before the repository is set up; after that a person makes it.
2. Report only the functions the agent changed: set the workspace's audit scope to `ai-output`, with `rerail settings --session <session-id> --scope ai-output`. A finding covers its whole function, so under `ai-output` a function is reported when an edit touches any of its lines. Under the default `touched-files` scope every function over the limit in an edited file is reported, including ones the agent never touched.

It differs from the study in when and how often the agent hears about it:

- Rerail checks after each edit and speaks only when a function is over the limit. The study measured once after each turn that changed a file, and sent a message even when nothing was over the limit.
- The study's every message also carried a reminder to keep code simple and tested; Rerail sends only the finding.
- The study's coverage feedback (the T and CT arms) is not reproduced: it needs the agent's own tests run under coverage after every turn, and Rerail does not run tests.

## API

The daemon serves the packs, and each call is also an MCP tool:
- `GET /api/rule-packs` (`rule_packs_list`): every pack with its rules.
- `GET /api/rule-packs/recommendations` (`rule_packs_recommend`): the packs that fit a target checkout (`repository` or `checkout`; an MCP call defaults to the calling agent's checkout), each with its rule count (the rules that start on), reason and folder. When the checkout is not on disk the repository's main checkout is scanned, else its most recently seen one. The scan runs synchronously in the daemon (about 0.3 to 0.5 s on a large repository), so a checkout is rescanned at most once a minute.
- `GET /api/rerail-rules` (`rerail_rules_list`) gives each By Rerail rule its `packId`: the pack that lists it, or `null`.

Packs are added to a repository with `PUT /api/repository/attachments` (`repository_attachments_set`), in a folder or for the whole repository; the first addition sets the repository up.
