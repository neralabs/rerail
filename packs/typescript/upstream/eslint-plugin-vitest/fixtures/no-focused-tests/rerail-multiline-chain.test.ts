import { describe, it } from 'vitest';

it.only
  .concurrent('focused', () => {});
describe
  .skip('skipped', () => {});
