import { test as describe } from 'vitest';

describe.fails.only('focused', () => {});
describe.fails.skip('skipped', () => {});
