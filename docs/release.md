# Releasing Rerail

How core is built, versioned and published. Rule packs release separately; see [pack-format.md](pack-format.md#release).

## Release

`.github/workflows/release.yml` builds the package the updater installs with `.github/workflows/package.yml`. It compiles the native kernel for macOS, Linux glibc, and Windows MSVC on arm64 and x64 (the Linux kernels against glibc 2.17, with napi-rs's cross toolchain, and rejected if one needs a glibc newer than 2.28), smoke-tests native loading and device storage on each host, rejects a Windows kernel that imports the Visual C++ runtime (the kernels link it statically, so a clean Windows install needs no redistributable), builds the runtime and bundled UI, refuses to pack unless all six `native/*.node` files are present, installs the tarball into a scratch prefix, and checks `rerail --help`, the bundled UI, and `nativeHealth()`. Jobs on Windows x64, Linux x64 and macOS arm64 then install that tarball the way a user would (`scripts/host/windows-install-smoke.ps1` on Windows; the environment matrix's `scripts/host/azure/guest.sh` through `matrix.mjs local` on Linux and macOS) and exercise the managed runtime; nothing is published unless all three pass.

- Merging the **Release** pull request releases core. `.github/workflows/release-pr.yml` keeps that pull request up to date on every push to `main` (it also carries the rule pack bumps; [docs/pack-format.md](pack-format.md#release)): when anything core ships has changed since its last `v<version>` tag, it bumps `package.json` and `package-lock.json`'s root entries to the next patch version (`0.1.0` → `0.1.1`) and dates `CHANGELOG.md`'s unreleased section as that version. Once it is merged, the same workflow tags the merge commit `v<version>` with the `neralabs-ci` app's token, which starts `release.yml`. "Anything core ships" is every path but the packs, documentation, CI, changesets, tests, fixtures, corpora, benchmarks, development and release scripts, and repository files npm does not pack, and `package-lock.json` only when more than its pack entries moved (`NOT_SHIPPED` in `scripts/release/core-paths.mjs`).
- A `v<version>` tag, which must equal `package.json`'s version, publishes `@neralabs/rerail` to npmjs through npm trusted publishing from the protected `release` environment, so a reviewer approves each publish. The tag names the channel, which is the npm dist-tag: `v0.2.0` → `latest`, `v0.2.0-beta.1` → `beta`, `v0.2.0-next.1` → `next`; any other prerelease suffix is rejected. `latest` is stable; `beta` and `next` are opt-in through `rerail update --channel <tag>`. The Release pull request only makes plain versions, so they go straight to `latest`; a `beta` or `next` preview is still released by setting the version and pushing its tag by hand. After publishing, the run lists core's dist-tags in its summary and fails if the channel's is not the version just published.
- `workflow_dispatch` with no inputs builds and uploads the tarball as a run artifact without publishing. Install it with `npm install <tarball>` to try a build. With `latest` set to an earlier released version, it builds nothing and points `latest` back at that version, with trusted publishing behind the same approval, so no one needs an npm login to put core back.
- `.github/workflows/release-cache.yml` keeps the native builds' cargo caches warm. A tag's run can restore only caches saved on `main`, so on `main`, after a change to `Cargo.lock`, the Cargo manifests, `.cargo/config.toml` or `rust-toolchain.toml`, and nightly, each target builds from scratch and saves its cache when none matches. `package.yml` restores only that exact cache. After such a change, let the Release cache run finish before merging the Release pull request, or that release builds cold.
- `.github/workflows/env-matrix.yml` builds the same package after every push to `main` that can change what ships and installs it on throwaway Azure VMs (Ubuntu and Debian on x64 and arm64, RHEL on x64, Windows 11 on x64 and arm64) and GitHub-hosted macOS runners (macOS 26 and 15 on Apple silicon, macOS 15 on Intel), each the way a user meets it: `npx @neralabs/rerail setup`, then the managed runtime, PATH in a new shell, the coding-agent hooks, daemon restart and removal. [docs/environment-matrix.md](environment-matrix.md) covers the environments, what each checks, and running it by hand.

Release notes: a pull request that changes what core ships adds its entry under `## Unreleased` at the top of `CHANGELOG.md`, adding that heading if the last release took it. The Release pull request turns the heading into `## <version> (<date>)`, dated the day it was last updated from `main`; with no unreleased section, the release gets a section saying no entries were written. To release a minor or major version instead of a patch, name it in the heading: `## 0.2.0 (unreleased)`. The Claude, Codex, and Cursor plugin manifests carry no version in the checkout: `npm pack` stamps `package.json`'s version into them (`prepack`) and restores the source form afterwards (`postpack`). If a pack fails partway, `node scripts/build/stamp-plugin-versions.mjs restore` puts them back.

## Shrinkwrap

The tarball ships an `npm-shrinkwrap.json`, so every install of `@neralabs/rerail` gets the dependency versions and
integrity hashes this repository's `package-lock.json` pinned instead of resolving them fresh. npm honors a published
package's shrinkwrap; the registry marks a version that has one (`_hasShrinkwrap`). The checkout keeps
`package-lock.json` as its only lockfile and never commits a shrinkwrap.

- `npm pack` writes it first (`prepack`: `node scripts/release/shrinkwrap-pack.mjs write`) and deletes it afterwards
  (`postpack`), and `package.json`'s `files` lists it. It holds the root and every package the root's dependencies
  reach in `package-lock.json`: no development dependency, no workspace pack, and no optional peer nothing else needs.
  A package only optional dependencies reach is marked `optional`.
- Writing it fails when `package-lock.json`'s root dependencies are not `package.json`'s, or the lockfile lacks a
  package a dependency needs. Run `npm install` and commit the lockfile.
- `release.yml` checks the packed file before publishing (`node scripts/release/shrinkwrap-pack.mjs check
  <tarball>`). It fails the release when the tarball has none, or one that differs from what the tag's
  `package-lock.json` gives, and names the packages that differ.
- If a pack fails partway, the file stays in the checkout, and npm would then use it instead of `package-lock.json`.
  `node scripts/release/shrinkwrap-pack.mjs remove` deletes it.
- npm installs a shrinkwrapped package's dependencies inside it, below `node_modules/@neralabs/rerail/node_modules`,
  not beside it.

The managed install (`src/setup/updater.ts`) runs no install scripts (`--ignore-scripts`): no production dependency
needs one, since `better-sqlite3` ships prebuilt binaries. Before it runs any staged code, it checks the registry
signatures and provenance attestations of everything it installed with `npm audit signatures`, and stops when one
does not verify. A registry that publishes no signing keys, such as some private mirrors, gets a warning and the
install goes on. A new dependency that needs an install script would not work after `rerail update`, so check that
before adding one.
