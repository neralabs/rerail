# Runbook: publishing the engine packages to npm for the first time

Use this once, before the first release whose core depends on the engine packages, and again for an engine package
added later (a new operating system or processor): the Release run's publish job fails at **First publish needed**
and names each package never published.

npm only allows a trusted publisher on an existing package, so you publish each name's first version by hand: a
placeholder `0.0.0` that holds no engine. The release workflow publishes every real version
([release.md, "Engine packages"](release.md#engine-packages)). The first run, on 2026-11-24, took about 20 minutes for
the six.

**Do this at most 2 days before that release is merged.** A trusted publisher setting expires unless a publish uses it
within 2 days ([step 4](#4-let-a-release-use-it-within-2-days)).

<details>
<summary>Why a placeholder, and why version 0.0.0</summary>

- **Every real version comes from the release workflow,** with provenance, as the bytes core pins. A real engine
  published by hand would have neither, and the release of that version could not finish: npm never replaces a
  published version, and the workflow refuses an engine package whose bytes are not the ones it packed.
- **`0.0.0` is below every release,** and no core names it: core pins its own exact version. Each package's `latest`
  stays on `0.0.0` until the first promotion moves it ([release.md](release.md#promotion-and-put-back)).
- **A name is permanent once published.** Until then it exists only in the table `ENGINE_PACKAGES` in
  `scripts/release/engine/packages.mjs`, which this runbook reads, so the names you publish are the names the release
  publishes.

</details>

## Before you start

Confirm the names are final and your account can publish them.

- **Merged:** `scripts/release/engine/packages.mjs` is on `main`, and its table holds the names the team agreed on.
- **npm:** `npm whoami` prints a `neralabs` member with publish rights; else `npm login` interactively with 2FA.
- **Node.js 24** or newer.
- **Packages missing:** this prints a 404 for a name never published (a 401 when npm is not logged in):
  ```sh
  npm dist-tag ls @neralabs/rerail-engine-<suffix>
  ```

<details>
<summary>Details for each check</summary>

- **Names:** `@neralabs/rerail-engine-<suffix>`, one per row of the table
  ([the six](installation.md#registry-access)). A name that is wrong after step 2 cannot be renamed, only deprecated
  (`npm deprecate`) beside a new one.
- **npm:** a token that bypasses 2FA cannot set up a trusted publisher.
- **Some published already:** skip those in step 2, and do steps 3 and 4 for all six.

</details>

## 1. Make the placeholders

Write each placeholder from the release's own table, in a temporary worktree of `main`, which leaves your own checkout
as it is.

```sh
git fetch origin main
git worktree add --detach /tmp/rerail-engine-first-publish origin/main
cd /tmp/rerail-engine-first-publish
node --input-type=module <<'EOF'
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { ENGINE_PACKAGES, enginePackageJson } from "./scripts/release/engine/packages.mjs";

const out = "/tmp/rerail-engine-placeholders";
const core = { ...JSON.parse(readFileSync("package.json", "utf8")), version: "0.0.0" };
for (const enginePackage of ENGINE_PACKAGES) {
  const folder = join(out, enginePackage.suffix);
  mkdirSync(folder, { recursive: true });
  writeFileSync(join(folder, "package.json"), `${JSON.stringify({ ...enginePackageJson(enginePackage, core), description: "A placeholder that reserves this name: it holds no engine." }, null, 2)}\n`);
  writeFileSync(join(folder, "README.md"), `# ${enginePackage.name}\n\nA placeholder that reserves this name: it holds no engine. Install [\`${core.name}\`](https://www.npmjs.com/package/${core.name}), which depends on the engine package of its own version.\n`);
  copyFileSync("LICENSE", join(folder, "LICENSE"));
}
writeFileSync(join(out, "names.txt"), `${ENGINE_PACKAGES.map(({ name }) => name).join("\n")}\n`);
EOF
```

It needs no `npm ci`. Read one `package.json` before publishing: the name, `"version": "0.0.0"`, `os`, `cpu`, `libc`
on the two Linux packages, and `repository` naming `neralabs/rerail`.

<details>
<summary>What the placeholder holds</summary>

`/tmp/rerail-engine-placeholders/<suffix>/` holds a `package.json`, a `README.md` and the repository's `LICENSE`, and
`names.txt` lists the six names for the later steps. The `package.json` is the one a real version gets
(`enginePackageJson`), at version `0.0.0` and with its own description: no scripts, no dependencies, and
`publishConfig.access` set to `public`. A real version also carries `NOTICE` and `THIRD_PARTY_NOTICES`, for the
third-party code the engine links; the placeholder holds no engine, so it carries only the license its `package.json`
names.

</details>

## 2. Publish each placeholder

Publish the six folders; npm asks for 2FA once.

```sh
for folder in /tmp/rerail-engine-placeholders/*/; do
  npm publish "$folder" --access public
done
```

Approve the "Authenticate your account at: <url>" link in the browser; on the first run one approval covered all six
publishes. Run the loop in a terminal, not piped or redirected: as with `npm trust list` in step 3, npm otherwise fails
with `EOTP` instead of waiting for the approval. Do not pass `--provenance`: only a publish from CI can carry one. A rerun after a failure is safe: npm
refuses a name it already has at `0.0.0`, and the loop goes on to the next.

Each publish ends with "Your package is being processed and may take a few minutes to become available." Until it is,
`npm dist-tag ls` shows `latest: 0.0.0-stage`; on the first run one package took several minutes longer than the
rest.

## 3. Check the result, and set up each trusted publisher

Confirm each publish and that each package has the access core has, then let the release workflow publish to it.

```sh
npm access list collaborators @neralabs/rerail
for name in $(cat /tmp/rerail-engine-placeholders/names.txt); do
  npm dist-tag ls "$name"                       # latest: 0.0.0, once npm has processed it
  npm access get status "$name"                 # public
  npm access list collaborators "$name"         # the same list as core's
done
```

npm gives the organization's `developers` team read-write on every package created under `@neralabs`, which is the
access core has, so the lists match with no grant. `npm view` can 404 for several minutes after a first publish;
`npm dist-tag ls` sees the package at once.

### Set up the trusted publisher

For each of the six, open `https://www.npmjs.com/package/@neralabs/rerail-engine-<suffix>/access` in the npmjs.com web
UI. Under **Trusted Publisher**, choose **GitHub Actions**:

| Field | Value |
|---|---|
| Organization or user | `neralabs` |
| Repository | `rerail` |
| Workflow filename | `release.yml` |
| Environment name | `release` |
| Allowed operations | **npm publish** and **npm dist-tag** |

Save. Under **Publishing access**, choose **Require two-factor authentication and disallow tokens**, then **Update
Package Settings**. Each save asks for 2FA.

Verify; npm asks for one browser approval. Every engine package's entry must equal core's. Run it in a terminal,
without `--json` and not piped: `--json`, or output that is not a terminal, makes npm fail with `EOTP` instead of
waiting for the approval.

```sh
for name in @neralabs/rerail $(cat /tmp/rerail-engine-placeholders/names.txt); do
  echo "== $name"
  npx -y npm@latest trust list "$name"
done
```

<details>
<summary>What to expect, why six entries, and why not the CLI</summary>

- Each entry must show `type: github`, `file: release.yml`, `repository: neralabs/rerail`, `environment: release` and
  `permissions: publish, stage publish, manageDistTags`, as core's does; only its `id` differs. No npm command reads
  the Publishing access setting, so check that on the web page.
- **The workflow is `release.yml`,** core's, not the packs' `release-packs.yml`: one release run publishes the six
  engine packages and then core, and its promotion moves all seven `latest` dist-tags.
- **Six entries, not one.** A run has one identity, and npm checks it against each package's own entry. A package
  without one fails its publish with an authentication error, and the release stops before core is published.
- Use the web UI, not `npm trust github` (npm 12): the CLI can't grant the dist-tag permission, and the workflow's
  promote job needs it.

</details>

## 4. Let a release use it within 2 days

The trusted publisher setting expires unless a publish uses it within 2 days, and one release run uses all six.

- **Merge the Release pull request** that releases core within the 2 days, and approve the `release` environment when
  asked.
- **Or push a preview first:** set a `<version>-next.1` version and push its `v<version>-next.1` tag by hand
  ([release.md](release.md#tags-and-channels)). It publishes the seven packages to `next` and nothing reaches `latest`.

<details>
<summary>Checking the run, and if the 2 days pass</summary>

- After approval, check that the **Publish to npmjs** job published the six engine packages and core, and that the
  **Published dist-tags** job passed: each package's `next` is that version.
- Then check an install: `npm view @neralabs/rerail@next optionalDependencies` lists the six at core's version, and
  after `rerail update --channel next`, `rerail status` shows `Engine: ok`.
- If the 2 days pass first, delete each setting on npmjs.com and add it again just before you merge.

</details>

## 5. Clean up

Remove the temporary worktree and the placeholders.

```sh
cd -
git worktree remove --force /tmp/rerail-engine-first-publish
rm -rf /tmp/rerail-engine-placeholders
```

## Troubleshooting

Find the message you see in the table; the fix is beside it.

<details>
<summary>Messages and fixes</summary>

| Symptom | Cause and fix |
|---|---|
| An "Authenticate your account" URL | npm needs 2FA. Approve the URL in the browser; the command carries on. |
| `EOTP`, "This operation requires a one-time password" | npm could not wait for the approval: the command ran with `--json`, or its output was piped or redirected. Run it again in a terminal without either. |
| `npm dist-tag ls` shows `latest: 0.0.0-stage` | npm is still processing the publish. Wait a few minutes and check again. |
| An engine package's collaborators differ from core's | The `developers` team lacks read-write on it. An owner of the organization runs `npm access grant read-write neralabs:developers <name>`; for anyone else npm answers `E403`. |
| `npm publish` is refused with a permission error (`E403`) | Your npm account cannot publish under `@neralabs`. Ask an owner of the organization for publish rights, then rerun the loop. |
| `npm publish` says it cannot publish over `0.0.0` | That name already has its placeholder. Nothing to do. |
| `npm access get status` in step 3 prints `private` | npm keeps the package private. Run `npm access set status=public <name>`, then check again. |
| `npm view` returns 404 just after publishing | The registry hasn't caught up. Use `npm dist-tag ls`. |
| The release's publish fails with an authentication error for one engine package | Trusted publisher missing, expired or mistyped (often the workflow or the environment name). Redo step 3 for that package, then re-run the release's failed job ([release.md](release.md#a-publish-that-stops-between-packages)). |
| A release run fails at **First publish needed** | An engine package has never been published. Nothing was published by that run. Do this runbook for the names it lists, then re-run the failed job. |
| A request for an engine package's dist-tags, made without a login, answers 401 | npm answers 401, not 404, for the dist-tags of a scoped name it has never had. It is not a sign-in fault: the release reads it as never published (**First publish needed**). |
| A name was published wrong | It cannot be renamed. Fix the table in a pull request, publish the right name with this runbook, and `npm deprecate` the wrong one. |

</details>

## Not confirmed yet

Steps 1 to 3 and 5 were run for the six packages on 2026-11-24, with npm 11.16, and corrected from that run. Step 4
waits on the first release that publishes them.

<details>
<summary>What rests on the pack runbook, and what on nothing yet</summary>

Taken from [publishing-a-new-pack.md](publishing-a-new-pack.md), which has been run for packs, and not checked again
for these packages:

- that a trusted publisher setting expires after 2 days without a publish;
- that `npm trust github` cannot grant the dist-tag permission.

Not known:

- that npmjs accepts an engine package's 35 to 42 MB tarball from the release workflow, and that its provenance and
  `npm audit signatures` pass for these names: only the first release run shows it. Publish a preview first
  (step 4) to find out before a plain version is tagged.

</details>
