# Runbook: publishing a new pack to npm

Use this when `@neralabs/rerail-pack-<id>` has never been published: the Release packs run on `main` fails at
**First publish needed** and names the pack.

npm only allows a trusted publisher on an existing package, so you publish the first version by hand. The release
workflow publishes the rest ("Release" in [pack-format.md](pack-format.md)). Allow about 30 minutes, mostly the
native build.

<details>
<summary>Where this fits in a new pack's way to npm</summary>

This runbook is the last step:

1. The pack is written and passes the validation contract
   ([language-packs.md, "Start a pack"](language-packs.md#start-a-pack)).
2. A ported pack's test data and samples, staged in the pull request, are published by **Publish pack data** once a
   code owner approves ([upstream.md, "Publishing pack data"](upstream.md#publishing-pack-data)).
3. Ports start as candidates and are promoted into `rules/` once they pass the conformance check
   ([conformance/README.md](../conformance/README.md#candidates-and-promotion)).
4. The pull request merges with its changeset, and the Release packs run on `main` asks for the first publish: this
   runbook.

</details>

## Before you start

Confirm the pack is ready and your accounts can publish it.

- **Not private:** its `package.json` has no `"private": true` (remove it, add `publishConfig` with `access`
  `public`, and add the pack's changeset in one pull request).
- **Merged** on `main` at `0.1.0` with a `patch` changeset, and listed in `src/engine/catalog/pack-list.generated.ts`.
- **Pack data published** (ported packs only), and shipped rules pass the conformance check.
- **AWS** (ported packs only): read access to the test data bucket.
- **npm:** `npm whoami` prints a `neralabs` member with publish rights; else `npm login` interactively with 2FA.
- **GitHub:** `gh auth status` shows write access to `neralabs/rerail`.
- **Rust** on `PATH`, for the native build: `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Package missing:** this prints a 404:
  ```sh
  npm dist-tag ls @neralabs/rerail-pack-<id>
  ```

<details>
<summary>Details for each check</summary>

- **Merged:** see "Versioning and release" in [pack-format.md](pack-format.md). Regenerate the pack list with
  `npm run generate:packs`; check 7 fails an unlisted pack. Set-up fetches only listed packs.
- **Pack data:** each tool-pack's `test-data.lock`, and any `samples.lock`, names an archive S3 holds, and no
  `publish/` is left on `main`. Both are [CI checks](upstream.md#ci). A pack with no ported rules has no pack data.
- **Conformance:** shipped rules pass the [conformance check](../conformance/README.md#the-gate). Ports still under
  evaluation stay in `candidates/`, which is never published.
- **AWS:** `aws sso login`, or any AWS credentials that can read `s3://nera-test-fixtures/rerail/archives/`. The
  publish gate validates the pack on its test data.
- **npm:** a token that bypasses 2FA cannot set up a trusted publisher.
- **GitHub:** the tag script creates the release tag.

</details>

## 1. Check out `main` cleanly

Publish from a clean, pushed `main` commit in a temporary worktree, which leaves your own checkout as it is.

```sh
git fetch origin main
git worktree add --detach /tmp/rerail-pack-publish origin/main
cd /tmp/rerail-pack-publish
npm ci
npm run build:native     # about 5 minutes from a cold cache
npm run fixtures:pull -- --pack <id>   # a ported pack's test data, which the publish gate reads
```

<details>
<summary>Why `main` and a clean checkout</summary>

The tag script tags HEAD as `@neralabs/rerail-pack-<id>@<version>`. It refuses a checkout with changes, or a
commit that isn't on GitHub. Don't publish from a feature branch: a squash merge drops the branch's commits, so the
tag would point at a commit `main` never had.

</details>

## 2. Run the publish gate and pack

Validate the packs and build their tarballs; never publish around a failed gate.

```sh
node --import tsx scripts/release/packs-pack.ts --out /tmp/rule-packs <id> [<id> ...]
```

It runs the release workflow's validation, then writes each tarball and a `manifest.json` to `/tmp/rule-packs`. On
failure, fix the pack in a pull request to `main` and restart at step 1.

## 3. Tag and publish

Tag each release, then publish the tarballs to npm, as the release workflow does.

```sh
node scripts/release/packs-tag.mjs /tmp/rule-packs
node scripts/release/packs-publish.mjs /tmp/rule-packs
```

Approve the "Authenticate your account at: <url>" link in the browser. One approval covers every pack in the run.
Reruns after a failure are safe.

<details>
<summary>What the scripts do, and how reruns work</summary>

- **The tag script** pushes each release tag to GitHub. On a rerun, a version already tagged is skipped.
- **The publish script** publishes each pack public under the `latest` dist-tag, then points `next` at it (`next`
  never trails `latest`). It waits on nothing npm does, so it doesn't check the package is public: step 4 does. On a
  rerun, a version npm already has has its `next` raised, not published again. A version older than the pack's
  `latest` is never published, since that would move `latest` back.
- If the publish fails after tagging, run the publish script again. Later versions don't depend on it: the release
  workflow publishes any version that is tagged but not on npm.

</details>

## 4. Check the result

Confirm the publish, then give the package the same access as core.

```sh
npm dist-tag ls @neralabs/rerail-pack-<id>        # latest: <version>, next: <version>
npm access get status @neralabs/rerail-pack-<id>  # public
git ls-remote --tags origin "refs/tags/@neralabs/rerail-pack-<id>@*"
```

`npm view` can 404 for several minutes after a first publish; `npm dist-tag ls` sees the package at once.

Whoever reads core must read every pack, or set-up can't fetch it for them. Give the new package core's teams with
the same permission (today `developers`, read-write, as on every pack), then compare the lists:

```sh
npm access grant read-write neralabs:developers @neralabs/rerail-pack-<id>
npm access list collaborators @neralabs/rerail
npm access list collaborators @neralabs/rerail-pack-<id>
```

<details>
<summary>Later versions and the CI read token</summary>

Later versions go to `latest` too, published by the release workflow when the Release pull request that bumps them
merges.

CI's **Rule packs installed from npm** job reads the packs with the repository secret `NPM_PACKS_READ_TOKEN`, a
read-only granular token. If that token names its packages one by one rather than the whole `@neralabs` scope, add
the new package to it on npmjs.com.

</details>

## 5. Set up the trusted publisher

Let the release workflow publish later versions without a token.

In the npmjs.com web UI, open `https://www.npmjs.com/package/@neralabs/rerail-pack-<id>/access`. Under
**Trusted Publisher**, choose **GitHub Actions**:

| Field | Value |
|---|---|
| Organization or user | `neralabs` |
| Repository | `rerail` |
| Workflow filename | `release-packs.yml` |
| Environment name | `release` |
| Allowed operations | **npm publish** and **npm dist-tag** |

Save. Under **Publishing access**, choose **Require two-factor authentication and disallow tokens**, then **Update
Package Settings**. Each save asks for 2FA.

Verify (each run asks for browser approval):

```sh
npx -y npm@latest trust list @neralabs/rerail-pack-<id> --json
```

<details>
<summary>What to expect, and why not the CLI</summary>

The output must show `"repository": "neralabs/rerail"`, `"file": "release-packs.yml"` and
`"environment": "release"`, with permissions including `createPackage` and `manageDistTags`. No npm command reads
the Publishing access setting, so check that on the web page.

Use the web UI, not `npm trust github` (npm 12): the CLI can't grant the dist-tag permission, and the workflow's
promote job needs it.

</details>

## 6. Let a publish use it within 2 days

The trusted publisher setting expires unless a publish uses it within 2 days.

The workflow publishes a pack only when a **Release** pull request that bumps it merges.

- **A Release pull request bumps it:** merge it within the 2 days.
- **None does:** add a `patch` changeset naming the pack (`npx changeset`, named `changed-…`), merge it, then merge the
  Release pull request it updates.
- **Approve** the `release` environment when asked.

<details>
<summary>Checking the run, and if the 2 days pass</summary>

- "None does" covers, for example, `main` already having the version you published by hand.
- After approval, check that the **Publish to npmjs** job published the new version and that the **Published
  dist-tags** job passed: `latest` is that version.
- If the 2 days pass first, delete the setting on npmjs.com and add it again just before you merge.

</details>

## 7. Clean up

Remove the temporary worktree and tarballs.

```sh
cd -
git worktree remove --force /tmp/rerail-pack-publish
rm -rf /tmp/rule-packs
```

## Troubleshooting

Find the message you see in the table; the fix is beside it.

<details>
<summary>Messages and fixes</summary>

| Symptom | Cause and fix |
|---|---|
| `the checkout has changes; tag from a clean checkout of the commit to release` | Changed or untracked files in the worktree. Restart at step 1. |
| `commit <sha> is not on GitHub, or gh cannot read the repository` | Not on a pushed commit, or `gh` isn't logged in. Use `origin/main` as in step 1; check `gh auth status`. |
| `npm access get status` in step 4 prints `private` | npm keeps the package private. Run `npm access set status=public <name>`, then check again. |
| `<name>@<version> is older than latest <latest>, so it was not published` | A newer version is already `latest` and supersedes this one. Nothing to do; publishing it would move `latest` back. |
| `EOTP` or an "Authenticate your account" URL | npm needs 2FA. Approve the URL in the browser; the command carries on. |
| The gate fails to load the native engine | Run `npm run build:native` with `~/.cargo/bin` on `PATH`. |
| The gate says a tool-pack's test data is missing and to run `npm run fixtures:pull` | The worktree has no pack data. Run `npm run fixtures:pull -- --pack <id>` with read access to the bucket, then step 2 again. |
| `npm view` returns 404 just after publishing | The registry hasn't caught up. Use `npm dist-tag ls`. |
| The workflow's publish fails with an authentication error for one pack | Trusted publisher missing, expired or mistyped (often the environment name). Redo step 5. |
| A release run fails at **First publish needed** | A pack has never been published. Start this runbook from the top. |

</details>
