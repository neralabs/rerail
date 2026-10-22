/**
 * The sql pack: SQLFluff's findings on its own rule test cases (`generate.py fixtures`, which copies the cases from the
 * pinned sqlfluff checkout) and on the pack's T4 corpora (`generate.py corpus`), in the virtual environment
 * fixtures/sql/common.py pins.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { readRerailCorpora } from "../../../conformance/port-readiness/corpus.js";
import { derivationEnvironment, type DeriveRecipe } from "../context.js";

interface Pins {
  python: string;
  requirements: string[];
  corpora: Record<string, { repository: string; commit: string; checkout?: string; units: [string, string][] }>;
}

// common.py's pins as JSON: the requirements are the distributions check_pins() checks, at its TOOLS versions.
const READ_PINS = `
import json, sys
sys.path.insert(0, sys.argv[1])
import common
dists = ("sqlfluff", "pglast", "tree-sitter", "pyyaml", "pymysql")
print(json.dumps({"python": common.PYTHON, "requirements": [f"{d}=={common.TOOLS[d]}" for d in dists], "corpora": common.CORPORA}))
`;

/** The directory a unit glob's matches lie under (its segments before the first wildcard), as a sparse path. */
const globDirectory = (pattern: string): string => {
  const segments = pattern.split("/");
  return `/${segments.slice(0, segments.findIndex((segment) => /[*?[]/u.test(segment))).join("/")}/`;
};

export const recipes: DeriveRecipe[] = [
  {
    pack: "sql",
    tool: "sqlfluff",
    inputs: ["fixtures/sql/common.py", "fixtures/sql/reference.py", "scripts/s3/pack_fixtures.py"],
    runtimes: ["uv"],
    async derive({ root, toolDirectory, exec, capture, checkout, python, pythonTools, corpora }) {
      const common = join(root, "fixtures/sql");
      const generator = join(toolDirectory, "generate.py");
      const version = /^PYTHON = "([^"]+)"$/mu.exec(readFileSync(join(common, "common.py"), "utf8"))?.[1];
      if (version === undefined) throw new Error("fixtures/sql/common.py declares no PYTHON");
      const interpreter = await python(version);

      // The pack's corpora, then the others common.CORPORA pins (check_pins() requires every one at its commit):
      // generate.py reads the rule cases of sqlfluff-dialects, and only the commit of the rest.
      const directory = await corpora();
      const pinned = new Set((await readRerailCorpora(root, "sql")).map((corpus) => corpus.name));
      const env = { ...derivationEnvironment(), RERAIL_SQL_CORPORA: directory };
      const pins = JSON.parse(await capture([interpreter, "-c", READ_PINS, common], { env })) as Pins;
      const cases = /^CASES = "([^"]+)"$/mu.exec(readFileSync(generator, "utf8"))?.[1];
      if (cases === undefined) throw new Error("generate.py declares no CASES");
      for (const [name, corpus] of Object.entries(pins.corpora)) {
        if (pinned.has(name)) continue;
        const sparse = [...new Set(corpus.units.map(([pattern]) => globDirectory(pattern)))];
        if (name === "sqlfluff-dialects") sparse.push(`/${cases}/`);
        await checkout(corpus.repository, corpus.commit, { directory: join(directory, corpus.checkout ?? name), sparse });
      }

      const venv = await pythonTools("sqlfluff", interpreter, pins.requirements);
      const venvPython = join(venv, "venv", "bin", "python");
      await exec([venvPython, generator, "fixtures"], { env });
      await exec([venvPython, generator, "corpus"], { env });
    },
  },
];
