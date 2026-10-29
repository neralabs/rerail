# Distribution Model

## Decision

Rerail ships as one customer-neutral local runtime. Customer identity, control-plane location, policy trust, release channel, and entitlement are supplied by a signed activation profile rather than compiled into the package.

The engine and latency-sensitive evaluation run on the customer's machine. Network services provide distribution, activation, entitlement renewal, and signed policy delivery; they are not in the evaluation path.

The distribution channel is the npm package `@neralabs/rerail` on npmjs, the only registry Rerail is published to (see [Registry](#registry)). A customer download portal may be added later without changing the runtime artifact or activation model.

## System shape

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

Host adapters normalize native lifecycle events and pass them to the same local runtime. Customer-specific behavior belongs in signed policy and configuration, not in adapter forks.

## Distribution artifact

Nera publishes one immutable package version for all customers and supported platforms:

```text
@neralabs/rerail@<version>
```

The package contains:

- OMP and Pi extensions.
- Transport-neutral lifecycle adapters for Claude Code, Codex, and Cursor.
- The persistent Rerail event stream.
- Supported native binaries.
- The local model and generic runtime assets.

A customer-specific package, branch, or native build is prohibited by default. Customer-specific identity and leak attribution belong in signed entitlements and policy bundles. This keeps releases reproducible and allows one security update to serve every deployment.

### Registry

npmjs is the registry of record. `.github/workflows/release.yml` publishes each tagged version there with npm trusted publishing (OIDC from a protected GitHub environment, no long-lived publish token). No other registry, mirror, or feed carries Rerail.

The package is public: it installs without an npm account, matching the Apache-2.0 source, which is public as well.

Installation:

```sh
npx @neralabs/rerail setup
```

### Release channels

A package version is built once and published under one npm dist-tag, the channel:

```text
next    preview, opt-in
beta    designated customer preview
latest  stable customer release
```

Releases are plain versions published straight to `latest`: core's `0.1.0`, `0.1.1`, … and each rule pack's
versions. Merging the **Release** pull request, which `.github/workflows/release-pr.yml` keeps up to date from `main`,
is the release: it bumps core's patch version when anything core ships has changed since its last `v<version>` tag,
beside the packs' changeset bumps. On merge the workflow tags core's new version, `release.yml` builds and publishes
it, and `release-packs.yml` publishes the packs; each publish waits for a reviewer to approve the `release`
environment, and each run then checks that `latest` is the version it published ([release](release.md#release),
[pack-format.md](pack-format.md#release)). `next` and `beta` stay available for a preview: a `v<version>-next.N` or
`-beta.N` tag pushed by hand publishes under that channel.

Putting a version back moves only its dist-tag, never the bytes: `release.yml`'s `workflow_dispatch` with `latest`
set points core's `latest` at an earlier released version, and `release-packs.yml`'s does the same for a pack, and
holds it there. Both use npm trusted publishing, so no one needs an npm login. The bytes must not be rebuilt between
channels. Every publish from CI carries an npm provenance attestation: both workflows pass `--provenance`, so a publish
npm cannot attest fails instead of going out without one.

## npm-backed explicit updates

npm is the authenticated artifact transport for updates. Rerail uses the machine's existing npm configuration and authentication to resolve and install the package; it never copies, logs, or sends npm credentials to a control plane.

Coding-agent configurations invoke the stable `rerail` launcher rather than a version-specific file. Managed versions live below the Rerail home directory:

```text
~/.rerail/
|- update-state.json
|- current -> versions/<active>/node_modules/@neralabs/rerail
|- bin/rerail
|- daemon.json
|- packs/node_modules/@neralabs/rerail-pack-<id>/
`- versions/
   |- 0.2.0/node_modules/@neralabs/rerail/
   `- 0.2.1/node_modules/@neralabs/rerail/
```

Rule packs are npm packages too, installed with the same npm configuration into `packs/`, beside the versions rather than inside one, so `rerail update` and `rerail update rollback` keep them. A pack names the core versions it works with (`peerDependencies["@neralabs/rerail"]`); after an update the catalog leaves out, and `rerail packs list` reports, any installed pack whose range excludes the new version until a newer pack is installed.

The state file is the active-version pointer. It records the selected npm dist-tag, current installation, previous known-good installation, and the last explicit check. State updates use a temporary file and atomic rename.

The launcher supports:

```sh
rerail update check
rerail update
rerail update status
rerail update rollback
rerail events          # internal testing
```

Updates are explicit. `rerail events` ([internal testing](../README.md#internal-testing)), the coding agent hooks, and the daemon run the active version and never contact a registry; a new version arrives only when a person runs `rerail update` (or `/rerail-update` inside OMP, Pi, Claude Code, and Cursor, `$rerail-update` in Codex). Auto-updating a machine's global package without consent is out of scope. A successful update becomes active only for newly started coding agent sessions; an already running event-loop process continues on its original version, but it evaluates nothing itself: like every managed client it sends events to the machine's one main daemon, which the first hook or session of the new version replaces (see "One main instance" in `docs/installation.md`).

An update:

1. Copies the Rerail database to `$RERAIL_HOME/rerail.sqlite.pre-update`, so every rule and repository's history survives a failed or regretted update.
2. Resolves the exact version behind the configured npm dist-tag.
3. Compares it with the active semantic version.
4. Acquires a machine-wide update lock.
5. Installs the exact package version into a staging directory with production dependencies only.
6. Confirms the installed package name and version.
7. Loads the native module and requires a successful native health response.
8. Moves the staged installation into its immutable version directory.
9. Atomically records the old current version as previous and activates the new version.
10. Retains the previous installation for explicit rollback.
11. Registers the active version again with every coding agent already connected (`rerail install --refresh`, run as that version's CLI), so the commands, skills and hooks it ships arrive and the ones it retired go, and writes the launcher again; hooks, status line and launcher then name the Node the update ran on. This step runs even when no newer version was found. A connected coding agent that cannot be registered again fails this step for that agent only, and the update then exits non-zero with the new version still active; an agent whose configuration cannot be read to tell whether it is connected is named and left alone. The update that brings this step runs the previous version's update, which lacks it: run `rerail install --refresh` once after it.

Failed resolution, authentication, installation, or health checks leave the current version unchanged. Revoked npm access therefore prevents future updates without breaking an installed version. Concurrent updates share the update lock and cannot race activation.

Channels map directly to npm dist-tags. `latest` is stable, while `beta` and `next` are opt-in. Production activation profiles eventually constrain the permitted channel; until activation is implemented, the local update state owns this selection.

Both the native entrypoint and a real `RERAIL-EVENTS/1` session-start exchange must pass before activation. An update that fails either check is discarded without changing the active version.

## Activation

Internal testing: activation profiles (`rerail activate`, `rerail activation request`, `rerail profile` and `--profile`) are not part of Rerail 0.1.0. See the README's [Internal testing](../README.md#internal-testing) section.

Installation and customer activation are separate operations. Package access authorizes download; a signed entitlement authorizes execution and selects a customer deployment.

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

The activation trust-root file is public but security-critical. It must arrive through the authenticated installer, customer portal, or another authenticated channel independent of the profile. Accepting a trust root delivered beside an untrusted profile provides no authenticity. A production release may embed the same public roots and remove the command-line requirement after the production keys exist.

Activation exchanges a short-lived, one-use code for:

- A flattened JWS customer profile.
- A unique device credential.
- The selected control-plane endpoint.
- The delegated policy-signing public key.
- Product features and npm release channel.
- Entitlement and offline-grace expiration.

The device generates an Ed25519 key pair. Its private key and returned device credential are stored in macOS Keychain, Linux Secret Service, or Windows Credential Manager and are never placed in npm configuration, coding-agent manifests, profile files, logs, or the package. The profile contains the RFC 7638 thumbprint of the public key, so copying a profile to another device fails verification.

`rerail profile status`, `rerail --profile <name> profile select`, and `rerail --profile <name> profile remove` inspect, select, and remove profiles. Removing a profile deletes its public activation state and its operating-system credential-store entry.

Coding-agent configuration invokes only the stable local `rerail` launcher:

```json
{
  "command": "rerail",
  "args": ["events", "--profile", "acme"]
}
```

The launcher verifies the selected profile before starting. The `local-policy-kernel` feature authorizes local execution. The signed `updateChannel` is authoritative, so a local `--channel` request cannot escape it. The profile's `automaticUpdates` flag is carried for compatibility but drives nothing: no code path installs a version unattended. Machines without an activated profile retain the unbound local development mode.

## Signed customer profile

Rerail accepts flattened JSON Web Signatures using Ed25519 (`alg: EdDSA`) and the required media type `rerail-profile+jws`. The signature covers the encoded payload bytes; Rerail verifies it before parsing or trusting any profile field.

A profile binds configuration to one customer, deployment, and device:

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

Verification requires a trusted `kid`, `EdDSA`, the Rerail JWS media type, a valid signature, exact issuer and audience, the local device-key thumbprint, and the applicable validity window. Profile refreshes cannot change customer or deployment identity and cannot reduce the monotonic sequence number. Public activation state is written atomically with mode `0600`; the previous signed state is retained.

## Control-plane deployment modes

All modes implement the same versioned control-plane interface.

### Shared multi-tenant

The default deployment. Tenant identity is derived from the authenticated device credential, never from a client-provided customer identifier. Data, policy keys, audit records, and administrative access remain tenant-isolated.

### Dedicated managed instance

The same control-plane release is deployed with separate compute, storage, and keys for a customer that requires contractual isolation, data residency, network allowlisting, or dedicated capacity. It is a deployment configuration, not a code fork.

### Customer-hosted

Nera supplies a signed image or deployment chart. The customer operates the control plane in its network. Local clients retain the same activation, authentication, and policy-verification protocol.

### Air-gapped

Nera supplies a signed runtime artifact, signed entitlement, and signed policy bundle through the customer's approved transfer process. Evaluation remains local and updates are imported explicitly.

## Trust separation

Separate keys grant separate powers:

```text
Nera release root
|- software update signing key
|- activation profile signing key
`- policy delegation root
   |- Acme policy key
   |- Contoso policy key
   `- Globex policy key
```

A customer policy server cannot sign executable updates. A compromised customer policy key cannot sign another customer's policies. Executable updates always require a Nera software-update signature, including when downloaded through a dedicated customer instance.

## Local operation

The runtime downloads a signed policy manifest, verifies customer and deployment identity, checks runtime compatibility, verifies the bundle digest and signature, validates the complete policy schema, and atomically activates it. The prior known-good bundle is retained for rollback.

Policy evaluation does not require a network round trip. During an outage the runtime uses its active policy bundle until the signed offline entitlement expires. Failure to download or validate a replacement never destroys the last-known-good state.

## Multiple profiles on one machine

Customer data must never be combined implicitly. Profiles have separate credentials, policy caches, session storage, and diagnostics:

```text
~/.rerail/profiles/<customer>/<deployment>/
```

A managed coding-agent binding may select a profile explicitly:

```sh
rerail --profile acme events
```

When no profile is specified, exactly one active machine profile must exist; otherwise startup fails with instructions to select one.

## Security limits

Restricted distribution before launch controls who can download built releases; it does not make code secret. The source is public under Apache-2.0, and a machine administrator can always inspect the runtime and its decrypted memory. Device-bound credentials and signed assets protect integrity and attribute activations; they are not confidentiality controls.

Software signatures protect integrity, not confidentiality. Policy and model encryption at rest prevents casual file copying but cannot hide assets while the authorized runtime evaluates them.

Rerail is licensed under Apache-2.0. Restricted npm access does not limit the rights the licence grants to recipients.

## Evolution

The public npm package is the distribution channel. A customer portal or other artifact registry, if one is ever added, must distribute the same signed, immutable artifacts and must not create customer-specific runtime forks.
