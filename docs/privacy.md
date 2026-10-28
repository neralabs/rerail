# Privacy

Rerail runs on your machine. This page says what it never does, what it sends over the network, and what it keeps on
your disk. It describes the code in this repository; the file named after each section is where to check it.

## What Rerail never does

- **No telemetry.** Rerail has no telemetry, analytics, crash reporting or usage metrics, and depends on no library
  that provides them (`package.json`).
- **Never sends your work anywhere.** Rerail itself never transmits your source code, prompts, coding-agent
  transcripts, rules or findings. Its only network traffic is listed below, and none of it carries them. The one
  thing it sends about your work is a steer feedback note you said yes to ([below](#steer-feedback)).
- **No hosted model.** Semantic matching uses an embedding model that ships inside the package
  (`model/`, `minishlab/potion-base-8M`), runs in-process, and is verified against its SHA-256 manifest before it
  loads. The engine is built with no HTTP or socket code (`native/Cargo.toml`, `native/src/model.rs`). Rerail calls no
  hosted LLM or inference API.
- **No silent updates.** A new version arrives only when you run `rerail update` or `rerail setup` (`src/setup/updater.ts`).
- **No pack install scripts.** Rule packs are installed and removed with `npm --ignore-scripts`
  (`src/setup/rule-pack-store.ts`).
- **No remote Git operations.** Every Git command Rerail runs is local, with hooks and fsmonitor disabled
  (`src/engine/git.ts`).
- **No third-party resources in the dashboard.** The local dashboard loads nothing from outside your machine (`ui/`).

## What goes over the network

Apart from the steer feedback notes you approve ([below](#steer-feedback)), all of it goes through `npm`, to the
registry your own npm configuration names, and carries package names and versions only (`src/runtime/npm.ts`):

| When | What npm fetches |
| --- | --- |
| `rerail update` (and `rerail update check`) | The latest version of `@neralabs/rerail` on your channel, then that version |
| `rerail setup` from a copy that is not the managed install (for example `npx`) | The managed install of `@neralabs/rerail` |
| Set-up, `rerail start`, `rerail packs list` and `rerail packs recommend` | Every Rerail rule pack, to match them against your repository **locally**; nothing about the repository is sent |
| `rerail packs add` and the packs you confirm in set-up | The packs you chose |
| When the SQLite binding fails its load check | `npm rebuild better-sqlite3`, which may fetch a prebuilt binary or Node headers through that package's own tooling |

Once installed, rules are evaluated without the network.

Signed customer profiles, which are in internal testing, add one more request: `rerail activate --code` sends the
activation code, a device public key, your platform and architecture, and the Rerail version to the activation URL
**you** supply. `rerail activate --file` is the offline alternative (`src/activation/customer-profile.ts`).

## Steer feedback

When a coding agent decides not to follow a steer, it records its one-line reason with `rerail feedback`
(`src/cli/feedback-cli.ts`). The note is kept in the database. With no standing answer, the agent is told to show you
that line word for word, to ask whether Rerail may send it, and not to answer for you: **yes** sends that note,
**always** sends it and every later one, **no** keeps every note on your computer, the ones approved but not yet sent
included, and stops the question. `rerail feedback sharing ask|always|never` changes the answer later; `never` also
holds back every note still waiting to go.

The answer is a command the agent runs, so it rests on the agent relaying your words. The agent cannot make a note
say something other than what the command prints back, and nothing is ever sent while the answer is `never`; but an
agent that disobeys its instructions could answer in your place. Check `rerail feedback sharing` if in doubt, and
set it to `never` to be sure.

A note you approved is sent over HTTPS to Nera's feedback API, `https://nera-rerail-feedback.azurewebsites.net/api/feedback`
(an Azure Function in Canada Central, whose storage stays in that region), by a background `rerail feedback flush`.
It carries the rule id, the rule's pack and pack version, the coding agent's name (such as `claude-code`), the reason,
the Rerail version, when the note was made and a random note id. It never carries the session, the repository, file
paths, code or anything else Rerail records; a reason holding control characters is refused. The reason is the agent's
own sentence, so it can name what it was doing: that is what you are asked to approve. Nera's function stores only
those fields; its platform request logs keep the region a request came from, not the note. `RERAIL_FEEDBACK_URL`
(for testing) sends notes elsewhere: it must be `https`, or `http` on this computer, and the agent is told, and tells
you, where a yes would send the note.

## Your coding agent's provider

Rerail steers your coding agent by adding messages to its session. Your agent sends its whole session, Rerail's
messages included, to its own model provider on its next request, under that provider's terms. Those messages can hold
file paths and line numbers, a rule's message (which may quote the matched code), a suggested fix, and the text of
rules you wrote. Rerail does not make that request; your agent does.

## What Rerail keeps on your machine

Rerail stores what it needs to evaluate rules and show you their history. None of it is encrypted at rest.

| Where | What |
| --- | --- |
| `~/.rerail/` (or `$RERAIL_HOME`) | `rerail.sqlite`: rules, repositories, sessions, audits and findings, steer feedback notes, and every event your coding agents report: your prompts, the agent's replies, and tool calls with their inputs and outputs. Also the daemon's state and log, your consent answers (`consent.json`: reading sessions, sharing feedback), installed rule packs and managed installs |
| `~/.cache/rerail/` | `content-v1.sqlite`: compressed copies of the source files Rerail has read, with their syntax trees and check results |
| `<your repository>/.rerail/sessions/` | One ledger per coding-agent session: its events, and the text of files as they were before the agent edited them. The folder holds a `.gitignore` of `*`, so Git never shows or commits it |

`~/.rerail/`, the database, the daemon's state and log files and the session ledgers are created readable by your
user only (directories `0700`, files `0600`).

## The local dashboard and API

The daemon serves the dashboard, its API and an MCP endpoint for your coding agents on `127.0.0.1:4737` by default
(`src/daemon/daemon.ts`). It refuses to listen on any other address unless you pass `--allow-remote`. It answers
`421` to any request whose `Host` header is not loopback or a name you allowed, which blocks DNS rebinding, and `403`
to a cross-origin write to its API (`src/api/http.ts`).

The API has no login: any program running on your machine can call it, as it can read your files.

## Deleting your data

- `rerail reset`, run inside a repository, deletes that repository's recorded history (sessions, events, steer
  feedback notes, audits) from the database. It keeps rules and settings, and does not touch `.rerail/sessions/`, the
  content cache or the daemon log. A note already sent to Nera is not recalled.
- `rerail install --remove` disconnects every coding agent, stops the daemon and takes Rerail off your `PATH`. It
  **moves** the database to a backup folder, `~/.rerail-clean-YYYYMMDD`, rather than deleting it.
- To remove everything: run `rerail install --remove`, then delete `~/.rerail`, `~/.rerail-clean-*`,
  `~/.cache/rerail` and the `.rerail/` folder of each repository you used Rerail in.

Questions about this page: [open an issue](https://github.com/neralabs/rerail/issues). Report a vulnerability as
[SECURITY.md](../SECURITY.md) describes.
