# Rule packs declared in the repository

Status: the design the Rerail owners settled in review. In internal testing, `rerail scan` reads the file as described
here (`src/engine/catalog/packs-file/`, `src/cli/scan-packs.ts`); sessions do not read it yet (a later change, below).

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
  "schema": "rerail.packs/1",
  "packs": [
    { "id": "complexity", "version": "0.1.8" },
    { "id": "typescript", "version": "0.1.8", "folders": ["web", "admin"] },
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
- **`folders`** (optional, on either kind): the folders the pack judges, each relative to the repository's root, as
  set-up's folder attachments are (the TypeScript pack on `web` only). Each follows the rules of `path`: inside the
  repository, no `..`, no absolute path, no symbolic link on the way, and a folder of the scanned tree. Left out, the
  pack judges the whole repository. A list keeps one entry per pack, so "names a pack twice" and one version per pack
  stand as written.
- **No file**: nothing changes. A scan and a session choose packs as they do today (below).

The file is refused, with the reason, when it is not JSON, has a key it does not define, names a pack twice, gives a
published pack no exact version, or lists a folder that is empty, repeated, outside the repository or not a folder.

<details>
<summary>Why this shape</summary>

- `schema` and `rerail.packs/1`: the shape repositories already commit (this one's own `.rerail/packs.json` among
  them) and a scanner outside Rerail already reads. Rerail takes that spelling and no other, so nothing already in
  use changes.
- `folders` is optional, so every file written before it keeps its meaning: each pack judges the whole repository. A
  reader that predates `folders` and ignores it runs such a pack on the whole repository, wider than the file says.
- `id` takes a package name as well as an id, as `rerail packs add` and `--pack` do: a third party's pack has no
  short id of its own, and a Rerail pack id is its package name's short form.
- `folders` is in the first version rather than later: without it the file cannot say what set-up writes today, and a
  repository moving to the file would have its packs widened to the whole repository, with new findings in folders
  never checked on the day it adopts the file. A pack's `match` and each rule's `scope` cannot say "this pack judges
  `web` and not `legacy`" when both hold the same language.

</details>

## `rerail scan` reads it

**One commit decides.** The scan reads its rules from one tree: `.rerail/rules/`, `.rerail/packs.json`, every `path`
pack and every folder the file lists come from the working tree, or, with `--rules-ref <ref>`, all from that ref. A
change under review can then add, remove, edit or widen neither a pack nor a rule that judges it. Only the scan's
command line can set the file aside, with `--pack` (below).

**The file replaces the machine's choice.** When the file exists, the scan always uses a throwaway configuration: the
file's packs, each attached at its `folders` or the whole repository, and the tree's own rules, each rule starting
with its pack's `on` and `mode`. The home's attachments and rule settings are not used, even where Rerail is set up in
the checkout, so a developer's local scan reproduces CI exactly. The home's approvals are, exactly as today
([Approval](#approval)). Without the file, the scan is what it is today: the home's configuration when Rerail is set up
there, else the tree's rules plus each `--pack`. With the file and `--pack`, the throwaway configuration holds the
`--pack` packs in place of the file's (below).

**Published packs are fetched at their version for the scan.** Each is installed with npm, at exactly the version the
file names, into the scan's own temporary folder, as set-up's staging install does: `--ignore-scripts`, no lockfile,
the person's npm login, `--prefer-offline` so a version npm has cached needs no network. The scan never installs into,
or reads a pack from, the home's `packs` folder: that folder holds one version of each pack for every repository on the
machine, and a pack linked there from a folder can be edited in place. Nothing else on the machine changes, and a
session's packs are untouched.

**A listed pack that does not run fails the scan.** A pack npm cannot give (no such version, no read access, registry
unreachable), a `path` that is not a pack, a listed folder that is not a folder of the scanned tree, or a pack the
catalog leaves out (its core range, a clash, a rule that does not compile) is an error naming the pack and why, exit
`1`, never a scan that passes with its rules missing. That is how `--pack` treats a pack that is not installed today.

**`--pack` replaces the file's packs.** `rerail scan --pack x` where the file exists runs only the `--pack` packs, each
the version installed in the Rerail home as without the file, and the tree's own `.rerail/rules/` (from the working
tree or `--rules-ref`); none of the file's packs run, and the file is not read. This is for experimental pack scans:
trying a pack, or a version of one, on a repository that lists its packs, without editing the file. Replacing rather
than adding keeps each scan's packs from one source, so the home's installed versions never mix with the file's pinned
ones and no rule is needed for a pack named in both. It does not weaken "one commit decides": the change under review
still cannot choose its packs, since `--pack` comes from the command that runs the scan, and the report says the file
was set aside (every pack `from: "home"`). Approval is as without the file: a third party's pack, or one installed
from a folder, runs in Log until approved. Without `--pack`, the file decides. The first draft refused `--pack` here,
as the choice that could be loosened later; replacing is that loosening.

**The report names the packs.** `rerail.scan/1` gains a field, which its contract allows: `packs`, each pack the scan
ran with its `id`, `packageName`, `version`, `from` (`npm`, `repository` with its `path`, or `home` for one from the
home's configuration or `--pack`) and its `folders` when the file gives them. A scan where `--pack` replaced the file's
packs lists only `home` packs. Every rule keeps `source.packId`, so each finding still names its pack.

## A session on a developer's machine reads it

A session in a started, set-up checkout whose tree has `.rerail/packs.json` runs the file's packs:

- **Attachments.** The file's packs are the repository's pack attachments for that checkout, at their `folders` or
  the whole repository, the way `.rerail/rules/` is attached without a row today. The database's pack attachments for
  that repository are not used while the file exists. Rule settings, library rules and approvals still apply by id,
  as they do across a pack upgrade.
- **`path` packs** are read from the checkout's working tree, as `.rerail/rules/` is, and seen at the next event after
  an edit.
- **Published packs** come from a store of exact versions in the Rerail home (`$RERAIL_HOME/pack-versions/`, one
  folder per package and version), so two repositories that list different versions of one pack each run theirs.
- **Who fetches them** (settled in review). The daemon fetches a listed version of Rerail's own packs
  (`@neralabs/*`) into the store by itself, since they already run without approval: a branch that bumps one runs the
  same rules in a session as in CI. A third party's pack still waits for a person: `rerail packs add --repo <dir>`
  installs what the checkout's file lists, and set-up and `rerail start` offer to. So no branch, including someone
  else's pull request, makes the daemon run an npm install of a third party's package with the developer's login. A
  listed version not in the store is reported where a missing attachment is today (`rerail status`, the dashboard),
  and the other packs run.
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
sessions run; its database attachments stay and apply again if the file is removed. Files already committed in the
`rerail.packs/1` shape keep working unchanged: `folders` is optional, and without it a pack judges the whole
repository, as before.

Rule settings stay in the database, and a scan with the file does not use them: every listed pack runs at its own
starting `on` and `mode`, so the only way past one noisy rule in CI is dropping its pack. Moving rule settings into
`.rerail` is therefore the next step after this one, so that a repository's tuning reaches every machine as its packs
do.

## The change

Behind `RERAIL_EXPERIMENTAL_HEADLESS_SCAN`, the switch `rerail scan` already needs.

1. This note.
2. `rerail scan` reads the file as above, `folders` included, with tests: two fresh Rerail homes scanning the same
   commit run the same rules; changing the file changes the next scan's rules; a file that cannot be parsed is a clear
   error; the report names every rule's pack; a pack with `folders` judges only those folders; this repository's own
   committed file reads as it is.
3. `rerail rules approve` lists a checkout's `path` packs.
4. Docs ([headless scan](headless-scan.md), [rule packs](rule-packs.md), [internal testing](internal-testing.md)) and a
   changelog entry.

Next, each its own pull request: sessions reading the file (the version store, the daemon's catalog per checkout, its
fetching of `@neralabs/*` packs, the dashboard's refusal), then rule settings in `.rerail`.

## Decisions

Settled in review of the first draft:

- `--pack` when the file exists: first refused; now it replaces the file's packs, for experimental pack scans.
- Sessions: the daemon fetches Rerail's own `@neralabs/*` packs by itself; a third party's pack waits for a person.
- `folders` per entry: in the first version, optional.
- A scan with the file ignores the home's rule settings; moving rule settings into `.rerail` is the next step.
- The file keeps the `schema: "rerail.packs/1"` shape already in use.
