import { describe, it, test as check } from 'vitest';

describe.only('suite', () => {
  it.skip('skipped', () => {});
  check('asserts nothing', () => {});
  it('todo');
});
