---
"@neralabs/rerail-pack-node": patch
---

Node.js pack: `n/prefer-node-protocol` reads the file's Node version from its nearest `package.json` (`engines.node`, else the `devEngines.runtime` entry named `node`, else `>=16.0.0`), as eslint-plugin-n does, so a package whose Node range includes versions without `node:` support (`>=14`) is no longer told to use `node:` in `require` or imports; `process.getBuiltinModule` is checked whatever the range. The `n/hashbang` candidate reads the nearest `package.json`'s `bin`: a file `bin` names must start with a node shebang, and only other files are told they need none.
