---
"@neralabs/rerail-pack-node": patch
---

Node.js pack: ships 17 eslint-plugin-n rules, judged on the Node test set (239 real files from open-source history at the commit that introduced each violation, with eslint-plugin-n's own findings, and 222 copies fixed by hand) as well as on the plugin's own tests. `n-no-process-exit` starts on, as the plugin's recommended configurations turn it on; the other 16 start off. `n-callback-return`, `n-hashbang`, `n-no-deprecated-api`, `n-no-exports-assign`, `n-no-path-concat`, `n-prefer-global-buffer` and `n-prefer-global-process` stay candidates: each misses findings the plugin reports, on its tests or on the test set.

The pack is now recommended for a repository with a Node.js server framework (`express`, `koa`, `fastify`, `@hapi/hapi`, `@nestjs/core`, `restify`, `@adonisjs/core`, `sails`, `@feathersjs/feathers`, `moleculer`) or `electron` dependency, or with `.cjs`, `.mjs`, `.cts` or `.mts` files outside tests, scripts, fixtures and configuration. An `@types/node` dependency, which browser projects carry for their tooling, no longer recommends it.
