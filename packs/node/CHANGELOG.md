# @neralabs/rerail-pack-node

## 0.1.1

### Patch Changes

- f374b76: Node pack: `n-prefer-promises-fs` no longer reports `fs.watch` (its promise form returns an async iterator, not a drop-in replacement), and `n-no-callback-literal`'s message now says to check the callback is error-first before passing an error or null (ws `verifyClient` and ssh2 `authHandler` are not).
- 37c902a: Node.js pack: the 10 rules Oxlint 1.88.0 also implements honour its suppression comments naming them (`// oxlint-disable-line no-process-env` and the other `oxlint-disable` forms), as well as ESLint's. `n-no-process-env` honours Biome 2.5.15's `biome-ignore` comments naming `lint/style/noProcessEnv` (or its group, or `lint`).
- fbb6104: Node.js pack: 22 eslint-plugin-n rules, moved to packs/node with its conformance recordings, candidates and licence record.
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
