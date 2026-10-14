import { test as base } from 'vitest';

const it = base.extend({});

it.only('focused', () => {});
it('asserts nothing', () => {});
