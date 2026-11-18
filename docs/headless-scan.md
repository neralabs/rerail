# Headless scan

> [!WARNING]
> In [internal testing](internal-testing.md): in the code but not ready for public use, and may change or go without
> notice.

`rerail scan` without `--session` scans a checkout in its own process, so it runs in CI.

- **Output:** `--json` prints one `rerail.scan/1` document (below); otherwise one line per violation and a summary.
- **Remotes:** a completed scan is queued for each [remote](remote-sink.md), unless `--no-send`.

<details>
<summary>What it does not need or touch</summary>

No daemon, no session and nothing set up first. It adds nothing to the Rerail database's session history. Outside its temporary folder, a checkout new to Rerail gets a clone id in Git's metadata folder
(`<git common dir>/rerail-repository-id`), as `rerail start` would.
Revision scans can also write to a versioned content cache beside the Rerail home database.

</details>

```sh
rerail scan [--repo <dir>] [--scope repository|git-diff] [--base <ref>] [--revision <commit-or-tree> --analysis-scope <name>] [--rules-ref <ref>] [--pack <id>]... [--rr <file>]...
            [--path <p>]... [--paths-from <file>] [--json] [--fail-on-violations] [--no-send]
```

| Option | Does |
| --- | --- |
| `--repo <dir>` | Git checkout to scan (any folder in it). Default: the current folder. |
| `--scope repository` | Default. Every file the rules read, or only the `--path`/`--paths-from` files. |
| `--scope git-diff` | Changed, staged and untracked files against `HEAD`, or against the merge base with `--base <ref>`. |
| `--revision <commit-or-tree>` | Read one Git snapshot in repository scope. Accepts a hash or a single revision expression. |
| `--analysis-scope <name>` | Retain cache entries for revision scans under this name. Required with `--revision`. |
| `--rules-ref <ref>` | Run `.rerail/rules/` **at that Git ref**, so a change cannot edit the rules judging it. |
| `--pack <id>` | Also run an installed pack. Repeatable. |
| `--rr <file>` | Run this local `.rr` policy. Repeatable. |
| `--json` | Print the report. |
| `--fail-on-violations` | Exit 3 on violations. |
| `--no-send` | Do not queue the report for this machine's [remotes](remote-sink.md). |

<details>
<summary>Option details</summary>

- `--scope repository` with `--path`/`--paths-from`: paths are relative to the checkout; missing ones are skipped.
- `--scope git-diff`: the merge base is of `<ref>` and `HEAD`, so `--base` gives a pull request's changes. Deleted
  files are not scanned. No changed files is a completed scan with nothing in it.
- `--rules-ref`: replaces the working tree's rules. Those rules run in Log until approved (see
  [Which rules run](#which-rules-run)).
- `--pack`: the pack must be installed in the Rerail home first (`rerail packs add <id>`). It takes an id
  (`complexity`) or package name (`@neralabs/rerail-pack-complexity`). A pack that is not installed is an error.
- `--fail-on-violations`: exits 3 only when the scan completed with violations.

</details>

Exit codes:

- `0`: completed. Violations alone do not fail it.
- `1`: failed: an execution failure, or bad input such as an unknown ref.
- `3`: violations, with `--fail-on-violations`.

## Which rules run

The scan uses your Rerail home's configuration when it can, otherwise a throwaway one.

- **Home**: the repository's packs, rules, settings and approvals. Used when Rerail is set up here and no `--pack` or
  `--rules-ref` is given.
- **Throwaway**: the checkout's `.rerail/rules/` or the rules at `--rules-ref`, plus every `--pack`.
- **Worktree `.rr`**: `.rr` policies in `.rerail/rules/` run alongside the configured JSON rules in a headless scan.
  Each file declares one policy. `rerail rules approve` approves its content and includes. Until approval,
  the scan runs it in Log mode.
- **Explicit `.rr`**: `--rr` selects the local policy files to run. The report records these policies in Log mode
  and marks its policy source as `rr`.

Worktree, `--rules-ref` and non-npm pack rules run in **Log** until approved with `rerail rules approve`.

<details>
<summary>Exact conditions and approval policy</summary>

Home applies when Rerail is started and set up in this checkout on this machine. It runs the rules as a session there
would. `rulesSource` is `{"kind":"home"}`; a throwaway configuration gives `{"kind":"worktree"}`, or
`{"kind":"ref","ref":…}` with `--rules-ref`.

Session [rule approvals](writing-rules.md) apply. These run in **Log** until a person approves that exact content in
the Rerail home with `rerail rules approve`:

- worktree rules;
- rules read from `--rules-ref`;
- rules of packs Rerail did not publish from npm (third-party, or installed from a folder or tarball).

Approval binds to the content digest. Approving a rule in a base-branch checkout also covers the same content read with
`--rules-ref`, and nothing else. Rerail's own npm packs need no approval.

</details>

### Scan historical source

```sh
rerail scan --revision HEAD~1 --analysis-scope review
rerail scan --revision HEAD --analysis-scope review --path src/main.ts --json
rerail scope delete review
```

Rerail discovers Git from the current folder or `--repo` and reads the selected tree directly.
The scan uses today's rules, packs, and approvals. Use `--rules-ref` to select rules independently.
The ref supplies its JSON and `.rr` policies. It also supplies local `.rr` includes.
Approvals apply to the policy and include bytes that the scan reads.
A `.rr` policy load error fails the scan, including with `--rules-ref`.
Explicit `--rr` scans use the registered repository identity when it exists.
Paths, package configuration, and `.rerailignore` come from the selected tree.
With `--path` or `--paths-from`, a directory selects its descendants.
A bare repository can supply source. Select its policy with `--rules-ref` or an installed `--pack`.
You can also supply a local policy file using `--rr`.

Git symlinks and submodules do not enter the source scan.
The runner reports missing objects and makes no fetch.
The JSON report includes `source` for a revision scan. Its `commit` is null for a tree.
One invocation covers one snapshot. Use a driver for repeated scans.
Use the same analysis scope to retain cache entries across those scans.
After the last scan, delete the scope to release its entries and start collection.
An active scan can keep its entries while it finishes. A failed scan releases its temporary ownership.
See [Git in rules](rule-syntax/git.md) to query history from a rule.

```sh
rerail scan --revision HEAD~1 --analysis-scope review --rr ./commit-policy.rr --json
```

By default, `rerail scan` keeps its content store beside the Rerail home database.
If you set `RERAIL_CONTENT_STORE_PATH` for a scan, use the same path for `rerail scope delete`.
To share this store with `rerail-engine`, set `RERAIL_TAPE_CACHE` to that path for the native command.

A filesystem scan uses its current directory as the analysis scope.
If you give `--repo`, the scan uses that directory as the scope.
`rerail stop` releases filesystem scopes for the directories it stops.

### In CI

A CI runner starts with an empty Rerail home. Install packs, approve **from the base branch's content** (never the
change under review), then scan the change:

```sh
# Rerail's own packs from npm: no approval needed.
rerail packs add typescript complexity
# A pack kept in the repository: install it from the base branch's checkout, then approve that version.
rerail packs add ./base/packs/acme
rerail rules approve @acme/rerail-pack-acme
# The repository's own rules: approve each as the base branch has it (rerail rules approve --repo ./base lists them).
rerail rules approve no-todo api-boundary --repo ./base
# Scan the change, judged by the base branch's rules.
rerail scan --repo ./head --scope git-diff --base origin/main --rules-ref origin/main \
  --pack typescript --pack complexity --pack acme --json > rerail-scan.json
```

`./base` is a base-branch checkout; `./head` is the change. Unapproved rules still run, reported with `"mode": "log"`.

## The report: `rerail.scan/1`

`--json` prints one document in this shape; fields may be added, but removing or changing one makes a new schema.

```jsonc
{
  "schema": "rerail.scan/1",
  "scanId": "6f0c…",                     // a new UUID per scan
  "tool": { "name": "rerail", "version": "<version>" },
  "startedAt": 1793732129819, "finishedAt": 1793732131165,   // milliseconds since the epoch
  "repository": { "key": "remote:github.com/org/repo", "displayName": "repo" },   // or "clone:<id>" without a remote
  "checkout": { "path": "/abs/checkout", "head": "<sha>|null", "ref": "refs/heads/main|null" },
  "source": { "revision": "HEAD~1", "tree": "<tree-id>", "commit": "<commit-id>|null" },
  "scope": "repository" | "git-diff",
  "base": null | { "ref": "origin/main", "mergeBase": "<sha>" },
  "paths": null | ["src/a.ts"],          // the files scanned, when not the whole checkout
  "rulesSource": { "kind": "home" } | { "kind": "worktree" } | { "kind": "ref", "ref": "origin/main" } | { "kind": "rr" },
  "status": "completed" | "failed",
  "rules": [
    { "ruleId": "no-todo", "payloadHash": "sha256:…", "mode": "log" | "steer" | "block",
      "source": { "kind": "pack" | "library_rule" | "rerail_rule" | "ref", "id": "…", "packId": "…|null", "ref": "…" } }
  ],
  "violations": [
    { "ruleId": "no-todo", "payloadHash": "sha256:…", "mode": "log", "code": "TODO", "clauseId": "todo",
      "path": "src/a.ts", "start": { "line": 3, "column": 5 }, "end": { "line": 3, "column": 9 },
      "message": "…", "fingerprint": "e7b8bfeee1c3c67007fbdad77d7df6bb" }
  ],
  "failures": [ { "code": "adapter-error", "message": "…", "path": null, "ruleId": null } ],
  "summary": { "violations": 1, "failures": 0 }
}
```

- **Positions**: 1-based lines and columns; `end` is exclusive.
- **`mode`**: the rule's effective mode where the violation is. A scan never steers.
- **`payloadHash`**: digest of the exact rule text that produced the finding.
- **`fingerprint`**: stable across scans, so you can tell new findings from existing ones.
- **`failures`**: execution failures and coverage gaps, as an audit records them ([reference](reference.md#scan-and-settings-controls)).

<details>
<summary>Exact field semantics</summary>

- **Positions**: `start` is the match's first character; `end` is **just past** its last. Both are null for a
  violation without a source span. An explicit `.rr` `@place` gives a line, column 1, and a null `end`.
- **`payloadHash`**: `sha256:` of the rule's canonical JSON, the same digest approvals and audits use.
  For a `.rr` policy, it hashes the list of file digests for the policy and its includes.
- **`cacheWarnings`** lists errors that prevented the scan from retaining its cache.
  The report keeps the findings and scan status.
  If the durable cache cannot open, the scan uses temporary in-memory storage.
- **`fingerprint`**: 32 hex characters, `sha256(ruleId \0 path \0 matched text)`, whitespace runs collapsed to one
  space. It is stable while the rule, path and text are, even if the text moves in the file. When one rule matches the
  same text more than once in a file, the second and later matches also hash their ordinal, so fingerprints are unique
  within a scan. The text is never in the report.
  The scan uses the evaluated file version for both the position and the matched text.

</details>
