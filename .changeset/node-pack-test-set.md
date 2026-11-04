---
"@neralabs/rerail-pack-node": patch
---

Set-up now recommends the pack for a repository with a Node.js server framework (`express`, `koa`, `fastify`, `@hapi/hapi`, `@nestjs/core`, `restify`, `@adonisjs/core`, `sails`, `@feathersjs/feathers`, `moleculer`) or `electron` dependency, or with `.cjs`, `.mjs`, `.cts` or `.mts` files outside tests, scripts, fixtures and configuration. An `@types/node` dependency alone, which browser projects carry for tooling, does not.
