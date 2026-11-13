# Distribution model

## Decision

Rerail ships as one customer-neutral local runtime; a signed activation profile configures it per customer.

The sole channel is the npm package `@neralabs/rerail` on npmjs ([Registry](#registry)).

<details>
<summary>What the profile supplies and what runs where</summary>

The profile supplies customer identity, control-plane location, policy trust, release channel and entitlement; none of it is compiled into the package.

The engine and latency-sensitive evaluation run on the customer's machine. Network services only distribute, activate, renew entitlements and deliver signed policy; they are not in the evaluation path.

A customer download portal may be added later without changing the runtime artifact or activation model.

</details>

## System shape

All coding agents feed one local runtime, which talks to a customer control plane.

```text
Coding agents                    Customer environment              Nera infrastructure
-------------                    --------------------              -------------------
OMP / Pi -----------\
Claude Code ---------+---------> local Rerail runtime <-----------> customer control plane
Codex ---------------+           local engine                      (shared or dedicated)
Cursor --------------/                    ^
                                           |
                                           +----------------------- signed software updates
```

Adapters normalize native lifecycle events for that runtime. Customer-specific behavior lives in signed policy and configuration, never adapter forks.

## Distribution artifact

One immutable package version serves every customer and platform; customer-specific builds are prohibited by default.

```text
@neralabs/rerail@<version>
```

<details>
<summary>Package contents and why there are no customer builds</summary>

The package contains:

- OMP and Pi extensions.
- Transport-neutral lifecycle adapters for Claude Code, Codex, and Cursor.
- The persistent Rerail event stream.
- Supported native binaries.
- The local model and generic runtime assets.

Customer-specific packages, branches and native builds are prohibited by default. Customer identity and leak attribution go in signed entitlements and policy bundles. Releases stay reproducible, and one security update serves every deployment.

</details>

### Registry

npmjs is the registry of record; no other registry, mirror or feed carries Rerail. `.github/workflows/release.yml` publishes each tagged version via npm trusted publishing (OIDC from a protected GitHub environment, no long-lived publish token).

The package is public, like the Apache-2.0 source; no npm account needed:

```sh
npx @neralabs/rerail setup
```

### Release channels

Each version is built once and published under one npm dist-tag, its channel:

```text
next    preview, opt-in
beta    designated customer preview
latest  stable customer release
```

Merging the **Release** pull request publishes core and rule packs to `latest`. Previews come from a hand-pushed `v<version>-next.N` or `-beta.N` tag.

<details>
<summary>How a release runs</summary>

Core (`0.1.0`, `0.1.1`, …) and rule packs publish plain versions straight to `latest`:

1. `.github/workflows/release-pr.yml` keeps the **Release** pull request current with `main`. It bumps core's patch version if anything core ships changed since its last `v<version>` tag, beside the packs' changeset bumps.
2. Merging it is the release: the workflow tags core's new version.
3. `release.yml` builds and publishes core; `release-packs.yml` publishes packs. Each publish waits for a reviewer to approve the `release` environment. Each run then checks `latest` is the version it published ([release](release.md#release), [pack-format.md](pack-format.md#release)).

**GitHub Releases:** every released version, core's and each pack's, gets one on its tag, which comes first: neither the tag nor the Release waits on npm, and a failed publish is re-run for the same tagged version. Its notes are that version's section of its `CHANGELOG.md`: notes are written once, in the changelog, for the people who upgrade. Core's `latest` release is the repository's latest; its `next` and `beta` releases are pre-releases ([release](release.md#release)).

</details>

<details>
<summary>Dist-tag rules, rollback and provenance</summary>

**`next` never trails `latest`,** so whoever follows `next` gets every stable release too:

- A `latest` publish or promotion, core's or a pack's, also raises that package's `next`, unless `next` holds a newer preview (`scripts/release/raise-next.mjs`; `releasePacks`, `promotePacks`).
- Each run's dist-tag check fails if a package it released under `latest` or `next` has a `next` unset or older than its `latest`.
- A core `next` or `beta` preview older than `latest` is refused before it is built.
- Previews leave `latest` alone; putting a version back leaves `next` where it is.

**Rollback** moves only the dist-tag; bytes are never rebuilt between channels. `workflow_dispatch` with `latest` set on `release.yml` (core) or `release-packs.yml` (a pack) points `latest` at an earlier released version and holds it there. Both use npm trusted publishing, so no npm login is needed.

**Provenance:** every publish from CI carries an npm provenance attestation. Both workflows pass `--provenance`, so a publish npm cannot attest fails instead of going out without one.

</details>

## npm-backed explicit updates

Updates travel over npm and run only when a person asks.

Rerail uses the machine's npm configuration and authentication. Agents call the stable `rerail` launcher, which runs the active version under `~/.rerail/`.

```sh
rerail update check
rerail update
rerail update status
rerail events          # internal testing; experimental: needs RERAIL_EXPERIMENTAL_EVENT_STREAM=1
```

Hooks, the daemon and `rerail events` never contact a registry. A new version arrives only via `rerail update`, and applies to newly started sessions.

<details>
<summary>Rerail home layout and rule packs</summary>

```text
~/.rerail/
|- update-state.json
|- current -> versions/<active>/node_modules/@neralabs/rerail
|- bin/rerail
|- daemon.json
|- packs/node_modules/@neralabs/rerail-pack-<id>/
|- packs/node_modules/[@<scope>/]rerail-pack-<id>/
`- versions/
   |- 0.2.0/node_modules/@neralabs/rerail/
   `- 0.2.1/node_modules/@neralabs/rerail/
```

- **npm** is the authenticated artifact transport. Rerail never copies, logs or sends npm credentials to a control plane.
- **Rule packs** are npm packages: Rerail's under `@neralabs`, a third party's as `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` ([rule packs](rule-packs.md#third-parties-packs)). They install with the same npm configuration into `packs/`, beside the versions, so `rerail update` keeps them.
- **Pack compatibility** is `peerDependencies["@neralabs/rerail"]`. After an update, packs whose range excludes the new version leave the catalog, reported by `rerail packs list`, until a newer pack is installed.
- **`update-state.json`** is the active-version pointer: selected dist-tag, current and previous known-good installs, last explicit check. It is written via a temporary file and atomic rename.

</details>

<details>
<summary>What an update does</summary>

**Explicit only.** `rerail events` ([internal testing](../README.md#internal-testing)), hooks and the daemon run the active version. Versions arrive via `rerail update`, `/rerail-update` (OMP, Pi, Claude Code, Cursor) or `$rerail-update` (Codex). Auto-updating a machine's global package without consent is out of scope.

**New sessions only.** A running event-loop process keeps its original version but evaluates nothing itself. Like every managed client, it sends events to the machine's one main daemon, which the new version's first hook or session replaces ([One main instance](how-setup-works.md#one-main-instance)).

An update:

1. Copies the database to `$RERAIL_HOME/rerail.sqlite.pre-update`, so every rule and repository's history can be restored by hand if the update changed the file ([Database compatibility](how-setup-works.md#database-compatibility)).
2. Resolves the exact version behind the configured dist-tag.
3. Compares it with the active semantic version.
4. Takes a machine-wide update lock.
5. Installs that exact version into a staging directory, production dependencies only.
6. Confirms package name and version.
7. Loads the native module and requires a successful health response.
8. Moves staging into its immutable version directory.
9. Atomically marks the old version previous and activates the new; the old stays in `versions/`.
10. Runs `rerail install --refresh` as the new CLI for every connected agent: commands, skills and hooks it ships arrive, retired ones go. It rewrites the launcher; hooks, status line and launcher then name the Node the update ran on.
    - Runs even with no newer version.
    - An agent that cannot be registered again fails alone; the update exits non-zero with the new version active.
    - Agents whose configuration cannot be read to tell whether they are connected are named and left alone.
    - The update that introduces this step runs the previous version's updater, which lacks it: run `rerail install --refresh` once after it.

**Health gate:** the native entrypoint and a real `RERAIL-EVENTS/1` session-start exchange must pass, or the update is discarded without changing the active version.

**Safe failure:** failed resolution, authentication, installation or health checks leave the current version unchanged, so revoked npm access blocks only future updates. Concurrent updates share the lock and cannot race activation.

**Channels** are dist-tags (`latest` stable; `beta`, `next` opt-in). Production activation profiles will eventually constrain the channel; until activation is implemented, local update state selects it.

</details>

## Activation

Activation binds an install to one customer deployment via a signed entitlement.

Internal testing: activation profiles (`rerail activate`, `rerail activation request`, `rerail profile` and `--profile`) are not part of Rerail's latest release. They are experimental: each refuses, and no stored profile restricts `rerail update`, unless `RERAIL_EXPERIMENTAL_ACTIVATION_PROFILES=1`. See [Internal testing](internal-testing.md).

Package access authorizes download; the entitlement authorizes execution.

Online activation:

```sh
rerail --profile acme activate \
  --code <one-use-code> \
  --activation-url https://activation.example.com \
  --trust-root ./activation-roots.json
```

Offline activation:

```sh
rerail --profile acme activation request \
  --deployment acme-airgap \
  --output activation-request.json

rerail --profile acme activate \
  --file acme-profile.jws \
  --trust-root ./activation-roots.json
```

The trust-root file must arrive over an authenticated channel independent of the profile.

<details>
<summary>Trust root, returned items and device binding</summary>

**Trust root:** public but security-critical. It must arrive through the authenticated installer, customer portal or another authenticated channel; one delivered beside an untrusted profile proves nothing. Once production keys exist, a release may embed the same public roots and drop the command-line requirement.

A short-lived, one-use code returns:

- A flattened JWS customer profile.
- A unique device credential.
- The selected control-plane endpoint.
- The delegated policy-signing public key.
- Product features and npm release channel.
- Entitlement and offline-grace expiration.

**Device binding:** the device generates an Ed25519 key pair. Its private key and the device credential live only in macOS Keychain, Linux Secret Service or Windows Credential Manager. They never go in npm config, agent manifests, profiles, logs or the package. The profile carries the public key's RFC 7638 thumbprint, so copied to another device it fails verification.

</details>

Inspect, select and remove profiles with `rerail profile status`, `rerail --profile <name> profile select` and `rerail --profile <name> profile remove`. Removal also deletes public activation state and the credential-store entry.

Agent configuration calls only the launcher:

```json
{
  "command": "rerail",
  "args": ["events", "--profile", "acme"]
}
```

<details>
<summary>What the launcher checks</summary>

The launcher verifies the selected profile before starting:

- The `local-policy-kernel` feature authorizes local execution.
- The signed `updateChannel` is authoritative; a local `--channel` request cannot escape it.
- `automaticUpdates` is carried for compatibility but drives nothing; no code path installs a version unattended.
- Without an activated profile, the machine keeps the unbound local development mode.

</details>

## Signed customer profile

A profile is an Ed25519-signed JWS binding configuration to one customer, deployment and device.

```json
{
  "schemaVersion": 1,
  "issuer": "https://activation.example.com",
  "audience": "rerail-customer-profile",
  "profileId": "prof_acme_01",
  "sequence": 7,
  "customerId": "acme",
  "deploymentId": "acme-us-prod",
  "deviceId": "dev_01",
  "deviceKeyThumbprint": "sha256:...",
  "controlPlane": {
    "url": "https://acme.rerail.example.com",
    "audience": "rerail-acme"
  },
  "policyTrust": {
    "keyId": "acme-policy-2026-01",
    "algorithm": "Ed25519",
    "publicKey": {"kty": "OKP", "crv": "Ed25519", "x": "..."}
  },
  "entitlement": {
    "features": ["local-policy-kernel", "managed-policies"],
    "automaticUpdates": true,
    "updateChannel": "latest",
    "offlineUntil": "2026-12-01T00:00:00Z"
  },
  "notBefore": "2026-09-01T00:00:00Z",
  "expiresAt": "2026-10-01T00:00:00Z"
}
```

<details>
<summary>Format and verification</summary>

**Format:** flattened JWS, `alg: EdDSA`, required media type `rerail-profile+jws`. The signature covers the encoded payload bytes. Rerail verifies it before parsing or trusting any profile field.

**Verification** checks a trusted `kid`, `EdDSA`, the Rerail JWS media type, a valid signature, exact issuer and audience, the local device-key thumbprint and the validity window.

**Refreshes** cannot change customer or deployment identity or lower the monotonic `sequence`. Public activation state is written atomically with mode `0600`; the previous signed state is kept.

</details>

## Control-plane deployment modes

Four modes share one versioned control-plane interface.

### Shared multi-tenant

The default. Tenant identity comes from the authenticated device credential, never a client-supplied ID. Data, policy keys, audit records and admin access are tenant-isolated.

### Dedicated managed instance

The same release with separate compute, storage and keys, for contractual isolation, data residency, network allowlisting or dedicated capacity. A deployment configuration, not a code fork.

### Customer-hosted

The customer runs Nera's signed image or deployment chart in its own network. Clients keep the same activation, authentication and policy-verification protocol.

### Air-gapped

Nera's signed runtime, entitlement and policy bundle arrive by the customer's approved transfer process. Evaluation stays local; updates are imported explicitly.

## Trust separation

Each key grants one power, limiting what a compromise can do.

```text
Nera release root
|- software update signing key
|- activation profile signing key
`- policy delegation root
   |- Acme policy key
   |- Contoso policy key
   `- Globex policy key
```

Policy servers cannot sign executable updates; one customer's compromised policy key cannot sign another's policies.

The software update signing key is planned, not built; today `rerail update` relies on npm's signatures ([installation.md](installation.md#registry-access)).

<details>
<summary>What `rerail update` verifies today</summary>

No Rerail code verifies a Nera signature on an update today. Before it runs a staged version, `rerail update` checks the registry signatures and provenance attestations of everything it installed with `npm audit signatures`. It installs with no install scripts. Those attestations come from Nera's trusted-publisher release workflow, so they bind a version to that workflow, not to a key Nera holds offline.

</details>

## Local operation

Policy is verified ahead of time, so evaluation needs no network round trip.

<details>
<summary>Policy activation and outages</summary>

The runtime downloads a signed policy manifest. It checks customer and deployment identity, runtime compatibility, bundle digest and signature, and the complete policy schema, then activates atomically. The prior known-good bundle is kept for rollback.

During an outage the runtime uses its active bundle until the signed offline entitlement expires. A failed download or validation of a replacement never destroys the last-known-good state.

</details>

## Multiple profiles on one machine

Customer data is never combined implicitly: profiles never share credentials, policy caches, sessions or diagnostics.

```text
~/.rerail/profiles/<customer>/<deployment>/
```

A managed coding-agent binding may select one explicitly:

```sh
rerail --profile acme events
```

Otherwise exactly one active profile must exist, or startup fails explaining how to select one.

## Security limits

Signatures and access controls protect integrity, not secrecy.

<details>
<summary>What each control does and does not protect</summary>

- **Restricted distribution** before launch limits who downloads built releases; it does not make code secret. The source is public under Apache-2.0.
- **Administrators** can always inspect the runtime and its decrypted memory.
- **Device-bound credentials and signed assets** protect integrity and attribute activations; they are not confidentiality controls.
- **Policy and model encryption at rest** stops casual file copying but cannot hide assets while the authorized runtime evaluates them.

Rerail is licensed under Apache-2.0. Restricted npm access does not limit the rights the licence grants recipients.

</details>

## Evolution

The public npm package is the distribution channel. Any future portal or registry must ship the same signed, immutable artifacts, with no customer-specific forks.
