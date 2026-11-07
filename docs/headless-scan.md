# Headless scan

> [!WARNING]
> In [internal testing](internal-testing.md): in the code but not ready for public use, and may change or go without
> notice.

`rerail scan` without `--session` scans a checkout in its own process, with no daemon, no session and nothing set up
first, so it runs in CI. It prints one JSON document with `--json` (the `rerail.scan/1` shape below), or one line per
violation and a summary without it. The scan adds nothing to the Rerail database's session history. With a
[remote](remote-sink.md) added, a completed scan is queued for it (`--no-send` keeps it local).

```sh
rerail scan [--repo <dir>] [--scope repository|git-diff] [--base <ref>] [--rules-ref <ref>] [--pack <id>]...
            [--path <p>]... [--paths-from <file>] [--json] [--fail-on-violations] [--no-send]
```

| Option | Does |
| --- | --- |
| `--repo <dir>` | The checkout to scan (any folder in it); the current folder by default. It must be a Git checkout. |
| `--scope repository` | Every file the rules read (the default). With `--path` or `--paths-from`, only those files (relative to the checkout; those that do not exist are left out). |
| `--scope git-diff` | The changed, staged and untracked files: against `HEAD`, or with `--base <ref>` against the merge base of `<ref>` and `HEAD` (a pull request's changes). Deleted files are not scanned. No changed files is a completed scan with nothing in it. |
| `--rules-ref <ref>` | Run the rules in `.rerail/rules/` **at that Git ref** instead of the working tree's. For reviewing a change: the change cannot edit the rules it is judged by. They follow the approval policy below. |
| `--pack <id>` | Also run that rule pack, as installed in the Rerail home (`rerail packs add <id>` first), by its id (`complexity`) or package name (`@neralabs/rerail-pack-complexity`). Repeatable. A pack that is not installed is an error. |
| `--json` | Print the report. |
| `--fail-on-violations` | Exit 3 when the scan completed with violations. |
| `--no-send` | Do not queue the report for this machine's [remotes](remote-sink.md). |

Exit codes: `0` the scan completed (violations alone do not fail it), `1` it failed (an execution failure, or bad
input such as an unknown ref), `3` violations with `--fail-on-violations`.

## Which rules run

- **The Rerail home's configuration**, when Rerail is started and set up in this checkout on this machine and neither
  `--pack` nor `--rules-ref` is given: the packs and rules added to the repository, its settings and your approvals, as
  a session there would run them. The report's `rulesSource` is `{"kind":"home"}`.
- **Otherwise** a throwaway configuration: the checkout's own `.rerail/rules/` (`{"kind":"worktree"}`), or the rules at
  `--rules-ref` (`{"kind":"ref","ref":…}`), plus every `--pack`.

Either way the approval policy is a session's ([rule approvals](writing-rules.md)): a worktree's rule, a rule read from
`--rules-ref`, and every rule of a pack Rerail did not publish from npm (a third party's, or one installed from a folder
or tarball) run in **Log** until a person approved that exact content in the Rerail home with `rerail rules approve`.
Approval is of content, by its digest: approving a rule in a checkout of the base branch approves the same content read
with `--rules-ref`, and nothing else. Rerail's own packs from npm need no approval.

The scan writes one thing outside its temporary folder: a checkout Rerail has never seen gets its clone id in Git's
metadata folder (`<git common dir>/rerail-repository-id`), as `rerail start` would give it.

### In CI

A CI runner starts with an empty Rerail home. Install the packs, approve what needs approving **from the base
branch's content** (never from the change under review), then scan the change:

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

Here `./base` is a checkout of the base branch and `./head` the change. Anything left unapproved still runs, in Log:
its findings are reported with `"mode": "log"`.

## The report: `rerail.scan/1`

```jsonc
{
  "schema": "rerail.scan/1",
  "scanId": "6f0c…",                     // a new UUID per scan
  "tool": { "name": "rerail", "version": "0.1.0" },
  "startedAt": 1793732129819, "finishedAt": 1793732131165,   // milliseconds since the epoch
  "repository": { "key": "remote:github.com/org/repo", "displayName": "repo" },   // or "clone:<id>" without a remote
  "checkout": { "path": "/abs/checkout", "head": "<sha>|null", "ref": "refs/heads/main|null" },
  "scope": "repository" | "git-diff",
  "base": null | { "ref": "origin/main", "mergeBase": "<sha>" },
  "paths": null | ["src/a.ts"],          // the files scanned, when not the whole checkout
  "rulesSource": { "kind": "home" } | { "kind": "worktree" } | { "kind": "ref", "ref": "origin/main" },
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

- **Positions**: lines and columns are **1-based**; `start` is the first character of the match and `end` the
  position **just past** its last (exclusive). `start` and `end` are null for a violation without a source span.
- **`mode`**: the rule's effective mode where the violation is. A scan itself never steers.
- **`payloadHash`**: the digest of the rule's content (`sha256:` of its canonical JSON), the same one approvals and
  audits name, so a finding joins to the exact rule text it came from.
- **`fingerprint`**: 32 hex characters, `sha256(ruleId \0 path \0 matched text)` with the text's whitespace runs
  collapsed to one space. It stays the same while the rule, the path and the matched text do, wherever the text moves
  in the file, so two scans tell new findings from existing ones. When one rule matches the same text more than once
  in a file, the second and later matches hash in their ordinal too, so fingerprints are unique within a scan. The text
  is never in the report.
- **`failures`**: execution failures and coverage gaps, as an audit records them ([reference](reference.md#scan-and-settings-controls)).

Fields may be added within `rerail.scan/1`; a change that removes or changes one is a new schema.
