# Rule packs declared in the repository

Status: a design for the Rerail owners to react to. In internal testing, `rerail scan` reads the file as described
here (`src/engine/catalog/packs-file/`, `src/cli/scan-packs.ts`); sessions do not read it yet.

## Problem

The packs a repository runs are machine state. `rerail scan --pack <id>` runs the version installed in that machine's
`$RERAIL_HOME/packs`, and a session runs the packs attached to the repository in that machine's database
(`rule_attachments`). Two machines can scan the same commit with different packs, or different versions of them: a
developer's laptop, a CI runner and a cloud runner all disagree as soon as one of them installs or updates a pack.

The decision this design follows: everything about a repository's rules is configured through `.rerail` in the
repository, which packs are enabled included, so any machine runs the same rules for the same commit. The repository's
own rules are already there (`.rerail/rules/`, [a worktree's own rules](rule-packs.md#a-worktrees-own-rules)); this adds
the packs.

## The file

`.rerail/packs.json`, beside `.rerail/rules/`:

```json
{
  "format": "rerail.packs/v1",
  "packs": [
    { "id": "complexity", "version": "0.1.8" },
    { "id": "@acme/rerail-pack-sql", "version": "1.2.0" },
    { "path": "packs/rerail" }
  ]
}
```

- **`id` and `version`**: a published pack. `id` takes what `rerail packs add` and `--pack` take: a Rerail pack's id
  (`complexity` is `@neralabs/rerail-pack-complexity`) or any pack's package name. `version` is one exact version
  (`0.1.8`, `1.0.0-next.2`): no range, no dist-tag, since either resolves to different content as new versions
  publish.
- **`path`**: a pack kept in the repository, a folder holding `pack.json` (and optionally `package.json`), relative to
  the repository's root. It must stay inside the repository; a path with `..`, an absolute path or a symbolic link on
  the way is refused.
- **No file**: nothing changes. A scan and a session choose packs as they do today (below).

Each entry names one pack for the whole repository. The file is refused, with the reason, when it is not JSON, has a
key it does not define, names a pack twice, or gives a published pack no exact version.

<details>
<summary>Why this shape, and what changed from the first proposal</summary>

- `format` and `/v1`, not `schema` and `/1`: the documents people write for Rerail say what they are with `format`
  (`rerail.rule-pack/v1` in `pack.json`, `rerail.policy/v1` in a rule document). `schema: "rerail.scan/1"` is the
  scan's output.
- `id` takes a package name as well as an id, as `rerail packs add` and `--pack` do: a third party's pack has no
  short id of its own, and a Rerail pack id is its package name's short form.
- No `folder` per entry yet. A session's attachments can add a pack to one folder (set-up adds a TypeScript pack to
  `web/` when only `web/` has TypeScript); the file adds every pack to the whole repository. A pack's own `match` and
  each rule's `scope` already keep its rules to the files they read. Adding `"folder"` later is a compatible change.

</details>

## `rerail scan` reads it

**One commit decides.** The scan reads its rules from one tree: `.rerail/rules/`, `.rerail/packs.json` and every
`path` pack the file lists come from the working tree, or, with `--rules-ref <ref>`, all three from that ref. A change
under review can then add, remove or edit neither a pack nor a rule that judges it.

**The file replaces the machine's choice.** When the file exists, the scan always uses a throwaway configuration: the
file's packs and the tree's own rules, each starting with its pack's `on` and `mode`. The home's attachments and rule
settings are not used, even where Rerail is set up in the checkout. The home's approvals are, exactly as today
([Approval](#approval)). Without the file, the scan is what it is today: the home's configuration when Rerail is set
up there, else the tree's rules plus each `--pack`.

**Published packs are fetched at their version for the scan.** Each is installed with npm, at exactly the version the
file names, into the scan's own temporary folder, as set-up's staging install does: `--ignore-scripts`, no lockfile,
the person's npm login, `--prefer-offline` so a version npm has cached needs no network. The scan never installs into,
or reads a pack from, the home's `packs` folder: that folder holds one version of each pack for every repository on the
machine, and a pack linked there from a folder can be edited in place. Nothing else on the machine changes, and a
session's packs are untouched.

**A listed pack that does not run fails the scan.** A pack npm cannot give (no such version, no read access, registry
unreachable), a `path` that is not a pack, or a pack the catalog leaves out (its core range, a clash, a rule that does
not compile) is an error naming the pack and why, exit `1`, never a scan that passes with its rules missing. That is
how `--pack` treats a pack that is not installed today.

**`--pack` is refused when the file exists.** `rerail scan --pack x` there fails: `this repository lists its packs in
.rerail/packs.json; add x there`. Adding on top would let a CI job's command line run rules the repository does not
declare, which is the drift the file removes; a pack to try out is one line in the file on a branch. Without the
file, `--pack` works as it does today.

**The report names the packs.** `rerail.scan/1` gains a field, which its contract allows: `packs`, each pack the scan
ran with its `id`, `packageName`, `version` and `from` (`npm`, `repository` with its `path`, or `home` for one from the
home's configuration or `--pack`). Every rule keeps `source.packId`, so each finding still names its pack.

## A session on a developer's machine reads it

A session in a started, set-up checkout whose tree has `.rerail/packs.json` runs the file's packs:

- **Attachments.** The file's packs are the repository's pack attachments for that checkout, for the whole
  repository, the way `.rerail/rules/` is attached without a row today. The database's pack attachments for that
  repository are not used while the file exists. Rule settings, library rules and approvals still apply by id, as
  they do across a pack upgrade.
- **`path` packs** are read from the checkout's working tree, as `.rerail/rules/` is, and seen at the next event after
  an edit.
- **Published packs** come from a store of exact versions in the Rerail home (`$RERAIL_HOME/pack-versions/`, one
  folder per package and version), so two repositories that list different versions of one pack each run theirs. A
  person fills it: `rerail packs add --repo <dir>` installs what the checkout's file lists, and set-up and
  `rerail start` offer to. The daemon never fetches by itself because a branch changed a file. A listed version not in
  the store is reported where a missing attachment is today (`rerail status`, the dashboard), and the other packs run.
- **Dashboard and API.** While the file exists, adding or removing a pack for the repository is refused with
  `this repository lists its packs in .rerail/packs.json`; turning rules on or off and their modes work as today.

Without the file, a session runs what the database attaches, as today.

## Approval

Content no one approved on this machine runs in Log, exactly as today, and nothing in this change weakens that:

- Rerail's own packs fetched from npm need no approval, as today.
- A third party's pack fetched from npm runs in Log until a person approves that version
  (`rerail rules approve <package>`), as today. Its approval digest is of its content, so an approval made after
  `rerail packs add <package>@<version>` covers the copy the scan fetches.
- A `path` pack counts as installed from a folder whatever its `package.json` says, `@neralabs/*` and a missing
  `package.json` included: it runs in Log until approved, and never passes as Rerail's. `rerail rules approve` in a
  checkout lists the checkout's `path` packs beside its own rules, so CI approves them from the base branch's checkout
  as it approves the rules today (`rerail rules approve <package> --repo ./base`).
- `.rerail/packs.json` itself needs no approval: it only chooses among packs, and each pack's content is approved or
  not on its own terms. Listing a third party's pack, or a `path` pack, does not make it steer.

## What it changes for existing installations

Nothing without the file. A repository that adds it changes what its scans run, and once sessions read it, what its
sessions run; its database attachments stay and apply again if the file is removed. Rule settings stay in the
database; moving them into `.rerail` is a separate decision.

## The change

Behind `RERAIL_EXPERIMENTAL_HEADLESS_SCAN`, the switch `rerail scan` already needs; read by the daemon for sessions.

1. This note.
2. `rerail scan` reads the file as above, with tests: two fresh Rerail homes scanning the same commit run the same
   rules; changing the file changes the next scan's rules; a file that cannot be parsed is a clear error; the report
   names every rule's pack.
3. `rerail rules approve` lists a checkout's `path` packs.
4. Docs ([headless scan](headless-scan.md), [rule packs](rule-packs.md), [internal testing](internal-testing.md)) and a
   changelog entry.

A session reading the file (above) needs the version store, the daemon's catalog per checkout and the dashboard's
refusal: its own pull request, after this one.

## Questions for the owners

- `--pack` when the file exists: refused (proposed) or added on top?
- Sessions: the daemon never fetches a listed pack by itself (proposed), or does once a person has started Rerail in
  the checkout?
- `folder` per entry: needed in v1, or later?
- A scan with the file ignores the home's rule settings (proposed), so tuning made on one machine does not reach CI.
  Is that the intent until settings move into `.rerail` too?
