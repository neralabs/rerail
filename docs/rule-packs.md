# Rule packs

A rule pack is a named set of rules. Rerail's own packs are **By Rerail**; a pack a third party publishes is **By** its publisher (`By @acme`), and Rerail has not reviewed it ([Third parties' packs](#third-parties-packs)). Rules you write yourself are **Created by you** and belong to no pack. Each pack is its own npm package, `@neralabs/rerail-pack-<id>` for Rerail's own and `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` for a third party's, and Rerail installs the packs you choose into its home (`$RERAIL_HOME/packs`); core itself ships no packs and no rules. A pack is a directory: its rule documents in `rules/`, and a `pack.json` that lists them with the metadata the dashboard shows (a title and a group for each rule), its match definition, and whether its rules start on.

A rule pack runs only where a repository adds it. Setting a repository up installs and adds the packs the person confirms; after that, packs are added and removed, and their rules turned on or off, in the dashboard (for the repository), or added, removed, turned off or given another mode per repository, per folder and per checkout through the API and its MCP tools (see [Rule scoping](rule-scoping.md)).

Rerail recommends packs for a repository by matching the repository's files and dependency manifests against each pack's match definition. A match is data: a pack runs no code to decide whether it fits. Rerail does not read the code for violations to do this, it uses no model, and it says nothing about what no pack covers; the only network it uses is npm, to fetch the packs.

## The packs

Every pack lives under `packs/` in this repository and is published from there ([pack format](pack-format.md)). Core keeps the list of their names in `src/engine/catalog/pack-list.generated.ts` (`npm run generate:packs`); what each pack fits is in the pack's own `pack.json`.

| Pack | Category | Rules (on) | Recommended for |
|---|---|--:|---|
| Cyclomatic complexity (limit 6) | language | 7 (7) | Python, TypeScript, JavaScript, PHP, Blade, Rust or Vue source outside tests. See [Cyclomatic complexity](#cyclomatic-complexity). |
| JavaScript and TypeScript (ESLint core and eslint-plugin-unicorn) | language | 121 (36) | JavaScript or TypeScript source: ESLint's core rules apply to both; its eslint-plugin-unicorn rules run on JavaScript files only. |
| TypeScript | language | 65 (23) | a `typescript` dependency, or `*.ts`/`*.tsx` files |
| Python | language | 537 (318) | Python source outside tests, scripts and docs |
| PHP | language | 15 (5) | PHP source: `*.php` files, not Blade views |
| Blade | language | 10 (8) | Blade templates: `*.blade.php` files |
| Rust | language | 33 (17) | `*.rs` files |
| SQL | language | 21 (19) | `*.sql` files |
| Next.js | framework | 21 (21) | a `next` dependency |
| Node.js | framework | 37 (11) | a Node.js server framework (`express`, `koa`, `fastify`, `@nestjs/core` and others) or `electron` dependency, or `*.cjs`/`*.mjs`/`*.cts`/`*.mts` files outside tests, scripts, fixtures and configuration |
| Vue | framework | 23 (21) | `*.vue` files |
| React | framework | 40 (21) | a `react` dependency, or `*.jsx` files |

Rerail recommends a pack only when at least one of its rules starts on: adding a pack whose rules all start off would run nothing.

A pack for a project's own conventions is made the same way: a `repository` pack directory with its rule documents in `rules/`, a `pack.json` that lists them with a match definition naming the files or dependencies that identify the project, and a `package.json` named `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` with core's range under `peerDependencies`. It is installed from its folder with `rerail packs add <folder>`, and is a third party's pack like any not published by Rerail.

A rule is listed in the mode its own document has (`steer`), unless `pack.json` sets one: a port of a rule upstream only warns about is listed with `"mode": "log"` ([pack format](pack-format.md#packjson)). A ported rule starts on, unless no approved upstream configuration turns it on or upstream deprecates it: such a rule is experimental and is listed with `"on": false`.

A pack's `mode` and `on` are the values its rules start with in a repository that adds it, and the values a rule a pack upgrade adds starts with; a rule or pack setting in the repository or a checkout overrides them.

A rule honours its linter's suppression comments only where its document says so (`suppress`), and only while the repository's `suppressions` setting for it is `honour` ([rule scoping](rule-scoping.md#rules-attachments-and-settings)); the pack's `honourSuppressionComments` is the value its rules start with.

## Installing packs

Packs are installed into one npm prefix in the Rerail home, `$RERAIL_HOME/packs`, never into a project's `node_modules` (one Rerail serves every repository on the machine, and a Python or PHP repository has none) nor into core's own install (`rerail update` replaces it). Installing uses the person's npm login, as installing Rerail does: whoever can read `@neralabs/rerail` can read every pack.

- **Set-up** (`npx @neralabs/rerail setup` in a repository, or `/rerail-start` in one that is not set up):
  - **Fetch.** It fetches every listed pack under the update channel's npm dist-tag: `latest` by default, where each new pack version is published ([pack format](pack-format.md#release)), or `next` for a pack that has no `latest` version. Packs go into a staging folder of its own under `$RERAIL_HOME/packs/staging`, which installs nothing and is removed once the packs are read, and each is checked as the catalog does (below). Fetching reads nothing in the repository; matching the packs against it waits until the person has said Rerail may read it.
  - **Installed versions.** A pack installed at a newer version than the one under the tag (`rerail packs add typescript@next`) is offered at that version, never replaced by an older one. When that newer version does not load (its core range, after setting up an older core again say), set-up names the pack among those it could not use, with why, and does not offer it. An answer that names an older version than the installed one is refused; `rerail packs add` downgrades.
  - **The question.** It lists each pack that fits with its rule count and why it fits, says when a yes replaces an installed version (which every repository on the computer runs), and lists the rules the repository's lint config turns off or configures. A pack that would fit but has no rule that starts on is named as having no rules yet, and "Skip, I'll pick packs" names every such pack.
  - **The answers.** Nothing is installed before an answer that installs, and an answer installs only what it carries.
    - "Yes" installs the versions shown, from npm's cache where discovery left them. An answer run by a coding agent carries them as `--packs typescript@0.1.1,…`.
    - "Change rule packs…" (`--answer choose`) installs only the packs the person picks from every pack set-up fetched, replacing any an earlier dashboard review added. It is not offered when set-up could fetch no pack.
    - "Not now" (`--answer not-now`) installs nothing, and asking again after a bad answer installs nothing.
    - "Skip, I'll pick packs" reads nothing in the repository: it lists every pack set-up fetched, installs the ones the person picks and adds each for the whole repository without its lint config, or none ("None for now", `--packs none`).
  - **After installing.** Set-up makes sure the catalog loads every pack named, and still loads every pack it loaded before. When npm fails or a pack is left out (its core range, a rule another pack holds), every pack the install changed is put back as npm recorded it (a pack added from a folder from that folder) or removed. Set-up then asks again, saying why: no read access, npm not logged in, the registry unreachable, or why the pack was left out. An installed version stands in for a pack npm cannot give.
- **`rerail packs`**:
  - `list` shows the installed packs with their versions (Rerail's own by id, a third party's by package name) and any left out, with why.
  - `add <id[@version]> …` installs one of Rerail's packs by id, or any pack by its package name (`@neralabs/rerail-pack-typescript`, `@acme/rerail-pack-sql`, `rerail-pack-lint`). It takes a version, range or dist-tag, or a tarball or folder that exists, and refuses anything else (npm options, `npm:` aliases, git and URL specs). It is checked as set-up's install is: when a pack it installed is left out, or another stops loading, what changed is put back and the command fails with why.
  - `remove <id | package> …` uninstalls a package by its name, or, given a pack id, every installed package that holds that id. By package name it refuses to remove the package that holds a pack id (the one that loads under it, or Rerail's own) while another installed package holding that id stays, since every repository that adds the id would then run the other.
  - `recommend [--repo <dir>]` runs set-up's discovery and prints what fits, installing nothing.

  npm runs no install script in any of them.

Only one change to the installed packs runs at a time: set-up's install and `rerail packs add` and `remove` hold a lock on the prefix (`$RERAIL_HOME/packs/.lock`), and one that finds it held waits for up to five minutes. A lock whose process has gone is taken over.

A running daemon follows an install or removal, and a change of the active core (`rerail update`), at once: the catalog is read again when they change, and every checkout's rules, and the dashboard's activity, are rebuilt from it. No restart is needed. An edit inside a pack linked from a folder is not seen until that pack's `pack.json` or `package.json` changes.

Each pack's core range (`peerDependencies["@neralabs/rerail"]`, `>=0.1.0-0 <0.2.0` today) covers the core versions it was released for. When `rerail update` makes active a core that an installed pack's range excludes, the command names each such pack with its range: repositories that add it run none of its rules until a version that fits is installed (`rerail packs add <package>`).

The catalog reads only the packs the person installed: those the packs folder's `package.json` lists as its own dependencies. A pack npm put in `node_modules` only as another package's dependency is never read; it is reported as left out, saying it is not in that `package.json` and that `rerail packs add <package>` installs it. When that `package.json` is missing or cannot be read, every pack on disk is reported that way, with why, rather than as no packs at all. Each installed pack is checked on its own when the catalog is read; a pack that fails is left out and reported (`rerail packs list`, `GET /api/rule-packs` `skipped`), and the others load:

1. Its `package.json` is a pack's (`rulePackPackageSchema`), named `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` for the `id` in its `pack.json`, and that name is the one npm installed it under: a package that calls itself `@neralabs/rerail-pack-foo` in a `rerail-pack-foo` folder is left out (`it is installed as rerail-pack-foo, but its package.json names @neralabs/rerail-pack-foo`), so no package borrows Rerail's publisher.
2. Its `pack.json` is valid (`rulePackFileSchema`, a `match` included), and every listed document exists and carries the listed id.
3. Its core range (`peerDependencies["@neralabs/rerail"]`) includes the active Rerail's version, prereleases included.
4. Every code rule compiles in the engine. A rule refused only as `language-not-supported` loads and stays inactive until the engine reads its language; any other refusal leaves the pack out.
5. No other installed pack that loads has its pack id or a rule it lists. Of two that clash, Rerail's own pack loads and the other is left out, naming the pack that holds the id (`pack id sql is taken by @neralabs/rerail-pack-sql`); of two third parties' packs, the first by package name loads. Where npm put each one never decides it. While Rerail's own pack for an id is installed but left out (by its core range, say), no other pack takes the id: the other is left out too (`pack id sql is kept for @neralabs/rerail-pack-sql, which is installed but left out`), and repositories that add the id run nothing until Rerail's pack loads again.

Why a pack is left out is said for the person who sees it in set-up, `rerail packs list` or `rerail update`: a core older than the pack's range says the pack needs a newer Rerail and that `rerail update` installs it, and a rule the engine refuses says this Rerail cannot run it, before the engine's own words. A published pack passed the engine of the core it was released with, so on a person's machine a refused rule almost always means their Rerail is older than the pack.

## Third parties' packs

Anyone can publish a rule pack to npm: a package named `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>`, where `<id>` is the `id` in its `pack.json`, that passes the checks above. Rerail loads one installed with `rerail packs add <package>` (or from its tarball or folder) as it loads its own. Rerail has not reviewed such a pack: its rules run in every repository that adds it, and the text they steer with reaches the coding agent there.

- **Publisher.** A pack's publisher is `rerail` for `@neralabs/*`, else its npm scope (`@acme`), else its package name (`rerail-pack-lint`). Each surface names it:
  - `rerail packs list` names a third party's pack by its package name.
  - The dashboard labels the pack and its rules `By @acme · not reviewed` where Rerail's say `By Rerail`, and names the publisher wherever it names such a pack (a rule's origin, History, the rules of a pack not added). A scope that reads like Rerail's (`@rerail`) is still marked as a third party's.
  - The steer note names it: `From the Acme SQL rule pack by @acme, which Rerail has not reviewed.`
  - A rule whose pack is no longer installed is `From a rule pack`, claiming no publisher.
- **Warning.** `rerail packs add` says, once for each third party's pack it installs, that Rerail has not reviewed it, that its rules run in Log until a person approves this version, and that from then on their steer text reaches the coding agent.
- **Approval.** Its rules run in Log until a person approves what this version ships (`rerail rules approve <package>`, or Approve on its sheet in the dashboard); each new version waits again ([approval](rule-scoping.md#approval)). `rerail packs list` marks it `waiting for approval`, and `GET /api/rule-packs` gives it `approval`. Rerail's own packs from npm need none.
- **Packs from a folder or tarball.** A pack installed from a folder or tarball (`rerail packs add <folder>`, which npm records as `file:<path>`) counts as unpublished and not reviewed, whatever its `package.json` calls it, `@neralabs/*` included. It still loads, and clashes, by the package name its folder gives. Each surface says where it came from:
  - `rerail packs list` adds `(from the folder <path>)`.
  - `rerail packs add` warns as for a third party's pack: `… was installed from the folder <path>, not from npm, and Rerail has not reviewed it.`
  - `GET /api/rule-packs` gives it `installedFrom: "folder"` (or `"tarball"`).
  - The dashboard labels it `From a folder · not reviewed` (a third party's: `By @acme · From a folder · not reviewed`), never `By Rerail`.
  - Set-up's table names it `SQL (from a folder)`, with the same warning before the question.
  - Its steer note says `From the SQL rule pack, installed from a folder, which Rerail has not reviewed.`
- **Clashes.** Pack ids and rule ids are not namespaced: a repository adds a pack by its id. When a third party's pack has the pack id of one of Rerail's, or a rule id one of Rerail's packs has, Rerail's loads and the other is left out with why; `rerail packs add` puts such a pack back and fails.
- **Set-up and updates.** Set-up fetches only Rerail's own packs. A third party's pack that is installed already is offered too, wherever it fits, named with its publisher in the table (`Acme SQL (by @acme)`) and with a line before the question saying Rerail has not reviewed it, that its rules run in Log until the person approves them, and that then their steer text reaches the coding agent; `--packs` names it by its package name (`@acme/rerail-pack-sql@1.0.0`). `rerail update` updates no pack; `rerail packs add <package>` does.

## A worktree's own rules

A checkout can carry rules of its own: every `.json` file directly in `<checkout>/.rerail/rules/` that holds a rule document (a `rerail.policy/v1` code rule, or a conversation rule) is one rule. The folder needs no `pack.json` or `package.json`. Rerail reads it as one pack, `worktree`, that only that checkout's catalog holds, and the pack is present there for the whole checkout without being added: no attachment, no install, and no other checkout of the repository, its main checkout and linked worktrees included, runs or lists it. It runs where the checkout's other rules run, so only while the checkout is started and its repository set up ([rule scoping](rule-scoping.md#start-and-stop)).

- **Rules.** Each rule starts on, in its document's mode; its title is its `description`, else its id, and its group is "This worktree". Settings name the pack and its rules by id as for any pack: a checkout setting on pack `worktree` turns the whole folder off in that worktree, and a repository setting on it applies to every checkout's own rules.
- **Edits.** The folder is read again whenever a file in it is added, removed or changed, so an edit applies at the next event of a running session, without touching anything else.
- **Checks.** Each file is checked on its own: one that fails is left out, with why, and the others load. A file must:
  - be a regular file of at most 1 MiB (a symbolic link is never followed, nor a symlinked `.rerail` or `rules` folder);
  - hold valid JSON without duplicate keys, and a valid rule document;
  - have an id that no other file of the folder has (the first by file name keeps it) and no installed pack has (the pack's rule wins);
  - compile, for a code rule, unless the engine refuses it only as `language-not-supported`.

  A library rule with the same id wins over it, as over any pack rule. When an installed pack has the pack id `worktree`, that pack loads and the folder does not.
- **Where it shows.**
  - The session footer reads `R E R A I L ● 27 rules · worktree` while they are in force ([in-session surfaces](in-session-surfaces.md#what-the-footer-means)).
  - `rerail packs list`, run in the checkout, lists its own rules and the files left out, with a `Not running:` line while Rerail is off there or the repository is not set up.
  - `rerail rules test <file>` checks one of its files as it is: it validates the file and runs the repository pass with no cases.
  - `GET /api/rule-packs?checkout=<path>` (`rule_packs_list`; an MCP call is scoped to the calling agent's checkout) lists the pack with publisher `worktree`, and the left-out files in `skipped`. `GET /api/repository/rules` for the checkout lists its rules as the pack's.
  - The dashboard, scoped to the checkout, shows the pack as **In this worktree**, never in the catalog of packs to add and with no Remove pack. Its Overview names each left-out file with why. The switches for the pack and its rules write this checkout's settings, where every other pack's write the repository's.
  - A steer from one of its rules says `From this worktree's own rules.`
- **Trust.** The folder is read from the working tree, so whatever a branch commits there runs in every checkout of that branch that is started, and its text reaches the coding agent there. Rerail has not reviewed it, as it has not reviewed a third party's pack, so each rule runs in Log until a person approves its content (`rerail rules approve <id>`, run in the checkout), and again after each edit ([approval](rule-scoping.md#approval)). Keep the folder out of Git (`.git/info/exclude`) to keep rules to one worktree.

## Where a pack's rules live

An installed pack's files are its rules. Core reads each installed pack's `pack.json` and the rule documents it lists,
keeps them in memory, and reads them again when the installed packs change; nothing copies them into the database. The
database holds what people decide about them: which packs and rules each repository adds (attachments), the `on`,
`mode`, `suppressions` and option values (`config`) set for them (settings), and the rules people write or copy and
edit (the library). All of these name a pack or rule by its id, so each case below is "whatever is installed now":

| Case | What happens |
|---|---|
| A pack is upgraded | Its new documents apply as soon as it is installed. Settings and attachments keep applying by id. A rule the upgrade adds starts with the pack's `on` and `mode`; a rule it removes disappears, and the settings that name it do nothing. A `config` naming an option the new document no longer declares stops that rule, with the reason, as any such `config` does. |
| A pack is downgraded | The same as an upgrade. |
| A pack is uninstalled | It leaves the catalog. Repositories that add it report it as unavailable (`missing`); their settings and library copies stay, and installing the pack again restores the same configuration. |
| Two versions of a pack are installed | Only one can be: Rerail installs packs into one place (`$RERAIL_HOME/packs`) and reads only `node_modules/rerail-pack-<id>` and `node_modules/@<scope>/rerail-pack-<id>` there that the packs folder's `package.json` lists, one version of each package. |
| Two packages hold one pack id | One loads: Rerail's own, else the first by package name. The other is left out and reported; repositories add packs by id, so they run the one that loads. While Rerail's own is installed but left out, neither loads. |
| A pack's core range excludes the running core | The pack is skipped and reported, and repositories that add it report it as `missing`. |
| A person edits a pack's rule | The rule is copied into the library and added where the original is; the copy keeps the edit through later upgrades, and the original follows them. Saved in the dashboard, the copy is approved as it is saved and the original switched off where the copy is added. A coding agent's copy runs in Log, and the original keeps running, until a person approves the copy ([copy on edit](rule-scoping.md#coding-agents)). Option values are changed with `config` settings instead. |
| `rerail rules export` / `import` | `rules.json` carries attachments and settings by id, never pack rules. On a machine without the pack, its attachments are `missing` until it is installed. |

## Format

A pack is `packs/<id>/pack.json`, in the `rerail.rule-pack/v1` format, beside the rule documents it lists in
`packs/<id>/rules/`. Every field of `pack.json`, with an example, is in [pack-format.md, "pack.json"](pack-format.md#packjson);
what every pack must prove is its [validation contract](pack-format.md#the-validation-contract).

## Matching

Rerail lists the checkout's files once and matches every pack against that list:
- Under Git it uses `git ls-files --cached --others --exclude-standard`, less files deleted from the worktree and files a sparse checkout leaves off disk (skip-worktree entries). Outside Git it walks at most 200,000 files.
- Dependency and build-output folders (`node_modules`, `dist`, `.venv`, `vendor`, …) and test-data folders (`fixtures`, `__fixtures__`, `testdata`, `test-fixtures`) are left out.

It then reads every dependency manifest (`package.json`, `requirements*.txt`, `pyproject.toml`, `composer.json`) that is left and is a regular file inside the repository of at most 1 MiB; a symlinked manifest (which Git lists like any tracked file) or a larger one is skipped unread. It returns each fitting pack with its rule count (the rules that start on), reason and folder. No source text is read.

The folder is where set-up adds the pack. A language or framework pack goes in the folder that holds everything it matched (`web` when only `web/` has TypeScript; the whole repository when matches are spread across it). A repository pack goes in the whole repository.

## Cyclomatic complexity

The complexity pack flags a function whose cyclomatic complexity is above 6: one plus the decision points inside it, with lizard 1.24.0's decision points and without its tokenizer defects, in Python, TypeScript, JavaScript, PHP, Blade, Rust and Vue (a Blade template's own directives and a Vue template run in no function and are never scored). SQL has no rule: no reference scores the SQL Rerail's grammar reads (pack README, "SQL"). A `switch` or `match` counts once however many cases it has, as slapkit scores a function (one difference, a default-only switch beside a cased one, is in the pack README). On Python it gives lizard's number for every function lizard reads correctly, checked function by function on jinja, click and marshmallow. Each finding names the function and the number, for example `` CYCLOMATIC_COMPLEXITY src/grade.py:1: `grade` has cyclomatic complexity 8, over the ceiling of 6. Decompose it into smaller functions. `` What counts in each language, where it deliberately differs from lizard, the real-code comparison and the corpus that proves the rest are in `packs/complexity/README.md`. Test files are not scored.

It reproduces the complexity feedback of the Metrics-in-the-Loop study (the C arm): the same tool semantics, the same limit, and only the functions the agent changed. To use it:

1. Add the `complexity` pack to the repository: set-up offers it for Python, TypeScript, JavaScript, PHP, Blade, Rust or Vue source, or install it with `rerail packs add complexity` and add it with `repository_attachments_set` (`PUT /api/repository/attachments`, body `{ "checkout": "<absolute path of a checkout>", "add": [{ "sourceKind": "pack", "sourceId": "complexity" }] }`, or `"repository": "<repository key>"` in place of `checkout`; the whole repository unless the entry names a `folder`). A coding agent may make this call only before the repository is set up; after that a person makes it. Once added, its rules are on.
2. Report only the functions the agent changed: this is the default `ai-output` audit scope. A finding covers its whole function, so a function is reported when an edit touches any of its lines. A workspace set to `touched-files` (`rerail settings --session <session-id> --scope touched-files`) is told about every function over the limit in an edited file, including ones the agent never touched; `--scope ai-output` sets it back.

It differs from the study in when and how often the agent hears about it:

- Rerail checks after each edit and speaks only when a function is over the limit. The study measured once after each turn that changed a file, and sent a message even when nothing was over the limit.
- The study's every message also carried a reminder to keep code simple and tested; Rerail sends only the finding.
- The study's coverage feedback (the T and CT arms) is not reproduced: it needs the agent's own tests run under coverage after every turn, and Rerail does not run tests.

## API

The daemon serves the packs, and each call is also an MCP tool:
- `GET /api/rule-packs` (`rule_packs_list`): every installed pack with its version, `packageName`, `publisher` (`rerail` for Rerail's own, else the npm scope or the unscoped package name) and rules, and `skipped`: each installed pack left out, with its path, package name, version and why.
- `GET /api/rule-packs/recommendations` (`rule_packs_recommend`): the installed packs that fit a target checkout (`repository` or `checkout`; an MCP call defaults to the calling agent's checkout), each with its rule count (the rules that start on), reason and folder. When the checkout is not on disk the repository's main checkout is scanned, else its most recently seen one. The scan blocks the daemon while it runs, so a checkout is rescanned at most once a minute; a pack installed or removed shows at once.
- `GET /api/rerail-rules` (`rerail_rules_list`) gives each By Rerail rule its `packId`: the pack that lists it, or `null`.

Packs are added to a repository with `PUT /api/repository/attachments` (`repository_attachments_set`), in a folder or for the whole repository; the first addition sets the repository up.
