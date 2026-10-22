// Regenerates the parsers vendored grammars commit generated (native/build.rs's `committed_parser`: SQL's
// native/grammars/tree-sitter-sql/src/parser.generated.c.gz), after a change to their grammar.json. The engine's build
// script is the generator: with RERAIL_GENERATE_PARSERS=1 it runs tree-sitter-generate (the version Cargo.lock pins)
// and writes the gzipped parser back beside grammar.json; without it, a build refuses a committed parser generated from
// anything else. About 15 minutes for SQL's, with the release build it runs.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../..", import.meta.url));
const result = spawnSync("cargo", ["build", "--release", "--manifest-path", "native/Cargo.toml"], {
  cwd: root,
  env: { ...process.env, RERAIL_GENERATE_PARSERS: "1" },
  stdio: "inherit",
});
if (result.error !== undefined) throw result.error;
process.exit(result.status ?? 1);
