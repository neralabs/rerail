const { test: it, describe } = require('vitest');

describe.skip('skipped', () => {});
it.only('focused', () => {});
