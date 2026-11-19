# Headless scan

> [!WARNING]
> In [internal testing](internal-testing.md): in the code but not ready for public use, and may change or go without
> notice.

`rerail scan` without `--session` scans a checkout in its own process, so it runs in CI.

- **Output:** `--json` prints one `rerail.scan/1` document (below); otherwise one line per violation and a summary.
- **Remotes:** a completed scan is queued for each [remote](remote-sink.md), unless `--no-send`.

<details>
<summary>What it does not need or touch</summary>

The scan runs without a daemon or session. It builds its scan configuration and reads rule output in memory.
A checkout new to Rerail gets a clone ID in `<git common dir>/rerail-repository-id`, as it does with `rerail start`.
A bare repository and its linked worktrees share this ID. Earlier versions could also write an ID under `<bare repo>/.rerail`.
On upgrade, Rerail keeps the common ID when it exists. Otherwise it adopts the earlier bare ID.
If the Git directory is read-only and has no ID, Rerail uses its volume, file number, and creation time when available.
Without a stable directory identity or an existing ID in the local Git configuration, a read-only scan cannot establish ownership.
The content store records this identity with each run. A later writable scan saves the same ID in the common directory.
Rerail resolves a requested Git revision before it registers the analysis scope. An invalid revision leaves no scope registration.
The scan can retain analysis results in the content cache beside the Rerail home database.

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
| `--path <path>` | Select a repository-relative path. Repeat for multiple paths. Use `--path=--alpha.py` for a name that starts with two dashes. |
| `--paths-from <file>` | Read one UTF-8 path per line. |
| `--json` | Print the report. |
| `--fail-on-violations` | Exit 3 on violations. |
| `--no-send` | Do not queue the report for this machine's [remotes](remote-sink.md). |

<details>
<summary>Option details</summary>

- With `--path` or `--paths-from`, paths are relative to the repository. The scan skips absent paths.
  The JSON report records the supplied names in `requestedPaths` and skipped names in `skippedPaths`.
  For an explicit revision selection, `skippedPathDetails` gives the original `path`, the `reason`, and the numeric Git `mode` when an entry exists.
  The reasons are `absent`, `symlink`, `gitlink`, `not-directory`, and `unsupported-mode`.
  If an ancestor prevents the lookup, its entry supplies the reason and mode. An unreadable tree produces a scan failure.
  Both providers resolve `.` and `..` components before selecting files. An absolute path or a path outside the repository is an error.
  The `paths` field gives the normalized selections. For example, `src/../src/a.py` selects `src/a.py`.
  A working-tree selection uses the spelling stored on the filesystem. On a case-insensitive volume, `SRC/A.PY` can select `src/a.py`.
  On a case-sensitive volume, each spelling selects its own entry. A revision always requires the spelling in the Git tree.
  For a working-tree selection through a symbolic link, `paths` keeps the link name. Revision scans skip Git symlinks.
  A trailing directory separator or `/.` requires a directory on both source providers.
  A path list accepts one UTF-8 name per line and preserves spaces in each name.
  An invalid UTF-8 line stops the scan and reports its line number.
  A Git diff can contain a path with invalid UTF-8. The scan reports its bytes in hex and continues with the readable names.
  If a Git object prevents path lookup, the scan reports the path and fails.
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
It reads the home settings, attachments, library rules, and approvals from one database snapshot.
A change committed after that read begins applies to a subsequent scan.

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
The selected directory determines the checkout, index, and object database for Git reads.
A parent process can have `GIT_DIR` or `GIT_WORK_TREE` set for another checkout.
The scan uses its selected directory for checkout metadata and rule queries in this case too.
Refreshing a view after the pack layout changes keeps the same repository and index.
The scan uses today's rules, packs, and approvals. Use `--rules-ref` to select rules independently.
The scan loads each selected `.rr` policy and its includes before it evaluates any policy.
A retry uses these programs and their original digests.
The ref supplies its JSON and `.rr` policies. It also supplies local `.rr` includes.
Rerail reads policy files from Git objects. Names use Git case rules.
The scan selects file names before it checks their UTF-8 encoding.
It reports selected names with invalid UTF-8 in hexadecimal and continues with readable entries.
In revision JSON, each finding or failure with a path has a `pathIdentity` object.
Its `encoding` is `utf8` for text or `hex` for bytes. Its `value` identifies the complete path, including the parent directories.
For a policy from `--rules-ref`, the identity gives the path inside the policy tree. The display path includes the ref.
Join Git directory entries to the decoded identity. The display label `[hex:ff2e7079]` can also be a literal Git file name.
The report gives the requested ref and the resolved commit for those policies.
The resolved commit stays the same if the ref moves during the scan.
If the ref has no rules directory, the ref supplies zero policies.
If Rerail cannot read an object for the policy directory or a policy file, the scan fails.
Approvals apply to the policy and include bytes that the scan reads.
A `.rr` policy load error fails the scan, including with `--rules-ref`.
Explicit `--rr` scans use the registered repository identity when it exists.
Paths, package configuration, and `.rerailignore` come from the selected tree.
When an unlisted file exceeds the per-file byte limit, Rerail inspects its first 8,000 bytes.
A NUL in that prefix skips the file as binary. A text prefix produces a byte-budget failure.
An unreadable prefix produces a source-read failure. A revision scan reads the prefix from the Git object database.
For a packed delta, the reader follows copy instructions to obtain the required ranges from its base.
The prefix serves only to classify the file. Acquiring a content handle still verifies the entire object before storing its bytes.
With `--path` or `--paths-from`, a directory selects its descendants.
The path `.` selects the entire tree. The scan still applies the ignore rules.
Python package lookup uses the markers in the selected tree, including markers outside the scan paths.
A JSON policy from `--rules-ref` must use valid UTF-8.
A bare repository can supply source. Select its policy with `--rules-ref` or an installed `--pack`.
You can also supply a local policy file using `--rr`.

Git symlinks and submodules do not enter the source scan.
The runner reports missing objects and makes no fetch.
If a subtree is unreadable, the report gives its path and marks the scan as failed.
The scan still reports findings from readable files.
For `--revision` and `--rules-ref`, explicit relative Git paths start in the invocation directory or the directory given to `--repo`.
The flags accept Git commit message searches such as `HEAD^{/^Release}`.
A revision error identifies the expression and the cause of the failed lookup.
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
The home remembers which store each named scan used. `rerail scope delete review` releases that scope and prints the target store.
If two stores registered the same name, select one with `rerail scope delete review --store <file>`.
Use the same option for a store created by a standalone driver or an older scan without a registration.
A missing registered store keeps its registration for retry. A different database at the same path cannot acknowledge the original store's release.
To share this store with `rerail-engine`, set `RERAIL_TAPE_CACHE` to that path for the native command.

A filesystem scan uses its current directory as the analysis scope.
If you give `--repo`, the scan uses that directory as the scope.
The reported scope path uses the directory name stored on disk, including its letter case.
`rerail stop` releases filesystem scopes for the directories it stops.
Rerail records the stores used by filesystem scans and watches.
A stop releases scopes in those stores even if the configured store changes.

Use `rerail scope stores` to list each registered store identity and path.
If a store goes offline permanently or moves, run `rerail scope retire-store <identity>`.
Maintenance stops polling its path. Rerail remembers directory stops for that identity.
If a scan uses the original store again, Rerail delivers those stops at its current path.
A replacement store at the old path keeps its own scopes.

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

`checkout.ref` gives the full name of the current symbolic reference when its name is valid UTF-8.
For a ref with other bytes, `checkout.ref` is null and `checkout.refHex` contains the original bytes in hexadecimal.
A detached HEAD has a null ref and no `refHex`.

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
  "rulesSource": { "kind": "home" } | { "kind": "worktree" } | { "kind": "ref", "ref": "origin/main", "commit": "<sha>" } | { "kind": "rr" },
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

### Locations for commit findings

In a `.rr` violation head, use `@git_commit_location(repo, commit)` to identify a commit.
Add a third argument to identify a metadata field, for example `@git_commit_location(r, c, "message")`.
Use the repository and commit from the rule. A policy can report a parent of the selected revision.

The JSON finding carries `location` with `kind: "git-commit"`, `repository`, `commit`, and `field`.
The field is null when the rule supplies two arguments. The finding's `path`, `start`, and `end` are null.
The text report prints the repository, full commit ID and field when present.
These values also identify the location for fingerprinting. A file named `git` keeps its file path and source position.

<details>
<summary>Exact field semantics</summary>

- **Positions**: `start` is the match's first character; `end` is **just past** its last. Both are null for a
  violation without a source span. An explicit `.rr` `@place` gives a line, column 1, and a null `end`.
- **`payloadHash`**: `sha256:` of the rule's canonical JSON, the same digest approvals and audits use.
  For a `.rr` policy, it hashes the list of file digests for the policy and its includes.
- **`analysisScope`** identifies the owner requested by this scan. A named scope has a `name`.
  A directory scope has a checkout-relative `path`. The checkout root uses `.`.
- **`cache.publication`** records the result of the ownership transaction.
  `published` means the store accepted ownership. `released` means a stop or scope deletion invalidated the run before publication.
  A failed analysis uses `aborted`. A store error uses `unavailable`.
- **`cache.collection`** records the separate collection call. Its values are `completed`, `failed`, and `skipped`.
  Collection can fail after a successful publication.
- **`cacheWarnings`** lists errors from either operation. A concurrent stop leaves successful findings available.
  The report keeps the findings and scan status.
  If the durable cache fails, the scan can retry in memory.
  The retry uses the same Git source and policy. Git read failures still fail the scan.
  If cache cleanup also fails, the scan still reports the original error.
- **`fingerprint`**: 32 hex characters, `sha256(ruleId \0 path \0 matched text)`, whitespace runs collapsed to one
  space. It is stable while the rule, path and text are, even if the text moves in the file. When one rule matches the
  same text more than once in a file, the second and later matches also hash their ordinal, so fingerprints are unique
  within a scan. The text is never in the report.
  The scan computes the fingerprint before it releases the evaluated source.

</details>

## Reading a file from a native revision view

Use `GitRevisionSource.readSourceChecked(path, limit)` when a caller needs to report why a read failed.
The limit is an integer from zero through 4,294,967,295 bytes.

| Result | Meaning |
| --- | --- |
| `{ status: "ok", bytes }` | The blob fits the limit and its bytes match its Git object ID. Empty files return an empty buffer. |
| `{ status: "absent" }` | The source tree has no regular file at the requested path. The view skips symlinks and submodules. |
| `{ status: "too-large" }` | The blob header reports a size above the limit. |
| Exception | The argument is invalid, the path names a directory, or the view cannot read a required tree or blob. |

After restoring a missing object, a caller can retry the same view.
The older `readSource(path)` uses a 32,000,000-byte limit and returns null for every failed read.
The older `readFile(path, limit)` returns null for an oversized blob and throws on absence or a read error.
