// Probe better-sqlite3 under the running Node, rebuild it in the target prefix when it fails, and re-probe.
// Self-contained: a fresh checkout has no dist/ yet, so this cannot import src/runtime/sqlite-repair.ts. The daemon
// launcher runs the same repair through that module before every spawn; this script is the manual entry point.
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const checkoutRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const probeArgs = ["-e", "new(require('better-sqlite3'))(':memory:').close()"];

export function parseEnsureSqliteArguments(argv, defaultPrefix = checkoutRoot) {
  let prefix = defaultPrefix;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--prefix") {
      const value = argv[index + 1];
      if (value === undefined) throw new Error("--prefix requires a directory");
      prefix = resolve(value);
      index += 1;
    } else if (argument.startsWith("--prefix=")) {
      prefix = resolve(argument.slice("--prefix=".length));
    } else {
      throw new Error(`Unknown ensure-sqlite option: ${argument}`);
    }
  }
  return { prefix };
}

/** npm as an inherited npm_execpath, the CLI script beside this Node on Windows, or `npm` from PATH. */
export function npmCommand() {
  const inherited = process.env.npm_execpath;
  if (inherited !== undefined && /(?:^|[\\/])npm-cli\.js$/iu.test(inherited)) return [process.execPath, inherited];
  if (process.platform === "win32") {
    return [process.execPath, join(dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js")];
  }
  return ["npm"];
}

function probe(prefix) {
  const result = spawnSync(process.execPath, probeArgs, { cwd: prefix, encoding: "utf8" });
  if (result.error) throw result.error;
  return result;
}

export function ensureSqlite(prefix, { log = line => console.log(line) } = {}) {
  let result = probe(prefix);
  if (result.status === 0) return { prefix, rebuilt: false };
  log(`Rebuilding better-sqlite3 in ${prefix} for Node ${process.versions.node} (ABI ${process.versions.modules})`);
  const [command, ...leading] = npmCommand();
  const rebuild = spawnSync(command, [...leading, "rebuild", "better-sqlite3"], { cwd: prefix, stdio: "inherit" });
  if (rebuild.error) throw rebuild.error;
  if (rebuild.status !== 0) throw new Error(`SQLite dependency rebuild failed in ${prefix}`);
  result = probe(prefix);
  if (result.status !== 0) throw new Error(`SQLite dependency in ${prefix} is still unusable: ${result.stderr}`);
  return { prefix, rebuilt: true };
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const { prefix } = parseEnsureSqliteArguments(process.argv.slice(2));
  ensureSqlite(prefix);
}
