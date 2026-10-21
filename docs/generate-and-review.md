# Generating and reviewing port readiness answers

The [port readiness gates](port-readiness.md) judge every ported rule against upstream's answers: what the upstream
tool reports on its own tests and on the real-code files the goldens name. A tool-pack's **generator** writes those
answers. `npm run generate` (`conformance/recorders/port-readiness/expected-cli.ts`) runs the pinned upstream through
the tool-pack's live adapter (`conformance/recorders/port-readiness/adapters/`), in the runtimes
`conformance/recorders/port-readiness/runtimes/` pins, and records the tool's facts, each rule's facts and names, each
upstream test file's expected findings, the test texts that are no file of their own, and upstream's findings on the
goldens' files. The gates never run upstream; they read what the generator wrote.

Nothing the generator produces is committed. The answers are large, rewritten whenever upstream or the generator
moves, and hold excerpts of upstream's tests, so they live in Rerail's private Azure Blob Storage, under each
upstream's licence together with Rerail's Apache-2.0 (each `packs/<pack>/licenses/<project>/UPSTREAM.md` says so).
The one thing the generator still writes into the tree is a case source a tool-pack's `fixtures/` lacks, with its
`UPSTREAM.md` row.

## Where the answers live

The answers of one tool-pack are one blob, keyed by

```text
<pack>/<tool>/<upstream version>/<inputs digest>
```

- **upstream version** is `gates.yaml`'s `upstream.version` (`0.16.9` for Ruff).
- **inputs digest** is a sha256 over every tracked file the generator reads: the generator's code (the import closure
  of `expected-cli.ts`, the engine sources it reaches included) and its data under
  `conformance/recorders/port-readiness/`, the runtimes the tool-pack installs, the locked npm packages the generator
  loads, and the tool-pack's own inputs (`gates.yaml`, `goldens.tsv`, `fixtures/`, the pack's `conformance.json`, its
  `licenses/*/UPSTREAM.md` and `conformance/port-readiness/corpus-tasks.json`). `npm run generate -- --key` prints the
  key and every file that went into it.

So an edit to a generator, a runtime pin, a manifest, a golden or a port list is a new key, and nothing else is: answers are
generated again exactly when something they depend on changed. The blob is `expected.json.gz`, a gzipped
`rerail.port-readiness-expected/v1` document holding every answer file by its path (`upstream.json`, `rules/`,
`cases/`, `texts/`, `corpus/`; [port-readiness.md](port-readiness.md#what-the-gates-read)), with its sha256, the
commit and the run that made it as metadata. A download whose bytes or key do not match is refused.

| Container | Holds | Written by |
| --- | --- | --- |
| `port-readiness-expected` | `<key>/expected.json.gz`, and `latest/<pack>/<tool>.json` naming main's current key | main's CI only |
| `port-readiness-expected-pr` | `pr-<n>/<key>/expected.json.gz`, a pull request's answers until review | pull requests' CI |

Locally, and in each CI leg, the answers are cached under `.rerail/port-readiness/expected/<key>/`, which the gates
read and nothing else. A gate run whose key is not cached fails with `answers for <key> are not available: run npm run
generate -- --pack <pack> --tool <tool>`.

## Run it

```sh
npm run generate -- --pack python --tool ruff                     # cached, else main's blob, else generate
npm run generate -- --pack python --tool ruff --force             # generate even if the answers exist
npm run generate -- --pack python --tool ruff --changes ruff.md   # also write the per-rule before/after against main
npm run port-readiness -- --pack python --tool ruff               # then judge the rules
```

`generate` prints the key and whether it read the local cache, downloaded the answers or generated them. Reading the
store needs the Azure CLI signed in (`az login`) with read access, and `RERAIL_EXPECTED_ACCOUNT` set to the storage
account; with it unset (and no `AZURE_STORAGE_CONNECTION_STRING`) the store is off, and a key the cache lacks is
generated. Generating needs `uv`, the network and the pinned runtimes; the `php` and `blade` tool-packs also need the
pinned PHP first on `PATH` (`conformance/recorders/port-readiness/runtimes/php/build.sh`), and `rust/clippy` needs
rustup. Clippy's answers on the corpus depend on the host (a `#[cfg(target_os = "linux")]` item is compiled on Linux
alone), and CI on Linux is the only uploader, so download them rather than trust a macOS generation.

A generation that finds a case source missing from `fixtures/` writes it, with its `UPSTREAM.md` row, and keys the
answers on the tree as it then is; commit both. CI never writes into the tree: with `--upload` or `--promote` such a
generation fails instead. `--upload pr:<n>|main`, `--promote`, `--no-generate` (exit 3 rather than generate on a
miss), `--drift` and `--key` are CI's (below); `--cache <dir>` moves the cache root.

## What CI does

`port-readiness.yml` runs `plan`, then one `answers` job per planned tool-pack, then the gate legs, then `report`.

On a **pull request**, a tool-pack's `answers` job computes the key, restores the answers from the Actions cache by
it, signs in to Azure with the pull request identity (OIDC) and downloads them from the store. On a miss it installs
the pinned runtimes and generates them in that job (`npm run generate -- --upload pr:<n> --changes ...`), uploading
them under `pr-<n>/` in `port-readiness-expected-pr`. It hands the answers to the gate legs as an artifact, so no gate
leg runs upstream's code. It saves the Actions cache only for answers it downloaded and verified, never for answers
it generated, since generating runs upstream's code and the Composer packages it resolves. The report job posts the
gates' results and, for every tool-pack whose key differs from main's, a comment with the per-rule before/after
against main's current answers (expected findings gained and lost on upstream's own tests, corpus files answered
differently, facts changed), collapsing that tool-pack's earlier ones. A fork's pull request gets no Azure token: its
`answers` jobs generate without uploading, and no before/after is posted.

**Review.** The before/after is the diff of the answers, which are not in the pull request's. `@neralabs/platform-leads`
own the generators, the gates, `packs/*/upstream/`, each pack's `conformance.json` and the port readiness workflows
(`.github/CODEOWNERS`), and approve every change to them: read the generator change and the before/after together,
and make sure every moved answer is explained by it.

**A failing shipped rule is fixed, never moved back.** When new answers make a shipped rule fail, fix the rule (or the
engine) until it passes. Moving it back to `candidates/` to turn CI green is not an option: a review that sees it
sends it back.

On the **push to main**, `ci.yml` promotes before it judges. `promote`, with main's identity, runs `npm run generate
-- --promote --no-generate --pr <n>` for every gated tool-pack, the merged pull request found from the commit: a key
main already has moves nothing, and a key the pull request uploaded is copied from `pr-<n>/` into
`port-readiness-expected` once its sha256 checks out. `promote-generate` generates, with no credentials, the tool-packs
neither container has (a fork's, or a key that changed with the merge), and `promote-upload` uploads them with main's
identity, so no job holding that identity runs upstream's code. Each then writes `latest/<pack>/<tool>.json`, the key
and commit later pull requests compare with. The `answers` jobs of a push to main never generate: a key main lacks
there fails, since promotion should have put it there.

**Nightly**, `port-readiness-nightly.yml` generates every tool-pack again with `--force` and no credentials
(`drift-generate`), then compares each with main's stored blob (`drift`, `--drift`): an upstream whose answers moved
under unchanged pins (a transitive package release, a runtime that resolves differently) fails and reports the
difference. Pin what moved and generate again in a pull request. Then it judges every rule uncached.

## Admin setup

Done once, by an Azure and repository admin:

1. **Storage account** with public blob access off, and the two containers:

   ```sh
   az storage account create -n <account> -g <group> --sku Standard_LRS --allow-blob-public-access false --min-tls-version TLS1_2
   az storage container create --account-name <account> --auth-mode login -n port-readiness-expected
   az storage container create --account-name <account> --auth-mode login -n port-readiness-expected-pr
   ```

2. **OIDC identities.** One Entra app per identity (or one app with both credentials), each with a federated
   credential from GitHub (issuer `https://token.actions.githubusercontent.com`, audience `api://AzureADTokenExchange`):
   subject `repo:neralabs/rerail:pull_request` for the pull request identity, `repo:neralabs/rerail:ref:refs/heads/main`
   for main's.

3. **RBAC, per container**, so a pull request cannot write main's answers even by editing its own workflow:

   | Identity | `port-readiness-expected` | `port-readiness-expected-pr` |
   | --- | --- | --- |
   | pull request | Storage Blob Data Reader | Storage Blob Data Contributor |
   | main | Storage Blob Data Contributor | Storage Blob Data Reader |
   | developers | Storage Blob Data Reader | Storage Blob Data Reader |

   Scope each assignment to the container:
   `/subscriptions/<subscription>/resourceGroups/<group>/providers/Microsoft.Storage/storageAccounts/<account>/blobServices/default/containers/<container>`.

4. **Repository variables** (Settings → Secrets and variables → Actions → Variables): `AZURE_CLIENT_ID_PR`,
   `AZURE_CLIENT_ID_MAIN`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID` and `RERAIL_EXPECTED_ACCOUNT`.

5. **Branch protection** on `main` requires review from code owners, so `@neralabs/platform-leads` approve every
   change `.github/CODEOWNERS` gives them.
