# Privacy

Rerail runs on your machine. Each claim below names the file in this repository that implements it.

## What Rerail never does

Rerail sends nothing about you or your work unless you add a remote, and calls no hosted model.

- **No telemetry**, analytics, crash reporting or usage metrics.
- **No sending your work** except to a [remote](remote-sink.md) you add.
- **No hosted model or LLM API.** Semantic matching runs a bundled model in-process.
- **No silent updates**, no pack install scripts, no remote Git, no third-party dashboard resources.

<details>
<summary>Details and source files</summary>

- **No telemetry**, analytics, crash reporting or usage metrics, and no dependency providing them (`package.json`).
- **No sending your work.** Code, prompts, agent transcripts, rules and findings go only to a [remote](remote-sink.md)
  you add (in internal testing; none by default). Rerail's other network traffic, listed below, carries none of them.
- **No hosted model or LLM API.** Semantic matching uses a bundled embedding model (`model/`,
  `minishlab/potion-base-8M`), run in-process after a SHA-256 manifest check. The engine has no HTTP or socket code
  (`native/Cargo.toml`, `native/src/model.rs`).
- **No silent updates.** Only `rerail update` or `rerail setup` installs a version (`src/setup/updater.ts`).
- **No pack install scripts.** Packs are installed and removed with `npm --ignore-scripts`
  (`src/setup/rule-pack-store.ts`).
- **No remote Git.** All Git commands are local, with hooks and fsmonitor disabled (`src/engine/git.ts`).
- **No third-party dashboard resources.** The dashboard loads nothing from outside your machine (`ui/`).

</details>

## What goes over the network

Only `npm` traffic to your configured registry, carrying package names and versions; rule evaluation needs no network.

npm runs for updates, setup, listing and adding packs, and SQLite rebuilds (`src/runtime/npm.ts`).

<details>
<summary>When npm runs</summary>

| When | What npm fetches |
| --- | --- |
| `rerail update`, `rerail update check` | Latest `@neralabs/rerail` on your channel, then that version |
| `rerail setup` from a non-managed copy (e.g. `npx`) | The managed `@neralabs/rerail` install |
| Set-up, `rerail start`, `rerail packs list`, `rerail packs recommend` | Every rule pack, matched to your repository **locally**; nothing about it is sent |
| `rerail packs add`, packs confirmed in set-up | The chosen packs |
| SQLite binding fails its load check | `npm rebuild better-sqlite3`; may fetch a prebuilt binary or Node headers through that package's own tooling |

</details>

Two internal-testing features also send to endpoints **you** supply.

<details>
<summary>Remote sink and signed customer profiles</summary>

- **Remote sink:** `rerail remote add` targets get sessions, events (payloads unless `--no-content`), audits,
  violations, their rules and headless scans, minus excluded repositories. With no remote, it sends nothing. See
  [Remote sink](remote-sink.md) for what it sends and where its token is kept (`src/runtime/remote-outbox.ts`).
- **Signed customer profiles:** `rerail activate --code` sends the code, a device public key, platform, architecture
  and Rerail version to the activation URL **you** supply. `rerail activate --file` is offline
  (`src/activation/customer-profile.ts`).

</details>

## Your coding agent's provider

Steers reach your agent's model provider through your agent, never through Rerail.

Rerail steers by adding messages to the agent's session. The agent's next request sends that session to its provider,
under the provider's terms. Steers can hold paths, line numbers, rule messages quoting matched code, suggested fixes
and your rules' text.

## What Rerail keeps on your machine

Rerail stores rules, history and every agent event unencrypted, in `~/.rerail/`, `~/.cache/rerail/` and each
repository's `.rerail/sessions/`.

<details>
<summary>Storage locations</summary>

| Where | What |
| --- | --- |
| `~/.rerail/` (or `$RERAIL_HOME`) | `rerail.sqlite`: rules, repositories, sessions, audits, findings, steer feedback, and all agent events (prompts, replies, tool inputs and outputs). Also daemon state and log, consent answer, rule packs, managed installs |
| `~/.cache/rerail/` (Windows: `%LOCALAPPDATA%\rerail`) | `content-v1.sqlite`: compressed source files Rerail read, with syntax trees and check results |
| `<your repository>/.rerail/sessions/` | One ledger per session: events and pre-edit file text. Its `.gitignore` of `*` keeps it out of Git |

Steer feedback is the agent's one-line reason for ignoring a steer (`rerail feedback`). It is sent nowhere;
`rerail feedback list` prints it for an issue.

</details>

### Secrets

Rerail redacts recognized secrets before they reach disk and skips output of secret-like files; unknown formats can
still be stored.

`rerail scrub` redacts older history, then rewrites the database file.

<details>
<summary>How redaction works</summary>

Redaction covers stored events, session ledgers, findings and `daemon.log`. Rules and steers still see the original
text in memory (`src/engine/secret-scrub.ts`).

- **Known formats** (cloud, Git host, chat, payment and AI keys and tokens; private keys; JWTs; URL passwords;
  `Authorization` credentials) become `[redacted:<kind>:<6 hex>]`. The hex starts the secret's SHA-256, so a secret
  always gets the same marker.
- **Secret-like keys** (`password`, `api_key`, `token`) with long, random values become `[redacted:<kind>]`, unhashed
  since they may be passwords.

Hashes, UUIDs, lockfile integrity strings and code are kept. Secrets in unknown formats can still be stored.
`rerail scrub` covers the database, its pre-update copy, recorded checkouts' ledgers and `daemon.log`.

</details>

<details>
<summary>Secret-like files</summary>

Secret-like file names match `SENSITIVE_FILE_NAMES` in `src/engine/path-boundary.ts` (`.env`, `id_rsa`,
`secrets.yaml`) or sit in folders like `.ssh`. Hidden names alone, like `.github/`, are not.

- **Tool output** from a call naming such a path (`cat .env`) is not stored. The database keeps the omission and byte
  size, the ledger the sample without text, and the call itself is kept. Paths built at run time are not seen.
- **Pre-edit text** of such a file, or one holding a recognized secret, is kept only as SHA-256 and size. A session
  resumed from the ledger cannot rewind it.

</details>

### Permissions

Rerail's files are private to your user: directories `0700`, files `0600`.

<details>
<summary>What is covered and repaired</summary>

This covers `~/.rerail/`, `~/.cache/rerail/`, the databases, daemon state, capability and log files, and ledgers.
SQLite side files inherit the database's mode.

On macOS and Linux, Rerail removes group and other access from these when it opens one your user owns, fixing old
releases or loose umasks. Covered: home and cache folders, `rerail.sqlite*` (pre-update copies too),
`content-v1.sqlite*`, `harness.sqlite*`, `daemon.json`, `access-token`, `daemon.log`. It never follows symlinks or
touches other users' files.

</details>

### Symlinks

Rerail refuses to write session records through a symlink at `.rerail`, `.rerail/sessions` or anything below.

<details>
<summary>Why, and the `.gitignore` exception</summary>

A repository controls its own `.rerail/` folder, so this stops it redirecting your records. Rerail names the path in
the error and log; the agent continues without Rerail.

Exception: a symlinked or edited `.rerail/sessions/.gitignore` is overwritten with `*`, so a committed `!*` cannot make
`git add -A` commit your records.

</details>

## The local dashboard and API

The daemon serves the dashboard, API and an MCP endpoint on loopback, behind an access capability.

- **Address:** `127.0.0.1:4737` by default; other addresses need `--allow-remote` (`src/daemon/daemon.ts`).
- **Capability:** a random value readable only by you; your browser gets it from the link `rerail daemon start` prints
  or `rerail manage` opens.

<details>
<summary>Status codes, capability scope and remote exposure</summary>

- **`421`** for a non-loopback `Host` you did not allow, blocking DNS rebinding.
- **`403`** for cross-origin API writes (`src/api/http.ts`).
- **`401`** without the capability (`access-token` in `~/.rerail`, and `daemon.json`).

The browser keeps the capability in that page's storage, unreadable by other sites or local servers.

This blocks other users and sandboxed programs that reach localhost from reading your history or changing your rules.
It does not block programs running as you; they can read the file.

`--allow-remote` or `--public-host` exposes the daemon with a fresh capability per start. It serves HTTPS with
`--tls-cert` and `--tls-key`, and refuses plain HTTP without `--insecure-http`. Rerail's clients verify the
certificate's recorded fingerprint before sending the capability.

</details>

## Support bundles

`rerail support-bundle` writes one local file for an issue, only when run, and sends nothing (`src/cli/support-bundle.ts`).

It leaves out prompts, replies, tool data, source code and rule text. Read it before attaching
([what it holds](troubleshooting.md#reporting-a-problem)); issues are public.

<details>
<summary>What the bundle holds</summary>

- **Included (an allowlist):** versions, Rerail state, rule ids and modes, session timeline by event kind and tool,
  delivered steers, violations by rule and line, the daemon log's tail.
- **Never read:** prompts, replies, tool input and output, source code, rule text.
- **Replaced:** capabilities, home folders, recorded repositories' paths and names, user and computer names. Apparent
  secrets and emails are removed.

Other absolute paths remain, patterns miss some secrets, and steers can quote code.

`/rerail-bug-report` reads the file with your agent, so it reaches the agent's provider
([above](#your-coding-agents-provider)). To find an existing issue, it searches GitHub with a shipped rule's id or
error words, never your rule ids, paths or project names. Nothing is filed until you submit the form or ask it to.

</details>

## Deleting your data

Three commands remove data, each reaching further.

- **`rerail reset`** deletes the current repository's history.
- **`rerail install --remove`** uninstalls Rerail and **moves** the database to a backup.
- **`rerail install --remove --delete-data`** deletes the databases, backups, cache and recorded ledgers.

<details>
<summary>Exactly what each command removes</summary>

- **`rerail reset`** in a repository deletes its history (sessions, events, steer feedback, audits) and rewrites the
  database so it cannot be recovered. Rules, settings, `.rerail/sessions/`, the content cache and daemon log stay. All
  writes zero freed space (`secure_delete`).
- **`rerail install --remove`** disconnects agents, stops the daemon, removes Rerail from `PATH`, and **moves** the
  database to `~/.rerail-clean-YYYYMMDD`.
- **`rerail install --remove --delete-data`** deletes the databases, every `~/.rerail-clean-*` backup, the content cache
  and recorded checkouts' `.rerail/sessions/`. `--dry-run` previews. Unrecorded checkouts and a custom
  `RERAIL_STORAGE_ROOT` are not reached.

</details>

Questions: [open an issue](https://github.com/neralabs/rerail/issues). Report vulnerabilities per
[SECURITY.md](../SECURITY.md).
