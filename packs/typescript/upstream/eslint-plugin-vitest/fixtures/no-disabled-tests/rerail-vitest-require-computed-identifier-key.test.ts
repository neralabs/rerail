const it = 'expect';
const { [it]: check } = require('vitest');

check.skip('skipped', () => {});
