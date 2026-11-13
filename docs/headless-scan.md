# Headless scan

> [!WARNING]
> In [internal testing](internal-testing.md): in the code but not ready for public use, and may change or go without
> notice.

`rerail scan` without `--session` scans a checkout in its own process, so it runs in CI.

- **Output:** `--json` prints one `rerail.scan/1` document (below); otherwise one line per violation and a summary.
- **Remotes:** a completed scan is queued for each [remote](remote-sink.md), unless `--no-send`.

<details>
<summary>What it does not need or touch</summary>

No daemon, no session and nothing set up first. It adds nothing to the Rerail database's session history. Its one write
outside its temporary folder: a checkout new to Rerail gets a clone id in Git's metadata folder
(`<git common dir>/rerail-repository-id`), as `rerail start` would.

</details>

```sh
rerail scan [--repo <dir>] [--scope repository|git-diff] [--base <ref>] [--rules-ref <ref>] [--pack <id>]...
            [--path <p>]... [--paths-from <file>] [--json] [--fail-on-violations] [--no-send]
```

| Option | Does |
| --- | --- |
| `--repo <dir>` | Git checkout to scan (any folder in it). Default: the current folder. |
| `--scope repository` | Default. Every file the rules read, or only the `--path`/`--paths-from` files. |
| `--scope git-diff` | Changed, staged and untracked files against `HEAD`, or against the merge base with `--base <ref>`. |
| `--rules-ref <ref>` | Run `.rerail/rules/` **at that Git ref**, so a change cannot edit the rules judging it. |
| `--pack <id>` | Also run an installed pack. Repeatable. |
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

Worktree, `--rules-ref` and non-npm pack rules run in **Log** until approved with `rerail rules approve`.

<details>
<summary>Exact conditions and approval policy</summary>

Home applies when Rerail is started and set up in this checkout on this machine. It runs the rules as a session there
would. `rulesSource` is `{"kind":"home"}`; a throwaway configuration gives `{"kind":"worktree"}`, or
`{"kind":"ref","ref":…}` with `--rules-ref`. It is `null` when the scan audited nothing, so no rules ran: the scan
fails, and a `checkout-not-audited` failure says why (for example, a copy of a checkout that no session has
recorded).

Session [rule approvals](writing-rules.md) apply. These run in **Log** until a person approves that exact content in
the Rerail home with `rerail rules approve`:

- worktree rules;
- rules read from `--rules-ref`;
- rules of packs Rerail did not publish from npm (third-party, or installed from a folder or tarball).

Approval binds to the content digest. Approving a rule in a base-branch checkout also covers the same content read with
`--rules-ref`, and nothing else. Rerail's own npm packs need no approval.

</details>

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
  "scope": "repository" | "git-diff",
  "base": null | { "ref": "origin/main", "mergeBase": "<sha>" },
  "paths": null | ["src/a.ts"],          // the files scanned, when not the whole checkout
  "rulesSource": { "kind": "home" } | { "kind": "worktree" } | { "kind": "ref", "ref": "origin/main" } | null,
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
  The scan adds its own: `rule-left-out` for a rule file at `--rules-ref` that does not load, and
  `checkout-not-audited` when no audit ran.

<details>
<summary>Exact field semantics</summary>

- **Positions**: `start` is the match's first character; `end` is **just past** its last. Both are null for a
  violation without a source span.
- **`payloadHash`**: `sha256:` of the rule's canonical JSON, the same digest approvals and audits use.
- **`fingerprint`**: 32 hex characters, `sha256(ruleId \0 path \0 matched text)`, whitespace runs collapsed to one
  space. It is stable while the rule, path and text are, even if the text moves in the file. When one rule matches the
  same text more than once in a file, the second and later matches also hash their ordinal, so fingerprints are unique
  within a scan. The text is never in the report.

</details>
