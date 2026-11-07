import { readFileSync } from "node:fs";
import { basename, extname, resolve } from "node:path";
import { bundledCodeRuleDslDirectory } from "../engine/code-rule/code-rule-compile.js";
import { loadNativeBinding, type CodeRuleDslSourceFormat, type NativeSlicePolicy } from "../engine/native/native.js";

export const ENGINE_USAGE = [
  "  rerail engine dump --rule <file>... [--clause <id>] [--fused] [--setplan]",
  "  rerail engine asm <file.rir>",
  "  rerail engine sql <store.sqlite> <request.json>",
];

/**
 * The engine's text tools (list step 18, P spec-tooling §2.2.1–2.2.2): `dump` lowers rule files without a store
 * and prints their `rir`; `asm` assembles a `rir` file through the verifier and prints it again, or the refusal.
 */
export function runEngineCommand(args: readonly string[], usage: () => string): number {
  const [subcommand, ...rest] = args;
  if (subcommand === "dump") return dump(rest, usage);
  if (subcommand === "asm") return assemble(rest, usage);
  if (subcommand === "sql") return sql(rest, usage);
  throw new Error(`Usage:\n${ENGINE_USAGE.join("\n")}\n\n${usage()}`);
}

function sql(args: readonly string[], usage: () => string): number {
  if (args.length !== 2) throw new Error(`rerail engine sql takes a store and a request file\n\n${usage()}`);
  process.stdout.write(loadNativeBinding().executeSqlRequest(resolve(args[0] ?? ""), readFileSync(resolve(args[1] ?? ""), "utf8")));
  return 0;
}

function dump(args: readonly string[], usage: () => string): number {
  const rules: string[] = [];
  let clause: string | undefined;
  let fused = false;
  let setplan = false;
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index] ?? "";
    if (argument === "--fused") fused = true;
    else if (argument === "--setplan") setplan = true;
    else if (argument === "--rule" || argument === "--clause") {
      const value = args[index + 1];
      if (value === undefined || value.startsWith("--")) throw new Error(`${argument} requires a value`);
      if (argument === "--rule") rules.push(value);
      else clause = value;
      index += 1;
    } else throw new Error(`Unknown engine dump option: ${argument}\n\n${usage()}`);
  }
  if (rules.length === 0) throw new Error(`rerail engine dump needs at least one --rule <file>\n\n${usage()}`);
  const policies = rules.map(policy);
  const capabilities = readFileSync(resolve(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  process.stdout.write(loadNativeBinding().dumpRir(policies, capabilities, clause ?? null, fused, setplan));
  return 0;
}

function assemble(args: readonly string[], usage: () => string): number {
  if (args.length !== 1) throw new Error(`rerail engine asm takes one file\n\n${usage()}`);
  process.stdout.write(loadNativeBinding().assembleRir(readFileSync(resolve(args[0] ?? ""), "utf8")));
  return 0;
}

/** A rule file as a policy: named by its file, its format from the extension. */
function policy(file: string): NativeSlicePolicy {
  const extension = extname(file).toLowerCase();
  const format: CodeRuleDslSourceFormat | undefined =
    extension === ".json" ? "json" : extension === ".yaml" || extension === ".yml" ? "yaml" : undefined;
  if (format === undefined) throw new Error(`${file}: a rule file is .json, .yaml or .yml`);
  return { name: basename(file, extname(file)), source: readFileSync(resolve(file), "utf8"), format };
}
