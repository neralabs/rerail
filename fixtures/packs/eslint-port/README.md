# eslint-port

A fixture pack whose ported ESLint rule honours `eslint-disable` comments as ESLint does: it passes, and copies of it
without the port's `suppress`, or with a `suppressionExemptions` entry, prove the suppression check
(test/engine/catalog/packs.test.ts).
