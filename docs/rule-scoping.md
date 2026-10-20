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

A repository is **Not set up** until its first configuration. Starting Rerail in a repository that is not set up runs set-up there: it asks before reading the repository, recommends rule packs with the reason for each, and asks you to confirm. "Start" installs the recommended packs ([Installing packs](rule-packs.md#installing-packs)), sets the repository up with them, each attached at the folder the recommendation names, and starts it: a language or framework pack at the folder holding everything that matched it (for example `web` when only `web/` has TypeScript), a repository pack for the whole repository. "I'll pick rule packs myself" (`--answer pick-myself`) reads no code: it lists every rule pack set-up fetched, installs the ones you pick and attaches each for the whole repository, taking nothing from the lint config; "None for now" (`--packs none`) sets the repository up with no packs. "Review them first" opens the dashboard's review page (`#rule-packs?review=1`, scoped to the repository and checkout) and set-up asks again rather than waiting on the browser. In the review the recommended packs show switched on until the first change; the first change adds the packs kept on, which sets the repository up without starting it, and later switches add or remove one pack. "Start Rerail with the rule packs you kept on" then starts the repository with those attachments; picking rule packs yourself after a review removes the packs the review added. Set-up remembers that it offered the review until Rerail is first started or stopped in the repository, and only a set-up made in the dashboard keeps that offer: a repository set up any other way (`rerail rules add` or `/rerail-create`, a coding agent adding rules or packs, `rerail rules import`) is set up, so starting it just starts it with what is attached. Set-up run again in a repository that is set up never asks again: it shows the summary, or says Rerail is stopped there. Set-up never adds rules created by you; you add them yourself.

A repository that is set up runs only what is attached to it. The rules of the installed rule packs run only in repositories that add them. Nothing else runs: there are no rule files in the working tree.

## Rules: attachments and settings

Rules come from two places:

- **By Rerail**: the rules of the rule packs installed from npm (`$RERAIL_HOME/packs`). They are read from the installed packs, never copied into the database.
- **Created by you**: the library, rules people wrote, stored once per machine. A library rule can't take a By Rerail rule's id, and one id keeps one kind (`code` or `conversation`). If an installed pack later ships a rule with a library rule's id, the library rule wins and the By Rerail rule is reported as unavailable.

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

## Which checkout judges a file

Each file is judged by the checkout that owns it, with that checkout's configuration. The owner is the nearest folder above the file that holds a `.git` entry Git confirms as a checkout's top level, found from both the file's lexical and real paths (a symlink into another checkout or out of its owner is not evaluated). A nested checkout owns its files when it is a submodule the checkout around it declares, or a checkout Rerail has recorded (such as a worktree kept inside the main checkout); while it is stopped or not set up it runs nothing, and the checkout around it does not judge its files either. Only a nested repository Rerail never recorded (an untracked `git init`, a stray `.git`) is judged with the checkout around it. Only checkouts that are recorded, started and set up evaluate files; a session that starts in a stopped or unrecorded checkout judges nothing.

## Coding agents

The HTTP API (`/api`) is for people; MCP callers (`/mcp`) are coding agents. The distinction is advisory, not a security boundary. A coding agent may:

- start, stop and change settings in its own checkout and the checkouts inside it, at any time;
- change the repository's attachments and settings only while it is not set up;
- add a rule it creates in the same call, or the copy of a By Rerail rule it edits, to a set-up repository.

Anything else is refused with `403 agent-not-permitted` and the dashboard URL where a person can make the change. Only a person links repository addresses, or starts and stops a whole repository.

**Copy on edit.** Writing a By Rerail rule's id through `library_rules_upsert` stores an edited copy in the library (`<id>-copy`, `-copy-2`, …) instead. The copy is attached wherever the original is present in the target repository, at the same folders, and a repository setting switches the original off there. The original stays a reference to its installed pack.

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

`rules_activity`, `sessions_list`, `violations_list`, `audits_list` and `repository_audits_list` take an optional `repository`; without it they cover the whole machine.

**MCP caller.** The MCP bridge sends `x-rerail-caller-cwd` on every request: the URI-encoded real path of the folder its session started in. The daemon resolves it to the caller's checkout and folder; a malformed header is a JSON-RPC `-32602` error. A direct MCP client without the header must name a target.

**Dashboard.** The rules page for a repository is `<origin>/?repository=<key>&checkout=<path>#rules`; `rerail manage`, `/rerail-manage` and the status line build it. Without a scope the dashboard lists repositories to pick from.

## Export and import

`rerail rules export` writes a version 1 dump: the library, and each repository's key, name, aliases, set-up time, attachments and repository settings. A setting's `suppressions` and `config` are written only when set, so a dump without them still imports into a Rerail that predates them. Checkout settings and start state belong to this machine and are not exported. `rerail rules import` accepts only that format and is all-or-nothing after every code rule compiles. `rerail reset` deletes only the current repository's execution history.

Each audit records, per rule, its revision, payload hash, source (pack, By Rerail rule or library rule), repository key, checkout, and a `placement` array with `on`, `mode`, `suppressions`, `onFrom`, `modeFrom` and `suppressionsFrom` for each folder (audits recorded before `suppressions` existed lack those two). Later edits don't rewrite past audits.
