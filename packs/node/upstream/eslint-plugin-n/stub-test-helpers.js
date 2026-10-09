// Rerail's stand-in for eslint-plugin-n's `tests/test-helpers.js` (`#test-helpers`); not upstream code.
// generate.mjs routes the copied upstream tests' `#test-helpers` and `../../test-helpers.js` imports here. Nothing is
// linted: every `run(name, rule, { valid, invalid })` is captured with the rule and the tester's language options, so
// the generator can write one fixture file per case. Upstream's helper merges the tester config over the same defaults
// (ecmaVersion 6, sourceType commonjs) and drops `skip: true` cases; the generator applies the latter.

/** Every captured run, in import order: `{ name, rule, languageOptions, typescript, valid, invalid }`. */
export const captured = []

export const gteEslintV9 = true
export const isCaseSensitiveFileSystem = false

const defaultLanguageOptions = { ecmaVersion: 6, sourceType: "commonjs" }

function tester(config, typescript) {
  const languageOptions = { ...defaultLanguageOptions, ...(config?.languageOptions ?? {}) }
  return {
    // A function expression: upstream calls `new (TsRuleTester(..).run)(name, rule, tests)`.
    run: function (name, rule, tests) {
      captured.push({ name, rule, languageOptions, typescript, valid: tests.valid, invalid: tests.invalid })
    },
  }
}

export function RuleTester(config) {
  return tester(config, false)
}

export function TsRuleTester(configOrFixturePath) {
  return tester(typeof configOrFixturePath === "object" ? configOrFixturePath : undefined, true)
}

export { RuleTester as FlatRuleTester }
