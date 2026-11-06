# Rule scoping

Rerail keeps rule configuration and execution history in one per-user SQLite database: `$RERAIL_HOME/rerail.sqlite`, or `~/.rerail/rerail.sqlite` when `RERAIL_HOME` is unset.

- A **repository** owns its rule configuration.
- Each **checkout** of it decides whether Rerail is on there, and may change rules for itself.
- Sessions, events, work links and audits belong to the repository of the checkout that recorded them.

## Repositories and checkouts

A repository is keyed by its remote address; each working copy of it is a checkout.

- **Repository**: keyed by its `origin` remote, or its only remote, as `remote:<host>/<path>`. A clone without a remote is keyed `clone:<clone id>`.
- **Alias**: another key that resolves to a repository, such as a rename, an ssh host alias or a mirror.
- **Checkout**: one working copy, either a clone's main checkout or a linked worktree.

Two clones of one remote are one repository and share its configuration. A fork has its own remote, so it is its own repository.

<details>
<summary>How keys and checkouts are assigned</summary>

The remote address is normalised: lower-case, without scheme, user, port, trailing `/` or `.git`. The https, ssh and scp forms of one address give one key.

The clone id lives in `rerail-repository-id` in Git's common directory, or `.rerail/repository-id` for a folder outside Git.

A person links an alias with `repository_link`. An address that already belongs to another set-up repository is refused.

A checkout is recorded once, under its physical (symlink-resolved) Git top level, and joins the first of:

1. the repository of another recorded checkout of the same clone;
2. the repository its remote names, by key or alias;
3. a new repository.

When a checkout of a clone-keyed repository later has a remote, the remote's repository adopts it:

- **Not set up:** the two merge, and the old key becomes an alias.
- **Set up:** only the checkouts move.

A checkout whose folder is gone, or that Git no longer lists, is forgotten with its own settings.

</details>

## Start and stop

Rerail is off until it is started, per checkout or for a whole repository.

A checkout uses its own start value, else that of the nearest recorded checkout of the same repository that contains it and has one, else the repository's.

| State | What happens |
|---|---|
| **started** | Rules run. |
| **stopped** | Nothing is recorded, evaluated or delivered; hooks don't contact the daemon. |
| **not-set-up** | Started, but the repository has no configuration yet: sessions are recorded, no rule runs. |

- `/rerail-start` and `/rerail-stop` (or `rerail start` and `rerail stop`) act on this checkout and its nested checkouts, which follow it again.
- `rerail start --repository` and `rerail stop --repository` act on every checkout of the repository, clearing their own values.

Stopping keeps rules and history.

## Set-up

Set-up gives a repository its first configuration, normally the rule packs recommended for its code.

A repository is **Not set up** until its first configuration. Starting Rerail there runs set-up. It asks before reading the repository, recommends rule packs with a reason for each, and asks you to confirm:

- **Start** installs the recommended packs ([Installing packs](rule-packs.md#installing-packs)), attaches each at the folder the recommendation names, and starts the repository.
- **Not now** (`--answer not-now`) installs, connects and turns on nothing.
- **Skip, I'll pick packs** (`--answer pick-myself`) reads no code and lets you pick packs yourself.
- **None for now** (`--packs none`) sets the repository up with no packs.

<details>
<summary>Where packs go, and other cases</summary>

- **Start** puts a language or framework pack at the folder holding everything that matched it (`web` when only `web/` has TypeScript). A repository pack covers the whole repository.
- **Skip, I'll pick packs** is offered at the question before the confirm. It lists every rule pack set-up fetched, installs the ones you pick and attaches each for the whole repository.
- An earlier version of set-up may have offered the dashboard's review. If nothing was changed there, set-up goes straight to the confirm question.
- A repository set up any way (the dashboard, `rerail rules add`, `/rerail-create`, a coding agent adding rules or packs, `rerail rules import`) is set up. Starting it starts what is attached, asking nothing.
- Set-up run again in a set-up repository never asks again: it shows the summary, or says Rerail is stopped there.
- Set-up never adds rules created by you; you add them yourself.

</details>

A set-up repository runs only what is attached to it, plus each checkout's own rule documents in `.rerail/rules/` ([a worktree's own rules](rule-packs.md#a-worktrees-own-rules)). Installed packs' rules run only in repositories that add them; no other rule file in the working tree is read.

### The repository's own lint config

So Rerail does not steer against what a repository deliberately configured, set-up reads its lint config at the repository root. The added packs' rules get matching settings, owned by the repository.

It reads ESLint, oxlint and Ruff configs as data and never runs them. A rule the config turns off for every file starts off; set-up never turns a rule on.

<details>
<summary>What is read, per linter</summary>

| Linter | Read | Not read |
|---|---|---|
| ESLint | The config ESLint would use: the first flat config (`eslint.config.{js,mjs,cjs,ts,mts,cts}`) when it is data throughout, parsed and never run; else `.eslintrc.json`, or `.eslintrc` when JSON; else `package.json`'s `eslintConfig`. JSON may have comments and trailing commas. Its own `rules` and its `overrides`' `rules`. | A flat config that runs code (a function, a statement that changes something, a namespace, a loop). A flat-config value that needs code run (an imported config such as `js.configs.recommended`, a call's result, an imported variable). `.eslintrc.{js,cjs,yaml,yml}`. An `.eslintrc`'s `extends` and plugins' presets. |
| oxlint | `.oxlintrc.json`, top-level `rules`. Each key resolves to a built-in rule as oxlint 1.88.0 resolves it; a rule's last key applies. | `extends`, `overrides`, `categories`, `plugins` (a key for a plugin the config leaves off still counts); rule options. |
| Ruff | `select`, `extend-select`, `ignore`, `extend-ignore`, `per-file-ignores`, `extend-per-file-ignores` (from `[lint]` where set there, else the top level). From the file Ruff uses (`.ruff.toml`, then `ruff.toml`, then `pyproject.toml`'s `[tool.ruff]`/`[tool.ruff.lint]`) and configs its `extend` names inside the repository. | A config `extend` names outside the repository; per-file ignores of an extended config in another folder; Bandit config. |

</details>

<details>
<summary>What set-up does with it</summary>

- **Nothing runs.** Each config is parsed as data, never run or imported. Set-up never turns a rule on.
- **Off.** A rule the config turns off for every file (`"off"`, `0`, oxlint's `"allow"`, a Ruff ignore that wins over its selects) starts off. If one config turns a rule off and another keeps it on, it starts off and set-up says so.
- **Options.** ESLint options become the port's options (`config`) when every value maps onto an option the port declares. Otherwise the port keeps its defaults, with a note.
- **Some files only.** An entry limited to some files (ESLint's `files: ["scripts/**"]`, a Ruff per-file ignore of `tests/**`) turns a rule off only in a folder it covers whole. For narrower files (`**/test_*.py`) the rule stays on, with a note.
- **A flat config Rerail cannot read.** If the flat config is more than data (defines a function, runs a loop, changes something), is a symlink, or exceeds 1 MiB, every rule Rerail ports from ESLint starts off, with no options. Set-up says why first. A single entry whose value needs the config run (a call's result, an imported variable) turns only that port off. A shared config the file imports (`js.configs.recommended`) is named as not read.
- **Configs not read.** An `.eslintrc.js`, `.eslintrc.cjs` or YAML config is named as not read and changes nothing.
- **Your choices win.** A rule you already set in the dashboard's review, in that folder, keeps your setting. "Skip, I'll pick packs" takes nothing from the lint config, even for the packs you then pick.

A port matches an upstream rule by the names its document honours suppression comments for. So `custom/no-console`, another plugin's rule, leaves the port of ESLint's `no-console` alone.

</details>

The exact reading rules (what counts as data in a flat config, the order ESLint applies entries in, how Ruff's selectors resolve) are in the header comments of `src/engine/catalog/lint-config/`.

## Rules: attachments and settings

Rules come from three sources; attachments make them present in a folder, and settings decide how they run.

- **By Rerail**: rules of the rule packs installed from npm (`$RERAIL_HOME/packs`). The dashboard labels a third party's pack's rules **By** its publisher instead (`By @acme`; [third parties' packs](rule-packs.md#third-parties-packs)).
- **Created by you**: the library, rules people wrote, stored once per machine.
- **In this worktree**: rule documents in one checkout's `.rerail/rules/`, present in that checkout only ([a worktree's own rules](rule-packs.md#a-worktrees-own-rules)).

<details>
<summary>Source details</summary>

- **By Rerail** rules are read from the installed packs and never copied into the database. A third party's pack is installed and read the same way, and everything below applies to it.
- **Created by you:** a library rule can't take a By Rerail rule's id, and an id keeps one kind (`code` or `conversation`). If an installed pack later ships a rule with a library rule's id, the library rule wins and the By Rerail rule is reported as unavailable.
- **In this worktree:** the documents are read from the checkout's working tree as the pack `worktree`. They are present in that whole checkout without an attachment, and in no other. Settings apply to them as to any pack's rules.

</details>

- A **rule attachment** adds a rule pack, a By Rerail rule or a library rule to a repository, in a folder (`""` is the whole repository).
- A **rule setting** changes `on`, `mode`, `suppressions` or `config` for one rule or one whole pack in a folder. It belongs to the repository (the default for every checkout) or to one checkout (this worktree only).

<details>
<summary>Attachment and setting fields</summary>

Folders are relative to the repository root, POSIX, without `.` or `..`.

- A rule in an attached pack belongs to that pack, even if also attached on its own.
- An attachment whose target no longer exists is reported as unavailable, never thrown.

A setting's fields:

- **`on`**.
- **`mode`**: Log, Steer or Block. Block is in [internal testing](../README.md#internal-testing) and is delivered like Steer.
- **`suppressions`**: `honour` or `ignore` linters' suppression comments (`// eslint-disable-next-line`, `# noqa`, `# nosec`, ...). A ported rule reads the comments of the linter it ports, where its document's `suppress` names them. Under `ignore`, no linter's comment silences it.
- **`config`**: option values of one code rule by Rerail, by option name. They replace the defaults its document's `define.options` declare, and the document is resolved again with them. It is set on a rule, not a pack, for the whole repository (folder `""`), because one compiled rule runs in every folder of a checkout. Library rules take no `config`: edit the rule instead. An option the document doesn't declare, or a value its declaration doesn't allow, is refused with `400 invalid-setting`.

A null field leaves it to the next owner.

</details>

`suppressions` governs only linters' comments. Rerail's own comment, `rerail-ignore`, is always honoured: `// rerail-ignore complexity-cyclomatic-typescript -- generated`.

<details>
<summary>Where <code>rerail-ignore</code> applies</summary>

It is written with `//`, `#`, `/* */` or `{{-- --}}` per language. It silences the code rules it names by id at the node it covers:

- the whole file, when it comes before the file's first code;
- the next node, when it stands on its own line;
- otherwise, the code it trails on its line.

Every code rule answers to it, with nothing in its document. `docs/code-rule-dsl-reference.html#rerail-ignore` has the exact rules.

</details>

**Rule Path Criteria** (`scope.include`, `scope.exclude` on a code rule, excludes winning) still narrow the paths a present rule evaluates. Execution `scope` and run mode still decide when it evaluates.

## Precedence

Which rules apply to a file, and which setting decides each of their fields.

For a file at path `p`, the rules present are those attached in a folder that contains `p` (`web` contains `web/x`, not `web2/x`). Each field (`on`, `mode`, `suppressions`, and each option in `config`) of each rule is decided separately; the first value wins:

1. by owner: this checkout, then the checkouts that contain it (nearest first), then the repository;
2. within one owner, the deepest folder that contains `p`;
3. within one folder, the rule's own setting before its pack's;
4. without a setting, the value the pack starts the rule with, then the rule's own default (on, in its document's mode).

<details>
<summary>Defaults, sources and errors</summary>

For `suppressions` the pack's start is its `honourSuppressionComments` (`honour` unless the pack sets it to false); a rule outside a pack honours them. Without a `config` value, an option keeps its document's value.

So `config` merges per option: a checkout that sets `{ "option": "type" }` keeps the other options the repository sets.

`repository_rules_get` reports what decided each field of every effective rule as `onSource`, `modeSource`, `suppressionsSource` and `configSource`. `configSource` names, per option, this checkout's or a containing checkout's setting, the repository's setting, the pack's starting value, or the rule's default.

If a stored `config` no longer applies to the rule's document (a later release removed the option), `configError` says why. The rule runs nowhere: it is quarantined with that reason.

Conversation rules, in [internal testing](../README.md#internal-testing), resolve the same way for the folder the session started in.

</details>

## Approval

A rule Rerail did not publish runs in Log, whatever its settings say, until a person approves its content.

This covers library rules, rules in a worktree's `.rerail/rules/`, and every rule of a pack that is not Rerail's own from npm. A person approves with:

- `rerail rules approve <id | package>` (no name lists what waits);
- the Approve button on the rule's page or the pack's sheet in the dashboard;
- `rule_approvals_set`.

<details>
<summary>What is approved, and when it waits again</summary>

A pack that is not Rerail's own from npm is a third party's pack, or any pack installed from a folder or tarball. Rerail's own published packs need none.

While a rule waits, no setting decides its mode: `modeSource` is `{ "by": "awaiting-approval" }`. Its `approval` gives the `subject` to approve (`kind` `rule` or `pack`, `id`, `digest`) and `approved: false`.

An approval is of content, on this machine:

- **A rule**: its id and the digest of its document.
- **A pack**: its package name and the digest of its version, what it says of itself (`name`, `category`, `covers`, `reason`), `honourSuppressionComments` and every rule document it lists.

An edit, or a new pack version, is new content and waits again. Going back to approved content needs no second approval. A library rule a person writes through the dashboard or HTTP API is approved with the write; one a coding agent writes, or `rerail rules add` adds, waits. Library rules stored before approvals existed count as approved.

</details>

<details>
<summary>How approving works, and its limits</summary>

`rerail rules approve` first prints what it approves: a rule's JSON in full, invisible characters written as escapes, or a pack's rule ids. Rerail's command files never pre-approve it, so a coding agent's host asks the person before it runs.

The session footer counts what waits (`2 to approve`), as does `/rerail`, and Overview lists it. An approval changes the configuration signature, so a running session picks it up at its next event.

`rule_approvals_set` refuses a coding agent, but `rerail rules approve` cannot tell who runs it: a coding agent with a shell can run it too. That the agent waits for the person's yes is the `/rerail-create` skill's instruction, not something Rerail enforces, as with the rest of the line between people and coding agents ([coding agents](#coding-agents)). Approval keeps a rule no person read from steering by default; it does not stop an agent set on approving one.

</details>

## Which checkout judges a file

Each file is judged by the checkout that owns it, with that checkout's configuration.

The owner is the nearest folder above the file holding a `.git` entry that Git confirms as a checkout's top level. Only checkouts that are recorded, started and set up evaluate files.

A worktree of a repository where Rerail runs is recorded the first time a session looks at it, wherever it sits and whichever folder the session started in, and judges its files itself. A session looks at it when a path it names falls in it, or when it first walks the repository's other worktrees (below), so any session of the repository records every such worktree, edited or not. Its record stays, as every recorded checkout's does. Git decides what counts: `git worktree list`, run in a recorded checkout of the same clone, must name it. Another clone of the same remote doesn't count.

Shell writes are found by walking files, so each session also walks the repository's other worktrees that may run rules, behind its own folder's walk. A command doesn't wait for this. The walk can't tell who wrote a file, so what it finds is recorded under the repository, in Log, and steers no session: an agent is steered only on its own tool calls and on shell writes in its own folder made while one of its tool calls ran. A shell write an agent makes in another worktree is therefore recorded but not steered, and so is another session's write in the agent's own folder. What Git wrote is left out, as in the session's own folder.

<details>
<summary>Nested checkouts and symlinks</summary>

The owner is found from both the file's lexical and real paths; a symlink into another checkout or out of its owner is not evaluated.

A nested checkout owns its files when it is a submodule the checkout around it declares, or a checkout Rerail has recorded (such as a worktree kept inside the main checkout). While it is stopped or not set up it runs nothing, and the checkout around it does not judge its files either. Only a nested repository Rerail never recorded and Git does not list as a worktree (an untracked `git init`, a stray `.git`, even one naming this repository's Git folder) is judged with the checkout around it.

A session that starts in a stopped or unrecorded checkout judges nothing.

</details>

## Coding agents

What a coding agent may change, and what only a person may do.

The HTTP API (`/api`) is for people; MCP callers (`/mcp`) are coding agents. The distinction is advisory, not a security boundary. A coding agent may:

- start, stop and change settings in its own checkout and the checkouts inside it, at any time;
- change the repository's attachments and settings only while it is not set up;
- add a rule it creates in the same call, or the copy of a By Rerail rule it edits, to a set-up repository.

Anything else is refused with `403 agent-not-permitted` and the dashboard URL where a person can make the change. Only a person links repository addresses, starts and stops a whole repository, or approves a rule's content ([approval](#approval)). A rule a coding agent writes runs in Log until then.

**Copy on edit.** Writing a By Rerail rule's id through `library_rules_upsert` stores an edited copy in the library instead (`<id>-copy`, `-copy-2`, …).

<details>
<summary>How the copy replaces the original</summary>

- The copy is attached wherever the original is present in the target repository, at the same folders.
- Its first approval switches the original off there with a repository setting: at once for a person's write, which is approved as saved, and for a coding agent's copy when a person approves it.
- Until then the original keeps running beside the copy, which runs in Log, so writing a copy never weakens a rule.
- The original stays a reference to its installed pack.

</details>

## Management surfaces

One set of operation definitions drives both HTTP/OpenAPI and MCP.

Every operation names its target in the query or body, never the path: `repository` (key or alias), `checkout` (an absolute path in a checkout) and `folder`.

<details>
<summary>How the target resolves</summary>

A checkout wins, then the repository, then the calling agent's checkout. A repository that does not match the checkout is `target-mismatch`. Reads never record a checkout; writes default to the whole repository.

`rules_activity`, `sessions_list`, `violations_list`, `audits_list` and `repository_audits_list` take an optional `repository`. Without it they cover the whole machine for a person, and the caller's own repository for a coding agent.

</details>

<details>
<summary>Operations and HTTP routes</summary>

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

</details>

**MCP caller.** The MCP bridge sends `x-rerail-caller-cwd` on every request: the URI-encoded real path of the folder its session started in.

<details>
<summary>How MCP callers are scoped</summary>

- The daemon resolves it to the caller's checkout and folder. A malformed header is a JSON-RPC `-32602` error.
- A direct MCP client without the header must name a target, and reads no history.
- A coding agent reads the history (sessions, transcript events, work links, audits, violations, rule activity) of its own checkout's repository only. Naming another repository is `403 agent-not-permitted`; another repository's session or audit answers `404`, as if it did not exist.
- A coding agent may turn Follow `.gitignore` on, before set-up, but never off.

</details>

**Dashboard.** A repository's dashboard link is `<origin>/?repository=<key>&checkout=<path>#rules`, which opens its Overview. `rerail manage`, `/rerail-manage` and the status line build it. Without a scope, the dashboard lists repositories to pick from. (`#rules` is the pre-v8 name the dashboard still resolves; its own is `#overview`.)

## Export and import

How rule configuration moves between machines, and what each audit records about it.

`rerail rules export` writes a version 1 dump: the library, and each repository's key, name, aliases, set-up time, attachments and repository settings. `rerail rules import <file>` reads it back. Checkout settings, start state and approvals belong to this machine and are not exported.

Each audit records, per rule, its revision, payload hash, source (pack, By Rerail rule or library rule), repository key, checkout, and a `placement` array. Later edits don't rewrite past audits.

<details>
<summary>Import, reset and audit details</summary>

A setting's `suppressions` and `config` are written only when set, so a dump without them still imports into a Rerail that predates them.

An imported library rule whose content was not approved here runs in Log until a person approves it; the command says how many do.

- `rerail rules import <file> --approve` approves each imported rule as it is imported, for a dump of your own rules, and prints each id. Rerail's command files never pre-approve it, so a coding agent's host asks first.
- `rerail rules import` accepts only that format, and is all-or-nothing after every code rule compiles.
- `rerail reset`, and Clear history in the dashboard's Settings (`repository_reset`), delete only that repository's execution history.

Each folder's entry in an audit's `placement` has `on`, `mode`, `suppressions`, `onFrom`, `modeFrom` and `suppressionsFrom`. Audits recorded before `suppressions` existed lack those two.

</details>
