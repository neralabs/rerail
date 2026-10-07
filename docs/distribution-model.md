# Distribution Model

## Decision

Rerail ships as one customer-neutral local runtime. Customer identity, control-plane location, policy trust, release channel, and entitlement are supplied by a signed activation profile rather than compiled into the package.

The policy kernel and latency-sensitive evaluation run on the customer's machine. Network services provide distribution, activation, entitlement renewal, and signed policy delivery; they are not in the evaluation path.

The initial distribution channel is the restricted npm package `@neralabs/rerail`, served from a private Azure Artifacts feed until launch (see [Pre-launch registry](#pre-launch-registry)). A customer download portal may replace npm access management later without changing the runtime artifact or activation model.

## System shape

```text
Harnesses                        Customer environment              Nera infrastructure
---------                        --------------------              -------------------
OMP / Pi -----------\
Claude Code ---------+---------> local Rerail runtime <-----------> customer control plane
Codex ---------------+           local policy kernel               (shared or dedicated)
Cursor --------------/                    ^
                                           |
                                           +----------------------- signed software updates
```

Harness adapters normalize native lifecycle events and pass them to the same local runtime. Customer-specific behavior belongs in signed policy and configuration, not in adapter forks.

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

### Pre-launch registry

Before launch, the registry of record is a private Azure Artifacts npm feed:

- Organization: `nera-software` (Entra tenant `nera.software`).
- Feed: `nera-software`.
- Registry: `https://pkgs.dev.azure.com/nera-software/_packaging/nera-software/npm/registry/`.

Each developer authenticates with their own organization-scoped, read-only personal access token (Packaging: Read). `just rerail` mints it through the Azure CLI after `az login` and stores it in `~/.npmrc`; re-running `just rerail` replaces an expired one. Organization access is Stakeholder, granted by a group rule on the `Nera Developers` Entra group, so adding someone to that group is the whole onboarding step. `rerail-release.yml` publishes tagged versions with the `AZURE_DEVOPS_NPM_TOKEN` secret to the registry named by the `RERAIL_AZURE_NPM_REGISTRY` repository variable.

#### Anonymous read at launch: undecided

Nobody outside the organization can install Rerail yet. The options considered, each set aside for now:

- Public npmjs: publishes the compiled package, native kernels included, to anyone.
- A shared read token embedded in the install script: it leaks with the script and expires on a schedule nobody owns.
- A read-through proxy in front of the feed: more infrastructure to run, secure, and keep up.
- A feed in a public Azure DevOps project: Azure DevOps is retiring public projects.

Consequence for NER-1610: the no-account one-line `npx` install in the first-run journey cannot work for people outside the organization until this is decided. NER-1610 builds against the private route in the meantime.

### npm access

This is the launch-era model, for when the package is on npmjs; it is not in effect yet. The package is restricted. Each customer receives read-only access through a dedicated npm organization team. Individual developers authenticate with their npm identities; automation uses a read-only granular token limited to this package.

Example access grant:

```sh
npm access grant read-only neralabs:customer-acme @neralabs/rerail
```

Customer installation:

```sh
npm login
npm install @neralabs/rerail
```

Revoking npm access prevents future downloads. It does not remove previously downloaded artifacts or terminate an offline entitlement.

### Release channels

A package version is built once. Promotion changes only its npm dist-tag:

```text
next    internal acceptance
beta    designated customer preview
latest  stable customer release
```

The bytes must not be rebuilt between channels. Production publication uses npm provenance and restricted access.

## npm-backed explicit updates

npm is the authenticated artifact transport for updates. Rerail uses the customer's existing npm authentication to resolve and install the restricted package; it never copies, logs, or sends npm credentials to a control plane.

Harness configurations invoke the stable `rerail` launcher rather than a version-specific file. Managed versions live below the Rerail home directory:

```text
~/.rerail/
|- update-state.json
|- current -> versions/<active>/node_modules/@neralabs/rerail
|- bin/rerail
|- daemon.json
`- versions/
   |- 0.2.0/node_modules/@neralabs/rerail/
   `- 0.2.1/node_modules/@neralabs/rerail/
```

The state file is the active-version pointer. It records the selected npm dist-tag, current installation, previous known-good installation, and the last explicit check. State updates use a temporary file and atomic rename.

The launcher supports:

```sh
rerail update check
rerail update
rerail update status
rerail update rollback
rerail events
```

Updates are explicit. `rerail events`, the harness hooks, and the daemon run the active version and never contact a registry; a new version arrives only when a person runs `rerail update` (or `/rerail-update` inside OMP and Pi, `/rerail update` inside Claude Code). Auto-updating a machine's global package without consent is out of scope. A successful update becomes active only for newly started harness sessions; an already running event-loop process continues on its original version, but it evaluates nothing itself: like every managed client it sends events to the machine's one main daemon, which the first hook or session of the new version replaces (see "One main instance" in `docs/installation.md`).

An update:

1. Exports the rules of the current project, and of the project the daemon serves, to `<project>/.rerail/rules.json`, so a later bootstrap can restore them.
2. Resolves the exact version behind the configured npm dist-tag.
3. Compares it with the active semantic version.
4. Acquires a machine-wide update lock.
5. Installs the exact package version into a staging directory with production dependencies only.
6. Confirms the installed package name and version.
7. Loads the native module and requires a successful native health response.
8. Moves the staged installation into its immutable version directory.
9. Atomically records the old current version as previous and activates the new version.
10. Retains the previous installation for explicit rollback.

Failed resolution, authentication, installation, or health checks leave the current version unchanged. Revoked npm access therefore prevents future updates without breaking an installed version. Concurrent updates share the update lock and cannot race activation.

Channels map directly to npm dist-tags. `latest` is stable, while `beta` and `next` are opt-in. Production activation profiles eventually constrain the permitted channel; until activation is implemented, the local update state owns this selection.

Both the native entrypoint and a real `RERAIL-EVENTS/1` session-start exchange must pass before activation. An update that fails either check is discarded without changing the active version.

## Activation

Installation and customer activation are separate operations. Package access authorizes download; a signed entitlement authorizes execution and selects a customer deployment.

Online activation:

```sh
rerail --profile acme activate \
  --code <one-use-code> \
  --activation-url https://activation.neralabs.com \
  --trust-root ./nera-activation-roots.json
```

Offline activation:

```sh
rerail --profile acme activation request \
  --deployment acme-airgap \
  --output activation-request.json

rerail --profile acme activate \
  --file acme-profile.jws \
  --trust-root ./nera-activation-roots.json
```

The activation trust-root file is public but security-critical. It must arrive through the authenticated installer, customer portal, or another authenticated channel independent of the profile. Accepting a trust root delivered beside an untrusted profile provides no authenticity. A production release may embed the same public roots and remove the command-line requirement after the production keys exist.

Activation exchanges a short-lived, one-use code for:

- A flattened JWS customer profile.
- A unique device credential.
- The selected control-plane endpoint.
- The delegated policy-signing public key.
- Product features and npm release channel.
- Entitlement and offline-grace expiration.

The device generates an Ed25519 key pair. Its private key and returned device credential are stored in macOS Keychain, Linux Secret Service, or Windows Credential Manager and are never placed in npm configuration, harness manifests, profile files, logs, or the package. The profile contains the RFC 7638 thumbprint of the public key, so copying a profile to another device fails verification.

`rerail profile status`, `rerail --profile <name> profile select`, and `rerail --profile <name> profile remove` inspect, select, and remove profiles. Removing a profile deletes its public activation state and its operating-system credential-store entry.

Harness configuration invokes only the stable local `rerail` launcher:

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
  "issuer": "https://activation.neralabs.com",
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

A managed harness binding may select a profile explicitly:

```sh
rerail --profile acme events
```

When no profile is specified, exactly one active machine profile must exist; otherwise startup fails with instructions to select one.

## Security limits

Private distribution controls who can download releases. It does not make customer-side code impossible to inspect. Native compilation, symbol stripping, device-bound credentials, signed assets, and customer watermarking raise extraction cost and make leaks attributable, but a machine administrator can eventually inspect code and decrypted memory.

Software signatures protect integrity, not confidentiality. Policy and model encryption at rest prevents casual file copying but cannot hide assets while the authorized runtime evaluates them.

The current repository license remains MIT until Nera makes a separate legal and product decision. Restricted npm access does not override rights granted by the license to recipients.

## Evolution

Restricted npm is appropriate for initial technical customers. Move customer-facing downloads to a portal or commercial artifact registry when npm organization membership becomes operationally limiting. The portal should continue distributing the same signed, immutable artifacts and must not create customer-specific runtime forks.
