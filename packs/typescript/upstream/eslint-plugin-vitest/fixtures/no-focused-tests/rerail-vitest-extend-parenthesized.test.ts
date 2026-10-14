import { test as base } from 'vitest';

const it = (base.extend({}));

it.only('focused', () => {});
it.skip('skipped', () => {});
it('asserts nothing', () => {});
