# Rule packs

A rule pack is a named set of rules, published as its own npm package. Core ships no packs and no rules.

| Label | Whose | Package |
|---|---|---|
| **By Rerail** | Rerail's | `@neralabs/rerail-pack-<id>` |
| **By** publisher (`By @acme`) | a third party's, not reviewed by Rerail ([Third parties' packs](#third-parties-packs)) | `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` |
| **Created by you** | yours, in no pack | none |

Rerail installs the packs you choose into `$RERAIL_HOME/packs`. A pack runs only where a repository adds it ([Rule scoping](rule-scoping.md)).

<details>
<summary>What a pack holds, who adds it, how packs are recommended</summary>

A pack directory holds rule documents in `rules/` and a `pack.json`. The `pack.json` lists them with the metadata the dashboard shows (a title and group per rule), a match definition, and whether its rules start on.

Set-up installs and adds the packs the person confirms. Afterwards the dashboard adds and removes packs and turns their rules on or off, for the repository. The API and its MCP tools add and remove packs, turn rules off or change their mode, per repository, folder and checkout.

Recommendations match the repository's files and dependency manifests against each match definition. Matching is data only: no pack code, no model, no reading code for violations, no network but npm to fetch the packs. Rerail says nothing about what no pack covers.

</details>

## The packs

Every pack lives under `packs/` in this repository and is published from there ([pack format](pack-format.md)), except
a private pack (`"private": true` in its `package.json`): it is never published and is not in
`src/engine/catalog/pack-list.generated.ts`; install it from a checkout with `rerail packs add packs/<id>`, and set-up
then offers it from that copy.

| Pack | Category | Rules (on) | Recommended for |
|---|---|--:|---|
| Cyclomatic complexity | language | 7 (7) | Python, TypeScript, JavaScript, PHP, Blade, Rust or Vue source outside tests. See [Cyclomatic complexity](#cyclomatic-complexity). |
| JavaScript | language | 121 (36) | JavaScript or TypeScript source. ESLint core rules apply to both; eslint-plugin-unicorn rules to JavaScript files only. |
| TypeScript | language | 65 (23) | a `typescript` dependency, or `*.ts`/`*.tsx` files |
| Python | language | 537 (317) | Python source outside tests, scripts and docs |
| PHP | language | 15 (5) | PHP source: `*.php` files, not Blade views |
| Blade | language | 10 (8) | Blade templates: `*.blade.php` files |
| Rust | language | 33 (17) | `*.rs` files |
| SQL | language | 21 (19) | `*.sql` files |
| Next.js | framework | 21 (21) | a `next` dependency |
| Node.js | framework | 37 (11) | a Node.js server framework (`express`, `koa`, `fastify`, `@nestjs/core` and others) or `electron` dependency, or `*.cjs`/`*.mjs`/`*.cts`/`*.mts` files outside tests, scripts, fixtures and configuration |
| Vue | framework | 23 (21) | `*.vue` files |
| React | framework | 51 (32) | a `react` dependency, or `*.jsx` files |

- **A project's own pack** is a `repository` pack you install from its folder with `rerail packs add <folder>`. It counts as a third party's pack.
- **Starting values** are a pack's `mode` and `on`: what its rules start with in a repository that adds it. Repository or checkout settings override them.

<details>
<summary>Pack list, own packs and starting values in detail</summary>

Core lists pack names in `src/engine/catalog/pack-list.generated.ts` (`npm run generate:packs`); each `pack.json` says what the pack fits. Rerail recommends a pack only if at least one of its rules starts on; otherwise it would run nothing.

A project's own pack holds rule documents in `rules/` and a `pack.json` whose match definition names the files or dependencies that identify the project. Its `package.json` is named `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>`, with core's range under `peerDependencies`.

Starting values also apply to rules a later upgrade adds.

- **Mode:** the rule document's (`steer`), unless `pack.json` sets one. A port of a rule upstream only warns about is listed with `"mode": "log"` ([pack format](pack-format.md#packjson)).
- **On:** a ported rule starts on, unless no approved upstream configuration turns it on or upstream deprecates it. Such a rule is experimental, listed with `"on": false`.
- **Suppressions:** a rule honours its linter's suppression comments only if its document says so (`suppress`) and the repository's `suppressions` setting for it is `honour` ([rule scoping](rule-scoping.md#rules-attachments-and-settings)). The pack's `honourSuppressionComments` is the starting value.

</details>

## Installing packs

All packs install into one npm prefix, `$RERAIL_HOME/packs`, with the person's npm login. npm runs no install script.

<details>
<summary>Why one prefix</summary>

Packs never install into a project's `node_modules`: one Rerail serves every repository on the machine, and Python or PHP repositories have none. Nor into core's install, which `rerail update` replaces. Installing uses the npm login that installs Rerail, so whoever can read `@neralabs/rerail` can read every pack.

</details>

### Set-up

`npx @neralabs/rerail setup` in a repository, or `/rerail-start` in one not set up, offers packs in five steps.

1. **Fetch** every listed pack into a staging folder. Nothing in the repository is read until the person allows it.
2. **Keep newer versions** the computer already has installed.
3. **Ask** which fitting packs to install.
4. **Install only what the answer carries:** "Yes", "Change rule packs…" (`--answer choose`), "Not now" (`--answer not-now`) or "Skip, I'll pick packs".
5. **Verify** the catalog still loads every pack; otherwise put the change back and ask again.

<details>
<summary>Each step in detail</summary>

1. **Fetch.** Set-up fetches every listed pack under the update channel's npm dist-tag: `latest` by default, where each pack version is published ([pack format](pack-format.md#release)), or `next` for a pack with no `latest`. They land in a staging folder under `$RERAIL_HOME/packs/staging`, which installs nothing and is removed once read. Each is checked as the [catalog](#catalog-checks) does.
2. **Keep newer versions.** A pack installed at a newer version than the tag's (`rerail packs add typescript@next`) is offered at that version. If it does not load (its core range, after setting up an older core again, say), set-up names it as unusable, with why, and does not offer it. An answer naming an older version than the installed one is refused; `rerail packs add` downgrades.
3. **Ask.** Set-up lists each fitting pack with its rule count and why it fits, and the rules the repository's lint config turns off or configures. It warns when a yes replaces an installed version, which every repository on the computer runs. A fitting pack with no rule that starts on is named as having no rules yet, as is every such pack under "Skip, I'll pick packs".
4. **Install only what the answer carries.**
   - **"Yes"** installs the versions shown, from npm's cache where discovery left them; a coding agent passes `--packs typescript@0.1.1,…`.
   - **"Change rule packs…"** (`--answer choose`) installs only the packs picked from those fetched, replacing any an earlier dashboard review added. Not offered if no pack was fetched.
   - **"Not now"** (`--answer not-now`), or asking again after a bad answer, installs nothing.
   - **"Skip, I'll pick packs"** reads nothing in the repository. It installs the fetched packs picked and adds each for the whole repository without its lint config, or none ("None for now", `--packs none`).
5. **Verify.** The catalog must load every pack named and every pack it loaded before. If npm fails or a pack is left out (core range, a rule another pack holds), every changed pack is put back as npm recorded it (a folder pack from its folder) or removed. Set-up asks again, saying why: no read access, npm not logged in, registry unreachable, or why the pack was left out. An installed version stands in for one npm cannot give.

</details>

### `rerail packs`

- **`list`**: installed packs with versions, and any left out, with why.
- **`add <id[@version]> …`**: installs a Rerail pack by id, or any pack by package name.
- **`remove <id | package> …`**: uninstalls a package by name, or every installed package holding a pack id.
- **`recommend [--repo <dir>]`**: runs set-up's discovery and prints what fits; installs nothing.

<details>
<summary>Arguments and refusals</summary>

- **`list`** names Rerail's packs by id and a third party's by package name.
- **`add`** takes ids or package names (`@neralabs/rerail-pack-typescript`, `@acme/rerail-pack-sql`, `rerail-pack-lint`). It takes a version, range, dist-tag, or an existing tarball or folder; it refuses npm options, `npm:` aliases, git and URL specs. If a pack it installed is left out, or another stops loading, it puts the change back and fails with why.
- **`remove`** refuses to remove, by name, the package holding an id (the one loading under it, or Rerail's own) while another holder stays. Repositories adding the id would then run the other.

</details>

### Locking, reloads and core ranges

Pack changes run one at a time, a running daemon picks them up without a restart, and each pack names the core versions it fits.

<details>
<summary>Lock, reload and core-range details</summary>

- **Lock.** Set-up's install, `rerail packs add` and `remove` hold `$RERAIL_HOME/packs/.lock`. Others wait up to five minutes; a lock whose process has gone is taken over.
- **Reload.** A running daemon re-reads the catalog at once after an install, removal or active-core change (`rerail update`), and rebuilds every checkout's rules and the dashboard's activity. Edits inside a folder-linked pack are seen only when its `pack.json` or `package.json` changes.
- **Core range.** `peerDependencies["@neralabs/rerail"]` (`>=0.1.0-0 <0.2.0` today) covers the core versions a pack was released for. When `rerail update` activates a core outside an installed pack's range, it names the pack and range. Repositories adding it run none of its rules until a fitting version is installed (`rerail packs add <package>`).

</details>

### Catalog checks

The catalog checks each installed pack on its own. A failing pack is left out and reported (`rerail packs list`, `GET /api/rule-packs` `skipped`); the others load.

A pack must pass:

1. **Package:** `package.json` is a pack's, named for its `pack.json` `id` and installed under that name.
2. **`pack.json`:** valid, and every listed document exists.
3. **Core range:** includes the active Rerail's version.
4. **Compilation:** every code rule compiles.
5. **No clash:** no other loading pack has its pack id or one of its rules.

<details>
<summary>Which packs are read, exact checks and left-out reasons</summary>

The catalog reads only packs the packs folder's `package.json` lists as its own dependencies. A pack npm installed only as another package's dependency is never read. It is reported as left out, saying it is not in that `package.json`, with `rerail packs add <package>` as the fix. If that `package.json` is missing or unreadable, every pack on disk is reported that way, with why, not as no packs.

1. **Package.** `package.json` matches `rulePackPackageSchema`, named `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>`. So `@neralabs/rerail-pack-foo` in a `rerail-pack-foo` folder is left out (`it is installed as rerail-pack-foo, but its package.json names @neralabs/rerail-pack-foo`): no package borrows Rerail's publisher.
2. **`pack.json`.** Valid (`rulePackFileSchema`, `match` included); every listed document carries the listed id.
3. **Core range.** Prereleases included.
4. **Compilation.** One refused only as `language-not-supported` loads, inactive until the engine reads its language; any other refusal leaves the pack out.
5. **No clash.** Rerail's own wins (`pack id sql is taken by @neralabs/rerail-pack-sql`); between third parties, the first by package name. Install location never decides. While Rerail's own pack for an id is installed but left out (by its core range, say), the other is left out too (`pack id sql is kept for @neralabs/rerail-pack-sql, which is installed but left out`). Repositories adding the id run nothing until Rerail's loads.

Reasons are written for the person reading set-up, `rerail packs list` or `rerail update`:

- **Core too old:** the pack needs a newer Rerail, which `rerail update` installs.
- **Rule refused:** this Rerail cannot run it, followed by the engine's own words. Published packs passed their release core's engine, so this almost always means the local Rerail is older than the pack.

</details>

## Third parties' packs

Anyone can publish a pack to npm; Rerail loads it like its own but has not reviewed it.

Install it with `rerail packs add <package>`, or from its tarball or folder. Its rules run in Log until a person approves this version with `rerail rules approve <package>` ([approval](rule-scoping.md#approval)).

<details>
<summary>Naming, publisher and how each surface marks it</summary>

It must be named `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` (`<id>` from `pack.json`) and pass the [checks above](#catalog-checks). Its rules run in every repository that adds it, and their steer text reaches the coding agent.

**Publisher** is `rerail` for `@neralabs/*`, else the npm scope (`@acme`), else the package name (`rerail-pack-lint`). A pack installed from a folder or tarball (`rerail packs add <folder>`, recorded by npm as `file:<path>`) counts as unpublished and not reviewed, whatever its `package.json` says, `@neralabs/*` included. It still loads, and clashes, by its folder's package name.

| Surface | Third party's pack | From a folder or tarball |
|---|---|---|
| `rerail packs list` | package name; `waiting for approval` | adds `(from the folder <path>)` |
| `rerail packs add` | warns once per pack (below) | `… was installed from the folder <path>, not from npm, and Rerail has not reviewed it.` |
| `GET /api/rule-packs` | `approval` | `installedFrom: "folder"` (or `"tarball"`) |
| Dashboard | `By @acme · not reviewed` | `From a folder · not reviewed` (third party: `By @acme · From a folder · not reviewed`), never `By Rerail` |
| Set-up table | `Acme SQL (by @acme)` | `SQL (from a folder)` |
| Steer note | `From the Acme SQL rule pack by @acme, which Rerail has not reviewed.` | `From the SQL rule pack, installed from a folder, which Rerail has not reviewed.` |

</details>

<details>
<summary>Dashboard, warnings, approval, clashes and updates</summary>

- **Dashboard.** It names the publisher wherever it names such a pack (a rule's origin, History, the rules of a pack not added). A Rerail-like scope (`@rerail`) is still a third party's. A rule whose pack is gone is `From a rule pack`, with no publisher.
- **Warning.** `rerail packs add`, and set-up before its question, say Rerail has not reviewed the pack, that its rules run in Log until a person approves this version, and that their steer text then reaches the coding agent.
- **Approval.** Approve with `rerail rules approve <package>` or Approve on the pack's dashboard sheet. Each new version waits again. Rerail's own packs from npm need none.
- **Clashes.** Pack and rule ids are not namespaced; repositories add packs by id. A third party's pack sharing a pack id or rule id with Rerail's is left out with why, and `rerail packs add` puts it back and fails.
- **Set-up and updates.** Set-up fetches only Rerail's packs, but offers an installed third party's pack wherever it fits; `--packs` names it by package name (`@acme/rerail-pack-sql@1.0.0`). `rerail update` updates no pack; `rerail packs add <package>` does.

</details>

## A worktree's own rules

A checkout can carry its own rules in `<checkout>/.rerail/rules/`, with no pack, install or attachment.

Each `.json` rule document directly in that folder is one rule. Rerail reads the folder as pack `worktree`, for that checkout only. Each rule runs in Log until a person approves it (`rerail rules approve <id>`, in the checkout).

<details>
<summary>What it reads, settings, edits and file checks</summary>

A rule document is a `rerail.policy/v1` code rule or a conversation rule; no `pack.json` or `package.json` is needed. The pack is present for the whole checkout without being added. No other checkout runs or lists it, including the main checkout and linked worktrees. It runs only while the checkout is started and its repository set up ([rule scoping](rule-scoping.md#start-and-stop)).

- **Rules.** Each starts on, in its document's mode. Title: its `description`, else its id. Group: "This worktree". Settings name pack `worktree` and its rules by id: a checkout setting on the pack turns the folder off in that worktree; a repository setting applies to every checkout's own rules.
- **Edits.** Adding, removing or changing a file applies at the running session's next event; nothing else needs touching.
- **Checks.** Each file is checked on its own; a failing one is left out, with why. A file must:
  - be a regular file of at most 1 MiB (symbolic links are never followed, nor a symlinked `.rerail` or `rules` folder);
  - hold valid JSON without duplicate keys, and a valid rule document;
  - have an id unique in the folder (the first by file name keeps it) and absent from installed packs (the pack's rule wins);
  - compile, for a code rule, unless refused only as `language-not-supported`.

  A library rule with the same id wins, as over any pack rule. An installed pack with id `worktree` loads instead of the folder.

</details>

<details>
<summary>Where it shows, and trust</summary>

- **Where it shows.**
  - Session footer: `R E R A I L ● 27 rules · worktree` while in force ([in-session surfaces](in-session-surfaces.md#what-the-footer-means)).
  - `rerail packs list`, in the checkout: its rules and left-out files, plus a `Not running:` line while Rerail is off there or the repository is not set up.
  - `rerail rules test <file>`: validates the file and runs the repository pass with no cases.
  - `GET /api/rule-packs?checkout=<path>` (`rule_packs_list`; MCP calls use the calling agent's checkout): the pack with publisher `worktree`, left-out files in `skipped`. `GET /api/repository/rules` for the checkout lists its rules as the pack's.
  - Dashboard, scoped to the checkout: **In this worktree**, never among packs to add, with no Remove pack. Overview names each left-out file with why. Its switches write this checkout's settings, not the repository's.
  - Steer note: `From this worktree's own rules.`
- **Trust.** The folder comes from the working tree, so whatever a branch commits there runs in every started checkout of that branch and reaches the coding agent. Rerail has not reviewed it, so approval is needed again after each edit ([approval](rule-scoping.md#approval)). To keep rules to one worktree, exclude the folder from Git (`.git/info/exclude`).

</details>

## Where a pack's rules live

Rule documents live only in the installed pack; the database holds people's decisions about them, by id.

Because those decisions name packs and rules by id, they apply to whatever version is installed now.

<details>
<summary>What the database holds, and what happens when packs change</summary>

Core reads each installed pack's `pack.json` and listed documents into memory, re-reading them when installed packs change; nothing copies them into the database. The database holds **attachments** (which packs and rules each repository adds), **settings** (`on`, `mode`, `suppressions`, option values in `config`) and **the library** (rules people write, or copy and edit).

| Case | What happens |
|---|---|
| A pack is upgraded | New documents apply once installed; settings and attachments keep applying by id. Added rules start with the pack's `on` and `mode`. Removed rules disappear; settings naming them do nothing. A `config` naming an option the document no longer declares stops that rule, with the reason, as any such `config` does. |
| A pack is downgraded | The same as an upgrade. |
| A pack is uninstalled | It leaves the catalog; repositories adding it report it as unavailable (`missing`). Settings and library copies stay, so reinstalling restores the configuration. |
| Two versions of a pack are installed | Only one can be. Rerail installs into `$RERAIL_HOME/packs` and reads only the `node_modules/rerail-pack-<id>` and `node_modules/@<scope>/rerail-pack-<id>` the packs folder's `package.json` lists, one version each. |
| Two packages hold one pack id | Rerail's own loads, else the first by package name; the other is reported as left out. Repositories add packs by id, so they run the one that loads. While Rerail's own is installed but left out, neither loads. |
| A pack's core range excludes the running core | Skipped and reported; repositories adding it report it as `missing`. |
| A person edits a pack's rule | The rule is copied into the library and added where the original is. The copy keeps the edit through upgrades; the original follows them. Saved in the dashboard, the copy is approved and the original switched off where the copy is added. A coding agent's copy runs in Log, beside the running original, until a person approves it ([copy on edit](rule-scoping.md#coding-agents)). Option values change through `config` settings instead. |
| `rerail rules export` / `import` | `rules.json` carries attachments and settings by id, never pack rules. Without the pack, its attachments are `missing` until it is installed. |

</details>

## Format

A pack is `packs/<id>/pack.json`, in the `rerail.rule-pack/v1` format, beside the rule documents it lists in `packs/<id>/rules/`.

Every `pack.json` field, with an example, is in [pack-format.md, "pack.json"](pack-format.md#packjson); what every pack must prove is its [validation contract](pack-format.md#the-validation-contract).

## Matching

Rerail lists the checkout's files once, reads its dependency manifests, and matches every pack against them. No source text is read.

Each fitting pack comes back with its rule count (rules that start on), reason and the folder where set-up adds it.

<details>
<summary>Which files and manifests count, and how the folder is chosen</summary>

- **Files.** Under Git: `git ls-files --cached --others --exclude-standard`, less files deleted from the worktree and skip-worktree entries a sparse checkout leaves off disk. Outside Git: a walk of at most 200,000 files.
- **Left out.** Dependency and build-output folders (`node_modules`, `dist`, `.venv`, `vendor`, …) and test-data folders (`fixtures`, `__fixtures__`, `testdata`, `test-fixtures`).
- **Manifests.** Remaining `package.json`, `requirements*.txt`, `pyproject.toml` and `composer.json` files, if regular files inside the repository of at most 1 MiB. Symlinked (Git lists them like any tracked file) or larger ones are skipped unread.

A language or framework pack goes in the folder holding all its matches (`web` when only `web/` has TypeScript; the whole repository when matches are spread). A repository pack goes in the whole repository.

</details>

## Cyclomatic complexity

The `complexity` pack flags functions with cyclomatic complexity above 6, reproducing the Metrics-in-the-Loop study's feedback.

A finding names the function and number: `` CYCLOMATIC_COMPLEXITY src/grade.py:1: `grade` has cyclomatic complexity 8, over the ceiling of 6. Decompose it into smaller functions. ``

<details>
<summary>How complexity is counted</summary>

Complexity is one plus a function's decision points, as lizard 1.24.0 counts them minus its tokenizer defects, in Python, TypeScript, JavaScript, PHP, Blade, Rust and Vue. Blade directives and Vue templates run in no function and are never scored; nor are test files.

- **`switch`/`match`** counts once however many cases, as slapkit scores a function. One difference (a default-only switch beside a cased one) is in the pack README.
- **Python** matches lizard for every function lizard reads correctly, checked function by function on jinja, click and marshmallow.
- **SQL** has no rule: no reference scores the SQL Rerail's grammar reads (pack README, "SQL").

</details>

`packs/complexity/README.md` has per-language counting, deliberate differences from lizard, the real-code comparison and the proving corpus.

### Using it like the study

The pack matches the study's C arm: same tool semantics, same limit, only the functions the agent changed.

1. **Add the `complexity` pack.** Set-up offers it for Python, TypeScript, JavaScript, PHP, Blade, Rust or Vue source. Or run `rerail packs add complexity`, then `repository_attachments_set` (`PUT /api/repository/attachments`, body `{ "checkout": "<absolute path of a checkout>", "add": [{ "sourceKind": "pack", "sourceId": "complexity" }] }`, or `"repository": "<repository key>"` in place of `checkout`).
2. **Keep the default `ai-output` audit scope**, which reports only functions the agent changed.

<details>
<summary>Attachment rules, audit scopes and differences from the study</summary>

The attachment covers the whole repository unless the entry names a `folder`. A coding agent may make this call only before set-up; afterwards a person does. Its rules start on.

A finding covers its whole function, so any touched line reports it. Under `touched-files` (`rerail settings --session <session-id> --scope touched-files`), every over-limit function in an edited file is reported, touched or not; `--scope ai-output` restores the default.

Differences from the study:

- **Timing.** Rerail checks after each edit and speaks only when a function is over the limit. The study measured after each turn that changed a file, and messaged even when nothing was over.
- **Content.** Study messages also urged simple, tested code; Rerail sends only the finding.
- **Coverage.** The T and CT arms' coverage feedback is not reproduced: it needs the agent's tests run under coverage each turn, and Rerail runs no tests.

</details>

## API

The daemon serves packs over HTTP; each call is also an MCP tool.

- **`GET /api/rule-packs`** (`rule_packs_list`): installed packs, plus `skipped` ones left out.
- **`GET /api/rule-packs/recommendations`** (`rule_packs_recommend`): installed packs fitting a target checkout.
- **`GET /api/rerail-rules`** (`rerail_rules_list`): each By Rerail rule's `packId`, the pack listing it, or `null`.
- **`PUT /api/repository/attachments`** (`repository_attachments_set`): adds packs to a repository, in a folder or whole. The first addition sets the repository up.

<details>
<summary>Response fields and scan limits</summary>

- **`GET /api/rule-packs`** gives each installed pack's version, `packageName`, `publisher` (`rerail` for Rerail's own, else the npm scope or unscoped package name) and rules. Each `skipped` entry has path, package name, version and why.
- **`GET /api/rule-packs/recommendations`** takes `repository` or `checkout`; MCP defaults to the calling agent's checkout. Each pack comes with rule count (rules that start on), reason and folder. If the checkout is not on disk, the repository's main checkout is scanned, else its most recently seen one. Scans block the daemon, so a checkout is rescanned at most once a minute; installs and removals show at once.

</details>
