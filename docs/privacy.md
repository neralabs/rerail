# Privacy

Rerail runs on your machine. Each claim below names the file in this repository that implements it.

## What Rerail never does

Rerail sends nothing about you or your work unless you add a remote or turn the experimental Merge Assist on, and calls
no hosted model.

- **No telemetry**, analytics, crash reporting or usage metrics.
- **No sending your work** except to a [remote](remote-sink.md) you add.
- **No hosted model or LLM API.** Semantic matching runs a bundled model in-process.
- **No silent updates**, no pack install scripts, no remote Git, no third-party dashboard resources.
- **One request on its own:** once a day, an update check. Unless you add a remote or turn Merge Assist on, no other
  network request runs without you asking, except an npm rebuild of the SQLite binding when it fails to load.

<details>
<summary>Details and source files</summary>

- **No telemetry**, analytics, crash reporting or usage metrics, and no dependency providing them (`package.json`).
- **No sending your work.** Code, prompts, agent transcripts, rules and findings go only to a [remote](remote-sink.md)
  you add (in internal testing; none by default). Rerail's other network traffic, listed below, carries none of them.
- **No credentials of its own.** Rerail makes no network request with credentials of its own. Where it runs a tool
  that reaches the network for you (`npm`, and with Merge Assist on, `gh`), that tool uses your own configuration and
  sign-in, and Rerail keeps no token. For its own installs Rerail gives npm a copy of your npm user config (`~/.npmrc`)
  without any `allow-scripts` setting, in a private temporary folder it removes when npm ends; a token in that file is
  copied with it, for npm alone to read.
- **No hosted model or LLM API.** Semantic matching uses a bundled embedding model (`model/`,
  `minishlab/potion-base-8M`), run in-process after a SHA-256 manifest check. The engine has no HTTP or socket code
  (`native/Cargo.toml`, `native/src/model.rs`).
- **No silent updates.** Only `rerail update` or `rerail setup` installs a version (`src/setup/updater.ts`).
- **One request on its own.** Once a day the daemon of an installed Rerail asks npm for the newest `@neralabs/rerail`
  on your update channel, so the dashboard can say a newer version is out. It installs nothing and sends nothing about
  your work: the request carries what any npm request does, npm's user agent (npm and Node versions, operating system,
  CPU) and the credentials your npm config holds for the registry. `RERAIL_UPDATE_CHECK=0` set for the daemon turns it
  off (`src/setup/update-check.ts`).
  Every other network request runs because you ran a command, added a remote or turned Merge Assist on (both in
  internal testing, below), with one exception: when the SQLite binding fails its load check, starting the daemon
  rebuilds it with npm (below).
- **No pack install scripts.** Packs are installed and removed with `npm --ignore-scripts`
  (`src/setup/rule-pack-store.ts`).
- **No remote Git.** All Git commands are local, with hooks and fsmonitor disabled (`src/engine/git.ts`).
- **No third-party dashboard resources.** The dashboard loads nothing from outside your machine (`ui/`).

</details>

## What goes over the network

Apart from Merge Assist (below), only `npm` traffic to your configured registry, carrying package names and versions;
rule evaluation needs no network.

npm runs for the daily update check, updates, set-up, recommending and adding packs, and SQLite rebuilds
(`src/runtime/npm.ts`).

<details>
<summary>When npm runs</summary>

| When | What npm fetches |
| --- | --- |
| Once a day, in the daemon of an installed Rerail, unless `RERAIL_UPDATE_CHECK=0` | The newest `@neralabs/rerail` version on your channel; nothing is installed |
| `rerail update`, `rerail update check` | Latest `@neralabs/rerail` on your channel, then that version and this computer's engine package, `@neralabs/rerail-engine-<suffix>`, at the same version |
| `rerail update`, `rerail packs update` | The newest version of each rule pack you installed, asked by its package name on your channel (on `next` for a pack the channel has no version of), then the newest of those that fits the active core |
| `rerail setup` from a non-managed copy (e.g. `npx`) | The managed `@neralabs/rerail` install, with this computer's engine package, `@neralabs/rerail-engine-<suffix>` |
| Set-up, `rerail start`, `rerail packs recommend` | Every rule pack, matched to your repository **locally**; nothing about it is sent |
| `rerail packs add`, packs confirmed in set-up | The chosen packs |
| SQLite binding fails its load check, when the daemon starts or during an update | `npm rebuild better-sqlite3`; may fetch a prebuilt binary or Node headers through that package's own tooling |

</details>

Two internal-testing features also send to endpoints **you** supply. Both are experimental and off by default
(`RERAIL_EXPERIMENTAL_REMOTE_SINK`, `RERAIL_EXPERIMENTAL_ACTIVATION_PROFILES`; [Internal testing](internal-testing.md)).

<details>
<summary>Remote sink and signed customer profiles</summary>

- **Remote sink:** `rerail remote add` targets get sessions, events (payloads unless `--no-content`), audits,
  violations, their rules and headless scans, minus excluded repositories. With no remote, or with the remote sink off
  in the daemon, it sends nothing. See
  [Remote sink](remote-sink.md) for what it sends and where its token is kept (`src/runtime/remote-outbox.ts`).
- **Signed customer profiles:** `rerail activate --code` sends the code, a device public key, platform, architecture
  and Rerail version to the activation URL **you** supply. `rerail activate --file` is offline
  (`src/activation/customer-profile.ts`).

</details>

Merge Assist, which is experimental and in internal testing, runs only with `RERAIL_EXPERIMENTAL_MERGE_ASSIST=1` set
for the Rerail daemon ([Internal testing](internal-testing.md#merge-assist)), and then only in a repository you turned
it on for. It adds read-only requests to GitHub through **`gh`**, with your own `gh` sign-in
(`src/runtime/merge-assist/github.ts`). Without the variable, or while it is off for a repository, Rerail runs no `gh`
for it:

| When | What `gh` reads |
| --- | --- |
| Merge Assist reads its readiness (when you turn it on; at a scan, before it lists open pull requests below; before another listing and at a status read when none was read for that process's `gh` configuration folder and whether `GH_TOKEN` or `GITHUB_TOKEN` is set, or when the last reading is five minutes old, one minute while `gh` cannot tell; at a status read when `gh`'s sign-in file changed; and after a listing whose sign-in file changed or did not answer its stat in time, or that listed nothing since `gh` was not signed in, which then lists again with the new reading) | Your sign-in state and login |
| A Merge Assist scan (at session start and on demand), and detection after your agent's edits (at most every five minutes) | The 200 newest open pull requests of the repository: number, title, URL, author, draft state, last update, head and base branch, head repository owner and whether it is a fork, and head commit |

Rerail runs only `gh auth status`, `gh pr list` and `gh api` with `GET`; it never writes to GitHub. A repository whose
remote is not on GitHub is not read at all; its recently committed remote-tracking branches stand in for pull requests.

## Your coding agent's provider

Steers reach your agent's model provider through your agent, never through Rerail.

Rerail steers by adding messages to the agent's session. The agent's next request sends that session to its provider,
under the provider's terms. Steers can hold paths, line numbers, rule messages quoting matched code, suggested fixes
and your rules' text.

With Merge Assist on, its notes about in-flight conflicts reach your agent's provider the same way. They can hold the
other side's label, author and one-line summary (a commit subject or pull request title), the contested file names,
and what your conflict hook command answered.

## What Rerail keeps on your machine

Rerail stores rules, history and every agent event unencrypted, in `~/.rerail/`, `~/.cache/rerail/` and each
repository's `.rerail/sessions/`.

<details>
<summary>Storage locations</summary>

| Where | What |
| --- | --- |
| `~/.rerail/` (or `$RERAIL_HOME`) | `rerail.sqlite`: rules, repositories, sessions, audits, findings, steer feedback, and all agent events (prompts, replies, tool inputs and outputs). With Merge Assist on, also its in-flight conflicts (both sides' summaries and contested hunks; a dismissed one keeps everything but its hunks and evidence: its title, files, each side's branch or pull request, commit, author, link, summary and location, who dismissed it, and the hook command's last answer: its note (up to 600 characters), status line and any error (up to 300 characters of the command's output)), its last scan, your `gh` sign-in state and login, which notes each session was handed, and your Merge Assist settings; turning Merge Assist off clears the conflicts, scans and `gh` sign-in state, and `rerail reset` clears the notes of the sessions it removes. Also daemon state and log, consent answer, rule packs, managed installs |
| `~/.rerail/claude-status-lines/` | One file per Claude Code session, named by its session id: the status line the daemon last drew for it. That is its rule and steer counts, any problem state, the newest open Merge Assist conflict's line, and dashboard links naming the repository's key and the checkout's path. No prompts, replies or code |
| `~/.rerail/untaken-events/` | One file per agent event a hook left while the daemon ran but did not read: the event as the database keeps it (a prompt, a reply, a tool's input), its recognized secrets redacted and a tool's output left out. At most 256 events and 16 MiB. Each file goes once the daemon reads again and takes it, or gives it up after three passes over a minute could not read it. A file a hook left part written as it ended goes at a later pass of the daemon. A file the daemon cannot remove stays until it can, and `daemon.log` names it once; on Windows, a file the daemon may not remove, or one another program holds open, stays without a line in `daemon.log`. Also two marks: the newest event taken and the folder's id |
| `~/.rerail/rerail.sqlite-receipts/` | One file per steer whose hook handed its receipt to the daemon while another process held the database: the session's id, host and folder, the receipt's id and the rules it credits. Each goes once its record is in `rerail.sqlite`; the daemon's next start records any left. No prompts, replies or code |
| `~/.cache/rerail/` (Windows: `%USERPROFILE%\AppData\Local\rerail`) | `content-v5.sqlite`: source files Rerail read, with syntax trees and check results. Sources and syntax trees from a scan are kept uncompressed in the `content-v5.sqlite.objects` folder beside it, and the facts derived from them in the `.frames` and `.memo` folders. `content-v5.sqlite.files` records the paths, sizes and times of the files Rerail looked at, with their contents' digests, so it can tell an unchanged file without reading it. On Windows, the copy an earlier release kept in `~/.cache/rerail` stays until `rerail install --remove` deletes it. Rerail 0.1.5 and earlier kept the source files it read, compressed, with syntax trees and check results, in `content-v1.sqlite` (with `-wal` and `-shm`): 0.2.0 no longer reads it and leaves it in place, so delete it yourself, or `rerail install --remove` deletes it with the rest of the content cache |
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
`rerail scrub` covers the database, its pre-update copy, recorded checkouts' ledgers and held steers, and `daemon.log`.
In a records folder it may not list, it covers only the sessions the database records, and its summary names that
folder.

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

Rerail's files are private to your user: directories `0700`, files `0600`. Windows has no such modes: a Rerail home
Rerail makes outside your profile folder, and a checkout's `.rerail\sessions` it makes there, get an access list that
only you, SYSTEM and Administrators are on. A `.rerail\sessions` that Git, you or Rerail 0.1.4 made gets one when the
daemon first opens a session there under which each record is its maker's alone, those already in it included but
another account's earlier ones, which keep the list they had: an
account you share the checkout with still records its own sessions there, but lists nothing in it and reads no one
else's records. What a session writes in its own folder is private from its first byte. Elsewhere, such a folder loses the group's and others' read then, and
becomes sticky where they may still add to it (`2775` becomes `3731`, `0777` `1733`, `0755` `0711`): no other account
lists it, one you share it with still records its own sessions there, and none can rename or remove your entries. A root
`RERAIL_STORAGE_ROOT` names keeps the access list or mode it has.

<details>
<summary>What is covered and repaired</summary>

This covers `~/.rerail/`, `~/.cache/rerail/`, the databases, daemon state, capability and log files, and ledgers.
SQLite side files inherit the database's mode.

On macOS and Linux, Rerail removes group and other access from these when it opens one your user owns, fixing old
releases or loose umasks. Covered: home and cache folders, `rerail.sqlite*` (pre-update copies too),
`content-v5.sqlite*` (the `.objects`, `.frames` and `.memo` folders are made private to you, and the `.files` database is created private), `harness.sqlite*`, `daemon.json`, `access-token`, `daemon.log`, `claude-status-lines/`. It never follows symlinks or
touches other users' files.

</details>

### Symlinks

Rerail refuses to write session records through a symlink at `.rerail`, `.rerail/sessions` or anything below.

<details>
<summary>Why, and the `.gitignore` exception</summary>

A repository controls its own `.rerail/` folder, so this stops it redirecting your records. Rerail names the path in
the error and log; the agent continues without Rerail.

Exception: a symlinked or edited `.rerail/sessions/.gitignore` is overwritten with `*`, so a committed `!*` cannot make
`git add -A` commit your records. One Rerail cannot overwrite stops the session's records instead. Rerail writes it
readable by every account (`0644`), since it holds only `*`; one another account's Rerail wrote and you may not read
is left as it is when that account also owns the `sessions` folder, and rewritten when you own the folder.

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
certificate's recorded fingerprint before sending the capability. A `--tls-key` other accounts can read is never
served with: the daemon then serves this computer alone, over plain HTTP on loopback, and a start that asked for
`--allow-remote` or `--public-host` exits 1 ([reference](reference.md#network-binding-and-tls)).

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
- **`rerail install --remove --delete-data`** deletes the databases, backups, cache, recorded ledgers and Rerail home.

<details>
<summary>Exactly what each command removes</summary>

- **`rerail reset`** in a repository deletes its history (sessions, events, steer feedback, audits) and rewrites the
  database so it cannot be recovered, and deletes the Claude Code status line kept for each session it deletes. Rules,
  settings, `.rerail/sessions/`, the content cache and daemon log stay. All writes zero freed space (`secure_delete`).
- **`rerail install --remove`** disconnects agents, stops the daemon, **moves** the database to
  `~/.rerail-clean-YYYYMMDD`, removes Rerail from `PATH`, and deletes the installation, the rule packs, the Claude
  Code status lines and the content cache.
- **`rerail install --remove --delete-data`** deletes the databases, every `~/.rerail-clean-*` backup, the content cache,
  recorded checkouts' `.rerail/sessions/`, the device secrets of `rerail activate` profiles, and everything else Rerail
  wrote in its home, then the home. `--dry-run` previews. Unrecorded checkouts, a custom `RERAIL_STORAGE_ROOT` and the
  `<file>.<timestamp>.bak` copies of agent configs are not deleted; the output names each with the command that does.

What each leaves behind: [Removing Rerail in detail](how-setup-works.md#what-each-removal-leaves-behind).

</details>

Questions: [open an issue](https://github.com/neralabs/rerail/issues). Report vulnerabilities per
[SECURITY.md](../SECURITY.md).
