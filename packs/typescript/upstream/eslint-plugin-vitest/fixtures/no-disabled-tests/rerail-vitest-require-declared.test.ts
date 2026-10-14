function require(name: string) {
  return { it: { skip(title: string, fn: () => void) {} } };
}
const { it } = require('vitest');

it.skip('skipped', () => {});
