# Remote sink

> [!WARNING]
> In [internal testing](internal-testing.md): in the code but not ready for public use, and may change or go without
> notice.

A remote is an HTTPS endpoint that receives this machine's Rerail data: sessions, events, audits and their
violations, the rules they ran, and [headless scans](headless-scan.md). It lets a team see Rerail across machines.
**Rerail has no remote until you add one, and with none nothing leaves the machine.** Rerail ships no remote of its
own; you point it at a service that implements the protocol below.

```sh
rerail remote add <url> [--name <name>] [--label <machine label>] (--token-stdin | --token-file <file>) [--no-content]
rerail remote list [--json]
rerail remote remove <name|url>
rerail remote status [--json]
rerail remote push [--json]
rerail remote exclude [--repo <dir>]
rerail remote include [--repo <dir>]
```

- **`remove`** deletes the remote's token and what the outbox knew of it: a remote added later under the same name
  starts from nothing and receives everything again. A send already under way when the remote is removed records
  nothing and stops; every `add` gives the remote a new id, which the outbox keeps its progress under.
- **`add`** takes the URL (HTTPS; plain HTTP only to `localhost`) and the bearer token the service gave this machine,
  from standard input or a file, never on the command line. The name defaults to the URL's host. A new remote
  receives what the database already holds first, then everything new.
- **`--no-content`** sends events without their payloads (prompts, tool input and output): each event says
  `"content":"omitted"`. Everything else is sent.
- **`exclude`** opts a repository (the one at `--repo`, or the current folder's) out of every remote: nothing of it
  that a remote does not have yet is sent, whether it was recorded before the `exclude` or after. `include` opts it
  back in **from that moment**: only what the repository records after `include` is sent. What it recorded until then
  is never sent, even if no send ran while it was excluded; a session that started then stays unsent with all its
  events and audits. Exclusions are kept, with when `include` ended them, in `remote.json`.
  - A repository is matched under every key it has had: its current key and its aliases (an older clone id, a remote
    it used to have).
  - A session recorded with no repository is placed by its folder's recorded checkout. One that no checkout places
    is held back while any repository is excluded, and sent once none is.
- **`status`** (and a line per remote in `rerail status`) shows when the remote last took a batch, what is waiting and
  the last error. It exits 1 while a remote is failing.
- **`push`** sends everything waiting now, in this process, without the daemon and without waiting for a retry: for
  CI and for hosts that do not run the daemon.

While a remote is set up, set-up's question about reading your sessions says they are sent, to which host, and how
to stop, instead of "Stays on this computer".

The daemon sends on its own every 10 seconds, on its own connections; a remote that is slow or down never holds up a
session. `rerail remote add` and `remove` take effect without a restart.

## On disk

| File in the Rerail home | Holds |
| --- | --- |
| `remote.json` | The remotes (name, id, URL, whether content is sent, label), and the repositories' exclusions. |
| `remote-tokens/<name>` | That remote's token, alone. Never in the database, which support bundles and `rerail scrub` read, and never printed. |
| `remote-outbox.sqlite` | What each remote has acknowledged (an event cursor, a hash per sent record), retry state, and queued scans. Deleting it makes every remote receive everything again. |
| `machine-id` | A random UUID made once, which names this machine to every remote. |

All four are readable by you only (files `0600`, the token folder `0700`), like the rest of the home. The token is
stored in a plain file, not the system keychain: the daemon reads it unattended.

## Protocol

`POST <url>/v1/batches`, `Authorization: Bearer <token>`, `Content-Type: application/x-ndjson`. One batch is at most
5 MB; the first line is the batch header, each following line one record:

```jsonc
{"type":"batch","protocol":1,"batchId":"<uuid>","machineId":"<uuid>","machineLabel":"<hostname>","rerailVersion":"0.1.0","sentAt":1793732129819}
{"type":"repository","id":"remote:github.com/org/repo","displayName":"repo","aliases":[]}
{"type":"session","id":"<session id>","host":"claude-code","cwd":"/abs","repositoryKey":"…","startedAt":…,"endedAt":…|null}
{"type":"event","id":"<machineId>:<sequence>","sessionId":"…","eventId":"…","kind":"user-prompt","observedAt":…,"sequence":42,"payload":{…}}
{"type":"rule","id":"sha256:…","ruleId":"no-todo","kind":"code","mode":"steer","document":{…}}
{"type":"audit","id":"<audit id>","sessionId":"…","workspaceRoot":"…","scope":"repository","mode":"log","status":"completed","startedAt":…,"finishedAt":…,"ruleRevisions":[…],"ruleProvenance":[…],"paths":[…],"failures":[…],"failureCount":0}
{"type":"violation","id":"<audit id>:<ordinal>","auditId":"…","sessionId":"…","ruleId":"…","violationKey":"…","logged":true,"code":"…","clauseId":"…","mode":"log","message":"…","path":"…","start":{"line":3,"column":5},"end":{"line":3,"column":9},"span":{…},"payloadHash":"sha256:…"}
{"type":"scan","id":"<scanId>", …the rerail.scan/1 report…}
{"type":"gap","id":"…","stream":"event","fromSequence":7,"toSequence":9,"reason":"deleted-locally"}
```

- Every record has a stable `id`: a record sent twice (a retry, or a session sent again once it ends) is the same
  record, and the service keeps the latest. The same records make the same `batchId`, so a batch resent after a lost
  answer can be stored once.
- What a record refers to comes before it, in the same batch or an earlier one: repositories, rules and sessions
  before events, audits and violations.
- Positions: a violation's `start` and `end` are **1-based**, `end` just past the match, as in a
  [scan report](headless-scan.md#the-report-rerailscan1). Its `span` is the engine's own, as audits store it: lines,
  columns and bytes **0-based**, ends exclusive. A `scan` record's positions are the report's (1-based).
- `machineLabel` is the computer's name (on macOS its Computer Name, elsewhere its host name), or the `--label` the
  remote was added with.
- An event's payload is its stored JSON; `"content":"omitted"` with `--no-content`, `"content":"unrepresentable"` with
  a null payload for the rare one stored only in Node's binary form.
- Rule documents are sent for the rules audits and scans ran, when Rerail can still find the exact content (an
  installed or bundled pack, a rule you created, a worktree's `.rerail/rules/`); a rule changed since is not sent.

Answers:

| Status | Rerail |
| --- | --- |
| `200` | The batch is stored: the cursor moves past it. |
| `400`, `413` | Rerail halves the batch and sends again, until it finds the one record the remote refuses on its own. That record is skipped and a `gap` (`stream: "batch"`, its event sequence or its stream and id) goes in the next batch. After three such records in one round, Rerail stops and retries later with backoff instead, so a remote that refuses everything does not empty the queue. |
| `401`, `403` | Waits five minutes, or until the remote is removed and added again with a new token (`rerail remote remove`, then `add`). |
| anything else, timeouts, network errors | Retries the same batch, waiting 2 s, then doubling to at most five minutes. |

**Gaps.** Rerail says what it could not send rather than leaving a hole: events deleted before they were sent (`rerail
reset`), a record larger than a batch (`reason: "too-large"`), and rejected batches. Data changed **after** it was
sent, for example by `rerail scrub`, is not sent again: scrub before adding a remote, or ask the service's owner to
delete it.
