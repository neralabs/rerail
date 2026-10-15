import { describe, test } from 'node:test';

test('focused', { only: true }, t => {
  t.assert.ok(true);
});
describe('skipped', { skip: true }, () => {});
