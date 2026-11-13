# Remote sink

> [!WARNING]
> In [internal testing](internal-testing.md): in the code but not ready for public use, and may change or go without
> notice. Experimental: unless `RERAIL_EXPERIMENTAL_REMOTE_SINK=1` (or `RERAIL_EXPERIMENTAL=1`) is set, the daemon
> sends nothing, a headless scan queues no report, and `rerail remote` refuses but for `list`, `status` and `remove`.

A remote is an HTTPS endpoint that receives this machine's sessions, events, audits, violations, the rules they ran,
and [headless scans](headless-scan.md). It lets a team see Rerail across machines.

**With no remote added, or with the remote sink off, nothing leaves the machine.** Rerail ships no remote; you point it at a service that
implements the [protocol](#protocol).

## Commands

`rerail remote` manages remotes and opts repositories out.

```sh
rerail remote add <url> [--name <name>] [--label <machine label>] (--token-stdin | --token-file <file>) [--no-content]
rerail remote list [--json]
rerail remote remove <name|url>
rerail remote status [--json]
rerail remote push [--json]
rerail remote exclude [--repo <dir>]
rerail remote include [--repo <dir>]
```

- **`add`** registers an HTTPS URL and the token the service gave this machine.
- **`--no-content`** sends events without their prompts and tool input and output.
- **`remove`** deletes the token and the outbox's progress; re-adding starts from nothing.
- **`status`** shows the last batch, what is waiting, and the last error.
- **`push`** sends everything waiting now, without the daemon. Use it in CI.

The daemon sends every 10 seconds; `add` and `remove` need no restart.

<details>
<summary>Command details</summary>

- **`add`** takes an HTTPS URL (plain HTTP only to `localhost`) and the service's bearer token for this machine, from
  standard input or a file, never the command line. The name defaults to the URL's host. A new remote first gets
  what the database already holds, then everything new.
- **`--no-content`** sends events without payloads (prompts, tool input and output), marked `"content":"omitted"`.
  Everything else is sent.
- **`remove`** deletes the remote's token and what the outbox knew of it. A remote added later under the same name
  starts from nothing and receives everything again. Each `add` gives the remote a new id, which the outbox keeps its
  progress under. A send under way at removal records nothing and stops.
- **`status`** shows when the remote last took a batch, what is waiting, and the last error. `rerail status` shows a
  line per remote. It exits 1 while a remote is failing.
- **`push`** sends everything waiting now, in this process, without the daemon and without waiting for a retry. It is
  for CI and for hosts that do not run the daemon.

The daemon sends on its own connections, so a remote that is slow or down never holds up a session.

While a remote is set up, set-up's question about reading your sessions says they are sent, to which host, and how
to stop, instead of "Stays on this computer".

</details>

### Excluding a repository

`exclude` opts a repository (`--repo`, or the current folder's) out of every remote; `include` opts it back in.

**`include`** applies **from that moment**: what the repository recorded before it is never sent.

<details>
<summary>Edge cases</summary>

- Nothing of the repository that a remote lacks is sent, whether recorded before or after `exclude`.
- After `include`, earlier data stays unsent even if no send ran while it was excluded. A session started then stays
  unsent with all its events and audits.
- A repository matches under every key it has had: its current key and its aliases (an older clone id, a former
  remote).
- A session with no repository is placed by its folder's recorded checkout. If no checkout places it, it is held back
  while any repository is excluded, and sent once none is.

Exclusions, and when `include` ended them, are kept in `remote.json`.

</details>

## On disk

Remote state lives in four files in the Rerail home, readable by you only.

The token is a plain file outside the database. Deleting `remote-outbox.sqlite` makes every remote receive everything
again.

<details>
<summary>Files and permissions</summary>

| File in the Rerail home | Holds |
| --- | --- |
| `remote.json` | Remotes (name, id, URL, whether content is sent, label) and exclusions. |
| `remote-tokens/<name>` | That remote's token, alone. Never printed; never in the database, which support bundles and `rerail scrub` read. |
| `remote-outbox.sqlite` | Acknowledgements (an event cursor, a hash per sent record), retry state, queued scans. Deleting it resends everything. |
| `machine-id` | A random UUID made once, naming this machine to every remote. |

All four are yours only (files `0600`, the token folder `0700`), like the rest of the home. The token is a plain
file, not in the system keychain, because the daemon reads it unattended.

</details>

## Protocol

Rerail posts NDJSON batches of records with stable ids to one endpoint.

`POST <url>/v1/batches`, `Authorization: Bearer <token>`, `Content-Type: application/x-ndjson`, at most 5 MB. Line
one is the batch header; each further line is a record:

```jsonc
{"type":"batch","protocol":1,"batchId":"<uuid>","machineId":"<uuid>","machineLabel":"<hostname>","rerailVersion":"<version>","sentAt":1793732129819}
{"type":"repository","id":"remote:github.com/org/repo","displayName":"repo","aliases":[]}
{"type":"session","id":"<session id>","host":"claude-code","cwd":"/abs","repositoryKey":"…","startedAt":…,"endedAt":…|null}
{"type":"event","id":"<machineId>:<sequence>","sessionId":"…","eventId":"…","kind":"user-prompt","observedAt":…,"sequence":42,"payload":{…}}
{"type":"rule","id":"sha256:…","ruleId":"no-todo","kind":"code","mode":"steer","document":{…}}
{"type":"audit","id":"<audit id>","sessionId":"…","workspaceRoot":"…","scope":"repository","mode":"log","status":"completed","startedAt":…,"finishedAt":…,"ruleRevisions":[…],"ruleProvenance":[…],"paths":[…],"failures":[…],"failureCount":0}
{"type":"violation","id":"<audit id>:<ordinal>","auditId":"…","sessionId":"…","ruleId":"…","violationKey":"…","logged":true,"code":"…","clauseId":"…","mode":"log","message":"…","path":"…","start":{"line":3,"column":5},"end":{"line":3,"column":9},"span":{…},"payloadHash":"sha256:…"}
{"type":"scan","id":"<scanId>", …the rerail.scan/1 report…}
{"type":"gap","id":"…","stream":"event","fromSequence":7,"toSequence":9,"reason":"deleted-locally"}
```

- **Stable ids.** A record sent twice is the same record; the service keeps the latest.
- **Order.** A record comes after the records it refers to.

<details>
<summary>Record rules</summary>

- **Stable ids.** A retry, or a session resent when it ends, repeats the same record. The same records give the same
  `batchId`, so a batch resent after a lost answer can be stored once.
- **Order.** Referenced records come first, in the same or an earlier batch: repositories, rules and sessions before
  events, audits and violations.
- **Positions.** A violation's `start` and `end` are **1-based**, `end` just past the match, as in a
  [scan report](headless-scan.md#the-report-rerailscan1). Its `span` is the engine's own, as audits store it: lines,
  columns and bytes **0-based**, ends exclusive. `scan` records use the report's 1-based positions.
- **`machineLabel`** is the computer's name (macOS Computer Name, else host name), or the `--label` given to `add`.
- **Payloads** are the stored JSON, or `"content":"omitted"` with `--no-content`. The rare event stored only in
  Node's binary form sends `"content":"unrepresentable"` and a null payload.
- **Rule documents** are sent for rules that audits and scans ran, if the exact content is still findable: an
  installed or bundled pack, a rule you created, a worktree's `.rerail/rules/`. A rule changed since is not sent.

</details>

### Answers

`200` stores the batch; Rerail skips records the remote refuses and retries other failures with backoff.

<details>
<summary>Per-status handling</summary>

| Status | Rerail |
| --- | --- |
| `200` | Stored; the cursor moves past the batch. |
| `400`, `413` | Halves and resends until it isolates the refused record, skips it, and sends a `gap` (`stream: "batch"`, its event sequence or stream and id) next batch. After three in one round, it backs off and retries later, so a remote refusing everything does not empty the queue. |
| `401`, `403` | Waits five minutes, or until the remote is re-added with a new token (`rerail remote remove`, then `add`). |
| anything else, timeouts, network errors | Retries the same batch after 2 s, doubling to at most five minutes. |

</details>

### Gaps

A `gap` record reports data Rerail could not send, instead of leaving a hole:

- events deleted before sending (`rerail reset`);
- a record larger than a batch (`reason: "too-large"`);
- rejected batches.

Data changed **after** sending, for example by `rerail scrub`, is not resent. Scrub before adding a remote, or ask the
service's owner to delete it.
