/**
 * bandit-b201-flask-debug-true must report `.run(debug=True)` on an app made by a factory (#252).
 *
 * Bandit 1.9.4's B201 (bandit/plugins/app_debug.py) does not trace the receiver. It reports a call whose qualified name
 * ends in `.run` and whose `debug` keyword is the literal `True` or the string "True", once the file has imported a
 * module whose name contains `flask`. The port used to report only a receiver assigned from `flask.Flask(...)`, so
 * `app = create_app(); app.run(debug=True)` was missed. It now reports Bandit's case too, once per call. On an app
 * made by `flask.Flask` it still reports any truthy `debug`, passed by keyword, position or `**{...}`, which Bandit
 * does not; every other case's expected positions are what Bandit 1.9.4 reports on it (line, then its column plus one).
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "bandit-b201-flask-debug-true";
const document = await readFile(repositoryPath("packs", "python", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The `line:column` (both 1-based) of each violation the rule reports in `source`. */
function reported(source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "app/main.py", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => `${violation.span.startLine + 1}:${violation.span.startColumn + 1}`);
}

const cases: readonly { name: string; source: string; expected: readonly string[] }[] = [
  {
    name: "an app returned by a factory",
    source: "from flask import Flask\nfrom myapp import create_app\napp = create_app()\napp.run(debug=True)\n",
    expected: ["4:1"],
  },
  {
    name: "a factory call as the receiver",
    source: "from flask import Flask\ncreate_app().run(debug=True)\n",
    expected: ["2:1"],
  },
  {
    name: "an app made by flask.Flask, as before",
    source: "import flask\napp = flask.Flask(__name__)\napp.run(debug=True)\n",
    expected: ["3:1"],
  },
  {
    name: "an app made by flask.Flask, with unpacked arguments beside debug",
    source: "from flask import Flask\napp = Flask(__name__)\napp.run(debug=True, **kw)\napp.run(*args, debug=True)\n",
    expected: ["3:1", "4:1"],
  },
  {
    name: "an imported app, a parameter and a loop variable",
    source:
      "from flask import Flask\nfrom myapp import app\napp.run(debug=True)\ndef serve(app):\n    app.run(debug=True)\n" +
      "for app in apps:\n    app.run(debug=True)\n",
    expected: ["3:1", "5:5", "7:5"],
  },
  {
    name: "a file that imports a module named like flask",
    source: "import flask_sqlalchemy\napp = make()\napp.run(debug=True)\n",
    expected: ["3:1"],
  },
  {
    name: "debug given as the string True, or parenthesized",
    source: 'from flask import Flask\napp.run(debug="True")\napp.run(debug=(True))\n',
    expected: ["2:1", "3:1"],
  },
  {
    name: "a run call before the flask import",
    source: "app.run(debug=True)\nfrom flask import Flask\n",
    expected: [],
  },
  {
    name: "a file that imports no flask module",
    source: "from myapp import create_app\napp = create_app()\napp.run(debug=True)\n",
    expected: [],
  },
  {
    name: "debug off, absent or not a literal True",
    source:
      "from flask import Flask\napp = Flask(__name__)\napp.run(debug=False)\napp.run()\n" +
      'app.run(debug=DEBUG)\napp.run(debug=f"True")\n',
    expected: [],
  },
  {
    name: "a truthy debug, positional or unpacked, on an app made by flask.Flask, as before",
    source:
      'from flask import Flask\napp = Flask(__name__)\napp.run(debug=1)\napp.run("0.0.0.0", 5000, True)\n' +
      'app.run(**{"debug": True})\n',
    expected: ["3:1", "4:1", "5:1"],
  },
  {
    name: "a truthy debug, positional or unpacked, on an app from a factory",
    source:
      'from flask import Flask\napp = create_app()\napp.run(debug=1)\napp.run("0.0.0.0", 5000, True)\n' +
      'app.run(**{"debug": True})\n',
    expected: [],
  },
  {
    name: "a bare run call or another method",
    source: "from flask import Flask\nrun(debug=True)\napp.start(debug=True)\n",
    expected: [],
  },
];

describe(RULE_ID, () => {
  it.each(cases)("$name", ({ source, expected }) => {
    expect(reported(source)).toEqual(expected);
  });
});
