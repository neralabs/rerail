# Security policy

Rerail runs inside the coding-agent loop: its hooks and extensions see the agent's tool calls, and its daemon listens on
loopback. We treat reports about it accordingly.

## Reporting a vulnerability

Report vulnerabilities privately through GitHub:
[Report a vulnerability](https://github.com/neralabs/rerail/security/advisories/new) (the repository's
**Security** tab, then **Report a vulnerability**). Include a description, the affected version, and steps to
reproduce.

Please do not open a public issue or disclose the vulnerability until it has been fixed.

## What to expect

- We acknowledge your report within **two business days**.
- We keep you informed while we investigate and fix it, and agree a disclosure date with you.
- We credit you in the advisory and the release notes, if you wish.

## Supported versions

Security fixes ship as a patch release of the **latest minor version** of each affected package: the core package
`@neralabs/rerail` and the rule packs `@neralabs/rerail-pack-*`. Earlier minor versions receive no fixes; upgrade to the
latest to get them.
