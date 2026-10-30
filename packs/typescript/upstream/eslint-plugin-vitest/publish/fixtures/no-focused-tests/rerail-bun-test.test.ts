import { describe, it } from 'bun:test';

describe.only('suite', () => {
  it.skip('skipped', () => {});
  it('todo');
});
