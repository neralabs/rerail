# Rerail rule pack: Rerail repository

Rules for working in this repository: they steer the coding agent toward the architecture and conventions Rerail has
written down (REVIEW.md, CONTEXT.md, `src/README.md`, `docs/`) and has broken before. Set-up offers the pack to a
repository with a `packs/*/pack.json`; it is a `repository` pack, added for the whole repository (see
`docs/rule-packs.md`).

The pack ships 34 rules: 14 in Steer mode, which steer the coding agent in the session, and 20 in Log mode, which only
record. A Steer rule matches nothing on `main` today. A rule is in Log when code the maintainers merged still breaks
it, when the place the code should use does not exist yet, or when it should run a while before it steers. Each Log
rule's Gaps say which, and what has to land before it can steer. Every rule is a code rule but
`rerail-pack-data-through-publish`, a conversation rule on the commands the coding agent runs.

The pack is private (`"private": true` in its `package.json`): it is never published, and
`rerail packs add ./packs/rerail` installs it from this checkout. Each rule was drafted with `rerail rules test` and
checked against the repository's history; the cases it was drafted with are in
[`fixtures/rerail/<rule id>.cases.json`](../../fixtures/rerail/), and
`test/cli/rerail-pack.test.ts` runs them. The history behind each rule is in [HISTORY.md](HISTORY.md), and the
drafts not shipped are in [candidates/README.md](candidates/README.md).

The repository pass of `rerail rules test` does not read hidden folders, so the counts it gives for the two workflow
rules, whose files are under `.github/`, come from a copy of the tree. Live checks read `.github/` as any other folder.

| Group | Rule | What it flags | Mode |
|---|---|---|---|
| Layering | `rerail-conformance-boundary` | Programs started in `conformance/` outside its command runner, and authoring code (recorders, expected-findings writers, pack-data imports) in it | Steer |
| Layering | `rerail-conformance-pinned-data` | Network calls and download or install programs in `conformance/` | Log |
| Layering | `rerail-host-sdk-imports` | A coding agent's SDK imported outside `src/hosts/` | Log |
| Layering | `rerail-language-branching` | Branching on a language, family or embedding outside its Engine Adapter | Log |
| Ownership | `rerail-command-sigil` | A command sigil decided in place, or a hard-coded `/rerail…` a Codex user reads | Steer |
| Ownership | `rerail-fixed-value-lists` | Hosts, audit values, rule modes or pack categories listed again instead of taken from their constant | Log |
| Ownership | `rerail-git-no-lazy-fetch` | Git started outside `src/engine/git.ts`, or without lazy fetch off | Steer |
| Ownership | `rerail-home-file-names` | A hand-kept list of the Rerail home's file names | Log |
| Ownership | `rerail-home-file-paths` | The home's `current`, `versions`, `update-state.json` or `bin` path joined by hand | Log |
| Ownership | `rerail-no-code-execution` | Pack matches or repository config run as code; npm pack installs that run scripts | Log |
| Ownership | `rerail-no-message-classification` | Failures told apart by message wording; steer text parsed back | Log |
| Ownership | `rerail-pack-package-names` | The `@neralabs/rerail-pack-` prefix spelled out in code | Steer |
| Ownership | `rerail-session-paths` | A session's storage root or model path resolved again outside `src/runtime/session-paths.ts` | Steer |
| Ownership | `rerail-start-state` | A start state derived outside `src/runtime/repository-start.ts` | Log |
| Ownership | `rerail-table-probes-in-storage` | Table or column probes of Rerail's database outside storage | Log |
| Ownership | `rerail-walk-limits` | A TypeScript repository walk's skipped folders or limits restated | Steer |
| Repository layout | `rerail-fixtures-root` | Rust reaching `fixtures/` by a relative path or the oracles variable | Steer |
| Repository layout | `rerail-generator-placement` | A file-writing script in `packs/` or `native/src/` beside its output | Steer |
| Repository layout | `rerail-pack-rule-test-placement` | A single pack rule's test outside `test/engine/code-rule/pack-rules/` | Steer |
| Repository layout | `rerail-rule-documents-in-packs` | A rule document or draft outside a pack's `rules/` or `candidates/` | Steer |
| Release and pack data | `rerail-pack-data-helper-copies` | An upstream generator's own copy of a shared pack-data helper, or its own working-directory fake | Log |
| Release and pack data | `rerail-pack-data-runtime-names` | A versioned runtime name written in a pack-data recipe | Steer |
| Release and pack data | `rerail-pack-data-through-publish` | Pack data uploaded to S3 from a workstation (a command, not code) | Steer |
| Release and pack data | `rerail-provenance-writers` | Licence and provenance records written or parsed outside their owners | Steer |
| Release and pack data | `rerail-workflow-script-facts` | Entry paths, shipped paths, tool-pack legs or pack-name tests hand-kept in workflows | Log |
| Release and pack data | `rerail-workflow-shared-conditions` | The release pull request skip repeated in one workflow | Log |
| Dashboard | `rerail-ui-api-precedence` | Setting precedence worked out in the dashboard | Log |
| Dashboard | `rerail-ui-color-tokens` | Raw colours in the dashboard, or a Nera token redefined | Steer |
| Dashboard | `rerail-ui-pure-modules` | Network, storage or DOM work in the dashboard's pure modules | Log |
| Native engine | `rerail-grammar-node-kinds` | Grammar node kinds named in shared engine code instead of on the family's Engine Adapter | Log |
| Native engine | `rerail-language-facts-from-registry` | Hand-kept lists of file suffixes or language names in shared engine code | Log |
| Native engine | `rerail-origin-ranges` | Ranges mapped by `host_range`/`host_span` instead of `Origin` | Log |
| Native engine | `rerail-store-reads-in-documents` | Content-store source or tape reads outside `workspace/documents.rs` | Log |
| Native engine | `rerail-walk-limits-native` | A Rust repository walk's skipped folders or limits restated | Log |

## Rule details

Each rule's details: what it flags and where, the history it is built on, and what it misses or may wrongly
flag. A Log rule's gaps say what is still on `main` today.

### `rerail-conformance-boundary`

Flags conformance code that starts a program with `child_process` instead of its command runner
(`conformance/process.ts`), imports from the pack-data, upstream or pack-generator scripts, holds a recorder, or writes
expected findings. It runs on TypeScript and JavaScript under `conformance/` and on anything under a `recorders/` folder
there; `conformance/process.ts`, the check's worker pool `conformance/languages/shards.ts` (it forks its own worker
script, not a program) and generated files are out of scope. Type-only `child_process` imports and JSON or YAML
data under a pack's `upstream/` pass; type imports from the authoring path are caught.

History: Built on #85's three direct `git` calls and #177's runner bypasses and nine authoring-path imports; it caught the
first draft of each and the unmerged direct calls of #160 and #175, fell to 0 at #177's merge, and the only rises in 15
recent merged PRs were #155's and #141's recorders, added before #177 moved recorders out of `conformance/`.

Gaps: The `samplesTree` route to `aws` through `scripts/s3/pack-fixtures.mjs` bypasses the runner and is not seen.
`scripts/pack-data/` still calls `child_process` directly and is out of scope.

### `rerail-conformance-pinned-data`

Flags network calls (`fetch`, `x.fetch`, Node's `http`/`https`, `undici`) and programs that download or install (`aws`,
`curl`, `git clone`, `git ls-remote`, `git remote add`, `npm ci`, `uv sync`) in conformance code, which reads only pinned data
through `scripts/s3/pack-fixtures.mjs`; network work belongs in `scripts/upstream/` or `scripts/pack-data/`. It runs on
TypeScript and JavaScript under `conformance/`, leaving out `conformance/process.ts` and generated files.

History: Built on #85's corpus download, #125's release fetch, #179's release polling and bump, and #177's `aws s3 cp`;
it caught the first draft of each, was 0 at #177's merge and quiet on 15 recent merged PRs.

Gaps: Log until it has run a while; its history shows no false positive, and main has 0 matches today. The `git fetch` of a pinned
base commit in `ungated.ts` is allowed and not matched, and the indirect `aws` route through the fixtures transport is
not seen.

### `rerail-host-sdk-imports`

Flags imports of coding-agent SDKs (`@anthropic-ai/claude-…`, `@openai/codex…`, `@cursor/sdk`, `@mariozechner/pi-…`, the
OMP SDK) and of Pi's tool schema library `@sinclair/typebox` outside `src/hosts/`, in every import form: `import`,
`import()`, `require`, `createRequire(…)(…)`, `import x = require()`, `/// <reference types>` and `declare module`. It runs
on TypeScript and JavaScript under `src/`, leaving out `src/hosts/**` and tests; shared layers describe the host surface
they need with their own types.

History: Built on #1's footer draft and #26, which moved the hosts into `src/hosts/` but left the OMP and Pi register
functions in `src/harness/native-semantic-tools.ts`; it caught both, was quiet on #19's `PresentationContext` fix
(14 → 13) and on 16 recent merged PRs.

Gaps: Log until `registerOmpSemanticTools` and `registerPiSemanticTools` move into `src/hosts/omp` and `src/hosts/pi`;
3 matches on main today, all in `src/harness/native-semantic-tools.ts`. It does not check that one host's SDK stays in
that host's folder.

### `rerail-language-branching`

Flags code that branches on a language, family or embedding (`Family::of(…) == …`, `GrammarDialect::Blade`, `Via::…`,
`matches!` and `match` arms on language labels, TypeScript `language === "python"` and `.includes(language)`) outside
its Engine Adapter, the embedders, the registry or the family's own modules; elsewhere the code asks the adapter
(`with_adapter!`, `with_embedder!`) or reads the registry. It runs on `native/src/**/*.rs` and `src/**/*.ts`, leaving out
`language.rs`, `language/**`, `build_languages.rs`, family-named modules, tests and `#[cfg(test)]` items.

History: Built on #32's Blade branches and #132's first draft, where PHP's scope module branched on Blade's view
embedding; it caught both and five unmerged branch commits, dropped 259 → 22 → 2 across #33's and #44's moves into the
adapter, and was quiet on #52's merge and 17 recent merged PRs.

Gaps: Log until the 4 branches on main today move onto the family's adapter: the Forte choices in `tape.rs:1170` and
`workspace/evaluate.rs:856`, the Blade guard in `encode_blade` (`tape.rs:1278`), and `import_target.rs:89` refusing
`importTarget` by family. Misses `use Family::*` with bare variants, `as` aliases, label constants, `.eq("python")`,
`grammar.name() == "blade"`, `if let "python" = language`, TypeScript `{…}[language]` lookups and template literals;
`scripts/` is out of scope. A new family module with a neutral name is flagged until the exemption names it. Grammar
node kinds (`node.kind() == "block"`) are branching too (#373 review) but are flagged by
[`rerail-grammar-node-kinds`](#rerail-grammar-node-kinds), in Log, not here.

### `rerail-command-sigil`

Flags code that decides the command sigil itself (a Codex test choosing `$` over `/`, a local `sigilFor`, a Codex table)
and text a Codex user can read that hard-codes a slash before a Rerail command (`/rerail-start`). The sigil comes from
`commandSigil` on `ADAPTER_CAPABILITIES` in `src/engine/adapter-contract.ts`, passing the host or agents in where missing.
It runs on `src/**/*.ts`, leaving out the OMP, Pi, Claude Code and Cursor hosts, OMP- and Pi-only text, `show-me.ts`,
the adapter contract and tests.

History: Built on the hard-coded slashes of #1, #23, #25, #121 and #182 (including `renderNotNow`) and the local sigil
decisions of #19, #61 and #222; it caught every one and was quiet on 16 recent merged PRs.

Gaps: Misses a bare-slash `"/" + command.name` or `` `/${command.name}` `` (legitimate for Claude Code and Cursor), a
hard-coded `$rerail-start` (legitimate in Codex-only notes) and a slash right after a substitution. Text that names a
slash command as a family noun ("Adds /rerail commands") is flagged.

### `rerail-fixed-value-lists`

Flags a restated whole list of hosts, audit scopes, statuses, modes or failure codes, rule modes or pack categories, as a
union type, array, `z.enum` or `switch`, where it should derive from the shared constant: `RERAIL_HOSTS`
(`src/engine/adapter-contract.ts`), `AUDIT_SCOPES`, `AUDIT_STATUSES`, `AUDIT_MODES`, `AUDIT_FAILURE_CODES`
(`src/engine/audit/audit.ts`), `RULE_MODES` (`src/engine/catalog/rule-mode.ts`) and `RULE_PACK_CATEGORIES`
(`src/engine/catalog/rule-packs.ts`). It runs on TypeScript under `src/`, `scripts/` and `integration/`, leaving out the
owners, the storage schema and migrations, tests and generated files; subsets and supersets pass.

History: Built on #1's copied failure-code lists, which #45 consolidated, and #162's rule-mode copy; it caught both, was
quiet on #45's merge (16 → 14) and on 14 recent merged PRs.

Gaps: Log because `AUDIT_STATUSES`, `AUDIT_FAILURE_CODES`, `RULE_PACK_CATEGORIES`, `AUDIT_SCOPES` and `AUDIT_MODES` are not
exported from `src/engine/index.ts`, so `scripts/` and `integration/` cannot follow the message; 15 matches on main today.
Misses reordered lists; matches inside comments and strings. UI host maps and Rust mode lists are out of scope.

### `rerail-git-no-lazy-fetch`

Flags a Git spawn whose environment does not turn lazy fetch off (`GIT_NO_LAZY_FETCH`), any Git started outside the
engine's Git module, `src/engine/git.ts` (`runGit`), including bare `"git"`, a hand-built Git path and shell command
strings, and a `git diff` or `git status` argument list (`["diff", …]`, after any leading options) without
`"--no-renames"`: with lazy fetch off, rename detection fails in a partial clone that lacks the source's blob. It runs
on `src/**/*.ts`, leaving out tests and declaration files.

History: Built on #152's first draft, whose `readGitBlobs` spawned `cat-file --batch` without the variable, and on #1's
four Git spawns outside any owner that #23 gathered; it caught #152's draft (3 → 4) and was quiet on its fix, its merge
and 20 recent merged PRs. The rename clause came from #373's review: `headless-scan.ts`'s `git diff` failed in a sparse
partial clone on an edited rename, and #373 fixed it (1 → 0).

Gaps: Misses a local function returning a Git path, an aliased `gitExecutable` import, `Bun.spawnSync(["git", …])`, an
execa tagged template, and an env passed as a value. Rust's `Command::new("git")` is not covered. The rename clause
reads only array literals that start with `diff` or `status`: an argument list built another way, `git log` or
`git show` with `--name-status`, `--stat` or `--follow`, and `-c diff.renames=false` instead of the flag are not seen.

### `rerail-home-file-names`

Flags a hand-kept list of the Rerail home's file names (an array, `Set`, regex, `switch` or object keys spelling
`daemon.json`, `consent.json`, `rerail.sqlite` and the rest) where it should be built from the file-name constants their
writers export, such as `RERAIL_DATABASE_FILE` and `DAEMON_STATE_FILE`, keeping suffix patterns like `-wal`. It runs on
`src/**/*.ts`, leaving out tests and generated files; prose naming files passes.

History: Built on #222's uninstall list; it caught the first draft sent back in review, and the fix and merge still
matched because they kept a hand list, which drifted when #241 added `access-token`. It is not quiet on recent merged
work: #362 merged two more hand lists, the private files in `src/engine/storage/private-files.ts` and the agent's
denied Read patterns in `src/setup/harness-registration.ts`. This pull request builds uninstall's list and the
harness-registration one from the writers' constants.

Gaps: Log because 1 match is on main today, `PRIVATE_HOME_FILES` in `src/engine/storage/private-files.ts`: the
engine cannot import the daemon's and the coding-agent store's constants (`DAEMON_STATE_FILE`, `DAEMON_LOG_FILE`,
`DAEMON_ACCESS_TOKEN_FILE` in `src/runtime/`, `CLAUDE_STATUS_DATABASE_FILE` in `src/setup/`) until they move into
`src/engine/storage/`. Misses `Set.add` chains, names built by `.map`, regexes like `/daemon[.](?:json|log)/` and lists
of only generic names (`current`, `versions`, `bin`, `packs`). An owner's own side-file list is flagged.

### `rerail-home-file-paths`

Flags `join` or `resolve` of the Rerail home with `current`, `versions`, `update-state.json` or `bin`, and the same paths
built as templates, where the active-version link, versions folder, update record and launcher paths should come from
the updater. It runs on `src/**/*.ts`, leaving out `updater.ts`, tests and generated files; the first argument must be a
home.

History: Built on the paths re-spelled by #25, #140 and #222; it caught each at the commit that added it, dropped at
#133, the one fix, and was quiet on 17 recent merged PRs.

Gaps: Log until the four names are exported constants in a module `src/engine/` can import and the matcher stops matching
comments; 11 matches on main today. `join(context.home, "bin")` with the user's home is logged; `home + sep + "current"`
is missed. `scripts/` is out of scope.

### `rerail-no-code-execution`

Flags code that runs a rule pack's match or a repository's config instead of reading it as data (`child_process`,
`node:vm`, `eval`, `new Function`, computed `import()` or `require`), and npm installs, uninstalls and updates without
`--ignore-scripts`. It runs on `src/**/*.ts`: the code-running checks in `src/engine/catalog/`, the pack store, pack and
repository set-up and repository start; the npm checks everywhere but `src/setup/updater.ts`; tests and fixtures are out.
`npm rebuild` passes.

History: Built on #140's first draft, which ran pack match checks in a child Node process and uninstalled packs without
`--ignore-scripts`; it caught it (0 → 5) and was quiet on its fix, its merge and 14 recent merged PRs.

Gaps: Log until it has run a while; main has 0 matches today. Rerail's own core install in the updater runs without `--ignore-scripts` and is
out of scope.

### `rerail-no-message-classification`

Flags TypeScript and Rust that classify a failure by another Rerail component's message wording (`startsWith`,
`includes`, `.test`, `===`, `switch`, `contains`, `match` on a message or a local constant holding the wording), and code
that parses steer notices back out of their text instead of building them from structured rule data. Classify by the
failure's code or a shared constant. It runs on `src/**/*.ts` and `native/src/**/*.rs`, leaving out tests, generated
files and the npm adapters `src/runtime/npm.ts` and `src/setup/rule-pack-store.ts`.

History: Built on the steer parsers of #19 and #61, #46's four message recognisers, #45's blind-file check and #162's
late-steer lead; it caught each first break, #162's fix cleared the steer-parse matches, and it was quiet on 18 recent
merged PRs.

Gaps: Log because a person's or another program's text in a variable named `message` or `error` (git stderr, SQLite's
"database is locked") cannot be told apart; 14 matches on main today. Misses renamed variables, an HTTP body read as
text, `Set` membership, `.some(m => msg.includes(m))` and a literal duplicated across files; `conformance/` and `ui/`
are out of scope.

### `rerail-pack-package-names`

Flags code that spells out the rule pack package prefix (`@neralabs/rerail-pack-`, `rerail-pack-` and template literal
types built on it) to build or parse a pack's npm package name or install folder. It runs on `src/**/*.ts`, leaving out
the owner `src/engine/catalog/rule-packs.ts`, generated files and tests. Build Rerail's own names with
`rulePackPackageName` from that module; recognise a name and its id with `packIdOfPackage` or
`RULE_PACK_PACKAGE_NAME` beside it.

History: Built on #140, which added 8 restatements of the prefix (0 before it, 4 already on its branch); #340 took it
from 8 to 1, it stayed quiet on 15 recent merged PRs, and #373 fixed the last, the folder join in `rule-pack-store.ts`.

Gaps: A substitution that reads like words (`${id as string}`) is let through, and `rulePackPackageName` builds only
`@neralabs` names, so it is wrong for a third party's pack.

### `rerail-session-paths`

Flags code that reads `RERAIL_STORAGE_ROOT` or `RERAIL_MODEL_PATH` from the environment, rebuilds the default storage root
or the bundled model path (for example from `import.meta.url`), or guesses the default root from a folder name. It runs
on `src/**/*.ts`, leaving out tests, `src/engine/package-root.ts` and the owner `src/runtime/session-paths.ts`. Resolve
paths once with `resolveSessionPaths()`, `resolveModelPath()` and `isDefaultStorageRoot()` and pass them in; for another
checkout's default root, as scrub and the support bundle need, call `defaultStorageRoot(checkout)`.

History: Built on #19's own model path in the rule-draft code and #1's daemon copy, both dropped by the #26 fix, and on the
folder-name guess #221 merged. It is not quiet on recent merged work: #368's scrub and #377's support bundle each built
another checkout's `.rerail/sessions` by hand; this pull request routes both through `defaultStorageRoot`.

Gaps: Misses `process.env[CONSTANT]` and a default built in two `join` steps. May flag the variable passed through into
a child's environment (`RERAIL_STORAGE_ROOT: env.RERAIL_STORAGE_ROOT ?? x`) and a folder-name check on another path, such
as `basename(dirname(identityPath)) === ".rerail"`, which the message says to keep. A bare `bundledModelPath()` that skips
`RERAIL_MODEL_PATH` is not flagged, because #26 merged that shape as the fix.

### `rerail-start-state`

Flags code that turns a checkout's status into a start state (`if (!status.started) return "stopped"`), tests started
and set-up together, or decides from a start state whether Rerail runs (`["started", "not-set-up"].includes(…)`). It
runs on `src/**/*.ts`, leaving out the owner `src/runtime/repository-start.ts`, `src/engine/**`, `src/activation/**`,
tests and generated files. Use `stateOf` and the owner's decisions in `src/runtime/repository-start.ts`.

History: Built on the copies #23, #25 and #26 merged (a private `stateOf` copy, a host extension that had to change in
lockstep when `not-set-up` was added) and their first drafts; quiet on 16 recent merged PRs, and no fix exists to check.

Gaps: Log because the owner has no "runs here" predicate yet and engine code cannot import it; 7 matches are on main
today, one from #413's headless scan. Misses `status.started === false`, `=== true && … === true`, a `Record<RepositoryStartState, boolean>` table,
`answered === "stopped"` used as a decision, and the older `=== "started"` spelling of "runs here". Its patterns also
match in comments and strings, such as a two-element `z.enum(["started", "not-set-up"])`.

### `rerail-table-probes-in-storage`

Flags a query of `sqlite_master` or `sqlite_schema` that checks whether a table or column exists, and code that catches
a `no such table` error instead of asking first. It runs on `src/**/*.ts`, leaving out `src/engine/storage/**`,
`claude-status-store.ts`, tests and generated files. Put the probe in the storage layer, beside `missingSchemaObjects` in
`src/engine/storage/database.ts`, and call it from there.

History: Built on the probes in the first drafts of #25 and #45 and the one re-added in #21 after #23 deleted it and
caused `no such table: checkouts`; quiet on the #49 central fix and on 12 recent merged PRs.

Gaps: Log because storage exports no table check to call yet and maintainers merged and kept all three probes, two at a
review's request; 6 matches are on main today, in `audit-store.ts`, `setup-review.ts`, `repository-start.ts`,
`settings-entries-store.ts`, `scrub-cli.ts` and #413's `headless-scan.ts`. The probe in
`src/runtime/repository-start.ts` is needed (it covers a file whose creator has not committed the schema), so it must
move, not go; the message says so.

### `rerail-walk-limits`

Flags TypeScript that restates a repository walk's skipped folders (a list of `node_modules`, `.git`, `target` and the
like) or its file and byte limits. It runs on `src/**/*.ts` and `src/**/*.tsx`, leaving out the owner
`src/engine/code-rule/code-rule-enforcer.ts`, tests and generated files. Import `IGNORED_DIRECTORIES`, `AUDIT_MAX_FILES`,
`MAX_SOURCE_BYTES` and `AUDIT_MAX_SOURCE_BYTES` from that module; budgets for other inputs may differ.

History: Built on the walk copy that reached main in #1 and the first draft of #19's bound copies, both now replaced; quiet
on #26's moves and on 20 recent merged PRs from #140 to #354.

Gaps: Misses glob lists (`"**/node_modules/**"`), object-key sets, inline checks like `files.length >= 100_000` and
single-name sets used with `startsWith(".")`. May flag dependency checks and classifiers that name two of the folders
(`part === "node_modules" || part === "vendor"`), a regex literal naming them, names in strings, and other caches or
budgets with the same number; the message says to leave names not about walking a user's repository.

### `rerail-fixtures-root`

Flags Rust in `native/` that reaches the fixtures directory by a relative path (`../fixtures/…`, in `include_str!` and
the like) or through the oracles variable. It runs on `native/**/*.rs`, leaving out `native/build.rs`,
`native/src/code_rule_dsl/mod.rs` and `native/target/**`. Use the `fixtures_dir!` and `oracle!` macros, which follow the
fixtures directory override.

History: Built on #56 and the later breaks it caught in their first drafts (an async oracle, two pre-convention imports
and others); quiet on their fixes and on 16 recent merged PRs; #56's 18 relative lines stayed on main until they
were moved to `oracle!()`.

Gaps: Misses paths built in pieces (`push("..")`, `push("fixtures")`, or a root split across statements). May flag an
ordinary Rust string used as test input that happens to contain `../fixtures/`.

### `rerail-generator-placement`

Flags a script that writes files (Node `fs` write calls, Python `open(…, "w")` and `write_text`, shell redirects and the
like) when it sits in `packs/` or `native/src/` beside its output. It runs on scripts in those trees, leaving out
`packs/*/upstream/**`, fixtures and tests. Put generators that write committed files under `scripts/generate/`, as the
pack builders in `scripts/generate/packs/` are.

History: Built on the first draft of #41 and the engine generators #1 put in `native/src`; quiet on #41's merge, on #26's
move and on 15 recent merged PRs.

Gaps: Misses an aliased destructure (`const { writeFile: w } = fsp`), `fh.write` after `open(…, 'w')`, a computed
`fs['writeFileSync']`, a variable Python mode, `os.replace`, `SplFileObject`, `sed -i` and a relative redirect after
`cd`; the shell clause reports only a file's first write, and the `scripts/dsl` generators are out of scope. May flag a
script that writes only into a `mkdtemp` directory; the message says to leave temporary output.

### `rerail-pack-rule-test-placement`

Flags a test of one or a few code rules of a single pack that sits anywhere under `test/engine/` other than
`test/engine/code-rule/pack-rules/`. It runs on `test/engine/**/*.test.ts`, leaving out the pack-rules folder itself, the
node pack module-system tests and `test/engine/catalog/javascript-rule-options.test.ts`, which is not a pack-rule test.
Engine-feature, pack-wide and conversation-rule tests stay where they are.

History: Built on every flat pack-rule test from #43 to #73, #64, #106's duplicates and #67, and their branches, each fix
back to its base or below; quiet on 15 of 16 recent merged PRs, rising only on #177's JavaScript rule options test, now
excluded.

Gaps: None known.

### `rerail-rule-documents-in-packs`

Flags a rule draft anywhere, and a code or conversation rule document outside a pack's `rules/` or `candidates/` folder.
It runs on every `*.json`, leaving out `packs/*/rules/`, `packs/*/candidates/**/rules/`, `fixtures/packs/**`, the five
engine-test input folders under `fixtures/`, `node_modules`, `target`, `*.expected.json` and `native/grammars/**`. Engine
test inputs belong under `fixtures/`.

History: Built on the first drafts of #176 (`packs/*.draft.json` and its three-way split) and #183 (a rule draft in a host
fixture); quiet on both fixes and on 15 recent merged PRs.

Gaps: One scope covers all three checks, so a draft inside an exempt fixtures folder is not seen, and a new engine-test
input folder under `fixtures/` is flagged with a "may stay" message. A draft inside a pack's `rules/` folder is left to
pack validation. May flag a recorded API response shaped like a rule; the message says it may stay. Moving a
conversation rule into `candidates/rules/` needs pack validation's conversation-rule support.

### `rerail-pack-data-helper-copies`

Flags an upstream generator that defines its own copy of a shared pack-data helper (`ECMA_EXTENSIONS`,
`codePointColumn`, `splitLines` and the like), or that fakes the working directory itself (`process.chdir`, assigning
`process.cwd`, `defineProperty`). It runs on `.mjs`, `.js`, `.cjs` and `.ts` under `packs/*/upstream/`, leaving out
`packs/complexity/**`, tests, fixtures, publish output and samples. Import the helpers and the working-directory fake from
`scripts/pack-data/eslint-expected.mjs`.

History: Built on #31, which added the copies (3 to 8), and a working-directory pin that went 8 to 10 and back on its fix.

Gaps: Log because `ECMA_EXTENSIONS` and `splitLines` have no shared owner yet, so the unowned clause can only ask for the
copies to change together; 8 matches are on main today, in 4 JavaScript and TypeScript generators.

### `rerail-pack-data-runtime-names`

Flags a pack-data recipe that writes or composes a versioned tool runtime name (`lizard-${version}`,
`typescript-eslint-8.70.1`, a `runtimes/` path, concatenation or a prerelease suffix). It runs on `scripts/pack-data/recipes/**/*.ts`,
leaving out tests and fixtures. Take the runtime from the upstream runtime its gates manifest names (`upstream.runtime`),
so a version bump reaches it.

History: Built on the first draft of #179 (two breaks) and #338's head, which built a new name the day after #234 fixed
one; it drops on #234's fix and stays quiet on #177, the only other merged PR touching recipes. #373 fixed the last
two, in `python.ts` and `complexity.ts`.

Gaps: Not flagged by design: `rust.ts` installs its toolchain from `common.TOOLCHAIN`, not from the clippy runtime in the
gates manifest.

### `rerail-pack-data-through-publish`

A conversation rule on shell tool calls. It fires on a workstation upload of pack data to S3: `fixtures:push` through
`npm`, `pnpm`, `yarn` or `bun`; `pack-fixtures.mjs` with `push`, `upload` or `samples push`; `aws s3 cp`, `mv` or
`sync` into `s3://nera-test-fixtures/rerail/` or the bucket root; and `aws s3api` `put-object`, `copy-object` or a
multipart upload with `--bucket nera-test-fixtures --key rerail/…`. It reads through chains, `bash -c`, `env`, `sudo`,
`timeout`, `xargs`, `aws-vault exec`, `op run` and `VAR=` prefixes. It lets through a single `--dry-run` command (npm
needs `--` before it), `fixtures:pull`, `fixtures:stage`, `pack`, `samples` without `push`, downloads, `s3 ls`,
`put-object-tagging`, other bucket prefixes, `gh workflow run pack-data-publish.yml`, and commands that only write
text: `git commit`, `gh pr|issue|release create|edit|comment`, heredocs and a leading `rg`, `grep`, `sed` or `awk`.
Data goes through `fixtures:stage` into `publish/`, is committed, and a maintainer dispatches pack-data-publish.

History: Built on the direct pushes in #207, three rounds of #208 and #338; every one is caught as the command that made
it, the fixes and the bot publishes pass, and on 297 real session commands it fires only on the 7 real workstation
uploads.

Gaps: A real push chained after a `git commit` or heredoc in the same command is missed, as are
`samples --pack react push` and `npm exec -- node …pack-fixtures.mjs push`; a dry-run flag inside a shell comment
exempts the push. A push inside a quoted `echo` after a separator may fire. A command passed as an argv array is not
read. A maintainer's sanctioned S3 repair is steered too.

### `rerail-provenance-writers`

Flags writes of `provenance.json` or `LICENSES.md` outside the shared writer, writes of a project's
`licenses/<project>/UPSTREAM.md`, a second parser or list of its `Not copied` lines, and a hand-kept copyleft list.
Per-file provenance goes through `writeProvenance` in `scripts/pack-data/provenance.mjs` (Python: `write_provenance`),
`Not copied` paths are read with `readUpstreamSources` in `scripts/s3/pack-fixtures.mjs`, and copyleft samples live in
`scripts/samples/licences.ts`. It runs on `scripts/**`, `conformance/**` and `packs/*/upstream/**` (TypeScript,
JavaScript and Python); tests, fixtures, `publish/`, the owners, `scripts/upstream/bump.ts` and
`scripts/check/check-licenses.mjs` are out of scope. Reading a record, or writing a tool's own
`upstream/<tool>/UPSTREAM.md`, passes.

History: caught Ruff's own `Not copied` parser and the six stale licence-record writers that reached main after #177
(all gone at #234's merge), the node generator's own provenance writes on #207 and the hard-coded Ruff copyleft list on
#85; quiet on 16 recent merged PRs.

Gaps: Misses a path held in a variable declared in another function, Python `record = LICENSES / "UPSTREAM.md"` then
`record.write_text(…)`, a copyleft list under an unrelated name and `UPSTREAM.md` as a template literal. A new script
that moves the Source row with the pins, as `bump.ts` does, is steered. Sample-licence trust and a second licence
decider are not covered: no code shape tells the break from the fix.

### `rerail-workflow-script-facts`

Flags workflow facts kept by hand instead of read from their owners: built hook entry paths (read `ENTRY_POINTS` from
`dist/engine/package-root.js`), `paths-ignore` and `src`+`native` trigger lists that restate what core ships (use
`coreChanges` from `scripts/release/core-paths.mjs`), hand-listed tool-pack matrices, and tests on a pack or tool name
such as `matrix.pack == 'php'` where the pack's own data says what it needs. It runs on `.github/workflows/` and
`.github/actions/` YAML. `dist/cli/cli.js` and `dist/index.js`, a workflow's own inputs and the `'all'` placeholder
pass.

History: caught #1's hook paths (fixed by #51), the first draft of #85's hand-listed legs, #204's `paths-ignore` (fixed
by #211) and the PHP and Clippy name tests of #155 and #141; quiet on #51, #211, #177 and 16 of 18 recent merged PRs,
while #85's merge keeps its test-pinned nightly and recorder lists.

Gaps: Log until `upstream-bump.yml:161` stops testing `matrix.pack == 'php' || matrix.pack == 'blade'` (2 matches on
main today): no planning step emits the runtime a leg needs yet. Misses a trigger list by file type (`**/*.ts`,
`**/*.rs`); in a `src`+`native` list only the second core entry is reported; `inputs.pack == 'complexity'` on a rule
pack gets the generic advice. The repo pass skips hidden `.github/`, so these counts come from a copy of the tree;
live checks are unaffected.

### `rerail-workflow-shared-conditions`

Flags the second and later `github.head_ref != 'changeset-release/main'` release pull request skips in one workflow
file. Decide the skip once, as an output of a job the others need (`ci.yml`'s changes job), and read that output in
each job. It runs on `.github/workflows/` and `.github/actions/` YAML.

History: recorded #81's repeated skip across four jobs and its growth in #179; quiet on every other recent merged PR.

Gaps: Log until `ci.yml` decides the skip once; main has 6 matches today (`ci.yml`'s 2nd–7th skips), counted on a
copy of the tree because the repo pass skips hidden `.github/`. Misses a skip spelled
`github.event.pull_request.head.ref == 'changeset-release/main'` inside a `!( … )` group, and other repeated guards
(fork and Dependabot trust, `repository_owner`).

### `rerail-ui-api-precedence`

Flags dashboard code that ranks rule-setting owners, compares folder depth or puts rule before pack, and code that
builds a decider object (`{ by, folder, target }`) instead of showing the `…Source` deciders the rules API returns. It
runs on `ui/**/*.js`. Rendering a decider as text and copying `*Source` fields pass; the engine's precedence lives in
`src/engine/catalog/rule-configuration.ts`.

History: caught the first draft of #50 and each self-review round's new copy; #50's merge keeps the two constructs the
maintainers merged and kept, and #196's deletion of one drops a match; quiet on 14 other merged PRs that touched `ui/`.

Gaps: Log because the API cannot yet say which folders a setting would decide and the settings write returns no
deciders, so the only compliant change today would delete the optimistic update; main has 10 matches today, in
`packSettingDecides` (`ui/derive.js`) and the client-built `decider` (`ui/settings-writes.js`). An `if` that renders
text from `source.by` is flagged. Misses ranking under names it doesn't know and an optimistic update that drops the
decider.

### `rerail-ui-color-tokens`

Flags raw colours in the dashboard: hex values, colour functions, named colours, hex inside `data:` URIs, effect
tokens that are not black or white alphas, and redefinition of a Nera token. Use a Nera token from
`ui/tokens.generated.css` or a `base.css` page-local token built from them. It runs on `ui/**` CSS, JS, MJS and HTML;
`ui/tokens.generated.css` is out of scope, and the six named effects in `base.css` (`--lift`, `--scrim`, the shadows)
pass as black or white alphas.

History: caught #196's raw token commits and #209's override; quiet on #353's fix, #209's fix and 15 recent merged PRs
that touch `ui/`.

Gaps: Flags ids, fragments and private fields such as `#add` or `this.#fade`, an effect whose value continues on the
next line, `#000` in `mask-image`, `[fill="white"]` selectors and a quoted colour in a comment. Misses `COLOR:white`,
`style['background-color']`, named colours on continuation lines, and system colours such as `Canvas`; a raw `#` in a
`data:` URI gets the generic message.

### `rerail-ui-pure-modules`

Flags imports of I/O or DOM modules (`api.js`, `poll.js`, `shell.js`, `dom.js`, `app.js`, the writes modules,
`views/`, `render/`, `frame/`) and use of `fetch`, storage, `document`, `window`, `location` or `history` in the
dashboard's pure modules. Do that work in `poll.js`, a writes module or a view and pass the result in. It runs only on
the seven files `ui/README.md` lists as pure: `derive.js`, `first-open.js`, `pack-model.js`, `history-model.js`,
`sessions-model.js`, `settings-model.js` and `repo-rules.js`. Imports of `state.js`, `format.js`, `routes.js` and
`render/rule-sheet-model.js`, and shadowed names such as a local `document`, pass.

History: caught the first draft of `ui/repo-rules.js` in #196; that import was merged and kept; quiet on 13 other merged
dashboard PRs. There is no fix to gate on.

Gaps: Log because main has 2 matches today, `ui/repo-rules.js` importing `api.js` and `poll.js`, which the maintainers
kept; the file must be split or dropped from the pure list first. Flags `export default "./api.js"` and treats a
destructured `({ fetch }) =>` or `catch (fetch)` as the global. Misses bracket access such as `window["fetch"]`. The
scope is a hand copy of the README's list.

### `rerail-grammar-node-kinds`

Flags tree-sitter node kinds named in shared native engine code: a kind compared with `node.kind()` (`==`, `!=`), listed
in a `match` on a kind or in `matches!`, tested with `.ends_with("comment")` or `["…"].contains(&node.kind())`, or passed
to `kind_id`, `kind_ids` or `id_for_node_kind`. Kinds are per-grammar names, so shared code that names one branches on a
language (#373 review). Put them on the family's Engine Adapter (`Adapter::COMMENT_KINDS`, `PREAMBLE_KINDS` and
`BODY_KINDS`, from #400; `ScopeRules::REFERENCE_KINDS` and `NAME_KINDS`; `ValueRules::EXPRESSION_KINDS` and
`VALUE_KINDS`) and read them through `with_adapter!(Family::of(dialect), A => …)`. Code that only runs once its family is
chosen moves into the family's module or is named for the family.

It runs on `native/src/**/*.rs`. Left out:
- the adapter, `language/adapter.rs` and `language/adapter/**`;
- family-named modules: a folder or file named for a registry family, language or dialect (`python`, `ecma`, `php`,
  `blade`, `vue`, `rust`, `sql`, `javascript`, `typescript`, `tsx`), alone or joined to another word with `_`, and
  `phpdoc`. A rule cannot read `languages/*.json`, so the names are kept by hand;
- modules not named for a family that only run after it is chosen: `calls/summary.rs`, `value/structure.rs` and
  `literal/annotation.rs`, `offsets.rs` and `semantic.rs` (Python); `value/key.rs`, `module_graph.rs` and
  `import_target/` (Ecma); the linters' suppression readers in `suppress/`, each read by its family's `suppresses`;
  the literal grammars (`brace`, `percent`, `printf`, `scanf`, `eslint_directive`) and their `formatArgs` styles, whose
  kinds are Rerail's own;
- in `language.rs`, `matcher/arguments.rs`, `matcher/format_args.rs`, `matcher/resolve.rs` and `scope/usage.rs`, the
  functions that run only after dispatch, listed by name in one clause per file, and `matcher/arguments.rs`'s check
  behind the `estree` flag, which only Ecma accepts;
- functions named for a family (`fn prepare_python`, `fn php_holds`), and `match` arms on a family-named variant
  (`LiteralHost::PythonString`, `OwnKinds::Blade`);
- tests and `#[cfg(test)]` items.

Punctuation tokens (`","`, `"("`) and `ERROR` pass: tree-sitter names them the same in every grammar. The triage of
every match is in [HISTORY.md](HISTORY.md#rerail-grammar-node-kinds-grammar-node-kinds-only-on-the-familys-adapter).

History: Built on #50's `rerail-ignore` reader, whose comment, preamble and Python body kinds review flagged on #50 and
which merged unchanged; the #373 review answered that they are branching, and #400 moved them onto each family's
adapter. The rule matched those 3 lines before #400 and matches none of them since.

Gaps: Log until the 11 shared matches on main today move onto the adapter: `compile/resolve.rs`'s `parenthesized_expression`, `dotted_name`,
`attribute`, `scoped_identifier` and `this` (6), `matcher/arguments.rs`'s `named_children` and `unparenthesized`,
`matcher/compare.rs`'s `element_index` and `unparenthesized`, and `matcher/util.rs`'s `is_comment_kind`, each reached
for several families. Misses kinds held in a constant or variable before the comparison, field names
(`node.field("name")`), helpers taking one kind (`has_token(&node, "type")`) and a kind read under another name than
`kind`. A new after-dispatch function in one of the five listed files, or a new family module with a neutral name, is
flagged until it is listed.

### `rerail-language-facts-from-registry`

Flags hand-kept lists of file suffixes or language names in shared engine and host code: Rust arrays, or-patterns,
`matches!`/`vec!` and chains of `ends_with(".py")` holding two or more, match arms mapping a suffix to a language, and
TypeScript arrays, unions or objects in `src/engine/code-rule/`. Take them from the registry (`label_of_path`, the tables
`language.rs` includes; host `CODE_RULE_DIALECT_SUFFIXES`, `SOURCE_LANGUAGES`, `LANGUAGE_REGISTRY`). It runs on
`native/src/**/*.rs` and `src/**/*.ts`; the registry's `language/` folder, `build_languages.rs`, generated files,
tests, `#[cfg(test)]` items and a family's own modules (named for the family) are left out, as are linter-port and
resolver probe-order files.

History: caught both of #52's first drafts (gone at their fix) and #2's and #33's hand-kept tables; quiet on #52's merge
and on 17 recent merged PRs; #207's merge adds the real `by_package_type` table.

Gaps: Log until the `by_package_type` table comes from the registry; main has 3 matches today: the two
`by_package_type` arms in `language.rs`, and a judgement call in `calls/closure.rs`. Misses a
lone `path.ends_with(".py")`, `scripts/**`, and resolver tables kept as eslint-plugin-n's and Next's probe order. A new
family module with a neutral name is flagged until it is listed.

### `rerail-origin-ranges`

Flags calls to `host_range` or `host_span` and new `fn host_range`/`fn host_span` mappers in the native rule engine.
Place a range in its file through the document's `Origin` (`of_node`, `of_range`) or `InFile`; a new nested document
gets its own `impl Origin` beside its type. It runs on `native/src/code_rule_dsl/**/*.rs`; `origin.rs`,
`literal/offsets.rs`, `embed/segments.rs`, tests, `fn map_range` bodies and `impl Origin` blocks are left out.

History: caught the two hull callers sent back on #132's first draft and every later reintroduction on unmerged
branches; quiet on #132's merge and on 14 other recent merged PRs.

Gaps: Log because nothing on main has ever broken it; main has 0 matches today. Misses renamed mappers and hand offset
arithmetic such as `base + r.start`. Any function named `map_range` may map freely.

### `rerail-store-reads-in-documents`

Flags calls to the content store's source and tape reads (`get_source`, `get_sources_bulk`, `get_sources_by_hash_bulk`,
`get_tape`, `get_tape_with_status`, `get_tapes_bulk`, `file_documents_bulk`) outside the workspace document contract.
Load through a function in `workspace/documents.rs`, adding one there if none fits. It runs on `native/src/**/*.rs`;
`content_store/`, `workspace/documents.rs` and tests are left out. Id lookups such as `tape_by_source` pass.

History: caught the one mistake-shaped break, a copied `sql_work` source read, and went quiet on its fix; no rise on 14
recent merged PRs that touch `native/src`.

Gaps: Log because the planned facts graph stores and reads tapes on purpose, and a steer would undo that; main has 1
match today, the contract's own dropped-source fallback in `workspace/evaluate.rs`. Flags inline `#[cfg(test)]` code in
non-test files. Misses a call after `//` inside a string on the same line, and calls through a method alias or wrapper
trait.

### `rerail-walk-limits-native`

Flags Rust repository walks that restate the host's skipped folders (a list, `matches!`, match arm or comparison chain
pairing `node_modules` or `__pycache__` with another skipped name) or its limits (a max/limit/cap name set to `100_000`,
`32_000_000` or 512 MiB). Thread the host's `RepositoryIndexOptions` (`ignored_directories`, `max_files`,
`max_source_bytes`, `max_file_bytes`) into the walk. It runs on `native/src/**/*.rs`; test files are left out, and
budgets of other sizes pass.

History: caught the first draft of #52 that review sent back; its merge kept copies of the host's limits, so the merge
matches too; quiet on 19 other recent merged PRs.

Gaps: Log because the options do not reach the module graph walk yet, and threading them is a refactor; main has 3
matches today, `MAX_GRAPH_BYTES`, `MAX_GRAPH_ENTRIES` and `IGNORED_DIRECTORIES` in `module_graph.rs`. Misses
dependency checks such as `part == "node_modules" || part.starts_with('.')`, classifier arms, inline test loaders
written as `==` chains or bare `matches!`, and names in doc comments. The graph's 1 MiB per-file limit is not flagged.
