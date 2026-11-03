# Rule Scoping

Rerail keeps rule configuration and execution history in one user-level SQLite database, `$RERAIL_HOME/rerail.sqlite` (`~/.rerail/rerail.sqlite` when `RERAIL_HOME` is unset). A **repository** owns its rule configuration; each **checkout** of it decides whether Rerail is on there and may change rules for itself. Sessions, events, work links and audits are attributed to the repository of the checkout that recorded them.

## Repositories and checkouts

A **repository** is keyed by the normalised address of its `origin` remote (or its only remote): `remote:<host>/<path>`, lower-case, without scheme, user, port, trailing `/` or `.git`, so the https, ssh and scp forms of one address give one key. A clone without a remote is keyed `clone:<clone id>`, the id Rerail keeps in `rerail-repository-id` in Git's common directory (a folder outside Git keeps one in `.rerail/repository-id`). A repository also has **aliases**: other keys that resolve to it, such as a rename, an ssh host alias or a mirror. A person links one with `repository_link`; an address that already belongs to another set-up repository is refused.

A **checkout** is one working copy: a clone's main checkout or a linked worktree, recorded once under its physical (symlink-resolved) Git top level. When Rerail first records a checkout it joins the repository of another recorded checkout of the same clone, else the repository its remote names (by key or alias), else a new repository. A clone-keyed repository whose checkout later has a remote is adopted into the remote's repository: when that one is not set up the two merge and the old key becomes an alias; when it is set up only the checkouts move. Checkouts whose folder is gone or that Git no longer lists are forgotten with their own settings.

Two clones of one remote are one repository and share its configuration. A fork has its own remote and is its own repository.

## Start and stop

Rerail is off until it is started. Each repository and each checkout has a start value; a checkout's own value wins, else the nearest recorded checkout of the same repository that contains it and has one, else the repository's. The state of a checkout is one of:

- **started**: rules run;
- **stopped**: nothing is recorded, evaluated or delivered, and hooks don't contact the daemon;
- **not-set-up**: started, but the repository has no configuration yet, so sessions are recorded and no rule runs.

`/rerail-start` and `/rerail-stop` (or `rerail start` and `rerail stop`) act on this checkout and the checkouts nested in it, which follow it again. `rerail start --repository` and `rerail stop --repository` act on every checkout of the repository, clearing their own values. Stopping keeps rules and history.

## Set-up

A repository is **Not set up** until its first configuration. Starting Rerail in a repository that is not set up runs set-up there: it asks before reading the repository, recommends rule packs with the reason for each, and asks you to confirm. "Start" installs the recommended packs ([Installing packs](rule-packs.md#installing-packs)), sets the repository up with them, each attached at the folder the recommendation names, and starts it: a language or framework pack at the folder holding everything that matched it (for example `web` when only `web/` has TypeScript), a repository pack for the whole repository. "Not now" (`--answer not-now`) installs, connects and turns on nothing. "Skip, I'll pick packs" at the question before it (`--answer pick-myself`) reads no code: it lists every rule pack set-up fetched, installs the ones you pick and attaches each for the whole repository, taking nothing from the lint config; "None for now" (`--packs none`) sets the repository up with no packs. A repository where an earlier version of set-up offered the dashboard's review, and nothing was changed there, goes straight to the confirm question. A repository set up any way (in the dashboard, `rerail rules add` or `/rerail-create`, a coding agent adding rules or packs, `rerail rules import`) is set up, so starting it just starts it with what is attached, asking nothing. Set-up run again in a repository that is set up never asks again: it shows the summary, or says Rerail is stopped there. Set-up never adds rules created by you; you add them yourself.

A repository that is set up runs only what is attached to it, and each checkout also runs its own rules, the rule documents in its `.rerail/rules/` folder ([a worktree's own rules](rule-packs.md#a-worktrees-own-rules)). The rules of the installed rule packs run only in repositories that add them. Nothing else runs: no other rule file in the working tree is read.

### The repository's own lint config

So Rerail does not steer against what a repository deliberately configured, set-up reads its lint config at the repository root, as data, and gives the rules of the packs it adds matching settings, owned by the repository. Nothing is executed or imported.

| Linter | Read | Not read |
|---|---|---|
| ESLint | the config ESLint itself would use: the first flat config (`eslint.config.{js,mjs,cjs,ts,mts,cts}`), parsed and never run, when it is data throughout; else `.eslintrc.json`, or `.eslintrc` when it is JSON, or else `package.json`'s `eslintConfig`, JSON with comments and trailing commas, its own `rules` and its `overrides`' `rules` | a flat config that runs code (a function, a statement that changes something, a namespace, a loop), and in a flat config a value Rerail would have to run code for (an imported config such as `js.configs.recommended`, a call's result, an imported variable); `.eslintrc.{js,cjs,yaml,yml}`; `extends` and plugins' presets of an `.eslintrc` |
| oxlint | `.oxlintrc.json`, top-level `rules`; each key resolved to a built-in rule as oxlint 1.88.0 resolves it, and a rule's last key applies | `extends`, `overrides`, `categories`, `plugins` (a key for a plugin the config leaves off still counts); rule options |
| Ruff | `select`, `extend-select`, `ignore`, `extend-ignore`, `per-file-ignores` and `extend-per-file-ignores` (each from `[lint]` where it is set there, else the top level) of the file Ruff uses (`.ruff.toml`, then `ruff.toml`, then `pyproject.toml`'s `[tool.ruff]`/`[tool.ruff.lint]`) and of the configs its `extend` names inside the repository | a config `extend` names outside the repository; per-file ignores of an extended config in another folder; Bandit config |

A flat config is read only when it is data throughout: imports, top-level `const`s, the export and type-only declarations, built from literals, arrays, objects without methods, spreads and member reads, with calls only into the packages the file imports or requires (not a module it names by path, which can be the config itself, nor a Node module that runs code, such as `node:vm`), `require("<package>")`, `new URL(…)`, `process.cwd()` and `import.meta.resolve(…)`, and none of the file's own `const`s handed to a call (except `defineConfig(...)` and typescript-eslint's `config(...)`, which Rerail takes to do what their documentation says, as it takes the packages the file imports). A key computed from anything but a literal, a `constructor`, `prototype` or `__proto__` key, and an object key JavaScript calls by itself (`toString`, `valueOf`, `toJSON`, `then`), an export named `then` and `export * from` a module named by path are code too, as is any global the file names other than in those calls, besides `process.env`, `__dirname`, `__filename`, `undefined`, `NaN` and `Infinity` (`require.cache`, `process.mainModule` and `globalThis` reach the config itself), a value of a module named by path handed to a call (it may be the config itself), and an exported object that may hold a `default` key (ESLint's loader may take that instead). A type-only import is erased, so its names are the globals. A file with anything else, a value of its own Rerail cannot follow (a choice between local configs, an exported `const`), a file Rerail cannot load (a link, or one over 1 MiB), or one that takes more than 100,000 steps or builds more than 100,000 values to read (a `const` named twice in the next, level after level, doubles both) is not statically readable: Rerail cannot tell which rules it turns off, so every rule Rerail ports from ESLint starts off, with no options, and the screen says why first. Only a value from an imported module (a shared config, or a choice between one and nothing) is named as not read and read around. A config that is read, is read as ESLint applies it: its exported array (through `export default`, `module.exports`, same-file `const`s, spreads, `defineConfig(...)`, `config(...)` and `extends`), one config object after another, a later one replacing a rule's entry (and keeping its options when it sets only a severity, `"warn"`, `["warn"]` or `"off"`, for every file and for the files an earlier entry limited to some files set them for); within one `rules` object a later key replaces an earlier one or a spread whole, as JavaScript does. An entry in an object with `files`, `ignores` or `basePath` applies to some files only: its options become the port's options for the whole repository only when no entry for every file sets the rule, and the options of an entry for every file become them only when no entry for some files sets others (there they would let through what ESLint reports); it turns a rule off only in a folder it covers whole (`files: ["scripts/**"]` turns it off in `scripts`), and an "off" for narrower files, or one a later object turns back on, leaves the rule on with a note. A rule entry whose value Rerail cannot read without running the config turns the port off, and the screen says why; the shared configs it does not read are named, and an entry with one of them after it is not used, since it may replace the entry; nor are options one of them may have set. An `.eslintrc`'s `overrides` are read the same way, after its own `rules`; an off an override's `extends` or nested `overrides` may replace is not used, nor is an `.oxlintrc.json` rule its `overrides` may replace.

Ruff's selection is resolved as Ruff resolves it: its selectors apply from the least specific (`ALL`, then `E`, `E7`, ...) to the most specific (`E711`), a select before an ignore of the same specificity, so the most specific selector that names a code decides. The configs `extend` names come first, and a later config's `select` replaces what they decided. A rule starts off only when an ignore decides it; when a more specific or later select re-enables it, it stays on and the screen says so. A per-file ignore turns a rule off in a folder its pattern covers whole (`"tests/**"`); for a narrower pattern (`"**/test_*.py"`) the rule stays on with a note.

A port matches an upstream rule through the names its document honours suppression comments for (`violation.suppress`): an ESLint key in `eslint.rules`; an oxlint key that names the same built-in oxlint rule as the port's ESLint rule (`no-console`, `eslint/no-console`; for typescript-eslint `typescript/x`, `@typescript-eslint/x` or a bare `x` unless another plugin's `x` comes first in oxlint; `react-hooks/x` is oxlint's `react/x`; eslint-plugin-n's `n/x` is oxlint's `node/x`), so `custom/no-console` (another plugin's rule) changes nothing; a Ruff selector that selects one of its `noqa.codes` (`E7` selects `E711`, `E` does not select `ERA001`, `ALL` selects every code). A rule set to `"off"`/`0` (oxlint also `"allow"`; an `.eslintrc` or `eslintConfig` severity in any case, `"OFF"`, as ESLint's eslintrc loader lower-cases it, and a flat config's in lower case only, as ESLint 9 refuses `"OFF"` there) for every file, or ignored by Ruff, starts off; when one config turns a rule off and another keeps it on, it starts off and the screen says so. ESLint options become the port's `config` when every value lands on an option its document declares: the first positional option is `option`, the second `option2`, an option object's keys keep their names and a nested object's keys are `outer.inner`, so `["error", "type"]` for `@typescript-eslint/consistent-type-definitions` gives `option = "type"`. Options that do not map, or that the document rejects, are left at the port's defaults with a note. Rules are never turned on, a file larger than 1 MiB or a symlink is not read, nor is a TOML file whose arrays or inline tables nest more than 64 deep, and a rule the person already set in the dashboard's review (in that folder) keeps that setting. "Skip, I’ll pick packs" takes nothing from the lint config, even for the packs the person then picks.

## Rules: attachments and settings

Rules come from three places:

- **By Rerail**: the rules of the rule packs installed from npm (`$RERAIL_HOME/packs`). They are read from the installed packs, never copied into the database. A third party's pack is installed and read the same way; the dashboard labels its rules **By** its publisher (`By @acme`) instead ([third parties' packs](rule-packs.md#third-parties-packs)), and everything below applies to them as to Rerail's.
- **Created by you**: the library, rules people wrote, stored once per machine. A library rule can't take a By Rerail rule's id, and one id keeps one kind (`code` or `conversation`). If an installed pack later ships a rule with a library rule's id, the library rule wins and the By Rerail rule is reported as unavailable.
- **In this worktree**: the rule documents in one checkout's `.rerail/rules/`, read from its working tree as the pack `worktree`, present for that whole checkout without an attachment and in no other checkout. Settings apply to them as to any pack's rules ([a worktree's own rules](rule-packs.md#a-worktrees-own-rules)).

A **rule attachment** adds a rule pack, a By Rerail rule or a library rule to a repository, in a folder (`""` is the whole repository; folders are relative to the repository root, POSIX, without `.` or `..`). A rule that is in an attached pack belongs to that pack even if it is also attached on its own. An attachment whose target no longer exists is reported as unavailable, never thrown.

A **rule setting** changes, for one rule or one whole pack in a folder, any of:

- `on`;
- `mode` (Log, Steer or Block; Block is in [internal testing](../README.md#internal-testing) and is delivered like Steer);
- `suppressions`: `honour` or `ignore` linters' suppression comments (`// eslint-disable-next-line`, `# noqa`, `# nosec`, ...). A ported rule reads the comments of the linter it ports, where its document's `suppress` names them. Under `ignore`, no linter's comment silences it;
- `config`: the option values of one code rule by Rerail, by option name. They replace the defaults its document's `define.options` declare, and the document is resolved again with them. `config` is set on a rule (not a pack) for the whole repository (folder `""`), because one compiled rule runs in every folder of a checkout. Your library rules take no `config`: edit the rule instead. Each option must be one the document declares, with a value its declaration allows; anything else is refused with `400 invalid-setting`.

A setting belongs to the repository (the default for every checkout) or to one checkout (this worktree only). A null field leaves it to the next owner.

The `suppressions` setting governs only linters' comments. Rerail's own comment, `rerail-ignore`, is always honoured, whatever the setting: `// rerail-ignore complexity-cyclomatic-typescript -- generated` (or `#`, `/* */`, `{{-- --}}` per language) silences the code rules it names by id at the node it covers: the whole file when it comes before the file's first code, the next node when it stands on its own line, and the code it trails on its line otherwise. Every code rule answers to it, with nothing in its document; `docs/code-rule-dsl-reference.html#rerail-ignore` has the exact rules.

**Rule Path Criteria** (`scope.include`, `scope.exclude` on a code rule, excludes winning) still narrow the paths a present rule evaluates. Execution `scope` and run mode still decide when it evaluates.

## Precedence

For a file at path `p`, the rules present are those attached in a folder that contains `p` (`web` contains `web/x`, not `web2/x`). Each field (`on`, `mode`, `suppressions`, and each option in `config`) of each rule is then decided separately, first value wins:

1. by owner: this checkout, then the checkouts that contain it (nearest first), then the repository;
2. within one owner, the deepest folder that contains `p`;
3. within one folder, the rule's own setting before its pack's;
4. without a setting, the value the pack starts the rule with, then the rule's own default (on, in its document's mode). For `suppressions` the pack's start is its `honourSuppressionComments` (`honour` unless the pack sets it to false); a rule outside a pack honours them. Without a `config` value, an option keeps its document's value.

So `config` merges per option: a checkout that sets `{ "option": "type" }` keeps the other options the repository sets.

Every effective rule reports what decided each field: `repository_rules_get` returns it as `onSource`, `modeSource`, `suppressionsSource`, and `configSource`, which names the setting that decided each option (this checkout's or a containing checkout's setting, the repository's setting, the pack's starting value, or the rule's default). If a stored `config` no longer applies to the rule's document (a later release removed the option), `configError` says why, and the rule runs nowhere: it is quarantined with that reason.

Conversation rules, which are in [internal testing](../README.md#internal-testing), are resolved the same way for the folder the session started in.

## Approval

A rule Rerail did not publish runs in Log, whatever its settings say, until a person approves its content: a rule created by you (the library), a rule in a worktree's `.rerail/rules/`, and every rule of a pack that is not Rerail's own from npm (a third party's pack, or any pack installed from a folder or tarball). Rerail's own published packs need none. Its mode then comes from no setting: `modeSource` is `{ "by": "awaiting-approval" }`, and the rule's `approval` gives the `subject` to approve (`kind` `rule` or `pack`, `id`, `digest`) and `approved: false`.

An approval is of content, on this machine: a rule by its id and the digest of its document, a pack by its package name and the digest of its version, what it says of itself (`name`, `category`, `covers`, `reason`), `honourSuppressionComments` and every rule document it lists. An edit, or a new version of a pack, is new content and waits again; going back to approved content needs no second approval. A library rule a person writes through the dashboard or HTTP API is approved with the write; one a coding agent writes, or `rerail rules add` adds, waits. Library rules stored before approvals existed count as approved.

A person approves with `rerail rules approve <id | package>` (no name lists what waits), the Approve button on the rule's page or the pack's sheet in the dashboard, or `rule_approvals_set`. `rerail rules approve` prints what it approves first: a rule's JSON in full with invisible characters written as escapes, or a pack's rule ids; Rerail's command files never pre-approve it, so a coding agent's host asks the person before it runs. The session footer counts what waits (`2 to approve`), as does `/rerail`, and Overview lists it. An approval changes the configuration signature, so a running session picks it up at its next event.

`rule_approvals_set` refuses a coding agent, but `rerail rules approve` cannot tell who runs it: a coding agent with a shell can run it as well as a person can. That the agent runs it only after the person says yes is the `/rerail-create` skill's instruction, not something Rerail enforces, as with the rest of the line between people and coding agents ([coding agents](#coding-agents)). Approval keeps a rule a person never read from steering by default; it does not stop an agent set on approving one.

## Which checkout judges a file

Each file is judged by the checkout that owns it, with that checkout's configuration. The owner is the nearest folder above the file that holds a `.git` entry Git confirms as a checkout's top level, found from both the file's lexical and real paths (a symlink into another checkout or out of its owner is not evaluated). A nested checkout owns its files when it is a submodule the checkout around it declares, or a checkout Rerail has recorded (such as a worktree kept inside the main checkout); while it is stopped or not set up it runs nothing, and the checkout around it does not judge its files either. Only a nested repository Rerail never recorded (an untracked `git init`, a stray `.git`) is judged with the checkout around it. Only checkouts that are recorded, started and set up evaluate files; a session that starts in a stopped or unrecorded checkout judges nothing.

## Coding agents

The HTTP API (`/api`) is for people; MCP callers (`/mcp`) are coding agents. The distinction is advisory, not a security boundary. A coding agent may:

- start, stop and change settings in its own checkout and the checkouts inside it, at any time;
- change the repository's attachments and settings only while it is not set up;
- add a rule it creates in the same call, or the copy of a By Rerail rule it edits, to a set-up repository.

Anything else is refused with `403 agent-not-permitted` and the dashboard URL where a person can make the change. Only a person links repository addresses, starts and stops a whole repository, or approves a rule's content ([approval](#approval)); a rule a coding agent writes runs in Log until then.

**Copy on edit.** Writing a By Rerail rule's id through `library_rules_upsert` stores an edited copy in the library (`<id>-copy`, `-copy-2`, …) instead. The copy is attached wherever the original is present in the target repository, at the same folders. Its first approval switches the original off there with a repository setting: at once for a person's write, which is approved as it is saved, and for a coding agent's copy when a person approves it. Until then the original keeps running beside the copy, which runs in Log, so writing a copy never weakens a rule. The original stays a reference to its installed pack.

## Management surfaces

The same operation definitions drive HTTP/OpenAPI and MCP. Every operation names its target in the query or body, never the path: `repository` (key or alias), `checkout` (an absolute path in a checkout) and `folder`. A checkout wins, then the repository, then the calling agent's checkout; a repository that does not match the checkout is `target-mismatch`. Reads never record a checkout, and writes default to the whole repository.

| Operation | Route | Purpose |
|---|---|---|
| `repositories_list` | `GET /api/repositories` | Every repository: key, name, aliases, set up, started, checkout count. |
| `repository_context_get` | `GET /api/repository` | One repository with its clones and checkouts (own and effective state), and the caller's checkout and folder. |
| `repository_rules_get` | `GET /api/repository/rules` | Effective rules with on, mode, suppressions and config, what decided each, and unavailable references. |
| `repository_settings_set` | `PUT /api/repository/settings` | Owner `repository` or `checkout`, folder, targets (rule or pack), `on`, `mode`, `suppressions` and `config` (null clears; omitted keeps). |
| `repository_attachments_set` | `PUT /api/repository/attachments` | Add or remove packs and rules per folder, with optional repository settings. The first addition, or `configure: true`, sets the repository up. |
| `repository_start`, `repository_stop` | `POST /api/repository/start`, `/stop` | Scope `checkout` (with the checkouts inside it) or `repository`. |
| `repository_link` | `POST /api/repository/aliases` | Make another remote address resolve to this repository. People only. |
| `rule_packs_recommend` | `GET /api/rule-packs/recommendations` | The packs that fit a checkout, each with the folder to add it in. |
| `coding_agents_list` | `GET /api/coding-agents` | Each coding agent Rerail works with: found on this computer, connected, and any problem. Changes nothing. |
| `rerail_rules_list` | `GET /api/rerail-rules` | By Rerail rules, each with its pack. |
| `library_rules_list`, `library_rules_get`, `library_rules_upsert`, `library_rules_remove` | `/api/library-rules[/{id}]` | Rules created by you. Upsert can attach a new rule (`attach: true`) and copies a By Rerail rule on edit. |
| `rule_approvals_set` | `POST /api/rule-approvals` | Approve a rule's or pack's content: `kind`, `id` and the `digest` read from `approval.subject`, with `checkout` for a worktree's own rule. `409 approval-stale` when the content changed since; `404 rule-not-found` when no rule or pack that needs approval has that name. People only. |

`rules_activity`, `sessions_list`, `violations_list`, `audits_list` and `repository_audits_list` take an optional `repository`; without it they cover the whole machine for a person, and the caller's own repository for a coding agent.

**MCP caller.** The MCP bridge sends `x-rerail-caller-cwd` on every request: the URI-encoded real path of the folder its session started in. The daemon resolves it to the caller's checkout and folder; a malformed header is a JSON-RPC `-32602` error. A direct MCP client without the header must name a target, and reads no history. A coding agent reads the history (sessions, transcript events, work links, audits, violations, rule activity) of its own checkout's repository only: naming another repository is `403 agent-not-permitted`, and another repository's session or audit answers `404` as one that does not exist. A coding agent may turn Follow `.gitignore` on, before set-up, but never off.

**Dashboard.** The dashboard link for a repository is `<origin>/?repository=<key>&checkout=<path>#rules`, which opens its Overview (`#rules` is the pre-v8 name the dashboard still resolves; its own is `#overview`); `rerail manage`, `/rerail-manage` and the status line build it. Without a scope the dashboard lists repositories to pick from.

## Export and import

`rerail rules export` writes a version 1 dump: the library, and each repository's key, name, aliases, set-up time, attachments and repository settings. A setting's `suppressions` and `config` are written only when set, so a dump without them still imports into a Rerail that predates them. Checkout settings, start state and approvals belong to this machine and are not exported, so an imported library rule whose content was not approved here runs in Log until a person approves it, and the command says how many do. `rerail rules import <file> --approve` approves each imported rule as it is imported, for a dump of your own rules, and prints each id; Rerail's command files never pre-approve it, so a coding agent's host asks first. `rerail rules import` accepts only that format and is all-or-nothing after every code rule compiles. `rerail reset`, and Clear history in the dashboard's Settings (`repository_reset`), delete only that repository's execution history.

Each audit records, per rule, its revision, payload hash, source (pack, By Rerail rule or library rule), repository key, checkout, and a `placement` array with `on`, `mode`, `suppressions`, `onFrom`, `modeFrom` and `suppressionsFrom` for each folder (audits recorded before `suppressions` existed lack those two). Later edits don't rewrite past audits.
