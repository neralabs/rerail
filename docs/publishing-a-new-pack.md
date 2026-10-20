# Runbook: publishing a new pack to npm

Use this when a pack's package, `@neralabs/rerail-pack-<id>`, has never been published. You can tell because the
Release packs run on `main` fails at **First publish needed** and names the pack.

npm only lets you set up a trusted publisher for a package that already exists. So the first version is published by
hand, and the release workflow publishes every version after it (see "Release" in [pack-format.md](pack-format.md)).
Allow about 30 minutes; most of it is the native build.

## Before you start

- The pack is merged on `main`, added at `0.1.0` with a `patch` changeset (see "Versioning and release" in
  [pack-format.md](pack-format.md)), and listed in `src/engine/catalog/pack-list.generated.ts` (`npm run
  generate:packs`; check 7 fails a pack that is not), so set-up fetches it.
- `npm whoami` prints your account, a member of the `neralabs` org with publish rights. If not, run `npm login`. It
  must be an interactive login with 2FA, because a token that bypasses 2FA cannot set up a trusted publisher.
- `gh auth status` shows a login with write access to `neralabs/rerail`. The publish script creates the release tag.
- Rust is on `PATH`, for the native build: `export PATH="$HOME/.cargo/bin:$PATH"`.
- Check that the package really is missing. This prints a 404:
  ```sh
  npm dist-tag ls @neralabs/rerail-pack-<id>
  ```

## 1. Check out `main` cleanly

The publish script tags the commit you publish from (HEAD) as `@neralabs/rerail-pack-<id>@<version>`. It refuses a
checkout with changes, or a commit that isn't on GitHub. Publish from `main`, not from a feature branch: a squash
merge drops the branch's commits, and the tag would then point at a commit that `main` never had. A temporary
worktree keeps your own checkout as it is:

```sh
git fetch origin main
git worktree add --detach /tmp/rerail-pack-publish origin/main
cd /tmp/rerail-pack-publish
npm ci
npm run build:native     # about 5 minutes from a cold cache
```

## 2. Run the publish gate and pack

```sh
node --import tsx scripts/release/packs-pack.ts --out /tmp/rule-packs <id> [<id> ...]
```

This runs the same pack validation the release workflow runs, then writes each pack's tarball and a `manifest.json` to
`/tmp/rule-packs`. If it fails, fix the pack in a pull request to `main` and start again from step 1. Never publish
around a failed gate.

## 3. Publish

```sh
node scripts/release/packs-publish.mjs /tmp/rule-packs
```

- npm prints "Authenticate your account at: <url>". Open the URL and approve it. One approval covers every pack in
  the run.
- For each pack the script publishes it as restricted under the `next` dist-tag, checks that an anonymous reader
  can't see it, and pushes the release tag to GitHub.
- It's safe to run again after a failure. A version that's already tagged is skipped. A version npm already has but
  that has no tag is checked and tagged, not published again.

## 4. Check the result

```sh
npm dist-tag ls @neralabs/rerail-pack-<id>        # latest: <version>, next: <version>
npm access get status @neralabs/rerail-pack-<id>  # private
git ls-remote --tags origin "refs/tags/@neralabs/rerail-pack-<id>@*"
```

- `latest` is set as well as `next`. npm sets `latest` on a package's first publish, whatever `--tag` says, so the
  first version goes straight to `latest`. Later versions go to `next` until they're promoted.
- `npm view` can keep returning 404 for several minutes after a first publish. Use `npm dist-tag ls` instead, which
  sees the package straight away.

Whoever can read core must be able to read every pack, or set-up fails to fetch it for them. Give the new package
the teams core has, with the same permission (today `developers`, read-write, as on every pack), and compare the two
lists:

```sh
npm access grant read-write neralabs:developers @neralabs/rerail-pack-<id>
npm access list collaborators @neralabs/rerail
npm access list collaborators @neralabs/rerail-pack-<id>
```

CI's **Rule packs installed from npm** job reads the packs with the repository secret `NPM_PACKS_READ_TOKEN`, a
read-only granular token. If that token names its packages one by one rather than the whole `@neralabs` scope, add the
new package to it on npmjs.com.

## 5. Set up the trusted publisher

Use the npmjs.com web UI, at `https://www.npmjs.com/package/@neralabs/rerail-pack-<id>/access`. `npm trust github`
(npm 12) can't grant the dist-tag permission, and the workflow's promote job needs it.

Under **Trusted Publisher**, choose **GitHub Actions** and fill in:

| Field | Value |
|---|---|
| Organization or user | `neralabs` |
| Repository | `rerail` |
| Workflow filename | `release-packs.yml` |
| Environment name | `release` |
| Allowed operations | **npm publish** and **npm dist-tag** |

Save it, then under **Publishing access** choose **Require two-factor authentication and disallow tokens** and click
**Update Package Settings**. Each save asks for 2FA.

To check it, run the command below. Each run asks you to approve in the browser.

```sh
npx -y npm@latest trust list @neralabs/rerail-pack-<id> --json
```

The output must show `"repository": "neralabs/rerail"`, `"file": "release-packs.yml"` and `"environment": "release"`,
and its permissions must include `createPackage` and `manageDistTags`. npm has no command that reads the Publishing
access setting, so check that one on the web page.

## 6. Let a publish use it within 2 days

A trusted publisher setting expires unless a publish uses it within 2 days. The workflow publishes a pack only when a
**Version packs** pull request that bumps it is merged.

- If an open Version packs pull request already bumps the pack, merge it within the 2 days.
- If not (for example, the version you published by hand is already the one on `main`), add a `patch` changeset that
  names the pack (`npx changeset`), merge it, then merge the Version packs pull request it updates.
- When the release run asks for approval of the `release` environment, approve it. Then check that the **Publish to
  npmjs** job published the new version and that `npm dist-tag ls` shows `next` at that version.
- If the 2 days pass first, delete the setting on npmjs.com and add it again just before you merge.

## 7. Clean up

```sh
cd -
git worktree remove --force /tmp/rerail-pack-publish
rm -rf /tmp/rule-packs
```

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `the checkout has changes; publish from a clean checkout` | Untracked or changed files in the worktree. Start again from step 1. |
| `commit <sha> is not on GitHub, or gh cannot read the repository` | You're not on a pushed commit, or `gh` isn't logged in. Use `origin/main` as in step 1 and check `gh auth status`. |
| `<name> answers an anonymous request with 200: it is not restricted` | The package is public. Run `npm access set status=private <name>`, then run the script again. |
| `EOTP` or an "Authenticate your account" URL | npm needs 2FA. Approve the URL in the browser; the command carries on. |
| The gate fails to load the native engine | Run `npm run build:native` with `~/.cargo/bin` on `PATH`. |
| `npm view` returns 404 just after publishing | The registry hasn't caught up yet. Use `npm dist-tag ls`. |
| The workflow's publish fails with an authentication error for one pack | The pack's trusted publisher is missing, expired or has a typo (often the environment name). Redo step 5. |
| A release run fails at **First publish needed** | A pack has never been published. Start this runbook from the top. |
