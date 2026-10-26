"use strict";
// Loaded in place of ava when eslint-driver.mjs loads upstream's test files: AVA's `test` registers a test the way the
// test-framework global `test` does, and its hooks and modifiers do nothing.
const register = (title, body) => globalThis.test(title, body);
const test = Object.assign(register, {
  only: register,
  serial: register,
  skip: (title, body) => globalThis.test.skip(title, body),
  failing: register,
  todo: () => undefined,
  before: () => undefined,
  after: () => undefined,
  beforeEach: () => undefined,
  afterEach: () => undefined,
  macro: (macro) => macro,
});
module.exports = test;
