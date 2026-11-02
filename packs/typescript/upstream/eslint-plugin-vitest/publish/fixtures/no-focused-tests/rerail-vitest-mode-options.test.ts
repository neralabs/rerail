import { describe, test } from 'vitest';

test('focused', { only: true }, () => {
  expect(1).toBe(1);
});
test('skipped', { only: false, skip: true }, () => {});
describe('focused later', { only: false, only: true }, () => {});
