"use strict";
// Loaded in place of vitest when eslint-driver.mjs loads upstream's test files: the test-framework globals it installs.
module.exports = {
  describe: globalThis.describe,
  it: globalThis.it,
  test: globalThis.test,
  beforeAll: globalThis.beforeAll,
  afterAll: globalThis.afterAll,
  beforeEach: globalThis.beforeEach,
  afterEach: globalThis.afterEach,
  expect: globalThis.expect,
  vi: globalThis.vi,
};
