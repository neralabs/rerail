/**
 * The sql pack: SQLFluff's findings on its own rule test cases (`generate.py fixtures`, which copies the cases from the
 * pinned sqlfluff checkout), in the virtual environment fixtures/sql/common.py pins, on the CPython build
 * fixtures/sql/python-downloads.json pins (the SQLite it links is a pin).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { fixturesPath } from "../../../test/support/oracles.js";
import { derivationEnvironment, type DeriveRecipe } from "../context.js";

interface Pins {
  python: string;
  requirements: string[];
  dialects: { repository: string; commit: string; checkout?: string };
}

// common.py's pins as JSON: the requirements are the distributions check_pins() checks, at its TOOLS versions.
const READ_PINS = `
import json, sys
sys.path.insert(0, sys.argv[1])
import common
dists = ("sqlfluff", "pglast", "tree-sitter", "pyyaml", "pymysql")
print(json.dumps({"python": common.PYTHON, "requirements": [f"{d}=={common.TOOLS[d]}" for d in dists], "dialects": common.CORPORA["sqlfluff-dialects"]}))
`;

export const recipes: DeriveRecipe[] = [
  {
    pack: "sql",
    tool: "sqlfluff",
    async derive({ toolDirectory, cacheRoot, exec, capture, checkout, python, pythonTools }) {
      const common = fixturesPath("sql");
      const generator = join(toolDirectory, "generate.py");
      const version = /^PYTHON = "([^"]+)"$/mu.exec(readFileSync(join(common, "common.py"), "utf8"))?.[1];
      if (version === undefined) throw new Error("fixtures/sql/common.py declares no PYTHON");
      const interpreter = await python(version, { downloads: join(common, "python-downloads.json") });

      // The sqlfluff checkout generate.py copies the rule cases from (common.CORPORA's sqlfluff-dialects), at its pin.
      const directory = join(cacheRoot, "corpora", "sql");
      const env = { ...derivationEnvironment(), RERAIL_SQL_CORPORA: directory };
      const pins = JSON.parse(await capture([interpreter, "-c", READ_PINS, common], { env })) as Pins;
      const cases = /^CASES = "([^"]+)"$/mu.exec(readFileSync(generator, "utf8"))?.[1];
      if (cases === undefined) throw new Error("generate.py declares no CASES");
      const { dialects } = pins;
      await checkout(dialects.repository, dialects.commit, { directory: join(directory, dialects.checkout ?? "sqlfluff-dialects"), sparse: [`/${cases}/`] });

      const venv = await pythonTools("sqlfluff", interpreter, pins.requirements);
      await exec([join(venv, "venv", "bin", "python"), generator, "fixtures"], { env });
    },
  },
];
