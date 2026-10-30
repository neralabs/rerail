import { describe, it } from 'node:test';

describe.only('suite', () => {
  it.only('focused', t => {
    t.assert.ok(true);
  });
  it.skip('skipped', () => {});
  it('todo');
});
