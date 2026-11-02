# Runbook: publishing a new pack to npm

Use this when a pack's package, `@neralabs/rerail-pack-<id>`, has never been published. You can tell because the
Release packs run on `main` fails at **First publish needed** and names the pack.

npm only lets you set up a trusted publisher for a package that already exists. So the first version is published by
hand, and the release workflow publishes every version after it (see "Release" in [pack-format.md](pack-format.md)).
Allow about 30 minutes; most of it is the native build.

This runbook is the last step of a new pack's way to npm:

1. The pack is written and passes the validation contract
   ([language-packs.md, "Start a pack"](language-packs.md#start-a-pack)).
2. A ported pack's test data and samples are staged in the pull request and published by **Publish pack data** once
   a code owner approves ([upstream.md, "Publishing pack data"](upstream.md#publishing-pack-data)).
3. Its ports start as candidates and are promoted into `rules/` once they pass the conformance check
   ([conformance/README.md](../conformance/README.md#candidates-and-promotion)).
4. The pull request merges with its changeset, and the Release packs run on `main` then asks for the first publish:
   this runbook.

## Before you start

- The pack is merged on `main`, added at `0.1.0` with a `patch` changeset (see "Versioning and release" in
  [pack-format.md](pack-format.md)), and listed in `src/engine/catalog/pack-list.generated.ts` (`npm run
  generate:packs`; check 7 fails a pack that is not), so set-up fetches it.
- For a ported pack, its pack data is published: every tool-pack's `test-data.lock` (and `samples.lock`, if it has
  one) names an archive S3 holds, and no `publish/` is left on `main` (both are CI checks:
  [docs/upstream.md](upstream.md#ci)). Its shipped rules pass the
  [conformance check](../conformance/README.md#the-gate); ports still under evaluation stay in `candidates/`, which
  is never published. A pack with no ported rules has no pack data.
- For a ported pack, you can read the test data bucket (`aws sso login`, or any AWS credentials with read access to
  `s3://nera-test-fixtures/rerail/archives/`): the publish gate validates the pack on its test data.
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
npm run fixtures:pull -- --pack <id>   # a ported pack's test data, which the publish gate reads
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
- For each pack the script publishes it as public under the `latest` dist-tag, checks that an anonymous reader
  gets the version, points `next` at it too (`next` never trails `latest`), and pushes the release tag to GitHub.
- It's safe to run again after a failure. A version that's already tagged is skipped. A version npm already has but
  that has no tag is checked, has its `next` raised and is tagged, not published again.

## 4. Check the result

```sh
npm dist-tag ls @neralabs/rerail-pack-<id>        # latest: <version>, next: <version>
npm access get status @neralabs/rerail-pack-<id>  # public
git ls-remote --tags origin "refs/tags/@neralabs/rerail-pack-<id>@*"
```

- Later versions go to `latest` too, published by the release workflow when the Release pull request that bumps
  them is merged.
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
**Release** pull request that bumps it is merged.

- If an open Release pull request already bumps the pack, merge it within the 2 days.
- If not (for example, the version you published by hand is already the one on `main`), add a `patch` changeset that
  names the pack (`npx changeset`), merge it, then merge the Release pull request it updates.
- When the release run asks for approval of the `release` environment, approve it. Then check that the **Publish to
  npmjs** job published the new version and that the **Published dist-tags** job passed: `latest` is that version.
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
| `<name>@<version> answers an anonymous request with <status>: it is not public` | npm keeps the package private. Run `npm access set status=public <name>`, then run the script again. |
| `EOTP` or an "Authenticate your account" URL | npm needs 2FA. Approve the URL in the browser; the command carries on. |
| The gate fails to load the native engine | Run `npm run build:native` with `~/.cargo/bin` on `PATH`. |
| The gate says a tool-pack's test data is missing and to run `npm run fixtures:pull` | The worktree has no pack data. Run `npm run fixtures:pull -- --pack <id>` with read access to the bucket, then step 2 again. |
| `npm view` returns 404 just after publishing | The registry hasn't caught up yet. Use `npm dist-tag ls`. |
| The workflow's publish fails with an authentication error for one pack | The pack's trusted publisher is missing, expired or has a typo (often the environment name). Redo step 5. |
| A release run fails at **First publish needed** | A pack has never been published. Start this runbook from the top. |
