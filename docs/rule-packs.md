# Rule packs

A rule pack is a named set of rules, origin **By Rerail**. Rules you write yourself are **Created by you** and belong to no pack. Each pack is its own npm package, `@neralabs/rerail-pack-<id>`, and Rerail installs the packs you choose into its home (`$RERAIL_HOME/packs`); core itself ships no packs and no rules. A pack is a directory: its rule documents in `rules/`, and a `pack.json` that lists them with the metadata the dashboard shows (a title and a group for each rule), its match definition, and whether its rules start on.

A rule pack runs only where a repository adds it. Setting a repository up installs and adds the packs the person confirms; after that, packs are added, removed, turned off or given another mode per repository, per folder and per checkout on the rules page or through the API (see [Rule scoping](rule-scoping.md)).

Rerail recommends packs for a repository by matching the repository's files and dependency manifests against each pack's match definition, and by running a pack's own match check where it has one. It does not read the code for violations to do this, it uses no model, and it says nothing about what no pack covers; the only network it uses is npm, to fetch the packs.

## The packs

Every pack lives under `packs/` in this repository and is published from there ([pack format](pack-format.md)). Core keeps the list of their names in `src/engine/catalog/pack-list.generated.ts` (`npm run generate:packs`); what each pack fits is in the pack's own `pack.json`.

| Pack | Category | Rules (on) | Recommended for |
|---|---|--:|---|
| Cyclomatic complexity (limit 6) | language | 3 (3) | Python, TypeScript or JavaScript source outside tests. See [Cyclomatic complexity](#cyclomatic-complexity). |
| JavaScript and TypeScript (ESLint core) | language | 65 (22) | JavaScript or TypeScript source: ESLint's core rules apply to both. |
| TypeScript | language | 22 (11) | a `typescript` dependency, or `*.ts`/`*.tsx` files |
| Python | language | 138 (113) | Python source outside tests, scripts and docs |
| PHP, Blade, Rust, SQL | language | 0 | never yet: every port is still a candidate, and a pack with no rule that starts on is never recommended |
| Next.js | framework | 17 (17) | a `next` dependency |
| Node.js | framework | 22 (22) | an `@types/node` dependency, or `*.mjs`/`*.cjs` files |
| React, Vue | framework | 0 | never yet, as above |

A pack for a project's own conventions is added the same way: a `repository` pack directory with its rule documents in `rules/` and a `pack.json` that lists them, with a match definition naming the files or dependencies that identify the project.

Every rule is listed in the mode its own document has (all `steer` today). A ported rule starts on, unless no approved upstream configuration turns it on or upstream deprecates it: such a rule is experimental and is listed with `"on": false`.

A pack's `mode` and `on` are the values its rules start with in a repository that adds it, and the values a rule a pack upgrade adds starts with; a rule or pack setting in the repository or a checkout overrides them.

A rule honours its linter's suppression comments only where its document says so (`suppress`), and only while the repository's `suppressions` setting for it is `honour` ([rule scoping](rule-scoping.md#rules-attachments-and-settings)); the pack's `honourSuppressionComments` is the value its rules start with.

## Installing packs

Packs are installed into one npm prefix in the Rerail home, `$RERAIL_HOME/packs`, never into a project's `node_modules` (one Rerail serves every repository on the machine, and a Python or PHP repository has none) nor into core's own install (`rerail update` replaces it). Installing uses the person's npm login, as installing Rerail does: whoever can read `@neralabs/rerail` can read every pack.

- **Set-up** (`npx @neralabs/rerail setup` in a repository, or `/rerail-start` in one that is not set up) fetches every listed pack under the update channel's npm dist-tag (`latest` by default; `next` for a pack not yet promoted) into `$RERAIL_HOME/packs/staging`, which installs nothing, checks each one as the catalog does (below), and matches it against the repository, only after the person has said Rerail may read the repository. The confirm question lists each pack that fits with the exact npm package and version a yes installs, and the rules the repository's lint config turns off or configures. Nothing is installed before an answer that installs: "Yes" and "Review them first" install the versions shown (an answer run by a coding agent carries them as `--packs typescript@0.1.1,…`), from npm's cache where discovery left them, and "Choose rule packs…" installs only the packs the person picks from every pack set-up fetched. "I'll pick rule packs myself" reads nothing in the repository: it lists every pack set-up fetched, installs the ones the person picks and adds each for the whole repository, or none ("None for now", `--packs none`). A pack npm cannot give is named on the screen with why (no read access, npm not logged in, the registry unreachable); an installed version stands in for it.
- **`rerail packs`**: `list` shows the installed packs with their versions and any left out, with why; `add <id[@version]> …` installs (a tarball or folder path works too); `remove <id> …` uninstalls; `recommend [--repo <dir>]` runs set-up's discovery and prints what fits, installing nothing.

A running daemon follows an install or removal at once: the catalog is read again when the installed packs change, and every checkout's rules are rebuilt from it. No restart is needed.

Each installed pack is checked on its own when the catalog is read; a pack that fails is left out and reported (`rerail packs list`, `GET /api/rule-packs` `skipped`), and the others load:

1. Its `package.json` is a pack's (`rulePackPackageSchema`), named `@neralabs/rerail-pack-<id>` for the `id` in its `pack.json`.
2. Its `pack.json` is valid (`rulePackFileSchema`, a `match` included), every listed document exists and carries the listed id, and every match check exists.
3. Its core range (`peerDependencies["@neralabs/rerail"]`) includes the active Rerail's version, prereleases included.
4. Every code rule compiles in the engine. A rule refused only as `language-not-supported` loads and stays inactive until the engine reads its language; any other refusal leaves the pack out.
5. No rule it lists is already in another installed pack.

## Where a pack's rules live

An installed pack's files are its rules. Core reads each installed pack's `pack.json` and the rule documents it lists,
keeps them in memory, and reads them again when the installed packs change; nothing copies them into the database. The database holds what people decide
about them: which packs and rules each repository adds (attachments), the `on`, `mode`, `suppressions` and option
values (`config`) set for them (settings), and the rules people write or copy and edit (the library). All of these name
a pack or rule by its id.

The alternative, copying a pack's rules into the database when it is installed and tracking which rows still match
their pack, was rejected: every case below would need code to reconcile the copies with what is installed, edits and
upgrades would collide in one row, and a `rules.json` dump would have to carry pack rules. With the files as the rules,
each case is "whatever is installed now":

| Case | What happens |
|---|---|
| A pack is upgraded | Its new documents apply as soon as it is installed. Settings and attachments keep applying by id. A rule the upgrade adds starts with the pack's `on` and `mode`; a rule it removes disappears, and the settings that name it do nothing. A `config` naming an option the new document no longer declares stops that rule, with the reason, as any such `config` does. |
| A pack is downgraded | The same as an upgrade. |
| A pack is uninstalled | It leaves the catalog. Repositories that add it report it as unavailable (`missing`); their settings and library copies stay, and installing the pack again restores the same configuration. |
| Two versions of a pack are installed | Only one can be: Rerail installs packs into one place (`$RERAIL_HOME/packs`) and reads only `node_modules/@neralabs/rerail-pack-<id>` there. |
| A pack's core range excludes the running core | The pack is skipped and reported, and repositories that add it report it as `missing`. |
| A person edits a pack's rule | The rule is copied into the library and the original switched off where the copy is added; the copy keeps the edit through later upgrades, and the original follows them. Option values are changed with `config` settings instead. |
| `rerail rules export` / `import` | `rules.json` carries attachments and settings by id, never pack rules. On a machine without the pack, its attachments are `missing` until it is installed. |

## Format

This is the pack format, `rerail.rule-pack/v1`. Where packs live in this repository and what each must prove is
specified in [pack-format.md](pack-format.md).

A pack is `packs/<id>/pack.json`, beside the rule documents it lists in `packs/<id>/rules/`. The TypeScript pack (`packs/typescript/`), with one of its rules:

```json
{
  "format": "rerail.rule-pack/v1",
  "id": "typescript",
  "name": "TypeScript",
  "category": "language",
  "covers": "22 typescript-eslint rules ported to Rerail, 11 of them on and 11 off until turned on, each scored against the tool's own recorded findings on its test fixtures.",
  "reason": "matches TypeScript",
  "match": {
    "any": [
      { "dependencies": { "ecosystem": "npm", "names": ["typescript"] } },
      { "files": ["**/*.ts", "**/*.mts", "**/*.cts"], "exclude": ["**/*.d.ts"], "label": "*.ts files" },
      { "files": ["**/*.tsx"], "label": "*.tsx files" }
    ]
  },
  "rules": [
    { "id": "typescript-eslint-ban-tslint-comment", "document": "rules/typescript-eslint-ban-tslint-comment.json", "title": "typescript-eslint ban-tslint-comment: Disallow `// tslint:<rule-flag>` comments", "group": "typescript-eslint", "on": false }
  ]
}
```

- `category` is `language`, `framework` or `repository`. It drives the dashboard's filter chips. Packs are listed languages first, then frameworks, then repository packs.
- `match` (required) is `{ "any": [signal, …] }`: what set-up recommends the pack for. A signal is one of:
  - `{ "files", "exclude"?, "label" }`: repository-relative globs. `**` crosses folders, `*` and `?` stay within one, and `{a,b}` gives alternatives.
  - `{ "dependencies": { "ecosystem", "names" } }`: any manifest in the repository that depends on one of the packages. `npm` reads `package.json` (`dependencies`, `devDependencies`, `peerDependencies`, `optionalDependencies`); `pypi` reads `requirements*.txt` and `pyproject.toml` (PEP 621 `dependencies` and `optional-dependencies`, Poetry's dependency tables less `python`), comparing names normalised as PEP 503 does; `composer` reads `composer.json` (`require`, `require-dev`), case-insensitively.
  - `{ "check": "checks/<name>.mjs", "label" }`: the pack's own code, for what globs and manifests cannot say (see [Matching](#matching)).
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

`test/engine/catalog/packs.test.ts` holds every pack under `packs/` to the [validation contract](pack-format.md#validation-contract), and `test/engine/catalog/rule-packs.test.ts` holds what a pack's match and examples promise: every rule starts in its document's mode, and every example is caught or passes as it says.

## Matching

`recommendRulePacks(checkoutRoot)` lists the checkout's files once and matches every pack against that list:
- Under Git it uses `git ls-files --cached --others --exclude-standard`, less files deleted from the worktree and files a sparse checkout leaves off disk (skip-worktree entries). Outside Git it walks at most 200,000 files.
- Dependency and build-output folders (`node_modules`, `dist`, `.venv`, `vendor`, …) and test-data folders (`fixtures`, `__fixtures__`, `testdata`, `test-fixtures`) are left out.

It then reads every dependency manifest (`package.json`, `requirements*.txt`, `pyproject.toml`, `composer.json`) that is left and is a regular file inside the repository of at most 1 MiB; a symlinked manifest (which Git lists like any tracked file) or a larger one is skipped unread. It returns each fitting pack with its rule count (the rules that start on), reason and folder. No source text is read by core. On a repository of about 4,700 source files a match takes about 90 ms; on cal.com (6,700) about 500 ms.

A `check` signal runs the pack's `checks/<name>.mjs` in its own Node process under Node's permission model: of the file system it may read only the repository and the pack, and it may not write, start processes or workers, or load native addons. Node 24's permission model does not restrict the network (from Node 25 it does), so the guard there is who publishes: set-up fetches only `@neralabs` packs, and npm runs none of their install scripts. Its default export is called with `{ root, files }` (the checkout's real path and the file list above) and answers `{ matched: false }` or `{ matched: true, paths: [<repository-relative path>, …] }`; the paths are its evidence and choose the pack's folder, as a file signal's do. A check that throws, answers anything else, or runs past 5 seconds counts as no match, and set-up prints why. Core caches each check's answer for one scan.

The folder is where set-up adds the pack. A language or framework pack goes in the folder that holds everything it matched (`web` when only `web/` has TypeScript; the whole repository when matches are spread across it). A repository pack goes in the whole repository.

## Cyclomatic complexity

The complexity pack flags a function whose cyclomatic complexity is above 6: one plus the decision points inside it, with lizard 1.24.0's decision points and without its tokenizer defects. On Python it gives lizard's number for every function lizard reads correctly, checked function by function on jinja, click and marshmallow. Each finding names the number, for example `CYCLOMATIC_COMPLEXITY src/grade.py:1: Cyclomatic complexity 8 (limit 6). Split this function into smaller ones or simplify its branching.` What counts in each language, where it deliberately differs from lizard, the real-code comparison and the corpus that proves the rest are in `packs/complexity/README.md`. Test files are not scored.

It reproduces the complexity feedback of the Metrics-in-the-Loop study (the C arm): the same tool semantics, the same limit, and only the functions the agent changed. To use it:

1. Add the `complexity` pack to the repository: set-up offers it for Python, TypeScript or JavaScript source, or install it with `rerail packs add complexity` and add it with `repository_attachments_set` (`PUT /api/repository/attachments`, body `{ "checkout": "<absolute path of a checkout>", "add": [{ "sourceKind": "pack", "sourceId": "complexity" }] }`, or `"repository": "<repository key>"` in place of `checkout`; the whole repository unless the entry names a `folder`). A coding agent may make this call only before the repository is set up; after that a person makes it. Once added, its rules are on.
2. Report only the functions the agent changed: set the workspace's audit scope to `ai-output`, with `rerail settings --session <session-id> --scope ai-output`. A finding covers its whole function, so under `ai-output` a function is reported when an edit touches any of its lines. Under the default `touched-files` scope every function over the limit in an edited file is reported, including ones the agent never touched.

It differs from the study in when and how often the agent hears about it:

- Rerail checks after each edit and speaks only when a function is over the limit. The study measured once after each turn that changed a file, and sent a message even when nothing was over the limit.
- The study's every message also carried a reminder to keep code simple and tested; Rerail sends only the finding.
- The study's coverage feedback (the T and CT arms) is not reproduced: it needs the agent's own tests run under coverage after every turn, and Rerail does not run tests.

## API

The daemon serves the packs, and each call is also an MCP tool:
- `GET /api/rule-packs` (`rule_packs_list`): every installed pack with its version (`null` for a pack read from a folder with no `package.json`) and rules, and `skipped`: each installed pack left out, with its path, package name, version and why.
- `GET /api/rule-packs/recommendations` (`rule_packs_recommend`): the packs that fit a target checkout (`repository` or `checkout`; an MCP call defaults to the calling agent's checkout), each with its rule count (the rules that start on), reason and folder. When the checkout is not on disk the repository's main checkout is scanned, else its most recently seen one. The scan runs synchronously in the daemon (about 0.3 to 0.5 s on a large repository), so a checkout is rescanned at most once a minute.
- `GET /api/rerail-rules` (`rerail_rules_list`) gives each By Rerail rule its `packId`: the pack that lists it, or `null`.

Packs are added to a repository with `PUT /api/repository/attachments` (`repository_attachments_set`), in a folder or for the whole repository; the first addition sets the repository up.
