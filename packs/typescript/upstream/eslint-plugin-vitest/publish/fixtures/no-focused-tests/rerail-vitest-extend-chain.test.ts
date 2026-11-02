const { test: base } = require('vitest');

const extended = base.extend({});
const it = extended.extend({});

it.only('focused', () => {});
