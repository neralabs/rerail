# Release plan: packs on npm, install from scratch, recommended packs

Owner: Jeff. Started 2026-10-16. A working checklist, not a design doc: tick items off in place and delete the file once
everything below has shipped.

The goal is to install Rerail from npm on a clean machine, have set-up recommend the right rule packs for a
repository, install the packs the person confirms, and start Rerail with their rules on.

The order matters:
- Packs must be on npm before NER-1753 can be tested; its acceptance tests use real packages, not tarballs.
- Core has to be released after NER-1753, because the published core recommends nothing.

## 0. Precursors

- [x] `npm login` as a neralabs member with publish rights.
- [x] Delete the Tempo-era `~/.npmrc` backup. It pointed `@neralabs` at the old Azure feed, and the packages are on
      npmjs now.
- [x] Clean state:
  - [x] `just nuke-rerail --yes`. The backup is in `~/.rerail-clean-20261016`.
  - [x] Delete the per-checkout `.tempo` folders.
  - [ ] Decide whether to delete the `~/.tempo-clean-2026*` backups (about 9 GB).
- [x] Global Rerail from main with `just rerail-dev --install-bridge`. This registers every harness and runs the dev
      daemon on `~/.rerail`. Its docs said `npm run build`; #130 fixes them.
- [ ] Make `rerail` work without editing `PATH` by hand. See "Getting `rerail` onto PATH" below.
- [x] Core's `latest` tag: `latest` and `next` both point at `0.1.0-next.2`. See "npm dist-tags" below.

## 1. Publish the packs

- [x] Reject release-packs run 38816533631. It was from before #128 and would have published 0.2.0.
- [x] Approve run 38908654907. It published `complexity`, `next` and `typescript` at 0.1.1 under the `next` tag.
- [ ] Publish the nine packs that were never published by hand, as `docs/pack-format.md` describes under "Publishing a
      new pack". You need to be logged in to npm with 2FA.
  ```sh
  node --import tsx scripts/release/packs-pack.ts --out /tmp/rule-packs blade javascript node php python react rust sql vue
  node scripts/release/packs-publish.mjs /tmp/rule-packs
  ```
- [ ] On npmjs.com, set up a trusted publisher for each of those nine packs:
  - org `neralabs`, repository `rerail`, workflow `release-packs.yml`, environment `release`
  - allow publish and dist-tag
  - then turn on "Require 2FA and disallow tokens"
- [ ] Check that the trusted publishers for `complexity`, `next` and `typescript` are still configured.
- [ ] Within 2 days, merge a patch changeset that touches the nine new packs. An unused trusted publisher setting
      expires.
- [ ] Decide when packs move from `next` to `latest`. Use the release-packs manual run (`workflow_dispatch`, pack id).
- Leave #84 (Version packs) open. It collects the version bumps from changesets until the new flows are on main.
- #81 (Version packs in its own workflow, so CI does less work) is optional. Main is already merged into it.

## 2. Test the full install from scratch

- [ ] `just nuke-rerail --yes`, then `npx @neralabs/rerail setup` (or `just rerail`), in a repository that isn't
      rerail.
- [ ] Check that every agent is connected: Claude Code, Codex (run `/hooks` and trust the Rerail hook), Cursor, Pi and
      OMP.
- [ ] Check `/rerail-show-me` steers in at least one agent.
- Today, set-up says no pack fits. That's the known gap: core bundles only `complexity`, which has no `match`.
- `just rerail-smoke` runs the installer against a throwaway local registry, with no npm credentials.

## 3. NER-1753: core finds, installs and loads packs from npm

The plan is in `PLAN-NER-1753.md` (third draft, decisions of 2026-10-13). No code yet.

Scope:
- `npm run generate:packs` writes `pack-list.generated.ts`, with `{ id, packageName }` for each pack.
- Discovery fetches each listed pack into `~/.rerail/packs/staging/` and runs its `match` against the repository.
- `rerail packs recommend | add | remove | list`.
- Set-up installs packs only after the person confirms them.
- `match` is required on every pack, and CI enforces it.
- `dependencies` (npm, pypi, composer) replaces `npmDependencies`. Executable checks are allowed.
- `complexity` is no longer bundled with core; it becomes a pack like the others.

Settle before building:
- [ ] Should the build embed each pack's `match` in the generated list, or only the package names (as the plan says)?
  - Embedding means recommending runs with no npm calls, and only the confirmed packs are downloaded.
  - The cost is that new match logic ships only with a core release.
- [ ] The `javascript` pack is recommended for every TypeScript repository. Rename it, or narrow its match?
- [ ] `php` and `python` match rerail itself through `packs/*/upstream`. Exclude vendored upstream trees.

## 4. Release core with NER-1753

- [ ] Tag `v0.1.0-next.N`. The release workflow publishes it to the `next` channel.
- [ ] Move `latest` once the end-to-end check in 5–6 passes.

## 5. Recommended packs on sample repositories

- [ ] Use repositories that aren't rerail:
  - a TypeScript, React and Next.js app
  - a plain Node.js service
  - a Python (Django or FastAPI) app
  - a PHP and Laravel app with Blade
  - a Rust crate
  - a Vue app
  - a repository with SQL migrations
- [ ] For each repository, write down the packs it should get, then compare that with what set-up recommends and the
      reason it gives.

## 6. Packs install and Rerail starts

- [ ] Confirm the recommendation. Check:
  - the packs land in `~/.rerail/packs`
  - the daemon starts
  - the dashboard lists each pack's rules
  - no attachment shows `unavailable: missing`
- [ ] A violation in a pack rule steers the agent.
- [ ] `rerail packs add`, then `remove`. The attachment shows `missing` after remove and comes back on re-add.
- [ ] `rerail update` keeps the installed packs; they live outside `~/.rerail/versions`.

## Later

- Move the guard against minor and major pack bumps into `version:packs`. Today it runs only in `npm test`, and main
  requires no status checks.
- Raise the 180 s OMP timeout in CI. It also flakes on main.
- NER-1883 settings audit decisions, and the first-run wording audit (Notion draft).
- `rerail update` doesn't rewrite command files. Run `rerail install` after updating.

## Getting `rerail` onto PATH

Today set-up writes `~/.rerail/bin/rerail` and prints "add `~/.rerail/bin` to PATH". Hooks and slash commands use
absolute paths, so only people who type `rerail` in a shell need it on PATH.

Customers, best first:
1. **Set-up links the launcher into a directory already on PATH.** It picks the first writable one, such as
   `~/.local/bin` or `/opt/homebrew/bin`. It never uses sudo. `rerail uninstall` removes the link.
2. **Set-up offers to add `~/.rerail/bin` to the shell profile.** This is how rustup, bun and deno do it:
   - zsh, bash and fish, in one marked, idempotent block that uninstall removes
   - the user PATH on Windows
   - ask in the terminal lane, and never write it in `--json` mode
   - it only takes effect in new shells
3. **Homebrew tap (`brew install neralabs/tap/rerail`).** Brew's `bin` is already on PATH. It's a second channel to
   maintain, and the formula would have to defer to the managed updater.
4. **`npm i -g @neralabs/rerail`.** npm's global `bin` is usually on PATH, but it conflicts with the managed
   `~/.rerail/versions` updater. It would only make sense as a thin shim that runs `~/.rerail/bin/rerail`.
5. **Don't need PATH at all.** Slash commands cover daily use, and `npx @neralabs/rerail <command>` works for the
   rest.

Developers:
- `just rerail-dev --install-bridge` can do the same linking as option 1, since it already writes the launcher.
- `direnv` with `PATH_add ~/.rerail/bin` puts it on PATH only inside the checkout.

Recommendation: options 1 and 2 in `rerail install`/set-up, used by the npm path and the just path alike, plus a
`rerail doctor` line that says when the launcher isn't on PATH.

## npm dist-tags

- `npx @neralabs/rerail setup` installs whatever the `latest` dist-tag points at. `@next` installs the newest
  pre-release.
- The release workflow chooses the tag from the git tag:
  - `v1.2.3` goes to `latest`
  - `v1.2.3-<channel>.N` goes to `<channel>`, so `v0.1.0-next.2` went to `next`
- The managed updater follows the channel saved in `~/.rerail/update-state.json`, which defaults to `latest`. So
  `rerail update` only moves people when `latest` moves.
- Today `latest` and `next` both point at `0.1.0-next.2`; `latest` was moved by hand after the first-run work. While
  everything is pre-release, keep `latest` equal to the newest `next` that passed sections 2, 5 and 6. Move it with
  `npm dist-tag add @neralabs/rerail@<version> latest`.
- Packs follow the same model: `next` is 0.1.1 and `latest` is 0.1.0. NER-1753's discovery reads the person's update
  channel and falls back to `next` when a pack has nothing under `latest`.
